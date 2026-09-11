use forge_domain::ProjectId;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{PostgresStore, StorageError, StorageTransaction, enum_text, i64_to_u64, u64_to_i64};

/// Canonical object represented by one immutable projection identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeProjectionKind {
    KnowledgePage,
    DerivedMemory,
}

/// A UUID is assigned once at canonical commit and reused for every index retry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeProjectionOperation {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub object_kind: KnowledgeProjectionKind,
    pub object_id: Uuid,
    pub revision: u64,
    pub content_hash: String,
    pub attempt_count: u32,
    pub index_ready: bool,
    pub retirement_requested: bool,
    pub retired: bool,
}

impl StorageTransaction<'_> {
    pub(super) async fn enqueue_knowledge_projection(
        &mut self,
        project: ProjectId,
        kind: KnowledgeProjectionKind,
        id: Uuid,
        revision: u64,
        hash: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE knowledge_projection_operations SET index_ready=false,retirement_requested=true,not_before=clock_timestamp() WHERE project_id=$1 AND object_kind=$2 AND object_id=$3 AND revision<$4")
            .bind(project.as_uuid()).bind(enum_text(&kind,"projection.kind")?).bind(id)
            .bind(u64_to_i64(revision,"projection.revision")?).execute(&mut *self.transaction).await?;
        sqlx::query("INSERT INTO knowledge_projection_operations(id,project_id,object_kind,object_id,revision,content_hash) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(object_kind,object_id,revision) DO NOTHING")
            .bind(Uuid::now_v7()).bind(project.as_uuid()).bind(enum_text(&kind,"projection.kind")?).bind(id)
            .bind(u64_to_i64(revision,"projection.revision")?).bind(hash).execute(&mut *self.transaction).await?;
        if kind == KnowledgeProjectionKind::KnowledgePage {
            // Source publication/withdrawal can change eligibility without changing
            // the derived record itself. Recheck current dependents with their same IDs.
            sqlx::query("UPDATE knowledge_projection_operations o SET index_ready=false,retired=false,not_before=clock_timestamp() FROM derived_memory_entries d WHERE o.project_id=$1 AND o.object_kind='derived_memory' AND o.object_id=d.id AND o.revision=d.revision AND EXISTS(SELECT 1 FROM jsonb_array_elements(d.canonical_snapshot->'source_refs') source WHERE source->>'kind'='knowledge_page' AND source->>'page_id'=$2)")
                .bind(project.as_uuid()).bind(id.to_string()).execute(&mut *self.transaction).await?;
            sqlx::query("UPDATE knowledge_projection_operations o SET index_ready=false,retired=false,not_before=clock_timestamp() FROM knowledge_pages p WHERE o.project_id=$1 AND o.object_kind='knowledge_page' AND o.object_id=p.id AND o.revision=p.revision AND EXISTS(SELECT 1 FROM jsonb_array_elements(p.canonical_snapshot->'content'->'source_refs') source WHERE source->>'kind'='knowledge_page' AND source->>'page_id'=$2)")
                .bind(project.as_uuid()).bind(id.to_string()).execute(&mut *self.transaction).await?;
        }
        Ok(())
    }

    /// Only the adapter's strict durable/index-ready acknowledgement may call this.
    pub async fn mark_knowledge_projection_ready(
        &mut self,
        id: Uuid,
        content_hash: &str,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query("UPDATE knowledge_projection_operations SET index_ready=true,last_error_code=NULL WHERE id=$1 AND content_hash=$2 AND NOT index_ready AND NOT retirement_requested AND NOT retired")
            .bind(id).bind(content_hash).execute(&mut *self.transaction).await?;
        Ok(result.rows_affected() == 1)
    }

    /// Serializes only external projection I/O across Core instances. No canonical
    /// row lock is held during the HTTP call; a crash releases this transaction lock.
    pub async fn try_lock_knowledge_projection_delivery(&mut self) -> Result<bool, StorageError> {
        Ok(
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(5780476418234003)")
                .fetch_one(&mut *self.transaction)
                .await?,
        )
    }

    /// A successful forget is retirement, never an indexing acknowledgement.
    pub async fn retire_knowledge_projection(&mut self, id: Uuid) -> Result<(), StorageError> {
        sqlx::query("UPDATE knowledge_projection_operations SET index_ready=false,retired=true,last_error_code=NULL WHERE id=$1")
            .bind(id).execute(&mut *self.transaction).await?;
        Ok(())
    }

    /// Stores a bounded classification, never an external response or source content.
    pub async fn fail_knowledge_projection(
        &mut self,
        id: Uuid,
        error_code: &str,
    ) -> Result<(), StorageError> {
        if error_code.is_empty()
            || error_code.len() > 100
            || !error_code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(StorageError::InvalidInput {
                reason: "invalid projection error code".into(),
            });
        }
        sqlx::query("UPDATE knowledge_projection_operations SET attempt_count=LEAST(attempt_count::bigint+1,2147483647)::integer,last_error_code=$2,not_before=clock_timestamp()+INTERVAL '5 seconds' WHERE id=$1 AND NOT index_ready")
            .bind(id).bind(error_code).execute(&mut *self.transaction).await?;
        Ok(())
    }

    /// Explicit rebuild preserves every immutable projection UUID.
    pub async fn rebuild_knowledge_projections(
        &mut self,
        project_id: ProjectId,
    ) -> Result<u64, StorageError> {
        let result = sqlx::query("UPDATE knowledge_projection_operations SET index_ready=false,retired=false,last_error_code=NULL,not_before=clock_timestamp() WHERE project_id=$1")
            .bind(project_id.as_uuid()).execute(&mut *self.transaction).await?;
        Ok(result.rows_affected())
    }
}

