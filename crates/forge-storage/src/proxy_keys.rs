//! Durable proxy issuance intent and observations; contains encrypted key material only.

use crate::{StorageError, StorageTransaction, encode_object};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

pub struct StoredProxyKey {
    pub spec: Value,
    pub sealed_key: Value,
    pub key_hash: Option<String>,
    pub revoked: bool,
    /// An issued request may still be executing, including across a Core crash.
    pub issuance_pending: bool,
}

impl StorageTransaction<'_> {
    pub async fn run_proxy_key(
        &mut self,
        run_id: Uuid,
    ) -> Result<Option<StoredProxyKey>, StorageError> {
        let row=sqlx::query("SELECT spec::text,sealed_key::text,key_hash,revoked_at IS NOT NULL AS revoked,issuance_pending FROM run_proxy_keys WHERE run_id=$1 FOR UPDATE")
            .bind(run_id).fetch_optional(&mut *self.transaction).await?;
        row.map(|row| {
            Ok(StoredProxyKey {
                spec: decode(row.try_get("spec")?)?,
                sealed_key: decode(row.try_get("sealed_key")?)?,
                key_hash: row.try_get("key_hash")?,
                revoked: row.try_get("revoked")?,
                issuance_pending: row.try_get("issuance_pending")?,
            })
        })
        .transpose()
    }
    pub async fn insert_proxy_key(
        &mut self,
        run_id: Uuid,
        spec: &Value,
        sealed_key: &Value,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO run_proxy_keys(run_id,spec,sealed_key) VALUES($1,$2::jsonb,$3::jsonb)",
        )
        .bind(run_id)
        .bind(encode_object(spec, "proxy.spec")?)
        .bind(encode_object(sealed_key, "proxy.sealed_key")?)
        .execute(&mut *self.transaction)
        .await?;
        Ok(())
    }
    pub async fn mark_proxy_key(
        &mut self,
        run_id: Uuid,
        key_hash: &str,
        revoked: bool,
        usage: Option<&Value>,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE run_proxy_keys SET key_hash=$2,revoked_at=CASE WHEN $3 THEN COALESCE(revoked_at,clock_timestamp()) ELSE revoked_at END,usage=COALESCE($4::jsonb,usage) WHERE run_id=$1")
            .bind(run_id).bind(key_hash).bind(revoked).bind(usage.map(|value|encode_object(value,"proxy.usage")).transpose()?).execute(&mut *self.transaction).await?;
        Ok(())
    }

    /// Persists uncertainty before issuing an external request. Caller serializes
    /// in-process issuers for this Run; an old true flag cannot be cleared by a retry.
    pub async fn begin_proxy_issuance(&mut self, run_id: Uuid) -> Result<(), StorageError> {
        sqlx::query("UPDATE run_proxy_keys SET issuance_pending=TRUE WHERE run_id=$1")
            .bind(run_id)
            .execute(&mut *self.transaction)
            .await?;
        Ok(())
    }

    /// Clears uncertainty only after this process has observed completion of the
    /// sole outstanding issue, and compensation if the Run was retired meanwhile.
    pub async fn settle_proxy_issuance(&mut self, run_id: Uuid) -> Result<(), StorageError> {
        sqlx::query("UPDATE run_proxy_keys SET issuance_pending=FALSE WHERE run_id=$1")
            .bind(run_id)
            .execute(&mut *self.transaction)
            .await?;
        Ok(())
    }

    /// Bounds repeated ambiguous-issuance cleanup without dropping its durable intent.
    pub async fn defer_proxy_revocation(&mut self, run_id: Uuid) -> Result<(), StorageError> {
        sqlx::query("UPDATE run_proxy_keys SET next_revocation_attempt_at=clock_timestamp()+INTERVAL '30 seconds' WHERE run_id=$1")
            .bind(run_id).execute(&mut *self.transaction).await?;
        Ok(())
    }
}
fn decode(value: String) -> Result<Value, StorageError> {
    serde_json::from_str(&value).map_err(|source| StorageError::Snapshot {
        aggregate: "proxy key",
        source,
    })
}
