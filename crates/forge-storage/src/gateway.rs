//! Run-scoped Gateway authorization and dedupe. This ledger never consumes the
//! independently ordered Supervisor observation/submission sequence.

use forge_domain::{ProjectId, TaskId, runtime::RunScope};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    PostgresStore, RunProjection, StorageError, StorageTransaction, encode_object, u64_to_i64,
};

/// A Gateway invocation whose scope was pinned by Core, not supplied by a worker.
pub struct GatewaySubmissionRecord {
    pub scope: RunScope,
    pub message_id: Uuid,
    pub payload_hash: String,
    pub receipt: Value,
}

#[derive(Debug)]
pub enum GatewayWriteResult {
    Applied,
    Replayed(Value),
    Denied,
    Conflict,
}

impl StorageTransaction<'_> {
    /// Locks in the same Project -> Run -> Lease order used by command paths.
    /// Stop requests revoke new Gateway access before physical stop completes.
    pub async fn validate_gateway_scope(
        &mut self,
        scope: &RunScope,
    ) -> Result<Option<RunProjection>, StorageError> {
        if scope.run_id.get_version_num() != 7
            || scope.fencing_token == 0
            || scope.environment_epoch == 0
        {
            return Err(StorageError::InvalidInput {
                reason: "invalid Gateway scope".to_owned(),
            });
        }
        let project_id: Option<Uuid> =
            sqlx::query_scalar("SELECT project_id FROM runs WHERE id = $1")
                .bind(scope.run_id)
                .fetch_optional(&mut *self.transaction)
                .await?;
        let Some(project_id) = project_id else {
            return Ok(None);
        };
        let enabled: Option<bool> =
            sqlx::query_scalar("SELECT execution_enabled FROM projects WHERE id = $1 FOR UPDATE")
                .bind(project_id)
                .fetch_optional(&mut *self.transaction)
                .await?;
        if enabled != Some(true) {
            return Ok(None);
        }
        let locked: Option<Uuid> = sqlx::query_scalar("SELECT id FROM runs WHERE id = $1 AND lease_fencing_token = $2 AND environment_epoch = $3 AND desired_state IN ('provision_requested','running') AND observed_state NOT IN ('stopped','failed','lost') FOR UPDATE")
            .bind(scope.run_id).bind(u64_to_i64(scope.fencing_token, "gateway.fencing_token")?)
            .bind(u64_to_i64(scope.environment_epoch, "gateway.environment_epoch")?)
            .fetch_optional(&mut *self.transaction).await?;
        if locked.is_none() {
            return Ok(None);
        }
        let Some(run) = self.load_run(scope.run_id).await? else {
            return Ok(None);
        };
        if run.assignment.hook().is_some() || run.employee_id.is_none() {
            return Ok(None);
        }
        let lease: Option<Uuid> = sqlx::query_scalar("SELECT l.id FROM leases l JOIN runs r ON forge_lease_owns_run(l,r) WHERE r.id=$1 AND l.lease_state='active' FOR UPDATE OF l")
            .bind(run.id)
            .fetch_optional(&mut *self.transaction).await?;
        Ok(lease.map(|_| run))
    }

    /// Shared current-authority predicate used by Gateway and egress revocation.
    pub async fn gateway_scope_is_active(&mut self, scope: RunScope) -> Result<bool, StorageError> {
        Ok(self.validate_gateway_scope(&scope).await?.is_some())
    }

    /// Looks up an immutable receipt without granting current execution scope.
    pub async fn gateway_submission_receipt(
        &mut self,
        scope: &RunScope,
        message_id: Uuid,
        payload_hash: &str,
    ) -> Result<GatewayWriteResult, StorageError> {
        let row = sqlx::query("SELECT run_id, fencing_token, environment_epoch, payload_hash, receipt FROM gateway_submissions WHERE message_id = $1")
            .bind(message_id).fetch_optional(&mut *self.transaction).await?;
        let Some(row) = row else {
            return Ok(GatewayWriteResult::Applied);
        };
        if row.try_get::<Uuid, _>("run_id")? != scope.run_id
            || row.try_get::<i64, _>("fencing_token")?
                != u64_to_i64(scope.fencing_token, "gateway.fencing_token")?
            || row.try_get::<i64, _>("environment_epoch")?
                != u64_to_i64(scope.environment_epoch, "gateway.environment_epoch")?
            || row.try_get::<String, _>("payload_hash")? != payload_hash
        {
            return Ok(GatewayWriteResult::Conflict);
        }
        Ok(GatewayWriteResult::Replayed(row.try_get("receipt")?))
    }

    /// Convenience form for named Core signals. Hash or scope reuse fails closed.
    pub async fn gateway_receipt(
        &mut self,
        scope: RunScope,
        message_id: Uuid,
        payload_hash: &str,
    ) -> Result<Option<Value>, StorageError> {
        match self
            .gateway_submission_receipt(&scope, message_id, payload_hash)
            .await?
        {
            GatewayWriteResult::Replayed(receipt) => Ok(Some(receipt)),
            GatewayWriteResult::Applied => Ok(None),
            _ => Err(StorageError::InvalidInput {
                reason: "Gateway message identity was reused with different scope or payload"
                    .to_owned(),
            }),
        }
    }

    /// Reserves the message identity in the same transaction as its named Core
    /// mutation, Event and outbox writes. Semantic rejection rolls it back.
    pub async fn record_gateway_submission(
        &mut self,
        input: &GatewaySubmissionRecord,
    ) -> Result<GatewayWriteResult, StorageError> {
        if input.message_id.get_version_num() != 7
            || input.payload_hash.len() != 64
            || !input.payload_hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(StorageError::InvalidInput {
                reason: "invalid Gateway receipt identity or digest".to_owned(),
            });
        }
        if !self.gateway_scope_is_active(input.scope).await? {
            return Ok(GatewayWriteResult::Denied);
        }
        let replay = self
            .gateway_submission_receipt(&input.scope, input.message_id, &input.payload_hash)
            .await?;
        if !matches!(replay, GatewayWriteResult::Applied) {
            return Ok(replay);
        }
        let inserted: Option<Uuid> = sqlx::query_scalar("INSERT INTO gateway_submissions (message_id,run_id,fencing_token,environment_epoch,payload_hash,receipt) VALUES ($1,$2,$3,$4,$5,$6::jsonb) ON CONFLICT DO NOTHING RETURNING message_id")
            .bind(input.message_id).bind(input.scope.run_id)
            .bind(u64_to_i64(input.scope.fencing_token, "gateway.fencing_token")?).bind(u64_to_i64(input.scope.environment_epoch, "gateway.environment_epoch")?)
            .bind(&input.payload_hash).bind(encode_object(&input.receipt,"gateway.receipt")?)
            .fetch_optional(&mut *self.transaction).await?;
        if inserted.is_some() {
            Ok(GatewayWriteResult::Applied)
        } else {
            self.gateway_submission_receipt(&input.scope, input.message_id, &input.payload_hash)
                .await
        }
    }
}

