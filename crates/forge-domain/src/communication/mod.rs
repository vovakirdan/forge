//! Durable Employee conversations; Task references grant no execution authority.
mod message;
mod receipt;
mod runtime_input;
mod thread;
mod waiver;
pub use runtime_input::{RuntimeInputAction, RuntimeInputCloseReason, RuntimeInputOutcome};

pub use message::{
    DeliveryRequirement, EmployeeMessage, MAX_MESSAGE_BYTES, MessageInput, MessageKind,
    MessageTarget, TaskMessageContext,
};
pub use receipt::{DeliveryReceipt, DeliveryReceiptKind, DeliveryScope};
pub use thread::{EmployeeThread, EmployeeThreadInput, SendMessageInput};
pub use waiver::MessageRequirementWaiver;

mod context;
#[cfg(test)]
mod tests;
pub use context::{CommunicationContext, CommunicationContextInput};
