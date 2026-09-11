use super::*;
use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{Response, StatusCode},
};
use std::{collections::VecDeque, convert::Infallible, sync::Arc};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle, time::sleep};

const TOKEN: &str = "synthetic-agentmemory-secret-not-a-real-key";

struct RequestRecord {
    path: String,
    authorization: String,
    body: Value,
}

struct Reply {
    status: StatusCode,
    bytes: Vec<u8>,
    delay: Duration,
    body_delay: Option<Duration>,
    chunked: bool,
    location: Option<String>,
}

impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: StatusCode::OK,
            bytes: serde_json::to_vec(&value).unwrap(),
            delay: Duration::ZERO,
            body_delay: None,
            chunked: false,
            location: None,
        }
    }
}

#[derive(Default)]
struct ServerState {
    replies: VecDeque<Reply>,
    requests: Vec<RequestRecord>,
}

struct Server {
    endpoint: String,
    state: Arc<Mutex<ServerState>>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(ServerState {
            replies: replies.into(),
            requests: Vec::new(),
        }));
        let app = Router::new()
            .fallback(handle)
            .with_state(Arc::clone(&state));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            endpoint,
            state,
            task,
        }
    }

    fn client(&self) -> AgentMemoryClient {
        AgentMemoryClient::new(
            &self.endpoint,
            SecretBytes::new(TOKEN.as_bytes().to_vec()),
            Duration::from_secs(2),
        )
        .unwrap()
    }
}

