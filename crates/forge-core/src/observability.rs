//! Bounded operational metrics and dependency probes, outside canonical writes.

pub(crate) mod http;
pub(crate) use http::trace_operation;
pub use http::{current_traceparent, middleware, router};

use crate::CoreService;
use async_nats::jetstream::Context;
use forge_storage::OperationalSnapshot;
use prometheus_client::{
    encoding::{EncodeLabelSet, EncodeLabelValue, text::encode},
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
    registry::Registry,
};
use serde::Serialize;
use std::{
    sync::{
        RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Fixed dimensions, never user-supplied paths, IDs, models or error messages.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, EncodeLabelValue)]
pub enum Operation {
    HttpRead,
    HttpCommand,
    Gateway,
    Grpc,
    Scheduler,
    Recovery,
    Adapter,
    Outbox,
}
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, EncodeLabelValue)]
enum ResultClass {
    Success,
    Failure,
}
#[derive(Clone, Debug, Hash, Eq, PartialEq, EncodeLabelSet)]
struct Labels {
    operation: Operation,
    result: ResultClass,
}

/// One process-local registry shared by all clones of a Core service.
pub struct Observability {
    registry: Registry,
    started: Instant,
    operations: Family<Labels, Counter>,
    durations: Histogram,
    uptime: Gauge,
    postgres_up: Gauge,
    supervisor_up: Gauge,
    queued: Gauge,
    leases: Gauge,
    runs: Gauge,
    unknown: Gauge,
    incidents: Gauge,
    outbox: Gauge,
    dead_letters: Gauge,
    outbox_age: Gauge,
    spool_bytes: Gauge,
    require_nats: AtomicBool,
    nats: RwLock<Option<Context>>,
    probes: tokio::sync::Semaphore,
}

impl Default for Observability {
    fn default() -> Self {
        let mut registry = Registry::default();
        let operations = Family::default();
        let durations = Histogram::new([0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 30.0]);
        registry.register(
            "forge_core_operations",
            "Completed local operations by fixed class",
            operations.clone(),
        );
        registry.register(
            "forge_core_operation_duration_seconds",
            "Observed operation duration",
            durations.clone(),
        );
        let gauges: Vec<Gauge> = (0..13).map(|_| Gauge::default()).collect();
        for ((name, help), gauge) in [
            ("uptime_seconds", "Monotonic process uptime"),
            (
                "postgres_up",
                "Last bounded PostgreSQL projection succeeded",
            ),
            (
                "supervisor_up",
                "Authenticated Supervisor channel connected",
            ),
            ("queue_depth", "Canonical queued entries"),
            ("active_leases", "Canonical active leases"),
            (
                "active_runs",
                "Runs observed provisioning running or stopping",
            ),
            (
                "unknown_environments",
                "Unreleased environments with unknown presence",
            ),
            ("open_incidents", "Unresolved canonical Run incidents"),
            ("outbox_pending", "Pending and leased outbox entries"),
            ("outbox_dead_letters", "Retained dead-letter outbox entries"),
            (
                "outbox_oldest_seconds",
                "Age of the oldest unpublished outbox entry",
            ),
            (
                "evidence_spool_bytes",
                "Occupied local evidence spool bytes",
            ),
            ("process_up", "Metrics exporter process is responding"),
        ]
        .into_iter()
        .zip(&gauges)
        {
            registry.register(format!("forge_core_{name}"), help, gauge.clone());
        }
        gauges[12].set(1);
        Self {
            registry,
            started: Instant::now(),
            operations,
            durations,
            uptime: gauges[0].clone(),
            postgres_up: gauges[1].clone(),
            supervisor_up: gauges[2].clone(),
            queued: gauges[3].clone(),
            leases: gauges[4].clone(),
            runs: gauges[5].clone(),
            unknown: gauges[6].clone(),
            incidents: gauges[7].clone(),
            outbox: gauges[8].clone(),
            dead_letters: gauges[9].clone(),
            outbox_age: gauges[10].clone(),
            spool_bytes: gauges[11].clone(),
            require_nats: AtomicBool::new(false),
            nats: RwLock::new(None),
            probes: tokio::sync::Semaphore::new(2),
        }
    }
}

