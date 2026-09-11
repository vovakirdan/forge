use std::collections::BTreeSet;

use forge_domain::{EmployeeId, ProjectId, knowledge::KnowledgePageStatus};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{MemorySearchDocument, agentmemory::ProjectionScope, canonical::sources_visible};
use crate::{CoreError, CoreService};

const MAX_RESULT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySearchMode {
    Indexed,
    CanonicalFallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySearchDegradation {
    NotConfigured,
    IndexUnavailable,
    RejectedProjection,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemorySearchHit {
    pub projection_id: Option<Uuid>,
    pub score: Option<f64>,
    pub document: MemorySearchDocument,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemorySearchResult {
    pub project_id: ProjectId,
    pub employee_id: Option<EmployeeId>,
    pub mode: MemorySearchMode,
    pub degradation: Option<MemorySearchDegradation>,
    /// Candidate scan/output bounds were reached; this is not exhaustive retrieval.
    pub truncated: bool,
    pub results: Vec<MemorySearchHit>,
}

impl CoreService {
    /// The transport supplies authenticated scope, never an index-provided ACL.
    /// Authority assembly is separate and must not use this top-k API.
    pub async fn query_search(
        &self,
        project_id: ProjectId,
        employee_id: Option<EmployeeId>,
        query: &str,
        limit: u32,
    ) -> Result<MemorySearchResult, CoreError> {
        if project_id.as_uuid().get_version_num() != 7 {
            return Err(CoreError::InvalidTransport {
                field: "memory_search.project_id",
                reason: "expected UUIDv7".into(),
            });
        }
        if query.trim().is_empty() || query.len() > 8192 || !(1..=100).contains(&limit) {
            return Err(CoreError::InvalidTransport {
                field: "memory_search",
                reason: "query must be 1 to 8192 bytes and limit 1 to 100".into(),
            });
        }
        if self.store.load_project(project_id).await?.is_none() {
            return Err(CoreError::NotFound {
                aggregate: "project",
            });
        }
        if let Some(employee) = employee_id {
            if employee.as_uuid().get_version_num() != 7 {
                return Err(CoreError::Forbidden);
            }
            if self
                .store
                .load_employee(employee)
                .await?
                .is_none_or(|value| value.employee.project_id() != project_id)
            {
                return Err(CoreError::Forbidden);
            }
        }
        let scope = ProjectionScope {
            project_id,
            employee_id: None,
        };
        let ranked = if let Some(client) = &self.agentmemory {
            match client.search(&scope, query, limit as usize).await {
                Ok(mut common) => {
                    if let Some(employee) = employee_id {
                        match client
                            .search(
                                &ProjectionScope {
                                    project_id,
                                    employee_id: Some(employee),
                                },
                                query,
                                limit as usize,
                            )
                            .await
                        {
                            Ok(personal) => common.extend(personal),
                            Err(_) => {
                                return self
                                    .canonical_search_fallback(
                                        project_id,
                                        employee_id,
                                        query,
                                        limit,
                                        MemorySearchDegradation::IndexUnavailable,
                                    )
                                    .await;
                            }
                        }
                    }
                    common.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));
                    common
                }
                Err(_) => {
                    return self
                        .canonical_search_fallback(
                            project_id,
                            employee_id,
                            query,
                            limit,
                            MemorySearchDegradation::IndexUnavailable,
                        )
                        .await;
                }
            }
        } else {
            return self
                .canonical_search_fallback(
                    project_id,
                    employee_id,
                    query,
                    limit,
                    MemorySearchDegradation::NotConfigured,
                )
                .await;
        };
        let mut result = MemorySearchResult {
            project_id,
            employee_id,
            mode: MemorySearchMode::Indexed,
            degradation: None,
            truncated: false,
            results: Vec::new(),
        };
        // Rehydrate after HTTP under the command serialization boundary. No network
        // call occurs while a canonical lock is held.
        let mut tx = self.store.begin().await?;
        tx.lock_project(project_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "project",
            })?;
        let mut seen = BTreeSet::new();
        let mut bytes = 0;
        for ranked in ranked {
            if !seen.insert(ranked.id) {
                continue;
            }
            let Some(operation) = self
                .store
                .load_knowledge_projection(project_id, ranked.id)
                .await?
            else {
                result.degradation = Some(MemorySearchDegradation::RejectedProjection);
                continue;
            };
            if !operation.index_ready {
                result.degradation = Some(MemorySearchDegradation::RejectedProjection);
                continue;
            }
            let Some(document) = self.projection_document(&mut tx, &operation).await? else {
                result.degradation = Some(MemorySearchDegradation::RejectedProjection);
                continue;
            };
            if !document.scope().visible_to(employee_id) {
                result.degradation = Some(MemorySearchDegradation::RejectedProjection);
                continue;
            }
            if !push_bounded(
                &mut result,
                MemorySearchHit {
                    projection_id: Some(ranked.id),
                    score: Some(ranked.score),
                    document,
                },
                &mut bytes,
                limit,
            ) {
                break;
            }
        }
        tx.commit().await?;
        Ok(result)
    }

    async fn canonical_search_fallback(
        &self,
        project_id: ProjectId,
        employee_id: Option<EmployeeId>,
        query: &str,
        limit: u32,
        reason: MemorySearchDegradation,
    ) -> Result<MemorySearchResult, CoreError> {
        let mut result = MemorySearchResult {
            project_id,
            employee_id,
            mode: MemorySearchMode::CanonicalFallback,
            degradation: Some(reason),
            truncated: false,
            results: Vec::new(),
        };
        let mut tx = self.store.begin().await?;
        tx.lock_project(project_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "project",
            })?;
        let pages = self
            .store
            .knowledge_pages_page(project_id, None, 100)
            .await?;
        let entries = self
            .store
            .visible_derived_memory(project_id, employee_id, None, 100)
            .await?;
        result.truncated = pages.len() == 100 || entries.len() == 100;
        // Explicit bounded substring fallback, not a replacement RAG/ranking engine.
        let query = query.to_lowercase();
        let candidates = pages
            .into_iter()
            .filter(|page| page.status == KnowledgePageStatus::Published)
            .map(MemorySearchDocument::KnowledgePage)
            .chain(entries.into_iter().map(MemorySearchDocument::DerivedMemory));
        let mut bytes = 0;
        for document in candidates {
            if !document.scope().visible_to(employee_id)
                || !(document.title().to_lowercase().contains(&query)
                    || document.markdown().to_lowercase().contains(&query))
                || !sources_visible(&mut tx, project_id, &document).await?
            {
                continue;
            }
            if !push_bounded(
                &mut result,
                MemorySearchHit {
                    projection_id: None,
                    score: None,
                    document,
                },
                &mut bytes,
                limit,
            ) {
                break;
            }
        }
        tx.commit().await?;
        Ok(result)
    }
}

fn push_bounded(
    result: &mut MemorySearchResult,
    hit: MemorySearchHit,
    bytes: &mut usize,
    limit: u32,
) -> bool {
    let size = serde_json::to_vec(&hit).map_or(MAX_RESULT_BYTES + 1, |value| value.len());
    if result.results.len() >= limit as usize || size > MAX_RESULT_BYTES.saturating_sub(*bytes) {
        result.truncated = true;
        return false;
    }
    *bytes += size;
    result.results.push(hit);
    true
}
