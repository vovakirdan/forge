//! Provider-safe MCP names map to fixed logical permissions, never arbitrary URLs.

use rmcp::model::{Tool, ToolAnnotations};
use serde_json::{Value, json};

pub(super) const TOOLS: [(&str, &str, &str); 6] = [
    (
        "board.list",
        "forge_list_board",
        "List bounded task summaries in this Project. Use next_after as the next after cursor.",
    ),
    (
        "task.read",
        "forge_read_task",
        "Read task intent and current state in this Project. No management or provider secrets are exposed.",
    ),
    (
        "artifact.submit",
        "forge_submit_artifact",
        "Attach structured evidence to your own active Run's Task. Reuse message_id only for identical retries; cite it in outcome submissions.",
    ),
    (
        "outcome.submit",
        "forge_submit_outcome",
        "Request a named outcome for your pinned stage; Core validates Pipeline requirements and evidence. Never invent a stage or outcome.",
    ),
    (
        "progress.report",
        "forge_report_progress",
        "Append a progress note for your own Run; this does not change task lifecycle.",
    ),
    (
        "human.request",
        "forge_request_human",
        "Ask a human decision for your own Task and explicitly request a graceful Run stop. No permission is granted by asking.",
    ),
];

pub(super) fn logical_tool(name: &str) -> Option<&'static str> {
    TOOLS
        .iter()
        .find(|(logical, _, _)| *logical == name)
        .map(|(logical, _, _)| *logical)
}

pub(super) fn tools() -> Vec<Tool> {
    TOOLS
        .iter()
        .map(|(logical, name, description)| {
            let schema = schema(logical);
            let mut tool = Tool::new(
                *name,
                *description,
                schema.as_object().expect("static object schema").clone(),
            );
            tool.annotations = Some(ToolAnnotations::from_raw(
                None,
                Some(matches!(*logical, "board.list" | "task.read")),
                Some(false),
                Some(true),
                Some(false),
            ));
            tool
        })
        .collect()
}

fn schema(logical: &str) -> Value {
    let uuid = json!({"type":"string","format":"uuid","description":"UUIDv7 identifier"});
    let text = json!({"type":"string"});
    let ids = json!({"type":"array","maxItems":64,"items":uuid});
    let (mut properties, mut required) = match logical {
        "board.list" => (
            json!({"limit":{"type":"integer","minimum":1,"maximum":100,"default":25},"after":uuid}),
            vec![],
        ),
        "task.read" => (
            json!({"task_id":uuid,"artifacts_after":uuid}),
            vec!["task_id"],
        ),
        "artifact.submit" => (
            json!({"artifact_kind":text,"title":{"type":"string","minLength":1,"maxLength":240},"metadata":{"type":"object"},"body":{}}),
            vec!["artifact_kind", "title", "body"],
        ),
        "outcome.submit" => (
            json!({"stage_id":text,"outcome":text,"artifact_ids":ids,"artifact_submission_message_ids":ids,"note":text,"cancellation_reason_key":text}),
            vec!["stage_id", "outcome"],
        ),
        "progress.report" => (
            json!({"note":{"type":"string","minLength":1,"maxLength":10000}}),
            vec!["note"],
        ),
        "human.request" => (
            json!({"question":{"type":"string","minLength":1,"maxLength":10000}}),
            vec!["question"],
        ),
        _ => unreachable!("static catalog"),
    };
    properties["message_id"] = uuid;
    if !matches!(logical, "board.list" | "task.read") {
        required.push("message_id");
    }
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_fixed_bounded_and_not_an_administrative_proxy() {
        let tools = tools();
        assert_eq!(tools.len(), 6);
        for tool in tools {
            assert_eq!(
                tool.input_schema.get("additionalProperties"),
                Some(&json!(false))
            );
        }
        for name in [
            "*",
            "task.cancel",
            "employee.create",
            "http.fetch",
            "shell.exec",
        ] {
            assert!(logical_tool(name).is_none());
        }
    }
}
