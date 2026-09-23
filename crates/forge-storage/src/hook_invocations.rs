//! Canonical owner-selected Hook execution, separate from provider and Task leases.
use crate::{
    PostgresStore, RunProjection, StorageError, StorageTransaction, encode_snapshot, u64_to_i64,
};
use forge_domain::{
    ProjectId, TaskHandoff, TaskId,
    runtime::{HookRunSpec, RunScope},
};
use serde_json::Value;
use sqlx::Row;
use time::OffsetDateTime;
use uuid::Uuid;

// Exclude retained blockers before the bounded scan. The dispatcher repeats
// these authority checks under the Project lock before creating a Run.
const DISPATCH_ELIGIBILITY: &str = "p.execution_enabled
 AND NOT EXISTS(SELECT 1 FROM project_recovery_settings s WHERE s.project_id=p.id AND s.hold)
 AND NOT EXISTS(SELECT 1 FROM runs r JOIN leases l ON forge_lease_owns_run(l,r)
  LEFT JOIN hook_invocations owned ON owned.id=r.hook_invocation_id
  WHERE r.project_id=p.id AND (r.task_id=t.id OR owned.task_id=t.id)
   AND (l.lease_state='active' OR EXISTS(SELECT 1 FROM run_environment_reservations e
    WHERE e.run_id=r.id AND e.released_at IS NULL)))
 AND NOT EXISTS(SELECT 1 FROM task_dependencies d JOIN tasks blocker ON blocker.id=d.blocker_task_id
  WHERE d.project_id=p.id AND d.blocked_task_id=t.id AND blocker.lifecycle<>d.required_blocker_lifecycle)";

/// Inapplicability is a canonical System result, not an invented execution.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookSkipSpec {
    pub schema_version: u16,
    pub invocation_id: Uuid,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub pipeline_version_id: forge_domain::PipelineVersionId,
    pub stage_id: forge_domain::StageId,
    pub stage_visit: u64,
    pub hook: forge_domain::ProjectHookVersion,
}

#[derive(Clone, Debug)]
pub struct StoredHookInvocation {
    pub spec: HookRunSpec,
    pub run_id: Option<Uuid>,
    pub state: String,
    pub core_instance: Uuid,
    pub expected_task_revision: u64,
    pub result: Option<Value>,
}

/// Bounded operator projection. Never deserialize or expose the private Hook
/// RunSpec, process output, candidate checkout, or raw result evidence.
#[derive(Clone, Debug)]
pub struct HookInvocationView {
    pub id: Uuid,
    pub task_id: TaskId,
    pub pipeline_version_id: forge_domain::PipelineVersionId,
    pub stage_id: String,
    pub stage_visit: u64,
    pub hook_version_id: Uuid,
    pub candidate_proposal_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
    pub state: String,
    pub verdict: Option<String>,
    pub mapped_outcome: Option<String>,
    pub artifact_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}
