//! Inbox-owned admissions use the same canonical Lease/Run/physical-capacity ledger.

use forge_domain::{
    CommunicationAssignmentRef, EmployeeId, ProjectId, Timestamp,
    communication::{CommunicationContext, EmployeeMessage},
    runtime::CommunicationRunSpec,
};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    RunProjection, StorageError, StorageTransaction, database_timestamp, encode_object, i64_to_u64,
};

#[derive(Clone, Debug)]
pub struct CommunicationClaim {
    pub owner: CommunicationAssignmentRef,
    pub project_id: ProjectId,
    pub employee_id: EmployeeId,
    pub source: EmployeeMessage,
    pub attempt_number: u32,
}

impl StorageTransaction<'_> {
    pub async fn messages_for_communication(
        &mut self,
        scope: &forge_domain::communication::DeliveryScope,
        source: Uuid,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<EmployeeMessage>, StorageError> {
        if scope.task_context.is_some() {
            return Err(invalid("requires Communication scope"));
        }
        let values:Vec<String>=sqlx::query_scalar("SELECT m.canonical_snapshot::text FROM employee_messages m JOIN employee_messages source ON source.id=$4 AND source.project_id=m.project_id AND source.employee_id=m.employee_id AND source.thread_id=m.thread_id WHERE m.project_id=$1 AND m.employee_id=$2 AND m.thread_id=$3 AND m.canonical_snapshot->'target'->>'kind'='inbox' AND (m.sequence<=source.sequence OR m.reply_to=source.id) AND ($5::uuid IS NULL OR m.id>$5) ORDER BY m.id LIMIT $6")
            .bind(scope.project_id.as_uuid()).bind(scope.employee_id.as_uuid()).bind(scope.thread_id).bind(source).bind(after).bind(i64::from(limit.clamp(1,101)))
            .fetch_all(&mut *self.transaction).await?;
        values
            .into_iter()
            .map(|value| crate::decode_snapshot(&value, "employee_message"))
            .collect()
    }

    pub async fn communication_requirement_satisfied(
        &mut self,
        scope: forge_domain::runtime::RunScope,
        source: Uuid,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs r JOIN communication_assignments c ON c.id=r.communication_assignment_id JOIN employee_messages m ON m.id=c.source_message_id JOIN employee_message_receipts receipt ON receipt.message_id=m.id AND receipt.run_id=r.id AND receipt.fencing_token=r.lease_fencing_token AND receipt.environment_epoch=r.environment_epoch WHERE r.id=$1 AND r.lease_fencing_token=$2 AND r.environment_epoch=$3 AND r.purpose='communication' AND c.attempt_number=r.attempt_number AND m.id=$4 AND (receipt.kind='answered' OR (m.requirement<>'answered' AND receipt.kind='acknowledged')))")
            .bind(scope.run_id).bind(crate::u64_to_i64(scope.fencing_token,"receipt.fence")?).bind(crate::u64_to_i64(scope.environment_epoch,"receipt.epoch")?).bind(source)
            .fetch_one(&mut *self.transaction).await?)
    }
    /// Caller retains the Project lock through credential pinning and Run commit.
    pub async fn claim_communication(
        &mut self,
        project_id: ProjectId,
    ) -> Result<Option<CommunicationClaim>, StorageError> {
        let enabled: Option<bool> =
            sqlx::query_scalar("SELECT execution_enabled FROM projects WHERE id=$1 FOR UPDATE")
                .bind(project_id.as_uuid())
                .fetch_optional(&mut *self.transaction)
                .await?;
        if enabled != Some(true) || !self.lock_run_admission(project_id, None).await? {
            return Ok(None);
        }
        let row = sqlx::query("SELECT c.id,c.employee_id,c.thread_id,c.source_message_id,c.attempt_number FROM communication_assignments c JOIN employees e ON e.id=c.employee_id AND e.project_id=c.project_id WHERE c.project_id=$1 AND EXISTS(SELECT 1 FROM employee_onboarding o WHERE o.project_id=c.project_id AND o.employee_id=c.employee_id AND o.state IN ('completed','skipped','legacy_bypass')) AND c.state='queued' AND e.employee_state='active' AND (c.retry_authorized OR NOT EXISTS(SELECT 1 FROM project_recovery_settings p WHERE p.project_id=c.project_id AND p.hold)) AND EXISTS(SELECT 1 FROM employee_runtime_bindings b WHERE b.employee_id=e.id AND b.project_id=c.project_id AND forge_admission_available(c.project_id,b.binding->'execution_profile')) AND NOT EXISTS(SELECT 1 FROM communication_assignments prior WHERE prior.thread_id=c.thread_id AND prior.state='leased') AND NOT EXISTS(SELECT 1 FROM run_environment_reservations r JOIN communication_assignments held ON held.id=r.communication_assignment_id WHERE held.thread_id=c.thread_id AND r.released_at IS NULL) AND (SELECT count(*) FROM (SELECT l.id FROM leases l WHERE l.employee_id=e.id AND l.lease_state='active' UNION SELECT r.lease_id FROM runs r JOIN run_environment_reservations physical ON physical.run_id=r.id WHERE physical.employee_id=e.id AND physical.released_at IS NULL) occupied)<e.max_concurrent_runs ORDER BY c.created_at,c.id FOR UPDATE OF c,e SKIP LOCKED LIMIT 1")
            .bind(project_id.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let employee_id = EmployeeId::from(row.try_get::<Uuid, _>("employee_id")?);
        if !self.lock_employee_capacity(project_id, employee_id).await? {
            return Ok(None);
        }
        let id: Uuid = row.try_get("id")?;
        let source_message_id = row.try_get("source_message_id")?;
        let source =
            self.load_employee_message(source_message_id)
                .await?
                .ok_or(StorageError::NotFound {
                    aggregate: "communication source",
                })?;
        let attempt = row
            .try_get::<i32, _>("attempt_number")?
            .checked_add(1)
            .ok_or(StorageError::IntegerOutOfRange {
                field: "communication.attempt",
            })?;
        sqlx::query("UPDATE communication_assignments SET state='leased',retry_authorized=FALSE,attempt_number=$2,updated_at=clock_timestamp() WHERE id=$1 AND state='queued'")
            .bind(id).bind(attempt).execute(&mut *self.transaction).await?;
        Ok(Some(CommunicationClaim {
            owner: CommunicationAssignmentRef {
                assignment_id: id,
                thread_id: row.try_get("thread_id")?,
                source_message_id,
            },
            project_id,
            employee_id,
            source,
            attempt_number: attempt as u32,
        }))
    }

    /// Validates exact admission and pins a v3 taskless Run before external effects.
    pub async fn create_communication_run(
        &mut self,
        claim: &CommunicationClaim,
        spec: &CommunicationRunSpec,
        context: &CommunicationContext,
        lease_id: Uuid,
        expires_at: Timestamp,
    ) -> Result<RunProjection, StorageError> {
        spec.validate()
            .map_err(|_| invalid("invalid Communication RunSpec"))?;
        if spec.assignment != claim.owner
            || spec.project_id != claim.project_id
            || context.data().assignment != claim.owner
            || context.data().run_id != spec.run_id
            || context.data().employee_id != claim.employee_id
            || context.data().project_id != claim.project_id
            || context.data().source_message != claim.source
            || lease_id.get_version_num() != 7
        {
            return Err(invalid("Communication owner differs from admission"));
        }
        let gate: Option<bool> =
            sqlx::query_scalar("SELECT execution_enabled FROM projects WHERE id=$1 FOR UPDATE")
                .bind(claim.project_id.as_uuid())
                .fetch_optional(&mut *self.transaction)
                .await?;
        if gate != Some(true)
            || !self
                .onboarding_allowed(claim.project_id, claim.employee_id)
                .await?
            || !self
                .lock_employee_capacity(claim.project_id, claim.employee_id)
                .await?
        {
            return Err(invalid("Communication admission is closed"));
        }
        self.require_run_admission(
            claim.project_id,
            3,
            &serde_json::to_value(spec).map_err(|_| invalid("invalid admission spec"))?,
        )
        .await?;
        let current: Option<Uuid> = sqlx::query_scalar("SELECT id FROM communication_assignments WHERE id=$1 AND project_id=$2 AND employee_id=$3 AND thread_id=$4 AND source_message_id=$5 AND state='leased' AND attempt_number=$6 FOR UPDATE")
            .bind(claim.owner.assignment_id).bind(claim.project_id.as_uuid()).bind(claim.employee_id.as_uuid())
            .bind(claim.owner.thread_id).bind(claim.owner.source_message_id).bind(i64::from(claim.attempt_number))
            .fetch_optional(&mut *self.transaction).await?;
        if current.is_none() {
            return Err(invalid("Communication admission is stale"));
        }
        let lease = sqlx::query("INSERT INTO leases(id,project_id,employee_id,purpose,communication_assignment_id,lease_scope,expires_at) VALUES($1,$2,$3,'communication',$4,$5::jsonb,$6) RETURNING fencing_token,environment_epoch")
            .bind(lease_id).bind(claim.project_id.as_uuid()).bind(claim.employee_id.as_uuid()).bind(claim.owner.assignment_id)
            .bind(serde_json::json!({"assignment":claim.owner})).bind(database_timestamp(expires_at))
            .fetch_one(&mut *self.transaction).await?;
        let fence: i64 = lease.try_get("fencing_token")?;
        let epoch: i64 = lease.try_get("environment_epoch")?;
        sqlx::query("INSERT INTO runs(id,project_id,employee_id,purpose,communication_assignment_id,lease_id,attempt_number,lease_fencing_token,environment_epoch,run_spec_version,run_spec,context_manifest) VALUES($1,$2,$3,'communication',$4,$5,$6,$7,$8,3,$9::jsonb,$10::jsonb)")
            .bind(spec.run_id).bind(claim.project_id.as_uuid()).bind(claim.employee_id.as_uuid()).bind(claim.owner.assignment_id)
            .bind(lease_id).bind(i64::from(claim.attempt_number)).bind(fence).bind(epoch)
            .bind(encode_object(&serde_json::to_value(spec).map_err(|_| invalid("RunSpec serialization"))?,"communication.spec")?)
            .bind(encode_object(&serde_json::to_value(context).map_err(|_| invalid("context serialization"))?,"communication.context")?)
            .execute(&mut *self.transaction).await?;
        let _ = (
            i64_to_u64(fence, "run.fence")?,
            i64_to_u64(epoch, "run.epoch")?,
        );
        self.reserve_environment(spec.run_id, None).await?;
        self.load_run(spec.run_id)
            .await?
            .ok_or(StorageError::NotFound {
                aggregate: "communication run",
            })
    }

    /// Result acceptance is logical only. Lease and physical reservation remain held.
    pub async fn complete_communication_run(&mut self, run_id: Uuid) -> Result<bool, StorageError> {
        let result = sqlx::query("UPDATE communication_assignments c SET state='completed',completed_at=clock_timestamp(),updated_at=clock_timestamp() FROM runs r JOIN leases l ON forge_lease_owns_run(l,r) WHERE r.id=$1 AND r.purpose='communication' AND r.desired_state IN ('provision_requested','running') AND r.observed_state NOT IN ('stopped','failed','lost') AND l.lease_state='active' AND c.id=r.communication_assignment_id AND c.attempt_number=r.attempt_number AND c.state='leased'")
            .bind(run_id).execute(&mut *self.transaction).await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn hold_communication_run(&mut self, run_id: Uuid) -> Result<(), StorageError> {
        sqlx::query("UPDATE communication_assignments c SET state='held',updated_at=clock_timestamp() FROM runs r WHERE r.id=$1 AND r.purpose='communication' AND c.id=r.communication_assignment_id AND c.attempt_number=r.attempt_number AND c.state='leased'")
            .bind(run_id).execute(&mut *self.transaction).await?;
        Ok(())
    }

    pub async fn communication_run_is_complete(
        &mut self,
        run_id: Uuid,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM communication_assignments c JOIN runs r ON r.communication_assignment_id=c.id WHERE r.id=$1 AND r.purpose='communication' AND c.attempt_number=r.attempt_number AND c.state='completed')")
            .bind(run_id).fetch_one(&mut *self.transaction).await?)
    }

    /// Operator retry is scoped to the latest retired attempt and never frees resources.
    pub async fn retry_communication_run(
        &mut self,
        project: ProjectId,
        run_id: Uuid,
    ) -> Result<bool, StorageError> {
        let result=sqlx::query("UPDATE communication_assignments c SET state='queued',retry_authorized=TRUE,updated_at=clock_timestamp() FROM runs r JOIN leases l ON forge_lease_owns_run(l,r) JOIN run_environment_reservations e ON e.run_id=r.id WHERE r.id=$1 AND r.project_id=$2 AND r.purpose='communication' AND l.lease_state<>'active' AND e.state='quiescent' AND e.released_at IS NOT NULL AND c.id=r.communication_assignment_id AND c.attempt_number=r.attempt_number AND c.state='held' AND NOT EXISTS(SELECT 1 FROM run_environment_reservations other WHERE other.communication_assignment_id=c.id AND other.released_at IS NULL) AND NOT EXISTS(SELECT 1 FROM escalations question WHERE question.source_run_id=r.id AND question.escalation_state<>'resolved')")
            .bind(run_id).bind(project.as_uuid()).execute(&mut *self.transaction).await?;
        Ok(result.rows_affected() == 1)
    }
}

fn invalid(reason: &str) -> StorageError {
    StorageError::InvalidInput {
        reason: reason.into(),
    }
}
