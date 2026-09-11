use super::*;

pub(super) struct Setup {
    pub root: std::path::PathBuf,
    pub harness: M0Harness,
    pub supervisor: forge_testkit::m0::ManualSupervisor,
    pub project: forge_domain::ProjectId,
    pub employee: forge_domain::EmployeeId,
    pub binding: RuntimeBinding,
    pub version: forge_domain::PipelineVersionId,
}

pub(super) async fn create(mode: &str) -> Result<Setup> {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let with_review = matches!(
        mode,
        "review_pass" | "review_rework" | "qa_report" | "qa_writer"
    );
    let with_integration = mode.starts_with("integration_");
    let root = std::env::temp_dir().join(format!("fgit-{}", Uuid::now_v7()));
    std::fs::DirBuilder::new().mode(0o700).create(&root)?;
    let secrets = SecretStore::initialize(&root.join("secrets"), MasterKeySource::OwnerOnlyFile)?;
    let shared = root.clone();
    let harness = M0Harness::start_configured(move |core| {
        Ok(core
            .with_secret_store(secrets)
            .with_execution_root(shared)?)
    })
    .await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let project = harness.create_project("Git proposal fixture").await?;
    let secret = Uuid::now_v7();
    let credential = Uuid::now_v7();
    let token = PrivateMaterialization::create(
        &root.join("token"),
        &SecretBytes::new(b"sk-ant-oat01-synthetic-no-inference".to_vec()),
    )?;
    harness.execute(project,CommandName::EnrollCredential,json!({"secret_id":secret,"binding_id":credential,"kind":"claude_subscription","source_file":token.path()})).await?;
    harness.create_employee(project, "Writer").await?;
    let employee = harness
        .store
        .list_employees(project)
        .await?
        .remove(0)
        .employee
        .id();
    let delivery = CredentialDeliveryMode::IsolatedRuntimeSecret;
    let binding = RuntimeBinding {
        budget: Default::default(),
        execution_profile: ExecutionProfileInput {
            id: Uuid::now_v7(),
            revision: 1,
            project_id: project,
            adapter_id: "claude_code_cli".into(),
            adapter_version: "2.1.263".into(),
            provider_id: "anthropic".into(),
            model: "no-inference".into(),
            credential_binding: CredentialBinding {
                id: credential,
                project_id: project,
                secret_id: secret,
                account_id: Some("fixture".into()),
                allowed_delivery_modes: BTreeSet::from([delivery]),
            },
            credential_delivery: delivery,
            capability_profile: ClaudeAdapter::capabilities(),
        }
        .try_into()?,
        image: format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
        surface: SurfaceSpec::FilesystemSandbox,
        access: SurfaceAccess::ReadWrite,
        limits: Default::default(),
        system_prompt: "Fixture".into(),
        employee_prompt: "Fixture".into(),
    };
    harness
        .execute(
            project,
            CommandName::ConfigureEmployeeRuntime,
            json!({"employee_id":employee,"binding":binding}),
        )
        .await?;
    if with_review {
        harness
            .create_employee(project, "Independent examiner")
            .await?;
        let reviewer = harness
            .store
            .list_employees(project)
            .await?
            .into_iter()
            .find(|entry| entry.employee.id() != employee)
            .context("reviewer")?
            .employee
            .id();
        let mut readonly_binding = binding.clone();
        readonly_binding.access = SurfaceAccess::ReadOnly;
        harness
            .execute(
                project,
                CommandName::ConfigureEmployeeRuntime,
                json!({"employee_id":reviewer,"binding":readonly_binding}),
            )
            .await?;
    }
    let version = harness
        .create_pipeline(
            project,
            if mode == "qa_writer" {
                qa_writer::pipeline()
            } else if with_integration {
                integration::pipeline(mode)
            } else if mode == "unbound_policy" {
                review::unbound_pipeline()
            } else if with_review {
                review::pipeline(mode)
            } else {
                single_stage_pipeline()
            },
        )
        .await?;
    Ok(Setup {
        root,
        harness,
        supervisor,
        project,
        employee,
        binding,
        version,
    })
}
