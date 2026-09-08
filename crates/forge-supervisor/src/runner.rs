//! Container-local wrapper. It has no host filesystem or canonical store access.

use forge_protocol::runtime::{RunnerExit, RunnerInvocation};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use std::{
    fs, io,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Component, Path},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    net::{TcpListener, UnixStream},
    process::Command,
    sync::{Notify, Semaphore},
    time::{Instant, sleep},
};

const PRIVATE: &str = "/run/forge";
const INPUT: &str = "/run/forge-input";
const EVIDENCE: &str = "/run/forge-evidence";
const MAX_LINE: usize = 1024 * 1024;
mod duplex;

/// Runs only inside the configured Podman image and writes bounded, redacted
/// evidence. A clean process exit never submits a Task/Pipeline outcome.
pub async fn run() -> io::Result<()> {
    if !Path::new("/run/.containerenv").is_file() {
        return Err(io::Error::other("runner requires a Podman environment"));
    }
    let invocation: RunnerInvocation =
        serde_json::from_slice(&fs::read(format!("{INPUT}/invocation.json"))?)
            .map_err(io::Error::other)?;
    validate(&invocation)?;
    let mut secrets = Vec::new();
    for managed in &invocation.managed_files {
        let path = private_path(&managed.relative_path, false)?;
        create_parent(&path)?;
        write_private(&path, managed.contents.as_bytes())?;
    }
    for credential in &invocation.credential_files {
        if !matches!(
            credential.source.as_str(),
            "/run/forge-secrets/auth.json"
                | "/run/forge-secrets/api-key"
                | "/run/forge-secrets/claude-setup-token"
        ) {
            return Err(io::Error::other("unsupported credential source"));
        }
        if credential.source == "/run/forge-secrets/claude-setup-token"
            && (invocation.adapter_id != "claude_code_cli"
                || credential.target != "/run/forge/claude-home/setup-token"
                || credential.writeback)
        {
            return Err(io::Error::other("invalid Claude credential delivery"));
        }
        let path = private_path(&credential.target, true)?;
        create_parent(&path)?;
        let bytes = read_private(Path::new(&credential.source))?;
        collect_secrets(&bytes, &mut secrets);
        write_private(&path, &bytes)?;
    }
    for name in ["home", "config", "cache", "data", "codex-home"] {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(Path::new(PRIVATE).join(name))?;
    }
    let redaction = RedactionPolicy {
        initial: secrets,
        codex_errors: invocation.adapter_id == "codex_cli",
        credential_paths: invocation
            .credential_files
            .iter()
            .map(|credential| std::path::PathBuf::from(&credential.target))
            .collect(),
    };
    let relay = if invocation.adapter_id == "project_hook" {
        None
    } else {
        let gateway = TcpListener::bind("127.0.0.1:4097").await?;
        let proxy = TcpListener::bind("127.0.0.1:4098").await?;
        Some(tokio::spawn(relay(gateway, proxy)))
    };
    let claude_input =
        invocation.runtime_input.is_some() && invocation.adapter_id == "claude_code_cli";
    let mut command = Command::new(&invocation.program);
    command
        .args(&invocation.args)
        .env_clear()
        .envs(&invocation.env)
        .process_group(0)
        .stdin(if claude_input {
            Stdio::piped()
        } else {
            Stdio::from(fs::File::open(format!("{INPUT}/stdin"))?)
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let process_group = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .ok_or_else(|| io::Error::other("invalid child identity"))?;
    let budget = Arc::new(OutputBudget {
        remaining: AtomicU64::new(invocation.max_output_bytes),
        incomplete: AtomicBool::new(false),
        failed: Notify::new(),
    });
    let (input_frames, input_receiver) = tokio::sync::mpsc::channel(8);
    let input_task = if let Some(config) = invocation.runtime_input.clone().filter(|_| claude_input)
    {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("native stdin unavailable"))?;
        Some(tokio::spawn(duplex::pump(
            stdin,
            config,
            duplex::Paths {
                initial: Path::new(INPUT).join("stdin"),
                mailbox: Path::new(INPUT).join("inputs"),
                private: Path::new(PRIVATE).into(),
                receipts: Path::new(EVIDENCE).join("input-receipts"),
            },
            input_receiver,
            Arc::clone(&budget),
        )))
    } else {
        None
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr unavailable"))?;
    let out = tokio::spawn(capture_with_input(
        stdout,
        format!("{EVIDENCE}/stdout.jsonl"),
        Arc::clone(&budget),
        redaction.clone(),
        claude_input.then_some(input_frames),
    ));
    let err = tokio::spawn(capture(
        stderr,
        format!("{EVIDENCE}/stderr.log"),
        Arc::clone(&budget),
        redaction,
    ));
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut deadline = None;
    let mut stop_requested = false;
    let status = loop {
        tokio::select! {
            status = child.wait() => break status?,
            _ = interrupt.recv() => { stop_requested = true; request_stop(process_group, &mut deadline, invocation.stop_grace_seconds); }
            _ = terminate.recv() => { stop_requested = true; request_stop(process_group, &mut deadline, invocation.stop_grace_seconds); }
            () = budget.failed.notified() => { stop_requested = true; request_stop(process_group, &mut deadline, invocation.stop_grace_seconds); }
            () = sleep(Duration::from_millis(100)), if deadline.is_some() => {
                if deadline.is_some_and(|at| Instant::now() >= at) { let _ = killpg(Pid::from_raw(process_group), Signal::SIGKILL); }
            }
        }
    };
    // Container teardown handles descendants; don't let orphaned pipe holders
    // prevent recording the direct process exit forever.
    let captures =
        tokio::time::timeout(Duration::from_secs(2), async { (out.await, err.await) }).await;
    if !matches!(captures, Ok((Ok(Ok(())), Ok(Ok(()))))) {
        budget.incomplete.store(true, Ordering::Release);
    }
    if let Some(input) = input_task {
        if !input.is_finished() {
            input.abort();
        }
        if !matches!(input.await, Ok(Ok(()))) {
            budget.incomplete.store(true, Ordering::Release);
        }
    }
    let exit = RunnerExit {
        exit_code: status.code(),
        output_incomplete: budget.incomplete.load(Ordering::Acquire),
        stop_requested,
    };
    write_private(
        Path::new(&format!("{EVIDENCE}/exit.json")),
        &serde_json::to_vec(&exit).map_err(io::Error::other)?,
    )?;
    if let Some(relay) = relay {
        relay.abort();
    }
    Ok(())
}

fn request_stop(group: i32, deadline: &mut Option<Instant>, grace: u32) {
    if deadline.is_none() {
        let _ = killpg(Pid::from_raw(group), Signal::SIGINT);
        *deadline = Some(Instant::now() + Duration::from_secs(u64::from(grace)));
    }
}

struct OutputBudget {
    remaining: AtomicU64,
    incomplete: AtomicBool,
    failed: Notify,
}

#[derive(Clone)]
struct RedactionPolicy {
    initial: Vec<Vec<u8>>,
    credential_paths: Vec<std::path::PathBuf>,
    codex_errors: bool,
}

impl RedactionPolicy {
    fn current_secrets(&self) -> io::Result<Vec<Vec<u8>>> {
        let mut secrets = self.initial.clone();
        // Credentials can rotate while the child runs. Re-read bounded private
        // snapshots before persisting a line, not only once during startup.
        for path in &self.credential_paths {
            collect_secrets(&read_private(path)?, &mut secrets);
        }
        secrets.sort_unstable_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.dedup();
        Ok(secrets)
    }
}

async fn capture(
    input: impl AsyncRead + Unpin,
    path: String,
    budget: Arc<OutputBudget>,
    redaction: RedactionPolicy,
) -> io::Result<()> {
    capture_with_input(input, path, budget, redaction, None).await
}

async fn capture_with_input(
    mut input: impl AsyncRead + Unpin,
    path: String,
    budget: Arc<OutputBudget>,
    redaction: RedactionPolicy,
    native: Option<tokio::sync::mpsc::Sender<duplex::Frame>>,
) -> io::Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .inspect_err(|_| mark_incomplete(&budget))?;
    let mut buffer = [0u8; 8192];
    let mut line = Vec::new();
    let mut discard = false;
    loop {
        let size = input
            .read(&mut buffer)
            .await
            .inspect_err(|_| mark_incomplete(&budget))?;
        if size == 0 {
            break;
        }
        for &byte in &buffer[..size] {
            if !discard {
                line.push(byte);
            }
            if line.len() > MAX_LINE {
                discard = true;
                line.clear();
                mark_incomplete(&budget);
            }
            if byte == b'\n' {
                if !discard {
                    if let Some(native) = &native {
                        duplex::forward(&line, native)
                            .await
                            .inspect_err(|_| mark_incomplete(&budget))?;
                    }
                    write_line(&mut file, &line, &budget, &redaction)?;
                }
                line.clear();
                discard = false;
            }
        }
    }
    if !discard && !line.is_empty() {
        write_line(&mut file, &line, &budget, &redaction)?;
    }
    file.flush().inspect_err(|_| mark_incomplete(&budget))?;
    file.sync_all().inspect_err(|_| mark_incomplete(&budget))
}

fn write_line(
    file: &mut fs::File,
    line: &[u8],
    budget: &OutputBudget,
    redaction: &RedactionPolicy,
) -> io::Result<()> {
    use std::io::Write;
    // Keep the adapter's typed failure while discarding its sensitive message.
    // Codes such as refresh_token_reused otherwise look like credential fields.
    let failure = redaction
        .codex_errors
        .then(|| safe_codex_failure(line))
        .flatten();
    // Credential-shaped records are not useful raw evidence. This also covers
    // nested/escaped auth JSON and a fresh token printed before atomic writeback.
    if failure.is_none()
        && (contains_credential_field(line)
            || serde_json::from_slice(line)
                .ok()
                .is_some_and(|value| contains_reasoning(&value)))
    {
        return Ok(());
    }
    let secrets = match redaction.current_secrets() {
        Ok(secrets) => secrets,
        Err(_) => {
            mark_incomplete(budget);
            return Ok(());
        }
    };
    let line = redact(failure.unwrap_or(line), &secrets);
    if budget
        .remaining
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
            remaining.checked_sub(line.len() as u64)
        })
        .is_err()
    {
        mark_incomplete(budget);
        return Ok(());
    }
    if let Err(error) = file.write_all(&line) {
        mark_incomplete(budget);
        return Err(error);
    }
    Ok(())
}

