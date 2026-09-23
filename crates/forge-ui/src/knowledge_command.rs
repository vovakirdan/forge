//! Exact owner command shapes for canonical Knowledge pages.

use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;

use crate::{
    command::{CommandTarget, uuid_v7},
    http::ApiError,
};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    project_id: String,
    expected_revision: u64,
    payload: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageIdentity {
    page_id: String,
    expected_page_revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorPayload {
    page_id: String,
    expected_page_revision: u64,
    kind: PageKind,
    title: String,
    markdown: String,
    #[serde(default)]
    source_refs: Vec<SourceRef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SupersedePayload {
    page_id: String,
    expected_page_revision: u64,
    title: String,
    markdown: String,
    #[serde(default)]
    source_refs: Vec<SourceRef>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum PageKind {
    Introduction,
    Architecture,
    Guide,
    Policy,
    Decision,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SourceRef {
    Artifact { artifact_id: String },
    Event { event_id: String },
    TaskHandoff { handoff_id: String },
    KnowledgePage { page_id: String, revision: u64 },
}

fn sources_valid(sources: &[SourceRef]) -> bool {
    sources.len() <= 128
        && sources.iter().all(|source| match source {
            SourceRef::Artifact { artifact_id } => uuid_v7(artifact_id),
            SourceRef::Event { event_id } => uuid_v7(event_id),
            SourceRef::TaskHandoff { handoff_id } => uuid_v7(handoff_id),
            SourceRef::KnowledgePage { page_id, revision } => {
                uuid_v7(page_id) && (1..MAX_SAFE_INTEGER).contains(revision)
            }
        })
}

fn content_valid(title: &str, markdown: &str, sources: &[SourceRef]) -> bool {
    !title.trim().is_empty()
        && title.len() <= 200
        && !markdown.trim().is_empty()
        && markdown.len() <= 65_536
        && sources_valid(sources)
}

pub(crate) struct KnowledgeCommand {
    expected_revision: u64,
    page_id: String,
}

impl KnowledgeCommand {
    pub(crate) fn parse(bytes: &[u8], target: CommandTarget) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id)
            || !(1..MAX_SAFE_INTEGER).contains(&envelope.expected_revision)
        {
            return Err(ApiError::BadRequest);
        }
        let (page_id, page_revision) = match target {
            CommandTarget::AuthorKnowledgePage => {
                let payload: AuthorPayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                let _ = payload.kind;
                if !content_valid(&payload.title, &payload.markdown, &payload.source_refs) {
                    return Err(ApiError::BadRequest);
                }
                (payload.page_id, payload.expected_page_revision)
            }
            CommandTarget::SupersedeKnowledgePage => {
                let payload: SupersedePayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !content_valid(&payload.title, &payload.markdown, &payload.source_refs) {
                    return Err(ApiError::BadRequest);
                }
                (payload.page_id, payload.expected_page_revision)
            }
            CommandTarget::PublishKnowledgePage | CommandTarget::WithdrawKnowledgePage => {
                let payload: PageIdentity =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                (payload.page_id, payload.expected_page_revision)
            }
            _ => return Err(ApiError::BadRequest),
        };
        if !uuid_v7(&page_id)
            || page_revision >= MAX_SAFE_INTEGER
            || (!matches!(target, CommandTarget::AuthorKnowledgePage) && page_revision == 0)
        {
            return Err(ApiError::BadRequest);
        }
        Ok(Self {
            expected_revision: envelope.expected_revision,
            page_id,
        })
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        if !uuid_v7(&receipt.command_id)
            || receipt.project_revision != self.expected_revision + 1
            || receipt.event_ids.is_empty()
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || !receipt.resource.as_ref().is_some_and(|resource| {
                resource.kind == "knowledge_page"
                    && uuid_v7(&resource.id)
                    && resource.id.eq_ignore_ascii_case(&self.page_id)
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
