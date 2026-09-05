//! Bounded transactional claiming and acknowledgement of durable outbox rows.
//!
//! PostgreSQL remains the delivery authority. A publisher can only reserve an
//! outbox row here; it must receive an external broker acknowledgement before
//! calling [`PostgresStore::mark_outbox_published`].

use std::time::Duration;

use forge_domain::{EventId, ProjectId, Timestamp};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    PostgresStore, StorageError, database_timestamp, domain_timestamp, i32_to_u32, u32_to_i32,
};

const MAX_BATCH_SIZE: u32 = 100;
const MAX_ATTEMPTS: u32 = 100;
const MAX_LEASE_DURATION: Duration = Duration::from_secs(60 * 60);
const MIN_LEASE_DURATION: Duration = Duration::from_secs(1);
const MAX_OWNER_LENGTH: usize = 128;
const MAX_ERROR_CODE_LENGTH: usize = 64;
const CLAIM_OUTBOX_BATCH_SQL: &str = "WITH candidate AS (SELECT id FROM outbox WHERE ((delivery_state = 'pending' AND available_at <= clock_timestamp()) OR (delivery_state = 'leased' AND lock_expires_at <= clock_timestamp())) AND attempt_count < $1 ORDER BY available_at ASC, created_at ASC, id ASC FOR UPDATE SKIP LOCKED LIMIT $2) UPDATE outbox AS o SET delivery_state = 'leased', lock_owner = $3, lock_expires_at = clock_timestamp() + ($4 * interval '1 millisecond'), attempt_count = o.attempt_count + 1 FROM candidate WHERE o.id = candidate.id RETURNING o.id, o.project_id, o.event_id, o.subject, o.envelope::text AS envelope, o.attempt_count, o.lock_owner, o.lock_expires_at";

/// A bounded request to reserve ready durable outbox records for one publisher.
#[derive(Clone, Debug)]
pub struct OutboxClaimRequest {
    /// Per-drain unique owner token. It prevents an expired previous lease from
    /// acknowledging a record claimed by a later drain.
    pub owner: String,
    /// Maximum number of records reserved in one database transaction.
    pub batch_size: u32,
    /// Maximum total publication attempts before a record becomes dead letter.
    pub max_attempts: u32,
    /// How long a publisher owns each record before another publisher may retry it.
    pub lease_duration: Duration,
}

impl OutboxClaimRequest {
    /// Creates a validated bounded outbox reservation request.
    pub fn new(
        owner: String,
        batch_size: u32,
        max_attempts: u32,
        lease_duration: Duration,
    ) -> Result<Self, StorageError> {
        validate_owner(&owner)?;
        if !(1..=MAX_BATCH_SIZE).contains(&batch_size) {
            return Err(StorageError::InvalidInput {
                reason: format!("outbox batch_size must be 1..={MAX_BATCH_SIZE}"),
            });
        }
        if !(1..=MAX_ATTEMPTS).contains(&max_attempts) {
            return Err(StorageError::InvalidInput {
                reason: format!("outbox max_attempts must be 1..={MAX_ATTEMPTS}"),
            });
        }
        if !(MIN_LEASE_DURATION..=MAX_LEASE_DURATION).contains(&lease_duration) {
            return Err(StorageError::InvalidInput {
                reason: "outbox lease_duration must be 1 second through 1 hour".to_owned(),
            });
        }

        Ok(Self {
            owner,
            batch_size,
            max_attempts,
            lease_duration,
        })
    }

    fn lease_duration_millis(&self) -> Result<i64, StorageError> {
        i64::try_from(self.lease_duration.as_millis()).map_err(|_| {
            StorageError::IntegerOutOfRange {
                field: "outbox.lease_duration_millis",
            }
        })
    }
}

/// A record reserved by one outbox publisher transaction.
#[derive(Clone, Debug)]
pub struct OutboxLease {
    /// Durable outbox row identity.
    pub id: Uuid,
    /// Owning Project; retained for observability and consumer routing.
    pub project_id: ProjectId,
    /// Immutable Event identity used as the JetStream deduplication key.
    pub event_id: EventId,
    /// Persisted NATS subject.
    pub subject: String,
    /// Versioned event envelope persisted alongside the Event transaction.
    pub envelope: Value,
    /// Attempt ordinal after this lease was acquired, starting at one.
    pub attempt_count: u32,
    /// Exact unique owner token that must acknowledge or release this lease.
    pub owner: String,
    /// Database-authoritative lease expiration time.
    pub expires_at: Timestamp,
}

