use std::{fmt, time::Duration};

use eventsource_stream::Eventsource;
use forge_provider_common::{
    SecretBytes,
    adapter::{AdapterError, RuntimeObservation},
};
use futures_util::{StreamExt, TryStreamExt};
use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::watch;
use zeroize::Zeroizing;

use crate::{
    EventInterpreter, SessionEvent,
    events::MAX_EVENT_BYTES,
    prepare::{local_url, validate_workdir},
};

#[derive(Clone, PartialEq, Eq)]
pub struct SessionId(String);
impl SessionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    fn parse(id: String) -> Result<Self, OpenCodeError> {
        if !id.starts_with("ses_")
            || id.len() > 256
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(OpenCodeError::InvalidResponse);
        }
        Ok(Self(id))
    }
}
impl fmt::Debug for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionId([opaque])")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionResult {
    TurnEnded,
    Aborted,
    ProviderFailed,
}

#[derive(Debug, Error)]
pub enum OpenCodeError {
    #[error("OpenCode runtime contract failed: {0}")]
    Contract(#[from] AdapterError),
    #[error("OpenCode local HTTP transport failed")]
    Transport,
    #[error("OpenCode rejected the local request with HTTP {0}")]
    Http(u16),
    #[error("OpenCode returned malformed or oversized data")]
    InvalidResponse,
    #[error("OpenCode event stream ended without confirmed turn completion")]
    UnexpectedEof,
    #[error("OpenCode managed process or private input failed")]
    Process,
    #[error("OpenCode event sink is unavailable")]
    EventSink,
}

/// A sandbox-local API client. It cannot call provider/admin APIs, follows no
/// redirects, imports no proxy environment, and never logs HTTP error bodies.
pub struct OpenCodeClient {
    http: Client,
    base: Url,
    workdir: String,
}
impl OpenCodeClient {
    pub fn new(endpoint: &str, workdir: &str) -> Result<Self, OpenCodeError> {
        let base = local_url(endpoint)?;
        validate_workdir(workdir)?;
        if base.path() != "/" {
            return Err(AdapterError::InvalidEnvironment.into());
        }
        let http = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| OpenCodeError::Transport)?;
        Ok(Self {
            http,
            base,
            workdir: workdir.into(),
        })
    }

    pub async fn healthy(&self) -> Result<bool, OpenCodeError> {
        let response = self
            .request(Method::GET, "global/health")?
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map_err(|_| OpenCodeError::Transport)?;
        if !response.status().is_success() {
            return Ok(false);
        }
        #[derive(Deserialize)]
        struct Health {
            healthy: bool,
            version: String,
        }
        let health: Health = bounded_json(response).await?;
        Ok(health.healthy && health.version == crate::PINNED_OPENCODE_VERSION)
    }

    pub async fn create_session(&self) -> Result<SessionId, OpenCodeError> {
        let response = self
            .request(Method::POST, "session")?
            .timeout(Duration::from_secs(15))
            .json(&serde_json::json!({"title":"Forge Run"}))
            .send()
            .await
            .map_err(|_| OpenCodeError::Transport)?;
        check_status(&response)?;
        #[derive(Deserialize)]
        struct Created {
            id: String,
        }
        SessionId::parse(bounded_json::<Created>(response).await?.id)
    }

