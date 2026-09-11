use forge_domain::{
    EmployeeId, ProjectId,
    knowledge::{DerivedMemoryEntry, MemoryScope},
};
use uuid::Uuid;

use super::{KnowledgeProjectionKind, validate_digest};
use crate::{PostgresStore, StorageError, StorageTransaction, encode_snapshot, u64_to_i64};

pub(super) fn decode_entry(value: &str) -> Result<DerivedMemoryEntry, StorageError> {
    let entry: DerivedMemoryEntry =
        serde_json::from_str(value).map_err(|source| StorageError::Snapshot {
            aggregate: "derived_memory",
            source,
        })?;
    entry
        .validate()
        .map_err(|source| StorageError::SnapshotInvariant {
            aggregate: "derived_memory",
            source,
        })?;
    validate_digest(&entry.markdown, &entry.content_hash)?;
    Ok(entry)
}

impl StorageTransaction<'_> {
    pub async fn load_derived_memory_entry(
        &mut self,
        project_id: ProjectId,
        id: Uuid,
    ) -> Result<Option<DerivedMemoryEntry>, StorageError> {
        let value: Option<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM derived_memory_entries WHERE project_id=$1 AND id=$2 FOR UPDATE")
            .bind(project_id.as_uuid()).bind(id).fetch_optional(&mut *self.transaction).await?;
        value.as_deref().map(decode_entry).transpose()
    }

    /// Exact retries are no-ops; changed content under an existing immutable revision is refused.
    pub async fn insert_derived_memory_revision(
        &mut self,
        entry: &DerivedMemoryEntry,
    ) -> Result<bool, StorageError> {
        entry
            .validate()
            .map_err(|source| StorageError::SnapshotInvariant {
                aggregate: "derived_memory",
                source,
            })?;
        validate_digest(&entry.markdown, &entry.content_hash)?;
        let previous = self
            .load_derived_memory_entry(entry.project_id, entry.id)
            .await?;
        if let Some(previous) = &previous {
            if previous == entry {
                return Ok(false);
            }
            if previous.subject != entry.subject
                || entry.revision
                    != previous
                        .revision
                        .checked_add(1)
                        .ok_or(StorageError::IntegerOutOfRange {
                            field: "memory.revision",
                        })?
            {
                return Err(StorageError::StaleRevision {
                    aggregate: "derived_memory",
                });
            }
        } else if entry.revision != 1 {
            return Err(StorageError::StaleRevision {
                aggregate: "derived_memory",
            });
        }
        let snapshot = encode_snapshot(entry, "derived_memory")?;
        let employee = match entry.subject.scope() {
            MemoryScope::Personal { employee_id } => Some(employee_id.as_uuid()),
            MemoryScope::Project => None,
        };
        let affected = if let Some(previous) = previous {
            sqlx::query("UPDATE derived_memory_entries SET revision=$3,content_hash=$4,withdrawn=$5,canonical_snapshot=$6::jsonb WHERE project_id=$1 AND id=$2 AND revision=$7")
                .bind(entry.project_id.as_uuid()).bind(entry.id).bind(u64_to_i64(entry.revision,"memory.revision")?)
                .bind(&entry.content_hash).bind(entry.withdrawn).bind(&snapshot).bind(u64_to_i64(previous.revision,"memory.revision")?)
                .execute(&mut *self.transaction).await?.rows_affected()
        } else {
            sqlx::query("INSERT INTO derived_memory_entries(id,project_id,revision,employee_id,task_id,content_hash,withdrawn,canonical_snapshot) VALUES($1,$2,$3,$4,$5,$6,$7,$8::jsonb) ON CONFLICT DO NOTHING")
                .bind(entry.id).bind(entry.project_id.as_uuid()).bind(u64_to_i64(entry.revision,"memory.revision")?)
                .bind(employee).bind(entry.subject.task_id().map(|task| task.as_uuid())).bind(&entry.content_hash)
                .bind(entry.withdrawn).bind(&snapshot).execute(&mut *self.transaction).await?.rows_affected()
        };
        if affected != 1 {
            return Err(StorageError::StaleRevision {
                aggregate: "derived_memory",
            });
        }
        sqlx::query("INSERT INTO derived_memory_revisions(entry_id,project_id,revision,canonical_snapshot) VALUES($1,$2,$3,$4::jsonb)")
            .bind(entry.id).bind(entry.project_id.as_uuid()).bind(u64_to_i64(entry.revision,"memory.revision")?)
            .bind(snapshot).execute(&mut *self.transaction).await?;
        self.enqueue_knowledge_projection(
            entry.project_id,
            KnowledgeProjectionKind::DerivedMemory,
            entry.id,
            entry.revision,
            &entry.content_hash,
        )
        .await?;
        Ok(true)
    }
}

impl PostgresStore {
    /// Canonical read for the Core only. Gateway callers must additionally recheck viewer scope.
    pub async fn load_derived_memory_entry(
        &self,
        project_id: ProjectId,
        id: Uuid,
    ) -> Result<Option<DerivedMemoryEntry>, StorageError> {
        let value: Option<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM derived_memory_entries WHERE project_id=$1 AND id=$2")
            .bind(project_id.as_uuid()).bind(id).fetch_optional(&self.pool).await?;
        value.as_deref().map(decode_entry).transpose()
    }

    /// Bounded canonical fallback; only this Employee's private entries join project entries.
    pub async fn visible_derived_memory(
        &self,
        project_id: ProjectId,
        employee_id: Option<EmployeeId>,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<DerivedMemoryEntry>, StorageError> {
        if !(1..=100).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "memory page must contain 1 to 100 items".into(),
            });
        }
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM derived_memory_entries WHERE project_id=$1 AND NOT withdrawn AND (employee_id IS NULL OR employee_id=$2) AND ($3::uuid IS NULL OR id>$3) ORDER BY id LIMIT $4")
            .bind(project_id.as_uuid()).bind(employee_id.map(|id| id.as_uuid())).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        values.iter().map(|value| decode_entry(value)).collect()
    }

    pub async fn derived_memory_history(
        &self,
        project_id: ProjectId,
        id: Uuid,
    ) -> Result<Vec<DerivedMemoryEntry>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM derived_memory_revisions WHERE project_id=$1 AND entry_id=$2 ORDER BY revision LIMIT 1024")
            .bind(project_id.as_uuid()).bind(id).fetch_all(&self.pool).await?;
        values.iter().map(|value| decode_entry(value)).collect()
    }
}