impl PostgresStore {
    /// A bounded Project-scoped page of canonical Task evidence previews. The
    /// preview is explicitly not a complete document or an acceptance verdict.
    pub async fn gateway_task_artifacts(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        after: Option<Uuid>,
    ) -> Result<Vec<Value>, StorageError> {
        let rows=sqlx::query("SELECT id,kind,title,stage_id,left(body_json::text,4096) AS body_preview,coalesce(length(body_json::text)>4096,false) AS truncated FROM artifacts WHERE project_id=$1 AND task_id=$2 AND ($3::uuid IS NULL OR id>$3) ORDER BY id LIMIT 8")
            .bind(project_id.as_uuid()).bind(task_id.as_uuid()).bind(after).fetch_all(&self.pool).await?;
        rows.into_iter().map(|row|Ok(serde_json::json!({"artifact_id":row.try_get::<Uuid,_>("id")?,"kind":row.try_get::<String,_>("kind")?,"title":row.try_get::<String,_>("title")?,"stage_id":row.try_get::<Option<String>,_>("stage_id")?,"body_preview":row.try_get::<Option<String>,_>("body_preview")?,"preview_truncated":row.try_get::<bool,_>("truncated")?}))).collect()
    }

    /// Bounded safe board projection: no RunSpec, secrets, environment paths or
    /// raw evidence are selected into this read model.
    pub async fn gateway_board(
        &self,
        project_id: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<Value>, StorageError> {
        if !(1..=100).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "board limit must be between 1 and 100".to_owned(),
            });
        }
        let rows = sqlx::query("SELECT id,task_key,title,lifecycle,current_stage_id,revision FROM tasks WHERE project_id = $1 AND ($2::uuid IS NULL OR id > $2) ORDER BY id LIMIT $3")
            .bind(project_id.as_uuid()).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        rows.into_iter().map(|row| Ok(serde_json::json!({
            "task_id": row.try_get::<Uuid,_>("id")?, "task_key": row.try_get::<String,_>("task_key")?,
            "title": row.try_get::<String,_>("title")?, "lifecycle": row.try_get::<String,_>("lifecycle")?,
            "stage_id": row.try_get::<Option<String>,_>("current_stage_id")?, "revision": row.try_get::<i64,_>("revision")?,
        }))).collect()
    }
}