fn safe_codex_failure(line: &[u8]) -> Option<&'static [u8]> {
    use forge_provider_common::adapter::{RuntimeFailureKind, RuntimeObservation};
    let line = std::str::from_utf8(line).ok()?;
    let RuntimeObservation::Failure { kind } =
        forge_provider_codex::parse_jsonl_event(line).ok()?
    else {
        return None;
    };
    Some(match kind {
        RuntimeFailureKind::AuthRefreshReused => {
            b"{\"type\":\"error\",\"message\":\"refresh_token_reused\"}\n"
        }
        RuntimeFailureKind::AuthExpired => {
            b"{\"type\":\"error\",\"message\":\"refresh_token_expired\"}\n"
        }
        RuntimeFailureKind::AuthInvalidated => {
            b"{\"type\":\"error\",\"message\":\"refresh_token_invalidated\"}\n"
        }
        RuntimeFailureKind::AuthRequired => {
            b"{\"type\":\"error\",\"message\":\"authentication required\"}\n"
        }
        RuntimeFailureKind::RateLimited => b"{\"type\":\"error\",\"message\":\"rate limit\"}\n",
        RuntimeFailureKind::ProviderUnavailable => {
            b"{\"type\":\"error\",\"message\":\"temporarily unavailable\"}\n"
        }
        RuntimeFailureKind::RuntimeError => b"{\"type\":\"error\",\"message\":\"runtime error\"}\n",
    })
}

