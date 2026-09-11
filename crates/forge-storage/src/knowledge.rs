//! Canonical page revisions and their durable projection intents.

mod context;
mod derived;
mod projection;
mod refresh;
mod sources;

pub use projection::{KnowledgeProjectionKind, KnowledgeProjectionOperation};
pub use refresh::KnowledgeContextRefreshRecord;

use forge_domain::{ProjectId, knowledge::KnowledgePage};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    PostgresStore, StorageError, StorageTransaction, encode_snapshot, enum_text, u64_to_i64,
};

fn decode_page(value: &str) -> Result<KnowledgePage, StorageError> {
    let page: KnowledgePage =
        serde_json::from_str(value).map_err(|source| StorageError::Snapshot {
            aggregate: "knowledge_page",
            source,
        })?;
    page.validate()
        .map_err(|source| StorageError::SnapshotInvariant {
            aggregate: "knowledge_page",
            source,
        })?;
    validate_digest(&page.content.markdown, &page.content_hash)?;
    Ok(page)
}

fn validate_digest(markdown: &str, hash: &str) -> Result<(), StorageError> {
    let calculated: String = Sha256::digest(markdown.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if hash != calculated {
        return Err(StorageError::InvalidInput {
            reason: "knowledge content hash mismatch".into(),
        });
    }
    Ok(())
}

impl StorageTransaction<'_> {
    /// Locks the canonical head under the owning Project transaction.
    pub async fn lock_knowledge_page(
        &mut self,
        project_id: ProjectId,
        id: Uuid,
    ) -> Result<Option<KnowledgePage>, StorageError> {
        let value: Option<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM knowledge_pages WHERE project_id=$1 AND id=$2 FOR UPDATE")
            .bind(project_id.as_uuid()).bind(id).fetch_optional(&mut *self.transaction).await?;
        value.as_deref().map(decode_page).transpose()
    }

    /// Stores head, immutable history and a stable index operation in the same transaction.
    pub async fn save_knowledge_page_revision(
        &mut self,
        page: &KnowledgePage,
        previous_revision: u64,
    ) -> Result<(), StorageError> {
        page.validate()
            .map_err(|source| StorageError::SnapshotInvariant {
                aggregate: "knowledge_page",
                source,
            })?;
        validate_digest(&page.content.markdown, &page.content_hash)?;
        if page.revision
            != previous_revision
                .checked_add(1)
                .ok_or(StorageError::IntegerOutOfRange {
                    field: "knowledge.revision",
                })?
        {
            return Err(StorageError::StaleRevision {
                aggregate: "knowledge_page",
            });
        }
        let snapshot = encode_snapshot(page, "knowledge_page")?;
        let result = if previous_revision == 0 {
            sqlx::query("INSERT INTO knowledge_pages(id,project_id,revision,kind,status,content_hash,canonical_snapshot) VALUES($1,$2,$3,$4,$5,$6,$7::jsonb) ON CONFLICT DO NOTHING")
                .bind(page.id).bind(page.project_id.as_uuid()).bind(u64_to_i64(page.revision,"knowledge.revision")?)
                .bind(enum_text(&page.kind,"knowledge.kind")?).bind(enum_text(&page.status,"knowledge.status")?)
                .bind(&page.content_hash).bind(&snapshot).execute(&mut *self.transaction).await?
        } else {
            sqlx::query("UPDATE knowledge_pages SET revision=$3,status=$4,content_hash=$5,canonical_snapshot=$6::jsonb WHERE project_id=$1 AND id=$2 AND revision=$7")
                .bind(page.project_id.as_uuid()).bind(page.id).bind(u64_to_i64(page.revision,"knowledge.revision")?)
                .bind(enum_text(&page.status,"knowledge.status")?).bind(&page.content_hash).bind(&snapshot)
                .bind(u64_to_i64(previous_revision,"knowledge.previous_revision")?).execute(&mut *self.transaction).await?
        };
        if result.rows_affected() != 1 {
            return Err(StorageError::StaleRevision {
                aggregate: "knowledge_page",
            });
        }
        sqlx::query("INSERT INTO knowledge_page_revisions(page_id,project_id,revision,canonical_snapshot) VALUES($1,$2,$3,$4::jsonb)")
            .bind(page.id).bind(page.project_id.as_uuid()).bind(u64_to_i64(page.revision,"knowledge.revision")?).bind(snapshot)
            .execute(&mut *self.transaction).await?;
        self.enqueue_knowledge_projection(
            page.project_id,
            KnowledgeProjectionKind::KnowledgePage,
            page.id,
            page.revision,
            &page.content_hash,
        )
        .await?;
        Ok(())
    }
}

impl PostgresStore {
    /// Bounded operator listing including drafts and withdrawn heads.
    pub async fn knowledge_pages_page(
        &self,
        project_id: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<KnowledgePage>, StorageError> {
        if !(1..=101).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "knowledge page bound must be 1 to 101".into(),
            });
        }
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM knowledge_pages WHERE project_id=$1 AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3")
            .bind(project_id.as_uuid()).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        values.iter().map(|value| decode_page(value)).collect()
    }

    /// Stable revision cursor avoids truncating long-lived publication histories.
    pub async fn knowledge_page_revisions_page(
        &self,
        project_id: ProjectId,
        id: Uuid,
        after_revision: u64,
        limit: u32,
    ) -> Result<Vec<KnowledgePage>, StorageError> {
        if !(1..=101).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "knowledge history bound must be 1 to 101".into(),
            });
        }
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM knowledge_page_revisions WHERE project_id=$1 AND page_id=$2 AND revision>$3 ORDER BY revision LIMIT $4")
            .bind(project_id.as_uuid()).bind(id).bind(u64_to_i64(after_revision,"knowledge.after_revision")?).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        values.iter().map(|value| decode_page(value)).collect()
    }

    /// Loads the canonical head within explicit Project scope.
    pub async fn load_knowledge_page(
        &self,
        project_id: ProjectId,
        id: Uuid,
    ) -> Result<Option<KnowledgePage>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM knowledge_pages WHERE project_id=$1 AND id=$2",
        )
        .bind(project_id.as_uuid())
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        value.as_deref().map(decode_page).transpose()
    }

    /// Returns immutable revisions in order. Callers bound page sizes by the domain limit.
    pub async fn knowledge_page_history(
        &self,
        project_id: ProjectId,
        id: Uuid,
    ) -> Result<Vec<KnowledgePage>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM knowledge_page_revisions WHERE project_id=$1 AND page_id=$2 ORDER BY revision LIMIT 1024")
            .bind(project_id.as_uuid()).bind(id).fetch_all(&self.pool).await?;
        values.iter().map(|value| decode_page(value)).collect()
    }

    /// All current published pages. Authority assembly must detect its own context overflow.
    pub async fn list_published_knowledge_pages(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<KnowledgePage>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM knowledge_pages WHERE project_id=$1 AND status='published' ORDER BY kind,id")
            .bind(project_id.as_uuid()).fetch_all(&self.pool).await?;
        values.iter().map(|value| decode_page(value)).collect()
    }
}
