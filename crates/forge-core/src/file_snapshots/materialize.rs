use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use forge_domain::runtime::RuntimeLaunchSpec;
use forge_provider_common::{PrivateMaterialization, SecretBytes};
use forge_storage::RunProjection;

use super::invalid;
use crate::{CoreError, CoreService, runtime_preparation::secure_directory};

impl CoreService {
    pub(crate) async fn prepare_file_inputs(
        &self,
        run: &RunProjection,
        spec: &RuntimeLaunchSpec,
    ) -> Result<(), CoreError> {
        if spec.file_inputs.is_empty() {
            return Ok(());
        }
        let execution = self
            .execution
            .as_ref()
            .ok_or_else(|| invalid("execution root unavailable"))?;
        let objects = self
            .evidence
            .as_ref()
            .and_then(|runtime| runtime.object_store.as_ref())
            .ok_or_else(|| invalid("file inputs require object storage"))?;
        let inputs = execution.root.join("file-inputs");
        secure_directory(&inputs)?;
        let run_root = inputs.join(run.id.to_string());
        secure_directory(&run_root)?;
        let epoch = run_root.join(run.environment_epoch.to_string());
        secure_directory(&epoch)?;
        for input in &spec.file_inputs {
            input.validate(run.project_id)?;
            let artifact = epoch.join(input.artifact_id.to_string());
            secure_directory(&artifact)?;
            let data = artifact.join("files");
            secure_directory(&data)?;
            for file in &input.manifest.files {
                let bytes = objects
                    .read_snapshot_file(file)
                    .await
                    .map_err(|_| invalid("file input unavailable or corrupt"))?;
                let path = data.join(&file.path);
                let parent = path.parent().ok_or_else(|| invalid("invalid input path"))?;
                let mut directory = data.clone();
                for component in parent
                    .strip_prefix(&data)
                    .map_err(|_| invalid("invalid input path"))?
                    .components()
                {
                    directory.push(component.as_os_str());
                    secure_directory(&directory)?;
                }
                write_input(&path, &bytes, file.executable)?;
            }
            let manifest =
                serde_json::to_vec(input).map_err(|_| invalid("invalid input manifest"))?;
            write_input(&artifact.join("manifest.json"), &manifest, false)?;
        }
        let manifest =
            serde_json::to_vec(&spec.file_inputs).map_err(|_| invalid("invalid input manifest"))?;
        write_input(&epoch.join("inputs.json"), &manifest, false)?;
        Ok(())
    }
}

fn write_input(path: &Path, bytes: &[u8], executable: bool) -> Result<(), CoreError> {
    if path
        .try_exists()
        .map_err(|_| invalid("input staging unavailable"))?
    {
        let parent = path
            .parent()
            .ok_or_else(|| invalid("input staging unavailable"))?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid("input staging unavailable"))?;
        let existing = forge_provider_common::selected_file::read_selected_file(
            parent,
            Path::new(name),
            bytes.len() as u64,
        )
        .map_err(|_| invalid("unsafe retained input"))?;
        if existing.bytes != bytes || existing.executable != executable {
            return Err(invalid("retained input differs from frozen Run"));
        }
        return Ok(());
    }
    PrivateMaterialization::create(path, &SecretBytes::new(bytes.to_vec()))
        .map_err(|_| invalid("input staging unavailable"))?;
    if executable {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| invalid("input mode unavailable"))?;
    }
    Ok(())
}
