//! Mechanical persistence; deliberately contains no command dispatch or Task policy.

use forge_application::{CommandTransaction, RepositoryError, ports::*};
use forge_domain::{
    Actor, Artifact, ArtifactId, DomainEvent, Employee, EventId, Pipeline, PipelineId,
    PipelineVersion, PipelineVersionId, Project, ProjectId, Task, TaskDependency, TaskId,
};
use uuid::Uuid;

use super::{MemoryQueueEntry, MemoryQueueState, MemoryTransaction};

fn invalid(reason: &str) -> RepositoryError {
    RepositoryError::InvalidInput {
        reason: reason.to_owned(),
    }
}

impl MemoryTransaction {
    fn require_project(&self, project: ProjectId) -> Result<(), RepositoryError> {
        if self.staged.projects.contains_key(&project) {
            Ok(())
        } else {
            Err(RepositoryError::NotFound {
                aggregate: "project",
            })
        }
    }

    fn require_task_scope(&self, project: ProjectId, task: TaskId) -> Result<(), RepositoryError> {
        match self.staged.tasks.get(&task) {
            Some(stored) if stored.task.project_id() == project => Ok(()),
            _ => Err(RepositoryError::NotFound { aggregate: "task" }),
        }
    }
}

impl CommandTransaction for MemoryTransaction {
    async fn lock_project_creation(&mut self, _id: ProjectId) -> Result<(), RepositoryError> {
        // The transaction already holds the global writer lock, including absent rows.
        Ok(())
    }

    async fn lock_project(&mut self, id: ProjectId) -> Result<Option<Project>, RepositoryError> {
        Ok(self.staged.projects.get(&id).cloned())
    }

    async fn insert_project(&mut self, project: &Project) -> Result<(), RepositoryError> {
        project
            .validate_snapshot()
            .map_err(|_| invalid("invalid project snapshot"))?;
        if self.staged.projects.contains_key(&project.id()) {
            return Err(invalid("duplicate project"));
        }
        self.staged.projects.insert(project.id(), project.clone());
        Ok(())
    }

    async fn update_project(
        &mut self,
        project: &Project,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        if project.revision() <= expected_revision {
            return Err(invalid("project revision must advance"));
        }
        if self
            .staged
            .projects
            .get(&project.id())
            .is_none_or(|p| p.revision() != expected_revision)
        {
            return Err(RepositoryError::StaleRevision {
                aggregate: "project",
            });
        }
        project
            .validate_snapshot()
            .map_err(|_| invalid("invalid project snapshot"))?;
        self.staged.projects.insert(project.id(), project.clone());
        Ok(())
    }

    async fn lock_pipeline(&mut self, id: PipelineId) -> Result<Option<Pipeline>, RepositoryError> {
        Ok(self.staged.pipelines.get(&id).cloned())
    }

    async fn lock_pipeline_version(
        &mut self,
        id: PipelineVersionId,
    ) -> Result<Option<PipelineVersion>, RepositoryError> {
        Ok(self.staged.pipeline_versions.get(&id).cloned())
    }

    async fn insert_pipeline(&mut self, pipeline: &Pipeline) -> Result<(), RepositoryError> {
        self.require_project(pipeline.project_id())?;
        pipeline
            .validate_snapshot()
            .map_err(|_| invalid("invalid pipeline snapshot"))?;
        if self.staged.pipelines.contains_key(&pipeline.id())
            || (pipeline.deleted_at().is_none()
                && self.staged.pipelines.values().any(|stored| {
                    stored.project_id() == pipeline.project_id()
                        && stored.name() == pipeline.name()
                        && stored.deleted_at().is_none()
                }))
        {
            return Err(invalid("duplicate pipeline"));
        }
        self.staged
            .pipelines
            .insert(pipeline.id(), pipeline.clone());
        Ok(())
    }

    async fn insert_pipeline_version(
        &mut self,
        version: &PipelineVersion,
    ) -> Result<(), RepositoryError> {
        version
            .validate_snapshot()
            .map_err(|_| invalid("invalid pipeline version"))?;
        if !self.staged.pipelines.contains_key(&version.pipeline_id()) {
            return Err(invalid("missing pipeline"));
        }
        if self.staged.pipeline_versions.contains_key(&version.id())
            || self.staged.pipeline_versions.values().any(|v| {
                v.pipeline_id() == version.pipeline_id() && v.version() == version.version()
            })
        {
            return Err(invalid("duplicate pipeline version"));
        }
        self.staged
            .pipeline_versions
            .insert(version.id(), version.clone());
        Ok(())
    }

