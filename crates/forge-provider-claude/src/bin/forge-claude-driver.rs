//! Container-local credential injector. The Supervisor controls stdin/stdout and
//! SIGINT/force-stop; replacing this process keeps signal ownership unambiguous.
use std::{
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, ExitCode},
};

use forge_provider_claude::{CLAUDE_CONFIG_DIR, validate_setup_token};
use forge_provider_common::PrivateMaterialization;

fn main() -> ExitCode {
    if launch().is_err() {
        // Never print subprocess arguments, the environment or secret I/O errors.
        eprintln!("Claude managed runtime initialization failed");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn launch() -> Result<(), ()> {
    let token = PrivateMaterialization::read_beneath(
        Path::new(CLAUDE_CONFIG_DIR),
        Path::new("setup-token"),
        16 * 1024,
    )
    .map_err(|_| ())?;
    let token_text = validate_setup_token(&token).map_err(|_| ())?;
    // PreparedInvocation.env is an allowlist installed with env_clear by the
    // Supervisor. The sole added value exists only in this sandboxed process.
    let _error = Command::new("/usr/local/bin/claude")
        .args(std::env::args_os().skip(1))
        .env("CLAUDE_CODE_OAUTH_TOKEN", token_text)
        .exec();
    Err(())
}
