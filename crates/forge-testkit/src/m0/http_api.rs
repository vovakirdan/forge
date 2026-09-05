use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use forge_core::{CoreService, router};
use tokio::{net::UnixListener, sync::watch, task::JoinHandle, time::timeout};
use uuid::Uuid;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// A real owner-only HTTP-over-UDS API listener for transport acceptance tests.
pub struct LocalHttpApi {
    socket_path: PathBuf,
    temp_dir: PathBuf,
    shutdown: watch::Sender<bool>,
    task: Option<JoinHandle<std::io::Result<()>>>,
}

impl LocalHttpApi {
    /// Starts a bounded public API listener over an isolated Unix socket.
    pub fn start(core: CoreService) -> Result<Self> {
        let temp_dir = test_directory()?;
        let socket_path = temp_dir.join("api.sock");
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("bind test Core API socket {}", socket_path.display()))?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
            .context("restrict test Core API socket")?;

        let (shutdown, shutdown_receiver) = watch::channel(false);
        let task = tokio::spawn(async move {
            axum::serve(listener, router(core).into_make_service())
                .with_graceful_shutdown(wait_for_shutdown(shutdown_receiver))
                .await
        });

        Ok(Self {
            socket_path,
            temp_dir,
            shutdown,
            task: Some(task),
        })
    }

    /// Returns the unique public API socket used by this test listener.
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket_path
    }

    /// Stops the API task and removes only its unique test socket directory.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(mut task) = self.task.take()
            && timeout(SHUTDOWN_TIMEOUT, &mut task).await.is_err()
        {
            task.abort();
            let _ = task.await;
        }
        let _ = fs::remove_file(&self.socket_path);
        let _ = fs::remove_dir(&self.temp_dir);
    }
}

impl Drop for LocalHttpApi {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

fn test_directory() -> Result<PathBuf> {
    let directory = env::temp_dir().join(format!("forge-m0-http-{}", Uuid::now_v7()));
    fs::create_dir(&directory).context("create isolated M0 API socket directory")?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .context("restrict M0 API socket directory")?;
    Ok(directory)
}

async fn wait_for_shutdown(mut receiver: watch::Receiver<bool>) {
    if !*receiver.borrow() {
        let _ = receiver.changed().await;
    }
}
