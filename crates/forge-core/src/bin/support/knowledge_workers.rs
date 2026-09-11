use std::time::Duration;

use forge_core::CoreService;
use tokio::{sync::watch, task::JoinHandle, time::MissedTickBehavior};

// Separate workers keep external index latency out of semantic admission and
// physical reconciliation. Both rely on durable work, so missed ticks can skip.
pub(super) fn spawn_projection(
    core: CoreService,
    mut shutdown: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if core.drain_memory_projections().await.is_err() {
                        tracing::warn!("memory projection delivery failed; canonical state remains available");
                    }
                },
                _ = shutdown.changed() => break,
            }
        }
    })
}

pub(super) fn spawn_semantic(
    core: CoreService,
    mut shutdown: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if let Err(error) = core.system_job_tick().await {
                        tracing::warn!(error = %error,
                            "SystemJob tick failed; durable inputs and attempts remain retained");
                    }
                },
                _ = shutdown.changed() => break,
            }
        }
    })
}
