use std::{path::PathBuf, process::ExitCode, time::Duration};

use clap::Parser;
use forge_supervisor::{SupervisorConfig, current_boot_id, default_socket_path, run};
use tokio::sync::watch;
use tracing::error;
use tracing_subscriber::EnvFilter;

/// Local deterministic M0 Supervisor for Forge Core's gRPC Unix socket.
#[derive(Debug, Parser)]
#[command(name = "forge-supervisor", version, about)]
struct Arguments {
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
}

#[tokio::main]
async fn main() -> ExitCode {
    let arguments = Arguments::parse();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
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
