//! Reconnecting Forge Supervisor with durable, fenced execution observations.
//!
//! Core remains the canonical writer. A Core transport interruption never stops
//! a worker; events remain in an owner-only bounded journal until acknowledged.

#![forbid(unsafe_code)]

use forge_protocol::{
    local_paths::default_runtime_directory,
    supervisor::v1::{
        AcknowledgementDisposition, CoreAcknowledgement, CoreToSupervisor, SupervisorHello,
        SupervisorInventory, SupervisorToCore, core_to_supervisor,
        supervisor_control_client::SupervisorControlClient, supervisor_to_core,
    },
};
use std::{
    collections::HashSet,
    io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
    time::{Duration, Instant, sleep},
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, info, warn};
use uuid::Uuid;

mod fake;
/// Local Git candidate snapshots and explicitly authorized integration effects.
pub mod git;
mod journal;
mod observability;
mod podman;
mod registry;
/// Container-local durable process wrapper, not a host execution API.
pub mod runner;
mod runtime_input;
mod surface;
mod transport;

use journal::{Journal, message_id, retryable_ack};
pub(crate) use registry::RunControl;
use registry::{EventSink, RunRegistry};

/// The only RunSpec version understood by the deterministic M0 executor.
pub const M0_RUN_SPEC_VERSION: u32 = 1;
const PROTOCOL_MAJOR: u32 = 1;
const OUTBOUND_CHANNEL_CAPACITY: usize = 64;
const STOP_GRACE_PERIOD: Duration = Duration::from_secs(2);

/// Connection identity and replay storage for one Supervisor process.
#[derive(Clone, Debug)]
pub struct SupervisorConfig {
    /// Unix-domain socket served by Core's Supervisor gRPC service.
    pub socket_path: PathBuf,
    /// Stable identity of the execution host.
    pub host_id: String,
    /// Current host boot identity.
    pub boot_id: String,
    /// Identity of this process, retained across Core reconnects.
    pub supervisor_instance_id: String,
    /// Delay before reconnecting to an unavailable Core.
    pub reconnect_delay: Duration,
    /// Owner-only replay state, retained across process and host restarts.
    pub state_directory: PathBuf,
    /// Upper bound for replay state; full journals refuse further work.
    pub journal_max_bytes: usize,
    /// Admission budget for retained raw evidence plus active output reservations.
    /// This is not a filesystem quota for arbitrary employee-created files.
    pub evidence_max_bytes: u64,
    /// Stable process start time, not the time of each reconnect handshake.
    pub started_at_unix_ms: i64,
    /// Trusted rootless Podman executable; never selected by a worker.
    pub podman_binary: PathBuf,
    /// Core materializes invocation/stdin and scoped credentials here.
    pub grants_directory: PathBuf,
    /// Per-run Core Gateway sockets; the administrative socket is never mounted.
    pub gateways_directory: PathBuf,
    /// Optional read-only loopback exporter, separate from the Core control UDS.
    pub observability_address: Option<std::net::SocketAddr>,
}

impl SupervisorConfig {
    /// Creates a configuration with replay state beside the supplied socket.
    /// Daemon composition overrides this when sockets use volatile storage.
    #[must_use]
    pub fn new(socket_path: PathBuf, host_id: String, boot_id: String) -> Self {
        let state_directory = socket_path.with_extension("supervisor-state");
        let grants_directory = state_directory.join("grants");
        let gateways_directory = state_directory.join("gateways");
        Self {
            socket_path,
            host_id,
            boot_id,
            supervisor_instance_id: new_id(),
            reconnect_delay: Duration::from_millis(500),
            state_directory,
            journal_max_bytes: 16 * 1024 * 1024,
            evidence_max_bytes: 2 * 1024 * 1024 * 1024,
            started_at_unix_ms: now_millis(),
            podman_binary: PathBuf::from("podman"),
            grants_directory,
            gateways_directory,
            observability_address: None,
        }
    }
}

/// Reads the Linux boot identity for reboot reconciliation.
pub fn current_boot_id() -> Result<String, SupervisorError> {
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(SupervisorError::HostBootIdentity)?;
    let boot_id = boot_id.trim();
    if boot_id.is_empty() {
        return Err(SupervisorError::InvalidHostBootIdentity);
    }
    Ok(boot_id.to_owned())
}

