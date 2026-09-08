//! One draft-creation path for direct management and explicitly promoted Findings.
use super::{
    CommandError, CommandTransaction, Engine,
    event::{event, event_payload},
    pipeline_access::ensure_assignable_pipeline,
    scheduler::task_persistence,
};
use crate::{CreateTaskCommand, TaskDraftContext};
use forge_domain::{
    AggregateRef, CommandId, DomainEvent, DomainEventKind, Project, Task, TaskId,
    TaskPipelineBinding, TaskSource, Timestamp,
};
use serde_json::json;

impl Engine<'_> {
    pub(super) async fn create_draft(
        &self,
        tx: &mut impl CommandTransaction,
        project: &mut Project,
        input: &CreateTaskCommand,
        source: TaskSource,
        command: CommandId,
        now: Timestamp,
    ) -> Result<(Task, DomainEvent), CommandError> {
        let version_id = if let Some(id) = input.selected_pipeline_id()? {
            tx.lock_pipeline(id)
                .await?
                .filter(|pipeline| pipeline.project_id() == project.id())
                .ok_or(CommandError::NotFound {
                    aggregate: "pipeline",
                })?
                .default_version_id()
        } else {
            input.pipeline_version_id()?
        };
        let version =
            tx.lock_pipeline_version(version_id)
                .await?
                .ok_or(CommandError::NotFound {
                    aggregate: "pipeline version",
                })?;
        let pipeline =
            tx.lock_pipeline(version.pipeline_id())
                .await?
                .ok_or(CommandError::NotFound {
                    aggregate: "pipeline",
                })?;
        ensure_assignable_pipeline(project, &pipeline, &version)?;
        if !version.supports_task_kind(input.kind) {
            return Err(CommandError::InvalidTransport {
                field: "kind",
                reason: "is not supported by the selected pipeline version".into(),
            });
        }
        let previous = project.revision();
        let key = project.allocate_task_key(now)?;
        let task = input.build_with_source(
            TaskDraftContext {
                id: TaskId::new(),
                project_id: project.id(),
                key,
                pipeline: TaskPipelineBinding::new(
                    pipeline.id(),
                    version.id(),
                    version.entry_stage_id().clone(),
                ),
                created_by: self.actors.human,
                created_at: now,
                priority_scheme: project.priority_scheme(),
            },
            source,
        )?;
        tx.insert_task(&task, task_persistence(project, &task, 0)?)
            .await?;
        tx.update_project(project, previous).await?;
        let audit = event(
            project.id(),
            AggregateRef::Task(task.id()),
            task.revision().get(),
            DomainEventKind::TaskCreated,
            self.actors.human,
            command,
            None,
            event_payload([
                ("task_key", json!(task.key().to_string())),
                ("title", json!(task.spec().title())),
            ]),
            now,
        )?;
        Ok((task, audit))
    }
}
