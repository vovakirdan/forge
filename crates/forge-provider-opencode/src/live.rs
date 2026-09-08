//! One private native session. HTTP submission alone is never an acceptance receipt.
use crate::{
    EventInterpreter, OpenCodeClient, OpenCodeError, SessionEvent, SessionId, SessionResult,
    client::{SseLimit, bounded_json, check_status},
    write_driver_event,
};
use eventsource_stream::Eventsource;
use forge_protocol::supervisor::v1::deliver_runtime_input::Action;
use forge_provider_common::{
    SecretBytes,
    adapter::RuntimeObservation,
    native_event::{NativeDriverEvent, UsageBasis},
    native_input::{NativeMailbox, addressed_prompt},
};
use futures_util::{StreamExt, TryStreamExt};
use reqwest::Method;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::watch;
use uuid::Uuid;
use zeroize::Zeroizing;

impl OpenCodeClient {
    #[allow(clippy::too_many_arguments)]
    pub async fn run_live_session<F>(
        &self,
        session: &SessionId,
        model: &str,
        prompt: SecretBytes,
        mut stop: watch::Receiver<bool>,
        mailbox: &mut NativeMailbox,
        mut emit: F,
    ) -> Result<SessionResult, OpenCodeError>
    where
        F: FnMut(SecretBytes) -> Result<(), OpenCodeError>,
    {
        if model.trim().is_empty() || model.chars().any(char::is_control) {
            return Err(OpenCodeError::InvalidResponse);
        }
        let run = Uuid::parse_str(&mailbox.scope().run_id).map_err(|_| OpenCodeError::Process)?;
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
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"))
        {
            return Err(OpenCodeError::InvalidResponse);
        }
        let mut limit = SseLimit::default();
        let chunks = response
            .bytes_stream()
            .map_err(|_| OpenCodeError::Transport)
            .and_then(move |chunk| std::future::ready(limit.accept(&chunk).map(|()| chunk)));
        let stream = chunks.eventsource();
        futures_util::pin_mut!(stream);
        emit(write_driver_event(&RuntimeObservation::ThreadStarted)?)?;
        let mut interpreter = EventInterpreter::new(session.as_str());
        let mut next = Some((run, prompt, None));
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            let (input, prompt, command) = if let Some(next) = next.take() {
                next
            } else {
                tokio::select! {
                    biased;
                    changed=stop.changed()=>{
                        if changed.is_err() || *stop.borrow(){self.abort(session).await?;return Ok(SessionResult::Aborted);}
                        continue;
                    }
                    event=stream.next()=>{
                        let event=event.ok_or(OpenCodeError::UnexpectedEof)?.map_err(|_| OpenCodeError::InvalidResponse)?;
                        let data=Zeroizing::new(event.data);
                        if let SessionEvent::Observation(RuntimeObservation::Failure{kind})=interpreter.ingest(&data)? {
                            emit(write_driver_event(&RuntimeObservation::Failure{kind})?)?;
                            return Ok(SessionResult::ProviderFailed);
                        }
                        continue;
                    }
                    _=tick.tick()=>{
                        let Some(command)=mailbox.next().map_err(|_| OpenCodeError::Process)? else {continue;};
                        if matches!(command.action,Some(Action::CloseAfterTurn(_))) {
                            mailbox.closed(&command).map_err(|_| OpenCodeError::Process)?;
                            return Ok(SessionResult::TurnEnded);
                        }
                        let id=Uuid::parse_str(&command.command_id).map_err(|_| OpenCodeError::Process)?;
                        let prompt=addressed_prompt(&command).map_err(|_| OpenCodeError::Process)?;
                        mailbox.mark_sent(&command).map_err(|_| OpenCodeError::Process)?;
                        (id,prompt,Some(command))
                    }
                }
            };
            if *stop.borrow() {
                return Ok(SessionResult::Aborted);
            }
            let message = native_message_id(input);
            interpreter.begin_native_turn(&message);
            self.native_prompt(session, model, &prompt, &message)
                .await?;
            let mut accepted = false;
            loop {
                tokio::select! {
                    biased;
                    changed=stop.changed()=>{
                        if changed.is_err() || *stop.borrow(){self.abort(session).await?;return Ok(SessionResult::Aborted);}
                    }
                    event=stream.next()=>{
                        let event=event.ok_or(OpenCodeError::UnexpectedEof)?.map_err(|_| OpenCodeError::InvalidResponse)?;
                        let data=Zeroizing::new(event.data);
                        // The native user message is persisted by OpenCode, not a stdin/HTTP echo.
                        let value:Value=serde_json::from_str(&data).map_err(|_| OpenCodeError::InvalidResponse)?;
                        if !accepted && value.get("type").and_then(Value::as_str)==Some("message.updated")
                            && user_matches(value.pointer("/properties/info"),session,&message) {
                            if let Some(command)=&command {mailbox.accepted(command).map_err(|_| OpenCodeError::Process)?;}
                            emit(NativeDriverEvent::InputAccepted{run_id:run,input_id:input,session_id:session.as_str().into(),turn_id:message.clone()}.encode()?)?;
                            accepted=true;
                        }
                        match interpreter.ingest(&data)? {
                            SessionEvent::Idle=>{
                                if !accepted {
                                    // An event can be coalesced upstream; query that exact native ID.
                                    self.confirm_message(session,&message).await?;
                                    if let Some(command)=&command {mailbox.accepted(command).map_err(|_| OpenCodeError::Process)?;}
                                    emit(NativeDriverEvent::InputAccepted{run_id:run,input_id:input,session_id:session.as_str().into(),turn_id:message.clone()}.encode()?)?;
                                }
                                emit(NativeDriverEvent::TurnFinished{run_id:run,input_id:input,session_id:session.as_str().into(),turn_id:message.clone(),completed:true,usage:interpreter.usage(),usage_basis:UsageBasis::Cumulative}.encode()?)?;
                                break;
                            }
                            SessionEvent::Observation(RuntimeObservation::Ignored)=>{},
                            SessionEvent::Observation(observation)=>{
                                let failed=matches!(observation,RuntimeObservation::Failure{..});
                                emit(write_driver_event(&observation)?)?;
                                if failed {
                                    if accepted {emit(NativeDriverEvent::TurnFinished{run_id:run,input_id:input,session_id:session.as_str().into(),turn_id:message.clone(),completed:false,usage:interpreter.usage(),usage_basis:UsageBasis::Cumulative}.encode()?)?;}
                                    self.abort(session).await?;return Ok(SessionResult::ProviderFailed);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn native_prompt(
        &self,
        session: &SessionId,
        model: &str,
        prompt: &SecretBytes,
        message: &str,
    ) -> Result<(), OpenCodeError> {
        #[derive(Serialize)]
        struct Part<'a> {
            r#type: &'static str,
            text: &'a str,
        }
        #[derive(Serialize)]
        struct Model<'a> {
            #[serde(rename = "providerID")]
            provider: &'static str,
            #[serde(rename = "modelID")]
            model: &'a str,
        }
        #[derive(Serialize)]
        struct Prompt<'a> {
            #[serde(rename = "messageID")]
            id: &'a str,
            model: Model<'a>,
            agent: &'static str,
            parts: [Part<'a>; 1],
        }
        let text =
            std::str::from_utf8(prompt.expose()).map_err(|_| OpenCodeError::InvalidResponse)?;
        let body = Prompt {
            id: message,
            model: Model {
                provider: "forge",
                model,
            },
            agent: "build",
            parts: [Part {
                r#type: "text",
                text,
            }],
        };
        let response = self
            .request(
                Method::POST,
                &format!("session/{}/prompt_async", session.as_str()),
            )?
            .timeout(Duration::from_secs(15))
            .json(&body)
            .send()
            .await
            .map_err(|_| OpenCodeError::Transport)?;
        check_status(&response)?;
        if response.status().as_u16() != 204 {
            return Err(OpenCodeError::InvalidResponse);
        }
        Ok(())
    }
    async fn confirm_message(
        &self,
        session: &SessionId,
        message: &str,
    ) -> Result<(), OpenCodeError> {
        let response = self
            .request(
                Method::GET,
                &format!("session/{}/message/{message}", session.as_str()),
            )?
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|_| OpenCodeError::Transport)?;
        check_status(&response)?;
        let value: Value = bounded_json(response).await?;
        if !user_matches(value.get("info"), session, message) {
            return Err(OpenCodeError::InvalidResponse);
        }
        Ok(())
    }
}

pub(crate) fn native_message_id(input: Uuid) -> String {
    format!("msg_{}", input.simple())
}
fn user_matches(value: Option<&Value>, session: &SessionId, message: &str) -> bool {
    value.is_some_and(|v| {
        v.get("id").and_then(Value::as_str) == Some(message)
            && v.get("sessionID").and_then(Value::as_str) == Some(session.as_str())
            && v.get("role").and_then(Value::as_str) == Some("user")
    })
}
