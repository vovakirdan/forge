//! Scoped native-provider CONNECT egress. TLS remains end-to-end encrypted.

use crate::{CoreError, CoreService};
use axum::{
    body::Body,
    extract::Request,
    http::{Method, StatusCode},
    response::Response,
};
use forge_domain::runtime::RunScope;
use hyper_util::rt::TokioIo;
use std::{
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};
use tokio::{
    net::{TcpStream, lookup_host},
    sync::Semaphore,
};

static TUNNELS: Semaphore = Semaphore::const_new(64);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

fn denied() -> CoreError {
    CoreError::InvalidTransport {
        field: "gateway.egress",
        reason: "provider destination or Run scope is not permitted".into(),
    }
}

impl CoreService {
    /// Checks scope and fixed native-provider destinations before opening a socket.
    pub(crate) async fn connect_run_egress(
        &self,
        scope: &RunScope,
        target: &str,
    ) -> Result<TcpStream, CoreError> {
        let host = allowed_destination(target).ok_or_else(denied)?;
        let mut transaction = self.store.begin().await?;
        let run = transaction
            .validate_gateway_scope(scope)
            .await?
            .ok_or_else(denied)?;
        let native = run
            .run_spec
            .pointer("/binding/execution_profile/adapter_id")
            .and_then(serde_json::Value::as_str)
            == Some("codex_cli");
        transaction.commit().await?;
        if !native {
            return Err(denied());
        }
        let addresses = tokio::time::timeout(CONNECT_TIMEOUT, lookup_host((host, 443)))
            .await
            .map_err(|_| denied())?
            .map_err(|_| denied())?;
        // Resolve once and connect to that exact checked IP; no second DNS lookup.
        for address in addresses {
            if !public_ipv4(address.ip()) {
                continue;
            }
            if let Ok(Ok(stream)) =
                tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address)).await
            {
                return Ok(stream);
            }
        }
        Err(denied())
    }
}

/// Fallback handler used only on the already run-scoped Gateway socket.
pub(crate) async fn connect_handler(
    core: CoreService,
    scope: RunScope,
    request: Request,
) -> Response {
    if request.method() != Method::CONNECT {
        return crate::inference::relay(core, scope, request).await;
    }
    let Some(authority) = request.uri().authority() else {
        return status(StatusCode::BAD_REQUEST);
    };
    let Ok(permit) = TUNNELS.try_acquire() else {
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    let Ok(mut upstream) = core.connect_run_egress(&scope, authority.as_str()).await else {
        return status(StatusCode::FORBIDDEN);
    };
    tokio::spawn(async move {
        let _permit = permit;
        let Ok(upgraded) = hyper::upgrade::on(request).await else {
            return;
        };
        let mut client = TokioIo::new(upgraded);
        let copy = tokio::io::copy_bidirectional(&mut client, &mut upstream);
        tokio::pin!(copy);
        let mut validity = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = &mut copy => break,
                _ = validity.tick() => {
                    let Ok(mut transaction)=core.store.begin().await else {break;};
                    let authorized=matches!(transaction.validate_gateway_scope(&scope).await,Ok(Some(_)));
                    let _=transaction.commit().await;
                    if !authorized {break;}
                }
            }
        }
    });
    status(StatusCode::OK)
}

fn status(code: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = code;
    response
}

fn allowed_destination(target: &str) -> Option<&str> {
    let (host, port) = target.rsplit_once(':')?;
    if port != "443" {
        return None;
    }
    matches!(host, "chatgpt.com" | "auth.openai.com" | "api.openai.com").then_some(host)
}

fn public_ipv4(address: IpAddr) -> bool {
    let IpAddr::V4(address) = address else {
        return false;
    };
    let [a, b, c, _] = address.octets();
    !(address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_documentation()
        || address.is_unspecified()
        || a == 0
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0)
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19))
        || address == Ipv4Addr::BROADCAST)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_native_tls_destinations_are_allowed() {
        assert_eq!(allowed_destination("chatgpt.com:443"), Some("chatgpt.com"));
        for target in [
            "chatgpt.com.evil:443",
            "chatgpt.com:80",
            "127.0.0.1:443",
            "169.254.169.254:443",
            "[::1]:443",
            "user@chatgpt.com:443",
        ] {
            assert!(allowed_destination(target).is_none());
        }
    }
    #[test]
    fn private_metadata_reserved_and_ipv6_addresses_are_denied() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "198.18.0.1",
            "0.1.2.3",
            "224.0.0.1",
            "::1",
        ] {
            assert!(!public_ipv4(ip.parse().expect("IP fixture")));
        }
        assert!(public_ipv4("1.1.1.1".parse().expect("public fixture")));
    }
}
