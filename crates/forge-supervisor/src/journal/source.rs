//! Freeze source selection before object export; this survives restart and compaction.
use forge_protocol::supervisor::v1::ProvisionRun;

use super::{Journal, scope_key};
use crate::{SupervisorError, git::GitSourceSelection};
#[cfg(test)]
mod tests;

impl Journal {
    pub fn source_selection(&self, provision: &ProvisionRun) -> Option<GitSourceSelection> {
        self.snapshot
            .runs
            .get(&scope_key(provision))
            .and_then(|record| record.source_selection.clone())
    }

    pub fn target_established(&self, key: &str) -> bool {
        self.snapshot.established_targets.contains(key)
    }

    /// Also used before a potentially delivered create-only CAS: uncertainty is not unborn.
    pub fn mark_target_established(&mut self, key: &str) -> Result<(), SupervisorError> {
        if self.target_established(key) {
            return Ok(());
        }
        self.change(|snapshot| {
            snapshot.established_targets.insert(key.to_owned());
            Ok(())
        })
    }

    pub fn record_source_selection(
        &mut self,
        provision: &ProvisionRun,
        selection: &GitSourceSelection,
    ) -> Result<(), SupervisorError> {
        let key = scope_key(provision);
        let spec: forge_domain::runtime::RuntimeLaunchSpec =
            serde_json::from_str(&provision.run_spec_json)?;
        if spec.source_request.as_ref() != Some(&selection.request) {
            return Err(SupervisorError::InvalidRunSpec);
        }
        self.change(|snapshot| {
            let record = snapshot
                .runs
                .get_mut(&key)
                .ok_or(SupervisorError::InvalidJournalMessage)?;
            if let Some(existing) = &record.source_selection {
                return if existing == selection {
                    Ok(())
                } else {
                    Err(SupervisorError::ConflictingProvision)
                };
            }
            if selection.target_exists {
                snapshot
                    .established_targets
                    .insert(selection.target_key.clone());
            }
            record.source_selection = Some(selection.clone());
            Ok(())
        })
    }
}
