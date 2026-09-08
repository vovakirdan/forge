//! Mechanical clocks, boot identity and recovery decisions; policy belongs to Core.

use crate::{
    PostgresStore, StorageError, StorageTransaction, database_timestamp, domain_timestamp,
    enum_text,
};
use forge_domain::{
    CommandId, ProjectId, Timestamp,
    runtime::{BootRecoveryPolicy, RecoveryAssessment},
};
use sqlx::Row;
use uuid::Uuid;

/// Canonical timestamps and physical identity used for watchdog evaluation.
#[derive(Clone, Debug)]
pub struct RunRecoveryState {
    /// Run identity.
    pub run_id: Uuid,
    /// Owner for Project-first locking.
    pub project_id: ProjectId,
    /// Canonical issue time, never provider-reported time.
    pub created_at: Timestamp,
    /// First accepted running observation.
    pub started_at: Option<Timestamp>,
    /// Last accepted liveness evidence, independent of progress.
    pub liveness_observed_at: Option<Timestamp>,
    /// First desired stop time; redelivery does not extend grace.
    pub stop_requested_at: Option<Timestamp>,
    /// Host that owns the physical reservation.
    pub host_id: Option<String>,
    /// Physical boot generation.
    pub boot_id: Option<String>,
    /// Whether the reservation still prohibits replacement writers.
    pub reserved: bool,
    /// Whether executor write authority is still current.
    pub lease_active: bool,
}

const RECOVERY_COLUMNS: &str = "r.id,r.project_id,r.created_at,r.started_at,r.liveness_observed_at,r.stop_requested_at,e.host_id,e.boot_id,(e.released_at IS NULL) AS reserved,(l.lease_state='active') AS lease_active";

fn recovery_state(row: sqlx::postgres::PgRow) -> Result<RunRecoveryState, StorageError> {
    Ok(RunRecoveryState {
        run_id: row.try_get("id")?,
        project_id: ProjectId::from(row.try_get::<Uuid, _>("project_id")?),
        created_at: domain_timestamp(row.try_get("created_at")?),
        started_at: row
            .try_get::<Option<time::OffsetDateTime>, _>("started_at")?
            .map(domain_timestamp),
        liveness_observed_at: row
            .try_get::<Option<time::OffsetDateTime>, _>("liveness_observed_at")?
            .map(domain_timestamp),
        stop_requested_at: row
            .try_get::<Option<time::OffsetDateTime>, _>("stop_requested_at")?
            .map(domain_timestamp),
        host_id: row.try_get("host_id")?,
        boot_id: row.try_get("boot_id")?,
        reserved: row.try_get("reserved")?,
        lease_active: row.try_get("lease_active")?,
    })
}

impl PostgresStore {
    /// Retryable proxy cleanup remains discoverable after physical reservation release.
    pub async fn pending_retired_proxy_revocations(&self) -> Result<Vec<Uuid>, StorageError> {
        Ok(sqlx::query_scalar("SELECT k.run_id FROM run_proxy_keys k JOIN runs r ON r.id=k.run_id JOIN leases l ON l.id=r.lease_id WHERE (k.revoked_at IS NULL OR k.issuance_pending) AND k.next_revocation_attempt_at<=clock_timestamp() AND (l.lease_state<>'active' OR r.desired_state IN ('stop_requested','force_stop_requested','stopped','failed'))")
            .fetch_all(&self.pool).await?)
    }
    /// Lists sandbox executions of every supported purpose while physical
    /// existence remains unresolved. No Employee or provider is required.
    pub async fn list_recovery_runs(&self) -> Result<Vec<RunRecoveryState>, StorageError> {
        let query = format!(
            "SELECT {RECOVERY_COLUMNS} FROM runs r JOIN leases l ON l.id=r.lease_id JOIN run_environment_reservations e ON e.run_id=r.id WHERE r.run_spec_version IN (2,3,4,5) AND e.released_at IS NULL ORDER BY r.created_at,r.id"
        );
        sqlx::query(&query)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(recovery_state)
            .collect()
    }

    /// Last fully reconciled local host generation.
    pub async fn execution_host(&self) -> Result<Option<(String, String)>, StorageError> {
        Ok(
            sqlx::query_as("SELECT host_id,boot_id FROM local_execution_host WHERE singleton")
                .fetch_optional(&self.pool)
                .await?,
        )
    }
}

