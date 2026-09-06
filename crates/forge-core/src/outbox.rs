//! Bounded NATS JetStream delivery of PostgreSQL's transactional outbox.
//!
//! This module deliberately has no background task. A Core daemon owns the
//! scheduling cadence and repeatedly calls [`OutboxPublisher::drain_once`].
//! PostgreSQL stays authoritative if JetStream is unavailable.

use std::time::Duration;

use async_nats::jetstream::{Context, context::Publish};
use forge_domain::{EventId, Timestamp};
use forge_storage::{
    OutboxClaimRequest, OutboxFailureMark, OutboxLease, OutboxPublishMark, PostgresStore,
    StorageError,
};
use thiserror::Error;
use uuid::Uuid;

use crate::CoreService;

const MAX_WORKER_ID_LENGTH: usize = 80;
const MIN_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60 * 60);

/// Validated configuration for one Core outbox publisher instance.
#[derive(Clone, Debug)]
pub struct OutboxPublisherConfig {
    worker_id: String,
    batch_size: u32,
    max_attempts: u32,
    lease_duration: Duration,
    initial_retry_delay: Duration,
    max_retry_delay: Duration,
}

impl OutboxPublisherConfig {
    /// Creates bounded publisher configuration without reading secrets or environment variables.
    pub fn new(
        worker_id: String,
        batch_size: u32,
        max_attempts: u32,
        lease_duration: Duration,
        initial_retry_delay: Duration,
        max_retry_delay: Duration,
    ) -> Result<Self, OutboxError> {
        if worker_id.trim().is_empty() || worker_id.len() > MAX_WORKER_ID_LENGTH {
            return Err(OutboxError::InvalidConfiguration {
                reason: format!(
                    "worker_id must be non-blank and at most {MAX_WORKER_ID_LENGTH} bytes"
                ),
            });
        }
        if !(MIN_RETRY_DELAY..=MAX_RETRY_DELAY).contains(&initial_retry_delay) {
            return Err(OutboxError::InvalidConfiguration {
                reason: "initial_retry_delay must be 100 milliseconds through 1 hour".to_owned(),
            });
        }
        if !(initial_retry_delay..=MAX_RETRY_DELAY).contains(&max_retry_delay) {
            return Err(OutboxError::InvalidConfiguration {
                reason: "max_retry_delay must be at least initial_retry_delay and at most 1 hour"
                    .to_owned(),
            });
        }

        // Reuse the storage boundary's exact claim limits. The generated owner
        // below is longer, so validate it at its longest valid shape here.
        let validation_owner = format!("{worker_id}:{}", Uuid::now_v7());
        OutboxClaimRequest::new(validation_owner, batch_size, max_attempts, lease_duration)?;

        Ok(Self {
            worker_id,
            batch_size,
            max_attempts,
            lease_duration,
            initial_retry_delay,
            max_retry_delay,
        })
    }

    /// Conservative defaults for a local Core daemon.
    pub fn local_default(worker_id: String) -> Result<Self, OutboxError> {
        Self::new(
            worker_id,
            32,
            5,
            Duration::from_secs(30),
            Duration::from_secs(1),
            Duration::from_secs(60),
        )
    }

    fn next_retry_at(&self, attempt_count: u32, now: Timestamp) -> Result<Timestamp, OutboxError> {
        let delay = retry_delay(
            attempt_count,
            self.initial_retry_delay,
            self.max_retry_delay,
        );
        let delay =
            time::Duration::try_from(delay).map_err(|_| OutboxError::InvalidConfiguration {
                reason: "retry delay cannot be represented by the domain clock".to_owned(),
            })?;
        let value = now
            .as_offset_date_time()
            .checked_add(delay)
            .ok_or_else(|| OutboxError::InvalidConfiguration {
                reason: "retry delay overflows the domain clock".to_owned(),
            })?;
        Ok(Timestamp::from_offset_date_time(value))
    }
}

