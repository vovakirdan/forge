//! Mechanical persistence for physical reservations and runtime incidents.

use forge_domain::{
    EmployeeId, ProjectId, TaskId,
    runtime::{RecoveryAssessment, RunScope, RuntimeBinding},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    PostgresStore, StorageError, StorageTransaction, encode_object, encode_snapshot, enum_text,
    u64_to_i64,
};

/// Safe classifications, never raw provider exception or auth material.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentKind {
    /// No positive Runner liveness arrived before its deadline.
    HeartbeatLost,
    /// The host rebooted; pre-boot execution cannot continue with its old Lease.
    HostReboot,
    /// Runtime existence or side effects cannot be determined.
    EnvironmentUnknown,
    /// No terminal pipeline outcome was accepted before the process exited.
    Interrupted,
    /// Provider credentials require operator attention.
    Authentication,
    /// Provider or account rate/cost limits were reached.
    BudgetExceeded,
    /// Provider rate/quota refusal, not proof of monetary budget exhaustion.
    RateLimited,
    /// Provider transport/service unavailable.
    ProviderUnavailable,
    /// A child runtime reported a failure without a more specific classification.
    RuntimeFailed,
    /// The bounded local evidence buffer could not retain all bytes.
    EvidenceIncomplete,
    /// An external process could not be prepared or started.
    StartFailed,
}

/// Positive environment observation separate from the logical Run lifecycle.
#[derive(Clone, Debug)]
pub struct EnvironmentReport {
    /// Run/fence/epoch the environment was created for.
    pub scope: RunScope,
    /// Stable local host identity.
    pub host_id: String,
    /// Host boot generation reported by the Supervisor.
    pub boot_id: String,
    /// Container ID or equivalent runtime identity.
    pub environment_id: String,
    /// Only positive exit/nonexistence evidence permits releasing a writer.
    pub quiescent: bool,
    /// Inability to establish liveness keeps the reservation quarantined.
    pub unknown: bool,
}

