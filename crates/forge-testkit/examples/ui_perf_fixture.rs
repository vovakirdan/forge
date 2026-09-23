//! Isolated keyless Core fixture for bounded browser pagination measurements.
use std::io::{BufRead, Write};

use anyhow::{Context, Result, ensure};
use forge_testkit::m0::{LocalHttpApi, M0Harness, single_stage_pipeline};
use serde_json::json;

const TASKS: usize = 1000;
const EMPLOYEES: usize = 20;

#[tokio::main]
async fn main() -> Result<()> {
    let harness = M0Harness::start().await?;
    let project_id = harness
        .create_project("UI4 keyless pagination acceptance")
        .await?;
    let version = harness
        .create_pipeline(project_id, single_stage_pipeline())
        .await?;
    for index in 0..EMPLOYEES {
        harness
            .create_employee(project_id, &format!("UI4 Employee {index:02}"))
            .await?;
    }
    for index in 0..TASKS {
        harness
            .create_task(project_id, version, &format!("UI4 Task {index:04}"))
            .await?;
    }
    ensure!(
        harness.store.list_tasks(project_id).await?.len() == TASKS,
        "Task seed count changed"
    );
    ensure!(
        harness.store.list_employees(project_id).await?.len() == EMPLOYEES,
        "Employee seed count changed"
    );
    let api = LocalHttpApi::start(harness.core.clone())?;
    let project = harness
        .store
        .load_project(project_id)
        .await?
        .context("Project absent")?;
    println!(
        "{}",
        json!({
            "core_socket":api.socket(), "project_id":project_id,
            "project_name":project.name(), "tasks":TASKS, "employees":EMPLOYEES
        })
    );
    std::io::stdout().flush()?;
    let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let (input_finished, input) = tokio::sync::oneshot::channel();
    let _input_thread = std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        let _ = input_finished.send(());
    });
    tokio::select! {
        _ = input => {},
        _ = signal.recv() => {},
        _ = tokio::signal::ctrl_c() => {},
    }
    api.shutdown().await;
    harness.shutdown().await;
    Ok(())
}
