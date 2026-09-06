use forge_provider_opencode::{SessionResult, driver};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match driver::run().await {
        Ok(SessionResult::TurnEnded) => std::process::ExitCode::SUCCESS,
        Ok(SessionResult::Aborted | SessionResult::ProviderFailed) => {
            std::process::ExitCode::from(2)
        }
        Err(error) => {
            // Error variants contain only static classifications/status codes.
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
