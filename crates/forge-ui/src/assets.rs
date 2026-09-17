//! Verified startup snapshot: requests never open filesystem paths.

use std::{
    collections::HashMap,
    fs,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Component, Path},
};

use bytes::Bytes;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const FILE_LIMIT: usize = 8 * 1024 * 1024;
const TOTAL_LIMIT: usize = 32 * 1024 * 1024;
const MANIFEST_LIMIT: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("invalid or unavailable live asset snapshot")]
pub struct AssetError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    files: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    sha256: String,
}

#[derive(Clone)]
pub(crate) struct Asset {
    pub body: Bytes,
    pub content_type: &'static str,
}

pub(crate) struct AssetSnapshot(HashMap<String, Asset>);

impl AssetSnapshot {
    pub fn load(root: &Path) -> Result<Self, AssetError> {
        reject_symlink_components(root)?;
        let root = fs::canonicalize(root).map_err(|_| AssetError)?;
        if !root.is_dir() {
            return Err(AssetError);
        }
        let manifest_path = root.join("forge-live-manifest.json");
        let manifest: Manifest =
            serde_json::from_slice(&read_bounded(&manifest_path, MANIFEST_LIMIT)?)
                .map_err(|_| AssetError)?;
        if manifest.format != "forge-live-v1" || manifest.files.len() > 256 {
            return Err(AssetError);
        }
        let mut assets = HashMap::new();
        let mut total = 0usize;
        for entry in manifest.files {
            if !valid_relative_path(&entry.path)
                || entry.sha256.len() != 64
                || !entry
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(AssetError);
            }
            let path = root.join(&entry.path);
            reject_symlink_components(&path)?;
            if !fs::canonicalize(&path)
                .map_err(|_| AssetError)?
                .starts_with(&root)
            {
                return Err(AssetError);
            }
            let body = read_bounded(&path, FILE_LIMIT)?;
            total = total.checked_add(body.len()).ok_or(AssetError)?;
            if total > TOTAL_LIMIT {
                return Err(AssetError);
            }
            let hash = format!("{:x}", Sha256::digest(&body));
            if hash != entry.sha256 {
                return Err(AssetError);
            }
            let content_type = content_type(&entry.path).ok_or(AssetError)?;
            if entry.path.ends_with(".html") && entry.path != "index.html" {
                return Err(AssetError);
            }
            if assets
                .insert(
                    format!("/{}", entry.path),
                    Asset {
                        body: Bytes::from(body),
                        content_type,
                    },
                )
                .is_some()
            {
                return Err(AssetError);
            }
        }
        if !assets.contains_key("/index.html") {
            return Err(AssetError);
        }
        Ok(Self(assets))
    }

    pub fn get(&self, path: &str) -> Option<&Asset> {
        self.0.get(if path == "/" { "/index.html" } else { path })
    }
}

fn valid_relative_path(path: &str) -> bool {
    let allowed = path == "index.html"
        || path
            .strip_prefix("assets/")
            .is_some_and(|name| !name.is_empty() && !name.contains('/') && !name.starts_with('.'));
    allowed
        && !path.starts_with('/')
        && !path.contains("//")
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-/".contains(&byte))
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && !path.split('/').any(|part| part.starts_with('.'))
}

fn reject_symlink_components(path: &Path) -> Result<(), AssetError> {
    let mut current = std::path::PathBuf::new();
    for part in path.components() {
        if matches!(part, Component::ParentDir) {
            return Err(AssetError);
        }
        current.push(part);
        if fs::symlink_metadata(&current)
            .map_err(|_| AssetError)?
            .file_type()
            .is_symlink()
        {
            return Err(AssetError);
        }
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, AssetError> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| AssetError)?;
    let metadata = file.metadata().map_err(|_| AssetError)?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(AssetError);
    }
    let mut body = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|_| AssetError)?;
    if body.len() > limit {
        return Err(AssetError);
    }
    Ok(body)
}

