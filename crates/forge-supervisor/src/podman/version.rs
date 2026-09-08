//! Version discovery is a separate, credential-free container. A successful
//! digest lookup alone does not prove the selected adapter is installed.

use std::{path::Path, process::Stdio, time::Duration};

use forge_domain::runtime::RuntimeLaunchSpec;
use tokio::{io::AsyncReadExt, process::Command};
use uuid::Uuid;

use crate::SupervisorError;

const PROBE_SECONDS: u64 = 15;
const MAX_VERSION_BYTES: u64 = 4096;

pub(super) async fn verify(podman: &Path, spec: &RuntimeLaunchSpec) -> Result<(), SupervisorError> {
    let profile = &spec.binding.execution_profile;
    let (program, expected) = expected_version(profile.adapter_id(), profile.adapter_version())?;
    probe(podman, &spec.binding.image, program, expected).await
}

fn expected_version(
    adapter: &str,
    version: &str,
) -> Result<(&'static str, &'static str), SupervisorError> {
    match (adapter, version) {
        ("codex_cli", "0.153.2") => Ok(("/usr/local/bin/codex", "codex-cli 0.153.2")),
        ("opencode_runtime", "1.18.29") => Ok(("/usr/local/bin/opencode", "1.18.29")),
        ("claude_code_cli", "2.1.263") => Ok(("/usr/local/bin/claude", "2.1.263 (Claude Code)")),
        _ => Err(SupervisorError::RuntimePreflightFailed),
    }
}

fn arguments(image: &str, name: &str, program: &str) -> Vec<String> {
    let uid = nix::unistd::Uid::effective().as_raw();
    let gid = nix::unistd::Gid::effective().as_raw();
    vec![
        "run".into(),
        "--rm".into(),
        "--name".into(),
        name.into(),
        "--pull=never".into(),
        "--network=none".into(),
        "--http-proxy=false".into(),
        "--read-only".into(),
        "--cap-drop=all".into(),
        "--security-opt=no-new-privileges".into(),
        "--userns=keep-id".into(),
        format!("--user={uid}:{gid}"),
        "--log-driver=none".into(),
        "--cpus=1".into(),
        "--memory=536870912".into(),
        "--memory-swap=536870912".into(),
        "--pids-limit=64".into(),
        format!("--timeout={PROBE_SECONDS}"),
        "--workdir=/tmp".into(),
        "--tmpfs=/tmp:rw,nosuid,nodev,mode=1777,size=16777216".into(),
        "--env=HOME=/tmp".into(),
        "--env=CODEX_HOME=/tmp/codex".into(),
        "--env=CLAUDE_CONFIG_DIR=/tmp/claude".into(),
        "--env=CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1".into(),
        "--env=OPENCODE_DISABLE_AUTOUPDATE=true".into(),
        "--env=OPENCODE_DISABLE_MODELS_FETCH=true".into(),
        "--env=OPENCODE_DISABLE_PROJECT_CONFIG=true".into(),
        "--env=OPENCODE_CONFIG_CONTENT={}".into(),
        format!("--entrypoint={program}"),
        image.into(),
        "--version".into(),
    ]
}

async fn probe(
    podman: &Path,
    image: &str,
    program: &str,
    expected: &str,
) -> Result<(), SupervisorError> {
    let name = format!("forge-version-probe-{}", Uuid::now_v7());
    let mut child = Command::new(podman)
        .args(arguments(image, &name, program))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| SupervisorError::RuntimePreflightFailed)?;
    let result = tokio::time::timeout(Duration::from_secs(PROBE_SECONDS), async {
        let mut stdout = child
            .stdout
            .take()
            .ok_or(SupervisorError::RuntimePreflightFailed)?;
        let mut bytes = Vec::new();
        (&mut stdout)
            .take(MAX_VERSION_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| SupervisorError::RuntimePreflightFailed)?;
        if bytes.len() as u64 > MAX_VERSION_BYTES {
            return Err(SupervisorError::RuntimePreflightFailed);
        }
        let status = child
            .wait()
            .await
            .map_err(|_| SupervisorError::RuntimePreflightFailed)?;
        if !status.success() || std::str::from_utf8(&bytes).ok().map(str::trim) != Some(expected) {
            return Err(SupervisorError::RuntimePreflightFailed);
        }
        Ok(())
    })
    .await
    .unwrap_or(Err(SupervisorError::RuntimePreflightFailed));
    if result.is_err() {
        let _ = tokio::time::timeout(Duration::from_secs(1), child.kill()).await;
        // Killing the Podman client does not prove its container stopped. Its
        // own --timeout still applies if this best-effort cleanup is interrupted.
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new(podman)
                .args(["rm", "--force", &name])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status(),
        )
        .await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_known_adapter_version_and_rejects_unimplemented_combinations() {
        assert_eq!(
            expected_version("codex_cli", "0.153.2").unwrap().1,
            "codex-cli 0.153.2"
        );
        assert_eq!(
            expected_version("opencode_runtime", "1.18.29").unwrap().1,
            "1.18.29"
        );
        assert!(expected_version("codex_cli", "0.153.3").is_err());
        assert_eq!(
            expected_version("claude_code_cli", "2.1.263").unwrap().1,
            "2.1.263 (Claude Code)"
        );
        assert!(expected_version("claude_code_cli", "2.1.264").is_err());
        assert!(expected_version("unknown", "1").is_err());
    }

    #[test]
    fn probe_arguments_have_no_host_mounts_or_credentials_and_bound_execution() {
        let args = arguments("image@sha256:fixture", "probe", "/usr/local/bin/codex");
        for required in [
            "--network=none",
            "--http-proxy=false",
            "--pull=never",
            "--read-only",
            "--timeout=15",
            "--log-driver=none",
        ] {
            assert!(args.iter().any(|arg| arg == required));
        }
        assert!(!args.iter().any(|arg| arg.starts_with("--mount")
            || arg.starts_with("--volume")
            || arg == "-v"
            || arg == "--env-host"
            || arg.contains("/run/forge-secrets")));
        assert_eq!(args.last().map(String::as_str), Some("--version"));
    }

    #[tokio::test]
    #[ignore = "requires rootless Podman and the actual pinned runtime image"]
    async fn actual_pinned_image_probes_both_clis_without_credentials() {
        let image =
            std::env::var("FORGE_RUNTIME_PROBE_IMAGE").expect("provide a pinned runtime image");
        assert!(image.contains("@sha256:"));
        for adapter in ["codex_cli", "opencode_runtime"] {
            let version = if adapter == "codex_cli" {
                "0.153.2"
            } else {
                "1.18.29"
            };
            let (program, expected) = expected_version(adapter, version).unwrap();
            probe(Path::new("podman"), &image, program, expected)
                .await
                .unwrap();
        }
        assert!(
            probe(
                Path::new("podman"),
                &image,
                "/usr/local/bin/codex",
                "codex-cli wrong-version"
            )
            .await
            .is_err()
        );
    }
}
