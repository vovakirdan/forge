//! Delivery of committed fenced stops stays in Core's runtime adapter.

use forge_domain::{CommandId, DomainEvent, Project, Timestamp};
use forge_protocol::supervisor::v1::{CoreToSupervisor, StopMode, StopRun, core_to_supervisor};
use forge_storage::{RunDesiredState, RunProjection};
use uuid::Uuid;

use crate::{CoreError, CoreService};

impl CoreService {
    /// Replays committed desired stops after the transaction boundary. Sending
    /// is idempotent for a Run fence and never writes canonical state.
    pub(crate) async fn deliver_pending_stop_requests(
        &self,
        project_id: forge_domain::ProjectId,
    ) -> Result<usize, CoreError> {
        let mut transaction = self.store.begin().await?;
        let runs = transaction.lock_active_runs_for_project(project_id).await?;
        transaction.commit().await?;
        let mut delivered = 0_usize;
        for run in runs {
            let Some(stop) = stop_message(&run) else {
                continue;
            };
            self.supervisor.send(stop).await?;
            delivered = delivered.saturating_add(1);
        }
        Ok(delivered)
    }
}

pub(crate) fn run_stop_event(
    project: &Project,
    run: &RunProjection,
    command_id: CommandId,
    actor: forge_domain::Actor,
    reason_code: &str,
    now: Timestamp,
) -> Result<DomainEvent, CoreError> {
    forge_application::engine::run_control::run_stop_event(
        project,
        &run.into(),
        command_id,
        actor,
        reason_code,
        now,
    )
    .map_err(Into::into)
}

fn stop_message(run: &RunProjection) -> Option<CoreToSupervisor> {
    let mode = match run.desired_state {
        RunDesiredState::StopRequested => StopMode::Graceful,
        RunDesiredState::ForceStopRequested => StopMode::Force,
        _ => return None,
    };
    Some(CoreToSupervisor {
        message: Some(core_to_supervisor::Message::StopRun(StopRun {
            command_id: Uuid::now_v7().to_string(),
            run_id: run.id.to_string(),
            lease_fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
            mode: mode as i32,
            reason_code: "core_stop_requested".to_owned(),
            grace_period_ms: run
                .run_spec
                .pointer("/binding/limits/stop_grace_seconds")
                .and_then(serde_json::Value::as_u64)
                .map(|seconds| seconds.saturating_mul(1_000))
                .unwrap_or(2_000),
        })),
    })
}

#[cfg(test)]
mod tests {
    use forge_storage::{RunDesiredState, RunObservedState, RunProjection};
    use serde_json::json;

    use super::stop_message;

    #[test]
    fn pending_graceful_stop_becomes_a_fenced_transport_request() {
        let run = RunProjection {
            id: uuid::Uuid::now_v7(),
            project_id: forge_domain::ProjectId::new(),
            task_id: forge_domain::TaskId::new(),
            queue_entry_id: uuid::Uuid::now_v7(),
            lease_id: uuid::Uuid::now_v7(),
            employee_id: forge_domain::EmployeeId::new(),
            stage_id: "work".to_owned(),
            attempt_number: 1,
            lease_fencing_token: 7,
            environment_epoch: 2,
            last_sequence: 1,
            desired_state: RunDesiredState::StopRequested,
            observed_state: RunObservedState::Running,
            run_spec_version: 1,
            run_spec: json!({}),
            context_manifest: json!({}),
            observed_details: json!({}),
        };

        let message = stop_message(&run);

        assert!(message.is_some());
    }
}
