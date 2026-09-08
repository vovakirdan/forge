//! Child environment and process-group cleanup are independent of provider shells.

use std::{path::Path, process::Stdio};

use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

use super::{GitBackend, GitBackendError};

const OUTPUT_LIMIT: usize = 1024 * 1024;

pub(super) struct GitOutput {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
}

impl GitOutput {
    pub fn require_success(self, operation: &'static str) -> Result<Vec<u8>, GitBackendError> {
        if self.code == Some(0) {
            Ok(self.stdout)
        } else {
            Err(GitBackendError::Command {
                operation,
                exit_code: self.code,
            })
        }
    }
}

struct ProcessGroup(Option<Pid>);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            let _ = killpg(pid, Signal::SIGKILL);
        }
    }
}

impl GitBackend {
    pub(super) async fn command(
        &self,
        directory: &Path,
        args: &[&str],
        input: &[u8],
        environment: &[(&str, &str)],
    ) -> Result<GitOutput, GitBackendError> {
        let mut command = Command::new("/usr/bin/git");
        command
            .current_dir(directory)
            .env_clear()
            .envs([
                ("PATH", "/usr/bin:/bin"),
                ("LC_ALL", "C"),
                ("GIT_CONFIG_NOSYSTEM", "1"),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_CONFIG_COUNT", "0"),
                ("GIT_TERMINAL_PROMPT", "0"),
                ("GIT_NO_REPLACE_OBJECTS", "1"),
                ("GIT_GRAFT_FILE", "/dev/null"),
                ("GIT_OPTIONAL_LOCKS", "0"),
            ])
            .envs(environment.iter().copied())
            .args([
                "--no-pager",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.untrackedCache=false",
                "-c",
                "maintenance.auto=false",
                "-c",
                "gc.auto=0",
                "-c",
                "core.attributesFile=/dev/null",
                "-c",
                "init.templateDir=/dev/null",
                "-c",
                "protocol.allow=never",
                "-c",
                "protocol.file.allow=always",
            ])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let pid = child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .map(Pid::from_raw)
            .ok_or_else(|| std::io::Error::other("missing Git process identity"))?;
        let mut group = ProcessGroup(Some(pid));
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("missing Git stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("missing Git stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("missing Git stderr"))?;
        let operation = async {
            let write = async {
                stdin.write_all(input).await?;
                stdin.shutdown().await?;
                drop(stdin);
                Ok::<_, GitBackendError>(())
            };
            let status = async { Ok::<_, GitBackendError>(child.wait().await?) };
            let ((), stdout, _, status) =
                tokio::try_join!(write, bounded(stdout), bounded(stderr), status)?;
            Ok::<_, GitBackendError>(GitOutput {
                code: status.code(),
                stdout,
            })
        };
        let result = tokio::time::timeout(self.timeout, operation).await;
        match result {
            Ok(Ok(output)) => {
                group.0 = None;
                Ok(output)
            }
            Ok(Err(error)) => {
                drop(group);
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
                Err(error)
            }
            Err(_) => {
                drop(group);
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
                Err(GitBackendError::Timeout)
            }
        }
    }

    pub(super) async fn text(
        &self,
        directory: &Path,
        args: &[&str],
        operation: &'static str,
    ) -> Result<String, GitBackendError> {
        let bytes = self
            .command(directory, args, &[], &[])
            .await?
            .require_success(operation)?;
        String::from_utf8(bytes).map_err(|_| GitBackendError::Command {
            operation,
            exit_code: Some(0),
        })
    }
}

async fn bounded(mut reader: impl AsyncRead + Unpin) -> Result<Vec<u8>, GitBackendError> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok(output);
        }
        if output.len() + count > OUTPUT_LIMIT {
            return Err(GitBackendError::OutputLimit);
        }
        output.extend_from_slice(&buffer[..count]);
    }
}
