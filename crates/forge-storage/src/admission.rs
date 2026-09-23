//! Shared admission counts only canonical ownership; no second capacity ledger.
use crate::{PostgresStore, StorageError, StorageTransaction};
use forge_domain::{ExecutionProfile, ProjectId, admission::AdmissionLimits};
use serde_json::Value;
use sqlx::Row;
use time::OffsetDateTime;

/// Configured local admission caps and occupied Runs observed in one SQL statement.
#[derive(Clone, Copy, Debug)]
pub struct AdmissionResourceSnapshot {
    pub policy_revision: u64,
    pub limits: AdmissionLimits,
    pub policy_updated_at: OffsetDateTime,
    pub observed_at: OffsetDateTime,
    pub host_occupied_runs: u64,
    pub project_occupied_runs: u64,
}

impl PostgresStore {
    /// Counts the canonical occupied Run view; Lease plus physical reservation
    /// for the same Run remain one unit until physical quiescence is confirmed.
    pub async fn admission_resource_snapshot(
        &self,
        project_id: ProjectId,
    ) -> Result<AdmissionResourceSnapshot, StorageError> {
        let row = sqlx::query(
            "SELECT revision,host_max_runs,project_max_runs,credential_account_max_runs,updated_at, \
             clock_timestamp() AS observed_at, \
             (SELECT count(*) FROM occupied_execution_runs) AS host_occupied_runs, \
             (SELECT count(*) FROM occupied_execution_runs WHERE project_id=$1) AS project_occupied_runs \
             FROM local_admission_policy WHERE singleton",
        )
        .bind(project_id.as_uuid())
        .fetch_one(&self.pool)
        .await?;
        let read_u64 = |name| -> Result<u64, StorageError> {
            u64::try_from(row.try_get::<i64, _>(name)?)
                .map_err(|_| invalid("invalid persisted admission count or revision"))
        };
        let read_u16 = |name| -> Result<u16, StorageError> {
            u16::try_from(row.try_get::<i32, _>(name)?)
                .map_err(|_| invalid("invalid persisted admission limit"))
        };
        Ok(AdmissionResourceSnapshot {
            policy_revision: read_u64("revision")?,
            limits: AdmissionLimits {
                host_max_runs: read_u16("host_max_runs")?,
                project_max_runs: read_u16("project_max_runs")?,
                credential_account_max_runs: read_u16("credential_account_max_runs")?,
            },
            policy_updated_at: row.try_get("updated_at")?,
            observed_at: row.try_get("observed_at")?,
            host_occupied_runs: read_u64("host_occupied_runs")?,
            project_occupied_runs: read_u64("project_occupied_runs")?,
        })
    }

    /// Operator startup configuration. Different limits require full quiescence;
    /// an identical configuration is safe during recovery or a concurrent startup.
    pub async fn configure_local_admission(
        &self,
        limits: AdmissionLimits,
    ) -> Result<(), StorageError> {
        if !limits.is_valid() {
            return Err(invalid("admission limits must be positive"));
        }
        let mut tx = self.begin().await?;
        let prior = tx.lock_admission_policy().await?;
        if prior != limits {
            let occupied: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM occupied_execution_runs)")
                    .fetch_one(&mut *tx.transaction)
                    .await?;
            if occupied {
                return Err(invalid(
                    "different admission configuration requires all Runs to be physically quiescent",
                ));
            }
            sqlx::query("UPDATE local_admission_policy SET revision=revision+1,host_max_runs=$1,project_max_runs=$2,credential_account_max_runs=$3,updated_at=clock_timestamp() WHERE singleton")
                .bind(i32::from(limits.host_max_runs)).bind(i32::from(limits.project_max_runs))
                .bind(i32::from(limits.credential_account_max_runs)).execute(&mut *tx.transaction).await?;
        }
        tx.commit().await
    }
}

impl StorageTransaction<'_> {
    async fn lock_admission_policy(&mut self) -> Result<AdmissionLimits, StorageError> {
        let row = sqlx::query("SELECT host_max_runs,project_max_runs,credential_account_max_runs FROM local_admission_policy WHERE singleton FOR UPDATE")
            .fetch_one(&mut *self.transaction).await?;
        let read = |name| -> Result<u16, StorageError> {
            u16::try_from(row.try_get::<i32, _>(name)?)
                .map_err(|_| invalid("invalid persisted admission limit"))
        };
        Ok(AdmissionLimits {
            host_max_runs: read("host_max_runs")?,
            project_max_runs: read("project_max_runs")?,
            credential_account_max_runs: read("credential_account_max_runs")?,
        })
    }

    /// Retain this lock through admission commit. None checks host/Project only
    /// (provider-free Hook or historical fake Run), not an unlimited account.
    pub async fn lock_run_admission(
        &mut self,
        project: ProjectId,
        profile: Option<&ExecutionProfile>,
    ) -> Result<bool, StorageError> {
        self.lock_admission_policy().await?;
        let profile = profile
            .map(|profile| {
                serde_json::to_value(profile).map_err(|_| invalid("invalid admission profile"))
            })
            .transpose()?;
        Ok(
            sqlx::query_scalar("SELECT forge_admission_available($1,$2)")
                .bind(project.as_uuid())
                .bind(profile)
                .fetch_one(&mut *self.transaction)
                .await?,
        )
    }

    pub(crate) async fn require_run_admission(
        &mut self,
        project: ProjectId,
        version: u16,
        spec: &Value,
    ) -> Result<(), StorageError> {
        let profile = match version {
            1 | 5 => None,
            2..=4 | 6..=7 => Some(
                serde_json::from_value::<ExecutionProfile>(
                    spec["binding"]["execution_profile"].clone(),
                )
                .map_err(|_| invalid("admission requires a valid pinned execution profile"))?,
            ),
            _ => return Err(invalid("unsupported admission RunSpec")),
        };
        if self.lock_run_admission(project, profile.as_ref()).await? {
            Ok(())
        } else {
            Err(StorageError::StaleRevision {
                aggregate: "shared Run capacity",
            })
        }
    }
}

fn invalid(reason: &str) -> StorageError {
    StorageError::InvalidInput {
        reason: reason.into(),
    }
}
