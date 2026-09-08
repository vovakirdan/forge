//! Common live drivers emit exact correlated metadata and cumulative Run usage.
use super::RuntimeReport;
use forge_provider_common::{
    adapter::{RuntimeObservation, RuntimeUsage},
    driver_event::parse_driver_event,
    native_event::{NativeDriverEvent, UsageBasis},
};
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
    let mut turns = HashSet::new();
    let mut active: Option<(Uuid, String, String)> = None;
    let mut session: Option<String> = None;
    let mut ended = 0;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let metadata = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .is_some_and(|v| v.get("forge_event").is_some());
        if !metadata {
            match parse_driver_event(line) {
                Ok(RuntimeObservation::Failure { kind }) => report.failure = Some(kind),
                Ok(RuntimeObservation::TurnCompleted { .. }) | Err(_) => {
                    report.protocol_incomplete = true
                }
                _ => {}
            }
            continue;
        }
        match NativeDriverEvent::decode(line.as_bytes()) {
            Ok(NativeDriverEvent::InputAccepted {
                run_id,
                input_id,
                session_id,
                turn_id,
            }) => {
                if run_id != run
                    || !allowed.contains(&input_id)
                    || !seen.insert(input_id)
                    || !turns.insert(turn_id.clone())
                    || active.is_some()
                    || session.as_ref().is_some_and(|prior| *prior != session_id)
                    || (seen.len() == 1 && input_id != run)
                {
                    report.protocol_incomplete = true;
                    continue;
                }
                session = Some(session_id.clone());
                active = Some((input_id, session_id, turn_id));
            }
            Ok(NativeDriverEvent::TurnFinished {
                run_id,
                input_id,
                session_id,
                turn_id,
                completed,
                usage,
                usage_basis,
            }) => {
                if run_id != run
                    || active.as_ref() != Some(&(input_id, session_id, turn_id))
                    || usage_basis != UsageBasis::Cumulative
                {
                    report.protocol_incomplete = true;
                    continue;
                }
                active = None;
                ended += 1;
                if completed {
                    report.completed_turns += 1;
                }
                if let (Some(previous), Some(current)) = (&report.usage, &usage)
                    && !monotonic(previous, current)
                {
                    report.protocol_incomplete = true;
                }
                report.usage = usage;
            }
            Ok(NativeDriverEvent::Failure {
                run_id,
                input_id,
                kind,
            }) if run_id == run && allowed.contains(&input_id) => report.failure = Some(kind),
            _ => report.protocol_incomplete = true,
        }
    }
    report.protocol_incomplete |= active.is_some() || ended == 0;
    if report.protocol_incomplete {
        report.usage = None;
    }
    report
}
fn monotonic(previous: &RuntimeUsage, current: &RuntimeUsage) -> bool {
    current.input_tokens >= previous.input_tokens
        && current.output_tokens >= previous.output_tokens
        && [
            (previous.cached_input_tokens, current.cached_input_tokens),
            (
                previous.cache_write_input_tokens,
                current.cache_write_input_tokens,
            ),
            (
                previous.reasoning_output_tokens,
                current.reasoning_output_tokens,
            ),
        ]
        .into_iter()
        .all(|(before, after)| match (before, after) {
            (Some(before), Some(after)) => after >= before,
            _ => true,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn turn(run: Uuid, input: Uuid, session: &str, tokens: u64) -> String {
        let turn = Uuid::now_v7().to_string();
        [
            NativeDriverEvent::InputAccepted {
                run_id: run,
                input_id: input,
                session_id: session.into(),
                turn_id: turn.clone(),
            },
            NativeDriverEvent::TurnFinished {
                run_id: run,
                input_id: input,
                session_id: session.into(),
                turn_id: turn,
                completed: true,
                usage: Some(RuntimeUsage {
                    input_tokens: tokens,
                    output_tokens: 2,
                    cached_input_tokens: None,
                    cache_write_input_tokens: None,
                    reasoning_output_tokens: None,
                }),
                usage_basis: UsageBasis::Cumulative,
            },
        ]
        .iter()
        .map(|event| String::from_utf8(event.encode().unwrap().expose().to_vec()).unwrap() + "\n")
        .collect()
    }
    #[test]
    fn cumulative_snapshots_are_not_summed() {
        let run = Uuid::now_v7();
        let input = Uuid::now_v7();
        let report = normalize(
            (turn(run, run, "ses_test", 10) + &turn(run, input, "ses_test", 25)).as_bytes(),
            run,
            &[input],
        );
        assert!(!report.protocol_incomplete);
        assert_eq!(report.completed_turns, 2);
        assert_eq!(report.usage.unwrap().input_tokens, 25);
    }
    #[test]
    fn duplicate_foreign_session_and_regressing_usage_fail_closed() {
        let run = Uuid::now_v7();
        let input = Uuid::now_v7();
        let first = turn(run, run, "ses_test", 10);
        for tail in [
            first.clone(),
            turn(run, input, "ses_other", 20),
            turn(run, input, "ses_test", 5),
        ] {
            let report = normalize((first.clone() + &tail).as_bytes(), run, &[input]);
            assert!(report.protocol_incomplete);
            assert!(report.usage.is_none());
        }
        assert!(
            normalize(
                (first + &turn(run, input, "ses_test", 20)).as_bytes(),
                run,
                &[]
            )
            .protocol_incomplete
        );
    }
}
