//! One command contract executed through the production engine on both stores.

#[path = "command_conformance/active_runs.rs"]
mod active_runs;
#[path = "command_conformance/compatibility.rs"]
mod compatibility;

#[path = "command_conformance/atomicity.rs"]
mod atomicity;
#[path = "command_conformance/boundary.rs"]
mod boundary;
#[path = "command_conformance/clocks.rs"]
mod clocks;
#[path = "command_conformance/commands.rs"]
mod commands;
#[path = "command_conformance/fixture.rs"]
mod fixture;
#[path = "command_conformance/production.rs"]
mod production;
#[path = "command_conformance/scopes.rs"]
mod scopes;
#[path = "command_conformance/snapshot.rs"]
mod snapshot;

macro_rules! conformance_case {
    ($name:ident, $scenario:path) => {
        mod $name {
            #[tokio::test]
            async fn memory() -> anyhow::Result<()> {
                $scenario(crate::fixture::BackendKind::Memory).await
            }

            #[tokio::test]
            #[ignore = "requires explicit local PostgreSQL; run just test-integration"]
            async fn postgres() -> anyhow::Result<()> {
                $scenario(crate::fixture::BackendKind::Postgres).await
            }
        }
    };
}

conformance_case!(all_m0_commands, crate::commands::all_m0_commands);
conformance_case!(
    negative_domain_cases,
    crate::commands::negative_domain_cases
);
conformance_case!(idempotency, crate::boundary::idempotency);
conformance_case!(revisions, crate::boundary::revisions);
conformance_case!(authorization, crate::boundary::authorization);
conformance_case!(concurrency, crate::boundary::concurrency);
conformance_case!(
    atomic_write_failures,
    crate::atomicity::every_write_failure_rolls_back
);
conformance_case!(
    uncommitted_drop,
    crate::atomicity::dropping_successful_uncommitted_command_rolls_back
);
conformance_case!(
    causal_clock,
    crate::clocks::canonical_and_operational_time_are_distinct
);
conformance_case!(
    composite_rollback,
    crate::atomicity::composite_queue_wait_and_artifact_writes_roll_back
);
conformance_case!(
    catalog_scope,
    crate::scopes::catalog_uniqueness_and_project_scope
);
conformance_case!(
    intent_integrity,
    crate::scopes::mutated_typed_intent_cannot_diverge_from_fingerprinted_json
);
conformance_case!(
    active_run_ownership,
    crate::active_runs::stop_cancel_and_dependency_preserve_physical_ownership
);