/// Shared CWD-independent Core socket convention.
#[must_use]
pub fn default_socket_path() -> PathBuf {
    default_runtime_directory().join("core.sock")
}

/// Persistent Supervisor state, deliberately independent of XDG_RUNTIME_DIR.
#[must_use]
pub fn default_state_directory() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("forge/supervisor"))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".local/state/forge/supervisor"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp/forge-supervisor-state"))
}

/// Local transport, execution and replay-storage errors. Diagnostics omit payloads.
#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("could not read the Linux host boot identity")]
    HostBootIdentity(#[source] io::Error),
    #[error("the Linux host boot identity is blank")]
    InvalidHostBootIdentity,
    #[error("could not connect to Forge Core over the Supervisor socket")]
    Transport(#[from] tonic::transport::Error),
    #[error("Core socket authentication boundary is not owner-only")]
    UnsafeSocket,
    #[error("Forge Core rejected or ended the Supervisor gRPC session")]
    Rpc(#[from] tonic::Status),
    #[error("Forge Core closed the Supervisor outbound stream")]
    CoreStreamClosed,
    #[error("could not serialize Supervisor JSON payload")]
    Json(#[from] serde_json::Error),
    #[error("run-scoped Supervisor sequence overflowed")]
    SequenceOverflow,
    #[error("Supervisor operational journal I/O failed")]
    JournalIo(#[from] io::Error),
    #[error("Supervisor operational journal is already locked")]
    JournalInUse,
    #[error("Supervisor journal storage is not owner-only")]
    UnsafeJournal,
    #[error("Supervisor journal capacity exhausted")]
    JournalFull,
    #[error(
        "Supervisor unresolved environment inventory capacity exhausted; resolve retained environments before admitting new work"
    )]
    InventoryCapacity,
    #[error(
        "retained evidence capacity exhausted; archive policy or a larger explicit budget is required"
    )]
    EvidenceCapacity,
    #[error("Supervisor journal version or host identity does not match")]
    JournalIdentity,
    #[error("Supervisor message violates journal scope or sequence")]
    InvalidJournalMessage,
    #[error("provision conflicts with retained execution identity")]
    ConflictingProvision,
    #[error("sandbox RunSpec or private invocation is invalid")]
    InvalidRunSpec,
    #[error("sandbox source or private runtime path is unsafe")]
    UnsafeSurface,
    #[error("Task-private Git workspace preparation failed")]
    SurfacePreparationFailed,
    #[error("rootless Podman with cgroup v2 is required")]
    RootlessUnavailable,
    #[error("pinned runtime executable failed the isolated version preflight")]
    RuntimePreflightFailed,
    #[error("Podman operation did not provide reliable environment evidence")]
    PodmanFailed,
}

