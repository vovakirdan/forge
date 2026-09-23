//! Read-only projections used by Core-owned local transports.

use forge_domain::{
    EmployeeId, Pipeline, PipelineId, PipelineVersion, PipelineVersionId, Project, ProjectId,
    TaskId,
};
use sqlx::Row;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::store::stored_artifact_from_row;
use crate::{
    PostgresStore, RunProjection, StorageError, StoredArtifact, decode_snapshot,
    run_projection_from_row,
};

const RUN_COLUMNS: &str = "id, project_id, purpose, communication_assignment_id, resolution_assignment_id, hook_invocation_id, task_id, queue_entry_id, lease_id, employee_id, stage_id, attempt_number, lease_fencing_token, environment_epoch, last_sequence, desired_state, observed_state, run_spec_version, run_spec::text AS run_spec, context_manifest::text AS context_manifest, observed_details::text AS observed_details";

/// Current physical occupancy and positive Run observation for one Employee.
/// This is an observation, not a prediction of dispatch or provider readiness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmployeeOperationalCounts {
    /// Unique Lease slots held by active leases or unreleased environments.
    pub occupied_slots: u64,
    /// Runs whose latest Supervisor observation is running.
    pub observed_running_runs: u64,
    /// Whether an operator-owned runtime binding exists; its contents stay private.
    pub runtime_binding_configured: bool,
}

/// Bounded catalog metadata; immutable version definitions stay on detail reads.
#[derive(Clone, Debug)]
pub struct PipelineCatalogItem {
    pub id: PipelineId,
    pub name: String,
    pub revision: u64,
    pub default_version_id: Option<PipelineVersionId>,
    pub latest_version_id: Option<PipelineVersionId>,
    pub latest_version: Option<u32>,
    pub deleted_at: Option<OffsetDateTime>,
    pub pinned_task_count: u64,
}

