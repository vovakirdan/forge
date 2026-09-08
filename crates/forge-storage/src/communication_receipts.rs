//! Append-only delivery facts and completion barriers, under Core's Project lock.
use crate::{
    StorageError, StorageTransaction, database_timestamp, decode_snapshot, encode_object,
    u64_to_i64,
};
use forge_domain::{
    ProjectId,
    communication::{DeliveryReceipt, EmployeeMessage, TaskMessageContext},
};
use uuid::Uuid;

impl StorageTransaction<'_> {
    pub async fn insert_message_requirement_waiver(
        &mut self,
        waiver: &forge_domain::communication::MessageRequirementWaiver,
    ) -> Result<(), StorageError> {
        waiver
            .validate_snapshot()
            .map_err(|_| StorageError::InvalidInput {
                reason: "invalid message waiver".into(),
            })?;
        let value = serde_json::to_value(waiver).map_err(|_| StorageError::InvalidInput {
            reason: "invalid waiver snapshot".into(),
        })?;
        sqlx::query("INSERT INTO employee_message_waivers(message_id,project_id,canonical_snapshot,created_at) VALUES($1,$2,$3::jsonb,$4)")
            .bind(waiver.message_id).bind(waiver.project_id.as_uuid()).bind(encode_object(&value,"waiver")?)
            .bind(database_timestamp(waiver.created_at)).execute(&mut *self.transaction).await?;
        Ok(())
    }
    /// Caller holds the Project mutation lock, serializing sends and acceptance.
    pub async fn pending_task_messages(
        &mut self,
        project: ProjectId,
        context: &TaskMessageContext,
    ) -> Result<Vec<Uuid>, StorageError> {
        let value = serde_json::to_value(context).map_err(|_| StorageError::InvalidInput {
            reason: "invalid message context".into(),
        })?;
        Ok(sqlx::query_scalar("SELECT m.id FROM employee_messages m WHERE m.project_id=$1 AND m.target_task_id=$2 AND m.canonical_snapshot->'target'->'context'=$3::jsonb AND m.requirement<>'informational' AND NOT EXISTS (SELECT 1 FROM employee_message_waivers w WHERE w.message_id=m.id) AND NOT EXISTS (SELECT 1 FROM employee_message_receipts r WHERE r.message_id=m.id AND ((m.requirement='acknowledged' AND r.kind IN ('acknowledged','answered')) OR (m.requirement='answered' AND r.kind='answered'))) ORDER BY m.id LIMIT 101")
            .bind(project.as_uuid()).bind(context.task_id.as_uuid()).bind(encode_object(&value,"message.context")?)
            .fetch_all(&mut *self.transaction).await?)
    }

    /// Reads only messages explicitly addressed to this Employee's Task visit.
    /// General Inbox conversations are intentionally excluded.
    pub async fn messages_for_task_delivery(
        &mut self,
        scope: &forge_domain::communication::DeliveryScope,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<EmployeeMessage>, StorageError> {
        let context = scope
            .task_context
            .as_ref()
            .ok_or_else(|| StorageError::InvalidInput {
                reason: "requires task assignment".into(),
            })?;
        let context_json =
            serde_json::to_value(context).map_err(|_| StorageError::InvalidInput {
                reason: "invalid message context".into(),
            })?;
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM employee_messages WHERE project_id=$1 AND employee_id=$2 AND target_task_id=$3 AND canonical_snapshot->'target'->'context'=$4::jsonb AND ($5::uuid IS NULL OR id>$5) AND (target_run_id IS NULL OR (target_run_id=$6 AND canonical_snapshot->'target'->>'fencing_token'=$7 AND canonical_snapshot->'target'->>'environment_epoch'=$8)) ORDER BY id LIMIT $9")
            .bind(scope.project_id.as_uuid()).bind(scope.employee_id.as_uuid()).bind(context.task_id.as_uuid())
            .bind(encode_object(&context_json,"message.context")?).bind(after).bind(scope.run_id)
            .bind(scope.fencing_token.to_string()).bind(scope.environment_epoch.to_string())
            .bind(i64::from(limit.clamp(1,101))).fetch_all(&mut *self.transaction).await?;
        values
            .into_iter()
            .map(|value| decode_snapshot(&value, "employee_message"))
            .collect()
    }

    /// Only Core-created, validated receipts enter this insert-only adapter.
    pub async fn insert_delivery_receipt(
        &mut self,
        receipt: &DeliveryReceipt,
    ) -> Result<bool, StorageError> {
        let value = serde_json::to_value(receipt).map_err(|_| StorageError::InvalidInput {
            reason: "invalid delivery receipt".into(),
        })?;
        let scope = receipt.scope();
        let inserted = sqlx::query("INSERT INTO employee_message_receipts(message_id,project_id,run_id,fencing_token,environment_epoch,kind,reply_id,canonical_snapshot,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8::jsonb,$9) ON CONFLICT DO NOTHING")
            .bind(receipt.message_id()).bind(scope.project_id.as_uuid()).bind(scope.run_id)
            .bind(u64_to_i64(scope.fencing_token,"receipt.fence")?).bind(u64_to_i64(scope.environment_epoch,"receipt.epoch")?)
            .bind(crate::enum_text(&receipt.kind(),"receipt.kind")?).bind(receipt.reply_id())
            .bind(encode_object(&value,"receipt")?).bind(database_timestamp(receipt.created_at()))
            .execute(&mut *self.transaction).await?;
        Ok(inserted.rows_affected() == 1)
    }
}