impl PostgresStore {
    pub async fn hook_invocations_page(
        &self,
        project: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<HookInvocationView>, StorageError> {
        if !(1..=101).contains(&limit) {
            return Err(invalid());
        }
        let rows = sqlx::query(
            r#"SELECT h.id,h.task_id,h.pipeline_version_id,h.stage_id,h.stage_visit,
                      h.hook_version_id,h.candidate_proposal_id,h.run_id,h.state,
                      CASE WHEN h.state='completed' AND h.result->>'verdict' IN
                        ('passed','failed','timed_out','skipped')
                        THEN h.result->>'verdict' END AS verdict,
                      CASE WHEN h.state='completed' AND h.result->>'verdict' IN
                        ('passed','failed','timed_out','skipped')
                        THEN v.definition->'stages'->h.stage_id->'system_action'->'outcomes'->>(h.result->>'verdict')
                      END AS mapped_outcome,h.artifact_id,h.created_at,h.updated_at
               FROM hook_invocations h
               JOIN pipeline_versions v ON v.id=h.pipeline_version_id
               JOIN pipelines p ON p.id=v.pipeline_id AND p.project_id=h.project_id
               WHERE h.project_id=$1 AND ($2::uuid IS NULL OR h.id<$2)
               ORDER BY h.id DESC LIMIT $3"#,
        )
        .bind(project.as_uuid())
        .bind(after)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let state: String = row.try_get("state")?;
                if !matches!(state.as_str(), "running" | "completed" | "held") {
                    return Err(invalid());
                }
                let stage_visit = crate::i64_to_u64(row.try_get("stage_visit")?, "hook.visit")?;
                Ok(HookInvocationView {
                    id: row.try_get("id")?,
                    task_id: TaskId::from(row.try_get::<Uuid, _>("task_id")?),
                    pipeline_version_id: forge_domain::PipelineVersionId::from(
                        row.try_get::<Uuid, _>("pipeline_version_id")?,
                    ),
                    stage_id: row.try_get("stage_id")?,
                    stage_visit,
                    hook_version_id: row.try_get("hook_version_id")?,
                    candidate_proposal_id: row.try_get("candidate_proposal_id")?,
                    run_id: row.try_get("run_id")?,
                    state,
                    verdict: row.try_get("verdict")?,
                    mapped_outcome: row.try_get("mapped_outcome")?,
                    artifact_id: row.try_get("artifact_id")?,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect()
    }
    pub async fn hook_projects(&self) -> Result<Vec<ProjectId>, StorageError> {
        let query = format!(
            "SELECT DISTINCT t.project_id FROM tasks t JOIN projects p ON p.id=t.project_id JOIN pipeline_versions v ON v.id=t.pipeline_version_id WHERE {DISPATCH_ELIGIBILITY} AND t.lifecycle='in_progress' AND v.definition->'stages'->t.current_stage_id->'system_action'->>'kind'='project_hook' AND NOT EXISTS(SELECT 1 FROM hook_invocations h WHERE h.task_id=t.id AND h.state='running') ORDER BY t.project_id LIMIT 64"
        );
        let ids: Vec<Uuid> = sqlx::query_scalar(&query).fetch_all(&self.pool).await?;
        Ok(ids.into_iter().map(ProjectId::from).collect())
    }
    pub async fn hook_work(&self, project: ProjectId) -> Result<Vec<TaskId>, StorageError> {
        let query = format!(
            "SELECT t.id FROM tasks t JOIN projects p ON p.id=t.project_id JOIN pipeline_versions v ON v.id=t.pipeline_version_id WHERE t.project_id=$1 AND {DISPATCH_ELIGIBILITY} AND t.lifecycle='in_progress' AND v.definition->'stages'->t.current_stage_id->'system_action'->>'kind'='project_hook' AND NOT EXISTS(SELECT 1 FROM hook_invocations h WHERE h.task_id=t.id AND h.state='running') ORDER BY t.updated_at,t.id LIMIT 64"
        );
        let ids: Vec<Uuid> = sqlx::query_scalar(&query)
            .bind(project.as_uuid())
            .fetch_all(&self.pool)
            .await?;
        Ok(ids.into_iter().map(TaskId::from).collect())
    }
    pub async fn unfinished_hook_runs(&self) -> Result<Vec<Uuid>, StorageError> {
        Ok(sqlx::query_scalar("SELECT r.id FROM hook_invocations h JOIN runs r ON r.id=h.run_id WHERE h.state='running' AND r.observed_state IN ('stopped','failed','lost') ORDER BY h.updated_at,h.id LIMIT 64").fetch_all(&self.pool).await?)
    }
}
impl StorageTransaction<'_> {
    pub async fn insert_hook_skip(
        &mut self,
        spec: &HookSkipSpec,
        core_instance: Uuid,
        revision: u64,
    ) -> Result<(), StorageError> {
        spec.hook.validate_snapshot().map_err(|_| invalid())?;
        let task = self
            .lock_task(spec.task_id)
            .await?
            .ok_or_else(invalid)?
            .task;
        if spec.schema_version != 1
            || spec.invocation_id.get_version_num() != 7
            || spec.project_id != task.project_id()
            || spec.hook.project_id != spec.project_id
            || spec.hook.applies_to(task.kind())
            || task.pipeline().pipeline_version_id() != spec.pipeline_version_id
            || task.current_stage_id() != Some(&spec.stage_id)
            || task
                .current_stage_visit()
                .is_none_or(|visit| visit.get() != spec.stage_visit)
            || task.revision().get() != revision
        {
            return Err(invalid());
        }
        sqlx::query("INSERT INTO hook_invocations(id,project_id,task_id,pipeline_version_id,stage_id,stage_visit,hook_version_id,state,core_instance,expected_task_revision,spec,result) VALUES($1,$2,$3,$4,$5,$6,$7,'running',$8,$9,$10::jsonb,'{\"verdict\":\"skipped\",\"reason\":\"task_kind_not_applicable\"}'::jsonb)")
            .bind(spec.invocation_id).bind(spec.project_id.as_uuid()).bind(spec.task_id.as_uuid()).bind(spec.pipeline_version_id.as_uuid()).bind(spec.stage_id.as_str()).bind(u64_to_i64(spec.stage_visit,"hook.visit")?).bind(spec.hook.id).bind(core_instance).bind(u64_to_i64(revision,"hook.revision")?).bind(encode_snapshot(spec,"hook.skip")?).execute(&mut *self.transaction).await?;
        Ok(())
    }
    pub async fn hook_for_run(
        &mut self,
        run: Uuid,
    ) -> Result<Option<StoredHookInvocation>, StorageError> {
        let row=sqlx::query("SELECT spec::text,run_id,state,core_instance,expected_task_revision,result FROM hook_invocations WHERE run_id=$1 FOR UPDATE")
            .bind(run).fetch_optional(&mut *self.transaction).await?;
        row.map(|row| {
            let spec: HookRunSpec =
                serde_json::from_str(&row.try_get::<String, _>("spec")?).map_err(|_| invalid())?;
            spec.validate().map_err(|_| invalid())?;
            if spec.run_id != run {
                return Err(invalid());
            }
            Ok(StoredHookInvocation {
                spec,
                run_id: row.try_get("run_id")?,
                state: row.try_get("state")?,
                core_instance: row.try_get("core_instance")?,
                expected_task_revision: crate::i64_to_u64(
                    row.try_get("expected_task_revision")?,
                    "hook.revision",
                )?,
                result: row.try_get("result")?,
            })
        })
        .transpose()
    }
    /// Project -> Task -> live runs. Hook scratch never acquires the Task writer surface.
    pub async fn hook_dispatch_open(
        &mut self,
        project: ProjectId,
        task: TaskId,
        exclude_run: Option<Uuid>,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects p WHERE p.id=$1 AND p.execution_enabled AND NOT EXISTS(SELECT 1 FROM project_recovery_settings s WHERE s.project_id=p.id AND s.hold)) AND NOT EXISTS(SELECT 1 FROM runs r JOIN leases l ON forge_lease_owns_run(l,r) LEFT JOIN hook_invocations h ON h.id=r.hook_invocation_id WHERE r.project_id=$1 AND (r.task_id=$2 OR h.task_id=$2) AND ($3::uuid IS NULL OR r.id<>$3) AND (l.lease_state='active' OR EXISTS(SELECT 1 FROM run_environment_reservations e WHERE e.run_id=r.id AND e.released_at IS NULL))) AND NOT EXISTS(SELECT 1 FROM task_dependencies d JOIN tasks b ON b.id=d.blocker_task_id WHERE d.project_id=$1 AND d.blocked_task_id=$2 AND b.lifecycle<>d.required_blocker_lifecycle)")
            .bind(project.as_uuid()).bind(task.as_uuid()).bind(exclude_run).fetch_one(&mut *self.transaction).await?)
    }
    pub async fn hook_scope_is_active(
        &mut self,
        scope: RunScope,
        core_instance: Uuid,
    ) -> Result<bool, StorageError> {
        let project: Option<Uuid> = sqlx::query_scalar("SELECT project_id FROM runs WHERE id=$1")
            .bind(scope.run_id)
            .fetch_optional(&mut *self.transaction)
            .await?;
        let Some(project) = project else {
            return Ok(false);
        };
        self.lock_project(ProjectId::from(project)).await?;
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs r JOIN leases l ON forge_lease_owns_run(l,r) JOIN hook_invocations h ON h.id=r.hook_invocation_id JOIN tasks t ON t.id=h.task_id JOIN projects p ON p.id=r.project_id WHERE r.id=$1 AND r.lease_fencing_token=$2 AND r.environment_epoch=$3 AND r.employee_id IS NULL AND r.purpose='hook' AND r.desired_state IN ('provision_requested','running') AND r.observed_state NOT IN ('stopped','failed','lost') AND l.lease_state='active' AND p.execution_enabled AND h.state='running' AND h.core_instance=$4 AND t.lifecycle='in_progress' AND t.revision=h.expected_task_revision AND t.current_stage_id=h.stage_id AND t.pipeline_version_id=h.pipeline_version_id AND NOT EXISTS(SELECT 1 FROM project_recovery_settings s WHERE s.project_id=p.id AND s.hold))")
            .bind(scope.run_id).bind(u64_to_i64(scope.fencing_token,"hook.fence")?).bind(u64_to_i64(scope.environment_epoch,"hook.epoch")?).bind(core_instance).fetch_one(&mut *self.transaction).await?)
    }
    pub async fn insert_hook_invocation(
        &mut self,
        stored: &StoredHookInvocation,
    ) -> Result<(), StorageError> {
        stored.spec.validate().map_err(|_| invalid())?;
        let spec = &stored.spec;
        let a = &spec.assignment;
        if stored.state != "running"
            || stored.run_id != Some(spec.run_id)
            || stored.result.is_some()
        {
            return Err(invalid());
        }
        sqlx::query("INSERT INTO hook_invocations(id,project_id,task_id,pipeline_version_id,stage_id,stage_visit,candidate_proposal_id,hook_version_id,run_id,state,core_instance,expected_task_revision,spec,result) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'running',$10,$11,$12::jsonb,$13)")
            .bind(a.invocation_id).bind(spec.project_id.as_uuid()).bind(a.task_id.as_uuid()).bind(a.pipeline_version_id.as_uuid()).bind(a.stage_id.as_str()).bind(u64_to_i64(a.stage_visit,"hook.visit")?).bind(a.candidate_proposal_id).bind(spec.hook.id).bind(stored.run_id).bind(stored.core_instance).bind(u64_to_i64(stored.expected_task_revision,"hook.revision")?).bind(encode_snapshot(spec,"hook.spec")?).bind(&stored.result).execute(&mut *self.transaction).await?;
        Ok(())
    }
    pub async fn create_hook_run(
        &mut self,
        stored: &StoredHookInvocation,
    ) -> Result<RunProjection, StorageError> {
        let spec = &stored.spec;
        spec.validate().map_err(|_| invalid())?;
        let saved = self.hook_for_run(spec.run_id).await?.ok_or_else(invalid)?;
        if saved.spec != stored.spec
            || saved.run_id != stored.run_id
            || saved.state != stored.state
            || saved.expected_task_revision != stored.expected_task_revision
            || saved.core_instance != stored.core_instance
        {
            return Err(invalid());
        }
        if stored.run_id != Some(spec.run_id)
            || !self
                .hook_dispatch_open(spec.project_id, spec.assignment.task_id, None)
                .await?
        {
            return Err(invalid());
        }
        self.require_run_admission(
            spec.project_id,
            5,
            &serde_json::to_value(spec).map_err(|_| invalid())?,
        )
        .await?;
        let lease_id = Uuid::now_v7();
        let lease=sqlx::query("INSERT INTO leases(id,project_id,employee_id,purpose,hook_invocation_id,lease_scope,expires_at) VALUES($1,$2,NULL,'hook',$3,$4::jsonb,clock_timestamp()+$5*interval '1 second') RETURNING fencing_token,environment_epoch")
            .bind(lease_id).bind(spec.project_id.as_uuid()).bind(spec.assignment.invocation_id).bind(serde_json::json!({"assignment":spec.assignment}))
            .bind(i64::from(spec.hook.limits.wall_seconds)+60).fetch_one(&mut *self.transaction).await?;
        sqlx::query("INSERT INTO runs(id,project_id,employee_id,purpose,hook_invocation_id,lease_id,attempt_number,lease_fencing_token,environment_epoch,run_spec_version,run_spec,context_manifest) VALUES($1,$2,NULL,'hook',$3,$4,1,$5,$6,5,$7::jsonb,$8::jsonb)")
            .bind(spec.run_id).bind(spec.project_id.as_uuid()).bind(spec.assignment.invocation_id).bind(lease_id).bind(lease.try_get::<i64,_>("fencing_token")?).bind(lease.try_get::<i64,_>("environment_epoch")?).bind(encode_snapshot(spec,"hook.spec")?).bind(serde_json::json!({"schema_version":5,"assignment":spec.assignment,"capability_grants":[]})).execute(&mut *self.transaction).await?;
        self.reserve_environment(spec.run_id, None).await?;
        self.load_run(spec.run_id).await?.ok_or_else(invalid)
    }
    pub async fn finish_hook(
        &mut self,
        id: Uuid,
        state: &str,
        result: Option<&Value>,
        artifact: Option<forge_domain::ArtifactId>,
    ) -> Result<(), StorageError> {
        if !matches!(state, "completed" | "held") {
            return Err(invalid());
        }
        let updated=sqlx::query("UPDATE hook_invocations SET state=$2,result=$3,artifact_id=$4,updated_at=clock_timestamp() WHERE id=$1 AND state='running'")
            .bind(id).bind(state).bind(result).bind(artifact.map(|id|id.as_uuid())).execute(&mut *self.transaction).await?;
        if updated.rows_affected() != 1 {
            return Err(invalid());
        }
        Ok(())
    }
    pub async fn insert_hook_handoff(
        &mut self,
        id: Uuid,
        handoff: &TaskHandoff,
    ) -> Result<(), StorageError> {
        let data = handoff.data();
        if data.producer != (forge_domain::HandoffProducer::SystemAction { operation_id: id }) {
            return Err(invalid());
        }
        let updated=sqlx::query("INSERT INTO task_handoffs(id,project_id,task_id,kind,body,hook_invocation_id) SELECT $1,$2,$3,'accepted',$4::jsonb,id FROM hook_invocations WHERE id=$5 AND project_id=$2 AND task_id=$3")
            .bind(data.id).bind(data.project_id.as_uuid()).bind(data.task_id.as_uuid()).bind(encode_snapshot(handoff,"hook.handoff")?).bind(id).execute(&mut *self.transaction).await?;
        if updated.rows_affected() != 1 {
            return Err(invalid());
        }
        Ok(())
    }
}
fn invalid() -> StorageError {
    StorageError::InvalidInput {
        reason: "invalid Hook owner, state or physical admission".into(),
    }
}
