use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{DomainError, StageId};

use super::{PipelineStage, PipelineTransitionTarget};

pub(super) fn validate_targets(
    stages: &BTreeMap<StageId, PipelineStage>,
) -> Result<(), DomainError> {
    for stage in stages.values() {
        for transition in stage.transitions() {
            if let PipelineTransitionTarget::Stage(target) = transition.target()
                && !stages.contains_key(target)
            {
                return Err(DomainError::InvalidPipeline {
                    reason: format!("stage {} targets missing stage {target}", stage.id()),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn validate_reachability(
    stages: &BTreeMap<StageId, PipelineStage>,
    entry_stage_id: &StageId,
) -> Result<(), DomainError> {
    let reachable = reachable_stages(stages, entry_stage_id);
    if reachable.len() != stages.len() {
        let unreachable = stages
            .keys()
            .filter(|stage_id| !reachable.contains(*stage_id))
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(DomainError::InvalidPipeline {
            reason: format!("unreachable stages: {unreachable}"),
        });
    }
    Ok(())
}

/// Rejects a cycle unless the immutable version supplies a finite Task-local
/// stage-entry budget. `StageVisit` is monotonic, so that budget bounds every
/// pass through every cycle in the graph.
pub(super) fn validate_cycle_limit(
    stages: &BTreeMap<StageId, PipelineStage>,
    max_stage_visits: Option<u32>,
) -> Result<(), DomainError> {
    let mut inbound = stages
        .keys()
        .cloned()
        .map(|stage_id| (stage_id, 0_u32))
        .collect::<BTreeMap<_, _>>();
    for stage in stages.values() {
        for transition in stage.transitions() {
            let PipelineTransitionTarget::Stage(target) = transition.target() else {
                continue;
            };
            let Some(count) = inbound.get_mut(target) else {
                return Err(DomainError::InvalidPipeline {
                    reason: format!("stage {} targets missing stage {target}", stage.id()),
                });
            };
            *count = count
                .checked_add(1)
                .ok_or_else(|| DomainError::InvalidPipeline {
                    reason: "pipeline stage graph has too many incoming transitions".to_owned(),
                })?;
        }
    }

    let mut pending = VecDeque::from_iter(
        inbound
            .iter()
            .filter_map(|(stage_id, count)| (*count == 0).then_some(stage_id.clone())),
    );
    let mut visited = 0_usize;
    while let Some(stage_id) = pending.pop_front() {
        visited = visited
            .checked_add(1)
            .ok_or_else(|| DomainError::InvalidPipeline {
                reason: "pipeline stage graph is too large".to_owned(),
            })?;
        let stage = stages
            .get(&stage_id)
            .ok_or_else(|| DomainError::InvalidPipeline {
                reason: format!("missing stage {stage_id} while validating graph"),
            })?;
        for transition in stage.transitions() {
            let PipelineTransitionTarget::Stage(target) = transition.target() else {
                continue;
            };
            let Some(count) = inbound.get_mut(target) else {
                return Err(DomainError::InvalidPipeline {
                    reason: format!("stage {} targets missing stage {target}", stage.id()),
                });
            };
            *count = count
                .checked_sub(1)
                .ok_or_else(|| DomainError::InvalidPipeline {
                    reason: "pipeline stage graph has an invalid inbound count".to_owned(),
                })?;
            if *count == 0 {
                pending.push_back(target.clone());
            }
        }
    }
    if visited != stages.len() && max_stage_visits.is_none() {
        return Err(DomainError::InvalidPipeline {
            reason: "pipeline graph contains a cycle but max_stage_visits is not finite".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn validate_terminal_route(
    stages: &BTreeMap<StageId, PipelineStage>,
    entry_stage_id: &StageId,
) -> Result<(), DomainError> {
    let mut reverse = BTreeMap::<StageId, Vec<StageId>>::new();
    let mut terminals = Vec::new();
    for stage in stages.values() {
        for transition in stage.transitions() {
            match transition.target() {
                PipelineTransitionTarget::Stage(target) => {
                    reverse
                        .entry(target.clone())
                        .or_default()
                        .push(stage.id().clone());
                }
                PipelineTransitionTarget::Done | PipelineTransitionTarget::Cancelled => {
                    terminals.push(stage.id().clone());
                }
            }
        }
    }
    let mut can_end = BTreeSet::new();
    let mut pending = VecDeque::from(terminals);
    while let Some(stage_id) = pending.pop_front() {
        if !can_end.insert(stage_id.clone()) {
            continue;
        }
        if let Some(previous) = reverse.get(&stage_id) {
            pending.extend(previous.iter().cloned());
        }
    }
    if !can_end.contains(entry_stage_id) {
        return Err(DomainError::InvalidPipeline {
            reason: "entry stage has no route to a terminal outcome".to_owned(),
        });
    }
    Ok(())
}

fn reachable_stages(
    stages: &BTreeMap<StageId, PipelineStage>,
    entry_stage_id: &StageId,
) -> BTreeSet<StageId> {
    let mut reachable = BTreeSet::new();
    let mut pending = VecDeque::from([entry_stage_id.clone()]);
    while let Some(stage_id) = pending.pop_front() {
        if !reachable.insert(stage_id.clone()) {
            continue;
        }
        let Some(stage) = stages.get(&stage_id) else {
            continue;
        };
        for transition in stage.transitions() {
            if let PipelineTransitionTarget::Stage(target) = transition.target() {
                pending.push_back(target.clone());
            }
        }
    }
    reachable
}
