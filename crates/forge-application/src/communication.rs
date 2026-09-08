//! Public inputs contain no sender identity or execution authority grants.
use forge_domain::{
    EmployeeId, TaskId,
    communication::{DeliveryRequirement, MessageKind, MessageTarget},
};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenEmployeeThreadCommand {
    pub employee_id: EmployeeId,
    #[serde(default)]
    pub task_id: Option<TaskId>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaiveMessageRequirementCommand {
    pub message_id: Uuid,
    pub reason: String,
}

#[derive(Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendEmployeeMessageCommand {
    pub thread_id: Uuid,
    pub expected_thread_revision: u64,
    pub target: MessageTarget,
    pub kind: MessageKind,
    pub requirement: DeliveryRequirement,
    pub body: String,
    #[serde(default)]
    pub reply_to: Option<Uuid>,
}

impl std::fmt::Debug for SendEmployeeMessageCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendEmployeeMessageCommand")
            .field("thread_id", &self.thread_id)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
