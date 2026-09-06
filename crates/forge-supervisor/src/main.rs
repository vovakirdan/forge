use std::{path::PathBuf, process::ExitCode, time::Duration};

use clap::Parser;
use forge_supervisor::{
    SupervisorConfig, current_boot_id, default_socket_path, default_state_directory, run,
};
use tokio::sync::watch;
use tracing::error;
use tracing_subscriber::EnvFilter;

/// Local deterministic M0 Supervisor for Forge Core's gRPC Unix socket.
#[derive(Debug, Parser)]
#[command(name = "forge-supervisor", version, about)]
struct Arguments {
    /// Read-only local metrics/health listener, isolated from Run networking.
    #[arg(long, default_value = "127.0.0.1:9879")]
    observability_address: std::net::SocketAddr,
    /// Core's Supervisor gRPC Unix-domain socket.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,

    /// Stable identity reported for this local execution host.
    #[arg(long, default_value = "local-m0")]
    host_id: String,

    /// Override the detected Linux host boot identity for recovery tests.
    #[arg(long)]
    boot_id: Option<String>,

    /// Delay before reconnecting when Core is not yet available.
    #[arg(long, default_value_t = 500, value_name = "MILLISECONDS")]
    reconnect_delay_ms: u64,

    /// Persistent owner-only directory for execution identity and replay state.
    #[arg(long, value_name = "PATH")]
    state_directory: Option<PathBuf>,
    /// Core-owned private runtime materializations.
    #[arg(long, value_name = "PATH")]
    grants_directory: Option<PathBuf>,
    /// Per-run scoped Gateway socket root.
    #[arg(long, value_name = "PATH")]
    gateways_directory: Option<PathBuf>,
    /// Retained raw evidence plus active output admission budget, without deletion.
    #[arg(long, default_value_t = 2 * 1024 * 1024 * 1024u64)]
    evidence_max_bytes: u64,
}

#[tokio::main]
async fn main() -> ExitCode {
    let arguments = Arguments::parse();
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init();

    let boot_id = match arguments.boot_id {
        Some(boot_id) if !boot_id.trim().is_empty() => boot_id,
        Some(_) => {
            error!("Supervisor boot identity override must not be blank");
            return ExitCode::FAILURE;
        }
        None => match current_boot_id() {
            Ok(boot_id) => boot_id,
            Err(error) => {
                error!(error = %error, "Supervisor could not determine host boot identity");
                return ExitCode::FAILURE;
            }
        },
    };

    let mut config = SupervisorConfig::new(
        arguments.socket.unwrap_or_else(default_socket_path),
        arguments.host_id,
        boot_id,
    );
    config.reconnect_delay = Duration::from_millis(arguments.reconnect_delay_ms);
    config.state_directory = arguments
        .state_directory
        .unwrap_or_else(default_state_directory);
    config.grants_directory = arguments
        .grants_directory
        .unwrap_or_else(|| config.state_directory.join("grants"));
    config.gateways_directory = arguments
        .gateways_directory
        .unwrap_or_else(|| config.state_directory.join("gateways"));
    config.evidence_max_bytes = arguments.evidence_max_bytes;
    config.observability_address = Some(arguments.observability_address);
    if let Some(parent) = config.state_directory.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        error!(error = %error, "Supervisor could not create its state parent directory");
        return ExitCode::FAILURE;
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let signal_task = tokio::spawn(wait_for_interrupt(shutdown_tx));
    let result = run(config, shutdown_rx).await;
    signal_task.abort();
    let _ = signal_task.await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(error = %error, "Supervisor terminated unexpectedly");
            ExitCode::FAILURE
        }
    }
}

async fn wait_for_interrupt(sender: watch::Sender<bool>) {
    if tokio::signal::ctrl_c().await.is_ok() {
        let _ = sender.send(true);
    }
}
