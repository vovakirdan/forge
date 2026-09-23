//! Core-owned read paths; HTTP handlers never access canonical storage directly.

use forge_domain::{
    EmployeeId, Pipeline, PipelineId, PipelineVersion, PipelineVersionId, Project, ProjectId,
    TaskId,
};
use forge_storage::{
    AdmissionResourceSnapshot, EmployeeOperationalCounts, PipelineCatalogItem, RunProjection,
    StoredArtifact, StoredEmployee, StoredEvent, StoredTask,
};
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
    pub(crate) async fn read_pipeline_catalog_page(
        &self,
        project_id: ProjectId,
        after: Option<PipelineId>,
        limit: u32,
    ) -> Result<Vec<PipelineCatalogItem>, CoreError> {
        self.read_project(project_id).await?;
        Ok(self
            .store()
            .list_pipeline_catalog_page(project_id, after, limit)
            .await?)
    }

    pub(crate) async fn read_project_resources(
        &self,
        project_id: ProjectId,
    ) -> Result<AdmissionResourceSnapshot, CoreError> {
        self.read_project(project_id).await?;
        Ok(self.store().admission_resource_snapshot(project_id).await?)
    }

    pub(crate) async fn read_projects(&self) -> Result<Vec<Project>, CoreError> {
        Ok(self.store().list_projects().await?)
    }

    pub(crate) async fn read_employees(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<StoredEmployee>, CoreError> {
        self.read_project(project_id).await?;
        Ok(self.store().list_employees(project_id).await?)
    }

    pub(crate) async fn read_employee(
        &self,
        project_id: ProjectId,
        employee_id: EmployeeId,
    ) -> Result<StoredEmployee, CoreError> {
        self.store()
            .load_employee(employee_id)
            .await?
            .filter(|employee| employee.employee.project_id() == project_id)
            .ok_or(CoreError::NotFound {
                aggregate: "employee",
            })
    }

    pub(crate) async fn read_employee_runs(
        &self,
        project_id: ProjectId,
        employee_id: EmployeeId,
    ) -> Result<Vec<RunProjection>, CoreError> {
        self.read_employee(project_id, employee_id).await?;
        Ok(self
            .store()
            .list_runs_for_employee(project_id, employee_id)
            .await?)
    }

    pub(crate) async fn read_employee_operations(
        &self,
        project_id: ProjectId,
        employee_id: EmployeeId,
    ) -> Result<(StoredEmployee, EmployeeOperationalCounts), CoreError> {
        let employee = self.read_employee(project_id, employee_id).await?;
        let counts = self
            .store()
            .employee_operational_counts(project_id, employee_id)
            .await?;
        Ok((employee, counts))
    }

    pub(crate) async fn read_employee_threads(
        &self,
        project_id: ProjectId,
        employee_id: forge_domain::EmployeeId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<forge_domain::communication::EmployeeThread>, CoreError> {
        self.read_employee(project_id, employee_id).await?;
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

    pub(crate) async fn read_message_delivery(
        &self,
        project_id: ProjectId,
        thread_id: Uuid,
        after: u64,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>, CoreError> {
        self.store()
            .load_employee_thread(project_id, thread_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "employee thread",
            })?;
        Ok(self
            .store()
            .list_message_delivery(project_id, thread_id, after, limit)
            .await?)
    }

    pub(crate) async fn read_run_diagnostics(
        &self,
        project_id: ProjectId,
        run_id: Uuid,
    ) -> Result<serde_json::Value, CoreError> {
        self.read_run(project_id, run_id).await?;
        super::run_activity::safe_diagnostics(self.store.run_diagnostics(run_id).await?)
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

    pub(crate) async fn read_pipeline_version_page(
        &self,
        project_id: ProjectId,
        after: Option<PipelineVersionId>,
        limit: u32,
    ) -> Result<Option<Vec<PipelineVersionRead>>, CoreError> {
        self.read_project(project_id).await?;
        Ok(self
            .store()
            .list_pipeline_version_page(project_id, after, limit)
            .await?
            .map(|rows| {
                rows.into_iter()
                    .map(|(pipeline, version)| PipelineVersionRead { pipeline, version })
                    .collect()
            }))
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

    pub(crate) async fn read_runs_page(
        &self,
        project_id: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Option<Vec<RunProjection>>, CoreError> {
        self.read_project(project_id).await?;
        Ok(self
            .store()
            .list_runs_for_project_page(project_id, after, limit)
            .await?)
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