/// Explicit result of one bounded publisher drain.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OutboxDrainReport {
    /// Rows reserved from PostgreSQL for this drain.
    pub claimed: u32,
    /// Rows acknowledged by JetStream and marked published in PostgreSQL.
    pub published: u32,
    /// Rows released for a later bounded retry.
    pub retry_scheduled: u32,
    /// Rows retained as dead letter after their retry budget was exhausted.
    pub dead_lettered: u32,
    /// Rows whose unique lease token was superseded before final marking.
    pub lease_lost: u32,
}

/// Core-owned bridge from durable outbox rows to a JetStream context.
#[derive(Clone)]
pub struct OutboxPublisher {
    store: PostgresStore,
    jetstream: Context,
    config: OutboxPublisherConfig,
    observability: std::sync::Arc<crate::observability::Observability>,
    #[cfg(feature = "test-support")]
    fail_after_broker_ack_for: Option<EventId>,
}

impl OutboxPublisher {
    /// Builds a publisher over an already-connected JetStream context.
    #[must_use]
    pub fn new(store: PostgresStore, jetstream: Context, config: OutboxPublisherConfig) -> Self {
        Self {
            store,
            jetstream,
            config,
            observability: std::sync::Arc::default(),
            #[cfg(feature = "test-support")]
            fail_after_broker_ack_for: None,
        }
    }

    /// Injects one process-crash-equivalent boundary after JetStream has
    /// acknowledged this durable Event and before PostgreSQL marks it
    /// published. Available only to the integration-test feature.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn fail_after_broker_ack_for(mut self, event_id: EventId) -> Self {
        self.fail_after_broker_ack_for = Some(event_id);
        self
    }

    /// Delivers at most one bounded batch and records every resulting state transition.
    ///
    /// A successful JetStream acknowledgement is always awaited before an
    /// outbox row is marked published. If PostgreSQL fails after that point,
    /// a later retry uses the same durable Event ID as `Nats-Msg-Id`.
    pub async fn drain_once(&self) -> Result<OutboxDrainReport, OutboxError> {
        let started = std::time::Instant::now();
        let result = crate::observability::trace_operation(
            None,
            crate::observability::Operation::Outbox,
            self.drain_once_inner(),
        )
        .await;
        self.observability.record(
            crate::observability::Operation::Outbox,
            result.is_ok(),
            started.elapsed(),
        );
        result
    }

    async fn drain_once_inner(&self) -> Result<OutboxDrainReport, OutboxError> {
        let request = OutboxClaimRequest::new(
            self.lease_owner(),
            self.config.batch_size,
            self.config.max_attempts,
            self.config.lease_duration,
        )?;
        let leases = self.store.claim_outbox_batch(&request).await?;
        let mut report = OutboxDrainReport {
            claimed: u32::try_from(leases.len()).map_err(|_| {
                OutboxError::InvalidConfiguration {
                    reason: "claimed outbox batch exceeds u32 report capacity".to_owned(),
                }
            })?,
            ..OutboxDrainReport::default()
        };

        for lease in &leases {
            match self.publish(lease).await {
                Ok(()) => {
                    #[cfg(feature = "test-support")]
                    if self.fail_after_broker_ack_for == Some(lease.event_id) {
                        return Err(OutboxError::InjectedPostPublishFault);
                    }
                    match self.store.mark_outbox_published(lease).await? {
                        OutboxPublishMark::Published => report.published += 1,
                        OutboxPublishMark::LeaseLost => report.lease_lost += 1,
                    }
                }
                Err(failure) => match self
                    .store
                    .release_outbox_after_failure(
                        lease,
                        failure.error_code(),
                        self.config
                            .next_retry_at(lease.attempt_count, Timestamp::now_utc())?,
                        self.config.max_attempts,
                    )
                    .await?
                {
                    OutboxFailureMark::RetryScheduled => report.retry_scheduled += 1,
                    OutboxFailureMark::DeadLettered => report.dead_lettered += 1,
                    OutboxFailureMark::LeaseLost => report.lease_lost += 1,
                },
            }
        }

        Ok(report)
    }

    fn lease_owner(&self) -> String {
        format!("{}:{}", self.config.worker_id, Uuid::now_v7())
    }

    async fn publish(&self, lease: &OutboxLease) -> Result<(), PublishFailure> {
        if !is_publishable_subject(&lease.subject) {
            return Err(PublishFailure::InvalidSubject);
        }
        let payload = serde_json::to_vec(&lease.envelope).map_err(|_| PublishFailure::Serialize)?;
        let publish = Publish::build()
            .payload(payload.into())
            .message_id(nats_message_id(lease.event_id));
        let acknowledgement = self
            .jetstream
            .send_publish(lease.subject.clone(), publish)
            .await
            .map_err(|_| PublishFailure::JetStream)?;
        acknowledgement
            .await
            .map_err(|_| PublishFailure::JetStream)?;
        Ok(())
    }
}

