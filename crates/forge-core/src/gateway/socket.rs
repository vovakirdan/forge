//! Owner-only, dedicated UDS lifecycle. No administrative socket is exposed.

use super::{CoreError, MAX_REQUEST_BYTES, RunGateway, ToolRequest, invalid, mcp, safe_error};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use std::{
    io,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{net::UnixListener, sync::oneshot, task::JoinHandle};

/// Retain until the Run has physically stopped. Dropping aborts its listener;
/// stale socket paths can only be replaced after a failed liveness check.
pub struct GatewayHandle {
    path: PathBuf,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    closed: Arc<AtomicBool>,
}
impl GatewayHandle {
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.path
    }
    pub async fn shutdown(mut self) {
        self.closed.store(true, Ordering::Release);
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for GatewayHandle {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub(super) async fn serve(gateway: RunGateway, root: &Path) -> Result<GatewayHandle, CoreError> {
    if !root.is_absolute() {
        return Err(invalid("Gateway root must be absolute"));
    }
    secure_directory(root)?;
    let directory = root.join(gateway.scope.run_id.to_string());
    secure_directory(&directory)?;
    let path = directory.join("gateway.sock");
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if !metadata.file_type().is_socket()
            || metadata.uid() != nix::unistd::Uid::current().as_raw()
        {
            return Err(invalid("existing Gateway path is not an owned socket"));
        }
        match tokio::net::UnixStream::connect(&path).await {
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                std::fs::remove_file(&path)
                    .map_err(|_| invalid("cannot replace stale Gateway socket"))?
            }
            _ => return Err(invalid("Gateway socket is already active or inaccessible")),
        }
    }
    let listener = UnixListener::bind(&path).map_err(|_| invalid("cannot bind Gateway socket"))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|_| invalid("cannot protect Gateway socket"))?;
    let (stop, stopped) = oneshot::channel();
    let closed = Arc::clone(&gateway.closed);
    let closed_on_exit = Arc::clone(&closed);
    let router = router(gateway);
    let task = tokio::spawn(async move {
        // Limit shutdown even when a malicious client keeps an HTTP body open.
        let serving = axum::serve(listener, router);
        tokio::select! {_ = serving => {},_ = stopped=>{}}
        closed_on_exit.store(true, Ordering::Release);
    });
    Ok(GatewayHandle {
        path,
        stop: Some(stop),
        task: Some(task),
        closed,
    })
}

pub(super) fn router(gateway: RunGateway) -> Router {
    let egress = gateway.clone();
    Router::new()
        .route("/tools", post(http_tool))
        .nest_service("/mcp", mcp::service(gateway.clone()))
        .fallback(move |request: Request| {
            crate::egress::connect_handler(egress.core.clone(), egress.scope, request)
        })
        .layer(middleware::from_fn_with_state(gateway.clone(), scope_gate))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn_with_state(
            gateway.core.clone(),
            crate::observability::middleware,
        ))
        .with_state(gateway)
}

async fn scope_gate(State(gateway): State<RunGateway>, request: Request, next: Next) -> Response {
    let Ok(_permit) = gateway.requests.try_acquire() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    if gateway.closed.load(Ordering::Acquire) {
        return StatusCode::FORBIDDEN.into_response();
    }
    // Tool calls authorize inside invoke, which can replay one immutable
    // stop-producing receipt after its own Run has stopped. MCP initialize/ping
    // reveal only static protocol metadata; list_tools still authorizes itself.
    let tool_post = request.method() == Method::POST
        && matches!(request.uri().path(), "/tools" | "/mcp" | "/mcp/");
    if request.method() != Method::CONNECT
        && !tool_post
        && let Err(error) = gateway.authorize().await
    {
        let _ = gateway
            .audit_decision("unknown", None, false, "transport_scope_revoked")
            .await;
        return (StatusCode::FORBIDDEN, Json(safe_error(&error))).into_response();
    }
    match tokio::time::timeout(Duration::from_secs(30), next.run(request)).await {
        Ok(response) => {
            if response.status().is_client_error() {
                let _ = gateway
                    .audit_decision("unknown", None, false, "transport_request_rejected")
                    .await;
            }
            response
        }
        Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
    }
}

async fn http_tool(
    State(gateway): State<RunGateway>,
    Json(request): Json<ToolRequest>,
) -> Response {
    match gateway.invoke(request).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => {
            let status = match error {
                CoreError::IdempotencyConflict => StatusCode::CONFLICT,
                CoreError::Storage(_) => StatusCode::SERVICE_UNAVAILABLE,
                _ => StatusCode::FORBIDDEN,
            };
            (status, Json(safe_error(&error))).into_response()
        }
    }
}

fn secure_directory(path: &Path) -> Result<(), CoreError> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(invalid("cannot create Gateway directory")),
    }
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| invalid("cannot inspect Gateway directory"))?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::Uid::current().as_raw()
        || metadata.mode() & 0o077 != 0
        || std::fs::canonicalize(path).map_err(|_| invalid("cannot resolve Gateway directory"))?
            != path
    {
        return Err(invalid(
            "Gateway directory must be owned, private and free of symlink components",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gateway_directory_rejects_symlinks_and_wide_permissions() {
        let path = std::env::temp_dir().join(format!("forge-gateway-dir-{}", uuid::Uuid::now_v7()));
        secure_directory(&path).expect("private new directory");
        assert_eq!(
            std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let link = path.join("link");
        std::os::unix::fs::symlink(&path, &link).expect("fixture symlink");
        assert!(secure_directory(&link).is_err());
        std::fs::remove_file(&link).expect("remove fixture symlink");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("widen fixture");
        assert!(secure_directory(&path).is_err());
        std::fs::remove_dir(&path).expect("remove empty fixture");
    }
}
