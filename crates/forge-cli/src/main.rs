//! Human-facing local CLI; all mutations remain named HTTP commands.

mod demo;

use std::{path::PathBuf, process::ExitCode};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use forge_cli::LocalClient;
use forge_protocol::wire::CommandRequest;
use serde_json::{Map, Value};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "forge", version, about = "Forge local control client")]
struct Arguments {
    /// Local Core HTTP Unix-domain socket.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Submit an audited named command through Core.
    #[command(name = "command")]
    Execute {
        /// Name from the versioned command vocabulary, such as create_task.
        name: String,
        /// Owning Project UUIDv7.
        #[arg(long)]
        project_id: String,
        /// Current Project revision expected by this command.
        #[arg(long)]
        expected_revision: u64,
        /// JSON object containing only the named command's payload.
        #[arg(long)]
        payload: String,
        /// Caller retry identity; generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Read one public JSON path, for example /v1/projects/<id>/tasks.
    Get {
        /// A public /v1/... read path.
        path: String,
    },
    /// Replay currently available canonical events for one Project.
    Watch {
        /// Owning Project UUIDv7.
        #[arg(long)]
        project_id: String,
        /// Replay strictly after this Project event sequence.
        #[arg(long)]
        after: Option<u64>,
    },
    /// Execute the deterministic M0 public-API smoke scenario.
    Demo {
        #[command(subcommand)]
        scenario: DemoScenario,
    },
}

#[derive(Debug, Subcommand)]
enum DemoScenario {
    /// Create, dispatch, and verify one completed Task with evidence.
    M0,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Arguments::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("forge: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(arguments: Arguments) -> Result<()> {
    let Arguments { socket, command } = arguments;
    let client = match socket {
        Some(socket) => LocalClient::new(socket),
        None => LocalClient::from_environment(),
    };
    match command {
        Command::Execute {
            name,
            project_id,
            expected_revision,
            payload,
            idempotency_key,
        } => {
            let payload = parse_payload(&payload)?;
            let request = CommandRequest {
                project_id,
                expected_revision,
                payload,
            };
            let key = idempotency_key.unwrap_or_else(|| Uuid::now_v7().to_string());
            let receipt = client.execute_command(&name, &request, &key).await?;
            print_json(&receipt)
        }
        Command::Get { path } => print_json(&client.get_json(&path).await?),
        Command::Watch { project_id, after } => {
            for event in client.watch_events(&project_id, after).await? {
                print_json(&event)?;
            }
            Ok(())
        }
        Command::Demo {
            scenario: DemoScenario::M0,
        } => print_json(&demo::run_m0(&client).await?),
    }
}

fn parse_payload(text: &str) -> Result<Map<String, Value>> {
    let value: Value = serde_json::from_str(text).context("payload must be valid JSON")?;
    value
        .as_object()
        .cloned()
        .context("payload must be a JSON object")
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_payload;

    #[test]
    fn command_payload_requires_a_json_object() {
        let result = parse_payload("[]");

        assert!(result.is_err());
    }
}
