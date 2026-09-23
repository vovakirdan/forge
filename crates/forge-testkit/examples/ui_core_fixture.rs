//! Keyless browser fixture: real Core and private schema, deterministic M0 execution only.

use std::io::{BufRead, Write};

use anyhow::{Context, Result};
use forge_testkit::m0::{LocalHttpApi, M0Harness};
use serde_json::json;

#[path = "ui_core_fixture/approval_commands.rs"]
mod approval_commands;
#[path = "ui_core_fixture/cancellation_commands.rs"]
mod cancellation_commands;
#[path = "ui_core_fixture/commands.rs"]
mod commands;
#[path = "ui_core_fixture/create_commands.rs"]
mod create_commands;
#[path = "ui_core_fixture/dependency_reads.rs"]
mod dependency_reads;
#[path = "ui_core_fixture/pipelines.rs"]
mod pipelines;
#[path = "ui_core_fixture/runs.rs"]
mod runs;
#[path = "ui_core_fixture/tasks.rs"]
mod tasks;
#[path = "ui_core_fixture/team.rs"]
mod team;

#[tokio::main]
async fn main() -> Result<()> {
    let mut harness = M0Harness::start().await?;
    let name = "Forge live <img src=x onerror=window.forgeInjected=true>";
    let project_id = harness.create_project(name).await?;
    let fixture = tasks::seed(&harness, project_id).await?;
    harness.attach_fake_supervisor().await?;
    let run_fixture = runs::seed(&harness).await?;
    let pipeline_fixture = pipelines::seed(&harness).await?;
    let command_fixture = commands::seed(&harness, "Draft command acceptance").await?;
    let priority_command_fixture = commands::seed(&harness, "Priority command acceptance").await?;
    let create_command_fixture = create_commands::seed(&harness).await?;
    let approval_command_fixture = approval_commands::seed(&harness).await?;
    let cancellation_command_fixture = cancellation_commands::seed(&harness).await?;
    let dependency_read_fixture = dependency_reads::seed(&harness).await?;
    let team_fixture = team::seed(&harness).await?;
    let existing_projects = harness.store.list_project_ids().await?.len();
    for index in existing_projects..25 {
        harness
            .create_project(&format!("Project selector page {index:02}"))
            .await?;
    }
    let later_id = harness.store.list_project_ids().await?[20];
    let later = harness
        .store
        .load_project(later_id)
        .await?
        .context("later Project missing")?;
    harness.start_project(project_id).await?;
    let project = harness
        .store
        .load_project(project_id)
        .await?
        .context("fixture Project missing after named command")?;
    let api = LocalHttpApi::start(harness.core.clone())?;
    // Only non-secret fixture coordinates cross stdout. The browser gateway
    // process receives these paths, never the database/NATS environment.
    let readiness = json!({
        "core_socket": api.socket(),
        "project_id": project_id,
        "project_name": name,
        "revision": project.revision(),
        "execution_gate": "open",
        "tasks": fixture.tasks,
        "second_project": fixture.second_project,
        "empty_project": fixture.empty_project,
        "runs_project": run_fixture.project,
        "other_runs_project": run_fixture.other_project,
        "pipelines_project": pipeline_fixture.project,
        "other_pipelines_project": pipeline_fixture.other_project,
        "commands_project": command_fixture,
        "priority_commands_project": priority_command_fixture,
        "create_commands_project": create_command_fixture,
        "approval_commands_project": approval_command_fixture,
        "cancellation_commands_project": cancellation_command_fixture,
        "dependency_reads_project": dependency_read_fixture
        ,"team_project": team_fixture
        ,"later_project": {"id": later.id(), "name": later.name()}
    })
    .to_string();
    // The browser's firstLine transport admits at most 8 KiB, including newline.
    anyhow::ensure!(
        readiness.len() < 8192,
        "browser fixture readiness exceeds 8 KiB"
    );
    println!("{readiness}");
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
