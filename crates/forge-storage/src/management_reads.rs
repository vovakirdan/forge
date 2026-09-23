//! Bounded, Project-scoped reads of retained operator control facts.

use forge_domain::{
    NextRunEmployeeConstraint, PipelineVersion, ProjectId, Task, Timestamp,
    resolution::ResolverRoute,
    runtime::{BootRecoveryPolicy, RecoveryAssessment},
};
use sqlx::Row;
use uuid::Uuid;

use crate::{PostgresStore, StorageError, decode_snapshot, domain_timestamp, i64_to_u64};

#[derive(Clone, Debug)]
pub struct RecoverySettingsRead {
    pub boot_policy: BootRecoveryPolicy,
    pub hold: bool,
}

#[derive(Clone, Debug)]
pub struct RecoveryRunRead {
    pub run_id: Uuid,
    pub task_id: Option<Uuid>,
    pub desired_state: crate::RunDesiredState,
    pub observed_state: crate::RunObservedState,
    pub created_at: Timestamp,
    pub started_at: Option<Timestamp>,
    pub liveness_observed_at: Option<Timestamp>,
    pub stop_requested_at: Option<Timestamp>,
    pub updated_at: Timestamp,
    pub lease_active: bool,
    pub reservation_state: Option<String>,
    pub reservation_released: Option<bool>,
    pub accepted_assessment: Option<RecoveryAssessment>,
    pub assessment_accepted_at: Option<Timestamp>,
    pub assessment_command_id: Option<Uuid>,
    pub recovery_queue_entry_id: Option<Uuid>,
}

/// Canonical facts needed to preview the non-start recovery command. The
/// command locks and checks these again before it changes anything.
#[derive(Clone, Debug)]
pub struct RecoveryAssessmentRead {
    pub run_id: Uuid,
    pub run_revision: u64,
    pub lease_fencing_token: u64,
    pub environment_epoch: u64,
    pub task_id: Option<Uuid>,
    pub stage_id: Option<String>,
    pub lease_active: bool,
    pub reservation_present: bool,
    pub reserved: bool,
    pub decision_accepted: bool,
    pub result_evidence_present: bool,
    pub boot_policy: BootRecoveryPolicy,
    pub task: Option<Task>,
    pub pipeline_version: Option<PipelineVersion>,
}

fn invalid() -> StorageError {
    StorageError::InvalidInput {
        reason: "invalid management read page".into(),
    }
}

fn decode<T: serde::de::DeserializeOwned>(
    value: &str,
    aggregate: &'static str,
) -> Result<T, StorageError> {
    serde_json::from_str(value).map_err(|source| StorageError::Snapshot { aggregate, source })
}

