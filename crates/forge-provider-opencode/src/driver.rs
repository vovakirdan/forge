//! Executed only as an OCI image helper by forge-runner. No Core API calls and
//! no host CLI spawn path exist in the adapter or Core preparation code.

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    os::unix::fs::DirBuilderExt,
    path::Path,
    process::Stdio,
    time::Duration,
};

use forge_provider_common::{PrivateMaterialization, SecretBytes, adapter::RuntimeObservation};
use tokio::{process::Command, sync::watch};
use zeroize::Zeroizing;

use crate::{
    OpenCodeClient, OpenCodeError, PINNED_OPENCODE_VERSION, SessionResult, VIRTUAL_KEY_PATH,
    prepare::validate_workdir, write_driver_event,
};

const SERVER_URL: &str = "http://127.0.0.1:4096";
const ENVIRONMENT_KEYS: [&str; 17] = [
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "TMPDIR",
    "LANG",
    "OPENCODE_DISABLE_PROJECT_CONFIG",
    "OPENCODE_DISABLE_MODELS_FETCH",
    "OPENCODE_DISABLE_AUTOUPDATE",
    "OPENCODE_DISABLE_AUTOCOMPACT",
    "OPENCODE_DISABLE_TERMINAL_TITLE",
    "OPENCODE_CONFIG_CONTENT",
    "NO_PROXY",
    "no_proxy",
    "NO_COLOR",
];

/// Entrypoint for the container-local binary. Parent Supervisor remains the
/// authority for deadlines and physical termination of the entire environment.
pub async fn run() -> Result<SessionResult, OpenCodeError> {
    let workdir = std::env::var("FORGE_OPENCODE_WORKDIR").map_err(|_| OpenCodeError::Process)?;
    let model = std::env::var("FORGE_OPENCODE_MODEL").map_err(|_| OpenCodeError::Process)?;
    validate_workdir(&workdir)?;
    let environment = managed_environment()?;
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(|_| OpenCodeError::Process)?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| OpenCodeError::Process)?;
    let signal = tokio::spawn(async move {
        tokio::select! {_=interrupt.recv()=>{},_=terminate.recv()=>{}}
        let _ = stop_tx.send(true);
    });
    let result = run_managed(&workdir, &model, &environment, stop_rx).await;
    signal.abort();
    result
}

async fn run_managed(
    workdir: &str,
    model: &str,
    environment: &BTreeMap<String, String>,
    stop: watch::Receiver<bool>,
) -> Result<SessionResult, OpenCodeError> {
    let mut version = Command::new("opencode");
    version
        .arg("--version")
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), version.output())
        .await
        .map_err(|_| OpenCodeError::Process)?
        .map_err(|_| OpenCodeError::Process)?;
    if !output.status.success()
        || output.stdout.len() > 1024
        || std::str::from_utf8(&output.stdout).ok().map(str::trim) != Some(PINNED_OPENCODE_VERSION)
    {
        return Err(forge_provider_common::adapter::AdapterError::VersionMismatch.into());
    }
    if *stop.borrow() {
        return Ok(SessionResult::Aborted);
    }
    let key = PrivateMaterialization::open(Path::new(VIRTUAL_KEY_PATH))
        .and_then(|file| file.read(4096))
        .map_err(|_| OpenCodeError::Process)?;
    let key_text = std::str::from_utf8(key.expose()).map_err(|_| OpenCodeError::Process)?;
    if !key_text.starts_with("sk-forge-") || key_text.chars().any(char::is_whitespace) {
        return Err(OpenCodeError::Process);
    }
    for path in [
        "/run/forge/home",
        "/run/forge/config",
        "/run/forge/cache",
        "/run/forge/data",
        "/run/forge/state",
    ] {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|_| OpenCodeError::Process)?;
    }
    let mut input = Zeroizing::new(Vec::new());
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut input)
        .map_err(|_| OpenCodeError::Process)?;
    if input.is_empty() || input.len() > 1024 * 1024 {
        return Err(OpenCodeError::Process);
    }
    let prompt = SecretBytes::new(std::mem::take(&mut input));
    let mut command = Command::new("opencode");
    command.args(["serve","--hostname","127.0.0.1","--port","4096"])
        .current_dir(workdir).env_clear().envs(environment).env("FORGE_LITELLM_KEY",key_text)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true)
        // Driver receives wrapper SIGINT first and calls HTTP abort. The server
        // has its own group; force-stop still targets the whole OCI environment.
        .process_group(0);
    let mut server = command.spawn().map_err(|_| OpenCodeError::Process)?;
    drop(command);
    drop(key);
    let client = OpenCodeClient::new(SERVER_URL, workdir)?;
    let result = async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if *stop.borrow() {
                return Ok(SessionResult::Aborted);
            }
            if server
                .try_wait()
                .map_err(|_| OpenCodeError::Process)?
                .is_some()
            {
                return Err(OpenCodeError::Process);
            }
            if client.healthy().await.unwrap_or(false) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(OpenCodeError::Process);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let session = client.create_session().await?;
        if let Some(scope) = forge_provider_common::native_input::managed_scope("opencode_runtime")
            .map_err(|_| OpenCodeError::Process)?
        {
            let mut mailbox = forge_provider_common::native_input::managed_mailbox(scope)
                .map_err(|_| OpenCodeError::Process)?;
            client
                .run_live_session(&session, model, prompt, stop, &mut mailbox, emit_bytes)
                .await
        } else {
            client
                .run_session(&session, model, &prompt, stop, emit)
                .await
        }
    }
    .await;
    // Exit of this child is not proof that every tool descendant is gone. The
    // wrapper/Supervisor waits for the container before reporting termination.
    let _ = server.kill().await;
    let _ = server.wait().await;
    result
}

fn managed_environment() -> Result<BTreeMap<String, String>, OpenCodeError> {
    let environment: BTreeMap<_, _> = ENVIRONMENT_KEYS
        .into_iter()
        .filter_map(|key| std::env::var(key).ok().map(|value| (key.into(), value)))
        .collect();
    for (key, value) in [
        ("HOME", "/run/forge/home"),
        ("OPENCODE_DISABLE_PROJECT_CONFIG", "true"),
        ("OPENCODE_DISABLE_MODELS_FETCH", "true"),
    ] {
        if environment.get(key).map(String::as_str) != Some(value) {
            return Err(OpenCodeError::Process);
        }
    }
    if !environment.contains_key("OPENCODE_CONFIG_CONTENT") {
        return Err(OpenCodeError::Process);
    }
    Ok(environment)
}

fn emit(event: RuntimeObservation) -> Result<(), OpenCodeError> {
    let bytes = write_driver_event(&event)?;
    emit_bytes(bytes)
}

fn emit_bytes(bytes: SecretBytes) -> Result<(), OpenCodeError> {
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(bytes.expose())
        .and_then(|()| stdout.write_all(b"\n"))
        .and_then(|()| stdout.flush())
        .map_err(|_| OpenCodeError::EventSink)
}
