//! Core-owned read paths; HTTP handlers never access canonical storage directly.

use forge_domain::{Pipeline, PipelineVersion, Project, ProjectId, TaskId};
use forge_storage::{RunProjection, StoredArtifact, StoredEvent, StoredTask};
use uuid::Uuid;

use crate::{CoreError, CoreService};

pub(crate) struct TaskRead {
    pub(crate) task: StoredTask,
    pub(crate) artifacts: Vec<StoredArtifact>,
}

pub(crate) struct PipelineVersionRead {
    pub(crate) pipeline: Pipeline,
    pub(crate) version: PipelineVersion,
}

impl CoreService {
    pub(crate) async fn read_projects(&self) -> Result<Vec<Project>, CoreError> {
        Ok(self.store().list_projects().await?)
    }

    pub(crate) async fn read_employee_threads(
        &self,
        project_id: ProjectId,
        employee_id: forge_domain::EmployeeId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<forge_domain::communication::EmployeeThread>, CoreError> {
        self.store()
            .load_employee(employee_id)
            .await?
            .filter(|employee| employee.employee.project_id() == project_id)
            .ok_or(CoreError::NotFound {
                aggregate: "employee",
            })?;
        Ok(self
            .store()
            .list_employee_threads(project_id, employee_id, after, limit)
            .await?)
    }

    pub(crate) async fn read_employee_messages(
        &self,
        project_id: ProjectId,
        thread_id: Uuid,
        after: u64,
        limit: u32,
    ) -> Result<Vec<forge_domain::communication::EmployeeMessage>, CoreError> {
        self.store()
            .load_employee_thread(project_id, thread_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "employee thread",
            })?;
        Ok(self
            .store()
            .list_employee_messages(project_id, thread_id, after, limit)
            .await?)
    }

    pub(crate) async fn read_run_diagnostics(
        &self,
        project_id: ProjectId,
        run_id: Uuid,
    ) -> Result<serde_json::Value, CoreError> {
        self.read_run(project_id, run_id).await?;
        Ok(self.store.run_diagnostics(run_id).await?)
    }
    pub(crate) async fn read_project(&self, project_id: ProjectId) -> Result<Project, CoreError> {
        self.store()
            .load_project(project_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "project",
            })
    }

    pub(crate) async fn read_tasks(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<StoredTask>, CoreError> {
        self.read_project(project_id).await?;
        Ok(self.store().list_tasks(project_id).await?)
    }

    pub(crate) async fn read_task(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
    ) -> Result<TaskRead, CoreError> {
        let task = self
            .store()
            .load_task(task_id)
            .await?
            .ok_or(CoreError::NotFound { aggregate: "task" })?;
        if task.task.project_id() != project_id {
            return Err(CoreError::NotFound { aggregate: "task" });
        }
        let artifacts = self.store().list_artifacts_for_task(task_id).await?;
        if artifacts
            .iter()
            .any(|artifact| artifact.artifact.project_id() != project_id)
        {
            return Err(CoreError::InvalidTransport {
                field: "artifact.project_id",
                reason: "does not match its task project".to_owned(),
            });
        }
        Ok(TaskRead { task, artifacts })
    }

    pub(crate) async fn read_pipeline_versions(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<PipelineVersionRead>, CoreError> {
        self.read_project(project_id).await?;
        let versions = self.store().list_pipeline_versions(project_id).await?;
        let mut result = Vec::with_capacity(versions.len());
        for version in versions {
            result.push(self.read_pipeline_version(project_id, version.id()).await?);
        }
        Ok(result)
    }

    pub(crate) async fn read_pipeline_version(
        &self,
        project_id: ProjectId,
        pipeline_version_id: forge_domain::PipelineVersionId,
    ) -> Result<PipelineVersionRead, CoreError> {
        let version = self
            .store()
            .load_pipeline_version(pipeline_version_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "pipeline version",
            })?;
        if version.project_id() != project_id {
            return Err(CoreError::NotFound {
                aggregate: "pipeline version",
            });
        }
        let pipeline = self
            .store()
            .load_pipeline(version.pipeline_id())
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "pipeline",
            })?;
        if pipeline.project_id() != project_id {
            return Err(CoreError::NotFound {
                aggregate: "pipeline version",
            });
        }
        Ok(PipelineVersionRead { pipeline, version })
    }

    pub(crate) async fn read_runs(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<RunProjection>, CoreError> {
        self.read_project(project_id).await?;
        Ok(self.store().list_runs_for_project(project_id).await?)
    }

    pub(crate) async fn read_run(
        &self,
        project_id: ProjectId,
        run_id: Uuid,
    ) -> Result<RunProjection, CoreError> {
        let run = self
            .store()
            .load_run(run_id)
            .await?
            .ok_or(CoreError::NotFound { aggregate: "run" })?;
        if run.project_id != project_id {
            return Err(CoreError::NotFound { aggregate: "run" });
        }
        Ok(run)
    }

    pub(crate) async fn read_events(
        &self,
        project_id: ProjectId,
        after_sequence: Option<u64>,
    ) -> Result<Vec<StoredEvent>, CoreError> {
        self.read_project(project_id).await?;
        Ok(self
            .store()
            .list_events(project_id, after_sequence, 1_000)
            .await?)
    }
}
