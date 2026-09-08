//! Canonical Integration ledger only; all Git/filesystem effects belong to Supervisor.
use crate::{PostgresStore, StorageError, StorageTransaction, encode_snapshot, u64_to_i64};
use forge_domain::{
    ProjectId, StageId, Task, TaskId,
    candidate_review::{CandidateReviewRecord, CandidateVerdict},
    git::GitIntegrationIntent,
    git_integration::{GitIntegrationOperation, IntegrationRequest, IntegrationResult},
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationState {
    Preparing,
    Prepared,
    Applying,
    Held,
    Completed,
    Retired,
}
#[derive(Clone, Debug)]
pub struct StoredIntegration {
    pub operation: GitIntegrationOperation,
    pub intent: Option<GitIntegrationIntent>,
    pub state: IntegrationState,
    pub core_instance: Uuid,
    pub expected_task_revision: u64,
    pub command: Option<IntegrationRequest>,
    pub result: Option<IntegrationResult>,
    pub wait_condition_id: Option<Uuid>,
    pub resolution: Option<String>,
}
impl PostgresStore {
    pub async fn git_integration_page(
        &self,
        project: ProjectId,
        task: TaskId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<StoredIntegration>, StorageError> {
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM git_integrations WHERE project_id=$1 AND task_id=$2 AND ($3::uuid IS NULL OR id>$3) ORDER BY id LIMIT $4")
            .bind(project.as_uuid()).bind(task.as_uuid()).bind(after).bind(i64::from(limit.min(101))).fetch_all(&self.pool).await?;
        let mut tx = self.begin().await?;
        let mut items = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(item) = tx.load_git_integration(project, id).await? {
                if item.operation.task_id != task {
                    return Err(invalid());
                }
                items.push(item);
            }
        }
        tx.commit().await?;
        Ok(items)
    }
    pub async fn integration_project(&self, id: Uuid) -> Result<Option<ProjectId>, StorageError> {
        Ok(
            sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM git_integrations WHERE id=$1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .map(ProjectId::from),
        )
    }
    pub async fn integration_work(&self) -> Result<Vec<(ProjectId, TaskId)>, StorageError> {
        let rows:Vec<(Uuid,Uuid)>=sqlx::query_as("SELECT project_id,task_id FROM (SELECT i.project_id,i.task_id,i.updated_at FROM git_integrations i WHERE i.state IN ('preparing','prepared','applying') OR (i.state='held' AND (i.resolution IS NOT NULL OR (i.command IS NOT NULL AND COALESCE(i.result->>'code','') NOT IN ('applied','retryable') AND NOT(i.command->>'phase'='reconcile' AND COALESCE(i.result->>'command_id'=i.command->>'command_id',false))))) UNION SELECT t.project_id,t.id,t.updated_at FROM tasks t JOIN pipeline_versions v ON v.id=t.pipeline_version_id WHERE t.lifecycle='in_progress' AND v.definition->'stages'->t.current_stage_id->'system_action'->>'kind'='git_integration' AND NOT EXISTS(SELECT 1 FROM git_integrations i WHERE i.task_id=t.id AND i.state NOT IN ('completed','retired'))) pending ORDER BY updated_at LIMIT 64")
            .fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|(project, task)| (ProjectId::from(project), TaskId::from(task)))
            .collect())
    }
}
impl StorageTransaction<'_> {
    pub async fn insert_integration_handoff(
        &mut self,
        operation: Uuid,
        handoff: &forge_domain::TaskHandoff,
    ) -> Result<(), StorageError> {
        let data = handoff.data();
        if data.producer
            != (forge_domain::HandoffProducer::SystemAction {
                operation_id: operation,
            })
        {
            return Err(invalid());
        }
        let result=sqlx::query("INSERT INTO task_handoffs(id,project_id,task_id,kind,body,integration_id) SELECT $1,$2,$3,'accepted',$4::jsonb,id FROM git_integrations WHERE project_id=$2 AND task_id=$3 AND id=$5 ON CONFLICT(integration_id) DO NOTHING")
            .bind(data.id).bind(data.project_id.as_uuid()).bind(data.task_id.as_uuid()).bind(encode_snapshot(handoff,"integration.handoff")?).bind(operation).execute(&mut *self.transaction).await?;
        if result.rows_affected() != 1 {
            return Err(invalid());
        }
        Ok(())
    }
    pub async fn integration_request_expired(&mut self, id: Uuid) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM git_integration_receipts WHERE command_id=$1 AND result IS NULL AND created_at<clock_timestamp()-interval '120 seconds')")
            .bind(id).fetch_one(&mut *self.transaction).await?)
    }
    pub async fn integration_for_task(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Option<StoredIntegration>, StorageError> {
        let id:Option<Uuid>=sqlx::query_scalar("SELECT id FROM git_integrations WHERE project_id=$1 AND task_id=$2 AND state NOT IN ('completed','retired') FOR UPDATE")
            .bind(project.as_uuid()).bind(task.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        match id {
            Some(id) => self.load_git_integration(project, id).await,
            None => Ok(None),
        }
    }
    pub async fn load_git_integration(
        &mut self,
        project: ProjectId,
        id: Uuid,
    ) -> Result<Option<StoredIntegration>, StorageError> {
        let row=sqlx::query("SELECT canonical_snapshot::text,intent::text,state,core_instance,expected_task_revision,command::text,result::text,wait_condition_id,resolution FROM git_integrations WHERE project_id=$1 AND id=$2 FOR UPDATE")
            .bind(project.as_uuid()).bind(id).fetch_optional(&mut *self.transaction).await?;
        row.map(|row| {
            let operation: GitIntegrationOperation =
                decode(&row.try_get::<String, _>("canonical_snapshot")?)?;
            operation.validate().map_err(|_| invalid())?;
            if operation.id != id || operation.project_id != project {
                return Err(invalid());
            }
            let intent: Option<GitIntegrationIntent> = row
                .try_get::<Option<String>, _>("intent")?
                .map(|s| decode(&s))
                .transpose()?;
            if let Some(intent) = &intent {
                operation.validate_intent(intent).map_err(|_| invalid())?;
            }
            Ok(StoredIntegration {
                operation,
                intent,
                state: decode(
                    &serde_json::to_string(&row.try_get::<String, _>("state")?)
                        .map_err(|_| invalid())?,
                )?,
                core_instance: row.try_get("core_instance")?,
                expected_task_revision: crate::i64_to_u64(
                    row.try_get("expected_task_revision")?,
                    "integration.revision",
                )?,
                command: row
                    .try_get::<Option<String>, _>("command")?
                    .map(|s| decode(&s))
                    .transpose()?,
                result: row
                    .try_get::<Option<String>, _>("result")?
                    .map(|s| decode(&s))
                    .transpose()?,
                wait_condition_id: row.try_get("wait_condition_id")?,
                resolution: row.try_get("resolution")?,
            })
        })
        .transpose()
    }
    pub async fn next_integration_fence(&mut self) -> Result<u64, StorageError> {
        crate::i64_to_u64(
            sqlx::query_scalar("SELECT nextval('git_integration_fence_seq')")
                .fetch_one(&mut *self.transaction)
                .await?,
            "integration.fence",
        )
    }
    pub async fn insert_git_integration(
        &mut self,
        stored: &StoredIntegration,
    ) -> Result<(), StorageError> {
        let op = &stored.operation;
        op.validate().map_err(|_| invalid())?;
        sqlx::query("INSERT INTO git_integrations(id,project_id,task_id,candidate_proposal_id,fence,canonical_snapshot,state,core_instance,expected_task_revision) VALUES($1,$2,$3,$4,$5,$6::jsonb,'preparing',$7,$8)")
            .bind(op.id).bind(op.project_id.as_uuid()).bind(op.task_id.as_uuid()).bind(op.candidate_proposal_id).bind(u64_to_i64(op.fence,"integration.fence")?)
            .bind(encode_snapshot(op,"integration")?).bind(stored.core_instance).bind(u64_to_i64(stored.expected_task_revision,"integration.revision")?).execute(&mut *self.transaction).await?;
        Ok(())
    }
    pub async fn save_git_integration(
        &mut self,
        stored: &StoredIntegration,
    ) -> Result<(), StorageError> {
        let result=sqlx::query("UPDATE git_integrations SET intent=$3::jsonb,state=$4,core_instance=$5,expected_task_revision=$6,command=$7::jsonb,result=$8::jsonb,wait_condition_id=$9,resolution=$10,updated_at=clock_timestamp() WHERE project_id=$1 AND id=$2 AND state NOT IN ('completed','retired')")
            .bind(stored.operation.project_id.as_uuid()).bind(stored.operation.id).bind(stored.intent.as_ref().map(|x|encode_snapshot(x,"integration.intent")).transpose()?)
            .bind(crate::enum_text(&stored.state,"integration.state")?).bind(stored.core_instance).bind(u64_to_i64(stored.expected_task_revision,"integration.revision")?)
            .bind(stored.command.as_ref().map(|x|encode_snapshot(x,"integration.request")).transpose()?).bind(stored.result.as_ref().map(|x|encode_snapshot(x,"integration.result")).transpose()?).bind(stored.wait_condition_id).bind(&stored.resolution)
            .execute(&mut *self.transaction).await?;
        if result.rows_affected() != 1 {
            return Err(invalid());
        }
        Ok(())
    }
    pub async fn save_integration_request(
        &mut self,
        request: &IntegrationRequest,
    ) -> Result<(), StorageError> {
        request.validate().map_err(|_| invalid())?;
        sqlx::query("INSERT INTO git_integration_receipts(command_id,operation_id,request) VALUES($1,$2,$3::jsonb)")
            .bind(request.command_id).bind(request.operation.id).bind(encode_snapshot(request,"integration.request")?).execute(&mut *self.transaction).await?;
        Ok(())
    }
    pub async fn integration_ever_authorized_apply(
        &mut self,
        id: Uuid,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM git_integration_receipts WHERE operation_id=$1 AND request->>'phase'='apply')")
            .bind(id).fetch_one(&mut *self.transaction).await?)
    }
    pub async fn integration_receipt(
        &mut self,
        id: Uuid,
    ) -> Result<Option<(IntegrationRequest, Option<IntegrationResult>)>, StorageError> {
        let row=sqlx::query("SELECT request::text,result::text FROM git_integration_receipts WHERE command_id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *self.transaction).await?;
        row.map(|row| {
            Ok((
                decode(&row.try_get::<String, _>("request")?)?,
                row.try_get::<Option<String>, _>("result")?
                    .map(|s| decode(&s))
                    .transpose()?,
            ))
        })
        .transpose()
    }
    pub async fn save_integration_result(
        &mut self,
        result: &IntegrationResult,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE git_integration_receipts SET result=$2::jsonb WHERE command_id=$1 AND operation_id=$3 AND result IS NULL")
            .bind(result.command_id).bind(encode_snapshot(result,"integration.result")?).bind(result.operation_id).execute(&mut *self.transaction).await?;
        Ok(())
    }
    pub async fn integration_dispatch_open(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects p WHERE p.id=$1 AND p.execution_enabled AND NOT EXISTS(SELECT 1 FROM project_recovery_settings h WHERE h.project_id=p.id AND h.hold)) AND NOT EXISTS(SELECT 1 FROM runs r LEFT JOIN leases l ON l.id=r.lease_id LEFT JOIN run_environment_reservations e ON e.run_id=r.id WHERE r.project_id=$1 AND r.task_id=$2 AND (l.lease_state='active' OR e.released_at IS NULL)) AND NOT EXISTS(SELECT 1 FROM task_dependencies d JOIN tasks b ON b.id=d.blocker_task_id WHERE d.project_id=$1 AND d.blocked_task_id=$2 AND b.lifecycle<>d.required_blocker_lifecycle)")
            .bind(project.as_uuid()).bind(task.as_uuid()).fetch_one(&mut *self.transaction).await?)
    }
    pub async fn integration_reviews_accepted(
        &mut self,
        task: &Task,
        proposal: Uuid,
        stages: &std::collections::BTreeSet<StageId>,
    ) -> Result<bool, StorageError> {
        for stage in stages {
            let text:Option<String>=sqlx::query_scalar("SELECT canonical_snapshot::text FROM candidate_reviews WHERE project_id=$1 AND task_id=$2 AND candidate_proposal_id=$3 AND canonical_snapshot->>'pipeline_version_id'=$4 AND canonical_snapshot->>'stage_id'=$5 ORDER BY recorded_at DESC,id DESC LIMIT 1")
                .bind(task.project_id().as_uuid()).bind(task.id().as_uuid()).bind(proposal).bind(task.pipeline().pipeline_version_id().to_string()).bind(stage.as_str()).fetch_optional(&mut *self.transaction).await?;
            let Some(text) = text else {
                return Ok(false);
            };
            let review: CandidateReviewRecord = decode(&text)?;
            review.validate().map_err(|_| invalid())?;
            if review.verdict != CandidateVerdict::Accepted
                || review.task_id != task.id()
                || review.candidate_proposal_id != proposal
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub async fn accepted_candidate_writer(
        &mut self,
        project: ProjectId,
        proposal: Uuid,
    ) -> Result<Option<forge_domain::git_delivery::GitStageProposal>, StorageError> {
        let text:Option<String>=sqlx::query_scalar("SELECT canonical_snapshot::text FROM git_stage_proposals WHERE project_id=$1 AND id=$2 AND proposal_state='accepted'")
            .bind(project.as_uuid()).bind(proposal).fetch_optional(&mut *self.transaction).await?;
        text.map(|s| decode(&s)).transpose()
    }
}
fn invalid() -> StorageError {
    StorageError::InvalidInput {
        reason: "invalid canonical Git integration".into(),
    }
}
fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, StorageError> {
    serde_json::from_str(text).map_err(|source| StorageError::Snapshot {
        aggregate: "git integration",
        source,
    })
}
