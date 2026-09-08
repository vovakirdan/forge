//! Provider-safe MCP names map to fixed logical permissions, never arbitrary URLs.

use rmcp::model::{Tool, ToolAnnotations};
use serde_json::{Value, json};

pub(super) const TOOLS: [(&str, &str, &str); 15] = [
    (
        "escalation.raise",
        "forge_raise_escalation",
        "Pause your exact Task or conversation and request a routed decision. Action approval always goes to a human; asking grants no extra authority.",
    ),
    (
        "resolution.read",
        "forge_read_resolution",
        "Read your exact assigned question and allowed answer keys. Context grants no Task execution authority.",
    ),
    (
        "resolution.submit",
        "forge_submit_resolution",
        "Answer your assigned question. A recommendation never completes a Task stage or approves a dangerous action.",
    ),
    (
        "resolution.decline",
        "forge_decline_resolution",
        "Decline this resolver assignment; management routes the question after physical quiescence.",
    ),
    (
        "finding.report",
        "forge_report_finding",
        "Report a separate observation with optional Artifact evidence from your Task. This never creates a Task or changes the current work.",
    ),
    (
        "communication.complete",
        "forge_complete_communication",
        "Complete your assigned conversation after its required acknowledgement or answer. This never changes a Task or proves sandbox quiescence.",
    ),
    (
        "inbox.list",
        "forge_list_instructions",
        "Read messages addressed to your exact Task stage visit. Reading is not acknowledgement. Check before submitting an outcome.",
    ),
    (
        "inbox.acknowledge",
        "forge_acknowledge_instruction",
        "Explicitly acknowledge an addressed message. This records your claim, not semantic verification.",
    ),
    (
        "inbox.reply",
        "forge_reply_instruction",
        "Append your canonical reply to an addressed message and satisfy its answer requirement. Reuse message_id for exact retries.",
    ),
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
                Some(matches!(
                    *logical,
                    "board.list" | "task.read" | "inbox.list" | "resolution.read"
                )),
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
        "escalation.raise" => (
            json!({"question":{"type":"string","minLength":1,"maxLength":20000},"category":{"type":"string","enum":["action_approval","clarification","scope_or_policy_conflict","stale_or_invalid_task","technical_decision","blocked"]},"route_key":{"type":"string"}}),
            vec!["question", "category"],
        ),
        "resolution.read" => (json!({}), vec![]),
        "resolution.submit" => (
            json!({"disposition":{"type":"string","enum":["continue_stage","needs_management_change","forward_to_human"]},"summary":{"type":"string","minLength":1,"maxLength":20000},"recommended_outcome_key":text}),
            vec!["disposition", "summary"],
        ),
        "resolution.decline" => (
            json!({"reason":{"type":"string","minLength":1,"maxLength":10000}}),
            vec!["reason"],
        ),
        "finding.report" => (
            json!({"description":{"type":"string","minLength":1,"maxLength":50000},"severity":{"type":"string","minLength":1,"maxLength":64},"evidence":ids}),
            vec!["description", "severity"],
        ),
        "communication.complete" => (json!({}), vec![]),
        "inbox.list" => (
            json!({"after":uuid,"limit":{"type":"integer","minimum":1,"maximum":100,"default":25}}),
            vec![],
        ),
        "inbox.acknowledge" => (json!({"target_message_id":uuid}), vec!["target_message_id"]),
        "inbox.reply" => (
            json!({"target_message_id":uuid,"body":text}),
            vec!["target_message_id", "body"],
        ),
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
            json!({"stage_id":text,"outcome":text,"artifact_ids":ids,"artifact_submission_message_ids":ids,"note":text,"cancellation_reason_key":text,"candidate_commit":{"type":"string","pattern":"^([0-9a-fA-F]{40}|[0-9a-fA-F]{64})$","description":"Explicit full HEAD; Git outcome acceptance waits for writer quiescence and inspection"}}),
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
    if !matches!(
        logical,
        "board.list" | "task.read" | "inbox.list" | "resolution.read"
    ) {
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
        assert_eq!(tools.len(), 15);
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