async fn handle(State(state): State<Arc<Mutex<ServerState>>>, request: Request) -> Response<Body> {
    let path = request.uri().path().to_owned();
    let authorization = request
        .headers()
        .get(AUTHORIZATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let body = to_bytes(request.into_body(), MAX_REQUEST_BYTES)
        .await
        .unwrap();
    let reply = {
        let mut state = state.lock().await;
        state.requests.push(RequestRecord {
            path,
            authorization,
            body: serde_json::from_slice(&body).unwrap(),
        });
        state.replies.pop_front().unwrap_or_else(|| {
            let mut reply = Reply::json(json!({}));
            reply.status = StatusCode::INTERNAL_SERVER_ERROR;
            reply
        })
    };
    sleep(reply.delay).await;
    let mut response = Response::builder()
        .status(reply.status)
        .header("content-type", "application/json");
    if let Some(location) = reply.location {
        response = response.header("location", location);
    }
    let body = if let Some(delay) = reply.body_delay {
        let bytes = Bytes::from(reply.bytes);
        Body::from_stream(async_stream::stream! {
            yield Ok::<_, Infallible>(bytes.slice(..1));
            sleep(delay).await;
            yield Ok::<_, Infallible>(bytes.slice(1..));
        })
    } else if reply.chunked {
        let chunks: Vec<_> = reply
            .bytes
            .chunks(4096)
            .map(|bytes| Ok::<_, Infallible>(Bytes::copy_from_slice(bytes)))
            .collect();
        Body::from_stream(futures_util::stream::iter(chunks))
    } else {
        Body::from(reply.bytes)
    };
    response.body(body).unwrap()
}

fn indexed() -> Value {
    json!({"success":true,"storage":"complete","strategy":"merge","stats":{"memories":1},
        "indexing":{"version":1,"mode":"strict","expected":1,"bm25Indexed":1,"vectorIndexed":1,"persisted":true}})
}

fn record() -> ProjectionRecord {
    let created_at = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    ProjectionRecord {
        id: Uuid::now_v7(),
        scope: ProjectionScope {
            project_id: ProjectId::new(),
            employee_id: Some(EmployeeId::new()),
        },
        revision: 3,
        title: "Canonical derived summary".into(),
        content: "Unicode content: проверено ✓".into(),
        created_at,
        updated_at: created_at + time::Duration::minutes(1),
    }
}

#[tokio::test]
async fn strict_merge_replays_the_same_projection_id_and_exact_memory_envelope() {
    let server = Server::new(vec![Reply::json(indexed()), Reply::json(indexed())]).await;
    let client = server.client();
    let record = record();
    client.upsert(&record).await.unwrap();
    client.upsert(&record).await.unwrap();
    let state = server.state.lock().await;
    assert_eq!(state.requests.len(), 2);
    assert_eq!(state.requests[0].body, state.requests[1].body);
    let request = &state.requests[0];
    assert_eq!(request.path, "/agentmemory/import");
    assert_eq!(request.authorization, format!("Bearer {TOKEN}"));
    assert_eq!(request.body["strategy"], "merge");
    assert_eq!(request.body["strictIndexing"], true);
    let export = &request.body["exportData"];
    assert_eq!(export["version"], "0.9.29");
    assert_eq!(export["sessions"], json!([]));
    assert_eq!(export["observations"], json!({}));
    assert_eq!(export["summaries"], json!([]));
    assert_eq!(export["memories"].as_array().unwrap().len(), 1);
    assert_eq!(
        export["memories"][0],
        json!({
            "id":record.id,"createdAt":record.created_at.format(&Rfc3339).unwrap(),
            "updatedAt":record.updated_at.format(&Rfc3339).unwrap(),"type":"fact",
            "title":record.title,"content":record.content,"concepts":[],"files":[],"sessionIds":[],
            "strength":7,"version":3,"isLatest":true,"project":record.scope.project_id,
            "agentId":record.scope.employee_id.unwrap(),
        })
    );
}

#[tokio::test]
async fn legacy_partial_wrong_version_and_nonpersisted_index_acknowledgements_fail_closed() {
    let mut invalid = vec![json!({"success":true,"stats":{"memories":1}}), json!({})];
    for (field, value) in [
        ("version", json!(2)),
        ("mode", json!("best_effort")),
        ("expected", json!(0)),
        ("expected", json!(2)),
        ("bm25Indexed", json!(0)),
        ("vectorIndexed", json!(0)),
        ("vectorIndexed", json!(2)),
        ("persisted", json!(false)),
    ] {
        let mut ack = indexed();
        ack["indexing"][field] = value;
        invalid.push(ack);
    }
    for (field, value) in [("success", json!(false)), ("storage", json!("partial"))] {
        let mut ack = indexed();
        ack[field] = value;
        invalid.push(ack);
    }
    let mut misplaced = indexed();
    misplaced.as_object_mut().unwrap().remove("storage");
    misplaced["indexing"]["storage"] = json!("complete");
    invalid.push(misplaced);
    let server = Server::new(invalid.iter().cloned().map(Reply::json).collect()).await;
    let client = server.client();
    let record = record();
    for _ in invalid {
        assert_eq!(
            client.upsert(&record).await,
            Err(AgentMemoryError::IncompleteIndexing)
        );
    }
}

#[tokio::test]
async fn project_scope_has_no_employee_hint_and_forget_is_idempotent() {
    let server = Server::new(vec![
        Reply::json(indexed()),
        Reply::json(json!({"success":true,"deleted":1})),
        Reply::json(json!({"success":true,"deleted":0})),
    ])
    .await;
    let client = server.client();
    let mut record = record();
    record.scope.employee_id = None;
    client.upsert(&record).await.unwrap();
    client.forget(record.id).await.unwrap();
    client.forget(record.id).await.unwrap();
    let state = server.state.lock().await;
    assert!(
        state.requests[0].body["exportData"]["memories"][0]
            .get("agentId")
            .is_none()
    );
    for request in &state.requests[1..] {
        assert_eq!(request.path, "/agentmemory/forget");
        assert_eq!(request.body, json!({"memoryId":record.id}));
    }
}

#[tokio::test]
async fn search_returns_ranked_ids_only_and_preserves_scope_hints_without_trusting_content() {
    let first = Uuid::now_v7();
    let second = Uuid::now_v7();
    let server = Server::new(vec![Reply::json(json!({"format":"compact","results":[
        {"obsId":first,"score":1.5,"title":"UNTRUSTED","content":"NOT CANONICAL","project":"foreign"},
        {"obsId":second,"score":0.2,"narrative":"DO NOT INJECT"}],"truncated":false}))]).await;
    let record = record();
    assert_eq!(
        server
            .client()
            .search(&record.scope, "requested terms", 2)
            .await
            .unwrap(),
        vec![
            RankedObservationId {
                id: first,
                score: 1.5
            },
            RankedObservationId {
                id: second,
                score: 0.2
            }
        ]
    );
    let state = server.state.lock().await;
    assert_eq!(state.requests[0].path, "/agentmemory/search");
    assert_eq!(
        state.requests[0].body,
        json!({"query":"requested terms","limit":2,"format":"compact","project":record.scope.project_id,"agentId":record.scope.employee_id.unwrap()})
    );
}

#[tokio::test]
async fn malformed_ids_scores_duplicate_ids_and_unbounded_search_result_counts_are_rejected() {
    let id = Uuid::now_v7();
    let mut responses = Vec::new();
    for value in [
        json!({"obsId":"not-a-uuid","score":1}),
        json!({"obsId":Uuid::nil(),"score":1}),
        json!({"obsId":id.to_string().to_uppercase(),"score":1}),
        json!({"obsId":id,"score":"1"}),
        json!({"obsId":id,"score":null}),
        json!({"obsId":id}),
    ] {
        responses.push(json!({"format":"compact","results":[value]}));
    }
    responses.push(json!({"format":"full","results":[]}));
    responses.push(
        json!({"format":"compact","results":[{"obsId":id,"score":1},{"obsId":id,"score":0.5}]}),
    );
    let count = responses.len();
    let server = Server::new(responses.into_iter().map(Reply::json).collect()).await;
    for _ in 0..count {
        assert_eq!(
            server.client().search(&record().scope, "test", 2).await,
            Err(AgentMemoryError::InvalidResponse)
        );
    }
    let server = Server::new(vec![Reply::json(json!({"format":"compact","results":[
        {"obsId":Uuid::now_v7(),"score":1},{"obsId":Uuid::now_v7(),"score":0.5}]}))])
    .await;
    assert_eq!(
        server.client().search(&record().scope, "test", 1).await,
        Err(AgentMemoryError::InvalidResponse)
    );
}

#[tokio::test]
async fn oversized_fixed_and_chunked_responses_are_bounded() {
    for chunked in [false, true] {
        let mut reply = Reply::json(json!({}));
        reply.bytes = vec![b' '; MAX_RESPONSE_BYTES + 1];
        reply.chunked = chunked;
        let server = Server::new(vec![reply]).await;
        assert_eq!(
            server.client().upsert(&record()).await,
            Err(AgentMemoryError::ResponseTooLarge)
        );
    }
}

#[tokio::test]
async fn timeouts_malformed_json_and_http_failures_have_redacted_errors() {
    let mut delayed = Reply::json(indexed());
    delayed.delay = Duration::from_millis(200);
    let server = Server::new(vec![delayed]).await;
    let client = AgentMemoryClient::new(
        &server.endpoint,
        SecretBytes::new(TOKEN.as_bytes().to_vec()),
        Duration::from_millis(20),
    )
    .unwrap();
    assert_eq!(
        client.upsert(&record()).await,
        Err(AgentMemoryError::Timeout)
    );
    let mut failed = Reply::json(json!({"error":TOKEN}));
    failed.status = StatusCode::SERVICE_UNAVAILABLE;
    let mut malformed = Reply::json(json!({}));
    malformed.bytes = TOKEN.as_bytes().to_vec();
    let server = Server::new(vec![failed, malformed]).await;
    let client = server.client();
    for expected in [
        AgentMemoryError::Rejected,
        AgentMemoryError::IncompleteIndexing,
    ] {
        let error = client.upsert(&record()).await.unwrap_err();
        assert_eq!(error, expected);
        assert!(!format!("{error:?} {error} {client:?}").contains(TOKEN));
        assert!(!format!("{error:?} {error} {client:?}").contains(&server.endpoint));
    }
}

#[tokio::test]
async fn redirects_never_forward_the_service_credential() {
    let destination = Server::new(vec![Reply::json(indexed())]).await;
    let mut redirect = Reply::json(json!({}));
    redirect.status = StatusCode::TEMPORARY_REDIRECT;
    redirect.location = Some(format!("{}/agentmemory/import", destination.endpoint));
    let origin = Server::new(vec![redirect]).await;
    assert_eq!(
        origin.client().upsert(&record()).await,
        Err(AgentMemoryError::Rejected)
    );
    assert!(destination.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn response_body_read_obeys_the_total_request_deadline() {
    let mut reply = Reply::json(indexed());
    reply.body_delay = Some(Duration::from_millis(200));
    let server = Server::new(vec![reply]).await;
    let client = AgentMemoryClient::new(
        &server.endpoint,
        SecretBytes::new(TOKEN.as_bytes().to_vec()),
        Duration::from_millis(20),
    )
    .unwrap();
    assert_eq!(
        client.upsert(&record()).await,
        Err(AgentMemoryError::Timeout)
    );
}

#[test]
fn client_rejects_external_dns_userinfo_queries_non_http_and_missing_credentials() {
    for endpoint in [
        "https://127.0.0.1:3111",
        "http://example.com:3111",
        "http://localhost:3111",
        "http://192.168.1.1:3111",
        "http://u:p@127.0.0.1:3111",
        "http://127.0.0.1:3111/?secret=x",
        "http://127.0.0.1:3111/#x",
        "http://127.0.0.1:3111/other",
    ] {
        assert!(matches!(
            AgentMemoryClient::new(
                endpoint,
                SecretBytes::new(TOKEN.as_bytes().to_vec()),
                Duration::from_secs(1)
            ),
            Err(AgentMemoryError::InvalidConfiguration)
        ));
    }
    for secret in [
        b"".as_slice(),
        b"white space".as_slice(),
        b"line\nbreak".as_slice(),
    ] {
        assert!(
            AgentMemoryClient::new(
                "http://127.0.0.1:3111",
                SecretBytes::new(secret.to_vec()),
                Duration::from_secs(1)
            )
            .is_err()
        );
    }
    for endpoint in [
        "http://127.0.0.1:3111",
        "http://127.0.0.1:3111/agentmemory",
        "http://[::1]:3111/agentmemory/",
    ] {
        assert!(
            AgentMemoryClient::new(
                endpoint,
                SecretBytes::new(TOKEN.as_bytes().to_vec()),
                Duration::from_secs(1)
            )
            .is_ok()
        );
    }
}

#[tokio::test]
async fn invalid_local_inputs_never_reach_the_memory_service() {
    let server = Server::new(Vec::new()).await;
    let client = server.client();
    let mut item = record();
    item.id = Uuid::nil();
    assert_eq!(
        client.upsert(&item).await,
        Err(AgentMemoryError::InvalidInput)
    );
    item = record();
    item.revision = 0;
    assert_eq!(
        client.upsert(&item).await,
        Err(AgentMemoryError::InvalidInput)
    );
    item = record();
    item.content = "x".repeat(MAX_CONTENT_BYTES + 1);
    assert_eq!(
        client.upsert(&item).await,
        Err(AgentMemoryError::InvalidInput)
    );
    item = record();
    item.updated_at = item.created_at - time::Duration::seconds(1);
    assert_eq!(
        client.upsert(&item).await,
        Err(AgentMemoryError::InvalidInput)
    );
    for (query, limit) in [("", 1), ("test", 0), ("test", MAX_SEARCH_LIMIT + 1)] {
        assert_eq!(
            client.search(&record().scope, query, limit).await,
            Err(AgentMemoryError::InvalidInput)
        );
    }
    assert_eq!(
        client.forget(Uuid::nil()).await,
        Err(AgentMemoryError::InvalidInput)
    );
    assert!(server.state.lock().await.requests.is_empty());
}
