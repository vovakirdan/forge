use std::{env, error::Error, io, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let workspace_root = manifest_dir
        .parent()
        .and_then(|path| path.parent())
        .ok_or_else(|| io::Error::other("forge-protocol must be in crates/"))?;
    let proto_root = workspace_root.join("proto");
    let proto_file = proto_root.join("forge/supervisor/v1/supervisor.proto");
    let descriptor_path =
        PathBuf::from(env::var("OUT_DIR")?).join("forge_supervisor_v1_descriptor.bin");

    println!("cargo::rerun-if-changed={}", proto_file.display());

    tonic_prost_build::configure()
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .field_attribute(
            "forge.supervisor.v1.ProvisionRun.traceparent",
            "#[serde(default)]",
        )
        .build_client(true)
        .build_server(true)
        .file_descriptor_set_path(descriptor_path)
        .compile_protos(&[proto_file], &[proto_root])?;

    Ok(())
}
