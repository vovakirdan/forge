//! Bounded replay ledger for input commands; independent of Run event sequence.
use super::Journal;
use crate::SupervisorError;
use forge_protocol::supervisor::v1::{
    DeliverRuntimeInput, RuntimeInputReceipt, SupervisorToCore, supervisor_to_core,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct InputRecord {
    pub request: DeliverRuntimeInput,
    hash: String,
    pub receipt: Option<SupervisorToCore>,
    pub pending: bool,
}

impl Journal {
    /// Commit precedes mailbox effect. Duplicate bytes never create another input.
    pub fn register_input(&mut self, input: &DeliverRuntimeInput) -> Result<bool, SupervisorError> {
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(input)?));
        if let Some(prior) = self.snapshot.runtime_inputs.get(&input.command_id) {
            if prior.hash != hash {
                return Err(SupervisorError::ConflictingProvision);
            }
            if prior.receipt.is_some() {
                self.change(|snapshot| {
                    if let Some(record) = snapshot.runtime_inputs.get_mut(&input.command_id) {
                        record.pending = true;
                    }
                    Ok(())
                })?;
                return Ok(false);
            }
            return Ok(true);
        }
        if self
            .snapshot
            .runtime_inputs
            .values()
            .filter(|record| record.request.run_id == input.run_id && record.receipt.is_none())
            .count()
            >= 129
        {
            return Err(SupervisorError::JournalFull);
        }
        self.change(|snapshot| {
            snapshot.runtime_inputs.insert(
                input.command_id.clone(),
                InputRecord {
                    request: input.clone(),
                    hash,
                    receipt: None,
                    pending: false,
                },
            );
            Ok(())
        })?;
        Ok(true)
    }

    pub fn input_request(&self, id: &str) -> Option<DeliverRuntimeInput> {
        self.snapshot
            .runtime_inputs
            .get(id)
            .map(|record| record.request.clone())
    }

    pub fn input_for_receipt(&self, id: &str) -> Option<DeliverRuntimeInput> {
        self.snapshot
            .runtime_inputs
            .values()
            .find(|record| {
                record
                    .receipt
                    .as_ref()
                    .and_then(super::message_id)
                    .is_some_and(|receipt| receipt == id)
            })
            .map(|record| record.request.clone())
    }

    pub fn record_input_receipt(
        &mut self,
        receipt: RuntimeInputReceipt,
    ) -> Result<(), SupervisorError> {
        let Some(record) = self.snapshot.runtime_inputs.get(&receipt.input_command_id) else {
            return Err(SupervisorError::InvalidJournalMessage);
        };
        if record.request.run_id != receipt.run_id
            || record.request.lease_fencing_token != receipt.lease_fencing_token
            || record.request.environment_epoch != receipt.environment_epoch
        {
            return Err(SupervisorError::InvalidJournalMessage);
        }
        if let Some(prior) = &record.receipt {
            if matches!(&prior.message,Some(supervisor_to_core::Message::RuntimeInputReceipt(prior)) if prior.status==receipt.status)
            {
                return Ok(());
            }
            return Err(SupervisorError::ConflictingProvision);
        }
        self.change(|snapshot| {
            let record = snapshot
                .runtime_inputs
                .get_mut(&receipt.input_command_id)
                .ok_or(SupervisorError::InvalidJournalMessage)?;
            record.receipt = Some(SupervisorToCore {
                message: Some(supervisor_to_core::Message::RuntimeInputReceipt(receipt)),
            });
            record.pending = true;
            // Hash retains equality while text remains in canonical source/evidence.
            if let Some(forge_protocol::supervisor::v1::deliver_runtime_input::Action::Message(
                message,
            )) = &mut record.request.action
            {
                message.source_message_json.clear();
            }
            Ok(())
        })
    }
}
