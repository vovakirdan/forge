use super::*;
impl CoreService {
    pub(super) async fn select_git_integration(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &mut Project,
        task: &Task,
    ) -> Result<Option<StoredIntegration>, CoreError> {
        let project_id = project.id();
        let task_id = task.id();
        if let Some(stored) = tx.integration_for_task(project_id, task_id).await? {
            return Ok(Some(stored));
        }
        if task.lifecycle() != LifecycleStatus::InProgress
            || !tx.integration_dispatch_open(project_id, task_id).await?
        {
            return Ok(None);
        }
        let version = pinned_pipeline_version(tx, project, task).await?;
        let Some(stage) = task.current_stage_id().and_then(|id| version.stage(id)) else {
            return Ok(None);
        };
        let Some(action @ SystemStageAction::GitIntegration { .. }) = stage.system_action() else {
            return Ok(None);
        };
        action.validate_version(stage, &version)?;
        action.validate_task(task)?;
        let TaskWorkSurface::Git(binding) = task.work_surface() else {
            return Ok(None);
        };
        let Some(candidate) = tx
            .latest_accepted_git_candidate(project_id, task_id)
            .await?
        else {
            self.integration_missing_candidate_wait(tx, project, task)
                .await?;
            return Ok(None);
        };
        let repository = tx
            .load_project_repository(binding.repository_id)
            .await?
            .filter(|repo| repo.project_id == project_id && binding.matches_repository(repo))
            .ok_or_else(|| invalid("Task repository is no longer registered"))?;
        repository.validate_snapshot()?;
        let writer = tx
            .accepted_candidate_writer(project_id, candidate.proposal_id)
            .await?
            .ok_or_else(|| invalid("accepted candidate has no writer provenance"))?;
        if candidate.surface_id != binding.surface_id || writer.task_id != task_id {
            return Err(invalid("candidate source mismatch"));
        }
        let op = GitIntegrationOperation {
            id: Uuid::now_v7(),
            fence: tx.next_integration_fence().await?,
            project_id,
            task_id,
            pipeline_version_id: version.id(),
            stage_id: stage.id().clone(),
            stage_visit: task
                .current_stage_visit()
                .ok_or_else(|| invalid("stage visit absent"))?
                .get(),
            task_revision: task.revision().get(),
            candidate_proposal_id: candidate.proposal_id,
            candidate: candidate.candidate,
            binding: binding.clone(),
            writer: writer.scope,
            created_at: crate::canonical_clock::project_mutation_time(project),
        };
        let stored = StoredIntegration {
            operation: op,
            intent: None,
            state: IntegrationState::Preparing,
            core_instance: self.instance_id,
            expected_task_revision: task.revision().get(),
            command: None,
            result: None,
            wait_condition_id: None,
            resolution: None,
        };
        tx.insert_git_integration(&stored).await?;
        Ok(Some(stored))
    }
}