impl PostgresStore {
    /// One Project-scoped read omits all RunSpec, provider input and evidence bodies.
    pub async fn recovery_assessment_read(
        &self,
        project: ProjectId,
        run_id: Uuid,
    ) -> Result<Option<RecoveryAssessmentRead>, StorageError> {
        let row = sqlx::query(
            r#"SELECT r.id,r.revision,r.lease_fencing_token,r.environment_epoch,
             CASE WHEN r.purpose='task_stage' THEN r.task_id ELSE NULL END AS task_id,
             CASE WHEN r.purpose='task_stage' THEN r.stage_id ELSE NULL END AS stage_id,
             (l.lease_state='active') AS lease_active,
             (e.run_id IS NOT NULL) AS reservation_present,
             (e.released_at IS NULL AND e.run_id IS NOT NULL) AS reserved,
             (d.run_id IS NOT NULL) AS decision_accepted,
             (EXISTS(SELECT 1 FROM artifacts a WHERE a.run_id=r.id)
               OR EXISTS(SELECT 1 FROM task_handoffs h WHERE h.run_id=r.id AND h.kind='accepted')
               OR EXISTS(SELECT 1 FROM employee_message_receipts m WHERE m.run_id=r.id AND m.kind IN ('acknowledged','answered'))) AS result_evidence_present,
             COALESCE(s.boot_policy,'recover_safe_then_hold') AS boot_policy,
             t.canonical_snapshot::text AS task_snapshot,
             v.definition::text AS version_snapshot
             FROM runs r JOIN leases l ON l.id=r.lease_id
             LEFT JOIN run_environment_reservations e ON e.run_id=r.id AND e.project_id=r.project_id
             LEFT JOIN run_recovery_decisions d ON d.run_id=r.id
             LEFT JOIN project_recovery_settings s ON s.project_id=r.project_id
             LEFT JOIN tasks t ON t.id=r.task_id AND t.project_id=r.project_id AND r.purpose='task_stage'
             LEFT JOIN pipeline_versions v ON v.id=t.pipeline_version_id
             WHERE r.project_id=$1 AND r.id=$2"#,
        )
        .bind(project.as_uuid())
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let policy: String = row.try_get("boot_policy")?;
            let task_snapshot: Option<String> = row.try_get("task_snapshot")?;
            let version_snapshot: Option<String> = row.try_get("version_snapshot")?;
            Ok(RecoveryAssessmentRead {
                run_id: row.try_get("id")?,
                run_revision: i64_to_u64(row.try_get("revision")?, "run.revision")?,
                lease_fencing_token: i64_to_u64(
                    row.try_get("lease_fencing_token")?,
                    "run.lease_fencing_token",
                )?,
                environment_epoch: i64_to_u64(
                    row.try_get("environment_epoch")?,
                    "run.environment_epoch",
                )?,
                task_id: row.try_get("task_id")?,
                stage_id: row.try_get("stage_id")?,
                lease_active: row.try_get("lease_active")?,
                reservation_present: row.try_get("reservation_present")?,
                reserved: row.try_get("reserved")?,
                decision_accepted: row.try_get("decision_accepted")?,
                result_evidence_present: row.try_get("result_evidence_present")?,
                boot_policy: decode(&format!("\"{policy}\""), "boot_policy")?,
                task: task_snapshot
                    .map(|snapshot| decode_snapshot(&snapshot, "task"))
                    .transpose()?,
                pipeline_version: version_snapshot
                    .map(|snapshot| decode_snapshot(&snapshot, "pipeline_version"))
                    .transpose()?,
            })
        })
        .transpose()
    }
    pub async fn resolver_route_page(
        &self,
        project: ProjectId,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ResolverRoute>, StorageError> {
        if !(1..=51).contains(&limit) {
            return Err(invalid());
        }
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM resolver_routes WHERE project_id=$1 AND ($2::text IS NULL OR route_key>$2) ORDER BY route_key LIMIT $3")
            .bind(project.as_uuid()).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        values
            .into_iter()
            .map(|value| {
                let route: ResolverRoute = decode(&value, "resolver_route")?;
                route.validate_snapshot().map_err(|_| invalid())?;
                if route.project_id != project {
                    return Err(invalid());
                }
                Ok(route)
            })
            .collect()
    }

    pub async fn next_run_constraint_page(
        &self,
        project: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<NextRunEmployeeConstraint>, StorageError> {
        if !(1..=51).contains(&limit) {
            return Err(invalid());
        }
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM task_next_run_constraints WHERE project_id=$1 AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3")
            .bind(project.as_uuid()).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        values
            .into_iter()
            .map(|value| {
                let constraint: NextRunEmployeeConstraint = decode(&value, "dispatch_constraint")?;
                constraint.validate_snapshot().map_err(|_| invalid())?;
                if constraint.project_id != project {
                    return Err(invalid());
                }
                Ok(constraint)
            })
            .collect()
    }

    pub async fn recovery_settings_read(
        &self,
        project: ProjectId,
    ) -> Result<RecoverySettingsRead, StorageError> {
        let row: Option<(String, bool)> = sqlx::query_as(
            "SELECT boot_policy,hold FROM project_recovery_settings WHERE project_id=$1",
        )
        .bind(project.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some((policy, hold)) => Ok(RecoverySettingsRead {
                boot_policy: decode(&format!("\"{policy}\""), "boot_policy")?,
                hold,
            }),
            None => Ok(RecoverySettingsRead {
                boot_policy: BootRecoveryPolicy::default(),
                hold: false,
            }),
        }
    }

    pub async fn recovery_run_page(
        &self,
        project: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<RecoveryRunRead>, StorageError> {
        if !(1..=51).contains(&limit) {
            return Err(invalid());
        }
        let rows = sqlx::query("SELECT r.id,r.task_id,r.desired_state,r.observed_state,r.created_at,r.started_at,r.liveness_observed_at,r.stop_requested_at,r.updated_at,\
                (l.lease_state='active') AS lease_active,e.state AS reservation_state,\
                (e.released_at IS NOT NULL) AS reservation_released,d.assessment,d.accepted_at,d.accepted_command_id,d.recovery_queue_entry_id \
                FROM runs r JOIN leases l ON l.id=r.lease_id \
                LEFT JOIN run_environment_reservations e ON e.run_id=r.id AND e.project_id=r.project_id \
                LEFT JOIN run_recovery_decisions d ON d.run_id=r.id \
                WHERE r.project_id=$1 AND ($2::uuid IS NULL OR r.id>$2) ORDER BY r.id LIMIT $3")
            .bind(project.as_uuid()).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                let desired: String = row.try_get("desired_state")?;
                let observed: String = row.try_get("observed_state")?;
                let assessment: Option<String> = row.try_get("assessment")?;
                let timestamp = |key| -> Result<Option<Timestamp>, StorageError> {
                    Ok(row
                        .try_get::<Option<time::OffsetDateTime>, _>(key)?
                        .map(domain_timestamp))
                };
                Ok(RecoveryRunRead {
                    run_id: row.try_get("id")?,
                    task_id: row.try_get("task_id")?,
                    desired_state: decode(&format!("\"{desired}\""), "run_desired_state")?,
                    observed_state: decode(&format!("\"{observed}\""), "run_observed_state")?,
                    created_at: domain_timestamp(row.try_get("created_at")?),
                    started_at: timestamp("started_at")?,
                    liveness_observed_at: timestamp("liveness_observed_at")?,
                    stop_requested_at: timestamp("stop_requested_at")?,
                    updated_at: domain_timestamp(row.try_get("updated_at")?),
                    lease_active: row.try_get("lease_active")?,
                    reservation_state: row.try_get("reservation_state")?,
                    reservation_released: row.try_get("reservation_released")?,
                    accepted_assessment: assessment
                        .map(|value| decode(&format!("\"{value}\""), "recovery_assessment"))
                        .transpose()?,
                    assessment_accepted_at: timestamp("accepted_at")?,
                    assessment_command_id: row.try_get("accepted_command_id")?,
                    recovery_queue_entry_id: row.try_get("recovery_queue_entry_id")?,
                })
            })
            .collect()
    }
}
