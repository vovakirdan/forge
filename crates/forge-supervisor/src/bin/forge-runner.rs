//! Container entrypoint; deliberately no host-mode fallback.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match forge_supervisor::runner::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("Forge runner failed; inspect scoped runtime evidence");
            ExitCode::FAILURE
        }
    }
}
