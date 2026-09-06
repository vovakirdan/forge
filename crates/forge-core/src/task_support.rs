//! Runtime orchestration uses the same Task rules as named application commands.

pub(crate) use forge_application::engine::task_support::{
    current_stage_executor, load_scoped_task, persist_task_and_project, pinned_pipeline_version,
    retained_persistence, stage_payload, task_event,
};
