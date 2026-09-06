//! Typed named calls. Worker-provided identity fields are always rejected.

use super::{RunGateway, ToolRequest, invalid};
use crate::CoreError;
use forge_domain::TaskId;
use forge_protocol::supervisor::v1::{
    ArtifactSubmission, StageOutcomeSubmission, executor_submission::Payload,
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardArguments {
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    after: Option<Uuid>,
}
fn default_limit() -> u32 {
    25
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskArguments {
    task_id: Uuid,
    #[serde(default)]
    artifacts_after: Option<Uuid>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactArguments {
    artifact_kind: String,
    title: String,
    #[serde(default = "empty_object")]
    metadata: Value,
    body: Value,
}
fn empty_object() -> Value {
    json!({})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeArguments {
    stage_id: String,
    outcome: String,
    #[serde(default)]
    artifact_ids: Vec<Uuid>,
    #[serde(default)]
    artifact_submission_message_ids: Vec<Uuid>,
    #[serde(default)]
    note: String,
    #[serde(default)]
    cancellation_reason_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressArguments {
    note: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HumanArguments {
    question: String,
}

fn decode<T: DeserializeOwned>(arguments: Value) -> Result<T, CoreError> {
    serde_json::from_value(arguments).map_err(|_| invalid("arguments do not match the tool schema"))
}

impl RunGateway {
    pub(super) async fn execute(
        &self,
        request: ToolRequest,
        digest: [u8; 32],
    ) -> Result<Value, CoreError> {
        let message_id = request.message_id;
        let payload = match request.tool.as_str() {
            "board.list" => {
                let args: BoardArguments = decode(request.arguments)?;
                let tasks = self
                    .core
                    .store
                    .gateway_board(self.project_id, args.after, args.limit)
                    .await?;
                let next = tasks.last().and_then(|task| task.get("task_id")).cloned();
                return Ok(json!({"tasks":tasks,"next_after":next}));
            }
            "task.read" => {
                let args: TaskArguments = decode(request.arguments)?;
                let task = self
                    .core
                    .store
                    .load_task(TaskId::from(args.task_id))
                    .await?
                    .ok_or(CoreError::NotFound { aggregate: "task" })?
                    .task;
                if task.project_id() != self.project_id {
                    return Err(CoreError::NotFound { aggregate: "task" });
                }
                let artifacts = self
                    .core
                    .store
                    .gateway_task_artifacts(self.project_id, task.id(), args.artifacts_after)
                    .await?;
                let artifacts_next_after = artifacts
                    .last()
                    .and_then(|artifact| artifact.get("artifact_id"))
                    .cloned();
                // Deliberate allowlist: neither arbitrary properties, RunSpec,
                // work surface paths nor secret-bearing provider details leak.
                return Ok(
                    json!({"task_id":task.id(),"task_key":task.key(),"title":task.spec().title(),
                    "description":task.spec().description().chars().take(4096).collect::<String>(),
                    "description_truncated":task.spec().description().chars().count()>4096,
                    "definition_of_done":task.spec().definition_of_done().map(|text|text.chars().take(4096).collect::<String>()),
                    "definition_of_done_truncated":task.spec().definition_of_done().is_some_and(|text|text.chars().count()>4096),
                    "lifecycle":task.lifecycle(),"stage_id":task.current_stage_id(),"revision":task.revision().get(),
                    "artifacts":artifacts,"artifacts_next_after":artifacts_next_after}),
                );
            }
            "artifact.submit" => {
                let args: ArtifactArguments = decode(request.arguments)?;
                Payload::Artifact(ArtifactSubmission {
                    artifact_kind: args.artifact_kind,
                    title: args.title,
                    metadata_json: args.metadata.to_string(),
                    body_json: args.body.to_string(),
                })
            }
            "outcome.submit" => {
                let args: OutcomeArguments = decode(request.arguments)?;
                Payload::StageOutcome(StageOutcomeSubmission {
                    stage_id: args.stage_id,
                    outcome: args.outcome,
                    artifact_ids: args.artifact_ids.iter().map(Uuid::to_string).collect(),
                    artifact_submission_message_ids: args
                        .artifact_submission_message_ids
                        .iter()
                        .map(Uuid::to_string)
                        .collect(),
                    note: args.note,
                    cancellation_reason_key: args.cancellation_reason_key,
                })
            }
            "progress.report" | "human.request" => {
                let detail = if request.tool == "progress.report" {
                    decode::<ProgressArguments>(request.arguments)?.note
                } else {
                    decode::<HumanArguments>(request.arguments)?.question
                };
                if detail.trim().is_empty() || detail.chars().count() > 10_000 {
                    return Err(invalid("detail must contain 1..10000 characters"));
                }
                let hash = digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                return self
                    .core
                    .record_gateway_signal(self.scope, message_id, &request.tool, &detail, &hash)
                    .await;
            }
            _ => return Err(invalid("unknown tool")),
        };
        self.core
            .submit_gateway(self.scope, message_id, digest, payload)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_cannot_override_scope_or_add_hidden_mutation_arguments() {
        for field in [
            "project_id",
            "run_id",
            "employee_id",
            "fencing_token",
            "environment_epoch",
            "task_id",
        ] {
            let mut value = json!({"artifact_kind":"note","title":"Note","body":"test"});
            value[field] = json!(Uuid::now_v7());
            assert!(decode::<ArtifactArguments>(value).is_err());
        }
        assert!(decode::<ProgressArguments>(json!({"note":"x","status":"done"})).is_err());
    }
}