    async fn insert_employee(&mut self, employee: &Employee) -> Result<(), RepositoryError> {
        self.require_project(employee.project_id())?;
        employee
            .validate_snapshot()
            .map_err(|_| invalid("invalid employee snapshot"))?;
        if self.staged.employees.contains_key(&employee.id())
            || self.staged.employees.values().any(|stored| {
                stored.project_id() == employee.project_id() && stored.name() == employee.name()
            })
        {
            return Err(invalid("duplicate employee"));
        }
        self.staged
            .employees
            .insert(employee.id(), employee.clone());
        Ok(())
    }

    async fn lock_task(&mut self, id: TaskId) -> Result<Option<StoredTask>, RepositoryError> {
        Ok(self.staged.tasks.get(&id).cloned())
    }

    async fn insert_task(
        &mut self,
        task: &Task,
        persistence: TaskPersistence,
    ) -> Result<(), RepositoryError> {
        self.require_project(task.project_id())?;
        task.validate_snapshot()
            .map_err(|_| invalid("invalid task snapshot"))?;
        if self.staged.tasks.contains_key(&task.id())
            || self
                .staged
                .tasks
                .values()
                .any(|s| s.task.project_id() == task.project_id() && s.task.key() == task.key())
        {
            return Err(invalid("duplicate task identity or sequence"));
        }
        self.staged.tasks.insert(
            task.id(),
            StoredTask {
                task: task.clone(),
                persistence,
            },
        );
        Ok(())
    }

    async fn update_task(
        &mut self,
        task: &Task,
        persistence: TaskPersistence,
        expected_revision: u64,
    ) -> Result<(), RepositoryError> {
        if task.revision().get() <= expected_revision {
            return Err(invalid("task revision must advance"));
        }
        if self
            .staged
            .tasks
            .get(&task.id())
            .is_none_or(|s| s.task.revision().get() != expected_revision)
        {
            return Err(RepositoryError::StaleRevision { aggregate: "task" });
        }
        task.validate_snapshot()
            .map_err(|_| invalid("invalid task snapshot"))?;
        self.staged.tasks.insert(
            task.id(),
            StoredTask {
                task: task.clone(),
                persistence,
            },
        );
        Ok(())
    }

    async fn list_dependencies(
        &mut self,
        project: ProjectId,
    ) -> Result<Vec<TaskDependency>, RepositoryError> {
        Ok(self
            .staged
            .dependencies
            .iter()
            .filter(|((p, _, _), _)| *p == project)
            .map(|(_, (edge, _))| *edge)
            .collect())
    }

    async fn insert_dependency(
        &mut self,
        project: ProjectId,
        dependency: TaskDependency,
        actor: Actor,
    ) -> Result<bool, RepositoryError> {
        self.require_task_scope(project, dependency.blocker_task_id())?;
        self.require_task_scope(project, dependency.blocked_task_id())?;
        let key = (
            project,
            dependency.blocker_task_id(),
            dependency.blocked_task_id(),
        );
        if self.staged.dependencies.contains_key(&key) {
            return Ok(false);
        }
        self.staged.dependencies.insert(key, (dependency, actor));
        Ok(true)
    }

    async fn remove_dependency(
        &mut self,
        project: ProjectId,
        blocker: TaskId,
        blocked: TaskId,
    ) -> Result<bool, RepositoryError> {
        Ok(self
            .staged
            .dependencies
            .remove(&(project, blocker, blocked))
            .is_some())
    }

    async fn lock_artifact(
        &mut self,
        id: ArtifactId,
    ) -> Result<Option<StoredArtifact>, RepositoryError> {
        Ok(self.staged.artifacts.get(&id).cloned())
    }

    async fn insert_artifact(
        &mut self,
        artifact: &Artifact,
        location: &ArtifactLocation,
    ) -> Result<(), RepositoryError> {
        self.require_project(artifact.project_id())?;
        artifact
            .validate_snapshot()
            .map_err(|_| invalid("invalid artifact snapshot"))?;
        if let Some(task) = location.task_id {
            self.require_task_scope(artifact.project_id(), task)?;
        }
        if self.staged.artifacts.contains_key(&artifact.id()) {
            return Err(invalid("duplicate immutable artifact"));
        }
        self.staged.artifacts.insert(
            artifact.id(),
            StoredArtifact {
                artifact: artifact.clone(),
                location: location.clone(),
            },
        );
        Ok(())
    }

