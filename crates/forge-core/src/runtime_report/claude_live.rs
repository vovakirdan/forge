//! Claude result.usage is per user turn; monetary/model totals are cumulative.
//! See official Agent SDK cost-tracking#track-costs-in-streaming-input-mode.
use super::{RuntimeReport, add_usage};
use forge_provider_claude::{ClaudeTurnEnd, parse_jsonl_event};
use forge_provider_common::adapter::RuntimeObservation;
use std::collections::HashSet;
use uuid::Uuid;

pub(super) fn normalize(bytes: &[u8], run: Uuid, inputs: &[Uuid]) -> RuntimeReport {
    let mut report = RuntimeReport::default();
    let Ok(text) = std::str::from_utf8(bytes) else {
        report.protocol_incomplete = true;
        return report;
    };
    let allowed: HashSet<_> = inputs.iter().copied().chain([run]).collect();
    let mut seen = HashSet::new();
    let mut echoed = HashSet::new();
    let mut active = Some(run);
    let mut result_count = 0;
    let mut missing_usage = false;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let Ok(event) = parse_jsonl_event(line) else {
            report.protocol_incomplete = true;
            continue;
        };
        if event.session_id.is_some_and(|id| id != run)
            || event.event_id.is_some_and(|id| !seen.insert(id))
        {
            report.protocol_incomplete = true;
            continue;
        }
        if let Some(id) = event.replayed_message_id {
            if !allowed.contains(&id)
                || !echoed.insert(id)
                || active.is_some_and(|prior| prior != id)
            {
                report.protocol_incomplete = true;
                continue;
            }
            active = Some(id);
        }
        if let Some(end) = event.turn_end {
            if active.take().is_none() {
                report.protocol_incomplete = true;
                continue;
            }
            result_count += 1;
            if end == ClaudeTurnEnd::Completed {
                report.completed_turns += 1;
            }
            if let Some(usage) = event.usage {
                report.usage = match report.usage.take() {
                    Some(previous) => add_usage(previous, usage),
                    None => Some(usage),
                };
                missing_usage |= report.usage.is_none();
            } else {
                missing_usage = true;
            }
        }
        for observation in event.observations {
            if let RuntimeObservation::Failure { kind } = observation {
                report.failure = Some(kind);
            }
        }
    }
    report.protocol_incomplete |=
        active.is_some() || result_count == 0 || result_count > echoed.len();
    if report.protocol_incomplete || missing_usage {
        report.usage = None;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn turn(run: Uuid, id: Uuid, input: u64, output: u64) -> String {
        [json!({"type":"user","session_id":run,"uuid":id,"message":{"role":"user","content":"synthetic"}}),
        json!({"type":"result","session_id":run,"uuid":Uuid::now_v7(),"subtype":"success","is_error":false,"usage":{"input_tokens":input,"output_tokens":output},"total_cost_usd":99})]
            .map(|value|value.to_string()).join("\n")+"\n"
    }
    #[test]
    fn two_turns_sum_turn_usage_not_cumulative_cost() {
        let run = Uuid::now_v7();
        let input = Uuid::now_v7();
        let text = turn(run, run, 7, 3) + &turn(run, input, 11, 5);
        let report = normalize(text.as_bytes(), run, &[input]);
        assert!(!report.protocol_incomplete);
        assert_eq!(report.completed_turns, 2);
        let usage = report.usage.unwrap();
        assert_eq!((usage.input_tokens, usage.output_tokens), (18, 8));
    }
    #[test]
    fn foreign_input_and_duplicate_results_cannot_inflate_usage() {
        let run = Uuid::now_v7();
        let input = Uuid::now_v7();
        let text = turn(run, run, 7, 3) + &turn(run, input, 11, 5);
        assert!(normalize(text.as_bytes(), run, &[]).protocol_incomplete);
        let duplicate = text.clone() + text.lines().last().unwrap();
        assert!(
            normalize(duplicate.as_bytes(), run, &[input])
                .usage
                .is_none()
        );
    }
}
