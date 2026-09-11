use std::collections::BTreeSet;

use forge_domain::{
    CapabilityProfile, CredentialBinding, CredentialDeliveryMode, EmployeeId,
    ExecutionProfileInput, ProjectId, RuntimeCapability, TransportEngine,
    runtime::{RuntimeBinding, SurfaceAccess, SurfaceSpec, SystemJobRunSpec},
    system_job::{SystemJobAssignmentRef, SystemJobInput, SystemJobKind},
};
use uuid::Uuid;

pub fn spec(project: ProjectId, employee: EmployeeId) -> SystemJobRunSpec {
    let delivery = CredentialDeliveryMode::IsolatedRuntimeSecret;
    let binding = RuntimeBinding {
        execution_profile: ExecutionProfileInput {
            id: Uuid::now_v7(),
            revision: 1,
            project_id: project,
            adapter_id: "codex_cli".into(),
            adapter_version: "0.153.2".into(),
            provider_id: "openai".into(),
            model: "synthetic-no-inference".into(),
            credential_binding: CredentialBinding {
                id: Uuid::now_v7(),
                project_id: project,
                secret_id: Uuid::now_v7(),
                account_id: Some("synthetic-system-job-account".into()),
                allowed_delivery_modes: BTreeSet::from([delivery]),
            },
            credential_delivery: delivery,
            capability_profile: CapabilityProfile {
                adapter_id: "codex_cli".into(),
                adapter_version: "0.153.2".into(),
                transport_engine: TransportEngine::CliWrapper,
                capabilities: BTreeSet::from([
                    RuntimeCapability::ControlledStop,
                    RuntimeCapability::NativeMcp,
                ]),
                credential_exposed_to_run: true,
            },
        }
        .try_into()
        .expect("synthetic profile"),
        image: format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
        surface: SurfaceSpec::None,
        access: SurfaceAccess::ReadOnly,
        limits: Default::default(),
        budget: Default::default(),
        system_prompt: "Synthetic ownership contract; no provider calls".into(),
        employee_prompt: "Process only the frozen input".into(),
    };
    SystemJobRunSpec {
        schema_version: 7,
        project_id: project,
        run_id: Uuid::now_v7(),
        assignment: SystemJobAssignmentRef {
            job_id: Uuid::now_v7(),
            attempt_id: Uuid::now_v7(),
            generation: 1,
            kind: SystemJobKind::Onboarding,
        },
        input: SystemJobInput {
            source_task_id: None,
            target_employee_id: Some(employee),
            covered_sequence: 0,
            // SHA-256 of the exact serialized frozen context below ({}).
            source_digest: "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
                .into(),
            context: serde_json::json!({}),
        },
        max_result_bytes: 16 * 1024,
        binding,
        instruction: "Read the frozen input only".into(),
    }
}
