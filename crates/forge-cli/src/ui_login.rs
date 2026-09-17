//! Interactive owner bootstrap; secret output is never redirected into daemon logs.

use std::{
    fs::OpenOptions,
    io::{IsTerminal, Write},
    path::Path,
};

use nix::{
    sys::{stat::fstat, termios::tcgetsid},
    unistd::{getsid, getuid},
};
use thiserror::Error;

use forge_protocol::ui_control::{ControlWireError, request_login_code};

/// Safe errors never include the received code or raw response.
#[derive(Debug, Error)]
pub enum LoginError {
    #[error(
        "UI login requires owner stdin/stdout on the controlling terminal; redirection is not allowed"
    )]
    TerminalRequired,
    #[error("could not write UI login code to the owner terminal")]
    TerminalWrite,
    #[error(transparent)]
    Control(#[from] ControlWireError),
}

fn owner_terminal() -> Result<std::fs::File, LoginError> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    if !stdin.is_terminal() || !stdout.is_terminal() {
        return Err(LoginError::TerminalRequired);
    }
    let session = getsid(None).map_err(|_| LoginError::TerminalRequired)?;
    let input = fstat(&stdin).map_err(|_| LoginError::TerminalRequired)?;
    let output = fstat(&stdout).map_err(|_| LoginError::TerminalRequired)?;
    if input.st_uid != getuid().as_raw()
        || output.st_uid != getuid().as_raw()
        || (input.st_dev, input.st_ino) != (output.st_dev, output.st_ino)
        || tcgetsid(&stdin).map_err(|_| LoginError::TerminalRequired)? != session
        || tcgetsid(&stdout).map_err(|_| LoginError::TerminalRequired)? != session
    {
        return Err(LoginError::TerminalRequired);
    }
    let terminal = OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .map_err(|_| LoginError::TerminalRequired)?;
    if !terminal.is_terminal()
        || tcgetsid(&terminal).map_err(|_| LoginError::TerminalRequired)? != session
    {
        return Err(LoginError::TerminalRequired);
    }
    Ok(terminal)
}

/// Refuse non-interactive callers before connecting or rotating the pending code.
pub async fn login(path: &Path) -> Result<(), LoginError> {
    let mut terminal = owner_terminal()?;
    let response = request_login_code(path).await?;
    writeln!(
        terminal,
        "Origin: {}\nCode: {}\nExpires: {}",
        response.origin, response.code, response.expires_at
    )
    .map_err(|_| LoginError::TerminalWrite)?;
    terminal.flush().map_err(|_| LoginError::TerminalWrite)
}
