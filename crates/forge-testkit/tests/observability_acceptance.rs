//! Operational diagnostics never become authority or contain request payloads.

use anyhow::Result;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_core::{CoreActors, CoreService, SupervisorHub, observability::CheckState};
use forge_domain::ActorId;
use forge_protocol::trace_context::TraceContext;
use forge_storage::PostgresStore;
use forge_testkit::m0::M0Harness;
use sqlx::postgres::PgPoolOptions;
use std::{
    io::Write,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

fn unavailable_core() -> CoreService {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(50))
        .connect_lazy("postgres://synthetic:synthetic@127.0.0.1:1/unavailable")
        .unwrap();
    CoreService::new(
        PostgresStore::from_pool(pool),
        CoreActors::new(ActorId::new(), ActorId::new()),
        Arc::new(SupervisorHub::default()),
    )
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);
impl Write for CapturedLogs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn health_is_live_without_dependencies_and_readiness_is_not() -> Result<()> {
    let core = unavailable_core();
    let router = forge_core::router(core.clone());
    let response = router
        .clone()
        .oneshot(Request::builder().uri("/healthz").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let response = router
        .oneshot(Request::builder().uri("/readyz").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
    assert_eq!(body["postgres"], "down");
    assert_eq!(body["supervisor"], "down");
    assert_eq!(body["nats"], "disabled");
    assert_eq!(body["object_store"], "disabled");
    let metrics = core.metrics().await?;
    assert!(metrics.contains("forge_core_postgres_up 0"));
    assert!(metrics.contains("forge_core_process_up 1"));
    Ok(())
}

#[tokio::test]
async fn trace_correlation_is_propagated_without_logging_headers_paths_or_bodies() -> Result<()> {
    let core = unavailable_core();
    let logs = CapturedLogs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    let parent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let secret = "synthetic-request-secret-never-a-label";
    let response = forge_core::router(core.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/commands/not_a_command?api_key={secret}"))
                .header("traceparent", parent)
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(format!("{{\"private_prompt\":\"{secret}\"}}")))?,
        )
        .await?;
    assert!(response.status().is_client_error());
    let propagated = TraceContext::parse(response.headers()["traceparent"].to_str()?).unwrap();
    assert_eq!(
        propagated.trace_id(),
        TraceContext::parse(parent).unwrap().trace_id()
    );
    assert_ne!(
        propagated.span_id(),
        TraceContext::parse(parent).unwrap().span_id()
    );
    let captured = String::from_utf8(logs.0.lock().unwrap().clone())?;
    assert!(!captured.is_empty());
    for line in captured.lines() {
        let _: serde_json::Value = serde_json::from_str(line)?;
    }
    assert!(captured.contains(&propagated.trace_id()));
    assert!(!captured.contains(secret));
    assert!(!captured.contains("not_a_command"));
    let response = forge_core::router(core.clone())
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header("traceparent", secret)
                .body(Body::empty())?,
        )
        .await?;
    assert!(TraceContext::parse(response.headers()["traceparent"].to_str()?).is_some());
    let metrics = core.metrics().await?;
    assert!(metrics.contains("operation=\"HttpCommand\""));
    for forbidden in [
        secret,
        "not_a_command",
        "task_id=",
        "run_id=",
        "trace_id=",
        "model=",
    ] {
        assert!(
            !metrics.contains(forbidden),
            "metrics contained forbidden dimension {forbidden}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn readiness_probes_real_local_dependencies_without_starting_an_employee() -> Result<()> {
    let harness = M0Harness::start().await?;
    let first = harness.core.readiness().await;
    assert_eq!(first.postgres, CheckState::Up);
    assert_eq!(first.supervisor, CheckState::Down);
    assert!(!first.ready);
    let supervisor = harness.attach_manual_supervisor().await?;
    assert!(harness.core.readiness().await.ready);
    let core = harness.core.clone().with_nats_health_required(true);
    assert_eq!(core.readiness().await.nats, CheckState::Down);
    let config =
        forge_core::OutboxPublisherConfig::local_default("observability-acceptance".into())?;
    let _publisher = core.outbox_publisher(harness.jetstream_context().await?, config);
    let ready = core.readiness().await;
    assert!(ready.ready);
    assert_eq!(ready.nats, CheckState::Up);
    let metrics = core.metrics().await?;
    assert!(metrics.contains("forge_core_postgres_up 1"));
    assert!(metrics.contains("forge_core_supervisor_up 1"));
    drop(supervisor);
    harness.shutdown().await;
    Ok(())
}
