//! Bounded local AgentMemory transport. Nothing returned here authorizes visibility
//! or supplies canonical context text; Core must revalidate every observation ID.

use std::{collections::BTreeSet, fmt, net::IpAddr, time::Duration};

use forge_domain::{EmployeeId, ProjectId};
use forge_provider_common::SecretBytes;
use futures_util::StreamExt;
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::Deserialize;
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;
use zeroize::Zeroizing;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_CONTENT_BYTES: usize = 256 * 1024;
const MAX_SEARCH_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProjectionScope {
    pub project_id: ProjectId,
    pub employee_id: Option<EmployeeId>,
}

/// The ledger supplies a stable ID for one immutable entry revision and content hash.
/// Deliberately not Debug: projection text can contain private project information.
pub(crate) struct ProjectionRecord {
    pub id: Uuid,
    pub scope: ProjectionScope,
    pub revision: u64,
    pub title: String,
    pub content: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RankedObservationId {
    pub id: Uuid,
    pub score: f64,
}

#[derive(Clone)]
pub(crate) struct AgentMemoryClient {
    client: Client,
    endpoint: Url,
}

impl fmt::Debug for AgentMemoryClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AgentMemoryClient([REDACTED])")
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum AgentMemoryError {
    #[error("AgentMemory local client configuration is invalid")]
    InvalidConfiguration,
    #[error("AgentMemory projection input is invalid")]
    InvalidInput,
    #[error("AgentMemory request timed out")]
    Timeout,
    #[error("AgentMemory transport failed")]
    Transport,
    #[error("AgentMemory rejected the request")]
    Rejected,
    #[error("AgentMemory response exceeded its size limit")]
    ResponseTooLarge,
    #[error("AgentMemory response contract is invalid")]
    InvalidResponse,
    #[error("AgentMemory did not acknowledge complete strict indexing")]
    IncompleteIndexing,
}

impl AgentMemoryClient {
    /// Endpoint is an HTTP loopback IP origin, optionally ending in /agentmemory.
    /// Credentials are explicit and never forwarded through redirects or proxies.
    pub(crate) fn new(
        endpoint: &str,
        credential: SecretBytes,
        timeout: Duration,
    ) -> Result<Self, AgentMemoryError> {
        let mut endpoint =
            Url::parse(endpoint).map_err(|_| AgentMemoryError::InvalidConfiguration)?;
        if endpoint.scheme() != "http"
            || endpoint
                .host_str()
                .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
                .is_none_or(|ip| !ip.is_loopback())
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !matches!(endpoint.path(), "/" | "/agentmemory" | "/agentmemory/")
            || timeout.is_zero()
            || timeout > Duration::from_secs(120)
            || credential.expose().is_empty()
            || credential.expose().len() > 4096
            || !credential
                .expose()
                .iter()
                .all(|byte| byte.is_ascii_graphic())
        {
            return Err(AgentMemoryError::InvalidConfiguration);
        }
        endpoint.set_path("/agentmemory/");
        let mut bearer = Zeroizing::new(b"Bearer ".to_vec());
        bearer.extend_from_slice(credential.expose());
        let mut authorization =
            HeaderValue::from_bytes(&bearer).map_err(|_| AgentMemoryError::InvalidConfiguration)?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        let client = Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(5)))
            .build()
            .map_err(|_| AgentMemoryError::InvalidConfiguration)?;
        Ok(Self { client, endpoint })
    }

    pub(crate) async fn upsert(&self, record: &ProjectionRecord) -> Result<(), AgentMemoryError> {
        validate_scope(record.scope)?;
        if record.id.get_version_num() != 7
            || record.revision == 0
            || record.revision > 9_007_199_254_740_991
            || record.title.trim().is_empty()
            || record.title.len() > 4096
            || record.content.trim().is_empty()
            || record.content.len() > MAX_CONTENT_BYTES
            || record.updated_at < record.created_at
        {
            return Err(AgentMemoryError::InvalidInput);
        }
        let created_at = record
            .created_at
            .format(&Rfc3339)
            .map_err(|_| AgentMemoryError::InvalidInput)?;
        let updated_at = record
            .updated_at
            .format(&Rfc3339)
            .map_err(|_| AgentMemoryError::InvalidInput)?;
        let mut memory = json!({
            "id": record.id, "createdAt": created_at, "updatedAt": updated_at,
            "type": "fact", "title": record.title, "content": record.content,
            "concepts": [], "files": [], "sessionIds": [], "strength": 7,
            "version": record.revision, "isLatest": true, "project": record.scope.project_id,
        });
        if let Some(employee_id) = record.scope.employee_id {
            memory["agentId"] = json!(employee_id);
        }
        let body = json!({
            "strategy": "merge", "strictIndexing": true,
            "exportData": {"version": "0.9.29", "exportedAt": updated_at,
                "sessions": [], "observations": {}, "summaries": [], "memories": [memory]},
        });
        let response = self.post("import", body).await?;
        let ack: ImportAcknowledgement =
            serde_json::from_slice(&response).map_err(|_| AgentMemoryError::IncompleteIndexing)?;
        if !ack.success
            || ack.storage != "complete"
            || ack.indexing.version != 1
            || ack.indexing.mode != "strict"
            || ack.indexing.expected != 1
            || ack.indexing.bm25_indexed != 1
            || ack.indexing.vector_indexed != 1
            || !ack.indexing.persisted
        {
            return Err(AgentMemoryError::IncompleteIndexing);
        }
        Ok(())
    }

    /// AgentMemory treats an already missing memory ID as a successful zero-delete.
    pub(crate) async fn forget(&self, id: Uuid) -> Result<(), AgentMemoryError> {
        if id.get_version_num() != 7 {
            return Err(AgentMemoryError::InvalidInput);
        }
        let response = self.post("forget", json!({"memoryId": id})).await?;
        let ack: ForgetAcknowledgement =
            serde_json::from_slice(&response).map_err(|_| AgentMemoryError::InvalidResponse)?;
        if !ack.success || ack.deleted > 1 {
            return Err(AgentMemoryError::InvalidResponse);
        }
        Ok(())
    }

    /// Scope is a retrieval hint, never authorization. Only IDs/scores cross this boundary.
    pub(crate) async fn search(
        &self,
        scope: &ProjectionScope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RankedObservationId>, AgentMemoryError> {
        validate_scope(*scope)?;
        if query.trim().is_empty() || query.len() > 8192 || !(1..=MAX_SEARCH_LIMIT).contains(&limit)
        {
            return Err(AgentMemoryError::InvalidInput);
        }
        let mut body =
            json!({"query":query,"limit":limit,"format":"compact","project":scope.project_id});
        if let Some(employee_id) = scope.employee_id {
            body["agentId"] = json!(employee_id);
        }
        let response = self.post("search", body).await?;
        let response: SearchResponse =
            serde_json::from_slice(&response).map_err(|_| AgentMemoryError::InvalidResponse)?;
        if response.format != "compact" || response.results.len() > limit {
            return Err(AgentMemoryError::InvalidResponse);
        }
        let mut ids = BTreeSet::new();
        response
            .results
            .into_iter()
            .map(|result| {
                let id = Uuid::parse_str(&result.obs_id)
                    .map_err(|_| AgentMemoryError::InvalidResponse)?;
                if id.get_version_num() != 7
                    || id.to_string() != result.obs_id
                    || !result.score.is_finite()
                    || !ids.insert(id)
                {
                    return Err(AgentMemoryError::InvalidResponse);
                }
                Ok(RankedObservationId {
                    id,
                    score: result.score,
                })
            })
            .collect()
    }

    async fn post(&self, operation: &str, body: Value) -> Result<Vec<u8>, AgentMemoryError> {
        let body = serde_json::to_vec(&body).map_err(|_| AgentMemoryError::InvalidInput)?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(AgentMemoryError::InvalidInput);
        }
        let url = self
            .endpoint
            .join(operation)
            .map_err(|_| AgentMemoryError::InvalidConfiguration)?;
        let response = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(transport_error)?;
        if !response.status().is_success() {
            return Err(AgentMemoryError::Rejected);
        }
        if response
            .content_length()
            .is_some_and(|len| len > MAX_RESPONSE_BYTES as u64)
        {
            return Err(AgentMemoryError::ResponseTooLarge);
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(transport_error)?;
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(AgentMemoryError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

fn validate_scope(scope: ProjectionScope) -> Result<(), AgentMemoryError> {
    if scope.project_id.as_uuid().get_version_num() != 7
        || scope
            .employee_id
            .is_some_and(|id| id.as_uuid().get_version_num() != 7)
    {
        return Err(AgentMemoryError::InvalidInput);
    }
    Ok(())
}

fn transport_error(error: reqwest::Error) -> AgentMemoryError {
    if error.is_timeout() {
        AgentMemoryError::Timeout
    } else {
        AgentMemoryError::Transport
    }
}

#[derive(Deserialize)]
struct ImportAcknowledgement {
    success: bool,
    storage: String,
    indexing: IndexingAcknowledgement,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexingAcknowledgement {
    version: u32,
    mode: String,
    expected: usize,
    bm25_indexed: usize,
    vector_indexed: usize,
    persisted: bool,
}

#[derive(Deserialize)]
struct ForgetAcknowledgement {
    success: bool,
    deleted: usize,
}

#[derive(Deserialize)]
struct SearchResponse {
    format: String,
    results: Vec<SearchResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchResult {
    obs_id: String,
    score: f64,
}

#[cfg(test)]
mod tests;