impl PostgresStore {
    /// Operator diagnostics, not a synthesized indexing acknowledgement.
    pub async fn knowledge_projection_status(
        &self,
        project: ProjectId,
    ) -> Result<serde_json::Value, StorageError> {
        Ok(sqlx::query_scalar("SELECT jsonb_build_object('indexed',count(*) FILTER(WHERE index_ready),'pending',count(*) FILTER(WHERE NOT index_ready AND NOT retired),'retired',count(*) FILTER(WHERE retired),'failed_attempts',COALESCE(sum(attempt_count),0),'canonical_max_revision',GREATEST(COALESCE((SELECT max(revision) FROM knowledge_pages WHERE project_id=$1),0),COALESCE((SELECT max(revision) FROM derived_memory_entries WHERE project_id=$1),0))) FROM knowledge_projection_operations WHERE project_id=$1")
            .bind(project.as_uuid()).fetch_one(&self.pool).await?)
    }

    pub async fn knowledge_onboarding_status(
        &self,
        project: ProjectId,
        employee: forge_domain::EmployeeId,
    ) -> Result<Option<serde_json::Value>, StorageError> {
        Ok(sqlx::query_scalar("SELECT jsonb_build_object('employee_id',employee_id,'state',state,'revision',revision,'job_id',job_id,'receipt',receipt) FROM employee_onboarding WHERE project_id=$1 AND employee_id=$2")
            .bind(project.as_uuid()).bind(employee.as_uuid()).fetch_optional(&self.pool).await?)
    }

    pub async fn derived_memory_revisions_page(
        &self,
        project: ProjectId,
        id: Uuid,
        after: u64,
        limit: u32,
    ) -> Result<Vec<forge_domain::knowledge::DerivedMemoryEntry>, StorageError> {
        validate_limit(limit)?;
        let values:Vec<String>=sqlx::query_scalar("SELECT canonical_snapshot::text FROM derived_memory_revisions WHERE project_id=$1 AND entry_id=$2 AND revision>$3 ORDER BY revision LIMIT $4")
            .bind(project.as_uuid()).bind(id).bind(u64_to_i64(after,"memory.after_revision")?).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        values
            .iter()
            .map(|value| super::derived::decode_entry(value))
            .collect()
    }

    pub async fn load_knowledge_projection(
        &self,
        project_id: ProjectId,
        id: Uuid,
    ) -> Result<Option<KnowledgeProjectionOperation>, StorageError> {
        sqlx::query("SELECT * FROM knowledge_projection_operations WHERE project_id=$1 AND id=$2")
            .bind(project_id.as_uuid())
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .map(decode_operation)
            .transpose()
    }

    /// Cyclic UUID cursor prevents old failed records from starving later projects.
    pub async fn pending_knowledge_projections_global(
        &self,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<KnowledgeProjectionOperation>, StorageError> {
        validate_limit(limit)?;
        sqlx::query("SELECT * FROM knowledge_projection_operations WHERE NOT index_ready AND NOT retired AND not_before<=clock_timestamp() ORDER BY (id>COALESCE($1,'00000000-0000-0000-0000-000000000000'::uuid)) DESC,id LIMIT $2")
            .bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?
            .into_iter().map(decode_operation).collect()
    }

    pub async fn pending_knowledge_projections(
        &self,
        project_id: ProjectId,
        limit: u32,
    ) -> Result<Vec<KnowledgeProjectionOperation>, StorageError> {
        validate_limit(limit)?;
        let rows = sqlx::query("SELECT * FROM knowledge_projection_operations WHERE project_id=$1 AND NOT index_ready AND NOT retired AND not_before<=clock_timestamp() ORDER BY id LIMIT $2")
            .bind(project_id.as_uuid()).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        rows.into_iter().map(decode_operation).collect()
    }
}

fn validate_limit(limit: u32) -> Result<(), StorageError> {
    if !(1..=100).contains(&limit) {
        return Err(StorageError::InvalidInput {
            reason: "projection batch must contain 1 to 100 items".into(),
        });
    }
    Ok(())
}

fn decode_operation(
    row: sqlx::postgres::PgRow,
) -> Result<KnowledgeProjectionOperation, StorageError> {
    let kind: String = row.try_get("object_kind")?;
    let object_kind = match kind.as_str() {
        "knowledge_page" => KnowledgeProjectionKind::KnowledgePage,
        "derived_memory" => KnowledgeProjectionKind::DerivedMemory,
        _ => {
            return Err(StorageError::InvalidInput {
                reason: "unknown projection kind".into(),
            });
        }
    };
    Ok(KnowledgeProjectionOperation {
        id: row.try_get("id")?,
        project_id: ProjectId::from(row.try_get::<Uuid, _>("project_id")?),
        object_kind,
        object_id: row.try_get("object_id")?,
        revision: i64_to_u64(row.try_get("revision")?, "projection.revision")?,
        content_hash: row.try_get("content_hash")?,
        attempt_count: crate::i32_to_u32(
            row.try_get("attempt_count")?,
            "projection.attempt_count",
        )?,
        index_ready: row.try_get("index_ready")?,
        retirement_requested: row.try_get("retirement_requested")?,
        retired: row.try_get("retired")?,
    })
}
