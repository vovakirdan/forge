//! Ciphertext-only repository. Cryptographic and credential policy belongs to Core.

use crate::{
    PostgresStore, StorageError, StorageTransaction, encode_object, i64_to_u64, u64_to_i64,
};
use forge_domain::ProjectId;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

/// Sealed payload excluded from automatic Debug output.
#[derive(Clone)]
pub struct StoredCredential {
    pub secret_id: Uuid,
    pub project_id: ProjectId,
    pub version: u64,
    pub sealed_record: Value,
}

impl StorageTransaction<'_> {
    pub async fn record_secret_cleanup(
        &mut self,
        run_id: Uuid,
        outcome: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO run_secret_cleanup(run_id,outcome) VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(run_id)
        .bind(outcome)
        .execute(&mut *self.transaction)
        .await?;
        Ok(())
    }
    pub async fn auth_cleanup_is_safe(&mut self, run_id: Uuid) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM run_auth_writeback_receipts WHERE run_id=$1 AND outcome IN ('unchanged','updated','conflict_retained','not_materialized'))").bind(run_id).fetch_one(&mut *self.transaction).await?)
    }
    pub async fn auth_writeback_recorded(&mut self, run_id: Uuid) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM run_auth_writeback_receipts WHERE run_id=$1)",
        )
        .bind(run_id)
        .fetch_one(&mut *self.transaction)
        .await?)
    }

    pub async fn record_auth_writeback(
        &mut self,
        run_id: Uuid,
        outcome: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO run_auth_writeback_receipts(run_id,outcome) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(run_id).bind(outcome).execute(&mut *self.transaction).await?;
        Ok(())
    }
    /// Caller holds the Project lock; auth updates serialize only this short CAS.
    pub async fn load_credential(
        &mut self,
        project_id: ProjectId,
        secret_id: Uuid,
    ) -> Result<Option<StoredCredential>, StorageError> {
        let row=sqlx::query("SELECT version,sealed_record::text AS sealed_record FROM provider_credentials WHERE secret_id=$1 AND project_id=$2 FOR UPDATE")
            .bind(secret_id).bind(project_id.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        row.map(|row| {
            let text: String = row.try_get("sealed_record")?;
            Ok(StoredCredential {
                secret_id,
                project_id,
                version: i64_to_u64(row.try_get("version")?, "credential.version")?,
                sealed_record: serde_json::from_str(&text).map_err(|source| {
                    StorageError::Snapshot {
                        aggregate: "sealed credential",
                        source,
                    }
                })?,
            })
        })
        .transpose()
    }

    /// Enrollment creates a new secret identity; no silent replacement of sessions.
    pub async fn insert_credential(
        &mut self,
        record: &StoredCredential,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO provider_credentials(secret_id,project_id,version,sealed_record) VALUES($1,$2,$3,$4::jsonb)")
            .bind(record.secret_id).bind(record.project_id.as_uuid()).bind(u64_to_i64(record.version,"credential.version")?)
            .bind(encode_object(&record.sealed_record,"credential.sealed_record")?).execute(&mut *self.transaction).await?;
        Ok(())
    }

    /// Pins encrypted input in the same transaction as Run creation.
    pub async fn pin_run_credential(
        &mut self,
        run_id: Uuid,
        record: &StoredCredential,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO run_credential_snapshots(run_id,secret_id,version,sealed_record) VALUES($1,$2,$3,$4::jsonb)")
            .bind(run_id).bind(record.secret_id).bind(u64_to_i64(record.version,"credential.version")?)
            .bind(encode_object(&record.sealed_record,"credential.sealed_record")?).execute(&mut *self.transaction).await?;
        Ok(())
    }

    /// Retrieves the original encrypted auth snapshot, never a worker-chosen base.
    pub async fn run_credential(
        &mut self,
        run_id: Uuid,
    ) -> Result<Option<StoredCredential>, StorageError> {
        let row=sqlx::query("SELECT r.project_id,s.secret_id,s.version,s.sealed_record::text AS sealed_record FROM run_credential_snapshots s JOIN runs r ON r.id=s.run_id WHERE s.run_id=$1")
            .bind(run_id).fetch_optional(&mut *self.transaction).await?;
        row.map(|row| {
            let text: String = row.try_get("sealed_record")?;
            Ok(StoredCredential {
                secret_id: row.try_get("secret_id")?,
                project_id: ProjectId::from(row.try_get::<Uuid, _>("project_id")?),
                version: i64_to_u64(row.try_get("version")?, "credential.version")?,
                sealed_record: serde_json::from_str(&text).map_err(|source| {
                    StorageError::Snapshot {
                        aggregate: "sealed credential",
                        source,
                    }
                })?,
            })
        })
        .transpose()
    }

    /// CAS cannot select a winner from filesystem mtime or Run completion time.
    pub async fn replace_credential(
        &mut self,
        record: &StoredCredential,
        expected_version: u64,
    ) -> Result<bool, StorageError> {
        let result=sqlx::query("UPDATE provider_credentials SET version=$3,sealed_record=$4::jsonb,updated_at=clock_timestamp() WHERE secret_id=$1 AND project_id=$2 AND version=$5")
            .bind(record.secret_id).bind(record.project_id.as_uuid()).bind(u64_to_i64(record.version,"credential.version")?)
            .bind(encode_object(&record.sealed_record,"credential.sealed_record")?).bind(u64_to_i64(expected_version,"credential.expected_version")?)
            .execute(&mut *self.transaction).await?;
        Ok(result.rows_affected() == 1)
    }

    /// Preserves encrypted recovery material before any ephemeral file cleanup.
    pub async fn retain_credential_conflict(
        &mut self,
        run_id: Uuid,
        reason: &str,
        recovery: &Value,
    ) -> Result<Uuid, StorageError> {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO credential_writeback_conflicts(id,run_id,reason,recovery) VALUES($1,$2,$3,$4::jsonb)")
            .bind(id).bind(run_id).bind(reason).bind(encode_object(recovery,"credential.recovery")?).execute(&mut *self.transaction).await?;
        Ok(id)
    }
}

impl PostgresStore {
    pub async fn secret_cleanup_candidates(
        &self,
        only_run: Option<Uuid>,
    ) -> Result<Vec<Uuid>, StorageError> {
        Ok(sqlx::query_scalar("SELECT r.id FROM runs r JOIN run_environment_reservations e ON e.run_id=r.id WHERE ($1::uuid IS NULL OR r.id=$1) AND r.run_spec_version IN (2,3,4) AND e.released_at IS NOT NULL AND (SELECT count(*) FROM run_evidence_streams s WHERE s.run_id=r.id AND NOT s.incomplete)=2 AND NOT EXISTS(SELECT 1 FROM run_secret_cleanup c WHERE c.run_id=r.id) ORDER BY e.released_at LIMIT 32").bind(only_run).fetch_all(&self.pool).await?)
    }
}
