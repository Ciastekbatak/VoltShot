pub mod http_uploader;
pub mod sxcu;
pub mod template;

pub use http_uploader::{CustomUploaderStore, SxcuUploader};
pub use sxcu::{HttpMethod, SxcuBodyType, SxcuDefinition};
pub use template::{ResponseContext, evaluate_template};

use sharex_core::{
    CoreError, UploadRequest, UploadResult, Uploader, UploaderLookup, resolve_unique_path,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Public metadata describing a registered uploader destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploaderMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

/// Reference offline uploader that mirrors uploaded files into a target directory.
#[derive(Debug, Clone)]
pub struct LocalMirrorUploader {
    target_dir: PathBuf,
}

impl LocalMirrorUploader {
    pub fn new(target_dir: impl Into<PathBuf>) -> Self {
        Self {
            target_dir: target_dir.into(),
        }
    }

    pub fn target_dir(&self) -> &Path {
        &self.target_dir
    }
}

impl Uploader for LocalMirrorUploader {
    fn id(&self) -> &'static str {
        "local-mirror"
    }

    fn name(&self) -> &'static str {
        "Local Mirror"
    }

    fn description(&self) -> &'static str {
        "Copies captures to a secondary local folder and returns a file:// URL."
    }

    fn upload(&self, request: &UploadRequest) -> Result<UploadResult, CoreError> {
        fs::create_dir_all(&self.target_dir)?;
        let path = Path::new(&request.file_name);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("upload");
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| !e.is_empty())
            .unwrap_or("bin");

        let destination = resolve_unique_path(&self.target_dir, stem, ext);
        fs::write(&destination, &request.bytes)?;
        let canonical = destination.canonicalize().unwrap_or(destination);
        let normalized = canonical.to_string_lossy().replace('\\', "/");
        let trimmed = normalized.strip_prefix("//?/").unwrap_or(&normalized);
        Ok(UploadResult {
            url: format!("file:///{}", trimmed.trim_start_matches('/')),
        })
    }
}

/// Ordered registry of available uploaders.
#[derive(Default)]
pub struct UploaderRegistry {
    uploaders: BTreeMap<&'static str, Box<dyn Uploader>>,
}

impl UploaderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_local_mirror(target_dir: impl Into<PathBuf>) -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(LocalMirrorUploader::new(target_dir)));
        registry
    }

    pub fn register(&mut self, uploader: Box<dyn Uploader>) {
        self.uploaders.insert(uploader.id(), uploader);
    }

    pub fn list_metadata(&self) -> Vec<UploaderMeta> {
        self.uploaders
            .values()
            .map(|u| UploaderMeta {
                id: u.id(),
                name: u.name(),
                description: u.description(),
            })
            .collect()
    }
}

impl UploaderLookup for UploaderRegistry {
    fn get(&self, id: &str) -> Option<&dyn Uploader> {
        self.uploaders.get(id).map(Box::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn local_mirror_uploader_writes_payload_and_returns_file_url() {
        let dir = tempdir().expect("tempdir");
        let registry = UploaderRegistry::with_local_mirror(dir.path().join("mirror"));

        let meta = registry.list_metadata();
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].id, "local-mirror");

        let uploader = registry.get("local-mirror").expect("registered");
        let result = uploader
            .upload(&UploadRequest {
                file_name: "demo.png".into(),
                mime_type: "image/png".into(),
                bytes: b"png-bytes".to_vec(),
            })
            .expect("upload succeeds");

        assert!(result.url.starts_with("file:///"));
        assert!(result.url.ends_with("demo.png"));
        assert_eq!(
            fs::read(dir.path().join("mirror").join("demo.png")).expect("read mirrored file"),
            b"png-bytes"
        );
    }
}
