//! Container-only app-server driver. One fresh thread, sequential native turns.
use crate::app_server::{self, AppServerTurn};
use forge_protocol::{runtime::RunnerInvocation, supervisor::v1::deliver_runtime_input::Action};
use forge_provider_common::{
    PrivateMaterialization, SecretBytes,
    adapter::RuntimeObservation,
    driver_event::write_driver_event,
    native_event::{NativeDriverEvent, UsageBasis},
    native_input::{NativeMailbox, addressed_prompt, managed_mailbox},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    path::Path,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::Command,
    sync::watch,
};
use uuid::Uuid;
use zeroize::Zeroizing;

pub async fn run() -> io::Result<()> {
    let bytes = PrivateMaterialization::open(Path::new("/run/forge-input/invocation.json"))
        .and_then(|file| file.read(1024 * 1024))
        .map_err(|_| invalid())?;
    let invocation: RunnerInvocation =
        serde_json::from_slice(bytes.expose()).map_err(|_| invalid())?;
    if invocation.adapter_id != "codex_cli"
        || invocation.program != "forge-codex-driver"
        || invocation.args.first().map(String::as_str) != Some("app-server")
        || invocation.env.get("HOME").map(String::as_str) != Some("/run/forge/home")
        || invocation.env.get("CODEX_HOME").map(String::as_str) != Some(crate::CODEX_HOME)
    {
        return Err(invalid());
    }
    let scope = invocation.runtime_input.clone().ok_or_else(invalid)?;
    let mut mailbox = managed_mailbox(scope)?;
    let model = invocation
        .env
        .get("FORGE_CODEX_MODEL")
        .ok_or_else(invalid)?;
    let cwd = invocation
        .env
        .get("FORGE_CODEX_WORKDIR")
        .ok_or_else(invalid)?;
    let mut input = Zeroizing::new(Vec::new());
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut input)?;
    if input.is_empty() || input.len() > 1024 * 1024 {
        return Err(invalid());
    }
    let prompt = SecretBytes::new(std::mem::take(&mut input));
    let mut child = Command::new("/usr/local/bin/codex")
        .args(&invocation.args)
        .env_clear()
        .envs(&invocation.env)
        .current_dir(cwd)
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or_else(invalid)?;
    let mut stdout = BufReader::new(child.stdout.take().ok_or_else(invalid)?);
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let signals = tokio::spawn(async move {
        tokio::select! { _=interrupt.recv()=>{}, _=terminate.recv()=>{} }
        let _ = stop_tx.send(true);
    });
    let result = session(
        &mut stdin,
        &mut stdout,
        &mut mailbox,
        model,
        cwd,
        prompt,
        stop_rx,
        emit,
    )
    .await;
    signals.abort();
    // Only OCI teardown proves every tool descendant is physically gone.
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}

#[allow(clippy::too_many_arguments)]
pub async fn session<W, R, F>(
    writer: &mut W,
    reader: &mut R,
    mailbox: &mut NativeMailbox,
    model: &str,
    cwd: &str,
    prompt: SecretBytes,
    mut stop: watch::Receiver<bool>,
    mut emit: F,
) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
    F: FnMut(SecretBytes) -> io::Result<()>,
{
    let run = Uuid::parse_str(&mailbox.scope().run_id).map_err(|_| invalid())?;
    let mut rpc = Rpc {
        writer,
        reader,
        deferred: VecDeque::new(),
        partial: Zeroizing::new(Vec::new()),
    };
    rpc.send(&app_server::initialize()).await?;
    let initialized = rpc.response("forge-initialize", &mut stop).await?;
    if initialized.get("result").is_none() || initialized.get("error").is_some() {
        return Err(invalid());
    }
    rpc.send(&json!({"method":"initialized"})).await?;
    rpc.send(&app_server::start_thread(model, cwd)).await?;
    let thread =
        app_server::validate_thread(&rpc.response("forge-thread", &mut stop).await?, model, cwd)
            .map_err(|_| invalid())?;
    emit(write_driver_event(&RuntimeObservation::ThreadStarted).map_err(|_| invalid())?)?;
    let mut next = Some((run, prompt, None));
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        let (input, prompt, command) = if let Some(next) = next.take() {
            next
        } else {
            tokio::select! {
                biased;
                _=stop.changed()=>return Err(invalid()),
                frame=rpc.read()=>{
                    // No active turn: unsolicited activity cannot authorize a new input.
                    let frame=frame?;
                    if frame.get("method").and_then(Value::as_str).is_none_or(|method|!matches!(method,"thread/status/changed"|"account/rateLimits/updated")) { return Err(invalid()); }
                    continue;
                }
                _=tick.tick()=>{
                    let Some(command)=mailbox.next()? else {continue;};
                    if matches!(command.action,Some(Action::CloseAfterTurn(_))) {
                        rpc.writer.shutdown().await?;
                        mailbox.closed(&command)?;
                        return Ok(());
                    }
                    let id=Uuid::parse_str(&command.command_id).map_err(|_| invalid())?;
                    let prompt=addressed_prompt(&command)?;
                    mailbox.mark_sent(&command)?;
                    (id,prompt,Some(command))
                }
            }
        };
        if *stop.borrow() {
            return Err(invalid());
        }
        let bytes = app_server::start_turn(input, &thread, &prompt).map_err(|_| invalid())?;
        rpc.bytes(&bytes).await?;
        let response = rpc.response(&input.to_string(), &mut stop).await?;
        if let Some(kind) = app_server::request_failure(&response) {
            emit(
                write_driver_event(&RuntimeObservation::Failure { kind }).map_err(|_| invalid())?,
            )?;
            return Err(invalid());
        }
        let turn = app_server::accepted_turn(&response, input).map_err(|_| invalid())?;
        if let Some(command) = &command {
            mailbox.accepted(command)?;
        }
        emit(
            NativeDriverEvent::InputAccepted {
                run_id: run,
                input_id: input,
                session_id: thread.clone(),
                turn_id: turn.clone(),
            }
            .encode()
            .map_err(|_| invalid())?,
        )?;
        emit(write_driver_event(&RuntimeObservation::TurnStarted).map_err(|_| invalid())?)?;
        let mut interpreter = AppServerTurn::new(thread.clone(), turn.clone());
        loop {
            let frame = if let Some(frame) = rpc.deferred.pop_front() {
                frame
            } else {
                tokio::select! {
                    biased;
                    _=stop.changed()=>{
                        rpc.send(&json!({"id":"forge-interrupt","method":"turn/interrupt","params":{"threadId":thread,"turnId":turn}})).await?;
                        return Err(invalid());
                    }
                    frame=rpc.read()=>frame?,
                }
            };
            let event = interpreter.ingest(&frame).map_err(|_| invalid())?;
            for observation in event.observations {
                emit(write_driver_event(&observation).map_err(|_| invalid())?)?;
            }
            if let Some(completed) = event.finished {
                emit(
                    NativeDriverEvent::TurnFinished {
                        run_id: run,
                        input_id: input,
                        session_id: thread.clone(),
                        turn_id: turn.clone(),
                        completed,
                        usage: interpreter.usage(),
                        usage_basis: UsageBasis::Cumulative,
                    }
                    .encode()
                    .map_err(|_| invalid())?,
                )?;
                if !completed {
                    return Err(invalid());
                }
                break;
            }
        }
    }
}

