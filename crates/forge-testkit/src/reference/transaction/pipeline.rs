//! Catalog persistence mirrors the PostgreSQL exact-revision and version-scope guards.
use super::{MemoryTransaction, invalid};
use forge_application::RepositoryError;
use forge_domain::Pipeline;

impl MemoryTransaction {
    pub(super) fn persist_pipeline_revision(
        &mut self,
        pipeline: &Pipeline,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        pipeline
            .validate_snapshot()
            .map_err(|_| invalid("invalid pipeline snapshot"))?;
        if self
            .staged
            .pipelines
            .get(&pipeline.id())
            .is_none_or(|stored| {
                stored.project_id() != pipeline.project_id()
                    || stored.revision() != expected_revision
            })
        {
            return Err(RepositoryError::StaleRevision {
                aggregate: "pipeline",
            });
        }
        if expected_revision.checked_add(1) != Some(pipeline.revision()) {
            return Err(invalid("pipeline revision must advance exactly once"));
        }
        if self
            .staged
            .pipeline_versions
            .get(&pipeline.default_version_id())
            .is_none_or(|version| {
                version.pipeline_id() != pipeline.id()
                    || version.project_id() != pipeline.project_id()
            })
        {
            return Err(invalid("default version does not belong to pipeline"));
        }
        self.staged
            .pipelines
            .insert(pipeline.id(), pipeline.clone());
        Ok(())
    }
}
