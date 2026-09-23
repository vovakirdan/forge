//! Technical evidence receipts stay separate from accepted Task artifacts.

use crate::{PostgresStore, StorageError, StorageTransaction, encode_object};
use forge_domain::{EvidenceObject, ProjectId};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

impl PostgresStore {
    /// Selects only allowlisted scalar coordinates from a TaskStage context.
    /// Raw TaskSpec, prompts, handoffs and knowledge never leave PostgreSQL.
    pub async fn run_context_coordinates(
        &self,
        project: ProjectId,
        run_id: Uuid,
    ) -> Result<Option<Value>, StorageError> {
        Ok(sqlx::query_scalar("SELECT CASE WHEN r.purpose='task_stage' \
            AND r.context_manifest->>'project_id'=r.project_id::text \
            AND r.context_manifest->>'run_id'=r.id::text \
            AND r.context_manifest->>'task_id'=r.task_id::text \
            AND r.context_manifest->>'employee_id'=r.employee_id::text \
            AND r.context_manifest ? 'context_snapshot_id' \
            THEN jsonb_build_object( \
              'context_snapshot_id',r.context_manifest->'context_snapshot_id', \
              'run_id',r.id,'project_id',r.project_id,'task_id',r.task_id, \
              'employee_id',r.employee_id,'pipeline_version_id',r.context_manifest->'pipeline_version_id', \
              'stage_id',r.stage_id,'stage_visit',r.context_manifest->'stage_visit', \
              'task_revision_before_dispatch',r.context_manifest->'task_revision_before_dispatch', \
              'run_spec_id',r.context_manifest->'run_spec_id') ELSE NULL END \
            FROM runs r WHERE r.project_id=$1 AND r.id=$2")
            .bind(project.as_uuid()).bind(run_id).fetch_optional(&self.pool).await?.flatten())
    }

