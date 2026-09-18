//! Coherent, bounded read projection of the canonical dependency relation.

use forge_domain::{DependencyCondition, LifecycleStatus, ProjectId, Task, TaskDependency, TaskId};
use sqlx::Row;
use uuid::Uuid;

use crate::{PostgresStore, StorageError, decode_snapshot};

#[derive(Clone, Copy, Debug)]
/// Which side of a dependency is anchored to the requested Task.
pub enum DependencyDirection {
    /// Prerequisites of the requested Task.
    BlockedBy,
    /// Tasks depending on the requested Task.
    Blocks,
}

/// One canonical edge with the related Task and blocker state from the same snapshot.
pub struct DependencyReadRow {
    /// Canonical edge and completion condition.
    pub dependency: TaskDependency,
    /// Task on the opposite side of the requested direction.
    pub related_task: Task,
    /// Blocker's canonical lifecycle, including for the outbound direction.
    pub blocker_lifecycle: LifecycleStatus,
}

/// A bounded page plus cursor-membership evidence.
pub struct DependencyReadPage {
    /// Whether the requested cursor still belongs to this relation.
    pub cursor_exists: bool,
    /// At most the requested number of rows, ordered by related Task ID.
    pub rows: Vec<DependencyReadRow>,
}

impl PostgresStore {
    /// Reads existence, cursor membership and both Task snapshots at one read-only snapshot.
    pub async fn read_task_dependencies(
        &self,
        project: ProjectId,
        task: TaskId,
        direction: DependencyDirection,
        after: Option<TaskId>,
        limit: u32,
    ) -> Result<DependencyReadPage, StorageError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tasks WHERE project_id=$1 AND id=$2)")
                .bind(project.as_uuid())
                .bind(task.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
        if !exists {
            return Err(StorageError::NotFound { aggregate: "task" });
        }
        let (anchor, related) = match direction {
            DependencyDirection::BlockedBy => ("blocked_task_id", "blocker_task_id"),
            DependencyDirection::Blocks => ("blocker_task_id", "blocked_task_id"),
        };
        let cursor_exists = if let Some(after) = after {
            sqlx::query_scalar::<_, bool>(&format!("SELECT EXISTS(SELECT 1 FROM task_dependencies WHERE project_id=$1 AND {anchor}=$2 AND {related}=$3)"))
                .bind(project.as_uuid()).bind(task.as_uuid()).bind(after.as_uuid()).fetch_one(&mut *tx).await?
        } else {
            true
        };
        let query = format!(
            "SELECT d.blocker_task_id, d.blocked_task_id, d.required_blocker_lifecycle, related.canonical_snapshot::text AS related_snapshot, blocker.canonical_snapshot::text AS blocker_snapshot FROM task_dependencies d JOIN tasks related ON related.id=d.{related} AND related.project_id=d.project_id JOIN tasks blocker ON blocker.id=d.blocker_task_id AND blocker.project_id=d.project_id WHERE d.project_id=$1 AND d.{anchor}=$2 AND ($3::uuid IS NULL OR d.{related}>$3) ORDER BY d.{related} LIMIT $4"
        );
        let records = sqlx::query(&query)
            .bind(project.as_uuid())
            .bind(task.as_uuid())
            .bind(after.map(TaskId::as_uuid))
            .bind(i64::from(limit))
            .fetch_all(&mut *tx)
            .await?;
        let mut rows = Vec::with_capacity(records.len());
        for row in records {
            let condition: String = row.try_get("required_blocker_lifecycle")?;
            if condition != "done" {
                return Err(StorageError::InvalidInput {
                    reason: "unsupported stored dependency condition".into(),
                });
            }
            let dependency = TaskDependency::new(
                row.try_get::<Uuid, _>("blocker_task_id")?.into(),
                row.try_get::<Uuid, _>("blocked_task_id")?.into(),
                DependencyCondition::Done,
            )
            .map_err(|source| StorageError::SnapshotInvariant {
                aggregate: "dependency",
                source,
            })?;
            let related_task: Task =
                decode_snapshot(&row.try_get::<String, _>("related_snapshot")?, "task")?;
            let blocker: Task =
                decode_snapshot(&row.try_get::<String, _>("blocker_snapshot")?, "task")?;
            let expected_related = match direction {
                DependencyDirection::BlockedBy => dependency.blocker_task_id(),
                DependencyDirection::Blocks => dependency.blocked_task_id(),
            };
            if related_task.project_id() != project
                || related_task.id() != expected_related
                || blocker.project_id() != project
                || blocker.id() != dependency.blocker_task_id()
            {
                return Err(StorageError::InvalidInput {
                    reason: "dependency Task snapshot scope mismatch".into(),
                });
            }
            rows.push(DependencyReadRow {
                dependency,
                related_task,
                blocker_lifecycle: blocker.lifecycle(),
            });
        }
        tx.commit().await?;
        Ok(DependencyReadPage {
            cursor_exists,
            rows,
        })
    }
}
