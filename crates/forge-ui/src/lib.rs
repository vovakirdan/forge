//! Local-owner browser boundary. Canonical reads remain behind the Core UDS.

mod assets;
mod auth;
pub mod control;
mod core_client;
#[cfg(test)]
mod core_client_tests;
mod http;
#[cfg(test)]
mod http_tests;
mod read_target;
#[cfg(test)]
mod read_target_tests;
#[cfg(test)]
mod server_tests;

use std::{convert::Infallible, path::PathBuf, sync::Arc, time::Duration};

use axum::{Router, body::Body};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, watch},
    task::JoinSet,
};
use tower::ServiceExt;

use assets::AssetSnapshot;
use auth::SessionStore;
use core_client::CoreClient;
use http::HttpState;

/// Only trusted owner paths are configurable. The TCP bind is deliberately fixed.
#[derive(Debug)]
pub struct GatewayConfig {
    /// Directory containing the checked `forge-live-v1` artifact.
    pub assets_dir: PathBuf,
    /// Fixed owner-local Core endpoint; never supplied by an HTTP request.
    pub core_socket: PathBuf,
    /// Separate private owner bootstrap endpoint.
    pub control_socket: PathBuf,
}

/// Startup/serving failures contain no authentication or upstream response bodies.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error(transparent)]
    Assets(#[from] assets::AssetError),
    #[error(transparent)]
    Control(#[from] control::ControlError),
    #[error("UI listener unavailable")]
    Listener,
}

/// Owns a static snapshot, volatile sessions, and both local listeners.
pub struct Gateway {
    listener: TcpListener,
    control: control::ControlServer,
    state: Arc<HttpState>,
}

impl Gateway {
    /// Load the live artifact and bind both listeners, without reading Core.
    pub async fn bind(config: GatewayConfig) -> Result<Self, GatewayError> {
        let assets = AssetSnapshot::load(&config.assets_dir)?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| GatewayError::Listener)?;
        let host = listener
            .local_addr()
            .map_err(|_| GatewayError::Listener)?
            .to_string();
        let origin = format!("http://{host}");
        let sessions = Arc::new(SessionStore::default());
        let issuer = sessions.clone();
        let code_origin = origin.clone();
        let control = control::bind_control(
            &config.control_socket,
            &origin,
            Arc::new(move || {
                let code = issuer
                    .issue_code()
                    .map_err(|_| control::ControlError::IssuanceFailed)?;
                Ok(forge_protocol::ui_control::LoginCodeResponse {
                    origin: code_origin.clone(),
                    code: code.code,
                    expires_at: code.expires_at,
                })
            }),
        )
        .await?;
        let state = Arc::new(HttpState {
            host,
            origin,
            sessions,
            assets,
            core: CoreClient {
                socket: config.core_socket,
            },
            capacity: Semaphore::new(8),
        });
        Ok(Self {
            listener,
            control,
            state,
        })
    }

    /// Actual loopback origin, safe to print before serving requests.
    pub fn origin(&self) -> &str {
        &self.state.origin
    }

    /// Serve until shutdown. No background request or session survives this future.
    pub async fn serve(self, mut shutdown: watch::Receiver<bool>) -> Result<(), GatewayError> {
        let Self {
            listener,
            control,
            state,
        } = self;
        let router = Router::new()
            .fallback(http::handle)
            .with_state(state.clone());
        let connections = Arc::new(Semaphore::new(32));
        let mut tasks = JoinSet::new();
        let (stop_control, control_shutdown) = watch::channel(false);
        let control_future = control.serve(control_shutdown);
        tokio::pin!(control_future);
        let mut control_finished = false;
        let mut result = loop {
            if *shutdown.borrow() {
                break Ok(());
            }
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break Ok(()) }
                }
                result = &mut control_future => {
                    control_finished = true;
                    break result.map_err(GatewayError::from)
                }
                Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
                accepted = listener.accept() => {
                    let (stream, _) = match accepted { Ok(value) => value, Err(_) => break Err(GatewayError::Listener) };
                    // No waiting queue: excess clients are closed before any request is read.
                    let Ok(permit) = connections.clone().try_acquire_owned() else { continue };
                    let router = router.clone();
                    tasks.spawn(async move {
                        let _permit = permit;
                        let service = service_fn(move |request: hyper::Request<Incoming>| {
                            let router = router.clone();
                            async move {
                                let response = router.oneshot(request.map(Body::new)).await?;
                                Ok::<_, Infallible>(response)
                            }
                        });
                        let mut builder = hyper::server::conn::http1::Builder::new();
                        builder.keep_alive(false).max_headers(64).max_buf_size(16 * 1024)
                            .timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(5));
                        // Also bounds idle clients, request bodies and slow response consumers.
                        let _ = tokio::time::timeout(Duration::from_secs(5), builder.serve_connection(TokioIo::new(stream), service)).await;
                    });
                }
            }
        };
        let _ = stop_control.send(true);
        if !control_finished && let Err(error) = control_future.await {
            result = Err(error.into());
        }
        tasks.shutdown().await;
        state.sessions.revoke_all();
        result
    }
}
