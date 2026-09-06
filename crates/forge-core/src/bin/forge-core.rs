//! Local Forge Core daemon: HTTP/SSE and Supervisor gRPC over owner-only UDS.

use std::{
    fs,
    future::Future,
    io::ErrorKind,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::UnixStream as StdUnixStream,
    },
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use forge_core::{
    CoreActors, CoreConfig, CoreService, OutboxPublisher, OutboxPublisherConfig, RuntimePaths,
    SupervisorHub, SupervisorService, router,
};
use forge_domain::ActorId;
use forge_protocol::supervisor::v1::supervisor_control_server::SupervisorControlServer;
use forge_storage::PostgresStore;
use nix::unistd::Uid;
use tokio::{net::UnixListener, sync::watch};
use tokio_stream::{StreamExt, wrappers::UnixListenerStream};
use tonic::transport::Server;
use tracing::info;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

// M0 has one unauthenticated local operator and no identity persistence yet.
// These process-independent UUIDv7 values preserve command idempotency across
// Core restarts until local authentication becomes a real product capability.
const M0_LOCAL_HUMAN_ACTOR_ID: &str = "01900000-0000-7000-8000-000000000001";
const M0_CORE_ACTOR_ID: &str = "01900000-0000-7000-8000-000000000002";
const OUTBOX_STREAM_NAME: &str = "FORGE_EVENTS_V1";
const OUTBOX_SUBJECT: &str = "forge.v1.>";
const OUTBOX_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const OUTBOX_RECONNECT_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Parser)]
#[command(name = "forge-core", version, about = "Forge local canonical Core")]
struct Arguments {
    /// Read-only local health/metrics listener, never the management API.
    #[arg(long, default_value = "127.0.0.1:9878")]
    observability_address: std::net::SocketAddr,
    /// Owner-only S3/MinIO JSON config. Absence retains PendingUpload locally.
    #[arg(long, requires = "execution_root")]
    evidence_config: Option<PathBuf>,
    /// Trusted local LiteLLM proxy endpoint (Core administration only).
    #[arg(long, requires = "litellm_master_key")]
    litellm_url: Option<String>,
    /// Owner-only file holding the LiteLLM administrative key.
    #[arg(long, requires = "litellm_url")]
    litellm_master_key: Option<PathBuf>,
    /// Shared owner-only Supervisor state root for isolated runtime grants.
    #[arg(long)]
    execution_root: Option<PathBuf>,
    /// Previously initialized Forge Secret Store directory.
    #[arg(long)]
    secret_store: Option<PathBuf>,
    /// Explicit deterministic simulator mode; never enabled automatically.
    #[arg(long)]
    fake_runtime: bool,
    /// PostgreSQL URL, or the FORGE_DATABASE_URL environment value.
    #[arg(long, value_name = "URL")]
    database_url: Option<String>,
    /// NATS JetStream URL, or the FORGE_NATS_URL environment value.
    #[arg(long, value_name = "URL")]
    nats_url: Option<String>,
    /// Local HTTP/JSON and SSE Unix-domain socket.
    #[arg(long, value_name = "PATH")]
    api_socket: Option<PathBuf>,
    /// Local Supervisor gRPC Unix-domain socket.
    #[arg(long, value_name = "PATH")]
    supervisor_socket: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init();
    match run(Arguments::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("forge-core: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(arguments: Arguments) -> Result<()> {
    let Arguments {
        observability_address,
        evidence_config,
        litellm_url,
        litellm_master_key,
        execution_root,
        secret_store,
        fake_runtime,
        database_url,
        nats_url,
        api_socket,
        supervisor_socket,
    } = arguments;
    let database_url = database_url
        .or_else(|| std::env::var("FORGE_DATABASE_URL").ok())
        .context("supply --database-url or FORGE_DATABASE_URL")?;
    let nats_url = nats_url.or_else(|| std::env::var("FORGE_NATS_URL").ok());
    let paths = RuntimePaths::from_environment();
    let api_socket_is_default = api_socket.is_none();
    let supervisor_socket_is_default = supervisor_socket.is_none();
    let api_socket = api_socket.unwrap_or(paths.api_socket);
    let supervisor_socket = supervisor_socket.unwrap_or(paths.supervisor_socket);
    if api_socket == supervisor_socket {
        bail!("API and Supervisor sockets must be distinct");
    }

    let config = CoreConfig::new(
        database_url,
        nats_url,
        RuntimePaths {
            api_socket: api_socket.clone(),
            supervisor_socket: supervisor_socket.clone(),
        },
    );
    let store = PostgresStore::connect(&config.database_url)
        .await
        .context("connect canonical PostgreSQL")?;
    store.validate_schema().await.context("validate schema")?;
    let core = CoreService::new(store, m0_actors(), Arc::new(SupervisorHub::default()))
        .with_nats_health_required(config.nats_url.is_some());
    let core = if fake_runtime {
        core.with_fake_runtime()
    } else {
        core
    };
    let core = match secret_store {
        Some(directory) => core.with_secret_store(
            forge_provider_common::SecretStore::load(&directory)
                .context("load the original Forge Secret Store key")?,
        ),
        None => core,
    };
    let core = match execution_root {
        Some(root) => core.with_execution_root(root)?,
        None => core,
    };
    let core = match evidence_config {
        Some(path) => core.with_evidence_config_file(&path)?,
        None => core,
    };
    let core = match (litellm_url, litellm_master_key) {
        (Some(url), Some(key)) => core.with_inference_proxy(&url, &key)?,
        _ => core,
    };
    let api_listener = bind_owner_socket(&api_socket, api_socket_is_default)?;
    let supervisor_listener = bind_owner_socket(&supervisor_socket, supervisor_socket_is_default)?;
    // PostgreSQL is the canonical command plane. NATS delivery is deliberately
    // background work, so a malformed or unavailable optional broker can never
    // prevent the local control sockets from becoming available.
    let outbox_task = start_outbox_publisher(core.clone(), config.nats_url.as_deref());
    info!(
        api_socket = %api_socket.display(),
        supervisor_socket = %supervisor_socket.display(),
        "Forge Core is ready"
    );

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let observability = forge_core::observability::serve_local(
        core.clone(),
        observability_address,
        shutdown_rx.clone(),
    )
    .await;
    let maintenance_core = core.clone();
    let mut maintenance_shutdown = shutdown_rx.clone();
    let maintenance = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                _=tick.tick()=>if let Err(error)=maintenance_core.watchdog_tick(forge_domain::Timestamp::now_utc(),forge_core::WatchdogDeadlines::default()).await {
                    tracing::warn!(error=%error,"watchdog tick failed; no uncertain reservation released");
                },
                _=maintenance_shutdown.changed()=>break,
            }
        }
    });
    let evidence_core = core.clone();
    let mut evidence_shutdown = shutdown_rx.clone();
    let evidence_worker = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                _=tick.tick()=>if let Err(error)=evidence_core.evidence_tick().await {tracing::warn!(error=%error,"evidence remains pending");},
                _=evidence_shutdown.changed()=>break,
            }
        }
    });
    let signal_task = tokio::spawn(wait_for_interrupt(shutdown_tx));
    let api = axum::serve(api_listener, router(core.clone()).into_make_service())
        .with_graceful_shutdown(wait_for_shutdown(shutdown_rx.clone()));
    let expected_supervisor_uid = Uid::effective().as_raw();
    let supervisor_incoming = UnixListenerStream::new(supervisor_listener).filter_map(
        move |incoming| match incoming {
            Ok(stream) => match stream.peer_cred() {
                Ok(peer) if peer.uid() == expected_supervisor_uid => {
                    Some(Ok::<_, std::io::Error>(stream))
                }
                Ok(peer) => {
                    tracing::warn!(peer_uid = peer.uid(), "rejected Supervisor socket peer");
                    None
                }
                Err(error) => {
                    tracing::warn!(error = %error, "could not read Supervisor socket peer credentials");
                    None
                }
            },
            Err(error) => {
                tracing::warn!(error = %error, "could not accept Supervisor socket connection");
                None
            }
        },
    );
    let supervisor = Server::builder()
        .add_service(SupervisorControlServer::new(SupervisorService::new(core)))
        .serve_with_incoming_shutdown(supervisor_incoming, wait_for_shutdown(shutdown_rx));
    let result = tokio::select! {
        result = api => result.context("serve local HTTP API"),
        result = supervisor => result.context("serve local Supervisor gRPC"),
    };
    signal_task.abort();
    let _ = signal_task.await;
    maintenance.abort();
    evidence_worker.abort();
    let _ = maintenance.await;
    let _ = evidence_worker.await;
    if let Some(task) = outbox_task {
        task.abort();
        let _ = task.await;
    }
    if let Some(task) = observability {
        task.abort();
        let _ = task.await;
    }
    result
}

