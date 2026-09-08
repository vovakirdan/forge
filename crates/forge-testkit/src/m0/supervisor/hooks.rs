//! Explicit synthetic process evidence over the real fenced Supervisor channel.
use super::*;

impl ManualSupervisor {
    pub async fn send_hook_result(
        &mut self,
        run: &RunProjection,
        sequence: u64,
        result: Option<&forge_domain::HookExecutionResult>,
    ) -> Result<CoreAcknowledgement> {
        let message_id = Uuid::now_v7().to_string();
        self.outbound
            .send(SupervisorToCore {
                message: Some(supervisor_to_core::Message::ObservedRunEvent(
                    ObservedRunEvent {
                        message_id: message_id.clone(),
                        run_id: run.id.to_string(),
                        lease_fencing_token: run.lease_fencing_token,
                        environment_epoch: run.environment_epoch,
                        sequence,
                        occurred_at_unix_ms: 0,
                        kind: RunEventKind::Stopped as i32,
                        details_json: result.map_or_else(
                            || "{}".into(),
                            |result| json!({"hook_result":result}).to_string(),
                        ),
                    },
                )),
            })
            .await?;
        self.wait_for_acknowledgement(&message_id).await
    }
}
