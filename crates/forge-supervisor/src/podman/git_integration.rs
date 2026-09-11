//! Deterministic Git effects never run in the Supervisor stop-control loop.
use super::PodmanBackend;
use crate::{
    SupervisorError,
    git::{GitBackend, GitBackendError, GitIntegrationRequest},
    registry::RunRegistry,
    surface,
};
use forge_domain::{
    git::GitIntegrationState,
    git_integration::{IntegrationCode, IntegrationPhase, IntegrationRequest, IntegrationResult},
};
use forge_protocol::supervisor::v1::GitIntegrationCommand;
use std::time::Duration;

impl PodmanBackend {
    pub(crate) async fn integrate_git(
        &self,
        wire: GitIntegrationCommand,
        registry: RunRegistry,
    ) -> Result<(), SupervisorError> {
        if wire.request_json.len() > 32768 {
            return Err(SupervisorError::InvalidRunSpec);
        }
        let request: IntegrationRequest = serde_json::from_str(&wire.request_json)?;
        request
            .validate()
            .map_err(|_| SupervisorError::InvalidRunSpec)?;
        if wire.command_id != request.command_id.to_string() {
            return Err(SupervisorError::InvalidRunSpec);
        }
        // A duplicate already completed request only replays its receipt. A live
        // concurrent operation never gets replaced by a competing Busy receipt.
        if registry.journal.lock().await.integration_replay(&request)? == Some(true) {
            registry.changed.notify_one();
            return Ok(());
        }
        let Ok(_exclusive) = self.provisioning.try_lock() else {
            return Ok(());
        };
        let replay = registry.journal.lock().await.integration_replay(&request)?;
        if replay == Some(true) {
            registry.changed.notify_one();
            return Ok(());
        }
        if request.host_id != self.config.host_id || request.boot_id != self.config.boot_id {
            return Err(SupervisorError::InvalidRunSpec);
        }
        if replay.is_none() {
            registry.journal.lock().await.begin_integration(&request)?;
        }
        let mut result = IntegrationResult {
            message_id: uuid::Uuid::now_v7(),
            command_id: request.command_id,
            operation_id: request.operation.id,
            fence: request.operation.fence,
            host_id: self.config.host_id.clone(),
            boot_id: self.config.boot_id.clone(),
            code: IntegrationCode::Unknown,
            intent: None,
        };
        let action = tokio::time::timeout(
            Duration::from_secs(60),
            self.integration_effect(&request, &registry, replay == Some(false)),
        )
        .await;
        match action {
            Ok(Ok((code, intent))) => {
                result.code = code;
                result.intent = intent;
            }
            Ok(Err(GitBackendError::NoChanges)) => result.code = IntegrationCode::NoChanges,
            Ok(Err(GitBackendError::StaleBase)) => result.code = IntegrationCode::StaleBase,
            Ok(Err(_)) => result.code = IntegrationCode::Refused,
            Err(_) => result.code = IntegrationCode::Unavailable,
        }
        // Local intent is durable before a Prepared reply can authorize Core apply.
        if result.code == IntegrationCode::Prepared
            && let Some(intent) = &result.intent
        {
            registry
                .journal
                .lock()
                .await
                .prepared_integration(&request.operation, intent)?;
        }
        registry.journal.lock().await.finish_integration(&result)?;
        registry.changed.notify_one();
        Ok(())
    }

