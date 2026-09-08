use super::*;
use forge_protocol::supervisor::v1::{
    DeliverRuntimeInput, RuntimeInputReceipt, RuntimeInputStatus,
};

impl ManualSupervisor {
    pub async fn next_runtime_input(&mut self, run: Uuid) -> Result<DeliverRuntimeInput> {
        self.next_matching("runtime input", |message| match &message.message {
            Some(core_to_supervisor::Message::DeliverRuntimeInput(input))
                if input.run_id == run.to_string() =>
            {
                Some(input.clone())
            }
            _ => None,
        })
        .await
    }
    pub async fn input_receipt(
        &mut self,
        input: &DeliverRuntimeInput,
        status: RuntimeInputStatus,
        receipt_id: Uuid,
    ) -> Result<CoreAcknowledgement> {
        self.outbound
            .send(SupervisorToCore {
                message: Some(supervisor_to_core::Message::RuntimeInputReceipt(
                    RuntimeInputReceipt {
                        message_id: receipt_id.to_string(),
                        input_command_id: input.command_id.clone(),
                        run_id: input.run_id.clone(),
                        lease_fencing_token: input.lease_fencing_token,
                        environment_epoch: input.environment_epoch,
                        status: status as i32,
                    },
                )),
            })
            .await?;
        self.wait_for_acknowledgement(&receipt_id.to_string()).await
    }
}
