//! Fixed-route inference relay on the scoped socket. No proxy administration API.

use crate::{CoreError, CoreService, credentials::credential_error};
use axum::{
    body::{Body, to_bytes},
    extract::Request,
    http::{Method, StatusCode},
    response::Response,
};
use forge_domain::runtime::RunScope;
use forge_provider_litellm::RunKeySpec;
use futures_util::StreamExt;
use serde_json::Value;

pub(crate) async fn relay(core: CoreService, scope: RunScope, request: Request) -> Response {
    match relay_inner(core, scope, request).await {
        Ok(response) => response,
        Err(_) => {
            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::FORBIDDEN;
            response
        }
    }
}

async fn relay_inner(
    core: CoreService,
    scope: RunScope,
    request: Request,
) -> Result<Response, CoreError> {
    let path = request.uri().path();
    if request.method() != Method::POST
        || request.uri().query().is_some()
        || !matches!(path, "/v1/chat/completions" | "/v1/responses")
    {
        return Err(credential_error());
    }
    let path = path.to_owned();
    let mut transaction = core.store.begin().await?;
    let run = transaction
        .validate_gateway_scope(&scope)
        .await?
        .ok_or_else(credential_error)?;
    if run
        .run_spec
        .pointer("/binding/execution_profile/adapter_id")
        .and_then(Value::as_str)
        != Some("opencode_runtime")
    {
        return Err(credential_error());
    }
    let saved = transaction
        .run_proxy_key(run.id)
        .await?
        .ok_or_else(credential_error)?;
    if saved.revoked || saved.key_hash.is_none() {
        return Err(credential_error());
    }
    let spec: RunKeySpec = serde_json::from_value(saved.spec).map_err(|_| credential_error())?;
    if spec.expires_at <= time::OffsetDateTime::now_utc() {
        return Err(credential_error());
    }
    let key = core.open_proxy_key(&run, &saved.sealed_key)?;
    transaction.commit().await?;
    let body = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        to_bytes(request.into_body(), 4 * 1024 * 1024),
    )
    .await
    .map_err(|_| credential_error())?
    .map_err(|_| credential_error())?;
    let input: Value = serde_json::from_slice(&body).map_err(|_| credential_error())?;
    validate_inference_body(&input)?;
    if !input
        .get("model")
        .and_then(Value::as_str)
        .is_some_and(|model| spec.models.contains(model))
    {
        return Err(credential_error());
    }
    let proxy = core.inference.as_ref().ok_or_else(credential_error)?;
    if !scope_active(&core, scope).await || spec.expires_at <= time::OffsetDateTime::now_utc() {
        return Err(credential_error());
    }
    let pending = proxy
        .http
        .post(format!("{}{path}", proxy.endpoint))
        .bearer_auth(std::str::from_utf8(key.expose()).map_err(|_| credential_error())?)
        .header("content-type", "application/json")
        .body(body)
        .send();
    let mut upstream = while_authorized(&core, scope, spec.expires_at, pending).await?;
    let status = upstream.status();
    if !status.is_success() {
        let mut bytes = Vec::new();
        while let Some(chunk) =
            while_authorized(&core, scope, spec.expires_at, upstream.chunk()).await?
        {
            if bytes.len() + chunk.len() > 16 * 1024 {
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
        let budget_exceeded = status == StatusCode::TOO_MANY_REQUESTS
            && serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/error/type")
                        .and_then(Value::as_str)
                        .map(|value| value == "budget_exceeded")
                })
                .unwrap_or(false);
        if budget_exceeded {
            core.record_proxy_budget_exhausted(&run).await?;
        }
        // Upstream exception bodies can contain auth/routes; never forward them.
        let mut response = Response::new(Body::from(if budget_exceeded {
            "{\"error\":{\"type\":\"budget_exceeded\"}}"
        } else {
            "{\"error\":{\"type\":\"provider_request_failed\"}}"
        }));
        *response.status_mut() = status;
        response.headers_mut().insert(
            "content-type",
            axum::http::HeaderValue::from_static("application/json"),
        );
        return Ok(response);
    }
    let content_type = upstream.headers().get("content-type").cloned();
    let stream = upstream.bytes_stream();
    let body = async_stream::stream! {
        futures_util::pin_mut!(stream);
        let mut tick=tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            let next=tokio::select! {
                next=stream.next()=>next.map(|result|result.map_err(|_|std::io::Error::other("inference stream failed"))),
                _=tick.tick()=>{
                    if !scope_active(&core,scope).await || spec.expires_at<=time::OffsetDateTime::now_utc() {
                        Some(Err(std::io::Error::other("inference scope revoked")))
                    } else {
                        continue;
                    }
                }
            };
            match next {
                Some(Ok(bytes))=>yield Ok(bytes),
                Some(Err(error))=>{yield Err(error);break;},
                None=>break,
            }
        }
    };
    let mut response = Response::new(Body::from_stream(body));
    *response.status_mut() = status;
    if let Some(content_type) = content_type {
        response.headers_mut().insert("content-type", content_type);
    }
    Ok(response)
}