impl PostgresStore {
    /// Bounded version page in the historical name/version order. `None` means
    /// the supplied cursor is not retained in this Project.
    pub async fn list_pipeline_version_page(
        &self,
        project_id: ProjectId,
        after: Option<PipelineVersionId>,
        limit: u32,
    ) -> Result<Option<Vec<(Pipeline, PipelineVersion)>>, StorageError> {
        if !(1..=101).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "pipeline version page limit must be 1..101".into(),
            });
        }
        let cursor: Option<(String, i64, Uuid)> = if let Some(after) = after {
            let value = sqlx::query_as("SELECT p.name,v.version,p.id FROM pipeline_versions v JOIN pipelines p ON p.id=v.pipeline_id WHERE p.project_id=$1 AND v.id=$2")
                .bind(project_id.as_uuid()).bind(after.as_uuid()).fetch_optional(&self.pool).await?;
            let Some(value) = value else {
                return Ok(None);
            };
            Some(value)
        } else {
            None
        };
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT p.canonical_snapshot::text,v.definition::text FROM pipeline_versions v \
             JOIN pipelines p ON p.id=v.pipeline_id WHERE p.project_id=$1 \
             AND ($2::text IS NULL OR (p.name,v.version,p.id)>($2,$3,$4)) \
             ORDER BY p.name ASC,v.version ASC,p.id ASC LIMIT $5",
        )
        .bind(project_id.as_uuid())
        .bind(cursor.as_ref().map(|v| v.0.as_str()))
        .bind(cursor.as_ref().map(|v| v.1))
        .bind(cursor.as_ref().map(|v| v.2))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(pipeline, version)| {
                let pipeline: Pipeline = decode_snapshot(&pipeline, "pipeline")?;
                let version: PipelineVersion = decode_snapshot(&version, "pipeline_version")?;
                if pipeline.project_id() != project_id
                    || version.project_id() != project_id
                    || version.pipeline_id() != pipeline.id()
                {
                    return Err(StorageError::InvalidInput {
                        reason: "pipeline version page scope mismatch".into(),
                    });
                }
                Ok((pipeline, version))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    /// Bounded newest-first Run page with a retained Project cursor.
    pub async fn list_runs_for_project_page(
        &self,
        project_id: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Option<Vec<RunProjection>>, StorageError> {
        if !(1..=101).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "Run page limit must be 1..101".into(),
            });
        }
        let cursor: Option<(OffsetDateTime, Uuid)> = if let Some(after) = after {
            let value =
                sqlx::query_as("SELECT created_at,id FROM runs WHERE project_id=$1 AND id=$2")
                    .bind(project_id.as_uuid())
                    .bind(after)
                    .fetch_optional(&self.pool)
                    .await?;
            let Some(value) = value else {
                return Ok(None);
            };
            Some(value)
        } else {
            None
        };
        let query = format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE project_id=$1 \
            AND ($2::timestamptz IS NULL OR (created_at,id)<($2,$3)) \
            ORDER BY created_at DESC,id DESC LIMIT $4"
        );
        let rows = sqlx::query(&query)
            .bind(project_id.as_uuid())
            .bind(cursor.as_ref().map(|v| v.0))
            .bind(cursor.as_ref().map(|v| v.1))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(run_projection_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    /// Reads one ID-ordered catalog page with one grouped Task usage aggregate.
    pub async fn list_pipeline_catalog_page(
        &self,
        project_id: ProjectId,
        after: Option<PipelineId>,
        limit: u32,
    ) -> Result<Vec<PipelineCatalogItem>, StorageError> {
        if !(1..=101).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "pipeline catalog page limit must be between 1 and 101".into(),
            });
        }
        let rows = sqlx::query(
            "WITH page AS MATERIALIZED ( \
               SELECT id,name,revision,default_version_id,deleted_at FROM pipelines \
               WHERE project_id=$1 AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3 \
             ), latest AS ( \
               SELECT DISTINCT ON (v.pipeline_id) v.pipeline_id,v.id,v.version \
               FROM pipeline_versions v JOIN page p ON p.id=v.pipeline_id \
               ORDER BY v.pipeline_id,v.version DESC \
             ), usage AS ( \
               SELECT t.pipeline_id,count(*) AS pinned_task_count FROM tasks t JOIN page p ON p.id=t.pipeline_id \
               WHERE t.project_id=$1 AND t.pipeline_version_id IS NOT NULL GROUP BY t.pipeline_id \
             ) \
             SELECT p.id,p.name,p.revision,p.default_version_id,p.deleted_at, \
                    latest.id AS latest_version_id,latest.version AS latest_version, \
                    COALESCE(usage.pinned_task_count,0) AS pinned_task_count \
             FROM page p LEFT JOIN latest ON latest.pipeline_id=p.id \
             LEFT JOIN usage ON usage.pipeline_id=p.id ORDER BY p.id",
        )
        .bind(project_id.as_uuid())
        .bind(after.map(|id| id.as_uuid()))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let count = |column| -> Result<u64, StorageError> {
                    u64::try_from(row.try_get::<i64, _>(column)?).map_err(|_| {
                        StorageError::InvalidInput {
                            reason: format!("invalid {column} in pipeline catalog"),
                        }
                    })
                };
                let latest_version = row
                    .try_get::<Option<i64>, _>("latest_version")?
                    .map(|value| {
                        u32::try_from(value).map_err(|_| StorageError::InvalidInput {
                            reason: "invalid latest version in pipeline catalog".into(),
                        })
                    })
                    .transpose()?;
                Ok(PipelineCatalogItem {
                    id: PipelineId::from(row.try_get::<Uuid, _>("id")?),
                    name: row.try_get("name")?,
                    revision: count("revision")?,
                    default_version_id: row
                        .try_get::<Option<Uuid>, _>("default_version_id")?
                        .map(PipelineVersionId::from),
                    latest_version_id: row
                        .try_get::<Option<Uuid>, _>("latest_version_id")?
                        .map(PipelineVersionId::from),
                    latest_version,
                    deleted_at: row.try_get("deleted_at")?,
                    pinned_task_count: count("pinned_task_count")?,
                })
            })
            .collect()
    }

    /// Reads the same physical capacity ledger used by Employee admission.
    pub async fn employee_operational_counts(
        &self,
        project_id: ProjectId,
        employee_id: EmployeeId,
    ) -> Result<EmployeeOperationalCounts, StorageError> {
        let row = sqlx::query(
            "SELECT \
             (SELECT count(*) FROM (SELECT l.id FROM leases l WHERE l.project_id=$1 AND l.employee_id=$2 AND l.lease_state='active' \
              UNION SELECT r.lease_id FROM runs r JOIN run_environment_reservations e ON e.run_id=r.id \
              WHERE e.project_id=$1 AND e.employee_id=$2 AND e.released_at IS NULL) occupied) AS occupied_slots, \
             (SELECT count(*) FROM runs WHERE project_id=$1 AND employee_id=$2 AND observed_state='running') AS observed_running_runs, \
             EXISTS(SELECT 1 FROM employee_runtime_bindings WHERE project_id=$1 AND employee_id=$2) AS runtime_binding_configured",
        )
        .bind(project_id.as_uuid())
        .bind(employee_id.as_uuid())
        .fetch_one(&self.pool)
        .await?;
        let count = |column| -> Result<u64, StorageError> {
            u64::try_from(row.try_get::<i64, _>(column)?).map_err(|_| StorageError::InvalidInput {
                reason: format!("invalid {column} count"),
            })
        };
        Ok(EmployeeOperationalCounts {
            occupied_slots: count("occupied_slots")?,
            observed_running_runs: count("observed_running_runs")?,
            runtime_binding_configured: row.try_get("runtime_binding_configured")?,
        })
    }

    /// Lists canonical Project snapshots in stable identity order for owner reads.
    pub async fn list_projects(&self) -> Result<Vec<Project>, StorageError> {
        let snapshots: Vec<String> =
            sqlx::query_scalar("SELECT canonical_snapshot::text FROM projects ORDER BY id ASC")
                .fetch_all(&self.pool)
                .await?;
        snapshots
            .into_iter()
            .map(|value| decode_snapshot(&value, "project"))
            .collect()
    }

    /// Lists all durable Project identities for recovery and stop reconciliation.
    pub async fn list_project_ids(&self) -> Result<Vec<ProjectId>, StorageError> {
        let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM projects ORDER BY id ASC")
            .fetch_all(&self.pool)
            .await?;
        Ok(ids.into_iter().map(ProjectId::from).collect())
    }

    /// Lists Projects whose durable execution gate allows scheduler reconciliation.
    pub async fn list_execution_open_project_ids(&self) -> Result<Vec<ProjectId>, StorageError> {
        let ids: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM projects WHERE execution_enabled ORDER BY id ASC")
                .fetch_all(&self.pool)
                .await?;
        Ok(ids.into_iter().map(ProjectId::from).collect())
    }

    /// Lists immutable Pipeline versions owned by one Project in stable order.
    pub async fn list_pipeline_versions(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<PipelineVersion>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar(
            "SELECT pipeline_versions.definition::text FROM pipeline_versions JOIN pipelines ON pipelines.id = pipeline_versions.pipeline_id WHERE pipelines.project_id = $1 ORDER BY pipelines.name ASC, pipeline_versions.version ASC",
        )
        .bind(project_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        values
            .into_iter()
            .map(|value| decode_snapshot(&value, "pipeline_version"))
            .collect()
    }

    /// Loads one current Run projection without granting any mutation authority.
    pub async fn load_run(&self, id: Uuid) -> Result<Option<RunProjection>, StorageError> {
        let query = format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = $1");
        let row = sqlx::query(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(run_projection_from_row).transpose()
    }

    /// Lists current Run projections for one Project in deterministic creation order.
    pub async fn list_runs_for_project(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<RunProjection>, StorageError> {
        let query = format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE project_id = $1 ORDER BY created_at DESC, id DESC"
        );
        let rows = sqlx::query(&query)
            .bind(project_id.as_uuid())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(run_projection_from_row).collect()
    }

    /// Lists retained Runs for one Employee, newest first. Hook and SystemJob
    /// executions have no Employee identity and cannot appear here.
    pub async fn list_runs_for_employee(
        &self,
        project_id: ProjectId,
        employee_id: EmployeeId,
    ) -> Result<Vec<RunProjection>, StorageError> {
        let query = format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE project_id = $1 AND employee_id = $2 ORDER BY created_at DESC, id DESC"
        );
        let rows = sqlx::query(&query)
            .bind(project_id.as_uuid())
            .bind(employee_id.as_uuid())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(run_projection_from_row).collect()
    }

    /// Lists immutable Artifact evidence directly attached to one Task.
    pub async fn list_artifacts_for_task(
        &self,
        task_id: TaskId,
    ) -> Result<Vec<StoredArtifact>, StorageError> {
        let rows = sqlx::query(
            "SELECT canonical_snapshot::text AS canonical_snapshot, task_id, run_id, stage_id, producer_type, producer_id, producer::text AS producer FROM artifacts WHERE task_id = $1 ORDER BY created_at ASC, id ASC",
        )
        .bind(task_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(stored_artifact_from_row).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::RUN_COLUMNS;

    #[test]
    fn run_projection_query_keeps_the_fencing_and_state_columns() {
        assert!(RUN_COLUMNS.contains("lease_fencing_token"));
        assert!(RUN_COLUMNS.contains("desired_state"));
    }
}