fn start_outbox_publisher(
    core: CoreService,
    nats_url: Option<&str>,
) -> Option<tokio::task::JoinHandle<()>> {
    let nats_url = nats_url?;
    let worker_id = format!("core-{}", Uuid::now_v7());
    let publisher_config = match OutboxPublisherConfig::local_default(worker_id) {
        Ok(config) => config,
        Err(_) => {
            tracing::error!("Forge outbox publisher configuration is invalid");
            return None;
        }
    };
    Some(spawn_outbox_reconnector(
        nats_url.to_owned(),
        move |jetstream| {
            let publisher = core.outbox_publisher(jetstream, publisher_config.clone());
            run_outbox_publisher(publisher)
        },
    ))
}

fn spawn_outbox_reconnector<F, Fut>(
    nats_url: String,
    run_connected: F,
) -> tokio::task::JoinHandle<()>
where
    F: FnMut(async_nats::jetstream::Context) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(run_outbox_reconnector(nats_url, run_connected))
}

async fn run_outbox_reconnector<F, Fut>(nats_url: String, mut run_connected: F)
where
    F: FnMut(async_nats::jetstream::Context) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    loop {
        match connect_outbox(&nats_url).await {
            Ok(jetstream) => {
                info!("Forge outbox broker connected");
                run_connected(jetstream).await;
                tracing::warn!("Forge outbox delivery loop stopped; reconnecting");
            }
            Err(_) => tracing::warn!("Forge outbox broker unavailable; reconnecting"),
        }
        tokio::time::sleep(OUTBOX_RECONNECT_DELAY).await;
    }
}