async fn while_authorized<T, E>(
    core: &CoreService,
    scope: RunScope,
    expires_at: time::OffsetDateTime,
    operation: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, CoreError> {
    tokio::pin!(operation);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(120));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            _=tick.tick()=>if !scope_active(core,scope).await || expires_at<=time::OffsetDateTime::now_utc() {return Err(credential_error());},
            _=&mut deadline=>return Err(credential_error()),
            result=&mut operation=>return result.map_err(|_|credential_error()),
        }
    }
}

fn validate_inference_body(input: &Value) -> Result<(), CoreError> {
    // Only inference content and sampling controls cross this boundary. Proxy
    // routing, credential, fallback, callback and per-request pricing controls
    // are deliberately absent, including any future unknown extension fields.
    const FIELDS: &[&str] = &[
        "model",
        "messages",
        "input",
        "instructions",
        "stream",
        "stream_options",
        "max_tokens",
        "max_completion_tokens",
        "max_output_tokens",
        "temperature",
        "top_p",
        "n",
        "stop",
        "seed",
        "presence_penalty",
        "frequency_penalty",
        "logit_bias",
        "logprobs",
        "top_logprobs",
        "response_format",
        "text",
        "reasoning",
        "reasoning_effort",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "verbosity",
        "user",
        "store",
    ];
    let object = input.as_object().ok_or_else(credential_error)?;
    if object.keys().any(|field| !FIELDS.contains(&field.as_str())) {
        return Err(credential_error());
    }
    if let Some(tools) = object.get("tools") {
        let tools = tools.as_array().ok_or_else(credential_error)?;
        if tools
            .iter()
            .any(|tool| tool.get("type").and_then(Value::as_str) != Some("function"))
        {
            return Err(credential_error());
        }
    }
    // Provider-hosted tools execute outside the issued sandbox/Gateway.
    if let Some(choice) = object.get("tool_choice") {
        let valid = match choice {
            Value::String(choice) => matches!(choice.as_str(), "none" | "auto" | "required"),
            Value::Object(choice) => choice.get("type").and_then(Value::as_str) == Some("function"),
            _ => false,
        };
        if !valid {
            return Err(credential_error());
        }
    }
    Ok(())
}

impl CoreService {
    async fn record_proxy_budget_exhausted(
        &self,
        run: &forge_storage::RunProjection,
    ) -> Result<(), CoreError> {
        let mut transaction = self.store.begin().await?;
        let mut project = transaction
            .lock_project(run.project_id)
            .await?
            .ok_or_else(credential_error)?;
        let now = crate::canonical_clock::project_mutation_time(&project);
        self.quarantine_execution(
            &mut transaction,
            &mut project,
            run,
            forge_storage::IncidentKind::BudgetExceeded,
            false,
            now,
        )
        .await?;
        transaction.commit().await?;
        let _ = self.deliver_pending_stop_requests(run.project_id).await;
        let _ = self.revoke_run_proxy_key(run).await;
        Ok(())
    }
}

async fn scope_active(core: &CoreService, scope: RunScope) -> bool {
    let Ok(mut transaction) = core.store.begin().await else {
        return false;
    };
    let active = matches!(transaction.gateway_scope_is_active(scope).await, Ok(true));
    active && transaction.commit().await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn denies_hosted_tools_but_keeps_worker_function_calls() {
        assert!(validate_inference_body(&serde_json::json!({"model":"alias","tools":[{"type":"function","function":{"name":"shell"}}]})).is_ok());
        for kind in [
            "mcp",
            "web_search",
            "computer_use_preview",
            "code_interpreter",
        ] {
            assert!(
                validate_inference_body(
                    &serde_json::json!({"model":"alias","tools":[{"type":kind}]})
                )
                .is_err()
            );
        }
    }
    #[test]
    fn permits_inference_but_not_proxy_overrides() {
        assert!(
            validate_inference_body(
                &serde_json::json!({"model":"alias","messages":[],"stream":true})
            )
            .is_ok()
        );
        for field in [
            "api_key",
            "api_base",
            "base_url",
            "fallbacks",
            "model_list",
            "litellm_params",
            "custom_llm_provider",
            "metadata",
            "max_budget",
        ] {
            let mut body = serde_json::json!({"model":"alias","messages":[]});
            body[field] = serde_json::json!("override");
            assert!(validate_inference_body(&body).is_err(), "{field}");
        }
    }
}
