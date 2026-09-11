//! Schema-specific authority validation before a retained execution is admitted.

use forge_domain::{
    StageId,
    runtime::{HookRunSpec, RuntimeLaunchSpec},
};
use forge_protocol::supervisor::v1::{ProvisionRun, provision_run::Assignment};

use crate::SupervisorError;

pub(crate) fn validate(provision: &ProvisionRun) -> Result<(), SupervisorError> {
    if provision.attempt == 0 {
        return Err(SupervisorError::InvalidRunSpec);
    }
    if (matches!(provision.run_spec_version, 5 | 7) && !provision.employee_id.is_empty())
        || (!matches!(provision.run_spec_version, 5 | 7)
            && uuid::Uuid::parse_str(&provision.employee_id).is_err())
    {
        return Err(SupervisorError::InvalidRunSpec);
    }
    let uuid = |value: &str| {
        uuid::Uuid::parse_str(value)
            .ok()
            .filter(|id| id.get_version_num() == 7)
    };
    let valid = match (provision.run_spec_version, &provision.assignment) {
        (1 | 2, None) => {
            uuid(&provision.task_id).is_some() && StageId::new(&provision.stage_id).is_ok()
        }
        (1 | 2 | 6, Some(Assignment::TaskStage(owner))) => {
            owner.task_id == provision.task_id
                && owner.stage_id == provision.stage_id
                && uuid(&owner.task_id).is_some()
                && uuid(&owner.queue_entry_id).is_some()
                && StageId::new(&owner.stage_id).is_ok()
        }
        (3, Some(Assignment::Communication(owner))) => {
            let spec = serde_json::from_str::<RuntimeLaunchSpec>(&provision.run_spec_json).ok();
            provision.task_id.is_empty()
                && provision.stage_id.is_empty()
                && spec.is_some_and(|spec| {
                    spec.schema_version == 3
                        && uuid(&provision.run_id) == Some(spec.surface_id)
                        && spec.communication.is_some_and(|assignment| {
                            uuid(&owner.assignment_id) == Some(assignment.assignment_id)
                                && uuid(&owner.thread_id) == Some(assignment.thread_id)
                                && uuid(&owner.source_message_id)
                                    == Some(assignment.source_message_id)
                        })
                })
        }
        (4, Some(Assignment::Resolution(owner))) => {
            let spec = serde_json::from_str::<RuntimeLaunchSpec>(&provision.run_spec_json).ok();
            provision.task_id.is_empty()
                && provision.stage_id.is_empty()
                && spec.is_some_and(|spec| {
                    spec.schema_version == 4
                        && uuid(&provision.run_id) == Some(spec.surface_id)
                        && spec.resolution.is_some_and(|assignment| {
                            uuid(&owner.assignment_id) == Some(assignment.assignment_id)
                                && uuid(&owner.escalation_id) == Some(assignment.escalation_id)
                                && owner.lease_generation == assignment.lease_generation
                                && owner.lease_generation > 0
                        })
                })
        }
        (7, Some(Assignment::SystemJob(owner))) => {
            let spec = serde_json::from_str::<forge_domain::runtime::SystemJobRunSpec>(
                &provision.run_spec_json,
            )
            .ok();
            provision.task_id.is_empty()
                && provision.stage_id.is_empty()
                && spec.is_some_and(|spec| {
                    spec.validate().is_ok()
                        && uuid(&provision.run_id) == Some(spec.run_id)
                        && uuid(&owner.job_id) == Some(spec.assignment.job_id)
                        && uuid(&owner.attempt_id) == Some(spec.assignment.attempt_id)
                        && owner.generation == spec.assignment.generation
                        && owner.kind
                            == match spec.assignment.kind {
                                forge_domain::system_job::SystemJobKind::Summarization => {
                                    "summarization"
                                }
                                forge_domain::system_job::SystemJobKind::Onboarding => "onboarding",
                            }
                })
        }
        (5, Some(Assignment::Hook(owner))) => {
            let spec = serde_json::from_str::<HookRunSpec>(&provision.run_spec_json).ok();
            provision.task_id.is_empty()
                && provision.stage_id.is_empty()
                && spec.is_some_and(|spec| {
                    spec.validate().is_ok()
                        && uuid(&provision.run_id) == Some(spec.run_id)
                        && uuid(&owner.invocation_id) == Some(spec.assignment.invocation_id)
                        && uuid(&owner.task_id) == Some(spec.assignment.task_id.as_uuid())
                        && uuid(&owner.pipeline_version_id)
                            == Some(spec.assignment.pipeline_version_id.as_uuid())
                        && owner.stage_id == spec.assignment.stage_id.as_str()
                        && owner.stage_visit == spec.assignment.stage_visit
                        && uuid(&owner.candidate_proposal_id)
                            == Some(spec.assignment.candidate_proposal_id)
                })
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(SupervisorError::InvalidRunSpec)
    }
}
