//! Explicit synthetic filesystem observations over the real gRPC boundary.
use super::*;
use forge_protocol::supervisor::v1::{GitCandidateInspectionResult, InspectGitCandidate};

impl ManualSupervisor {
    /// Waits for an exact post-quiescence candidate request.
    pub async fn next_git_inspection(&mut self) -> Result<InspectGitCandidate> {
        self.next_matching("InspectGitCandidate", |message| match &message.message {
            Some(core_to_supervisor::Message::InspectGitCandidate(request)) => {
                Some(request.clone())
            }
            _ => None,
        })
        .await
    }
    /// Supplies fixture-controlled evidence, never presented as actual Git proof.
    pub async fn send_git_inspection(
        &mut self,
        result: GitCandidateInspectionResult,
    ) -> Result<CoreAcknowledgement> {
        let id = result.message_id.clone();
        self.outbound
            .send(SupervisorToCore {
                message: Some(supervisor_to_core::Message::GitCandidateInspection(result)),
            })
            .await?;
        self.wait_for_acknowledgement(&id).await
    }
}
