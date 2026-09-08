//! Future Employee selection is fenced by the exact Task visit, not a fallback preference.
use forge_domain::{
    Actor, AggregateRef, CommandId, ConstraintBlockReason, ConstraintEndReason, DomainEventKind,
    Employee, EmployeeState, NextRunConstraintState, NextRunEmployeeConstraint, Project, Task,
    Timestamp,
};
use forge_storage::{QueueEntry, StorageTransaction};
use serde_json::json;

use crate::{
    CoreError,
    event::{event, event_payload},
};

pub(crate) enum EmployeeSelection {
    Ready {
        employee: Employee,
        constraint: Option<NextRunEmployeeConstraint>,
    },
    Unavailable,
    ChangeConstraint {
        constraint: NextRunEmployeeConstraint,
        result: NextRunConstraintState,
    },
}

pub(crate) async fn select_employee(
    transaction: &mut StorageTransaction<'_>,
    queue: &QueueEntry,
    task: &Task,
) -> Result<EmployeeSelection, CoreError> {
    let version = transaction
        .lock_pipeline_version(queue.input.pipeline_version_id)
        .await?
        .filter(|version| version.project_id() == task.project_id())
        .ok_or(CoreError::NotFound {
            aggregate: "PipelineVersion",
        })?;
    let stage = version
        .stage(&queue.input.stage_id)
        .ok_or(CoreError::NotFound {
            aggregate: "PipelineStage",
        })?;
    if let Some(policy) = stage.acceptance_policy() {
        policy.validate_task_surface(task.work_surface())?;
    }
    let excluded = if stage
        .acceptance_policy()
        .is_some_and(|policy| policy.requires_independence())
    {
        let Some(candidate) = transaction
            .latest_accepted_git_candidate(task.project_id(), task.id())
            .await?
        else {
            return Ok(EmployeeSelection::Unavailable);
        };
        candidate.contributors
    } else {
        std::collections::BTreeSet::new()
    };
    let constraint = transaction
        .load_task_dispatch_constraint(queue.input.project_id, task.id())
        .await?;
    if let Some(constraint) = &constraint {
        if !constraint.applies_to(task) {
            return Ok(EmployeeSelection::ChangeConstraint {
                constraint: constraint.clone(),
                result: NextRunConstraintState::Cancelled {
                    reason: ConstraintEndReason::StageLeft,
                },
            });
        }
        if matches!(constraint.state, NextRunConstraintState::Blocked { .. }) {
            return Ok(EmployeeSelection::Unavailable);
        }
        let target = transaction
            .lock_employee(constraint.employee_id)
            .await?
            .filter(|stored| stored.employee.project_id() == task.project_id())
            .ok_or(CoreError::NotFound {
                aggregate: "constrained Employee",
            })?
            .employee;
        let reason = if target.state() != EmployeeState::Enabled {
            Some(ConstraintBlockReason::EmployeeDisabled)
        } else if excluded.contains(&target.id())
            || !target.is_eligible_for(queue.input.pipeline_version_id, &queue.input.stage_id)
            || !workspace_matches(transaction, &target, stage, task).await?
        {
            Some(ConstraintBlockReason::StageIneligible)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Ok(EmployeeSelection::ChangeConstraint {
                constraint: constraint.clone(),
                result: NextRunConstraintState::Blocked { reason },
            });
        }
    }
    let employees = transaction
        .lock_available_employees(queue.input.project_id)
        .await?;
    let mut selected = None;
    for stored in employees {
        if constraint
            .as_ref()
            .is_none_or(|constraint| constraint.employee_id == stored.employee.id())
            && stored
                .employee
                .is_eligible_for(queue.input.pipeline_version_id, &queue.input.stage_id)
            && !excluded.contains(&stored.employee.id())
            && workspace_matches(transaction, &stored.employee, stage, task).await?
            && admission_matches(transaction, &stored.employee).await?
        {
            selected = Some(stored.employee);
            break;
        }
    }
    Ok(match selected {
        Some(employee) => EmployeeSelection::Ready {
            employee,
            constraint,
        },
        None => EmployeeSelection::Unavailable,
    })
}

async fn workspace_matches(
    tx: &mut StorageTransaction<'_>,
    employee: &Employee,
    stage: &forge_domain::PipelineStage,
    task: &Task,
) -> Result<bool, CoreError> {
    let Some(requirement) = stage.workspace() else {
        return Ok(true);
    };
    let Some(binding) = tx.runtime_binding(employee.id()).await? else {
        return Ok(false);
    };
    let surface = match task.work_surface() {
        forge_domain::TaskWorkSurface::Git(git) => {
            forge_domain::runtime::SurfaceSpec::GitWorktree {
                repository: git.source.as_path().to_string_lossy().into_owned(),
                base_ref: git.initial_base.as_str().into(),
            }
        }
        forge_domain::TaskWorkSurface::None => binding.surface,
    };
    Ok(requirement.is_compatible(&surface, binding.access))
}

async fn admission_matches(
    tx: &mut StorageTransaction<'_>,
    employee: &Employee,
) -> Result<bool, CoreError> {
    let binding = tx.runtime_binding(employee.id()).await?;
    Ok(tx
        .lock_run_admission(
            employee.project_id(),
            binding.as_ref().map(|binding| &binding.execution_profile),
        )
        .await?)
}

#[expect(
    clippy::too_many_arguments,
    reason = "explicit transaction, aggregate, intent, and canonical audit scope"
)]
pub(crate) async fn persist_constraint_result(
    transaction: &mut StorageTransaction<'_>,
    project: &mut Project,
    task: &Task,
    mut constraint: NextRunEmployeeConstraint,
    result: NextRunConstraintState,
    actor: Actor,
    command_id: CommandId,
    now: Timestamp,
) -> Result<(), CoreError> {
    let expected = constraint.state.clone();
    constraint.transition(result)?;
    transaction
        .update_task_dispatch_constraint(&constraint, &expected)
        .await?;
    let previous = project.revision();
    project.record_child_mutation(now)?;
    transaction.update_project(project, previous).await?;
    transaction
        .append_event_and_outbox(&event(
            project.id(),
            AggregateRef::Task(task.id()),
            task.revision().get(),
            DomainEventKind::TaskDispatchConstraintChanged,
            actor,
            command_id,
            None,
            event_payload([("constraint", json!(constraint))]),
            now,
        )?)
        .await?;
    Ok(())
}