    async fn integration_effect(
        &self,
        request: &IntegrationRequest,
        registry: &RunRegistry,
        incomplete_replay: bool,
    ) -> Result<
        (
            IntegrationCode,
            Option<forge_domain::git::GitIntegrationIntent>,
        ),
        GitBackendError,
    > {
        let op = &request.operation;
        let retained = registry
            .journal
            .lock()
            .await
            .retained_integration_intent(op.id);
        if request.phase == IntegrationPhase::Reconcile || incomplete_replay {
            let Some(intent) = retained else {
                let absent = request.phase == IntegrationPhase::Reconcile
                    && request.intent.is_none()
                    && registry
                        .journal
                        .lock()
                        .await
                        .integration_has_no_preparation(op);
                return Ok((
                    if absent {
                        IntegrationCode::NoPreparation
                    } else {
                        IntegrationCode::Unknown
                    },
                    None,
                ));
            };
            op.validate_intent(&intent)
                .map_err(|_| GitBackendError::InvalidIntent)?;
            if request
                .intent
                .as_ref()
                .is_some_and(|requested| requested != &intent)
            {
                return Err(GitBackendError::InvalidIntent);
            }
            if request.intent.is_none() {
                // Core has no apply permit yet. Recover the existing preparation;
                // never regenerate its timestamp, parent order or merge identity.
                return Ok((IntegrationCode::Prepared, Some(intent)));
            }
            let state = GitBackend::default()
                .reconcile_integration(op.binding.source.as_path(), &intent)
                .await?;
            return Ok((integration_code(state), Some(intent)));
        }
        let source = registry
            .journal
            .lock()
            .await
            .integration_source(op)
            .map_err(|_| GitBackendError::WriterActive)?;
        let records = registry.journal.lock().await.records();
        // Every retained Task environment must have positive physical exit proof,
        // including the read-only reviewer whose logical outcome already finished.
        for record in records
            .iter()
            .filter(|record| record.provision.task_id == op.task_id.to_string())
        {
            match self
                .inspect(&record.provision)
                .await
                .map_err(|_| GitBackendError::WriterActive)?
            {
                Some(environment)
                    if environment.id == record.environment_id
                        && !environment.state.running
                        && matches!(environment.state.status.as_str(), "exited" | "stopped") => {}
                None if record.quiescence_confirmed => {}
                _ => return Err(GitBackendError::WriterActive),
            }
        }
        let worktree = surface::inspection_worktree(
            &self.config.state_directory,
            &op.task_id.to_string(),
            &source,
        )
        .map_err(|_| GitBackendError::UnsafePath)?;
        let root = self.config.state_directory.join("integrations");
        surface::private_directory(&root).map_err(|_| GitBackendError::UnsafePath)?;
        let staging = root.join(op.id.to_string());
        let backend = GitBackend::default();
        let target_key = backend
            .target_key(op.binding.source.as_path(), &op.binding.target_ref)
            .await?;
        if backend
            .target_exists(op.binding.source.as_path(), &op.binding.target_ref)
            .await?
        {
            registry
                .journal
                .lock()
                .await
                .mark_target_established(&target_key)
                .map_err(|_| GitBackendError::InvalidIntent)?;
        }
        if request.phase == IntegrationPhase::Prepare {
            if retained.is_some() {
                return Err(GitBackendError::InvalidIntent);
            }
            let allow_initial_publication = matches!(
                op.binding.initial_base,
                forge_domain::git::GitInitialRevision::Unborn { .. }
            ) && !registry
                .journal
                .lock()
                .await
                .target_established(&target_key);
            let intent = backend
                .prepare_integration(GitIntegrationRequest {
                    target: op.binding.source.as_path(),
                    branch: &op.binding.target_ref,
                    candidate_source: &worktree,
                    candidate: &op.candidate,
                    staging_directory: &staging,
                    operation_id: op.id,
                    committed_at: op.created_at,
                    allow_initial_publication,
                })
                .await?;
            if intent.expected_target.is_some() {
                registry
                    .journal
                    .lock()
                    .await
                    .mark_target_established(&target_key)
                    .map_err(|_| GitBackendError::InvalidIntent)?;
            }
            return Ok((IntegrationCode::Prepared, Some(intent)));
        }
        let intent = request
            .intent
            .as_ref()
            .ok_or(GitBackendError::InvalidIntent)?;
        if retained.as_ref() != Some(intent) {
            return Err(GitBackendError::InvalidIntent);
        }
        // A delivered create may survive a crash before its reply. Never reclassify
        // this branch as unborn simply because an operator later removed its ref.
        registry
            .journal
            .lock()
            .await
            .mark_target_established(&target_key)
            .map_err(|_| GitBackendError::InvalidIntent)?;
        let state = GitBackend::default()
            .apply_integration(op.binding.source.as_path(), &staging, intent)
            .await?;
        Ok((integration_code(state), Some(intent.clone())))
    }
}
fn integration_code(state: GitIntegrationState) -> IntegrationCode {
    match state {
        GitIntegrationState::Applied => IntegrationCode::Applied,
        GitIntegrationState::Retryable => IntegrationCode::Retryable,
        GitIntegrationState::Unknown => IntegrationCode::Unknown,
    }
}
#[cfg(test)]
mod tests;
