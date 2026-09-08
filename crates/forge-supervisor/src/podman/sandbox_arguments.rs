//! Shared sandbox bounds, independent of provider or execution purpose.
use forge_domain::runtime::ResourceLimits;
use forge_protocol::supervisor::v1::ProvisionRun;
pub(super) fn create(
    provision: &ProvisionRun,
    limits: &ResourceLimits,
    workdir: &str,
) -> Vec<String> {
    let uid = nix::unistd::Uid::effective().as_raw();
    let gid = nix::unistd::Gid::effective().as_raw();
    vec![
        "create".into(),
        "--name".into(),
        super::environment_name(provision),
        "--pull=never".into(),
        "--network=none".into(),
        "--http-proxy=false".into(),
        "--read-only".into(),
        "--cap-drop=all".into(),
        "--security-opt=no-new-privileges".into(),
        "--userns=keep-id".into(),
        "--user".into(),
        format!("{uid}:{gid}"),
        "--log-driver=none".into(),
        "--stop-signal=SIGINT".into(),
        "--cpus".into(),
        format!("{:.3}", f64::from(limits.cpu_millis) / 1000.0),
        "--memory".into(),
        limits.memory_bytes.to_string(),
        "--memory-swap".into(),
        limits.memory_bytes.to_string(),
        "--pids-limit".into(),
        limits.pids.to_string(),
        "--tmpfs".into(),
        "/tmp:rw,nosuid,nodev,size=268435456".into(),
        "--workdir".into(),
        workdir.into(),
        "--entrypoint=/usr/local/bin/forge-runner".into(),
    ]
}
