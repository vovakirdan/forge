use super::{audit, invalid};
use crate::{CoreError, CoreService};
use forge_domain::{DomainEventKind, system_job::SystemJobKind};
use serde_json::json;

impl CoreService {
    pub(crate) async fn reconcile_system_jobs(&self) -> Result<usize, CoreError> {
        let mut changed = 0;
        for (project_id, id) in self.store.system_job_reconciliation_ids().await? {
            let mut tx = self.store.begin().await?;
            let Some(mut project) = tx.lock_project(project_id).await? else {
                continue;
            };
            let Some(attempt) = tx.system_job_attempt(project_id, id).await? else {
                continue;
            };
            if attempt.state != "running" {
                continue;
            }
            let Some(job) = tx
                .lock_system_job(project_id, attempt.spec.assignment.job_id)
                .await?
            else {
                return Err(invalid("attempt lost its owner"));
            };
            let Some(run) = tx.load_run(attempt.spec.run_id).await? else {
                return Err(invalid("attempt lost its Run"));
            };
            if !attempt.physical_quiescent {
                if attempt.core_instance != self.instance_id
                    || attempt.deadline_expired
                    || attempt.result.is_some()
                {
                    // A previous Core instance never restores a semantic run's authority.
                    // Common Supervisor recovery must prove it stopped before retry.
                    tx.request_run_stop(
                        run.id,
                        run.lease_fencing_token,
                        run.environment_epoch,
                        attempt.deadline_expired || attempt.core_instance != self.instance_id,
                    )
                    .await?;
                }
                tx.commit().await?;
                let _ = self.deliver_pending_stop_requests(project_id).await;
                continue;
            }
            let stale = job.generation != attempt.spec.assignment.generation
                || job.covered_sequence != attempt.spec.input.covered_sequence;
            let now = crate::canonical_clock::project_mutation_time(&project);
            let mut entries = Vec::new();
            let (attempt_state, job_state, reason, new_generation) = if stale {
                (
                    "superseded",
                    "pending",
                    Some("newer_source_available"),
                    true,
                )
            } else if let Some(result) = attempt.result {
                // Result was accepted while its scope was live. Stop or restart
                // after submission does not lose the durable accepted output.
                match super::result::SystemJobResult::decode(result.clone(), &attempt.spec) {
                    Ok(parsed) => {
                        if parsed
                            .validate_sources(&mut tx, &attempt.spec)
                            .await
                            .is_err()
                        {
                            (
                                "superseded",
                                "held",
                                Some("source_no_longer_visible"),
                                false,
                            )
                        } else {
                            entries = self
                                .materialize_system_job_result(
                                    &mut tx,
                                    &project,
                                    &attempt.spec,
                                    result,
                                    now,
                                )
                                .await?;
                            if attempt.spec.assignment.kind == SystemJobKind::Onboarding {
                                let target = attempt
                                    .spec
                                    .input
                                    .target_employee_id
                                    .ok_or_else(|| invalid("onboarding target missing"))?;
                                let context = &attempt.spec.input.context;
                                tx.set_onboarding_receipt(project_id, target, "completed", json!({
                                    "job_id": job.id,
                                    "attempt_id": id,
                                    "run_id": run.id,
                                    "source_digest": attempt.spec.input.source_digest,
                                    "source_refs": context.get("source_refs"),
                                    "employee": context.get("employee"),
                                    "execution_profile_id": context.get("execution_profile_id"),
                                    "execution_profile_revision": context.get("execution_profile_revision"),
                                    "note_entries": entries,
                                    "at": now,
                                    "meaning": "familiarity_receipt_not_comprehension_or_provider_session_proof"
                                })).await?;
                            }
                            ("completed", "completed", None, false)
                        }
                    }
                    Err(_) => ("failed", "held", Some("invalid_persisted_result"), false),
                }
            } else {
                ("failed", "pending", Some("no_accepted_result"), false)
            };
            tx.retire_system_job_attempt(id, attempt_state).await?;
            tx.set_system_job_state(job.id, job_state, reason, new_generation)
                .await?;
            audit(
                &mut tx,
                &mut project,
                self.actors.core,
                DomainEventKind::SystemJobChanged,
                json!({"job_id": job.id, "attempt_id": id, "state": attempt_state,
                    "reason": reason, "entries": entries}),
            )
            .await?;
            tx.commit().await?;
            changed += 1;
        }
        Ok(changed)
    }
}