impl Observability {
    /// Observational only; recording cannot return an error into a command path.
    pub fn record(&self, operation: Operation, success: bool, elapsed: Duration) {
        self.operations
            .get_or_create(&Labels {
                operation,
                result: if success {
                    ResultClass::Success
                } else {
                    ResultClass::Failure
                },
            })
            .inc();
        self.durations.observe(elapsed.as_secs_f64());
    }
    pub(crate) fn observe_nats(&self, context: Context) {
        *self
            .nats
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(context);
    }
    fn update(&self, snapshot: OperationalSnapshot) {
        self.queued.set(snapshot.queued);
        self.leases.set(snapshot.active_leases);
        self.runs.set(snapshot.active_runs);
        self.unknown.set(snapshot.unknown_environments);
        self.incidents.set(snapshot.open_incidents);
        self.outbox.set(snapshot.pending_outbox);
        self.dead_letters.set(snapshot.dead_letter_outbox);
        self.outbox_age
            .set(snapshot.oldest_outbox_seconds.min(i64::MAX as f64) as i64);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Up,
    Down,
    Disabled,
}
impl CheckState {
    fn from_up(up: bool) -> Self {
        if up { Self::Up } else { Self::Down }
    }
}
/// Local dependency readiness, not permission to execute or provider health.
#[derive(Debug, Serialize)]
pub struct Readiness {
    pub ready: bool,
    pub postgres: CheckState,
    pub supervisor: CheckState,
    pub nats: CheckState,
    pub object_store: CheckState,
}

impl CoreService {
    /// Mark the optional outbox adapter as required for complete readiness.
    #[must_use]
    pub fn with_nats_health_required(self, required: bool) -> Self {
        self.observability
            .require_nats
            .store(required, Ordering::Release);
        self
    }

    /// Records a fixed operation class without touching canonical state.
    pub fn record_operation(&self, operation: Operation, success: bool, elapsed: Duration) {
        self.observability.record(operation, success, elapsed);
    }

    /// Probes only configured local infrastructure; no provider inference.
    pub async fn readiness(&self) -> Readiness {
        let supervisor = CheckState::from_up(
            self.supervisor.is_connected().await
                && (self.fake_runtime_enabled || self.supervisor.is_reconciled().await),
        );
        let postgres = async {
            CheckState::from_up(matches!(
                tokio::time::timeout(Duration::from_secs(2), self.store.healthcheck()).await,
                Ok(Ok(()))
            ))
        };
        let nats = async {
            if self.observability.require_nats.load(Ordering::Acquire) {
                let context = self
                    .observability
                    .nats
                    .read()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .clone();
                match context {
                    Some(context) => CheckState::from_up(matches!(
                        tokio::time::timeout(Duration::from_secs(2), context.query_account()).await,
                        Ok(Ok(_))
                    )),
                    None => CheckState::Down,
                }
            } else {
                CheckState::Disabled
            }
        };
        let object_store = async {
            match self
                .evidence
                .as_ref()
                .and_then(|runtime| runtime.object_store.as_ref())
            {
                Some(store) => CheckState::from_up(
                    tokio::time::timeout(Duration::from_secs(2), store.healthcheck())
                        .await
                        .unwrap_or(false),
                ),
                None => CheckState::Disabled,
            }
        };
        let (postgres, nats, object_store) = tokio::join!(postgres, nats, object_store);
        Readiness {
            ready: [postgres, supervisor, nats, object_store]
                .iter()
                .all(|state| *state != CheckState::Down),
            postgres,
            supervisor,
            nats,
            object_store,
        }
    }

    /// Exports process metrics even when canonical projections are unavailable.
    pub async fn metrics(&self) -> Result<String, std::fmt::Error> {
        let observation = &self.observability;
        observation
            .uptime
            .set(i64::try_from(observation.started.elapsed().as_secs()).unwrap_or(i64::MAX));
        observation
            .supervisor_up
            .set(i64::from(self.supervisor.is_connected().await));
        match tokio::time::timeout(Duration::from_secs(2), self.store.operational_snapshot()).await
        {
            Ok(Ok(snapshot)) => {
                observation.postgres_up.set(1);
                observation.update(snapshot);
            }
            _ => {
                observation.postgres_up.set(0);
            }
        }
        if let Some(runtime) = &self.evidence
            && let Ok(bytes) = runtime.spool.used_bytes()
        {
            observation
                .spool_bytes
                .set(i64::try_from(bytes).unwrap_or(i64::MAX));
        }
        let mut text = String::new();
        encode(&mut text, &observation.registry)?;
        Ok(text)
    }
}

/// Starts a metrics-only loopback listener. Export failure is diagnostic only.
pub async fn serve_local(
    core: CoreService,
    address: std::net::SocketAddr,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Option<tokio::task::JoinHandle<()>> {
    if !address.ip().is_loopback() {
        tracing::warn!(
            component = "observability",
            "refused non-loopback metrics listener"
        );
        return None;
    }
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(_) => {
            tracing::warn!(
                component = "observability",
                "metrics listener unavailable; canonical service continues"
            );
            return None;
        }
    };
    Some(tokio::spawn(async move {
        let server = axum::serve(listener, router(core));
        tokio::select! { _ = server => {}, _ = shutdown.changed() => {} }
    }))
}
