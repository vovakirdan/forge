use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{DomainError, LifecycleStatus, TaskId};

/// Gate a blocking Task must satisfy before its dependent may begin a stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyCondition {
    /// The blocking Task must reach terminal success.
    Done,
}

/// Directed relationship from a blocker to the Task it prevents from starting.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskDependency {
    blocker_task_id: TaskId,
    blocked_task_id: TaskId,
    condition: DependencyCondition,
}

impl TaskDependency {
    /// Creates one directed dependency.
    ///
    /// # Errors
    ///
    /// Refuses self-dependencies, which are a cycle by definition.
    pub fn new(
        blocker_task_id: TaskId,
        blocked_task_id: TaskId,
        condition: DependencyCondition,
    ) -> Result<Self, DomainError> {
        if blocker_task_id == blocked_task_id {
            return Err(DomainError::DependencyCycle);
        }
        Ok(Self {
            blocker_task_id,
            blocked_task_id,
            condition,
        })
    }

    /// Returns the Task that must satisfy the gate.
    #[must_use]
    pub const fn blocker_task_id(self) -> TaskId {
        self.blocker_task_id
    }

    /// Returns the Task prevented from starting until the gate is satisfied.
    #[must_use]
    pub const fn blocked_task_id(self) -> TaskId {
        self.blocked_task_id
    }

    /// Returns the blocking condition.
    #[must_use]
    pub const fn condition(self) -> DependencyCondition {
        self.condition
    }

    /// Returns whether a blocker lifecycle satisfies this gate.
    #[must_use]
    pub fn is_satisfied_by(self, lifecycle: LifecycleStatus) -> bool {
        match self.condition {
            DependencyCondition::Done => lifecycle == LifecycleStatus::Done,
        }
    }
}

/// Bidirectional, acyclic dependency projection for one Project.
///
/// Persistence may materialize the two lookup directions separately, but this
/// pure type is the canonical graph-validation algorithm.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TaskDependencyGraph {
    blockers_by_task: BTreeMap<TaskId, BTreeSet<TaskId>>,
    blocked_by_task: BTreeMap<TaskId, BTreeSet<TaskId>>,
    dependencies: BTreeMap<(TaskId, TaskId), TaskDependency>,
}

impl TaskDependencyGraph {
    /// Adds a dependency after proving it cannot close a directed cycle.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::DependencyCycle`] when the edge would produce a
    /// path from the blocked Task back to the blocker.
    pub fn add(&mut self, dependency: TaskDependency) -> Result<(), DomainError> {
        let key = (dependency.blocker_task_id(), dependency.blocked_task_id());
        if self.dependencies.contains_key(&key) {
            return Ok(());
        }
        if self.reaches(dependency.blocked_task_id(), dependency.blocker_task_id()) {
            return Err(DomainError::DependencyCycle);
        }
        self.blockers_by_task
            .entry(dependency.blocked_task_id())
            .or_default()
            .insert(dependency.blocker_task_id());
        self.blocked_by_task
            .entry(dependency.blocker_task_id())
            .or_default()
            .insert(dependency.blocked_task_id());
        self.dependencies.insert(key, dependency);
        Ok(())
    }

    /// Removes a dependency when it is no longer part of the Project plan.
    #[must_use]
    pub fn remove(&mut self, blocker_task_id: TaskId, blocked_task_id: TaskId) -> bool {
        let Some(dependency) = self
            .dependencies
            .remove(&(blocker_task_id, blocked_task_id))
        else {
            return false;
        };
        if let Some(blockers) = self.blockers_by_task.get_mut(&dependency.blocked_task_id()) {
            blockers.remove(&dependency.blocker_task_id());
            if blockers.is_empty() {
                self.blockers_by_task.remove(&dependency.blocked_task_id());
            }
        }
        if let Some(blocked) = self.blocked_by_task.get_mut(&dependency.blocker_task_id()) {
            blocked.remove(&dependency.blocked_task_id());
            if blocked.is_empty() {
                self.blocked_by_task.remove(&dependency.blocker_task_id());
            }
        }
        true
    }

    /// Lists the Tasks directly blocking `task_id` in stable identity order.
    pub fn blockers_for(&self, task_id: TaskId) -> impl Iterator<Item = TaskDependency> + '_ {
        self.blockers_by_task
            .get(&task_id)
            .into_iter()
            .flatten()
            .filter_map(move |blocker_task_id| {
                self.dependencies.get(&(*blocker_task_id, task_id)).copied()
            })
    }

    /// Lists the Tasks directly blocked by `task_id` in stable identity order.
    pub fn blocked_for(&self, task_id: TaskId) -> impl Iterator<Item = TaskDependency> + '_ {
        self.blocked_by_task
            .get(&task_id)
            .into_iter()
            .flatten()
            .filter_map(move |blocked_task_id| {
                self.dependencies.get(&(task_id, *blocked_task_id)).copied()
            })
    }

    /// Lists dependency gates whose blocker does not currently satisfy them.
    pub fn unmet_blockers<F>(&self, task_id: TaskId, lifecycle_for: F) -> Vec<TaskDependency>
    where
        F: FnMut(TaskId) -> Option<LifecycleStatus>,
    {
        let mut lifecycle_for = lifecycle_for;
        self.blockers_for(task_id)
            .filter(|dependency| {
                lifecycle_for(dependency.blocker_task_id())
                    .is_none_or(|lifecycle| !dependency.is_satisfied_by(lifecycle))
            })
            .collect()
    }

    fn reaches(&self, from: TaskId, sought: TaskId) -> bool {
        let mut pending = vec![from];
        let mut visited = BTreeSet::new();
        while let Some(current) = pending.pop() {
            if !visited.insert(current) {
                continue;
            }
            if current == sought {
                return true;
            }
            if let Some(next) = self.blocked_by_task.get(&current) {
                pending.extend(next.iter().copied());
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{DependencyCondition, TaskDependency, TaskDependencyGraph};
    use crate::{LifecycleStatus, TaskId};

    #[test]
    fn maintains_both_directions_and_rejects_cycles() {
        let first = TaskId::new();
        let second = TaskId::new();
        let third = TaskId::new();
        let mut graph = TaskDependencyGraph::default();
        graph
            .add(TaskDependency::new(first, second, DependencyCondition::Done).expect("edge"))
            .expect("acyclic");
        graph
            .add(TaskDependency::new(second, third, DependencyCondition::Done).expect("edge"))
            .expect("acyclic");

        assert_eq!(graph.blockers_for(third).count(), 1);
        assert_eq!(graph.blocked_for(first).count(), 1);
        assert!(
            graph
                .add(TaskDependency::new(third, first, DependencyCondition::Done).expect("edge"))
                .is_err()
        );
        assert_eq!(
            graph
                .unmet_blockers(second, |_| Some(LifecycleStatus::Done))
                .len(),
            0
        );
    }
}
