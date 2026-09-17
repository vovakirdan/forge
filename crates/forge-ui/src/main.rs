use std::path::PathBuf;

use clap::{Parser, Subcommand};
use forge_ui::{Gateway, GatewayConfig};
use tokio::sync::watch;

#[derive(Parser)]
#[command(name = "forge-ui", about = "Local-owner Forge browser gateway")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long)]
        assets_dir: PathBuf,
        #[arg(long)]
        core_socket: Option<PathBuf>,
        #[arg(long)]
        control_socket: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Args {
        command:
            Command::Serve {
                assets_dir,
                core_socket,
                control_socket,
            },
    } = Args::parse();
    let runtime = forge_protocol::local_paths::default_runtime_directory();
    let gateway = Gateway::bind(GatewayConfig {
        assets_dir,
        core_socket: core_socket.unwrap_or_else(|| runtime.join("api.sock")),
        control_socket: control_socket.unwrap_or_else(|| runtime.join("ui-control.sock")),
    })
    .await?;
    println!("Origin: {}", gateway.origin());
    let (stop, shutdown) = watch::channel(false);
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let serve = gateway.serve(shutdown);
    tokio::pin!(serve);
    tokio::select! {
        result = &mut serve => result?,
        signal = tokio::signal::ctrl_c() => { signal?; let _ = stop.send(true); serve.await?; }
        _ = terminate.recv() => { let _ = stop.send(true); serve.await?; }
    }
    Ok(())
}
