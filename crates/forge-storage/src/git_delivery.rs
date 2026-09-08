//! Canonical Git intent and observation storage. No filesystem or Git execution.
use crate::{
    PostgresStore, StorageError, StorageTransaction, database_timestamp, encode_snapshot,
    enum_text, u64_to_i64,
};
use forge_domain::{
    EmployeeId, TaskId,
    git_delivery::{GitProposalState, GitStageProposal, TaskGitCandidate},
};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct StoredGitProposal {
    pub proposal: GitStageProposal,
    pub state: GitProposalState,
    pub inspection: Option<GitInspectionRequest>,
    pub result: Option<Value>,
}
#[derive(Clone, Debug)]
pub struct GitInspectionRequest {
    pub command_id: Uuid,
    pub host_id: String,
    pub boot_id: String,
}
impl PostgresStore {
    /// Bounded oldest-first repair scan; state changes still require Project lock.
    pub async fn pending_git_proposal_runs(&self) -> Result<Vec<Uuid>, StorageError> {
        Ok(sqlx::query_scalar("SELECT run_id FROM git_stage_proposals WHERE proposal_state IN ('awaiting_quiescence','inspecting') ORDER BY updated_at,id LIMIT 64")
            .fetch_all(&self.pool).await?)
    }
}
impl StorageTransaction<'_> {
    /// Historical retry receipts, independent of the currently active request.
    pub async fn git_inspection_receipt(
        &mut self,
        proposal_id: Uuid,
        command_id: Uuid,
    ) -> Result<Option<Value>, StorageError> {
        let text:Option<String>=sqlx::query_scalar("SELECT result::text FROM git_inspection_receipts WHERE proposal_id=$1 AND command_id=$2")
            .bind(proposal_id).bind(command_id).fetch_optional(&mut *self.transaction).await?;
        text.map(|text| decode(&text)).transpose()
    }
    /// Busy is safe to retry because inspection cannot execute worker code.
    pub async fn retry_busy_git_inspection(&mut self, id: Uuid) -> Result<bool, StorageError> {
        let updated=sqlx::query("UPDATE git_stage_proposals SET proposal_state='awaiting_quiescence',inspection_command_id=NULL,inspection_host_id=NULL,inspection_boot_id=NULL,inspection_requested_at=NULL,inspection_result=NULL,updated_at=clock_timestamp() WHERE id=$1 AND proposal_state='inspecting' AND inspection_attempts<3")
            .bind(id).execute(&mut *self.transaction).await?;
        Ok(updated.rows_affected() == 1)
    }
    /// Caller serializes all acceptance and manager mutations with the Project lock.
    pub async fn load_git_proposal(
        &mut self,
        run_id: Uuid,
    ) -> Result<Option<StoredGitProposal>, StorageError> {
        let row=sqlx::query("SELECT canonical_snapshot::text,proposal_state,inspection_command_id,inspection_host_id,inspection_boot_id,inspection_result::text AS result FROM git_stage_proposals WHERE run_id=$1 FOR UPDATE")
            .bind(run_id).fetch_optional(&mut *self.transaction).await?;
        row.map(|row| {
            let proposal: GitStageProposal =
                decode(&row.try_get::<String, _>("canonical_snapshot")?)?;
            proposal
                .validate()
                .map_err(|source| StorageError::SnapshotInvariant {
                    aggregate: "Git proposal",
                    source,
                })?;
            let state = serde_json::from_value(Value::String(row.try_get("proposal_state")?))
                .map_err(|source| StorageError::Snapshot {
                    aggregate: "Git proposal state",
                    source,
                })?;
            let command: Option<Uuid> = row.try_get("inspection_command_id")?;
            let inspection = command
                .map(|command_id| {
                    Ok::<_, StorageError>(GitInspectionRequest {
                        command_id,
                        host_id: row.try_get("inspection_host_id")?,
                        boot_id: row.try_get("inspection_boot_id")?,
                    })
                })
                .transpose()?;
            let result = row
                .try_get::<Option<String>, _>("result")?
                .map(|s| decode(&s))
                .transpose()?;
            Ok(StoredGitProposal {
                proposal,
                state,
                inspection,
                result,
            })
        })
        .transpose()
    }
    pub async fn insert_git_proposal(
        &mut self,
        proposal: &GitStageProposal,
    ) -> Result<(), StorageError> {
        proposal
            .validate()
            .map_err(|source| StorageError::SnapshotInvariant {
                aggregate: "Git proposal",
                source,
            })?;
        sqlx::query("INSERT INTO git_stage_proposals(id,project_id,task_id,run_id,surface_id,canonical_snapshot,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6::jsonb,$7,$7)")
            .bind(proposal.id).bind(proposal.project_id.as_uuid()).bind(proposal.task_id.as_uuid())
            .bind(proposal.scope.run_id).bind(proposal.surface_id).bind(encode_snapshot(proposal,"Git proposal")?)
            .bind(database_timestamp(proposal.submitted_at)).execute(&mut *self.transaction).await?;
        Ok(())
    }
    pub async fn set_git_proposal_state(
        &mut self,
        id: Uuid,
        state: GitProposalState,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE git_stage_proposals SET proposal_state=$2,updated_at=clock_timestamp() WHERE id=$1")
            .bind(id).bind(enum_text(&state,"Git proposal state")?).execute(&mut *self.transaction).await?;
        Ok(())
    }
    pub async fn begin_git_inspection(
        &mut self,
        id: Uuid,
        request: &GitInspectionRequest,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE git_stage_proposals SET proposal_state='inspecting',inspection_command_id=$2,inspection_host_id=$3,inspection_boot_id=$4,inspection_attempts=inspection_attempts+1,inspection_requested_at=clock_timestamp(),updated_at=clock_timestamp() WHERE id=$1 AND proposal_state='awaiting_quiescence'")
            .bind(id).bind(request.command_id).bind(&request.host_id).bind(&request.boot_id).execute(&mut *self.transaction).await?;
        Ok(())
    }
    /// The same command is replayed until its response arrives or requires attention.
    pub async fn git_inspection_expired(&mut self, id: Uuid) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT inspection_requested_at < clock_timestamp()-interval '90 seconds' FROM git_stage_proposals WHERE id=$1")
            .bind(id).fetch_one(&mut *self.transaction).await?)
    }
    pub async fn record_git_inspection(
        &mut self,
        id: Uuid,
        result: &Value,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO git_inspection_receipts(command_id,proposal_id,result) SELECT inspection_command_id,id,$2::jsonb FROM git_stage_proposals WHERE id=$1 ON CONFLICT DO NOTHING")
            .bind(id).bind(encode_snapshot(result,"Git inspection result")?).execute(&mut *self.transaction).await?;
        sqlx::query("UPDATE git_stage_proposals SET inspection_result=$2::jsonb,updated_at=clock_timestamp() WHERE id=$1 AND inspection_result IS NULL")
            .bind(id).bind(encode_snapshot(result,"Git inspection result")?).execute(&mut *self.transaction).await?;
        Ok(())
    }
    /// Positive exact physical quiescence is independent of logical lease release.
    pub async fn git_writer_quiescent(
        &mut self,
        proposal: &GitStageProposal,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs r JOIN run_environment_reservations e ON e.run_id=r.id WHERE r.id=$1 AND r.lease_fencing_token=$2 AND r.environment_epoch=$3 AND r.observed_state='stopped' AND e.fencing_token=$2 AND e.environment_epoch=$3 AND e.surface_id=$4 AND e.state='quiescent' AND e.released_at IS NOT NULL)")
            .bind(proposal.scope.run_id).bind(u64_to_i64(proposal.scope.fencing_token,"fence")?)
            .bind(u64_to_i64(proposal.scope.environment_epoch,"epoch")?).bind(proposal.surface_id)
            .fetch_one(&mut *self.transaction).await?)
    }
    pub async fn git_task_contributors(
        &mut self,
        task_id: TaskId,
    ) -> Result<std::collections::BTreeSet<EmployeeId>, StorageError> {
        // Conservative: every Task execution with a writable surface is attributed,
        // including interrupted/failed Runs, regardless of untrusted Git author text.
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT DISTINCT r.employee_id FROM runs r WHERE r.task_id=$1 AND r.run_spec_version=2 AND COALESCE(r.run_spec->'binding'->>'access','read_write')='read_write'")
            .bind(task_id.as_uuid()).fetch_all(&mut *self.transaction).await?;
        Ok(ids.into_iter().map(EmployeeId::from).collect())
    }
    pub async fn insert_task_git_candidate(
        &mut self,
        candidate: &TaskGitCandidate,
    ) -> Result<(), StorageError> {
        candidate
            .validate()
            .map_err(|source| StorageError::SnapshotInvariant {
                aggregate: "Git candidate",
                source,
            })?;
        sqlx::query("INSERT INTO task_git_candidates(proposal_id,project_id,task_id,canonical_snapshot,verified_at) VALUES($1,$2,$3,$4::jsonb,$5) ON CONFLICT(proposal_id) DO NOTHING")
            .bind(candidate.proposal_id).bind(candidate.project_id.as_uuid()).bind(candidate.task_id.as_uuid())
            .bind(encode_snapshot(candidate,"Git candidate")?).bind(database_timestamp(candidate.verified_at))
            .execute(&mut *self.transaction).await?;
        Ok(())
    }
    /// Separate post-quiescence acceptance gate; never reactivates a revoked lease.
    pub async fn retire_git_proposal_queue(
        &mut self,
        proposal: &GitStageProposal,
    ) -> Result<(), StorageError> {
        // Retiring logical queue intent does not release the physical reservation.
        sqlx::query("UPDATE queue_entries q SET queue_state='cancelled',cancelled_at=clock_timestamp() FROM runs r JOIN git_stage_proposals p ON p.run_id=r.id WHERE p.id=$1 AND r.id=$2 AND r.lease_fencing_token=$3 AND r.environment_epoch=$4 AND q.id=r.queue_entry_id AND q.queue_state='leased'")
            .bind(proposal.id).bind(proposal.scope.run_id).bind(u64_to_i64(proposal.scope.fencing_token,"fence")?)
            .bind(u64_to_i64(proposal.scope.environment_epoch,"epoch")?).execute(&mut *self.transaction).await?;
        Ok(())
    }
    /// A delayed proposal cannot put a later writer's visit into waiting.
    pub async fn git_proposal_has_replacement_run(
        &mut self,
        proposal: &GitStageProposal,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs r LEFT JOIN leases l ON l.id=r.lease_id LEFT JOIN run_environment_reservations e ON e.run_id=r.id WHERE r.task_id=$1 AND r.id<>$2 AND (l.lease_state='active' OR (e.run_id IS NOT NULL AND e.released_at IS NULL)))")
            .bind(proposal.task_id.as_uuid()).bind(proposal.scope.run_id).fetch_one(&mut *self.transaction).await?)
    }
    /// Accepts only the exact inspected queue whose writer has stopped.
    pub async fn complete_git_proposal_queue(
        &mut self,
        proposal: &GitStageProposal,
    ) -> Result<bool, StorageError> {
        let changed=sqlx::query("UPDATE queue_entries q SET queue_state='completed',completed_at=clock_timestamp() FROM runs r JOIN leases l ON l.id=r.lease_id JOIN git_stage_proposals p ON p.run_id=r.id WHERE p.id=$1 AND p.proposal_state='inspecting' AND p.inspection_result IS NOT NULL AND r.observed_state='stopped' AND l.lease_state='released' AND q.id=r.queue_entry_id AND q.queue_state='leased' AND r.lease_fencing_token=$2 AND r.environment_epoch=$3")
            .bind(proposal.id).bind(u64_to_i64(proposal.scope.fencing_token,"fence")?)
            .bind(u64_to_i64(proposal.scope.environment_epoch,"epoch")?).execute(&mut *self.transaction).await?;
        Ok(changed.rows_affected() == 1)
    }
}
fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, StorageError> {
    serde_json::from_str(text).map_err(|source| StorageError::Snapshot {
        aggregate: "Git delivery",
        source,
    })
}
