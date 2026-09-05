//! Task intent, terminal data, and lifecycle mutation rules.

mod operations;
mod spec;
mod task;
mod terminal;

#[cfg(test)]
mod tests;

pub use spec::{TaskKind, TaskScope, TaskSource, TaskSpec, TaskSpecInput};
pub use task::Task;
pub use terminal::{
    CancellationData, CancellationRequest, CompletionData, CompletionSource, NewTask,
    TaskPipelineBinding, TaskWorkSurface, TerminalData,
};
