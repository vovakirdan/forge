//! Immutable stage requirements restrict an assigned runtime; they grant no authority.

use crate::runtime::{SurfaceAccess, SurfaceSpec};
use serde::{Deserialize, Serialize};

/// Source category required by a stage, independent of any host repository path.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageWorkspaceKind {
    /// Any explicitly configured surface, including an ephemeral workspace.
    #[default]
    Any,
    /// A Task-owned non-Git filesystem surface.
    Filesystem,
    /// A Task-owned Git surface.
    Git,
}

/// A stage's exact workspace mode, pinned with its immutable Pipeline version.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageWorkspaceRequirements {
    /// Required source category; `any` by default.
    #[serde(default)]
    pub kind: StageWorkspaceKind,
    /// Required access; never inferred from stage names or prompt text.
    pub access: SurfaceAccess,
}

impl StageWorkspaceRequirements {
    /// Checks an operator-assigned binding without increasing its permissions.
    #[must_use]
    pub fn is_compatible(&self, surface: &SurfaceSpec, access: SurfaceAccess) -> bool {
        self.access == access
            && match self.kind {
                StageWorkspaceKind::Any => true,
                StageWorkspaceKind::Filesystem => matches!(surface, SurfaceSpec::FilesystemSandbox),
                StageWorkspaceKind::Git => matches!(
                    surface,
                    SurfaceSpec::GitWorktree { .. } | SurfaceSpec::GitCandidateSnapshot { .. }
                ),
            }
    }
}
