//! Bounded, replay-safe provider diagnostics; no inferred stage success or cost.

use crate::{
    CoreError, CoreService,
    credentials::credential_error,
    event::{event, event_payload},
};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, Timestamp,
    runtime::{RecoveryAssessment, RuntimeLaunchSpec},
};
use forge_protocol::runtime::RunnerExit;
use forge_provider_common::{
    PrivateMaterialization,
    adapter::{RuntimeFailureKind, RuntimeObservation, RuntimeUsage},
};
use forge_storage::{IncidentKind, RunProjection};
use serde::Serialize;
use serde_json::json;
mod claude_live;
mod native_live;

#[derive(Default, Serialize)]
struct RuntimeReport {
    provider_exit: Option<RunnerExit>,
    usage: Option<RuntimeUsage>,
    failure: Option<RuntimeFailureKind>,
    protocol_incomplete: bool,
    completed_turns: u64,
}

impl CoreService {
    pub(crate) async fn collect_runtime_report(
        &self,
        run: &RunProjection,
    ) -> Result<(), CoreError> {
        if run.assignment.hook().is_some() {
            return Ok(());
        }
        let Some(execution) = &self.execution else {
            return Ok(());
        };
        let mut transaction = self.store.begin().await?;
        let project = transaction
            .lock_project(run.project_id)
            .await?
            .ok_or_else(credential_error)?;
        if transaction.runtime_report_recorded(run.id).await? {
            return Ok(());
        }
        let spec: RuntimeLaunchSpec =
            serde_json::from_value(run.run_spec.clone()).map_err(|_| credential_error())?;
        let live_inputs = if matches!(spec.schema_version, 2 | 3 | 6)
            && spec
                .binding
                .execution_profile
                .capability_profile()
                .capabilities
                .contains(&forge_domain::RuntimeCapability::LiveInput)
        {
            Some(transaction.runtime_input_ids(run.id).await?)
        } else {
            None
        };
        let root = execution.root.clone();
        let relative = std::path::PathBuf::from("evidence")
            .join(run.id.to_string())
            .join(run.environment_epoch.to_string());
        let (exit, stdout) = tokio::task::spawn_blocking(move || {
            (
                PrivateMaterialization::read_beneath(&root, &relative.join("exit.json"), 4096),
                PrivateMaterialization::read_beneath(
                    &root,
                    &relative.join("stdout.jsonl"),
                    spec.binding.budget.max_output_bytes,
                ),
            )
        })
        .await
        .map_err(|_| credential_error())?;
        let mut report = match stdout {
            Ok(bytes)
                if live_inputs.is_some()
                    && spec.binding.execution_profile.adapter_id() == "claude_code_cli" =>
            {
                claude_live::normalize(
                    bytes.expose(),
                    run.id,
                    live_inputs.as_deref().unwrap_or_default(),
                )
            }
            Ok(bytes) if live_inputs.is_some() => native_live::normalize(
                bytes.expose(),
                run.id,
                live_inputs.as_deref().unwrap_or_default(),
            ),
            Ok(bytes) => normalize(
                spec.binding.execution_profile.adapter_id(),
                bytes.expose(),
                run.id,
            ),
            Err(_) => RuntimeReport {
                protocol_incomplete: true,
                ..RuntimeReport::default()
            },
        };
        report.provider_exit = exit
            .ok()
            .and_then(|bytes| serde_json::from_slice(bytes.expose()).ok());
        report.protocol_incomplete |= report
            .provider_exit
            .as_ref()
            .is_none_or(|exit| exit.output_incomplete);
        // A nonzero child exit is failure evidence. The wrapper/container's own
        // exit code is intentionally not used as the provider's exit status.
        if report.failure.is_none()
            && report.provider_exit.as_ref().is_some_and(|exit| {
                exit.exit_code.is_some_and(|code| code != 0) && !exit.stop_requested
            })
        {
            report.failure = Some(RuntimeFailureKind::RuntimeError);
        }
        let now = Timestamp::now_utc();
        if let Some(failure) = report.failure {
            let kind = match failure {
                RuntimeFailureKind::AuthRefreshReused
                | RuntimeFailureKind::AuthExpired
                | RuntimeFailureKind::AuthInvalidated
                | RuntimeFailureKind::AuthRequired => IncidentKind::Authentication,
                RuntimeFailureKind::RateLimited => IncidentKind::RateLimited,
                RuntimeFailureKind::ProviderUnavailable => IncidentKind::ProviderUnavailable,
                RuntimeFailureKind::RuntimeError => IncidentKind::RuntimeFailed,
            };
            let (id, inserted) = transaction
                .record_run_incident(run.project_id, run.id, kind, RecoveryAssessment::Unknown)
                .await?;
            if inserted {
                transaction
                    .append_event_and_outbox(&event(
                        project.id(),
                        AggregateRef::Project(project.id()),
                        project.revision(),
                        DomainEventKind::RunIncidentRaised,
                        self.actors.core,
                        CommandId::new(),
                        None,
                        event_payload([
                            ("run_id", json!(run.id)),
                            ("incident_id", json!(id)),
                            ("failure", json!(failure)),
                        ]),
                        now,
                    )?)
                    .await?;
            }
        }
        let report = serde_json::to_value(report).map_err(|_| credential_error())?;
        if transaction.record_runtime_report(run.id, &report).await? {
            transaction
                .append_event_and_outbox(&event(
                    project.id(),
                    AggregateRef::Project(project.id()),
                    project.revision(),
                    DomainEventKind::RunRuntimeReported,
                    self.actors.core,
                    CommandId::new(),
                    None,
                    event_payload([("run_id", json!(run.id)), ("report", report)]),
                    now,
                )?)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

fn normalize(adapter: &str, bytes: &[u8], run_id: uuid::Uuid) -> RuntimeReport {
    if adapter == "claude_code_cli" {
        return normalize_claude(bytes, run_id);
    }
    let mut report = RuntimeReport::default();
    let Ok(text) = std::str::from_utf8(bytes) else {
        report.protocol_incomplete = true;
        return report;
    };
    let mut missing_usage = false;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let observation = match adapter {
            "codex_cli" => forge_provider_codex::parse_jsonl_event(line),
            "opencode_runtime" => forge_provider_opencode::parse_driver_event(line),
            _ => {
                report.protocol_incomplete = true;
                break;
            }
        };
        match observation {
            Ok(RuntimeObservation::TurnCompleted { usage }) => {
                report.completed_turns = report.completed_turns.saturating_add(1);
                if let Some(usage) = usage {
                    report.usage = match report.usage.take() {
                        Some(prior) => add_usage(prior, usage),
                        None => Some(usage),
                    };
                    missing_usage |= report.usage.is_none();
                } else {
                    missing_usage = true;
                }
            }
            Ok(RuntimeObservation::Failure { kind }) => report.failure = Some(kind),
            Err(_) => report.protocol_incomplete = true,
            _ => {}
        }
    }
    if missing_usage || report.protocol_incomplete {
        report.usage = None;
    }
    report
}

/// The v2 static-input runner invokes exactly one Claude query loop. Result usage
/// is cumulative for that loop, independent of success, and must not be summed
/// with the usage carried on `TurnCompleted` or a second terminal frame.
fn normalize_claude(bytes: &[u8], run_id: uuid::Uuid) -> RuntimeReport {
    let mut report = RuntimeReport::default();
    let Ok(text) = std::str::from_utf8(bytes) else {
        report.protocol_incomplete = true;
        return report;
    };
    let mut terminal_seen = false;
    let mut identities = std::collections::HashSet::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let Ok(event) = forge_provider_claude::parse_jsonl_event(line) else {
            report.protocol_incomplete = true;
            continue;
        };
        if event.session_id.is_some_and(|id| id != run_id)
            || event.replayed_message_id.is_some_and(|id| id != run_id)
            || event.event_id.is_some_and(|id| !identities.insert(id))
        {
            report.protocol_incomplete = true;
            continue;
        }
        if let Some(end) = event.turn_end {
            if terminal_seen {
                report.protocol_incomplete = true;
                continue;
            }
            terminal_seen = true;
            report.usage = event.usage;
            report.completed_turns =
                u64::from(end == forge_provider_claude::ClaudeTurnEnd::Completed);
        }
        for observation in event.observations {
            if let RuntimeObservation::Failure { kind } = observation {
                report.failure = Some(kind);
            }
        }
    }
    report.protocol_incomplete |= !terminal_seen;
    if report.protocol_incomplete {
        report.usage = None;
    }
    report
}

fn add_usage(a: RuntimeUsage, b: RuntimeUsage) -> Option<RuntimeUsage> {
    fn optional(a: Option<u64>, b: Option<u64>) -> Option<u64> {
        a?.checked_add(b?)
    }
    Some(RuntimeUsage {
        input_tokens: a.input_tokens.checked_add(b.input_tokens)?,
        output_tokens: a.output_tokens.checked_add(b.output_tokens)?,
        cached_input_tokens: optional(a.cached_input_tokens, b.cached_input_tokens),
        cache_write_input_tokens: optional(a.cache_write_input_tokens, b.cache_write_input_tokens),
        reasoning_output_tokens: optional(a.reasoning_output_tokens, b.reasoning_output_tokens),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_reported_usage_without_promoting_text_to_success() {
        let report=normalize("codex_cli",b"{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":3,\"cached_input_tokens\":2}}\n", uuid::Uuid::nil());
        assert_eq!(report.completed_turns, 1);
        assert_eq!(report.usage.expect("usage").input_tokens, 10);
        assert!(report.provider_exit.is_none());
    }
    #[test]
    fn missing_usage_stays_unknown_and_raw_errors_are_not_retained() {
        let report=normalize("codex_cli",b"{\"type\":\"turn.completed\"}\n{\"type\":\"error\",\"message\":\"refresh_token_reused secret-body\"}\n", uuid::Uuid::nil());
        assert!(report.usage.is_none());
        assert_eq!(report.failure, Some(RuntimeFailureKind::AuthRefreshReused));
        assert!(
            !serde_json::to_string(&report)
                .expect("json")
                .contains("secret-body")
        );
    }

    #[test]
    fn public_runtime_report_does_not_retain_provider_text_or_unknown_fields() {
        let frames = [
            json!({"type":"item.completed","item":{"type":"agent_message","text":"private-assistant-canary"}}),
            json!({"type":"unknown.frame","raw":"private-ignored-canary"}),
            json!({"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":3},
                "auth":"private-auth-canary","prompt":"private-prompt-canary"}),
        ].map(|frame| frame.to_string()).join("\n");
        let report = normalize("codex_cli", frames.as_bytes(), uuid::Uuid::now_v7());
        let wire = serde_json::to_string(&report).unwrap();
        assert_eq!(report.completed_turns, 1);
        for canary in [
            "private-assistant-canary",
            "private-ignored-canary",
            "private-auth-canary",
            "private-prompt-canary",
        ] {
            assert!(!wire.contains(canary));
        }
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 5);
    }

    #[test]
    fn claude_usage_survives_aborted_turn_and_cannot_be_counted_twice() {
        let run = uuid::Uuid::now_v7();
        let result = json!({"type":"result","session_id":run,"is_error":false,"subtype":"success","terminal_reason":"aborted_tools","usage":{"input_tokens":4,"output_tokens":2}}).to_string();
        let report = normalize("claude_code_cli", result.as_bytes(), run);
        assert_eq!(report.usage.unwrap().input_tokens, 4);
        assert_eq!(report.completed_turns, 0);
        assert!(!report.protocol_incomplete);
        let duplicate = format!("{result}\n{result}\n");
        let report = normalize("claude_code_cli", duplicate.as_bytes(), run);
        assert!(report.usage.is_none());
        assert!(report.protocol_incomplete);
        let report = normalize("claude_code_cli", result.as_bytes(), uuid::Uuid::now_v7());
        assert!(report.usage.is_none());
        assert!(report.protocol_incomplete);
    }

    #[test]
    fn claude_failure_is_sanitized_and_success_has_one_usage_counter() {
        let run = uuid::Uuid::now_v7();
        let error = json!({"type":"assistant","session_id":run,"is_api_error_message":true,"api_error_status":401,"message":{"role":"assistant","content":[{"type":"text","text":"private error body"}]}}).to_string();
        let result = json!({"type":"result","session_id":run,"is_error":false,"subtype":"success","usage":{"input_tokens":4,"output_tokens":2}}).to_string();
        let report = normalize("claude_code_cli", result.as_bytes(), run);
        assert_eq!(report.completed_turns, 1);
        assert_eq!(report.usage.unwrap().input_tokens, 4);
        let report = normalize(
            "claude_code_cli",
            format!("{error}\n{result}").as_bytes(),
            run,
        );
        assert_eq!(report.failure, Some(RuntimeFailureKind::AuthRequired));
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("private error body")
        );
    }
}
