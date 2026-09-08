//! Accepted Task revisions are distinct from observations awaiting Manager action.
use forge_domain::{ProjectId, TaskId, git_delivery::TaskGitCandidate};
use sqlx::Row;

use crate::{StorageError, StorageTransaction};

impl StorageTransaction<'_> {
    /// Caller holds the Project gate while freezing the next Run's source.
    /// A verified but unaccepted proposal never grants a review workspace.
    pub async fn latest_accepted_git_candidate(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
    ) -> Result<Option<TaskGitCandidate>, StorageError> {
        let row = sqlx::query(
            "SELECT c.proposal_id,c.canonical_snapshot::text FROM task_git_candidates c JOIN git_stage_proposals p ON p.id=c.proposal_id AND p.project_id=c.project_id AND p.task_id=c.task_id WHERE c.project_id=$1 AND c.task_id=$2 AND p.proposal_state='accepted' ORDER BY c.verified_at DESC,c.proposal_id DESC LIMIT 1",
        )
        .bind(project_id.as_uuid())
        .bind(task_id.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        row.map(|row| {
            let candidate: TaskGitCandidate = serde_json::from_str(
                &row.try_get::<String, _>("canonical_snapshot")?,
            )
            .map_err(|source| StorageError::Snapshot {
                aggregate: "Task Git candidate",
                source,
            })?;
            candidate
                .validate()
                .map_err(|source| StorageError::SnapshotInvariant {
                    aggregate: "Task Git candidate",
                    source,
                })?;
            if candidate.project_id != project_id
                || candidate.task_id != task_id
                || candidate.proposal_id != row.try_get::<uuid::Uuid, _>("proposal_id")?
            {
                return Err(StorageError::InvalidInput {
                    reason: "accepted Git candidate does not match its canonical Task scope".into(),
                });
            }
            Ok(candidate)
        })
        .transpose()
    }
}
