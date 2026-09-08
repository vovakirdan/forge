use super::*;
use forge_cli::profile_template::{ProfileLane, ProfileTemplateInput, build};

pub(super) struct Settings {
    pub credential: PathBuf,
    pub proxy: Option<(String, PathBuf)>,
    pub template: Value,
    pub lane: ProfileLane,
}

impl Settings {
    pub fn environment() -> Result<Self> {
        ensure!(
            std::env::var("FORGE_M2_LIVE").as_deref() == Ok("1"),
            "set FORGE_M2_LIVE=1 only after approving real usage"
        );
        let path =
            std::env::var("FORGE_M2_LIVE_SETTINGS").context("explicit settings path required")?;
        let path = absolute(&path)?;
        let settings = PrivateMaterialization::open(&path)?.read(64 * 1024)?;
        Self::parse(
            serde_json::from_slice(settings.expose()).context("invalid live settings JSON")?,
        )
    }

    pub fn parse(value: Value) -> Result<Self> {
        let fields = value
            .as_object()
            .context("live settings must be an object")?;
        ensure!(
            fields.keys().all(|key| matches!(
                key.as_str(),
                "lane"
                    | "credential_file"
                    | "account_id"
                    | "model"
                    | "image"
                    | "limits"
                    | "budget"
                    | "proxy_endpoint"
                    | "proxy_master_file"
            )),
            "unknown live setting; key bytes do not belong in this document"
        );
        let lane: ProfileLane = serde_json::from_value(value["lane"].clone())
            .context("explicit supported lane required")?;
        let credential = absolute(
            value["credential_file"]
                .as_str()
                .context("explicit credential_file required")?,
        )?;
        let proxy = match lane {
            ProfileLane::OpenRouterApi | ProfileLane::OpenAiApi => {
                let endpoint = value["proxy_endpoint"]
                    .as_str()
                    .context("API lane requires explicit local proxy_endpoint")?;
                let url = reqwest::Url::parse(endpoint)?;
                ensure!(
                    url.scheme() == "http"
                        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
                        && url.port().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none()
                        && url.path() == "/",
                    "live gate requires an explicit loopback HTTP proxy origin"
                );
                let master = absolute(
                    value["proxy_master_file"]
                        .as_str()
                        .context("explicit proxy master file required")?,
                )?;
                let spend = value["budget"]["max_spend_microusd"]
                    .as_u64()
                    .context("live API gate requires explicit monetary cap")?;
                ensure!(
                    spend > 0 && spend <= 5_000_000,
                    "live gate cap must be positive and at most five USD per Run"
                );
                Some((endpoint.to_owned(), master))
            }
            _ => {
                ensure!(
                    value.get("proxy_endpoint").is_none()
                        && value.get("proxy_master_file").is_none(),
                    "subscription smoke does not accept API proxy settings"
                );
                None
            }
        };
        let template = json!({
            "lane":value["lane"],"account_id":value["account_id"],"model":value["model"],"image":value["image"],
            "limits":value["limits"],"budget":value["budget"],"live_input":true,"access":"read_write",
            "system_prompt":"This is an explicitly approved bounded live transport smoke. Work only in the assigned directory. Never inspect credentials, host paths, other Tasks or environment files. Do not submit an artifact or Task outcome; the operator will stop the Run.",
            "employee_prompt":"First execute exactly this shell command in your working directory: `printf 'ready\\n' > forge-live-ready`. Then report ready and wait for the next user input. Do not start a sleep, background process, extra task or repeated command. When a later input requests another marker, execute only that requested command. Do not close the Forge Task."
        });
        let settings = Self {
            credential,
            proxy,
            template,
            lane,
        };
        let binding = settings
            .binding(
                ProjectId::new(),
                Uuid::now_v7(),
                Uuid::now_v7(),
                Uuid::now_v7(),
            )?
            .binding;
        ensure!(
            (60..=240).contains(&binding.limits.wall_seconds),
            "live smoke wall limit must be 60-240 seconds"
        );
        ensure!(
            binding.budget.max_output_bytes <= 8 * 1024 * 1024,
            "live smoke output cap must be at most 8 MiB"
        );
        Ok(settings)
    }

    pub fn binding(
        &self,
        project: ProjectId,
        employee: Uuid,
        secret: Uuid,
        binding: Uuid,
    ) -> Result<forge_cli::profile_template::ConfigureRuntimeTemplate> {
        let mut value = self.template.clone();
        value["project_id"] = json!(project);
        value["employee_id"] = json!(employee);
        value["secret_id"] = json!(secret);
        value["binding_id"] = json!(binding);
        let input: ProfileTemplateInput = serde_json::from_value(value)?;
        build(input)
    }
    pub fn credential_kind(&self) -> &'static str {
        match self.lane {
            ProfileLane::CodexCli => "codex_chatgpt",
            ProfileLane::ClaudeCli => "claude_subscription",
            _ => "api_key",
        }
    }
}

fn absolute(value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    ensure!(
        path.is_absolute() && !value.chars().any(char::is_control),
        "explicit absolute private-file path required"
    );
    Ok(path)
}

#[test]
fn live_settings_require_explicit_lane_auth_model_image_and_bounded_spend() -> Result<()> {
    assert!(Settings::parse(json!({})).is_err());
    let mut value = json!({"lane":"openai_api","credential_file":"/explicit/api-key","model":"explicit-model","image":format!("localhost/fixture@sha256:{}","0".repeat(64)),"limits":{"cpu_millis":2000,"memory_bytes":4294967296_u64,"pids":256,"wall_seconds":180,"stop_grace_seconds":5},"budget":{"max_output_bytes":1048576,"requests_per_minute":10,"tokens_per_minute":10000,"max_spend_microusd":50000},"proxy_endpoint":"http://127.0.0.1:4000","proxy_master_file":"/explicit/proxy-master"});
    Settings::parse(value.clone())?;
    value["budget"]["max_spend_microusd"] = Value::Null;
    assert!(Settings::parse(value.clone()).is_err());
    value["budget"]["max_spend_microusd"] = json!(50000);
    value["proxy_endpoint"] = json!("https://unapproved.example");
    assert!(Settings::parse(value.clone()).is_err());
    value["lane"] = json!("codex_cli");
    assert!(Settings::parse(value).is_err());
    Ok(())
}
