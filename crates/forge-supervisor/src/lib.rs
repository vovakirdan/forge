//! Deterministic M0 Supervisor process boundary for Forge Runs.
//!
//! This crate is deliberately only a transport-side simulator. It never
//! mutates Forge state, provisions a container, or invokes a provider. Core
//! remains the canonical writer and validates every emitted submission.

#![forbid(unsafe_code)]

use std::{
    collections::HashMap,
    io,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use forge_protocol::{
    local_paths::default_runtime_directory,
    supervisor::v1::{
        AcknowledgementDisposition, CoreAcknowledgement, CoreToSupervisor, ProvisionRun, StopRun,
        SupervisorHello, SupervisorToCore, core_to_supervisor,
        supervisor_control_client::SupervisorControlClient, supervisor_to_core,
    },
};
use hyper_util::rt::TokioIo;
use nix::unistd::Uid;
use thiserror::Error;
use tokio::{
    net::UnixStream,
    sync::{Mutex, Notify, mpsc, watch},
    task::JoinSet,
    time::{Duration, Instant, sleep},
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;
use tracing::{debug, info, warn};
use uuid::Uuid;

mod fake;

/// The only RunSpec version understood by the deterministic M0 executor.
pub const M0_RUN_SPEC_VERSION: u32 = 1;

const PROTOCOL_MAJOR: u32 = 1;
const OUTBOUND_CHANNEL_CAPACITY: usize = 64;
const STOP_GRACE_PERIOD: Duration = Duration::from_secs(2);

/// Connection identity and retry configuration for one Supervisor process.
#[derive(Clone, Debug)]
pub struct SupervisorConfig {
    /// Unix-domain socket served by Forge Core's Supervisor gRPC service.
    pub socket_path: PathBuf,
    /// Stable identity of the local execution host.
    pub host_id: String,
    /// Identity of the current host boot used by recovery reconciliation.
    pub boot_id: String,
    /// Stable identity of this Supervisor process across Core reconnects.
    pub supervisor_instance_id: String,
    /// Pause before reconnecting after Core is unavailable or disconnects.
    pub reconnect_delay: Duration,
}

impl SupervisorConfig {
    /// Creates a Supervisor configuration using the supplied Core socket and host identity.
    #[must_use]
    pub fn new(socket_path: PathBuf, host_id: String, boot_id: String) -> Self {
        Self {
            socket_path,
            host_id,
            boot_id,
            supervisor_instance_id: new_id(),
            reconnect_delay: Duration::from_millis(500),
        }
    }
}

/// Reads the Linux host boot identity used to distinguish reboot recovery from
/// a transient Supervisor reconnect.
pub fn current_boot_id() -> Result<String, SupervisorError> {
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(SupervisorError::HostBootIdentity)?;
    let boot_id = boot_id.trim();
    if boot_id.is_empty() {
        return Err(SupervisorError::InvalidHostBootIdentity);
    }
    Ok(boot_id.to_owned())
}

/// Returns the shared CWD-independent local Core socket convention.
#[must_use]
pub fn default_socket_path() -> PathBuf {
    default_runtime_directory().join("core.sock")
}

/// Errors local to the Supervisor transport and fake executor.
#[derive(Debug, Error)]
pub enum SupervisorError {
    /// The Linux host boot identity could not be read.
    #[error("could not read the Linux host boot identity")]
    HostBootIdentity(#[source] io::Error),

    /// The Linux host boot identity was blank.
    #[error("the Linux host boot identity is blank")]
    InvalidHostBootIdentity,

    /// Core's local gRPC endpoint could not be reached.
    #[error("could not connect to Forge Core over the Supervisor socket")]
    Transport(#[from] tonic::transport::Error),

    /// The local Core socket or enclosing directory is not owner-only.
    #[error("Core socket authentication boundary is not owner-only")]
    UnsafeSocket,

    /// Core ended or rejected the gRPC session.
    #[error("Forge Core rejected or ended the Supervisor gRPC session")]
    Rpc(#[from] tonic::Status),

    /// Core's inbound gRPC stream no longer accepts messages.
    #[error("Forge Core closed the Supervisor outbound stream")]
    CoreStreamClosed,

    /// A value could not be serialized into a safe JSON wire payload.
    #[error("could not serialize Supervisor JSON payload")]
    Json(#[from] serde_json::Error),

    /// A per-run sequence cannot advance without overflowing the protocol field.
    #[error("run-scoped Supervisor sequence overflowed")]
    SequenceOverflow,
}

/// Runs a reconnecting Supervisor control client until `shutdown` becomes true.
///
/// A temporary unavailable Core is normal during local startup. This loop never
/// exposes `run_spec_json` or Core acknowledgement text in logs.
pub async fn run(
    config: SupervisorConfig,
    shutdown: watch::Receiver<bool>,
) -> Result<(), SupervisorError> {
    loop {
        if shutdown_requested(&shutdown) {
            return Ok(());
        }

        match serve_session(&config, shutdown.clone()).await {
            Ok(SessionEnd::Shutdown) => return Ok(()),
            Ok(SessionEnd::Disconnected) => {
                warn!(socket = %config.socket_path.display(), "Core disconnected Supervisor session");
            }
            Err(error) => {
                warn!(socket = %config.socket_path.display(), error = %error, "Supervisor session unavailable");
            }
        }

        if wait_to_reconnect(shutdown.clone(), config.reconnect_delay).await {
            return Ok(());
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionEnd {
    Shutdown,
    Disconnected,
}

async fn serve_session(
    config: &SupervisorConfig,
    mut shutdown: watch::Receiver<bool>,
) -> Result<SessionEnd, SupervisorError> {
    let channel = connect(&config.socket_path).await?;
    let mut client = SupervisorControlClient::new(channel);
    let (outbound, receiver) = mpsc::channel(OUTBOUND_CHANNEL_CAPACITY);
    // The first client-stream frame is the identity handshake. Queue it before
    // opening the RPC so a Core that waits for Hello cannot deadlock the session.
    send_hello(&outbound, config).await?;
    let response = client
        .supervisor_session(ReceiverStream::new(receiver))
        .await?;
    let mut inbound = response.into_inner();

    info!(socket = %config.socket_path.display(), "Supervisor connected to Core");

    let registry = RunRegistry::default();
    let mut workers = JoinSet::new();

    loop {
        if shutdown_requested(&shutdown) {
            registry.request_stop_all().await;
            drain_workers(&mut workers).await;
            return Ok(SessionEnd::Shutdown);
        }

        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || shutdown_requested(&shutdown) {
                    registry.request_stop_all().await;
                    drain_workers(&mut workers).await;
                    return Ok(SessionEnd::Shutdown);
                }
            }
            message = inbound.message() => match message? {
                Some(CoreToSupervisor { message: Some(core_to_supervisor::Message::ProvisionRun(provision)) }) => {
                    let control = registry.register(&provision).await;
                    let worker_outbound = outbound.clone();
                    let worker_registry = registry.clone();
                    workers.spawn(async move {
                        let result = fake::execute_provision(provision, worker_outbound, control.clone()).await;
                        worker_registry.remove(&control).await;
                        result
                    });
                }
                Some(CoreToSupervisor { message: Some(core_to_supervisor::Message::StopRun(stop)) }) => {
                    if !registry.request_stop(&stop).await {
                        debug!(run_id = %stop.run_id, "ignored stale or unknown Core stop request");
                    }
                }
                Some(CoreToSupervisor { message: Some(core_to_supervisor::Message::Acknowledgement(ack)) }) => {
                    log_acknowledgement(&ack);
                }
                Some(CoreToSupervisor { message: None }) => {
                    warn!("received empty Core-to-Supervisor envelope");
                }
                None => {
                    registry.request_stop_all().await;
                    drain_workers(&mut workers).await;
                    return Ok(SessionEnd::Disconnected);
                }
            },
            completed = workers.join_next(), if !workers.is_empty() => {
                match completed {
                    Some(Ok(Ok(()))) => {}
                    Some(Ok(Err(error))) => warn!(error = %error, "fake Run ended before normal completion"),
                    Some(Err(error)) => warn!(error = %error, "fake Run worker failed"),
                    None => {}
                }
            }
        }
    }
}

async fn connect(socket_path: &Path) -> Result<Channel, SupervisorError> {
    validate_socket_path(socket_path)?;
    let socket_path = socket_path.to_owned();
    let expected_uid = Uid::effective().as_raw();
    Endpoint::from_static("http://[::]:50051")
        .connect_with_connector(service_fn(move |_| {
            let socket_path = socket_path.clone();
            async move {
                let stream = UnixStream::connect(socket_path).await?;
                if stream.peer_cred()?.uid() != expected_uid {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Core peer identity does not match local Supervisor user",
                    ));
                }
                Ok::<_, io::Error>(TokioIo::new(stream))
            }
        }))
        .await
        .map_err(SupervisorError::Transport)
}

fn validate_socket_path(socket_path: &Path) -> Result<(), SupervisorError> {
    let owner = Uid::effective().as_raw();
    let directory = socket_path.parent().ok_or(SupervisorError::UnsafeSocket)?;
    let directory_metadata =
        std::fs::symlink_metadata(directory).map_err(|_| SupervisorError::UnsafeSocket)?;
    let socket_metadata =
        std::fs::symlink_metadata(socket_path).map_err(|_| SupervisorError::UnsafeSocket)?;
    let directory_is_safe = directory_metadata.file_type().is_dir()
        && directory_metadata.uid() == owner
        && directory_metadata.mode() & 0o077 == 0;
    let socket_is_safe = socket_metadata.file_type().is_socket()
        && socket_metadata.uid() == owner
        && socket_metadata.mode() & 0o077 == 0;
    if directory_is_safe && socket_is_safe {
        Ok(())
    } else {
        Err(SupervisorError::UnsafeSocket)
    }
}

async fn send_hello(
    outbound: &mpsc::Sender<SupervisorToCore>,
    config: &SupervisorConfig,
) -> Result<(), SupervisorError> {
    let hello = hello_for(config);
    send(
        outbound,
        SupervisorToCore {
            message: Some(supervisor_to_core::Message::Hello(hello)),
        },
    )
    .await
}

fn hello_for(config: &SupervisorConfig) -> SupervisorHello {
    SupervisorHello {
        message_id: new_id(),
        supervisor_instance_id: config.supervisor_instance_id.clone(),
        host_id: config.host_id.clone(),
        boot_id: config.boot_id.clone(),
        protocol_major: PROTOCOL_MAJOR,
        started_at_unix_ms: now_millis(),
    }
}

pub(crate) async fn send(
    outbound: &mpsc::Sender<SupervisorToCore>,
    message: SupervisorToCore,
) -> Result<(), SupervisorError> {
    outbound
        .send(message)
        .await
        .map_err(|_| SupervisorError::CoreStreamClosed)
}

fn log_acknowledgement(ack: &CoreAcknowledgement) {
    match AcknowledgementDisposition::try_from(ack.disposition).ok() {
        Some(AcknowledgementDisposition::Accepted) => {
            debug!(message_id = %ack.acknowledged_message_id, "Core accepted Supervisor message");
        }
        disposition => {
            warn!(
                message_id = %ack.acknowledged_message_id,
                disposition = ?disposition,
                reason_code = %ack.reason_code,
                "Core did not accept Supervisor message"
            );
        }
    }
}

#[derive(Default)]
struct RunRegistry {
    active: Arc<Mutex<HashMap<String, Arc<RunControl>>>>,
}

impl Clone for RunRegistry {
    fn clone(&self) -> Self {
        Self {
            active: Arc::clone(&self.active),
        }
    }
}

impl RunRegistry {
    async fn register(&self, provision: &ProvisionRun) -> Arc<RunControl> {
        let next = Arc::new(RunControl::new(
            provision.run_id.clone(),
            provision.lease_fencing_token,
            provision.environment_epoch,
        ));
        let previous = self
            .active
            .lock()
            .await
            .insert(provision.run_id.clone(), Arc::clone(&next));
        if let Some(previous) = previous {
            previous.request_stop();
        }
        next
    }

    async fn request_stop(&self, request: &StopRun) -> bool {
        let active = self.active.lock().await.get(&request.run_id).cloned();
        match active {
            Some(active)
                if active.lease_fencing_token == request.lease_fencing_token
                    && active.environment_epoch == request.environment_epoch =>
            {
                active.request_stop();
                true
            }
            _ => false,
        }
    }

    async fn request_stop_all(&self) {
        let active: Vec<_> = self.active.lock().await.values().cloned().collect();
        for run in active {
            run.request_stop();
        }
    }

    async fn remove(&self, control: &Arc<RunControl>) {
        let mut active = self.active.lock().await;
        if active
            .get(&control.run_id)
            .is_some_and(|current| Arc::ptr_eq(current, control))
        {
            active.remove(&control.run_id);
        }
    }
}

pub(crate) struct RunControl {
    run_id: String,
    lease_fencing_token: u64,
    environment_epoch: u64,
    stopped: AtomicBool,
    stop_notification: Notify,
}

impl RunControl {
    fn new(run_id: String, lease_fencing_token: u64, environment_epoch: u64) -> Self {
        Self {
            run_id,
            lease_fencing_token,
            environment_epoch,
            stopped: AtomicBool::new(false),
            stop_notification: Notify::new(),
        }
    }

    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    fn request_stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.stop_notification.notify_one();
    }

    pub(crate) async fn stop_notified(&self) {
        self.stop_notification.notified().await;
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
    tokio::select! {
        () = sleep(delay) => false,
        changed = shutdown.changed() => changed.is_err() || shutdown_requested(&shutdown),
    }
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{SupervisorConfig, hello_for};

    #[test]
    fn reconnect_hellos_retain_the_process_identity() {
        let config = SupervisorConfig::new(
            PathBuf::from("/tmp/forge-supervisor-test.sock"),
            "test-host".to_owned(),
            "test-boot".to_owned(),
        );

        let first = hello_for(&config);
        let second = hello_for(&config);

        assert_eq!(first.supervisor_instance_id, second.supervisor_instance_id);
        assert_ne!(first.message_id, second.message_id);
    }
}
