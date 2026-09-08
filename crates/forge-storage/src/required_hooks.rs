//! Explicit repository acceptance is bound to the latest checked candidate.
use crate::{StorageError, StorageTransaction};
use forge_domain::{
    HookExecutionResult, HookVerdict, PipelineVersion, Task, git_delivery::TaskGitCandidate,
    git_integration::SystemStageAction,
};
use sqlx::Row;
use uuid::Uuid;

impl StorageTransaction<'_> {
    /// Caller retains the Project gate. A pending writer proposal can be checked
    /// before its acceptance, but an old accepted candidate cannot stand in for it.
    pub async fn required_hooks_satisfied(
        &mut self,
        task: &Task,
        version: &PipelineVersion,
        candidate_override: Option<Uuid>,
    ) -> Result<bool, StorageError> {
        if version.project_id() != task.project_id()
            || version.id() != task.pipeline().pipeline_version_id()
        {
            return Ok(false);
        }
        let mut required = Vec::new();
        for stage in version.stages() {
            let Some(
                action @ SystemStageAction::ProjectHook {
                    hook_version_id, ..
                },
            ) = stage.system_action()
            else {
                continue;
            };
            let Some(hook) = self
                .load_project_hook_version(task.project_id(), *hook_version_id)
                .await?
            else {
                return Ok(false);
            };
            if action.validate_hook_version(stage, version, &hook).is_err() {
                return Ok(false);
            }
            if hook.required && hook.applies_to(task.kind()) {
                required.push((stage.id(), hook.id));
            }
        }
        // Projects without explicit applicable required hooks need no candidate,
        // repository, runner, inferred test framework, or synthetic success.
        if required.is_empty() {
            return Ok(true);
        }
        let candidate = if let Some(id) = candidate_override {
            let snapshot:Option<String>=sqlx::query_scalar("SELECT c.canonical_snapshot::text FROM task_git_candidates c JOIN git_stage_proposals p ON p.id=c.proposal_id AND p.project_id=c.project_id AND p.task_id=c.task_id WHERE c.proposal_id=$1 AND c.project_id=$2 AND c.task_id=$3 AND p.canonical_snapshot->>'pipeline_version_id'=$4 AND p.proposal_state IN ('accepted','inspecting')")
                .bind(id).bind(task.project_id().as_uuid()).bind(task.id().as_uuid()).bind(version.id().to_string()).fetch_optional(&mut *self.transaction).await?;
            snapshot
                .map(|value| {
                    serde_json::from_str::<TaskGitCandidate>(&value).map_err(|source| {
                        StorageError::Snapshot {
                            aggregate: "hook.candidate",
                            source,
                        }
                    })
                })
                .transpose()?
        } else {
            self.latest_accepted_git_candidate(task.project_id(), task.id())
                .await?
        };
        let Some(candidate) = candidate else {
            return Ok(false);
        };
        if candidate.validate().is_err()
            || candidate.project_id != task.project_id()
            || candidate.task_id != task.id()
            || candidate_override.is_some_and(|id| candidate.proposal_id != id)
        {
            return Ok(false);
        }
        for (stage, hook) in required {
            let row=sqlx::query("SELECT id,state,result FROM hook_invocations WHERE project_id=$1 AND task_id=$2 AND pipeline_version_id=$3 AND stage_id=$4 AND hook_version_id=$5 AND candidate_proposal_id=$6 ORDER BY created_at DESC,id DESC LIMIT 1")
                .bind(task.project_id().as_uuid()).bind(task.id().as_uuid()).bind(version.id().as_uuid()).bind(stage.as_str()).bind(hook).bind(candidate.proposal_id).fetch_optional(&mut *self.transaction).await?;
            let Some(row) = row else { return Ok(false) };
            if row.try_get::<String, _>("state")? != "completed" {
                return Ok(false);
            }
            let Some(value) = row.try_get::<Option<serde_json::Value>, _>("result")? else {
                return Ok(false);
            };
            let Ok(result) = serde_json::from_value::<HookExecutionResult>(value) else {
                return Ok(false);
            };
            if result.validate().is_err()
                || result.invocation_id != row.try_get::<Uuid, _>("id")?
                || result.verdict != HookVerdict::Passed
                || result.candidate_proposal_id != candidate.proposal_id
                || result.candidate != candidate.candidate
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
