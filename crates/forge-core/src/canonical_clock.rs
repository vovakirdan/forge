//! Project-local causal time; operational deadlines continue to use wall time.

use forge_domain::{Project, Timestamp};

/// Call only after locking the Project that serializes all child mutations.
pub(crate) fn project_mutation_time(project: &Project) -> Timestamp {
    project_mutation_time_at(project, Timestamp::now_utc())
}

/// Keeps canonical revisions nondecreasing if the host wall clock rolls back.
/// No process-local state is needed: the locked durable snapshot is the floor.
pub(crate) fn project_mutation_time_at(project: &Project, observed_wall: Timestamp) -> Timestamp {
    let last_mutation = project.updated_at();
    if observed_wall < last_mutation {
        tracing::warn!(
            project_id = %project.id(),
            ?observed_wall,
            ?last_mutation,
            "host wall clock regressed; canonical mutation retains causal Project time"
        );
    }
    forge_application::engine::project_mutation_time_at(project, observed_wall)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_domain::ProjectId;
    use time::macros::datetime;

    #[test]
    fn an_old_wall_clock_cannot_regress_the_locked_project_clock() {
        let last = Timestamp::from_offset_date_time(datetime!(2026-09-06 11:22:19 UTC));
        let old = Timestamp::from_offset_date_time(datetime!(2026-09-06 11:22:18 UTC));
        let mut project = Project::new(ProjectId::new(), "clock fixture", last).expect("project");
        let logical = project_mutation_time_at(&project, old);
        assert_eq!(logical, last);
        project
            .record_child_mutation(logical)
            .expect("causal mutation");
        assert_eq!(project.updated_at(), last);
        // The pure domain still rejects a caller bypassing this Core boundary.
        assert!(project.record_child_mutation(old).is_err());
    }

    #[test]
    fn equal_and_forward_wall_clocks_are_preserved() {
        let last = Timestamp::from_offset_date_time(datetime!(2026-09-06 11:22:19 UTC));
        let forward = Timestamp::from_offset_date_time(datetime!(2026-09-06 11:22:20 UTC));
        let project = Project::new(ProjectId::new(), "clock fixture", last).expect("project");
        assert_eq!(project_mutation_time_at(&project, last), last);
        assert_eq!(project_mutation_time_at(&project, forward), forward);
    }
}
