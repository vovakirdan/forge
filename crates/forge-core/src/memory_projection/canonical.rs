use forge_domain::{
    ProjectId,
    knowledge::{
        DerivedMemoryEntry, KnowledgePage, KnowledgePageStatus, KnowledgeSourceRef, MemoryScope,
    },
};
use forge_storage::{
    KnowledgeProjectionKind, KnowledgeProjectionOperation, StorageError, StorageTransaction,
};
use serde::{Deserialize, Serialize};

use crate::{CoreError, CoreService};

/// Content comes only from Forge's canonical head, never an index response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "record", rename_all = "snake_case")]
pub enum MemorySearchDocument {
    KnowledgePage(KnowledgePage),
    DerivedMemory(DerivedMemoryEntry),
}

impl MemorySearchDocument {
    pub(super) fn scope(&self) -> MemoryScope {
        match self {
            Self::KnowledgePage(_) => MemoryScope::Project,
            Self::DerivedMemory(entry) => entry.subject.scope(),
        }
    }
    pub(super) fn sources(&self) -> &[KnowledgeSourceRef] {
        match self {
            Self::KnowledgePage(page) => &page.content.source_refs,
            Self::DerivedMemory(entry) => &entry.source_refs,
        }
    }
    pub(super) fn markdown(&self) -> &str {
        match self {
            Self::KnowledgePage(page) => &page.content.markdown,
            Self::DerivedMemory(entry) => &entry.markdown,
        }
    }
    pub(super) fn title(&self) -> &str {
        match self {
            Self::KnowledgePage(page) => &page.content.title,
            Self::DerivedMemory(_) => "Derived memory",
        }
    }
    pub(super) fn revision_hash(&self) -> (u64, &str) {
        match self {
            Self::KnowledgePage(page) => (page.revision, &page.content_hash),
            Self::DerivedMemory(entry) => (entry.revision, &entry.content_hash),
        }
    }
}

impl CoreService {
    pub(super) async fn projection_document(
        &self,
        transaction: &mut StorageTransaction<'_>,
        operation: &KnowledgeProjectionOperation,
    ) -> Result<Option<MemorySearchDocument>, CoreError> {
        if operation.retirement_requested || operation.retired {
            return Ok(None);
        }
        let document = match operation.object_kind {
            KnowledgeProjectionKind::KnowledgePage => self
                .store
                .load_knowledge_page(operation.project_id, operation.object_id)
                .await?
                .filter(|page| page.status == KnowledgePageStatus::Published)
                .map(MemorySearchDocument::KnowledgePage),
            KnowledgeProjectionKind::DerivedMemory => self
                .store
                .load_derived_memory_entry(operation.project_id, operation.object_id)
                .await?
                .filter(|entry| !entry.withdrawn)
                .map(MemorySearchDocument::DerivedMemory),
        };
        let Some(document) = document else {
            return Ok(None);
        };
        if document.revision_hash() != (operation.revision, operation.content_hash.as_str())
            || !sources_visible(transaction, operation.project_id, &document).await?
        {
            return Ok(None);
        }
        Ok(Some(document))
    }
}

pub(super) async fn sources_visible(
    transaction: &mut StorageTransaction<'_>,
    project: ProjectId,
    document: &MemorySearchDocument,
) -> Result<bool, CoreError> {
    match transaction
        .validate_knowledge_sources(project, document.scope(), document.sources())
        .await
    {
        Ok(()) => Ok(true),
        Err(StorageError::NotFound {
            aggregate: "knowledge_source",
        }) => Ok(false),
        Err(error) => Err(error.into()),
    }
}