fn content_type(path: &str) -> Option<&'static str> {
    match Path::new(path).extension()?.to_str()? {
        "html" => Some("text/html; charset=utf-8"),
        "js" => Some("text/javascript; charset=utf-8"),
        "css" => Some("text/css; charset=utf-8"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "svg" => Some("image/svg+xml"),
        "ico" => Some("image/x-icon"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "woff" => Some("font/woff"),
        "woff2" => Some("font/woff2"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(body: &[u8], path: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("index.html"), body).unwrap();
        fs::write(dir.path().join("forge-live-manifest.json"), serde_json::to_vec(&serde_json::json!({
            "format":"forge-live-v1", "files":[{"path":path,"sha256":format!("{:x}",Sha256::digest(body))}]
        })).unwrap()).unwrap();
        dir
    }

    #[test]
    fn immutable_snapshot_survives_file_change_and_never_falls_back() {
        let dir = fixture(b"original", "index.html");
        let snapshot = AssetSnapshot::load(dir.path()).unwrap();
        fs::write(dir.path().join("index.html"), b"changed").unwrap();
        assert_eq!(snapshot.get("/").unwrap().body, "original");
        for path in [
            "/api",
            "/../index.html",
            "/%2e%2e/index.html",
            "/missing.js",
            "/forge-live-manifest.json",
        ] {
            assert!(snapshot.get(path).is_none());
        }
    }

    #[test]
    fn rejects_hash_mismatch_path_escape_symlink_and_oversize() {
        let dir = fixture(b"original", "index.html");
        fs::write(dir.path().join("index.html"), b"changed").unwrap();
        assert!(AssetSnapshot::load(dir.path()).is_err());
        let escape = fixture(b"original", "../index.html");
        assert!(AssetSnapshot::load(escape.path()).is_err());
        let link = fixture(b"original", "index.html");
        fs::rename(link.path().join("index.html"), link.path().join("source")).unwrap();
        std::os::unix::fs::symlink("source", link.path().join("index.html")).unwrap();
        assert!(AssetSnapshot::load(link.path()).is_err());
        let large = fixture(&vec![b'x'; FILE_LIMIT + 1], "index.html");
        assert!(AssetSnapshot::load(large.path()).is_err());
    }

    #[test]
    fn rejects_wrong_manifest_duplicate_entries_and_total_snapshot_overflow() {
        let directory = fixture(b"original", "index.html");
        let manifest = directory.path().join("forge-live-manifest.json");
        for value in [
            serde_json::json!({"format":"demo","files":[]}),
            serde_json::json!({"format":"forge-live-v1","files":[]}),
            serde_json::json!({"format":"forge-live-v1","extra":"unexpected","files":[]}),
        ] {
            fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(AssetSnapshot::load(directory.path()).is_err());
        }
        let entry = serde_json::json!({"path":"index.html","sha256":format!("{:x}",Sha256::digest(b"original"))});
        fs::write(&manifest,serde_json::to_vec(&serde_json::json!({"format":"forge-live-v1","files":[entry.clone(),entry.clone()]})).unwrap()).unwrap();
        assert!(AssetSnapshot::load(directory.path()).is_err());

        fs::create_dir(directory.path().join("assets")).unwrap();
        let large = vec![b'x'; FILE_LIMIT];
        let hash = format!("{:x}", Sha256::digest(&large));
        let mut files = vec![entry];
        for index in 0..4 {
            let path = format!("assets/chunk-{index}.js");
            fs::write(directory.path().join(&path), &large).unwrap();
            files.push(serde_json::json!({"path":path,"sha256":hash}));
        }
        fs::write(
            &manifest,
            serde_json::to_vec(&serde_json::json!({"format":"forge-live-v1","files":files}))
                .unwrap(),
        )
        .unwrap();
        assert!(AssetSnapshot::load(directory.path()).is_err());
    }
}