async fn connect_outbox(nats_url: &str) -> Result<async_nats::jetstream::Context> {
    let client = connect_nats(nats_url).await?;
    let jetstream = async_nats::jetstream::new(client);
    ensure_outbox_stream(&jetstream).await?;
    Ok(jetstream)
}

async fn connect_nats(nats_url: &str) -> Result<async_nats::Client> {
    let address = nats_url
        .parse::<async_nats::ServerAddr>()
        .map_err(|_| anyhow::anyhow!("parse configured NATS JetStream URL"))?;
    let options = match (address.username(), address.password()) {
        // async-nats does not derive CONNECT credentials from URL userinfo. The
        // one-component form is Forge's local NATS token convention.
        (Some(token), None) => async_nats::ConnectOptions::with_token(token.to_owned()),
        (Some(user), Some(password)) => {
            async_nats::ConnectOptions::with_user_and_password(user.to_owned(), password.to_owned())
        }
        (None, None) => async_nats::ConnectOptions::new(),
        (None, Some(_)) => bail!("configured NATS URL has a password without a username"),
    };
    options
        .connection_timeout(OUTBOX_CONNECT_TIMEOUT)
        .connect(nats_url)
        .await
        .map_err(|_| anyhow::anyhow!("connect configured NATS JetStream"))
}

