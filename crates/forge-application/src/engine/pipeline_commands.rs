//! Catalog changes never rewrite immutable graphs or historical Task bindings.
use super::{
    CommandError, CommandTransaction, Engine, RepositoryError,
    event::{event, event_payload},
    receipt::{finish_command, resource},
};
use crate::{CommandEnvelope, CommandPayload, pipeline_input::PipelinePublication};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, PipelineVersionId, Project, Timestamp,
};
use forge_protocol::wire::CommandReceipt;
use serde_json::json;

impl Engine<'_> {
    pub(super) async fn manage_pipeline(
        &self,
        tx: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CommandError> {
        let (pipeline_id, expected_revision) = match &envelope.payload {
            CommandPayload::PublishPipelineVersion {
                pipeline_id,
                expected_pipeline_revision,
                ..
            }
            | CommandPayload::SetPipelineDefaultVersion {
                pipeline_id,
                expected_pipeline_revision,
                ..
            }
            | CommandPayload::DeletePipeline {
                pipeline_id,
                expected_pipeline_revision,
            } => (*pipeline_id, *expected_pipeline_revision),
            _ => return Err(CommandError::UnsupportedCommand),
        };
        let mut pipeline = tx
            .lock_pipeline(pipeline_id)
            .await?
            .filter(|value| value.project_id() == project.id())
            .ok_or(CommandError::NotFound {
                aggregate: "pipeline",
            })?;
        if pipeline.revision() != expected_revision {
            return Err(RepositoryError::StaleRevision {
                aggregate: "pipeline",
            }
            .into());
        }
        let mut events = Vec::new();
        let mut response_resource = resource("pipeline", pipeline.id().as_uuid());
        let catalog_kind = match &envelope.payload {
            CommandPayload::PublishPipelineVersion {
                definition,
                make_default,
                ..
            } => {
                let version = definition.build(PipelinePublication {
                    pipeline_id,
                    version_id: PipelineVersionId::new(),
                    project_id: project.id(),
                    version: pipeline.next_version()?,
                    created_by: self.actors.human,
                    created_at: now,
                })?;
                pipeline.publish_version(&version, *make_default)?;
                tx.insert_pipeline_version(&version).await?;
                events.push(event(
                    project.id(),
                    AggregateRef::PipelineVersion(version.id()),
                    u64::from(version.version()),
                    DomainEventKind::PipelineVersionPublished,
                    self.actors.human,
                    command_id,
                    None,
                    event_payload([
                        ("pipeline_id", json!(pipeline.id())),
                        ("pipeline_version_id", json!(version.id())),
                        ("version", json!(version.version())),
                        ("catalog_revision", json!(pipeline.revision())),
                        ("make_default", json!(make_default)),
                    ]),
                    now,
                )?);
                response_resource = resource("pipeline_version", version.id().as_uuid());
                make_default.then_some(DomainEventKind::PipelineDefaultVersionChanged)
            }
            CommandPayload::SetPipelineDefaultVersion {
                pipeline_version_id,
                ..
            } => {
                let version = tx
                    .lock_pipeline_version(*pipeline_version_id)
                    .await?
                    .ok_or(CommandError::NotFound {
                        aggregate: "pipeline version",
                    })?;
                pipeline.set_default_version(&version)?;
                Some(DomainEventKind::PipelineDefaultVersionChanged)
            }
            CommandPayload::DeletePipeline { .. } => {
                pipeline.soft_delete(now)?;
                Some(DomainEventKind::PipelineDeleted)
            }
            _ => return Err(CommandError::UnsupportedCommand),
        };
        tx.update_pipeline(&pipeline, expected_revision).await?;
        if let Some(kind) = catalog_kind {
            events.push(event(
                project.id(),
                AggregateRef::Pipeline(pipeline.id()),
                pipeline.revision(),
                kind,
                self.actors.human,
                command_id,
                None,
                event_payload([
                    ("pipeline_id", json!(pipeline.id())),
                    ("default_version_id", json!(pipeline.default_version_id())),
                    ("latest_version", json!(pipeline.latest_version())),
                    ("deleted_at", json!(pipeline.deleted_at())),
                ]),
                now,
            )?);
        }
        let previous = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(&project, previous).await?;
        finish_command(
            tx,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            events,
            Some(response_resource),
        )
        .await
    }
}
