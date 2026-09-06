//! Metrics-only loopback exporter. It cannot control Runs or expose their payloads.

use crate::{SupervisorConfig, registry::RunRegistry};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use forge_protocol::supervisor::v1::EnvironmentPresence;
use prometheus_client::{encoding::text::encode, metrics::gauge::Gauge, registry::Registry};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
use tokio::{net::TcpListener, sync::watch, task::JoinHandle};

struct Metrics {
    registry: Registry,
    runs: RunRegistry,
    started: Instant,
    uptime: Gauge,
    connected: Gauge,
    journal_healthy: Gauge,
    active: Gauge,
    unknown: Gauge,
    quiescent: Gauge,
    pending: Gauge,
}

impl Metrics {
    fn new(runs: RunRegistry) -> Self {
        let mut registry = Registry::default();
        let gauges: Vec<Gauge> = (0..7).map(|_| Gauge::default()).collect();
        for ((name, help), gauge) in [
            ("uptime_seconds", "Monotonic Supervisor uptime"),
            ("core_connected", "Authenticated Core channel connected"),
            ("journal_healthy", "Last journal write succeeded"),
            ("environments_active", "Active physical environments"),
            ("environments_unknown", "Uncertain physical environments"),
            ("environments_quiescent", "Retained quiescent identities"),
            (
                "pending_observations",
                "Unacknowledged durable observations",
            ),
        ]
        .into_iter()
        .zip(&gauges)
        {
            registry.register(format!("forge_supervisor_{name}"), help, gauge.clone());
        }
        Self {
            registry,
            runs,
            started: Instant::now(),
            uptime: gauges[0].clone(),
            connected: gauges[1].clone(),
            journal_healthy: gauges[2].clone(),
            active: gauges[3].clone(),
            unknown: gauges[4].clone(),
            quiescent: gauges[5].clone(),
            pending: gauges[6].clone(),
        }
    }
}

pub(super) async fn serve(
    config: &SupervisorConfig,
    runs: RunRegistry,
    mut shutdown: watch::Receiver<bool>,
) -> Option<JoinHandle<()>> {
    let address = config.observability_address?;
    if !address.ip().is_loopback() {
        tracing::warn!(
            component = "observability",
            "refused non-loopback metrics listener"
        );
        return None;
    }
    let listener = match TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(_) => {
            tracing::warn!(
                component = "observability",
                "metrics listener unavailable; supervision continues"
            );
            return None;
        }
    };
    let router = Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .route("/metrics", get(metrics))
        .with_state(Arc::new(Metrics::new(runs)));
    Some(tokio::spawn(async move {
        tokio::select! { _ = axum::serve(listener,router) => {}, _ = shutdown.changed() => {} }
    }))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok"}))
}
async fn ready(State(metrics): State<Arc<Metrics>>) -> Response {
    let connected = metrics.runs.connected.load(Ordering::Acquire);
    let journal_healthy = metrics.runs.journal.lock().await.is_healthy();
    (if connected && journal_healthy {StatusCode::OK} else {StatusCode::SERVICE_UNAVAILABLE},
        Json(serde_json::json!({"ready":connected && journal_healthy,"core_connected":connected,"journal_healthy":journal_healthy}))).into_response()
}
async fn metrics(State(metrics): State<Arc<Metrics>>) -> Response {
    metrics
        .uptime
        .set(i64::try_from(metrics.started.elapsed().as_secs()).unwrap_or(i64::MAX));
    metrics
        .connected
        .set(i64::from(metrics.runs.connected.load(Ordering::Acquire)));
    let journal = metrics.runs.journal.lock().await;
    metrics.journal_healthy.set(i64::from(journal.is_healthy()));
    let inventory = journal.inventory();
    for (presence, gauge) in [
        (EnvironmentPresence::Active, &metrics.active),
        (EnvironmentPresence::Unknown, &metrics.unknown),
        (EnvironmentPresence::Quiescent, &metrics.quiescent),
    ] {
        gauge.set(
            i64::try_from(
                inventory
                    .iter()
                    .filter(|entry| entry.presence == presence as i32)
                    .count(),
            )
            .unwrap_or(i64::MAX),
        );
    }
    metrics
        .pending
        .set(i64::try_from(journal.pending().count()).unwrap_or(i64::MAX));
    drop(journal);
    let mut text = String::new();
    if encode(&mut text, &metrics.registry).is_err() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    (
        [(
            "content-type",
            "application/openmetrics-text; version=1.0.0; charset=utf-8",
        )],
        text,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{journal::Journal, new_id, tests::provision};

    #[tokio::test]
    async fn readiness_tracks_connection_and_durable_journal_failures() {
        let root = std::env::temp_dir().join(format!("forge-observability-{}", new_id()));
        let runs = RunRegistry::new(Journal::open(&root, "metrics-test", 1024 * 1024).unwrap());
        let metrics_state = Arc::new(Metrics::new(runs.clone()));
        assert_eq!(
            ready(State(metrics_state.clone())).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        runs.connected.store(true, Ordering::Release);
        assert_eq!(
            ready(State(metrics_state.clone())).await.status(),
            StatusCode::OK
        );
        let request = provision();
        runs.journal
            .lock()
            .await
            .register(&request, "boot")
            .unwrap();
        let response = metrics(State(metrics_state.clone())).await;
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("forge_supervisor_environments_active 1"));
        assert!(!text.contains(&request.run_id));
        {
            let mut journal = runs.journal.lock().await;
            journal.set_test_capacity(1);
            assert!(journal.finish(&request, true).is_err());
        }
        assert_eq!(
            ready(State(metrics_state.clone())).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        {
            let mut journal = runs.journal.lock().await;
            journal.set_test_capacity(1024 * 1024);
            journal.finish(&request, true).unwrap();
        }
        assert_eq!(
            ready(State(metrics_state.clone())).await.status(),
            StatusCode::OK
        );
        assert_eq!(health().await.0["status"], "ok");
        drop(metrics_state);
        drop(runs);
        std::fs::remove_dir_all(root).unwrap();
    }
}
