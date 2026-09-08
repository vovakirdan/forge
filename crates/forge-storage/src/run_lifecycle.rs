//! Fenced Run message ordering, observations, queue completion, and lease release.

use forge_domain::{EventId, Timestamp};
use serde_json::{Map, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    RunDesiredState, RunObservedState, StorageError, StorageTransaction, database_timestamp,
    encode_object, enum_text, i64_to_u64, u64_to_i64,
};

/// Outcome of a Run write guarded by Lease fencing and environment epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FencedWrite {
    /// The Run identity, fence, and exact next sequence were accepted.
    Applied,
    /// The message was stale, duplicated, belonged to another Lease epoch, or
    /// arrived after an already-terminal observed state.
    Ignored,
    /// The sequence skipped one or more Run-scoped Supervisor messages.
    SequenceGap {
        /// The only sequence value Core can accept next.
        expected: u64,
        /// The received, non-contiguous sequence value.
        received: u64,
    },
    /// The observation would regress or contradict the observed Run lifecycle.
    InvalidObservedStateTransition {
        /// Current persisted observed state.
        from: RunObservedState,
        /// State carried by the rejected observation.
        to: RunObservedState,
    },
    /// The referenced Run no longer exists.
    Missing,
}

/// An observed Supervisor state event that cannot change Task lifecycle itself.
#[derive(Clone, Debug)]
pub struct ObservedRunUpdate {
    /// Target Run identity.
    pub run_id: Uuid,
    /// Lease fencing token carried by Supervisor.
    pub lease_fencing_token: u64,
    /// Environment epoch carried by Supervisor.
    pub environment_epoch: u64,
    /// Exact next per-Run sequence shared by all Supervisor messages.
    pub sequence: u64,
    /// Observed runtime state.
    pub observed_state: RunObservedState,
    /// Non-authoritative structured details object.
    pub details: Value,
    /// Supervisor-reported event time retained only in observation details.
    pub reported_at: Timestamp,
    /// Authoritative time at which Core received this observation.
    pub received_at: Timestamp,
}

/// Durable dedupe record for an Executor submission before Core validates semantics.
#[derive(Clone, Debug)]
pub struct ExecutorSubmissionRecord {
    /// Globally unique Supervisor message identity.
    pub message_id: Uuid,
    /// Target Run identity.
    pub run_id: Uuid,
    /// Lease fencing token carried by the submission.
    pub lease_fencing_token: u64,
    /// Environment epoch carried by the submission.
    pub environment_epoch: u64,
    /// Exact next per-Run submission sequence shared with observations.
    pub sequence: u64,
    /// Safe receipt object retained only for dedupe/audit.
    pub receipt: Value,
    /// Event that resulted from accepted semantic processing, if already known.
    pub source_event_id: Option<EventId>,
}