/// Result of marking a specific lease after a broker acknowledgement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutboxPublishMark {
    /// The acknowledged broker publication was durably marked published.
    Published,
    /// A later owner reclaimed or otherwise changed the row first.
    LeaseLost,
}

/// Result of releasing a lease after a failed broker publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutboxFailureMark {
    /// The record remains canonical and will be retried at its persisted time.
    RetryScheduled,
    /// The bounded attempt limit was reached and the record is retained as dead letter.
    DeadLettered,
    /// A later owner reclaimed or otherwise changed the row first.
    LeaseLost,
}

impl PostgresStore {
    /// Atomically reserves pending or expired leased records with `SKIP LOCKED`.
    ///
    /// The returned owner is unique to the caller's drain. Expired leases at
    /// their attempt bound become `dead_letter` before new candidates are
    /// selected, so no row can remain permanently leased after a crash.
    pub async fn claim_outbox_batch(
        &self,
        request: &OutboxClaimRequest,
    ) -> Result<Vec<OutboxLease>, StorageError> {
        let mut transaction = self.begin().await?;
        mark_exhausted_expired_leases(&mut transaction.transaction, request.max_attempts).await?;

        let rows = sqlx::query(CLAIM_OUTBOX_BATCH_SQL)
            .bind(u32_to_i32(request.max_attempts, "outbox.max_attempts")?)
            .bind(i64::from(request.batch_size))
            .bind(&request.owner)
            .bind(request.lease_duration_millis()?)
            .fetch_all(&mut *transaction.transaction)
            .await?;
        let claims = rows
            .into_iter()
            .map(outbox_lease_from_row)
            .collect::<Result<_, _>>()?;
        transaction.commit().await?;
        Ok(claims)
    }

    /// Marks an outbox record published only after its JetStream publish acknowledgement.
    pub async fn mark_outbox_published(
        &self,
        lease: &OutboxLease,
    ) -> Result<OutboxPublishMark, StorageError> {
        let result = sqlx::query(
            "UPDATE outbox SET delivery_state = 'published', lock_owner = NULL, lock_expires_at = NULL, published_at = clock_timestamp() WHERE id = $1 AND event_id = $2 AND delivery_state = 'leased' AND lock_owner = $3",
        )
        .bind(lease.id)
        .bind(lease.event_id.as_uuid())
        .bind(&lease.owner)
        .execute(&self.pool)
        .await?;
        Ok(if result.rows_affected() == 1 {
            OutboxPublishMark::Published
        } else {
            OutboxPublishMark::LeaseLost
        })
    }

    /// Releases a failed broker publication for bounded retry or dead letter.
    ///
    /// The raw broker error is never persisted because it can contain topology
    /// details. Callers provide a short stable error code instead.
    pub async fn release_outbox_after_failure(
        &self,
        lease: &OutboxLease,
        error_code: &str,
        retry_at: Timestamp,
        max_attempts: u32,
    ) -> Result<OutboxFailureMark, StorageError> {
        validate_error_code(error_code)?;
        let row = sqlx::query(
            "UPDATE outbox SET delivery_state = CASE WHEN attempt_count >= $1 THEN 'dead_letter' ELSE 'pending' END, available_at = CASE WHEN attempt_count >= $1 THEN available_at ELSE $2 END, lock_owner = NULL, lock_expires_at = NULL, last_error_code = $3, last_error_at = clock_timestamp() WHERE id = $4 AND event_id = $5 AND delivery_state = 'leased' AND lock_owner = $6 RETURNING delivery_state",
        )
        .bind(u32_to_i32(max_attempts, "outbox.max_attempts")?)
        .bind(database_timestamp(retry_at))
        .bind(error_code)
        .bind(lease.id)
        .bind(lease.event_id.as_uuid())
        .bind(&lease.owner)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(OutboxFailureMark::LeaseLost);
        };
        let state: String = row.try_get("delivery_state")?;
        match state.as_str() {
            "pending" => Ok(OutboxFailureMark::RetryScheduled),
            "dead_letter" => Ok(OutboxFailureMark::DeadLettered),
            _ => Err(StorageError::InvalidInput {
                reason: "outbox failure release returned an unknown delivery state".to_owned(),
            }),
        }
    }
}

