//! Synthetic Integration peer; physical Git is tested separately with real toy repositories.
use super::*;
use forge_domain::git_integration::{IntegrationRequest, IntegrationResult};
use forge_protocol::supervisor::v1::GitIntegrationReply;
impl ManualSupervisor {
    pub async fn next_git_integration(&mut self) -> Result<IntegrationRequest> {
        let wire = self
            .next_matching("GitIntegrationCommand", |message| match &message.message {
                Some(core_to_supervisor::Message::GitIntegration(request)) => Some(request.clone()),
                _ => None,
            })
            .await?;
        let request: IntegrationRequest = serde_json::from_str(&wire.request_json)?;
        anyhow::ensure!(
            request.command_id.to_string() == wire.command_id,
            "command mismatch"
        );
        Ok(request)
    }
    pub async fn send_git_integration(
        &mut self,
        result: IntegrationResult,
    ) -> Result<CoreAcknowledgement> {
        let message_id = result.message_id.to_string();
        self.outbound
            .send(SupervisorToCore {
                message: Some(supervisor_to_core::Message::GitIntegration(
                    GitIntegrationReply {
                        message_id: message_id.clone(),
                        result_json: serde_json::to_string(&result)?,
                    },
                )),
            })
            .await?;
        self.wait_for_acknowledgement(&message_id).await
    }
}