impl StorageTransaction<'_> {
    /// Stores a Supervisor observation only when its fence, exact sequence, and
    /// monotonic observed-state transition are current.
    pub async fn record_observed_run_event(
        &mut self,
        update: &ObservedRunUpdate,
    ) -> Result<FencedWrite, StorageError> {
        let row = sqlx::query(
            "SELECT lease_fencing_token, environment_epoch, last_sequence, desired_state, observed_state FROM runs WHERE id = $1 FOR UPDATE",
        )
        .bind(update.run_id)
        .fetch_optional(&mut *self.transaction)
        .await?;
        let Some(row) = row else {
            return Ok(FencedWrite::Missing);
        };

        let token = i64_to_u64(
            row.try_get("lease_fencing_token")?,
            "run.lease_fencing_token",
        )?;
        let epoch = i64_to_u64(row.try_get("environment_epoch")?, "run.environment_epoch")?;
        if token != update.lease_fencing_token || epoch != update.environment_epoch {
            return Ok(FencedWrite::Ignored);
        }

        let last = i64_to_u64(row.try_get("last_sequence")?, "run.last_sequence")?;
        match sequence_decision(last, update.sequence)? {
            SequenceDecision::Apply => {}
            SequenceDecision::Ignored => return Ok(FencedWrite::Ignored),
            SequenceDecision::Gap { expected, received } => {
                return Ok(FencedWrite::SequenceGap { expected, received });
            }
        }

        let current = observed_state_from_text(row.try_get("observed_state")?)?;
        if !observed_state_transition_is_monotonic(current, update.observed_state) {
            return Ok(FencedWrite::InvalidObservedStateTransition {
                from: current,
                to: update.observed_state,
            });
        }

        let state = enum_text(&update.observed_state, "run.observed_state")?;
        let current_state = enum_text(&current, "run.observed_state")?;
        let current_desired = desired_state_from_text(row.try_get("desired_state")?)?;
        let desired = desired_state_after_observation(current_desired, update.observed_state);
        let starts = update.observed_state == RunObservedState::Running;
        let finishes = is_confirmed_terminal_stop(update.observed_state);
        let details = observation_details(&update.details, update.reported_at)?;
        let result = sqlx::query(
            "UPDATE runs SET observed_state = $2, desired_state = $3, observed_details = $4::jsonb, last_sequence = $5, revision = revision + 1, started_at = CASE WHEN $6 THEN COALESCE(started_at, $8) ELSE started_at END, finished_at = CASE WHEN $7 THEN COALESCE(finished_at, $8) ELSE finished_at END WHERE id = $1 AND lease_fencing_token = $9 AND environment_epoch = $10 AND last_sequence = $11 AND observed_state = $12",
        )
        .bind(update.run_id)
        .bind(state)
        .bind(enum_text(&desired, "run.desired_state")?)
        .bind(encode_object(&details, "run.observed_details")?)
        .bind(u64_to_i64(update.sequence, "run.sequence")?)
        .bind(starts)
        .bind(finishes)
        .bind(database_timestamp(canonical_observation_time(update)))
        .bind(u64_to_i64(update.lease_fencing_token, "run.lease_fencing_token")?)
        .bind(u64_to_i64(update.environment_epoch, "run.environment_epoch")?)
        .bind(u64_to_i64(last, "run.last_sequence")?)
        .bind(current_state)
        .execute(&mut *self.transaction)
        .await?;
        fenced_result(&mut self.transaction, update.run_id, result.rows_affected()).await
    }

    /// Records a deduplicated Executor submission only for its current active Lease.
    ///
    /// A delayed message from a released or revoked Lease is ignored before it
    /// can advance the shared Run sequence or create a durable dedupe receipt.
    pub async fn record_executor_submission(
        &mut self,
        submission: &ExecutorSubmissionRecord,
    ) -> Result<FencedWrite, StorageError> {
        let project_id: Option<Uuid> =
            sqlx::query_scalar("SELECT project_id FROM runs WHERE id = $1")
                .bind(submission.run_id)
                .fetch_optional(&mut *self.transaction)
                .await?;
        let Some(project_id) = project_id else {
            return Ok(FencedWrite::Missing);
        };
        let execution_enabled: Option<bool> =
            sqlx::query_scalar("SELECT execution_enabled FROM projects WHERE id = $1 FOR UPDATE")
                .bind(project_id)
                .fetch_optional(&mut *self.transaction)
                .await?;
        let Some(execution_enabled) = execution_enabled else {
            return Ok(FencedWrite::Missing);
        };

        let row = sqlx::query(
            "SELECT project_id, task_id, employee_id, lease_id, lease_fencing_token, environment_epoch, last_sequence, desired_state, observed_state FROM runs WHERE id = $1 AND purpose='task_stage' FOR UPDATE",
        )
        .bind(submission.run_id)
        .fetch_optional(&mut *self.transaction)
        .await?;
        let Some(row) = row else {
            return Ok(FencedWrite::Missing);
        };

        let token = i64_to_u64(
            row.try_get("lease_fencing_token")?,
            "run.lease_fencing_token",
        )?;
        let epoch = i64_to_u64(row.try_get("environment_epoch")?, "run.environment_epoch")?;
        if token != submission.lease_fencing_token || epoch != submission.environment_epoch {
            return Ok(FencedWrite::Ignored);
        }

        let last = i64_to_u64(row.try_get("last_sequence")?, "run.last_sequence")?;
        match sequence_decision(last, submission.sequence)? {
            SequenceDecision::Apply => {}
            SequenceDecision::Ignored => return Ok(FencedWrite::Ignored),
            SequenceDecision::Gap { expected, received } => {
                return Ok(FencedWrite::SequenceGap { expected, received });
            }
        }

        let desired_state = desired_state_from_text(row.try_get("desired_state")?)?;
        if !executor_submission_is_permitted(execution_enabled, desired_state) {
            return Ok(FencedWrite::Ignored);
        }

        let observed_state = observed_state_from_text(row.try_get("observed_state")?)?;
        if is_terminal_observed_state(observed_state) {
            return Ok(FencedWrite::Ignored);
        }

        let active_lease: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM leases WHERE id = $1 AND project_id = $2 AND task_id = $3 AND employee_id = $4 AND fencing_token = $5 AND environment_epoch = $6 AND lease_state = 'active' FOR UPDATE",
        )
        .bind(row.try_get::<Uuid, _>("lease_id")?)
        .bind(row.try_get::<Uuid, _>("project_id")?)
        .bind(row.try_get::<Uuid, _>("task_id")?)
        .bind(row.try_get::<Uuid, _>("employee_id")?)
        .bind(u64_to_i64(token, "run.lease_fencing_token")?)
        .bind(u64_to_i64(epoch, "run.environment_epoch")?)
        .fetch_optional(&mut *self.transaction)
        .await?;
        if active_lease.is_none() {
            return Ok(FencedWrite::Ignored);
        }

        let inserted: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO consumer_dedupe (consumer_name, message_id, source_event_id, receipt) VALUES ('core.executor_submission.v1', $1, $2, $3::jsonb) ON CONFLICT DO NOTHING RETURNING message_id",
        )
        .bind(submission.message_id)
        .bind(submission.source_event_id.map(|id| id.as_uuid()))
        .bind(encode_object(&submission.receipt, "submission.receipt")?)
        .fetch_optional(&mut *self.transaction)
        .await?;
        if inserted.is_none() {
            return Ok(FencedWrite::Ignored);
        }

        let result = sqlx::query(
            "UPDATE runs SET last_sequence = $2, revision = revision + 1 WHERE id = $1 AND lease_fencing_token = $3 AND environment_epoch = $4 AND last_sequence = $5",
        )
        .bind(submission.run_id)
        .bind(u64_to_i64(submission.sequence, "submission.sequence")?)
        .bind(u64_to_i64(submission.lease_fencing_token, "run.lease_fencing_token")?)
        .bind(u64_to_i64(submission.environment_epoch, "run.environment_epoch")?)
        .bind(u64_to_i64(last, "run.last_sequence")?)
        .execute(&mut *self.transaction)
        .await?;
        fenced_result(
            &mut self.transaction,
            submission.run_id,
            result.rows_affected(),
        )
        .await
    }

    /// Persists a Core stop request without assuming the Supervisor has stopped the Run.
    pub async fn request_run_stop(
        &mut self,
        run_id: Uuid,
        lease_fencing_token: u64,
        environment_epoch: u64,
        force: bool,
    ) -> Result<FencedWrite, StorageError> {
        let desired = if force {
            RunDesiredState::ForceStopRequested
        } else {
            RunDesiredState::StopRequested
        };
        let result = sqlx::query(
            "UPDATE runs SET desired_state = $2, revision = revision + 1 WHERE id = $1 AND lease_fencing_token = $3 AND environment_epoch = $4 AND (desired_state IN ('provision_requested', 'running') OR ($5 AND desired_state = 'stop_requested') OR ($5 AND desired_state IN ('failed','stopped') AND EXISTS(SELECT 1 FROM run_environment_reservations e WHERE e.run_id=runs.id AND e.released_at IS NULL)))",
        )
        .bind(run_id)
        .bind(enum_text(&desired, "run.desired_state")?)
        .bind(u64_to_i64(lease_fencing_token, "run.lease_fencing_token")?)
        .bind(u64_to_i64(environment_epoch, "run.environment_epoch")?)
        .bind(force)
        .execute(&mut *self.transaction)
        .await?;
        fenced_result(&mut self.transaction, run_id, result.rows_affected()).await
    }

    /// Marks the leased queue entry complete after Core accepts an executor stage outcome.
    ///
    /// This deliberately retains the active Lease. The assigned Employee stays
    /// unavailable until [`Self::release_lease_after_terminal_observation`]
    /// observes a fenced `stopped` or `failed` Run state.
    pub async fn complete_queue_after_accepted_stage_outcome(
        &mut self,
        run_id: Uuid,
        lease_fencing_token: u64,
        environment_epoch: u64,
    ) -> Result<FencedWrite, StorageError> {
        let result = sqlx::query(
            "UPDATE queue_entries AS q SET queue_state = 'completed', completed_at = clock_timestamp() FROM runs AS r JOIN leases AS l ON l.id = r.lease_id AND l.project_id = r.project_id AND l.task_id = r.task_id AND l.employee_id = r.employee_id AND l.fencing_token = r.lease_fencing_token AND l.environment_epoch = r.environment_epoch WHERE r.id = $1 AND r.lease_fencing_token = $2 AND r.environment_epoch = $3 AND q.id = r.queue_entry_id AND q.queue_state = 'leased' AND l.lease_state = 'active' AND r.observed_state NOT IN ('stopped', 'failed', 'lost')",
        )
        .bind(run_id)
        .bind(u64_to_i64(lease_fencing_token, "run.lease_fencing_token")?)
        .bind(u64_to_i64(environment_epoch, "run.environment_epoch")?)
        .execute(&mut *self.transaction)
        .await?;
        fenced_result(&mut self.transaction, run_id, result.rows_affected()).await
    }

    /// Releases an active Lease only after a fenced terminal stop observation.
    ///
    /// Queue completion is intentionally independent: a manager-initiated stop
    /// or a lost Run can free a worker without falsely declaring its queue entry
    /// a successful stage outcome.
    pub async fn release_lease_after_terminal_observation(
        &mut self,
        run_id: Uuid,
        lease_fencing_token: u64,
        environment_epoch: u64,
    ) -> Result<FencedWrite, StorageError> {
        let result = sqlx::query(
            "UPDATE leases AS l SET lease_state = 'released', released_at = clock_timestamp(), revision = l.revision + 1 FROM runs AS r WHERE r.id = $1 AND r.lease_id = l.id AND r.lease_fencing_token = $2 AND r.environment_epoch = $3 AND l.fencing_token = r.lease_fencing_token AND l.environment_epoch = r.environment_epoch AND l.lease_state = 'active' AND r.observed_state IN ('stopped', 'failed', 'lost')",
        )
        .bind(run_id)
        .bind(u64_to_i64(lease_fencing_token, "run.lease_fencing_token")?)
        .bind(u64_to_i64(environment_epoch, "run.environment_epoch")?)
        .execute(&mut *self.transaction)
        .await?;
        fenced_result(&mut self.transaction, run_id, result.rows_affected()).await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SequenceDecision {
    Apply,
    Ignored,
    Gap { expected: u64, received: u64 },
}

fn sequence_decision(last: u64, received: u64) -> Result<SequenceDecision, StorageError> {
    let expected = last
        .checked_add(1)
        .ok_or_else(|| StorageError::IntegerOutOfRange {
            field: "run.next_sequence",
        })?;
    if expected > i64::MAX as u64 {
        return Err(StorageError::IntegerOutOfRange {
            field: "run.next_sequence",
        });
    }
    Ok(match received.cmp(&expected) {
        std::cmp::Ordering::Equal => SequenceDecision::Apply,
        std::cmp::Ordering::Less => SequenceDecision::Ignored,
        std::cmp::Ordering::Greater => SequenceDecision::Gap { expected, received },
    })
}

fn observed_state_from_text(value: String) -> Result<RunObservedState, StorageError> {
    serde_json::from_value(Value::String(value)).map_err(|source| StorageError::Snapshot {
        aggregate: "run.observed_state",
        source,
    })
}

fn desired_state_from_text(value: String) -> Result<RunDesiredState, StorageError> {
    serde_json::from_value(Value::String(value)).map_err(|source| StorageError::Snapshot {
        aggregate: "run.desired_state",
        source,
    })
}

fn canonical_observation_time(update: &ObservedRunUpdate) -> Timestamp {
    update.received_at
}

fn observation_details(details: &Value, reported_at: Timestamp) -> Result<Value, StorageError> {
    let Some(details) = details.as_object() else {
        return Err(StorageError::InvalidJsonShape {
            field: "run.observed_details",
            expected: "object",
        });
    };
    let mut enriched: Map<String, Value> = details.clone();
    let reported_at =
        serde_json::to_value(reported_at).map_err(|source| StorageError::Snapshot {
            aggregate: "run.observed_details",
            source,
        })?;
    enriched.insert("reported_observed_at".to_owned(), reported_at);
    Ok(Value::Object(enriched))
}

fn executor_submission_is_permitted(
    execution_enabled: bool,
    desired_state: RunDesiredState,
) -> bool {
    execution_enabled
        && matches!(
            desired_state,
            RunDesiredState::ProvisionRequested | RunDesiredState::Running
        )
}

fn desired_state_after_observation(
    current: RunDesiredState,
    observed: RunObservedState,
) -> RunDesiredState {
    match observed {
        // A late `running` report confirms provisioning, but it must never
        // erase a stop intent committed by Core. The Supervisor transport is
        // asynchronous: Core can request a graceful/forced stop while a
        // previously emitted `running` observation is still in flight.
        RunObservedState::Running
            if matches!(
                current,
                RunDesiredState::ProvisionRequested | RunDesiredState::Running
            ) =>
        {
            RunDesiredState::Running
        }
        RunObservedState::Running => current,
        RunObservedState::Stopped => RunDesiredState::Stopped,
        RunObservedState::Failed => RunDesiredState::Failed,
        // A `lost` report is deliberately not a successful reconciliation:
        // preserve Core's last request so later recovery can explain the gap.
        RunObservedState::Unknown
        | RunObservedState::Provisioning
        | RunObservedState::Stopping
        | RunObservedState::Lost => current,
    }
}

fn observed_state_transition_is_monotonic(from: RunObservedState, to: RunObservedState) -> bool {
    from == to || observed_state_phase(from) < observed_state_phase(to)
        // A later positive physical-stop observation resolves uncertainty/failure;
        // it does not accept another executor result or restart the Run.
        || (matches!(from,RunObservedState::Lost|RunObservedState::Failed) && to==RunObservedState::Stopped)
}

fn observed_state_phase(state: RunObservedState) -> u8 {
    match state {
        RunObservedState::Unknown => 0,
        RunObservedState::Provisioning => 1,
        RunObservedState::Running => 2,
        RunObservedState::Stopping => 3,
        RunObservedState::Stopped | RunObservedState::Failed | RunObservedState::Lost => 4,
    }
}

fn is_terminal_observed_state(state: RunObservedState) -> bool {
    observed_state_phase(state) == 4
}

fn is_confirmed_terminal_stop(state: RunObservedState) -> bool {
    matches!(state, RunObservedState::Stopped | RunObservedState::Failed)
}

pub(crate) async fn fenced_result(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: Uuid,
    rows_affected: u64,
) -> Result<FencedWrite, StorageError> {
    if rows_affected == 1 {
        return Ok(FencedWrite::Applied);
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM runs WHERE id = $1)")
        .bind(run_id)
        .fetch_one(&mut **transaction)
        .await?;
    Ok(if exists {
        FencedWrite::Ignored
    } else {
        FencedWrite::Missing
    })
}

#[cfg(test)]
#[path = "run_lifecycle_tests.rs"]
mod tests;
