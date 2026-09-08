//! Append-only, exact-Run native input delivery and independent observed receipts.
use crate::{
    PostgresStore, RunProjection, StorageError, StorageTransaction, database_timestamp,
    encode_snapshot, enum_text, i64_to_u64, u64_to_i64,
};
use forge_domain::{
    ProjectId, Timestamp,
    communication::{RuntimeInputAction, RuntimeInputOutcome},
    runtime::RunScope,
};
use sqlx::Row;
use uuid::Uuid;

pub const MAX_RUNTIME_INPUTS: usize = 128;

#[derive(Clone, Debug)]
pub struct StoredRuntimeInput {
    pub id: Uuid,
    pub scope: RunScope,
    pub sequence: u64,
    pub action: RuntimeInputAction,
    pub outcome: Option<RuntimeInputOutcome>,
}

impl PostgresStore {
    pub async fn runtime_input_runs(
        &self,
        project: Option<ProjectId>,
        after: Option<Uuid>,
    ) -> Result<Vec<Uuid>, StorageError> {
        Ok(sqlx::query_scalar("SELECT r.id FROM runs r WHERE r.purpose IN ('task_stage','communication') AND ($1::uuid IS NULL OR r.project_id=$1) AND r.run_spec->'binding'->'execution_profile'->'capability_profile'->'capabilities' ? 'live_input' AND (EXISTS(SELECT 1 FROM run_environment_reservations e WHERE e.run_id=r.id AND e.released_at IS NULL) OR EXISTS(SELECT 1 FROM runtime_input_deliveries d WHERE d.run_id=r.id AND NOT EXISTS(SELECT 1 FROM runtime_input_observations o WHERE o.input_id=d.id))) ORDER BY ($2::uuid IS NULL OR r.id>$2) DESC,r.id LIMIT 256")
            .bind(project.map(ProjectId::as_uuid)).bind(after).fetch_all(&self.pool).await?)
    }
}

