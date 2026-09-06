use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use forge_provider_common::SecretBytes;
use forge_provider_litellm::{
    LiteLlmClient, LiteLlmError, RouteSpec, RunKeySpec, generate_virtual_key,
};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

#[derive(Default)]
struct FixtureState {
    key: Option<Value>,
    deployments: Vec<Value>,
    issues: usize,
    lose_issue_response: bool,
    corrupt_issue: bool,
}
type Shared = Arc<Mutex<FixtureState>>;

async fn fixture() -> (LiteLlmClient, Shared, tokio::task::JoinHandle<()>) {
    let state = Shared::default();
    let app = Router::new()
        .route("/key/info", get(info))
        .route("/key/generate", post(issue))
        .route("/key/delete", post(delete))
        .route("/model/info", get(models))
        .route("/model/new", post(deploy))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = LiteLlmClient::new(
        &endpoint,
        &SecretBytes::new(b"synthetic-admin-only".to_vec()),
    )
    .unwrap();
    (client, state, server)
}
fn admin(headers: &HeaderMap) {
    assert_eq!(headers["authorization"], "Bearer synthetic-admin-only");
}
async fn info(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    admin(&headers);
    let key = &query["key"];
    assert_eq!(key.len(), 64, "lookup must never send the plaintext key");
    assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
    match &state.lock().unwrap().key {
        Some(info) => (StatusCode::OK, Json(json!({"info":info}))),
        None => (StatusCode::NOT_FOUND, Json(json!({"error":"not found"}))),
    }
}
async fn issue(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    admin(&headers);
    assert!(body["key"].as_str().unwrap().starts_with("sk-forge-"));
    let seconds: i64 = body["duration"]
        .as_str()
        .unwrap()
        .trim_end_matches('s')
        .parse()
        .unwrap();
    body["expires"] = json!(
        (OffsetDateTime::now_utc() + Duration::seconds(seconds))
            .format(&Rfc3339)
            .unwrap()
    );
    body["spend"] = json!(0.25);
    body["metadata"]["allowed_passthrough_routes"] = json!([]);
    body.as_object_mut().unwrap().remove("key");
    let mut state = state.lock().unwrap();
    state.issues += 1;
    if state.corrupt_issue {
        body["models"] = json!(["*"]);
    }
    state.key = Some(body);
    (
        if state.lose_issue_response {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::OK
        },
        Json(json!({"key":"never copy issuance response into a log"})),
    )
}
async fn delete(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    admin(&headers);
    let mut state = state.lock().unwrap();
    assert_eq!(body["keys"].as_array().unwrap().len(), 1);
    assert_eq!(body["keys"][0].as_str().unwrap().len(), 64);
    assert!(body.get("key_aliases").is_none());
    state.key = None;
    Json(json!({"deleted_keys":1}))
}
async fn models(State(state): State<Shared>, headers: HeaderMap) -> Json<Value> {
    admin(&headers);
    Json(json!({"data":state.lock().unwrap().deployments}))
}
async fn deploy(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Json<Value> {
    admin(&headers);
    assert_eq!(body["litellm_params"]["api_key"], "synthetic-upstream-only");
    body["litellm_params"]["api_key"] = json!("masked-by-service");
    state.lock().unwrap().deployments.push(body);
    Json(json!({"ok":true}))
}
fn spec() -> RunKeySpec {
    RunKeySpec {
        run_id: Uuid::now_v7(),
        project_id: Uuid::now_v7(),
        environment_epoch: 1,
        fencing_token: 7,
        execution_profile_id: Uuid::now_v7(),
        execution_profile_revision: 2,
        models: ["forge-route-exact".to_owned()].into(),
        expires_at: OffsetDateTime::now_utc() + Duration::minutes(10),
        requests_per_minute: 6,
        tokens_per_minute: 9000,
        max_budget_usd: Some(2.0),
    }
}

#[tokio::test]
async fn issue_reconcile_observe_and_revoke_use_exact_run_identity() {
    let (client, state, server) = fixture().await;
    let spec = spec();
    let key = generate_virtual_key().unwrap();
    let issued = client
        .create_or_reconcile(&spec, &key, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert!(!issued.reused);
    assert!(
        client
            .create_or_reconcile(&spec, &key, OffsetDateTime::now_utc())
            .await
            .unwrap()
            .reused
    );
    assert_eq!(state.lock().unwrap().issues, 1);
    let usage = client
        .usage(&spec, &issued.key_hash, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(usage.spend_usd, Some(0.25));
    assert!(!usage.final_accounting_confirmed);
    client.revoke(&spec, &issued.key_hash).await.unwrap();
    client.revoke(&spec, &issued.key_hash).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn lost_create_response_reconciles_without_minting_another_key() {
    let (client, state, server) = fixture().await;
    state.lock().unwrap().lose_issue_response = true;
    let spec = spec();
    let key = generate_virtual_key().unwrap();
    assert!(matches!(
        client
            .create_or_reconcile(&spec, &key, OffsetDateTime::now_utc())
            .await,
        Err(LiteLlmError::Http(503))
    ));
    assert!(
        client
            .create_or_reconcile(&spec, &key, OffsetDateTime::now_utc())
            .await
            .unwrap()
            .reused
    );
    assert_eq!(state.lock().unwrap().issues, 1);
    server.abort();
}

#[tokio::test]
async fn policy_drift_expiry_and_budget_reset_are_never_silently_adopted() {
    for (field, value) in [
        ("models", json!(["*"])),
        ("allowed_routes", json!(["*"])),
        ("max_budget", json!(9000)),
        ("rpm_limit", json!(100)),
        ("tpm_limit", json!(100000)),
        ("budget_duration", json!("1d")),
        ("expires", json!("2099-01-01T00:00:00Z")),
        ("expires", json!("2001-01-01T00:00:00Z")),
        ("auto_rotate", json!(true)),
        ("blocked", json!(true)),
        ("permissions", json!({"get_spend_routes":true})),
        ("config", json!({"model_list":["other"]})),
        ("aliases", json!({"forge-route-exact":"other"})),
        ("allowed_passthrough_routes", json!(["*"])),
    ] {
        let (client, state, server) = fixture().await;
        let spec = spec();
        let key = generate_virtual_key().unwrap();
        client
            .create_or_reconcile(&spec, &key, OffsetDateTime::now_utc())
            .await
            .unwrap();
        state.lock().unwrap().key.as_mut().unwrap()[field] = value;
        assert!(
            matches!(
                client
                    .create_or_reconcile(&spec, &key, OffsetDateTime::now_utc())
                    .await,
                Err(LiteLlmError::PolicyConflict)
            ),
            "field {field}"
        );
        assert_eq!(state.lock().unwrap().issues, 1);
        server.abort();
    }
}

#[tokio::test]
async fn issuance_response_does_not_override_failed_policy_readback() {
    let (client, state, server) = fixture().await;
    state.lock().unwrap().corrupt_issue = true;
    assert!(matches!(
        client
            .create_or_reconcile(
                &spec(),
                &generate_virtual_key().unwrap(),
                OffsetDateTime::now_utc()
            )
            .await,
        Err(LiteLlmError::PolicyConflict)
    ));
    server.abort();
}

#[tokio::test]
async fn service_metadata_cannot_widen_passthrough_scope() {
    let (client, state, server) = fixture().await;
    let spec = spec();
    let key = generate_virtual_key().unwrap();
    client
        .create_or_reconcile(&spec, &key, OffsetDateTime::now_utc())
        .await
        .unwrap();
    state.lock().unwrap().key.as_mut().unwrap()["metadata"]["allowed_passthrough_routes"] =
        json!(["*"]);
    assert!(matches!(
        client
            .create_or_reconcile(&spec, &key, OffsetDateTime::now_utc())
            .await,
        Err(LiteLlmError::PolicyConflict)
    ));
    server.abort();
}

#[tokio::test]
async fn revoke_rejects_other_run_identity_without_deleting() {
    let (client, state, server) = fixture().await;
    let spec = spec();
    let issued = client
        .create_or_reconcile(
            &spec,
            &generate_virtual_key().unwrap(),
            OffsetDateTime::now_utc(),
        )
        .await
        .unwrap();
    let mut other = spec.clone();
    other.fencing_token += 1;
    assert!(matches!(
        client.revoke(&other, &issued.key_hash).await,
        Err(LiteLlmError::PolicyConflict)
    ));
    assert!(state.lock().unwrap().key.is_some());
    server.abort();
}

#[tokio::test]
async fn immutable_route_creation_reconciles_and_rejects_duplicate_aliases() {
    let (client, state, server) = fixture().await;
    let spec = RouteSpec {
        project_id: Uuid::now_v7(),
        credential_binding_id: Uuid::now_v7(),
        secret_id: Uuid::now_v7(),
        secret_version: 1,
        provider_id: "openai".into(),
        model: "gpt-explicit".into(),
    };
    let key = SecretBytes::new(b"synthetic-upstream-only".to_vec());
    assert!(!client.ensure_route(&spec, &key).await.unwrap().reused);
    assert!(client.ensure_route(&spec, &key).await.unwrap().reused);
    let duplicate = state.lock().unwrap().deployments[0].clone();
    state.lock().unwrap().deployments.push(duplicate);
    assert!(matches!(
        client.ensure_route(&spec, &key).await,
        Err(LiteLlmError::PolicyConflict)
    ));
    server.abort();
}

#[test]
fn administration_endpoint_rejects_credentials_redirect_targets_and_cleartext_remote() {
    let key = SecretBytes::new(b"synthetic-key".to_vec());
    for endpoint in [
        "http://remote.example",
        "https://user:secret@example.com",
        "https://example.com/other",
        "https://example.com/?token=secret",
    ] {
        assert!(LiteLlmClient::new(endpoint, &key).is_err());
    }
}
