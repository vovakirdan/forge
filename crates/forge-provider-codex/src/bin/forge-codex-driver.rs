#[tokio::main]
async fn main() -> std::process::ExitCode {
    match forge_provider_codex::driver::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("Codex native runtime contract or transport failed");
            std::process::ExitCode::FAILURE
        }
    }
}
