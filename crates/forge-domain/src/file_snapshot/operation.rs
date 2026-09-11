use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::FileSnapshotManifest;
use crate::{Actor, ArtifactId, ProjectId, TaskId, Timestamp};

/// Exact stopped writer scope sent over authenticated local Supervisor transport.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileCaptureRequest {
    pub operation_id: Uuid,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub run_id: Uuid,
    pub fencing_token: u64,
    pub environment_epoch: u64,
    pub surface_id: Uuid,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FileSnapshotSource {
    LocalFiles { root: String, paths: Vec<String> },
    TaskSurface { request: FileCaptureRequest },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileSnapshotState {
    Pending,
    Sealed,
    Failed,
}

/// Durable request and result; Pending capture holds a Task maintenance reservation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSnapshotOperation {
    pub id: Uuid,
    pub artifact_id: ArtifactId,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub title: String,
    pub source: FileSnapshotSource,
    pub state: FileSnapshotState,
    pub manifest: Option<FileSnapshotManifest>,
    pub error_code: Option<String>,
    pub created_by: Actor,
    pub created_at: Timestamp,
}
