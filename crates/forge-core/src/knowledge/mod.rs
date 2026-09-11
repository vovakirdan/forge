//! Knowledge writes share the Core command transaction; retrieval is never canonical authority.

use forge_application::CommandEnvelope;
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, Project, Timestamp,
    knowledge::{DerivedMemoryEntry, KnowledgePage, KnowledgePageMutation, MemoryScope},
};
use forge_protocol::wire::CommandReceipt;
use forge_storage::StorageTransaction;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    CoreError, CoreService,
    command::{finish_command, resource},
    event::{event, event_payload},
};

impl CoreService {
    #[expect(
        clippy::too_many_arguments,
        reason = "knowledge commands retain the shared atomic command boundary"
    )]
    pub(crate) async fn manage_knowledge_page(
        &self,
        tx: &mut StorageTransaction<'_>,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
        input: &KnowledgePageMutation,
    ) -> Result<CommandReceipt, CoreError> {
        let previous = tx
            .lock_knowledge_page(project.id(), input.page_id())
            .await?;
        if previous.is_none() && input.expected_revision() != 0 {
            return Err(CoreError::NotFound {
                aggregate: "knowledge_page",
            });
        }
        if previous
            .as_ref()
            .is_some_and(|page| page.revision != input.expected_revision())
        {
            return Err(forge_storage::StorageError::StaleRevision {
                aggregate: "knowledge_page",
            }
            .into());
        }
        let markdown = input
            .content()
            .map(|content| content.markdown.as_str())
            .or_else(|| previous.as_ref().map(|page| page.content.markdown.as_str()))
            .ok_or(CoreError::NotFound {
                aggregate: "knowledge_page",
            })?;
        let hash = format!("{:x}", Sha256::digest(markdown.as_bytes()));
        let page = KnowledgePage::apply(
            previous.as_ref(),
            project.id(),
            input,
            self.actors.human,
            command_id,
            now,
            hash,
        )?;
        // Withdrawal must remain possible after a cited page was itself withdrawn.
        if !matches!(input, KnowledgePageMutation::Withdraw { .. }) {
            tx.validate_knowledge_sources(
                project.id(),
                MemoryScope::Project,
                &page.content.source_refs,
            )
            .await?;
        }
        tx.save_knowledge_page_revision(&page, input.expected_revision())
            .await?;
        let previous_project_revision = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(&project, previous_project_revision)
            .await?;
        let audit = event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::KnowledgePageChanged,
            self.actors.human,
            command_id,
            None,
            event_payload([
                ("page_id", json!(page.id)),
                ("page_revision", json!(page.revision)),
                ("kind", json!(page.kind)),
                ("status", json!(page.status)),
                ("content_hash", json!(page.content_hash)),
            ]),
            now,
        )?;
        finish_command(
            tx,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            vec![audit],
            Some(resource("knowledge_page", page.id)),
        )
        .await
    }

    /// System-job completion validates its input allowlist/fence before calling this in that same transaction.
    pub(crate) async fn persist_derived_memory(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &Project,
        entry: &DerivedMemoryEntry,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<bool, CoreError> {
        entry.validate()?;
        if entry.project_id != project.id() {
            return Err(CoreError::Forbidden);
        }
        if entry.content_hash != format!("{:x}", Sha256::digest(entry.markdown.as_bytes())) {
            return Err(CoreError::InvalidTransport {
                field: "memory.content_hash",
                reason: "does not match canonical Markdown".into(),
            });
        }
        tx.validate_knowledge_sources(project.id(), entry.subject.scope(), &entry.source_refs)
            .await?;
        if let Some(coverage) = entry.coverage {
            tx.validate_memory_coverage(project.id(), coverage).await?;
        }
        if !tx.insert_derived_memory_revision(entry).await? {
            return Ok(false);
        }
        let audit = event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::DerivedMemoryChanged,
            self.actors.core,
            command_id,
            None,
            event_payload([
                ("entry_id", json!(entry.id)),
                ("entry_revision", json!(entry.revision)),
                ("scope", json!(entry.subject.scope())),
                ("job_id", json!(entry.created_by_job_id)),
                ("content_hash", json!(entry.content_hash)),
                ("withdrawn", json!(entry.withdrawn)),
            ]),
            now,
        )?;
        tx.append_event_and_outbox(&audit).await?;
        Ok(true)
    }
}