    pub async fn abort(&self, session: &SessionId) -> Result<(), OpenCodeError> {
        let response = self
            .request(Method::POST, &format!("session/{}/abort", session.0))?
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|_| OpenCodeError::Transport)?;
        check_status(&response)?;
        if !bounded_json::<bool>(response).await? {
            return Err(OpenCodeError::InvalidResponse);
        }
        Ok(())
    }

    /// Subscribe before submission. EOF, malformed events, or HTTP errors do not
    /// imply success. Stop requests use the server's abort API before teardown.
    pub async fn run_session<F>(
        &self,
        session: &SessionId,
        model: &str,
        prompt: &SecretBytes,
        mut stop: watch::Receiver<bool>,
        mut emit: F,
    ) -> Result<SessionResult, OpenCodeError>
    where
        F: FnMut(RuntimeObservation) -> Result<(), OpenCodeError>,
    {
        if model.trim().is_empty() || model.chars().any(char::is_control) {
            return Err(AdapterError::ProfileMismatch.into());
        }
        if *stop.borrow() {
            return Ok(SessionResult::Aborted);
        }
        let response = tokio::time::timeout(
            Duration::from_secs(15),
            self.request(Method::GET, "event")?
                .header("accept", "text/event-stream")
                .send(),
        )
        .await
        .map_err(|_| OpenCodeError::Transport)?
        .map_err(|_| OpenCodeError::Transport)?;
        check_status(&response)?;
        if !response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"))
        {
            return Err(OpenCodeError::InvalidResponse);
        }
        let mut limit = SseLimit::default();
        let chunks = response
            .bytes_stream()
            .map_err(|_| OpenCodeError::Transport)
            .and_then(move |chunk| {
                let result = limit.accept(&chunk).map(|()| chunk);
                std::future::ready(result)
            });
        let stream = chunks.eventsource();
        futures_util::pin_mut!(stream);
        emit(RuntimeObservation::ThreadStarted)?;
        if *stop.borrow() {
            return Ok(SessionResult::Aborted);
        }
        self.prompt(session, model, prompt).await?;
        let mut interpreter = EventInterpreter::new(session.as_str());
        loop {
            tokio::select! {
                biased;
                changed=stop.changed()=>{
                    if changed.is_err() || *stop.borrow(){self.abort(session).await?;return Ok(SessionResult::Aborted);}
                },
                event=stream.next()=>{
                    let event=event.ok_or(OpenCodeError::UnexpectedEof)?.map_err(|_|OpenCodeError::InvalidResponse)?;
                    // The SSE library owns this frame briefly; clear it on drop.
                    let data=Zeroizing::new(event.data);
                    match interpreter.ingest(&data)? {
                        SessionEvent::Idle=>{
                            emit(RuntimeObservation::TurnCompleted{usage:interpreter.usage()})?;
                            return Ok(SessionResult::TurnEnded);
                        },
                        SessionEvent::Observation(RuntimeObservation::Ignored)=>{},
                        SessionEvent::Observation(observation)=>{
                            let failed=matches!(observation,RuntimeObservation::Failure{..});
                            emit(observation)?;
                            if failed {self.abort(session).await?;return Ok(SessionResult::ProviderFailed);}
                        }
                    }
                }
            }
        }
    }

    async fn prompt(
        &self,
        session: &SessionId,
        model: &str,
        prompt: &SecretBytes,
    ) -> Result<(), OpenCodeError> {
        #[derive(Serialize)]
        struct Part<'a> {
            r#type: &'static str,
            text: &'a str,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Model<'a> {
            #[serde(rename = "providerID")]
            provider_id: &'static str,
            #[serde(rename = "modelID")]
            model_id: &'a str,
        }
        #[derive(Serialize)]
        struct Prompt<'a> {
            model: Model<'a>,
            agent: &'static str,
            parts: [Part<'a>; 1],
        }
        let text =
            std::str::from_utf8(prompt.expose()).map_err(|_| OpenCodeError::InvalidResponse)?;
        let body = Prompt {
            model: Model {
                provider_id: "forge",
                model_id: model,
            },
            agent: "build",
            parts: [Part {
                r#type: "text",
                text,
            }],
        };
        let response = self
            .request(Method::POST, &format!("session/{}/prompt_async", session.0))?
            .timeout(Duration::from_secs(15))
            .json(&body)
            .send()
            .await
            .map_err(|_| OpenCodeError::Transport)?;
        check_status(&response)
    }

    fn request(
        &self,
        method: Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, OpenCodeError> {
        let url = self
            .base
            .join(path)
            .map_err(|_| OpenCodeError::InvalidResponse)?;
        Ok(self
            .http
            .request(method, url)
            .header("x-opencode-directory", &self.workdir))
    }
}

fn check_status(response: &reqwest::Response) -> Result<(), OpenCodeError> {
    if response.status().is_success() {
        Ok(())
    } else {
        Err(OpenCodeError::Http(response.status().as_u16()))
    }
}
async fn bounded_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T, OpenCodeError> {
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| OpenCodeError::Transport)?
    {
        if bytes.len() + chunk.len() > MAX_EVENT_BYTES {
            return Err(OpenCodeError::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| OpenCodeError::InvalidResponse)
}

/// Bound unterminated frames *before* the standard SSE decoder buffers them.
#[derive(Default)]
struct SseLimit {
    frame_bytes: usize,
    line_bytes: usize,
}
impl SseLimit {
    fn accept(&mut self, bytes: &[u8]) -> Result<(), OpenCodeError> {
        for byte in bytes {
            self.frame_bytes += 1;
            if self.frame_bytes > MAX_EVENT_BYTES {
                return Err(OpenCodeError::InvalidResponse);
            }
            if *byte == b'\n' {
                if self.line_bytes == 0 {
                    self.frame_bytes = 0;
                }
                self.line_bytes = 0;
            } else if *byte != b'\r' {
                self.line_bytes += 1;
            }
        }
        Ok(())
    }
}
