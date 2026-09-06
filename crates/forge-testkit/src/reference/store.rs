//! Copy-on-write transaction ownership for the reference adapter.

use std::sync::Arc;

use forge_application::RepositoryError;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::MemorySnapshot;

/// Independent in-memory database; clones share exactly one committed state.
#[derive(Clone, Debug, Default)]
pub struct MemoryStore(Arc<Mutex<MemorySnapshot>>);

impl MemoryStore {
    /// Serializes writers, including creation of previously absent projects.
    pub async fn begin(&self) -> MemoryTransaction {
        let committed = self.0.clone().lock_owned().await;
        let staged = committed.clone();
        MemoryTransaction { committed, staged }
    }

    /// Copies only committed state; uncommitted writes are never visible.
    pub async fn snapshot(&self) -> MemorySnapshot {
        self.0.lock().await.clone()
    }
}

/// An uncommitted snapshot with an exclusive database guard.
///
/// Dropping it, including during unwinding or cancellation, releases the guard
/// without modifying committed state. Global serialization is intentional for
/// this small reference backend; it does not claim PostgreSQL performance.
pub struct MemoryTransaction {
    committed: OwnedMutexGuard<MemorySnapshot>,
    pub(super) staged: MemorySnapshot,
}

impl MemoryTransaction {
    /// Exposes staged state for explicit test-only setup and assertions.
    /// Production commands must use the application engine instead.
    pub fn snapshot_mut(&mut self) -> &mut MemorySnapshot {
        &mut self.staged
    }

    /// Reads the current transaction snapshot, including uncommitted writes.
    #[must_use]
    pub fn snapshot(&self) -> &MemorySnapshot {
        &self.staged
    }

    /// Publishes once, after validating deferred catalog references.
    pub async fn commit(mut self) -> Result<(), RepositoryError> {
        for pipeline in self.staged.pipelines.values() {
            let version = self
                .staged
                .pipeline_versions
                .get(&pipeline.default_version_id());
            if version.is_none_or(|version| version.pipeline_id() != pipeline.id()) {
                return Err(RepositoryError::InvalidInput {
                    reason: "pipeline default version must belong to its catalog".to_owned(),
                });
            }
        }
        *self.committed = self.staged;
        Ok(())
    }

    /// Discards staged writes; identical to dropping without commit.
    pub async fn rollback(self) {}
}