struct Rpc<'a, W, R> {
    writer: &'a mut W,
    reader: &'a mut R,
    deferred: VecDeque<Value>,
    partial: Zeroizing<Vec<u8>>,
}
impl<W: AsyncWrite + Unpin, R: AsyncBufRead + Unpin> Rpc<'_, W, R> {
    async fn bytes(&mut self, bytes: &SecretBytes) -> io::Result<()> {
        self.writer.write_all(bytes.expose()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await
    }
    async fn send(&mut self, value: &Value) -> io::Result<()> {
        self.bytes(&SecretBytes::new(
            serde_json::to_vec(value).map_err(|_| invalid())?,
        ))
        .await
    }
    async fn read(&mut self) -> io::Result<Value> {
        let line = read_line(self.reader, &mut self.partial).await?;
        serde_json::from_slice(line.expose()).map_err(|_| invalid())
    }
    async fn response(&mut self, id: &str, stop: &mut watch::Receiver<bool>) -> io::Result<Value> {
        loop {
            if *stop.borrow() {
                return Err(invalid());
            }
            let frame = tokio::select! { _=stop.changed()=>return Err(invalid()), frame=self.read()=>frame? };
            if frame.get("id").and_then(Value::as_str) == Some(id) {
                return Ok(frame);
            }
            if frame.get("id").is_some()
                || frame.get("method").is_none()
                || self.deferred.len() >= 128
            {
                return Err(invalid());
            }
            self.deferred.push_back(frame);
        }
    }
}

/// Bound before allocation; an unterminated provider line cannot exhaust memory.
async fn read_line(
    reader: &mut (impl AsyncBufRead + Unpin),
    line: &mut Zeroizing<Vec<u8>>,
) -> io::Result<SecretBytes> {
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Err(invalid());
        }
        let end = available.iter().position(|byte| *byte == b'\n');
        let count = end.map_or(available.len(), |i| i + 1);
        if line.len() + count > 1024 * 1024 {
            return Err(invalid());
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(SecretBytes::new(std::mem::take(line)));
        }
    }
}
fn emit(bytes: SecretBytes) -> io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(bytes.expose())?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}
fn invalid() -> io::Error {
    io::Error::other("Codex native runtime contract failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn partial_json_line_survives_idle_tick_cancellation() {
        let (mut writer, reader) = tokio::io::duplex(128);
        let mut reader = BufReader::new(reader);
        let mut pending = Zeroizing::new(Vec::new());
        writer.write_all(b"{\"method\":").await.unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                read_line(&mut reader, &mut pending)
            )
            .await
            .is_err()
        );
        assert!(!pending.is_empty());
        writer.write_all(b"\"heartbeat\"}\n").await.unwrap();
        let bytes = read_line(&mut reader, &mut pending).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(bytes.expose()).unwrap()["method"],
            "heartbeat"
        );
        assert!(pending.is_empty());
    }
}