impl StorageTransaction<'_> {
    pub async fn runtime_input_ids(&mut self, run: Uuid) -> Result<Vec<Uuid>, StorageError> {
        Ok(sqlx::query_scalar("SELECT id FROM runtime_input_deliveries WHERE run_id=$1 AND kind='message' ORDER BY sequence").bind(run).fetch_all(&mut *self.transaction).await?)
    }
    pub async fn runtime_inputs(
        &mut self,
        run: Uuid,
    ) -> Result<Vec<StoredRuntimeInput>, StorageError> {
        let rows = sqlx::query("SELECT d.id,d.run_id,d.fencing_token,d.environment_epoch,d.sequence,d.canonical_snapshot::text,o.outcome FROM runtime_input_deliveries d LEFT JOIN runtime_input_observations o ON o.input_id=d.id WHERE d.run_id=$1 AND o.input_id IS NULL ORDER BY (d.kind='close_after_turn') DESC,d.sequence LIMIT 129")
            .bind(run).fetch_all(&mut *self.transaction).await?;
        rows.iter().map(decode).collect()
    }

    pub async fn runtime_input(
        &mut self,
        id: Uuid,
    ) -> Result<Option<StoredRuntimeInput>, StorageError> {
        sqlx::query("SELECT d.id,d.run_id,d.fencing_token,d.environment_epoch,d.sequence,d.canonical_snapshot::text,o.outcome FROM runtime_input_deliveries d LEFT JOIN LATERAL (SELECT outcome FROM runtime_input_observations WHERE input_id=d.id ORDER BY (outcome='runtime_accepted') DESC,created_at DESC,receipt_id DESC LIMIT 1) o ON true WHERE d.id=$1")
            .bind(id).fetch_optional(&mut *self.transaction).await?.as_ref().map(decode).transpose()
    }

    /// Excluding scoped identities in SQL makes each bounded sweep advance.
    pub async fn messages_for_native_input(
        &mut self,
        scope: &forge_domain::communication::DeliveryScope,
    ) -> Result<Vec<forge_domain::communication::EmployeeMessage>, StorageError> {
        let context = scope
            .task_context
            .as_ref()
            .ok_or_else(|| StorageError::InvalidInput {
                reason: "native message requires Task visit".into(),
            })?;
        let values:Vec<String>=sqlx::query_scalar("SELECT m.canonical_snapshot::text FROM employee_messages m WHERE m.project_id=$1 AND m.employee_id=$2 AND m.target_task_id=$3 AND m.canonical_snapshot->'target'->'context'=$4::jsonb AND (m.target_run_id IS NULL OR (m.target_run_id=$5 AND m.canonical_snapshot->'target'->>'fencing_token'=$6 AND m.canonical_snapshot->'target'->>'environment_epoch'=$7)) AND NOT (m.canonical_snapshot->'sender'->>'kind'='employee' AND m.canonical_snapshot->'sender'->>'id'=m.employee_id::text) AND NOT EXISTS(SELECT 1 FROM runtime_input_deliveries d WHERE d.run_id=$5 AND d.source_message_id=m.id) ORDER BY m.id LIMIT 128")
            .bind(scope.project_id.as_uuid()).bind(scope.employee_id.as_uuid()).bind(context.task_id.as_uuid()).bind(encode_snapshot(context,"input.context")?).bind(scope.run_id).bind(scope.fencing_token.to_string()).bind(scope.environment_epoch.to_string()).fetch_all(&mut *self.transaction).await?;
        values
            .into_iter()
            .map(|value| decode_snapshot(&value, "input.source"))
            .collect()
    }

    /// Caller holds Project mutation lock and validates current assignment/source.
    pub async fn create_runtime_input(
        &mut self,
        run: &RunProjection,
        action: &RuntimeInputAction,
        now: Timestamp,
    ) -> Result<Option<Uuid>, StorageError> {
        let (kind, source) = match action {
            RuntimeInputAction::Message { source } => ("message", Some(source.data().id)),
            RuntimeInputAction::CloseAfterTurn { .. } => ("close_after_turn", None),
        };
        let id = Uuid::now_v7();
        let inserted=sqlx::query("INSERT INTO runtime_input_deliveries(id,project_id,employee_id,run_id,fencing_token,environment_epoch,source_message_id,kind,canonical_snapshot,created_at) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9::jsonb,$10 WHERE (SELECT count(*) FROM runtime_input_deliveries d WHERE d.run_id=$4 AND d.kind='message' AND NOT EXISTS(SELECT 1 FROM runtime_input_observations o WHERE o.input_id=d.id)) < 128 OR $8='close_after_turn' ON CONFLICT DO NOTHING")
            .bind(id).bind(run.project_id.as_uuid()).bind(run.require_employee_id()?.as_uuid()).bind(run.id)
            .bind(u64_to_i64(run.lease_fencing_token,"input.fence")?).bind(u64_to_i64(run.environment_epoch,"input.epoch")?)
            .bind(source).bind(kind).bind(encode_snapshot(action,"runtime_input")?).bind(database_timestamp(now))
            .execute(&mut *self.transaction).await?;
        Ok((inserted.rows_affected() == 1).then_some(id))
    }

    pub async fn observe_runtime_input(
        &mut self,
        input: Uuid,
        receipt: Uuid,
        outcome: RuntimeInputOutcome,
        now: Timestamp,
    ) -> Result<bool, StorageError> {
        let inserted=sqlx::query("INSERT INTO runtime_input_observations(input_id,receipt_id,outcome,created_at) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING")
            .bind(input).bind(receipt).bind(enum_text(&outcome,"input.outcome")?).bind(database_timestamp(now))
            .execute(&mut *self.transaction).await?;
        Ok(inserted.rows_affected() == 1)
    }

    pub async fn runtime_input_observation(
        &mut self,
        receipt: Uuid,
    ) -> Result<Option<(Uuid, RuntimeInputOutcome)>, StorageError> {
        let row = sqlx::query(
            "SELECT input_id,outcome FROM runtime_input_observations WHERE receipt_id=$1",
        )
        .bind(receipt)
        .fetch_optional(&mut *self.transaction)
        .await?;
        row.map(|row| {
            Ok((
                row.try_get("input_id")?,
                decode_snapshot(
                    &format!("\"{}\"", row.try_get::<String, _>("outcome")?),
                    "input.outcome",
                )?,
            ))
        })
        .transpose()
    }
}

fn decode(row: &sqlx::postgres::PgRow) -> Result<StoredRuntimeInput, StorageError> {
    let outcome: Option<String> = row.try_get("outcome")?;
    Ok(StoredRuntimeInput {
        id: row.try_get("id")?,
        scope: RunScope {
            run_id: row.try_get("run_id")?,
            fencing_token: i64_to_u64(row.try_get("fencing_token")?, "input.fence")?,
            environment_epoch: i64_to_u64(row.try_get("environment_epoch")?, "input.epoch")?,
        },
        sequence: i64_to_u64(row.try_get("sequence")?, "input.sequence")?,
        action: decode_snapshot(
            &row.try_get::<String, _>("canonical_snapshot")?,
            "runtime_input",
        )?,
        outcome: outcome
            .map(|value| decode_snapshot(&format!("\"{value}\""), "input.outcome"))
            .transpose()?,
    })
}

fn decode_snapshot<T: serde::de::DeserializeOwned>(
    value: &str,
    aggregate: &'static str,
) -> Result<T, StorageError> {
    serde_json::from_str(value).map_err(|source| StorageError::Snapshot { aggregate, source })
}