impl StorageTransaction<'_> {
    /// One Project boot reconciliation is atomic with its gate, incidents and audit.
    pub async fn begin_project_boot_reconciliation(
        &mut self,
        project_id: ProjectId,
        host: &str,
        boot: &str,
    ) -> Result<bool, StorageError> {
        let result=sqlx::query("INSERT INTO project_boot_reconciliations(project_id,host_id,boot_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(project_id.as_uuid()).bind(host).bind(boot).execute(&mut *self.transaction).await?;
        Ok(result.rows_affected() == 1)
    }
    /// Structural contradiction check; not a semantic evaluator.
    pub async fn run_has_result_evidence(&mut self, run_id: Uuid) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artifacts WHERE run_id=$1) OR EXISTS(SELECT 1 FROM task_handoffs WHERE run_id=$1 AND kind='accepted') OR EXISTS(SELECT 1 FROM employee_message_receipts WHERE run_id=$1 AND kind IN ('acknowledged','answered'))")
            .bind(run_id).fetch_one(&mut *self.transaction).await?)
    }
    /// Rechecks watchdog timestamps after Core acquires the Project lock.
    pub async fn run_recovery_state(
        &mut self,
        run_id: Uuid,
    ) -> Result<Option<RunRecoveryState>, StorageError> {
        let query = format!(
            "SELECT {RECOVERY_COLUMNS} FROM runs r JOIN leases l ON l.id=r.lease_id JOIN run_environment_reservations e ON e.run_id=r.id WHERE r.id=$1 FOR UPDATE OF r,l,e"
        );
        sqlx::query(&query)
            .bind(run_id)
            .fetch_optional(&mut *self.transaction)
            .await?
            .map(recovery_state)
            .transpose()
    }

    /// Call only for accepted heartbeat/running observations, never progress text.
    pub async fn record_run_liveness(
        &mut self,
        run_id: Uuid,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE runs SET liveness_observed_at=GREATEST(liveness_observed_at,$2) WHERE id=$1",
        )
        .bind(run_id)
        .bind(database_timestamp(now))
        .execute(&mut *self.transaction)
        .await?;
        Ok(())
    }

    /// Revokes logical authority without freeing uncertain physical ownership.
    pub async fn quarantine_run(
        &mut self,
        run_id: Uuid,
        reason: &str,
        quiescent: bool,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE queue_entries q SET queue_state='cancelled',cancelled_at=clock_timestamp() FROM runs r WHERE r.id=$1 AND q.id=r.queue_entry_id AND q.queue_state='leased'")
            .bind(run_id).execute(&mut *self.transaction).await?;
        self.hold_communication_run(run_id).await?;
        sqlx::query("UPDATE leases l SET lease_state='revoked',revoked_at=clock_timestamp(),revocation_reason=$2,revision=l.revision+1 FROM runs r WHERE r.id=$1 AND l.id=r.lease_id AND l.lease_state='active'")
            .bind(run_id).bind(reason).execute(&mut *self.transaction).await?;
        sqlx::query("UPDATE run_environment_reservations SET state=CASE WHEN $2 THEN 'quiescent' ELSE 'unknown' END,released_at=CASE WHEN $2 THEN clock_timestamp() ELSE released_at END WHERE run_id=$1 AND released_at IS NULL")
            .bind(run_id).bind(quiescent).execute(&mut *self.transaction).await?;
        Ok(())
    }

    /// Marks the boot ledger only after Core finishes every affected Project.
    pub async fn record_execution_host(
        &mut self,
        host_id: &str,
        boot_id: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO local_execution_host(singleton,host_id,boot_id) VALUES(TRUE,$1,$2) ON CONFLICT(singleton) DO UPDATE SET host_id=EXCLUDED.host_id,boot_id=EXCLUDED.boot_id,reconciled_at=clock_timestamp()")
            .bind(host_id).bind(boot_id).execute(&mut *self.transaction).await?;
        Ok(())
    }

    /// Durable per-Run inventory dedupe shares the observation transaction.
    pub async fn record_inventory_receipt(
        &mut self,
        message_id: Uuid,
        run_id: Uuid,
    ) -> Result<bool, StorageError> {
        let result=sqlx::query("INSERT INTO run_inventory_receipts(message_id,run_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(message_id).bind(run_id).execute(&mut *self.transaction).await?;
        Ok(result.rows_affected() == 1)
    }

    /// Returns the explicit Project policy or the conservative product default.
    pub async fn boot_recovery_policy(
        &mut self,
        project_id: ProjectId,
    ) -> Result<BootRecoveryPolicy, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT boot_policy FROM project_recovery_settings WHERE project_id=$1",
        )
        .bind(project_id.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        value
            .map(|value| {
                serde_json::from_value(serde_json::Value::String(value)).map_err(|source| {
                    StorageError::Snapshot {
                        aggregate: "boot_policy",
                        source,
                    }
                })
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }

    /// Called by the audited management command, never Supervisor policy.
    pub async fn configure_boot_recovery_policy(
        &mut self,
        project_id: ProjectId,
        policy: BootRecoveryPolicy,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO project_recovery_settings(project_id,boot_policy) VALUES($1,$2) ON CONFLICT(project_id) DO UPDATE SET boot_policy=EXCLUDED.boot_policy")
            .bind(project_id.as_uuid()).bind(enum_text(&policy,"boot_policy")?).execute(&mut *self.transaction).await?;
        Ok(())
    }

    /// A separate dispatch hold never pretends the human breaker was opened.
    pub async fn set_recovery_hold(
        &mut self,
        project_id: ProjectId,
        hold: bool,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO project_recovery_settings(project_id,hold) VALUES($1,$2) ON CONFLICT(project_id) DO UPDATE SET hold=EXCLUDED.hold")
            .bind(project_id.as_uuid()).bind(hold).execute(&mut *self.transaction).await?;
        Ok(())
    }

    /// Records a single accepted assessment, not an untrusted evaluator candidate.
    pub async fn accept_recovery_assessment(
        &mut self,
        run_id: Uuid,
        command_id: CommandId,
        assessment: RecoveryAssessment,
        queue_entry_id: Option<Uuid>,
    ) -> Result<bool, StorageError> {
        let result=sqlx::query("INSERT INTO run_recovery_decisions(run_id,assessment,accepted_command_id,recovery_queue_entry_id) VALUES($1,$2,$3,$4) ON CONFLICT(run_id) DO NOTHING")
            .bind(run_id).bind(enum_text(&assessment,"recovery_assessment")?).bind(command_id.as_uuid()).bind(queue_entry_id)
            .execute(&mut *self.transaction).await?;
        Ok(result.rows_affected() == 1)
    }
}