    async fn enqueue(&mut self, input: &QueueEntryInput) -> Result<bool, RepositoryError> {
        self.require_task_scope(input.project_id, input.task_id)?;
        let same_task = |entry: &&MemoryQueueEntry| {
            entry.input.project_id == input.project_id && entry.input.task_id == input.task_id
        };
        if self.staged.queue.iter().filter(same_task).any(|e| {
            e.input.stage_id == input.stage_id && e.input.task_revision == input.task_revision
        }) {
            return Ok(false);
        }
        if self
            .staged
            .queue
            .iter()
            .filter(same_task)
            .any(|e| matches!(e.state, MemoryQueueState::Queued | MemoryQueueState::Leased))
        {
            return Err(invalid("task already has active queue intent"));
        }
        self.staged.queue.push(MemoryQueueEntry {
            id: Uuid::now_v7(),
            input: input.clone(),
            state: MemoryQueueState::Queued,
        });
        Ok(true)
    }

    async fn invalidate_queued_entries_for_task(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<u64, RepositoryError> {
        let mut affected = 0;
        for entry in &mut self.staged.queue {
            if entry.input.project_id == project
                && entry.input.task_id == task
                && entry.state == MemoryQueueState::Queued
            {
                entry.state = MemoryQueueState::Cancelled;
                affected += 1;
            }
        }
        Ok(affected)
    }

    async fn lock_active_runs_for_project(
        &mut self,
        project: ProjectId,
    ) -> Result<Vec<ActiveRun>, RepositoryError> {
        Ok(self
            .staged
            .runs
            .iter()
            .filter(|r| r.scope.project_id == project && r.lease_active)
            .map(|r| r.scope.clone())
            .collect())
    }

    async fn lock_active_runs_for_task(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Vec<ActiveRun>, RepositoryError> {
        Ok(self
            .staged
            .runs
            .iter()
            .filter(|r| r.scope.project_id == project && r.scope.task_id == task && r.lease_active)
            .map(|r| r.scope.clone())
            .collect())
    }

    async fn request_run_stop(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
        force: bool,
    ) -> Result<bool, RepositoryError> {
        let Some(run) = self.staged.runs.iter_mut().find(|r| {
            r.scope.id == run
                && r.scope.lease_fencing_token == fence
                && r.scope.environment_epoch == epoch
        }) else {
            return Ok(false);
        };
        if run
            .stop_requested
            .is_some_and(|already_force| already_force || !force)
        {
            return Ok(false);
        }
        run.stop_requested = Some(force);
        Ok(true)
    }

    async fn cancel_leased_queue_for_fenced_run(
        &mut self,
        run: Uuid,
        fence: u64,
        epoch: u64,
    ) -> Result<bool, RepositoryError> {
        let Some(run) = self.staged.runs.iter().find(|r| {
            r.scope.id == run
                && r.scope.lease_fencing_token == fence
                && r.scope.environment_epoch == epoch
                && r.lease_active
        }) else {
            return Ok(false);
        };
        let Some(queue) = self
            .staged
            .queue
            .iter_mut()
            .find(|q| q.id == run.queue_entry_id && q.state == MemoryQueueState::Leased)
        else {
            return Ok(false);
        };
        queue.state = MemoryQueueState::Cancelled;
        Ok(true)
    }

    async fn set_recovery_hold(
        &mut self,
        project: ProjectId,
        hold: bool,
    ) -> Result<(), RepositoryError> {
        self.require_project(project)?;
        if hold {
            self.staged.recovery_holds.insert(project);
        } else {
            self.staged.recovery_holds.remove(&project);
        }
        Ok(())
    }

    async fn lookup_idempotency(
        &mut self,
        project: ProjectId,
        key: &str,
    ) -> Result<Option<IdempotencyRecord>, RepositoryError> {
        Ok(self
            .staged
            .idempotency
            .get(&(project, key.to_owned()))
            .cloned())
    }

    async fn insert_idempotency(
        &mut self,
        record: &IdempotencyRecord,
    ) -> Result<(), RepositoryError> {
        self.require_project(record.project_id)?;
        let key = (record.project_id, record.key.clone());
        if self.staged.idempotency.contains_key(&key) {
            return Err(invalid("duplicate idempotency key"));
        }
        if record.key.trim().is_empty()
            || record.command_name.trim().is_empty()
            || record.request_hash.trim().is_empty()
            || !record.actor.is_object()
            || !record.receipt.is_object()
        {
            return Err(invalid("invalid idempotency record"));
        }
        self.staged.idempotency.insert(key, record.clone());
        Ok(())
    }

    async fn append_event_and_outbox(
        &mut self,
        event: &DomainEvent,
    ) -> Result<EventId, RepositoryError> {
        self.require_project(event.project_id())?;
        self.append_audit(event)
    }
}
