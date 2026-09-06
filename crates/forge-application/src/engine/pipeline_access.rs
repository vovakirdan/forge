//! Pipeline ownership checks used by task creation and pinned Task execution.

use super::CommandTransaction;
use forge_domain::{ExecutorKind, Project, StageEligibility};

use super::CommandError;

/// Validates a pinned Pipeline identity while allowing its catalog entry to be
/// soft-deleted. Existing Tasks must remain able to finish on their immutable
/// version.
pub fn ensure_pinned_pipeline_valid(
    project: &Project,
    pipeline: &forge_domain::Pipeline,
    version: &forge_domain::PipelineVersion,
) -> Result<(), CommandError> {
    if pipeline.project_id() != project.id() || version.project_id() != project.id() {
        return Err(CommandError::InvalidTransport {
            field: "pipeline_version_id",
            reason: "does not belong to the command project".to_owned(),
        });
    }
    if version.pipeline_id() != pipeline.id() {
        return Err(CommandError::InvalidTransport {
            field: "pipeline_version_id",
            reason: "does not belong to its named pipeline".to_owned(),
        });
    }
    Ok(())
}

/// Validates a Pipeline before it is newly assigned to a Task.
pub fn ensure_assignable_pipeline(
    project: &Project,
    pipeline: &forge_domain::Pipeline,
    version: &forge_domain::PipelineVersion,
) -> Result<(), CommandError> {
    ensure_pinned_pipeline_valid(project, pipeline, version)?;
    if pipeline.deleted_at().is_some() {
        return Err(CommandError::InvalidTransport {
            field: "pipeline_version_id",
            reason: "belongs to a deleted pipeline and cannot receive new tasks".to_owned(),
        });
    }
    Ok(())
}

/// Rejects dangling, cross-Project, or non-Employee stage selectors before an
/// Employee catalog entry becomes scheduler-visible.
pub async fn validate_employee_stage_eligibility(
    transaction: &mut impl CommandTransaction,
    project: &Project,
    employee: &forge_domain::Employee,
) -> Result<(), CommandError> {
    let StageEligibility::Only(targets) = employee.stage_eligibility() else {
        return Ok(());
    };
    for target in targets {
        let version = transaction
            .lock_pipeline_version(target.pipeline_version_id())
            .await?
            .ok_or(CommandError::NotFound {
                aggregate: "employee eligibility pipeline version",
            })?;
        if version.project_id() != project.id() {
            return Err(CommandError::InvalidTransport {
                field: "stage_eligibility.stages",
                reason: "references a pipeline version from another project".to_owned(),
            });
        }
        let Some(stage) = version.stage(target.stage_id()) else {
            return Err(CommandError::InvalidTransport {
                field: "stage_eligibility.stages",
                reason: "references a stage absent from its pipeline version".to_owned(),
            });
        };
        if stage.executor_kind() != ExecutorKind::Employee {
            return Err(CommandError::InvalidTransport {
                field: "stage_eligibility.stages",
                reason: "may select only employee-owned stages".to_owned(),
            });
        }
    }
    Ok(())
}