/// Runs until shutdown. Transport reconnects never recreate or cancel workers.
pub async fn run(
    config: SupervisorConfig,
    shutdown: watch::Receiver<bool>,
) -> Result<(), SupervisorError> {
    let journal = Journal::open(
        &config.state_directory,
        &config.host_id,
        config.journal_max_bytes,
    )?;
    let registry = RunRegistry::new(journal);
    let observability = observability::serve(&config, registry.clone(), shutdown.clone()).await;
    let mut workers = JoinSet::new();
    let backend = podman::PodmanBackend::new(&config);
    for provision in backend.reconcile(&registry).await? {
        let control = registry.adopt_control(&provision).await;
        let worker_backend = backend.clone();
        let worker_registry = registry.clone();
        workers.spawn(async move {
            worker_backend
                .run(provision, worker_registry, control, true)
                .await
        });
    }
    loop {
        if shutdown_requested(&shutdown) {
            break;
        }
        match serve_session(&config, shutdown.clone(), &registry, &mut workers, &backend).await {
            Ok(SessionEnd::Shutdown) => break,
            Ok(SessionEnd::Disconnected) => {
                warn!(socket = %config.socket_path.display(), "Core disconnected Supervisor session")
            }
            Err(error) => {
                warn!(socket = %config.socket_path.display(), error = %error, "Supervisor session unavailable")
            }
        }
        registry
            .connected
            .store(false, std::sync::atomic::Ordering::Release);
        if wait_to_reconnect(shutdown.clone(), config.reconnect_delay).await {
            break;
        }
    }
    registry.request_stop_all().await;
    drain_workers(&mut workers).await;
    if let Some(task) = observability {
        task.abort();
        let _ = task.await;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionEnd {
    Shutdown,
    Disconnected,
}

async fn serve_session(
    config: &SupervisorConfig,
    mut shutdown: watch::Receiver<bool>,
    registry: &RunRegistry,
    workers: &mut JoinSet<Result<(), SupervisorError>>,
    backend: &podman::PodmanBackend,
) -> Result<SessionEnd, SupervisorError> {
    let channel = transport::connect(&config.socket_path).await?;
    let mut client = SupervisorControlClient::new(channel);
    let (outbound, receiver) = mpsc::channel(OUTBOUND_CHANNEL_CAPACITY);
    outbound
        .send(SupervisorToCore {
            message: Some(supervisor_to_core::Message::Hello(hello_for(config))),
        })
        .await
        .map_err(|_| SupervisorError::CoreStreamClosed)?;
    let mut inbound = client
        .supervisor_session(ReceiverStream::new(receiver))
        .await?
        .into_inner();
    registry
        .connected
        .store(true, std::sync::atomic::Ordering::Release);
    let mut sent = HashSet::new();
    info!(socket = %config.socket_path.display(), "Supervisor connected to Core");
    loop {
        if shutdown_requested(&shutdown) {
            return Ok(SessionEnd::Shutdown);
        }
        let pending = registry
            .journal
            .lock()
            .await
            .pending_heads()
            .find(|message| message_id(message).is_some_and(|id| !sent.contains(id)))
            .cloned();
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || shutdown_requested(&shutdown) { return Ok(SessionEnd::Shutdown); }
            }
            message = inbound.message() => match message? {
                Some(CoreToSupervisor { message: Some(core_to_supervisor::Message::ProvisionRun(provision)) }) => {
                    match registry.register(&provision, &config.boot_id).await {
                        Ok(Some(control)) => {
                            let worker_registry = registry.clone();
                            let worker_backend = backend.clone();
                            workers.spawn(async move {
                                if matches!(provision.run_spec_version,2..=6) {
                                    return worker_backend.run(provision, worker_registry, control, false).await;
                                }
                                let result = fake::execute_provision(provision.clone(), worker_registry.sink(), control).await;
                                // Fake work exists only in this future. An error still
                                // proves it stopped, but cannot imply Task completion.
                                worker_registry.finish(&provision, true).await?;
                                result
                            });
                        }
                        Ok(None) => send_inventory(&outbound, config, registry, provision.command_id).await?,
                        Err(SupervisorError::ConflictingProvision) => {
                            warn!(run_id = %provision.run_id, "refused conflicting provision without replacing existing execution");
                            send_inventory(&outbound, config, registry, provision.command_id).await?;
                        }
                        Err(error) => return Err(error),
                    }
                }
                Some(CoreToSupervisor { message: Some(core_to_supervisor::Message::StopRun(stop)) }) => {
                    if !registry.request_stop(&stop).await { debug!(run_id = %stop.run_id, "ignored stale or unknown Core stop request"); }
                }
                Some(CoreToSupervisor { message: Some(core_to_supervisor::Message::DeliverRuntimeInput(input)) }) => {
                    let config=config.clone();let registry=registry.clone();
                    workers.spawn(async move {runtime_input::deliver(&config,&registry,input).await});
                }
                Some(CoreToSupervisor { message: Some(core_to_supervisor::Message::Acknowledgement(ack)) }) => {
                    log_acknowledgement(&ack);
                    let consumed=registry.journal.lock().await.input_for_receipt(&ack.acknowledged_message_id);
                    registry.journal.lock().await.acknowledge(&ack)?;
                    if retryable_ack(&ack) {
                        return Ok(SessionEnd::Disconnected);
                    }
                    if matches!(AcknowledgementDisposition::try_from(ack.disposition),Ok(AcknowledgementDisposition::Accepted|AcknowledgementDisposition::IgnoredStale|AcknowledgementDisposition::Rejected)) && let Some(input)=consumed {
                        let _=runtime_input::cleanup_mailbox(config,&input);
                    }
                    sent.remove(&ack.acknowledged_message_id);
                }
                Some(CoreToSupervisor { message: Some(core_to_supervisor::Message::RequestInventory(request)) }) => {
                    send_inventory(&outbound, config, registry, request.command_id).await?;
                }
                Some(CoreToSupervisor { message: Some(core_to_supervisor::Message::InspectGitCandidate(request)) }) => {
                    let backend = backend.clone();
                    let registry = registry.clone();
                    workers.spawn(async move { backend.inspect_git_candidate(request, registry).await });
                }
                Some(CoreToSupervisor { message: Some(core_to_supervisor::Message::GitIntegration(request)) }) => {
                    let backend=backend.clone();let registry=registry.clone();
                    workers.spawn(async move {backend.integrate_git(request,registry).await});
                }
                Some(CoreToSupervisor { message: Some(core_to_supervisor::Message::CaptureFileSnapshot(request)) }) => {
                    let backend = backend.clone();
                    let registry = registry.clone();
                    workers.spawn(async move { backend.capture_file_snapshot(request, registry).await });
                }
                Some(CoreToSupervisor { message: None }) => warn!("received empty Core-to-Supervisor envelope"),
                None => return Ok(SessionEnd::Disconnected),
            },
            permit = outbound.reserve(), if pending.is_some() => {
                let permit = permit.map_err(|_| SupervisorError::CoreStreamClosed)?;
                if let Some(message) = pending {
                    if let Some(id) = message_id(&message) { sent.insert(id.clone()); }
                    permit.send(message);
                }
            }
            () = registry.changed.notified() => {}
            completed = workers.join_next(), if !workers.is_empty() => {
                match completed {
                    Some(Ok(Ok(()))) | None => {}
                    Some(Ok(Err(error))) => warn!(error = %error, "Run observation delivery failed"),
                    Some(Err(error)) => warn!(error = %error, "Run worker failed"),
                }
            }
        }
    }
}

