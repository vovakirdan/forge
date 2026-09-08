//! Shared admission counts only canonical ownership; no second capacity ledger.
use crate::{PostgresStore, StorageError, StorageTransaction};
use forge_domain::{ExecutionProfile, ProjectId, admission::AdmissionLimits};
use serde_json::Value;
use sqlx::Row;

impl PostgresStore {
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
            2..=4 => Some(
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
