//! Keyless browser acceptance fixture: real Core, private schema, no Employees or Runs.

use std::io::{BufRead, Write};

use anyhow::{Context, Result};
use forge_testkit::m0::{LocalHttpApi, M0Harness};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    let harness = M0Harness::start_configured(Ok).await?;
    let name = "Forge live <img src=x onerror=window.forgeInjected=true>";
    let project_id = harness.create_project(name).await?;
    harness.start_project(project_id).await?;
    let project = harness
        .store
        .load_project(project_id)
        .await?
        .context("fixture Project missing after named command")?;
    let api = LocalHttpApi::start(harness.core.clone())?;
    // Only non-secret fixture coordinates cross stdout. The browser gateway
    // process receives these paths, never the database/NATS environment.
    println!(
        "{}",
        json!({
            "core_socket": api.socket(),
            "project_id": project_id,
            "project_name": name,
            "revision": project.revision(),
            "execution_gate": "open"
        })
    );
    std::io::stdout().flush()?;
    let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let (input_finished, input) = tokio::sync::oneshot::channel();
    // A detached OS thread cannot keep Tokio shutdown waiting on a blocked
    // stdin read after SIGTERM. EOF remains the normal fixture shutdown path.
    let _input_thread = std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        let _ = input_finished.send(());
    });
    tokio::select! {
        _ = input => {}
        _ = signal.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    api.shutdown().await;
    harness.shutdown().await;
    Ok(())
}
