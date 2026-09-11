//! Employee identity comes from the fenced Run, never from tool arguments.
use super::{RunGateway, ToolRequest, invalid};
use crate::CoreError;
use forge_domain::knowledge::{KnowledgePageStatus, MemoryScope};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArguments {
    query: String,
    #[serde(default = "default_limit")]
    limit: u32,
}
fn default_limit() -> u32 {
    10
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArguments {
    kind: DocumentKind,
    id: Uuid,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum DocumentKind {
    KnowledgePage,
    DerivedMemory,
}

impl RunGateway {
    pub(super) async fn execute_memory(
        &self,
        request: ToolRequest,
        digest: [u8; 32],
    ) -> Result<Value, CoreError> {
        let mut tx = self.core.store.begin().await?;
        let run = tx
            .validate_gateway_scope(&self.scope)
            .await?
            .ok_or_else(|| invalid("memory viewer Run is no longer active"))?;
        let employee = run.require_employee_id()?;
        if run.assignment.system_job().is_some() {
            return Err(CoreError::Forbidden);
        }
        match request.tool.as_str() {
            "memory.search" => {
                let args: SearchArguments = serde_json::from_value(request.arguments)
                    .map_err(|_| invalid("invalid search arguments"))?;
                tx.commit().await?;
                let result = self
                    .core
                    .query_search(self.project_id, Some(employee), &args.query, args.limit)
                    .await?;
                self.authorize().await?;
                serde_json::to_value(result).map_err(|_| invalid("invalid search result"))
            }
            "memory.refresh" => {
                if request.arguments != json!({}) {
                    return Err(invalid("refresh has no arguments"));
                }
                tx.commit().await?;
                let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
                self.core
                    .refresh_run_knowledge(self.scope, request.message_id, &hash)
                    .await
            }
            "memory.read" => {
                let args: ReadArguments = serde_json::from_value(request.arguments)
                    .map_err(|_| invalid("invalid memory read arguments"))?;
                if args.id.get_version_num() != 7 {
                    return Err(invalid("memory identity must be UUIDv7"));
                }
                let value = match args.kind {
                    DocumentKind::KnowledgePage => {
                        let page = self
                            .core
                            .store
                            .load_knowledge_page(self.project_id, args.id)
                            .await?
                            .filter(|p| p.status == KnowledgePageStatus::Published)
                            .ok_or(CoreError::NotFound {
                                aggregate: "knowledge_page",
                            })?;
                        tx.validate_knowledge_sources(
                            self.project_id,
                            MemoryScope::Project,
                            &page.content.source_refs,
                        )
                        .await?;
                        json!({"kind":"knowledge_page","record":page})
                    }
                    DocumentKind::DerivedMemory => {
                        let entry = self
                            .core
                            .store
                            .load_derived_memory_entry(self.project_id, args.id)
                            .await?
                            .filter(|e| {
                                !e.withdrawn && e.subject.scope().visible_to(Some(employee))
                            })
                            .ok_or(CoreError::NotFound {
                                aggregate: "derived_memory",
                            })?;
                        tx.validate_knowledge_sources(
                            self.project_id,
                            entry.subject.scope(),
                            &entry.source_refs,
                        )
                        .await?;
                        json!({"kind":"derived_memory","record":entry})
                    }
                };
                tx.commit().await?;
                Ok(value)
            }
            _ => Err(invalid("unsupported memory operation")),
        }
    }
}