async fn mark_exhausted_expired_leases(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    max_attempts: u32,
) -> Result<(), StorageError> {
    sqlx::query(
        "WITH exhausted AS (SELECT id FROM outbox WHERE delivery_state = 'leased' AND lock_expires_at <= clock_timestamp() AND attempt_count >= $1 FOR UPDATE SKIP LOCKED) UPDATE outbox AS o SET delivery_state = 'dead_letter', lock_owner = NULL, lock_expires_at = NULL, last_error_code = 'lease_expired_attempt_limit', last_error_at = clock_timestamp() FROM exhausted WHERE o.id = exhausted.id",
    )
    .bind(u32_to_i32(max_attempts, "outbox.max_attempts")?)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn outbox_lease_from_row(row: sqlx::postgres::PgRow) -> Result<OutboxLease, StorageError> {
    let envelope: Value =
        serde_json::from_str(&row.try_get::<String, _>("envelope")?).map_err(|source| {
            StorageError::Snapshot {
                aggregate: "outbox.envelope",
                source,
            }
        })?;
    if !envelope.is_object() {
        return Err(StorageError::InvalidJsonShape {
            field: "outbox.envelope",
            expected: "object",
        });
    }
    let owner: Option<String> = row.try_get("lock_owner")?;
    let expires_at = row.try_get("lock_expires_at")?;
    let Some(owner) = owner else {
        return Err(StorageError::InvalidInput {
            reason: "claimed outbox row has no lock owner".to_owned(),
        });
    };
    let Some(expires_at) = expires_at else {
        return Err(StorageError::InvalidInput {
            reason: "claimed outbox row has no lock expiry".to_owned(),
        });
    };
    Ok(OutboxLease {
        id: row.try_get("id")?,
        project_id: row.try_get::<Uuid, _>("project_id")?.into(),
        event_id: row.try_get::<Uuid, _>("event_id")?.into(),
        subject: row.try_get("subject")?,
        envelope,
        attempt_count: i32_to_u32(row.try_get("attempt_count")?, "outbox.attempt_count")?,
        owner,
        expires_at: domain_timestamp(expires_at),
    })
}

fn validate_owner(value: &str) -> Result<(), StorageError> {
    if value.trim().is_empty() || value.len() > MAX_OWNER_LENGTH {
        return Err(StorageError::InvalidInput {
            reason: format!("outbox owner must be non-blank and at most {MAX_OWNER_LENGTH} bytes"),
        });
    }
    Ok(())
}

fn validate_error_code(value: &str) -> Result<(), StorageError> {
    let valid = !value.is_empty()
        && value.len() <= MAX_ERROR_CODE_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    if valid {
        Ok(())
    } else {
        Err(StorageError::InvalidInput {
            reason:
                "outbox error_code must be 1..=64 lowercase ASCII letters, digits, or underscores"
                    .to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{CLAIM_OUTBOX_BATCH_SQL, OutboxClaimRequest};

    #[test]
    fn claim_request_enforces_bounded_batch_and_attempt_limits() {
        let result =
            OutboxClaimRequest::new("core:drain".to_owned(), 101, 3, Duration::from_secs(30));

        assert!(result.is_err());
    }

    #[test]
    fn claim_request_rejects_a_non_unique_blank_owner() {
        let result = OutboxClaimRequest::new("  ".to_owned(), 10, 3, Duration::from_secs(30));

        assert!(result.is_err());
    }

    #[test]
    fn claim_query_reserves_rows_with_skip_locked_and_a_fresh_attempt() {
        assert!(CLAIM_OUTBOX_BATCH_SQL.contains("FOR UPDATE SKIP LOCKED"));
        assert!(CLAIM_OUTBOX_BATCH_SQL.contains("delivery_state = 'leased'"));
        assert!(CLAIM_OUTBOX_BATCH_SQL.contains("attempt_count = o.attempt_count + 1"));
    }
}
