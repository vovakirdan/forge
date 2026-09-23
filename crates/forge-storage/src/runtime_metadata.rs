//! Scoped runtime assignment read for safe operator metadata projections.

use forge_domain::{EmployeeId, ProjectId, runtime::RuntimeBinding};
use sqlx::Row;
use time::OffsetDateTime;

use crate::{PostgresStore, StorageError};

pub struct RuntimeBindingRecord {
    pub binding: RuntimeBinding,
    pub revision: u64,
    pub updated_at: OffsetDateTime,
}

impl PostgresStore {
    pub async fn employee_runtime_binding_record(
        &self,
        project_id: ProjectId,
        employee_id: EmployeeId,
    ) -> Result<Option<RuntimeBindingRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT binding::text AS binding,revision,updated_at FROM employee_runtime_bindings \
             WHERE project_id=$1 AND employee_id=$2",
        )
        .bind(project_id.as_uuid())
        .bind(employee_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let raw: String = row.try_get("binding")?;
            let binding: RuntimeBinding =
                serde_json::from_str(&raw).map_err(|source| StorageError::Snapshot {
                    aggregate: "employee_runtime_binding",
                    source,
                })?;
            binding.validate().map_err(|_| StorageError::InvalidInput {
                reason: "invalid stored runtime binding".into(),
            })?;
            let revision: i64 = row.try_get("revision")?;
            let revision = u64::try_from(revision).map_err(|_| StorageError::InvalidInput {
                reason: "invalid runtime binding revision".into(),
            })?;
            Ok(RuntimeBindingRecord {
                binding,
                revision,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .transpose()
    }
}
