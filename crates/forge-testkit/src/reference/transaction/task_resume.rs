use super::{MemoryTransaction, invalid};
use forge_application::RepositoryError;
use forge_domain::{ScheduledResumeState, TaskResumeSchedule};

impl MemoryTransaction {
    pub(super) fn insert_resume(
        &mut self,
        schedule: &TaskResumeSchedule,
    ) -> Result<(), RepositoryError> {
        schedule
            .validate_snapshot()
            .map_err(|_| invalid("invalid resume schedule"))?;
        self.require_task_scope(schedule.project_id, schedule.task_id)?;
        if schedule.state != ScheduledResumeState::Pending
            || self.staged.resume_schedules.contains_key(&schedule.id)
            || self.staged.resume_schedules.values().any(|stored| {
                stored.task_id == schedule.task_id
                    && stored.wait_condition_id == schedule.wait_condition_id
                    && stored.state == ScheduledResumeState::Pending
            })
        {
            return Err(invalid(
                "duplicate pending resume schedule or invalid state",
            ));
        }
        self.staged
            .resume_schedules
            .insert(schedule.id, schedule.clone());
        Ok(())
    }

    pub(super) fn update_resume(
        &mut self,
        schedule: &TaskResumeSchedule,
    ) -> Result<(), RepositoryError> {
        let stored =
            self.staged
                .resume_schedules
                .get(&schedule.id)
                .ok_or(RepositoryError::NotFound {
                    aggregate: "resume schedule",
                })?;
        if stored.state != ScheduledResumeState::Pending {
            return Err(RepositoryError::StaleRevision {
                aggregate: "resume schedule",
            });
        }
        let mut permitted = stored.clone();
        permitted
            .resolve(schedule.state.clone())
            .map_err(|_| invalid("invalid schedule result"))?;
        if &permitted != schedule {
            return Err(invalid("resume schedule identity is immutable"));
        }
        self.staged
            .resume_schedules
            .insert(schedule.id, schedule.clone());
        Ok(())
    }
}
