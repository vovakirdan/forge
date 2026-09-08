//! Immutable revision-bound verdicts; no unverified prose becomes acceptance.
use crate::{PostgresStore, StorageError, StorageTransaction, database_timestamp, encode_snapshot};
use forge_domain::{ProjectId, TaskId, candidate_review::CandidateReviewRecord};

impl StorageTransaction<'_> {
    pub async fn insert_candidate_review(
        &mut self,
        review: &CandidateReviewRecord,
    ) -> Result<(), StorageError> {
        review
            .validate()
            .map_err(|source| StorageError::SnapshotInvariant {
                aggregate: "candidate review",
                source,
            })?;
        sqlx::query("INSERT INTO candidate_reviews(id,project_id,task_id,run_id,candidate_proposal_id,canonical_snapshot,recorded_at) VALUES($1,$2,$3,$4,$5,$6::jsonb,$7)")
            .bind(review.id).bind(review.project_id.as_uuid()).bind(review.task_id.as_uuid()).bind(review.scope.run_id)
            .bind(review.candidate_proposal_id).bind(encode_snapshot(review,"candidate review")?).bind(database_timestamp(review.recorded_at))
            .execute(&mut *self.transaction).await?;
        Ok(())
    }
}
impl PostgresStore {
    /// Ordered bounded operator evidence, including rejected and stale revisions.
    pub async fn list_candidate_reviews(
        &self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Vec<CandidateReviewRecord>, StorageError> {
        self.candidate_review_page(project, task, None, 100).await
    }
    pub async fn candidate_review_page(
        &self,
        project: ProjectId,
        task: TaskId,
        after: Option<uuid::Uuid>,
        limit: u32,
    ) -> Result<Vec<CandidateReviewRecord>, StorageError> {
        if !(1..=101).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "review page limit must be 1..101".into(),
            });
        }
        let values:Vec<String>=sqlx::query_scalar("SELECT canonical_snapshot::text FROM candidate_reviews WHERE project_id=$1 AND task_id=$2 AND ($3::uuid IS NULL OR id<$3) ORDER BY id DESC LIMIT $4")
            .bind(project.as_uuid()).bind(task.as_uuid()).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        values
            .into_iter()
            .map(|text| {
                let review: CandidateReviewRecord =
                    serde_json::from_str(&text).map_err(|source| StorageError::Snapshot {
                        aggregate: "candidate review",
                        source,
                    })?;
                review
                    .validate()
                    .map_err(|source| StorageError::SnapshotInvariant {
                        aggregate: "candidate review",
                        source,
                    })?;
                Ok(review)
            })
            .collect()
    }
}