    /// One bounded Project/Run evidence receipt page; object-store bytes are not read.
    pub async fn run_evidence_page(
        &self,
        project: ProjectId,
        run_id: Uuid,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<(EvidenceObject, bool)>, StorageError> {
        if !(1..=51).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "evidence page limit must be 1..51".into(),
            });
        }
        let rows: Vec<(String, bool)> = sqlx::query_as(
            "SELECT e.receipt::text,e.stored FROM run_evidence_objects e \
            JOIN runs r ON r.id=e.run_id WHERE r.project_id=$1 AND r.id=$2 \
            AND ($3::uuid IS NULL OR e.id>$3) ORDER BY e.id LIMIT $4",
        )
        .bind(project.as_uuid())
        .bind(run_id)
        .bind(after)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(value, stored)| {
                let receipt: EvidenceObject =
                    serde_json::from_str(&value).map_err(|source| StorageError::Snapshot {
                        aggregate: "evidence",
                        source,
                    })?;
                if receipt.data().scope.project_id != project
                    || receipt.data().scope.run_id != run_id
                {
                    return Err(StorageError::InvalidInput {
                        reason: "evidence scope mismatch".into(),
                    });
                }
                Ok((receipt, stored))
            })
            .collect()
    }
    /// Bounded operator diagnostics. Auth records, proxy keys and prompts never
    /// participate in this projection; absent measurements remain JSON null.
    pub async fn run_diagnostics(&self, run_id: Uuid) -> Result<Value, StorageError> {
        let row:Value=sqlx::query_scalar("SELECT jsonb_build_object('runtime_report',(SELECT report FROM run_runtime_reports WHERE run_id=$1),'handoff',(SELECT body FROM task_handoffs WHERE run_id=$1),'incidents',COALESCE((SELECT jsonb_agg(to_jsonb(i)) FROM (SELECT id,kind,assessment,created_at FROM run_incidents WHERE run_id=$1 ORDER BY created_at LIMIT 100) i),'[]'::jsonb),'evidence',COALESCE((SELECT jsonb_agg(receipt) FROM (SELECT receipt FROM run_evidence_objects WHERE run_id=$1 ORDER BY id LIMIT 4096) e),'[]'::jsonb),'streams',COALESCE((SELECT jsonb_agg(jsonb_build_object('stream',stream,'incomplete',incomplete)) FROM run_evidence_streams WHERE run_id=$1),'[]'::jsonb),'proxy_usage',(SELECT usage FROM run_proxy_keys WHERE run_id=$1))")
            .bind(run_id).fetch_one(&self.pool).await?;
        let source: Option<Value> =
            sqlx::query_scalar("SELECT descriptor FROM run_git_source_selections WHERE run_id=$1")
                .bind(run_id)
                .fetch_optional(&self.pool)
                .await?;
        let mut row = row;
        row["git_source"] = source.unwrap_or(Value::Null);
        Ok(row)
    }
    pub async fn evidence_import_candidates(
        &self,
        only_run: Option<Uuid>,
    ) -> Result<Vec<Uuid>, StorageError> {
        Ok(sqlx::query_scalar("SELECT r.id FROM runs r JOIN run_environment_reservations e ON e.run_id=r.id WHERE ($1::uuid IS NULL OR r.id=$1) AND e.released_at IS NOT NULL AND r.run_spec_version IN (2,3,4,5,6,7) AND ((SELECT count(*) FROM run_evidence_streams s WHERE s.run_id=r.id)<2 OR (r.purpose<>'hook' AND NOT EXISTS(SELECT 1 FROM run_runtime_reports rr WHERE rr.run_id=r.id))) ORDER BY e.released_at LIMIT 32")
            .bind(only_run).fetch_all(&self.pool).await?)
    }
    pub async fn list_run_evidence(&self, run_id: Uuid) -> Result<Vec<Value>, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT receipt FROM run_evidence_objects WHERE run_id=$1 ORDER BY id LIMIT 4096",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?)
    }
}
impl StorageTransaction<'_> {
    pub async fn runtime_report_recorded(&mut self, run_id: Uuid) -> Result<bool, StorageError> {
        Ok(
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM run_runtime_reports WHERE run_id=$1)")
                .bind(run_id)
                .fetch_one(&mut *self.transaction)
                .await?,
        )
    }
    pub async fn record_runtime_report(
        &mut self,
        run_id: Uuid,
        report: &Value,
    ) -> Result<bool, StorageError> {
        let result=sqlx::query("INSERT INTO run_runtime_reports(run_id,report) VALUES($1,$2::jsonb) ON CONFLICT DO NOTHING").bind(run_id).bind(encode_object(report,"runtime_report")?).execute(&mut *self.transaction).await?;
        Ok(result.rows_affected() == 1)
    }
    pub async fn evidence_stream_imported(
        &mut self,
        run_id: Uuid,
        stream: &str,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM run_evidence_streams WHERE run_id=$1 AND stream=$2)",
        )
        .bind(run_id)
        .bind(stream)
        .fetch_one(&mut *self.transaction)
        .await?)
    }
    pub async fn record_evidence_stream(
        &mut self,
        run_id: Uuid,
        stream: &str,
        incomplete: bool,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO run_evidence_streams(run_id,stream,incomplete) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(run_id).bind(stream).bind(incomplete).execute(&mut *self.transaction).await?;
        Ok(())
    }
    pub async fn record_evidence_object(
        &mut self,
        receipt: &EvidenceObject,
        stored: bool,
    ) -> Result<bool, StorageError> {
        let data = serde_json::to_value(receipt).map_err(|source| StorageError::Snapshot {
            aggregate: "evidence",
            source,
        })?;
        let scope = receipt.data().scope;
        let owns:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs WHERE id=$1 AND project_id=$2 AND ((purpose='task_stage' AND task_id=$3) OR (purpose IN ('communication','resolution','hook','system_job') AND $3::uuid IS NULL)))")
            .bind(scope.run_id).bind(scope.project_id.as_uuid()).bind(scope.task_id.map(|id|id.as_uuid())).fetch_one(&mut *self.transaction).await?;
        if !owns {
            return Err(StorageError::InvalidInput {
                reason: "evidence scope differs from Run owner".into(),
            });
        }
        let result=sqlx::query("INSERT INTO run_evidence_objects(id,run_id,receipt,stored) VALUES($1,$2,$3::jsonb,$4) ON CONFLICT(id) DO UPDATE SET receipt=EXCLUDED.receipt,stored=EXCLUDED.stored WHERE NOT run_evidence_objects.stored AND EXCLUDED.stored RETURNING id")
            .bind(receipt.data().id).bind(receipt.data().scope.run_id).bind(encode_object(&data,"evidence")?).bind(stored).fetch_optional(&mut *self.transaction).await?;
        Ok(result.is_some())
    }
    pub async fn evidence_object_stored(&mut self, id: Uuid) -> Result<Option<bool>, StorageError> {
        let row = sqlx::query("SELECT stored FROM run_evidence_objects WHERE id=$1")
            .bind(id)
            .fetch_optional(&mut *self.transaction)
            .await?;
        row.map(|row| Ok(row.try_get("stored")?)).transpose()
    }
}