impl StorageTransaction<'_> {
    /// Handoff existence is Run-local; an earlier stage result is not this Run's result.
    pub async fn run_has_handoff(&mut self, run_id: Uuid) -> Result<bool, StorageError> {
        Ok(
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_handoffs WHERE run_id=$1)")
                .bind(run_id)
                .fetch_one(&mut *self.transaction)
                .await?,
        )
    }
    /// Reads the operator-owned assignment while the Project lock is held.
    pub async fn runtime_binding(
        &mut self,
        employee_id: EmployeeId,
    ) -> Result<Option<RuntimeBinding>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT binding::text FROM employee_runtime_bindings WHERE employee_id = $1",
        )
        .bind(employee_id.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        value
            .map(|value| {
                let binding: RuntimeBinding = decode_payload(&value, "runtime binding")?;
                binding.validate().map_err(|_| StorageError::InvalidInput {
                    reason: "invalid stored runtime binding".into(),
                })?;
                Ok(binding)
            })
            .transpose()
    }

    /// Binds a validated profile under the enclosing named-command transaction.
    pub async fn bind_employee_runtime(
        &mut self,
        project_id: ProjectId,
        employee_id: EmployeeId,
        binding: &RuntimeBinding,
    ) -> Result<(), StorageError> {
        binding.validate().map_err(|_| StorageError::InvalidInput {
            reason: "invalid runtime binding".into(),
        })?;
        if binding.execution_profile.project_id() != project_id {
            return Err(StorageError::InvalidInput {
                reason: "runtime profile project mismatch".into(),
            });
        }
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM employees WHERE id=$1 AND project_id=$2)",
        )
        .bind(employee_id.as_uuid())
        .bind(project_id.as_uuid())
        .fetch_one(&mut *self.transaction)
        .await?;
        if !valid {
            return Err(StorageError::NotFound {
                aggregate: "employee",
            });
        }
        let profile = &binding.execution_profile;
        let snapshot = encode_snapshot(profile, "execution profile")?;
        sqlx::query("INSERT INTO execution_profile_versions(profile_id,revision,project_id,snapshot) VALUES($1,$2,$3,$4::jsonb) ON CONFLICT DO NOTHING")
            .bind(profile.id()).bind(u64_to_i64(profile.revision(),"profile.revision")?).bind(project_id.as_uuid()).bind(&snapshot).execute(&mut *self.transaction).await?;
        let identical:bool=sqlx::query_scalar("SELECT project_id=$3 AND snapshot=$4::jsonb FROM execution_profile_versions WHERE profile_id=$1 AND revision=$2")
            .bind(profile.id()).bind(u64_to_i64(profile.revision(),"profile.revision")?).bind(project_id.as_uuid()).bind(snapshot).fetch_one(&mut *self.transaction).await?;
        if !identical {
            return Err(StorageError::InvalidInput {
                reason: "execution profile revision is immutable; publish a new revision".into(),
            });
        }
        sqlx::query("INSERT INTO employee_runtime_bindings(employee_id,project_id,binding) VALUES($1,$2,$3::jsonb) ON CONFLICT(employee_id) DO UPDATE SET binding=EXCLUDED.binding,revision=employee_runtime_bindings.revision+1,updated_at=clock_timestamp()")
            .bind(employee_id.as_uuid()).bind(project_id.as_uuid()).bind(encode_snapshot(binding, "runtime binding")?)
            .execute(&mut *self.transaction).await?;
        Ok(())
    }

    /// Adds physical ownership in the same transaction as the logical Lease.
    pub async fn reserve_environment(
        &mut self,
        run_id: Uuid,
        surface_id: Option<Uuid>,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO run_environment_reservations(run_id,project_id,task_id,employee_id,surface_id,fencing_token,environment_epoch) SELECT id,project_id,task_id,employee_id,$2,lease_fencing_token,environment_epoch FROM runs WHERE id=$1")
            .bind(run_id).bind(surface_id).execute(&mut *self.transaction).await?;
        Ok(())
    }

    /// Updates an exact physical reservation; uncertainty never unlocks it.
    pub async fn record_environment_report(
        &mut self,
        report: &EnvironmentReport,
    ) -> Result<bool, StorageError> {
        let state = if report.unknown {
            "unknown"
        } else if report.quiescent {
            "quiescent"
        } else {
            "active"
        };
        let result = sqlx::query("UPDATE run_environment_reservations SET state=$4,host_id=$5,boot_id=$6,environment_id=$7,observed_at=clock_timestamp(),released_at=CASE WHEN $4='quiescent' THEN clock_timestamp() ELSE NULL END WHERE run_id=$1 AND fencing_token=$2 AND environment_epoch=$3 AND released_at IS NULL")
            .bind(report.scope.run_id).bind(u64_to_i64(report.scope.fencing_token,"reservation.fence")?)
            .bind(u64_to_i64(report.scope.environment_epoch,"reservation.epoch")?).bind(state)
            .bind(&report.host_id).bind(&report.boot_id).bind(&report.environment_id)
            .execute(&mut *self.transaction).await?;
        Ok(result.rows_affected() == 1)
    }

    /// Creates one deduplicated incident. Caller must append its event/outbox.
    pub async fn record_run_incident(
        &mut self,
        project_id: ProjectId,
        run_id: Uuid,
        kind: IncidentKind,
        assessment: RecoveryAssessment,
    ) -> Result<(Uuid, bool), StorageError> {
        let id = Uuid::now_v7();
        let kind = enum_text(&kind, "incident.kind")?;
        let result = sqlx::query("INSERT INTO run_incidents(id,project_id,run_id,kind,assessment) VALUES($1,$2,$3,$4,$5) ON CONFLICT(run_id,kind) DO NOTHING")
            .bind(id).bind(project_id.as_uuid()).bind(run_id).bind(&kind).bind(enum_text(&assessment,"incident.assessment")?)
            .execute(&mut *self.transaction).await?;
        let inserted = result.rows_affected() == 1;
        let id = sqlx::query_scalar("SELECT id FROM run_incidents WHERE run_id=$1 AND kind=$2")
            .bind(run_id)
            .bind(kind)
            .fetch_one(&mut *self.transaction)
            .await?;
        Ok((id, inserted))
    }

    /// Inserts immutable handoff once; an accepted result cannot be overwritten.
    pub async fn insert_run_handoff(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
        run_id: Uuid,
        accepted: bool,
        body: &Value,
    ) -> Result<bool, StorageError> {
        let result=sqlx::query("INSERT INTO task_handoffs(run_id,project_id,task_id,kind,body) VALUES($1,$2,$3,$4,$5::jsonb) ON CONFLICT(run_id) DO NOTHING")
            .bind(run_id).bind(project_id.as_uuid()).bind(task_id.as_uuid())
            .bind(if accepted {"accepted"} else {"interrupted"}).bind(encode_object(body,"handoff.body")?)
            .execute(&mut *self.transaction).await?;
        Ok(result.rows_affected() == 1)
    }

    /// Reads the previous canonical handoff without waiting for derived summaries.
    pub async fn latest_handoff(&mut self, task_id: TaskId) -> Result<Option<Value>, StorageError> {
        let value:Option<String>=sqlx::query_scalar("SELECT body::text FROM task_handoffs WHERE task_id=$1 ORDER BY created_at DESC,run_id DESC LIMIT 1")
            .bind(task_id.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        value
            .map(|value| decode_payload(&value, "handoff"))
            .transpose()
    }
}

impl PostgresStore {
    /// Read-only operator view; no raw provider diagnostics or secret fields.
    pub async fn list_run_incidents(&self, project_id: ProjectId) -> Result<Value, StorageError> {
        let text:String=sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(i) ORDER BY i.created_at),'[]'::jsonb)::text FROM run_incidents i WHERE project_id=$1")
            .bind(project_id.as_uuid()).fetch_one(&self.pool).await?;
        decode_payload(&text, "incidents")
    }
}

fn decode_payload<T: serde::de::DeserializeOwned>(
    value: &str,
    aggregate: &'static str,
) -> Result<T, StorageError> {
    serde_json::from_str(value).map_err(|source| StorageError::Snapshot { aggregate, source })
}