impl CoreService {
    /// Builds an outbox publisher that a daemon loop can drive with `drain_once`.
    #[must_use]
    pub fn outbox_publisher(
        &self,
        jetstream: Context,
        config: OutboxPublisherConfig,
    ) -> OutboxPublisher {
        self.observability.observe_nats(jetstream.clone());
        let mut publisher = OutboxPublisher::new(self.store.clone(), jetstream, config);
        publisher.observability = std::sync::Arc::clone(&self.observability);
        publisher
    }
}

/// Failures from configuration or the canonical outbox state boundary.
#[derive(Debug, Error)]
pub enum OutboxError {
    /// PostgreSQL could not reserve or update a durable outbox row.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// A caller attempted an unbounded or unsafe publisher configuration.
    #[error("invalid outbox publisher configuration: {reason}")]
    InvalidConfiguration {
        /// Safe structural explanation without endpoint or credential data.
        reason: String,
    },

    /// Test-only synthetic interruption after a durable JetStream PubAck.
    #[cfg(feature = "test-support")]
    #[error("test-only interruption after broker acknowledgement")]
    InjectedPostPublishFault,
}

#[derive(Clone, Copy, Debug)]
enum PublishFailure {
    InvalidSubject,
    Serialize,
    JetStream,
}

impl PublishFailure {
    const fn error_code(self) -> &'static str {
        match self {
            Self::InvalidSubject => "invalid_subject",
            Self::Serialize => "envelope_serialization_failed",
            Self::JetStream => "jetstream_publish_failed",
        }
    }
}

fn nats_message_id(event_id: EventId) -> String {
    event_id.to_string()
}

fn is_publishable_subject(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_' || byte == b'-'
        })
}

fn retry_delay(attempt_count: u32, initial: Duration, maximum: Duration) -> Duration {
    let exponent = attempt_count.saturating_sub(1).min(31);
    let multiplier = 1_u32 << exponent;
    initial
        .checked_mul(multiplier)
        .unwrap_or(maximum)
        .min(maximum)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use forge_domain::EventId;

    use super::{OutboxPublisherConfig, is_publishable_subject, nats_message_id, retry_delay};

    #[test]
    fn retry_delay_doubles_then_stops_at_the_explicit_cap() {
        let result = retry_delay(4, Duration::from_secs(1), Duration::from_secs(5));

        assert_eq!(result, Duration::from_secs(5));
    }

    #[test]
    fn publish_subject_rejects_subscription_wildcards() {
        assert!(!is_publishable_subject(
            "forge.v1.project.*.event.task_ready"
        ));
    }

    #[test]
    fn publish_subject_accepts_a_canonical_event_subject() {
        assert!(is_publishable_subject(
            "forge.v1.project.019c.example.event.task_ready"
        ));
    }

    #[test]
    fn jetstream_dedupe_id_is_the_durable_event_id() {
        let event_id = EventId::new();

        assert_eq!(nats_message_id(event_id), event_id.to_string());
    }

    #[test]
    fn configuration_reuses_storage_claim_bounds() {
        let result = OutboxPublisherConfig::new(
            "core".to_owned(),
            101,
            5,
            Duration::from_secs(30),
            Duration::from_secs(1),
            Duration::from_secs(10),
        );

        assert!(result.is_err());
    }
}