async fn ensure_outbox_stream(jetstream: &async_nats::jetstream::Context) -> Result<()> {
    let stream = jetstream
        .get_or_create_stream(outbox_stream_config())
        .await
        .map_err(|_| anyhow::anyhow!("ensure configured Forge JetStream stream"))?;
    if !stream
        .cached_info()
        .config
        .subjects
        .iter()
        .any(|subject| subject == OUTBOX_SUBJECT)
    {
        bail!("configured Forge JetStream stream does not retain {OUTBOX_SUBJECT}");
    }
    Ok(())
}

fn outbox_stream_config() -> async_nats::jetstream::stream::Config {
    async_nats::jetstream::stream::Config {
        name: OUTBOX_STREAM_NAME.to_owned(),
        subjects: vec![OUTBOX_SUBJECT.to_owned()],
        ..Default::default()
    }
}

async fn run_outbox_publisher(publisher: OutboxPublisher) {
    loop {
        let delay = match publisher.drain_once().await {
            Ok(report) if report.claimed == 0 => Duration::from_millis(500),
            Ok(_) => Duration::from_millis(10),
            Err(_) => {
                tracing::warn!("Forge outbox delivery failed; retrying bounded batch");
                Duration::from_secs(1)
            }
        };
        tokio::time::sleep(delay).await;
    }
}

fn m0_actors() -> CoreActors {
    let human = Uuid::parse_str(M0_LOCAL_HUMAN_ACTOR_ID)
        .expect("M0 local human actor constant must be a UUIDv7");
    let core = Uuid::parse_str(M0_CORE_ACTOR_ID).expect("M0 Core actor constant must be a UUIDv7");
    CoreActors::new(ActorId::from(human), ActorId::from(core))
}

fn bind_owner_socket(path: &Path, forge_owned_parent: bool) -> Result<UnixListener> {
    let parent = path
        .parent()
        .context("socket path must have a parent directory")?;
    prepare_socket_parent(parent, forge_owned_parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            remove_stale_socket(path)?;
        }
        Ok(_) => bail!("refusing to replace non-socket path {}", path.display()),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("inspect socket {}", path.display()));
        }
    }
    let listener =
        UnixListener::bind(path).with_context(|| format!("bind socket {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restrict socket permissions {}", path.display()))?;
    Ok(listener)
}

fn remove_stale_socket(path: &Path) -> Result<()> {
    match StdUnixStream::connect(path) {
        Ok(_) => bail!(
            "refusing to replace a live Forge socket: {}",
            path.display()
        ),
        Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
            fs::remove_file(path)
                .with_context(|| format!("remove stale socket {}", path.display()))?;
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "cannot determine whether existing Forge socket is live: {}",
                    path.display()
                )
            });
        }
    }
    Ok(())
}

fn prepare_socket_parent(parent: &Path, forge_owned: bool) -> Result<()> {
    if forge_owned {
        fs::create_dir_all(parent)
            .with_context(|| format!("create Forge socket directory {}", parent.display()))?;
    }
    let metadata = fs::symlink_metadata(parent)
        .with_context(|| format!("inspect socket directory {}", parent.display()))?;
    if !metadata.file_type().is_dir() {
        bail!(
            "socket parent must be a real directory: {}",
            parent.display()
        );
    }
    if metadata.uid() != Uid::effective().as_raw() {
        bail!(
            "socket parent must be owned by the current user: {}",
            parent.display()
        );
    }
    if forge_owned {
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("restrict Forge socket directory {}", parent.display()))?;
    } else if metadata.mode() & 0o077 != 0 {
        bail!(
            "explicit socket parent must already be owner-only: {}",
            parent.display()
        );
    }
    Ok(())
}

async fn wait_for_interrupt(sender: watch::Sender<bool>) {
    if tokio::signal::ctrl_c().await.is_ok() {
        let _ = sender.send(true);
    }
}

async fn wait_for_shutdown(mut receiver: watch::Receiver<bool>) {
    if !*receiver.borrow() {
        let _ = receiver.changed().await;
    }
}

#[cfg(test)]
#[path = "support/tests.rs"]
mod tests;