async fn send_inventory(
    outbound: &mpsc::Sender<SupervisorToCore>,
    config: &SupervisorConfig,
    registry: &RunRegistry,
    request_command_id: String,
) -> Result<(), SupervisorError> {
    let inventory = SupervisorInventory {
        message_id: new_id(),
        request_command_id,
        host_id: config.host_id.clone(),
        boot_id: config.boot_id.clone(),
        entries: registry.journal.lock().await.inventory(),
    };
    outbound
        .send(SupervisorToCore {
            message: Some(supervisor_to_core::Message::Inventory(inventory)),
        })
        .await
        .map_err(|_| SupervisorError::CoreStreamClosed)
}

fn hello_for(config: &SupervisorConfig) -> SupervisorHello {
    SupervisorHello {
        message_id: new_id(),
        supervisor_instance_id: config.supervisor_instance_id.clone(),
        host_id: config.host_id.clone(),
        boot_id: config.boot_id.clone(),
        protocol_major: PROTOCOL_MAJOR,
        started_at_unix_ms: config.started_at_unix_ms,
    }
}

pub(crate) async fn send(
    outbound: &EventSink,
    message: SupervisorToCore,
) -> Result<(), SupervisorError> {
    outbound.send(message).await
}

fn log_acknowledgement(ack: &CoreAcknowledgement) {
    match AcknowledgementDisposition::try_from(ack.disposition).ok() {
        Some(AcknowledgementDisposition::Accepted) => {
            debug!(message_id = %ack.acknowledged_message_id, "Core accepted Supervisor message")
        }
        disposition => {
            warn!(message_id = %ack.acknowledged_message_id, disposition = ?disposition, reason_code = %ack.reason_code, "Core did not accept Supervisor message")
        }
    }
}

async fn drain_workers(workers: &mut JoinSet<Result<(), SupervisorError>>) {
    let deadline = Instant::now() + STOP_GRACE_PERIOD;
    while !workers.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero()
            || tokio::time::timeout(remaining, workers.join_next())
                .await
                .is_err()
        {
            workers.abort_all();
            break;
        }
    }
    while workers.join_next().await.is_some() {}
}

fn shutdown_requested(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

async fn wait_to_reconnect(mut shutdown: watch::Receiver<bool>, delay: Duration) -> bool {
    tokio::select! { () = sleep(delay) => false, changed = shutdown.changed() => changed.is_err() || shutdown_requested(&shutdown) }
}

pub(crate) fn new_id() -> String {
    Uuid::now_v7().to_string()
}

pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(i64::MAX as u128) as i64
        })
}

mod execution_assignment;
#[cfg(test)]
mod podman_tests;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod tests;
