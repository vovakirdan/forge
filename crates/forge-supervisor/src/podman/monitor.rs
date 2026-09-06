//! Persistent physical observation and stop control, independent of journal health.

use super::{PodmanBackend, valid_environment_id};
use crate::{RunControl, SupervisorError, registry::RunRegistry};
use forge_domain::runtime::SandboxRunSpec;
use forge_protocol::supervisor::v1::{ProvisionRun, RunEventKind, StopMode};
use serde_json::json;
use std::time::Duration;
use tokio::time::{Instant, sleep};

impl PodmanBackend {
    pub(super) async fn observe(
        &self,
        provision: &ProvisionRun,
        spec: &SandboxRunSpec,
        registry: &RunRegistry,
        control: &RunControl,
    ) -> Result<(), SupervisorError> {
        let mut stopping = None;
        let mut announced = false;
        let mut exited = false;
        let mut unknown = false;
        let mut last_identity = registry
            .journal
            .lock()
            .await
            .records()
            .into_iter()
            .find(|record| {
                crate::journal::scope_key(&record.provision) == crate::journal::scope_key(provision)
            })
            .map(|record| record.environment_id)
            .filter(|id| valid_environment_id(id));
        let mut wall_deadline = None;
        let mut heartbeat = Instant::now();
        loop {
            let inspection = match self.inspect(provision).await {
                Ok(Some(inspection)) => inspection,
                _ => {
                    if !unknown {
                        unknown = registry
                            .emit(
                                provision,
                                RunEventKind::EnvironmentLost,
                                json!({"reason_code":"podman_inspection_unknown"}),
                            )
                            .await
                            .is_ok();
                        let _ = registry.mark_unknown(provision).await;
                    }
                    // A previously verified immutable container ID cannot be
                    // recycled. Stop remains available while inspect is broken.
                    if let Some(id) = &last_identity {
                        self.apply_stop(
                            provision,
                            spec,
                            registry,
                            control,
                            id,
                            wall_deadline,
                            &mut stopping,
                        )
                        .await;
                    }
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            last_identity = Some(inspection.id.clone());
            if !inspection.state.running {
                if matches!(inspection.state.status.as_str(), "exited" | "stopped")
                    || (inspection.state.status == "created" && control.stopped())
                {
                    if !exited {
                        exited = registry.emit(provision, RunEventKind::Stopped,
                            json!({"reason_code":"container_exited", "exit_code":inspection.state.exit_code})).await.is_ok();
                    }
                    if exited && registry.finish(provision, true).await.is_ok() {
                        return Ok(());
                    }
                } else {
                    let _ = registry.mark_unknown(provision).await;
                }
                sleep(Duration::from_secs(1)).await;
                continue;
            }
            if !announced || unknown {
                let _ = registry
                    .attach_environment(provision, &inspection.id, true)
                    .await;
                announced = registry
                    .emit(
                        provision,
                        RunEventKind::Running,
                        json!({"executor":"rootless_podman"}),
                    )
                    .await
                    .is_ok();
                unknown = !announced;
            }
            if wall_deadline.is_none() {
                // An invalid start timestamp must fail closed, not disable the
                // monitor or grant an unlimited runtime.
                wall_deadline = Some(
                    wall_deadline_from_started_at(
                        &inspection.state.started_at,
                        spec.binding.limits.wall_seconds,
                    )
                    .unwrap_or_else(|_| Instant::now()),
                );
            }
            self.apply_stop(
                provision,
                spec,
                registry,
                control,
                &inspection.id,
                wall_deadline,
                &mut stopping,
            )
            .await;
            if heartbeat.elapsed() >= Duration::from_secs(30) {
                let _ = registry
                    .emit(
                        provision,
                        RunEventKind::Heartbeat,
                        json!({"environment_alive":true}),
                    )
                    .await;
                heartbeat = Instant::now();
            }
            tokio::select! { () = sleep(Duration::from_millis(250)) => {}, () = control.stop_notified(), if stopping.is_none() => {} }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one fenced environment stop checkpoint"
    )]
    async fn apply_stop(
        &self,
        provision: &ProvisionRun,
        spec: &SandboxRunSpec,
        registry: &RunRegistry,
        control: &RunControl,
        id: &str,
        wall_deadline: Option<Instant>,
        stopping: &mut Option<Instant>,
    ) {
        let elapsed_limit = wall_deadline.is_some_and(|deadline| deadline <= Instant::now());
        let requested = control.stop_request();
        if stopping.is_none() && (requested.is_some() || elapsed_limit) {
            let (mode, grace) = requested.unwrap_or((
                StopMode::Graceful,
                Duration::from_secs(u64::from(spec.binding.limits.stop_grace_seconds)),
            ));
            // Journal failure cannot prevent the physical kill switch. Keep
            // observing until Stopped can be persisted and replayed to Core.
            let _ = registry.emit(provision, RunEventKind::Stopping,
                json!({"reason_code":if elapsed_limit { "wall_limit" } else { "core_stop_requested" }})).await;
            let _ = self
                .signal(
                    id,
                    if mode == StopMode::Force {
                        "KILL"
                    } else {
                        "INT"
                    },
                )
                .await;
            *stopping = Some(Instant::now() + grace);
        } else if stopping.is_some_and(|deadline| deadline <= Instant::now())
            || requested.is_some_and(|(mode, _)| mode == StopMode::Force)
        {
            let _ = self.signal(id, "KILL").await;
        }
    }
}

fn wall_deadline_from_started_at(
    started_at: &str,
    wall_seconds: u32,
) -> Result<Instant, SupervisorError> {
    let started =
        time::OffsetDateTime::parse(started_at, &time::format_description::well_known::Rfc3339)
            .map_err(|_| SupervisorError::PodmanFailed)?;
    let remaining = time::Duration::seconds(i64::from(wall_seconds))
        - (time::OffsetDateTime::now_utc() - started);
    Ok(Instant::now() + Duration::try_from(remaining).unwrap_or_default())
}
