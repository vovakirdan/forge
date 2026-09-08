//! Recovery never turns uncertain side effects into an implicit new attempt.

#[path = "recovery/boot.rs"]
mod boot;
#[path = "recovery/commands.rs"]
mod commands;

use forge_domain::ProjectId;
use forge_protocol::{
    supervisor::v1::{CoreToSupervisor, ProvisionRun, core_to_supervisor},
    wire::M0_RUN_SPEC_VERSION,
};
use forge_storage::{RunDesiredState, RunObservedState, RunProjection};
use uuid::Uuid;

use crate::{CoreError, CoreService};

impl CoreService {
    /// Replays only durable desired messages that are safe in the M0 simulator.
    ///
    /// A Run which has produced even a provisioning observation is intentionally
    /// held rather than silently re-executed. M0 has no recovery assessment or
    /// WorkSurface inspection yet, so only a Run with no observation at all can
    /// be re-delivered to a freshly attached deterministic Supervisor.
    pub(crate) async fn recover_after_supervisor_attach(&self) -> Result<(), CoreError> {
        let project_ids = self.store.list_project_ids().await?;
        for project_id in &project_ids {
            self.deliver_pending_stop_requests(*project_id).await?;
            self.redeliver_unobserved_provisions(*project_id).await?;
        }

        let open_project_ids = self.store.list_execution_open_project_ids().await?;
        for project_id in open_project_ids {
            self.dispatch_available(project_id).await?;
        }
        self.reconcile_runtime_inputs(None).await?;
        Ok(())
    }

    async fn redeliver_unobserved_provisions(
        &self,
        project_id: ProjectId,
    ) -> Result<(), CoreError> {
        let runs = self.store.list_runs_for_project(project_id).await?;
        for run in runs.iter().filter(|run| should_redeliver_provision(run)) {
            self.supervisor.send(recovered_provision(run)?).await?;
        }
        Ok(())
    }
}

fn should_redeliver_provision(run: &RunProjection) -> bool {
    u32::from(run.run_spec_version) == M0_RUN_SPEC_VERSION
        && run.desired_state == RunDesiredState::ProvisionRequested
        && run.observed_state == RunObservedState::Unknown
}

fn recovered_provision(run: &RunProjection) -> Result<CoreToSupervisor, CoreError> {
    if u32::from(run.run_spec_version) != M0_RUN_SPEC_VERSION {
        return Err(CoreError::InvalidTransport {
            field: "run.run_spec_version",
            reason: "is not supported by the local M0 Supervisor".to_owned(),
        });
    }
    let context_snapshot_id = run
        .context_manifest
        .get("context_snapshot_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CoreError::InvalidTransport {
            field: "run.context_manifest.context_snapshot_id",
            reason: "is required for recovery".to_owned(),
        })?;
    Uuid::parse_str(context_snapshot_id).map_err(|_| CoreError::InvalidTransport {
        field: "run.context_manifest.context_snapshot_id",
        reason: "must be a UUID".to_owned(),
    })?;
    let run_spec_json =
        serde_json::to_string(&run.run_spec).map_err(|error| CoreError::InvalidTransport {
            field: "run.run_spec",
            reason: error.to_string(),
        })?;
    Ok(CoreToSupervisor {
        message: Some(core_to_supervisor::Message::ProvisionRun(ProvisionRun {
            command_id: Uuid::now_v7().to_string(),
            run_id: run.id.to_string(),
            task_id: run.require_task_id()?.as_uuid().to_string(),
            employee_id: run.require_employee_id()?.as_uuid().to_string(),
            stage_id: run.require_task_stage()?.stage_id.to_string(),
            attempt: run.attempt_number,
            lease_fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
            context_snapshot_id: context_snapshot_id.to_owned(),
            run_spec_json,
            run_spec_version: M0_RUN_SPEC_VERSION,
            traceparent: crate::observability::current_traceparent(),
            assignment: None,
        })),
    })
}

#[cfg(test)]
mod tests {
    use forge_storage::{RunDesiredState, RunObservedState, RunProjection};
    use serde_json::json;

    use super::{recovered_provision, should_redeliver_provision};

    fn run() -> RunProjection {
        RunProjection {
            id: uuid::Uuid::now_v7(),
            project_id: forge_domain::ProjectId::new(),
            assignment: forge_domain::ExecutionAssignment::TaskStage(
                forge_domain::TaskStageAssignment {
                    task_id: forge_domain::TaskId::new(),
                    queue_entry_id: uuid::Uuid::now_v7(),
                    stage_id: forge_domain::StageId::new("work").unwrap(),
                },
            ),
            lease_id: uuid::Uuid::now_v7(),
            employee_id: Some(forge_domain::EmployeeId::new()),
            attempt_number: 1,
            lease_fencing_token: 1,
            environment_epoch: 1,
            last_sequence: 0,
            desired_state: RunDesiredState::ProvisionRequested,
            observed_state: RunObservedState::Unknown,
            run_spec_version: 1,
            run_spec: json!({"artifacts": [], "stage_outcome": {"outcome": "done"}}),
            context_manifest: json!({"context_snapshot_id": uuid::Uuid::now_v7()}),
            observed_details: json!({}),
        }
    }

    #[test]
    fn unobserved_provision_is_the_only_replayable_m0_run() {
        assert!(should_redeliver_provision(&run()));
    }

    #[test]
    fn recovery_reuses_the_original_run_fence() {
        let run = run();

        let message = recovered_provision(&run).expect("valid recovery provision");
        let Some(forge_protocol::supervisor::v1::core_to_supervisor::Message::ProvisionRun(
            provision,
        )) = message.message
        else {
            panic!("expected provision message");
        };

        assert_eq!(provision.run_id, run.id.to_string());
    }
}
