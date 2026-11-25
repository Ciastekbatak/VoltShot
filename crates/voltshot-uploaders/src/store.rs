use crate::S3Config;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;
use tempfile::NamedTempFile;
use voltshot_core::CoreError;

/// Persisted uploader destinations and shortener settings.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UploaderSettingsFile {
    /// Identifier of the selected upload destination.
    pub selected_destination: String,
    /// Directory used by the local mirror destination.
    pub mirror_folder: Option<String>,
    /// Optional S3 connection and credentials.
    pub s3: Option<S3Config>,
    /// Raw VoltShot `.sxcu` configurations, which may contain credentials.
    pub custom_sxcu_json: Vec<String>,
    /// Identifier of the selected shortener, if any.
    pub active_shortener: Option<String>,
}

impl Default for UploaderSettingsFile {
    fn default() -> Self {
        Self {
            selected_destination: "local-mirror".into(),
            mirror_folder: None,
            s3: None,
            custom_sxcu_json: Vec::new(),
            active_shortener: None,
        }
    }
}

impl fmt::Debug for UploaderSettingsFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploaderSettingsFile")
            .field("selected_destination", &self.selected_destination)
            .field("mirror_folder", &self.mirror_folder)
            .field("s3", &self.s3)
            .field("custom_sxcu_count", &self.custom_sxcu_json.len())
            .field("active_shortener", &self.active_shortener)
            .finish()
    }
}

impl UploaderSettingsFile {
    /// Write settings atomically, creating parent directories and using Unix mode `0600`.
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let mut file = NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        serde_json::to_writer_pretty(&mut file, self).map_err(|error| {
            CoreError::ConfigError(format!("could not encode uploader settings: {error}"))
        })?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    /// Load JSON settings, returning defaults only if the file does not exist.
    pub fn load_or_default(path: &Path) -> Result<Self, CoreError> {
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error.into()),
        };
        serde_json::from_slice(&data)
            .map_err(|error| CoreError::ConfigError(format!("invalid uploader settings: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trips_destination_credentials_and_shortener_in_nested_directory() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("nested/user/uploaders.json");
        let settings = UploaderSettingsFile {
            selected_destination: "s3".into(),
            mirror_folder: Some("backup".into()),
            s3: Some(S3Config {
                endpoint: "https://s3.example.test".into(),
                region: "auto".into(),
                bucket: "captures".into(),
                access_key_id: "test-access-id".into(),
                secret_access_key: "test-access-secret".into(),
                key_prefix: "images".into(),
                custom_domain: Some("https://cdn.example.test".into()),
                use_path_style: true,
                public_read_acl: false,
            }),
            custom_sxcu_json: vec![
                r#"{"Name":"test","Headers":{"Authorization":"test-custom-secret"}}"#.into(),
            ],
            active_shortener: Some("tinyurl".into()),
        };
        settings.save(&path).unwrap();
        assert_eq!(
            UploaderSettingsFile::load_or_default(&path).unwrap(),
            settings
        );
        let debug = format!("{settings:?}");
        for secret in ["test-access-id", "test-access-secret", "test-custom-secret"] {
            assert!(!debug.contains(secret));
        }

        let new_settings = UploaderSettingsFile::default();
        new_settings.save(&path).unwrap();
        assert_eq!(
            UploaderSettingsFile::load_or_default(&path).unwrap(),
            new_settings
        );
    }

    #[test]
    fn missing_file_and_partial_json_use_defaults_but_malformed_json_fails() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("uploaders.json");
        let defaults = UploaderSettingsFile::default();
        assert_eq!(
            UploaderSettingsFile::load_or_default(&path).unwrap(),
            defaults
        );

        fs::write(&path, br#"{"active_shortener":"is-gd"}"#).unwrap();
        let partial = UploaderSettingsFile::load_or_default(&path).unwrap();
        assert_eq!(
            partial,
            UploaderSettingsFile {
                active_shortener: Some("is-gd".into()),
                ..defaults
            }
        );

        fs::write(&path, b"not JSON").unwrap();
        assert!(matches!(
            UploaderSettingsFile::load_or_default(&path),
            Err(CoreError::ConfigError(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn new_and_replaced_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().unwrap();
        let path = directory.path().join("keys.json");
        let settings = UploaderSettingsFile::default();
        settings.save(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        settings.save(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
