//! Narrow Gateway entry to the same artifact/outcome validators and canonical
//! handlers used by Supervisor. No synthesized Supervisor identity or sequence.

use forge_domain::runtime::RunScope;
use forge_protocol::supervisor::v1::executor_submission;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{CoreError, CoreService};

use super::{
    bridge::InboundResult,
    validation::{self, SubmissionIngress, SubmissionScope},
};

impl CoreService {
    pub(crate) async fn submit_gateway(
        &self,
        scope: RunScope,
        message_id: Uuid,
        payload_hash: [u8; 32],
        payload: executor_submission::Payload,
    ) -> Result<Value, CoreError> {
        let scope = SubmissionScope {
            message_id,
            run_id: scope.run_id,
            lease_fencing_token: scope.fencing_token,
            environment_epoch: scope.environment_epoch,
            sequence: 0,
            ingress: SubmissionIngress::Gateway { payload_hash },
        };
        let invalid = |_| CoreError::InvalidTransport {
            field: "gateway.arguments",
            reason: "artifact or outcome has an invalid structural shape".to_owned(),
        };
        let result = match payload {
            executor_submission::Payload::Artifact(artifact) => {
                self.record_executor_artifact(
                    None,
                    validation::parse_artifact_submission(scope, artifact).map_err(invalid)?,
                )
                .await?
            }
            executor_submission::Payload::StageOutcome(outcome) => {
                self.record_executor_stage_outcome(
                    None,
                    validation::parse_stage_outcome_submission(scope, outcome).map_err(invalid)?,
                )
                .await?
            }
        };
        match result {
            InboundResult::Accepted(_) => Ok(json!({"status":"accepted","message_id":message_id})),
            InboundResult::Ignored(_) | InboundResult::Rejected(_, _) => {
                Err(CoreError::InvalidTransport {
                    field: "gateway.scope",
                    reason: "Run scope is no longer active".to_owned(),
                })
            }
        }
    }
}