fn contains_credential_field(line: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(line).to_ascii_lowercase();
    ["refresh_token", "access_token", "id_token", "api_key"]
        .iter()
        .any(|field| lower.contains(field))
}

fn mark_incomplete(budget: &OutputBudget) {
    budget.incomplete.store(true, Ordering::Release);
    budget.failed.notify_one();
}

fn contains_reasoning(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => object.iter().any(|(key, value)| {
            matches!(key.as_str(), "reasoning" | "thinking")
                || (key == "type"
                    && value.as_str().is_some_and(|kind| {
                        kind.contains("reasoning") || kind.contains("thinking")
                    }))
                || contains_reasoning(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(contains_reasoning),
        _ => false,
    }
}

fn redact(line: &[u8], secrets: &[Vec<u8>]) -> Vec<u8> {
    let mut output = Vec::with_capacity(line.len());
    let mut offset = 0;
    while offset < line.len() {
        if let Some(secret) = secrets
            .iter()
            .find(|secret| !secret.is_empty() && line[offset..].starts_with(secret))
        {
            output.extend_from_slice(b"[REDACTED]");
            offset += secret.len();
        } else if line[offset..].starts_with(b"sk-") || line[offset..].starts_with(b"eyJ") {
            let length = line[offset..]
                .iter()
                .take_while(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                })
                .count();
            if length >= 16 {
                output.extend_from_slice(b"[REDACTED]");
                offset += length;
            } else {
                output.push(line[offset]);
                offset += 1;
            }
        } else {
            output.push(line[offset]);
            offset += 1;
        }
    }
    output
}

fn collect_secrets(bytes: &[u8], secrets: &mut Vec<Vec<u8>>) {
    fn visit(value: &serde_json::Value, secrets: &mut Vec<Vec<u8>>) {
        match value {
            serde_json::Value::Object(object) => {
                for (key, value) in object {
                    if (key.contains("token") || key.to_ascii_lowercase().contains("api_key"))
                        && let Some(value) = value.as_str()
                        && !value.is_empty()
                    {
                        secrets.push(value.as_bytes().to_vec());
                    } else {
                        visit(value, secrets);
                    }
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    visit(value, secrets);
                }
            }
            _ => {}
        }
    }
    if let Ok(value) = serde_json::from_slice(bytes) {
        visit(&value, secrets);
    } else if !bytes.is_empty() {
        secrets.push(bytes.to_vec());
    }
}

async fn relay(gateway: TcpListener, proxy: TcpListener) -> io::Result<()> {
    let capacity = Arc::new(Semaphore::new(32));
    loop {
        let (mut client, _) = tokio::select! { connection = gateway.accept() => connection?, connection = proxy.accept() => connection? };
        let Ok(permit) = Arc::clone(&capacity).try_acquire_owned() else {
            continue;
        };
        tokio::spawn(async move {
            let _permit = permit;
            if let Ok(mut host) = UnixStream::connect("/run/forge-gateway/gateway.sock").await {
                let _ = tokio::io::copy_bidirectional(&mut client, &mut host).await;
            }
        });
    }
}

fn validate(invocation: &RunnerInvocation) -> io::Result<()> {
    if invocation.adapter_id == "project_hook"
        && (!invocation.managed_files.is_empty()
            || !invocation.credential_files.is_empty()
            || invocation.runtime_input.is_some())
    {
        return Err(io::Error::other(
            "project hooks cannot receive provider material",
        ));
    }
    if let Some(input) = &invocation.runtime_input
        && (!matches!(
            invocation.adapter_id.as_str(),
            "claude_code_cli" | "codex_cli" | "opencode_runtime"
        ) || uuid::Uuid::parse_str(&input.run_id)
            .ok()
            .is_none_or(|id| id.get_version_num() != 7)
            || input.fencing_token == 0
            || input.environment_epoch == 0)
    {
        return Err(io::Error::other("invalid native input scope"));
    }
    if invocation.runtime_input.is_some()
        && !matches!(
            (invocation.adapter_id.as_str(), invocation.program.as_str()),
            ("claude_code_cli", "forge-claude-driver")
                | ("codex_cli", "forge-codex-driver")
                | ("opencode_runtime", "forge-opencode-driver")
        )
    {
        return Err(io::Error::other("invalid native driver"));
    }
    if invocation.program.is_empty()
        || invocation.max_output_bytes == 0
        || invocation.max_output_bytes > 256 * 1024 * 1024
        || invocation.stop_grace_seconds == 0
    {
        return Err(io::Error::other("invalid runner contract"));
    }
    if invocation.adapter_id == "claude_code_cli"
        && (invocation.program != "forge-claude-driver"
            || !matches!(invocation.credential_files.as_slice(), [credential]
                if credential.source == "/run/forge-secrets/claude-setup-token"
                    && credential.target == "/run/forge/claude-home/setup-token"
                    && !credential.writeback))
    {
        return Err(io::Error::other("invalid Claude runner contract"));
    }
    Ok(())
}

fn private_path(value: &str, absolute: bool) -> io::Result<std::path::PathBuf> {
    let path = Path::new(value);
    let relative = if absolute {
        path.strip_prefix(PRIVATE).map_err(io::Error::other)?
    } else {
        path
    };
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(io::Error::other("invalid private runtime path"));
    }
    Ok(Path::new(PRIVATE).join(relative))
}

fn create_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("invalid private path"))?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
}

fn read_private(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.mode() & 0o077 != 0
        || metadata.len() > MAX_LINE as u64
        || metadata.nlink() != 1
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
    {
        return Err(io::Error::other("unsafe credential file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_LINE as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_LINE {
        return Err(io::Error::other("credential exceeds limit"));
    }
    Ok(bytes)
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests;
