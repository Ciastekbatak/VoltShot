use crate::i18n::Strings;
use crate::theme::SharexPalette;
use eframe::egui::{self, RichText};
use sharex_core::{ClipboardSink, CoreError, ImageFormat, UploadRequest, UploadResult, Uploader};
use sharex_uploaders::{
    CustomUploaderStore, LocalMirrorUploader, SxcuUploader, discord_webhook_preset,
    github_gist_preset_for_request, imgur_preset, list_builtin_presets,
    pastebin_preset_for_request,
};
use std::fs;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

const CUSTOM: &str = "custom-sxcu";
const LOCAL_MIRROR: &str = "local-mirror";

enum UploadTarget {
    Uploader(Box<dyn Uploader>),
    Gist { token: String, public: bool },
    Pastebin(String),
}

impl UploadTarget {
    fn upload(self, path: &Path) -> Result<UploadResult, CoreError> {
        let file_name = path
            .file_name()
            .ok_or_else(|| CoreError::UploadFailed("The capture has no file name.".into()))?
            .to_string_lossy()
            .into_owned();
        let mime_type = path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| ext.parse::<ImageFormat>().ok())
            .map_or("application/octet-stream", ImageFormat::mime_type);
        let request = UploadRequest {
            file_name,
            mime_type: mime_type.into(),
            bytes: fs::read(path)?,
        };
        let result = match self {
            Self::Uploader(uploader) => uploader.upload(&request),
            Self::Gist { token, public } => {
                SxcuUploader::from_sxcu(github_gist_preset_for_request(&token, public, &request)?)?
                    .upload(&request)
            }
            Self::Pastebin(key) => {
                SxcuUploader::from_sxcu(pastebin_preset_for_request(&key, None, &request)?)?
                    .upload(&request)
            }
        }?;
        if result.url.trim().is_empty() {
            return Err(CoreError::UploadFailed(
                "The destination did not return a URL.".into(),
            ));
        }
        Ok(result)
    }
}

/// Configures and runs manual uploads of saved captures.
pub struct UploadersPanelState {
    destination: &'static str,
    mirror_folder: String,
    imgur_client_id: String,
    discord_webhook_url: String,
    gist_token: String,
    gist_public: bool,
    pastebin_key: String,
    sxcu_json: String,
    custom_uploaders: CustomUploaderStore,
    pending: Option<Receiver<Result<UploadResult, CoreError>>>,
    status: Option<Result<String, String>>,
    url: Option<String>,
}

impl UploadersPanelState {
    pub fn new(mirror_folder: &Path) -> Self {
        Self {
            destination: LOCAL_MIRROR,
            mirror_folder: mirror_folder.display().to_string(),
            imgur_client_id: String::new(),
            discord_webhook_url: String::new(),
            gist_token: String::new(),
            gist_public: false,
            pastebin_key: String::new(),
            sxcu_json: String::new(),
            custom_uploaders: CustomUploaderStore::new(),
            pending: None,
            status: None,
            url: None,
        }
    }

    fn load_custom_uploader(&mut self) -> Result<(), CoreError> {
        self.custom_uploaders.load(CUSTOM, &self.sxcu_json)
    }

    fn upload_target(&self) -> Result<UploadTarget, CoreError> {
        let uploader: Box<dyn Uploader> = match self.destination {
            LOCAL_MIRROR => {
                let folder = self.mirror_folder.trim();
                if folder.is_empty() {
                    return Err(CoreError::ConfigError("Enter a mirror folder.".into()));
                }
                Box::new(LocalMirrorUploader::new(folder))
            }
            "imgur" => Box::new(SxcuUploader::from_sxcu(imgur_preset(
                &self.imgur_client_id,
            )?)?),
            "discord-webhook" => Box::new(SxcuUploader::from_sxcu(discord_webhook_preset(
                &self.discord_webhook_url,
            )?)?),
            "github-gist" => {
                return Ok(UploadTarget::Gist {
                    token: self.gist_token.clone(),
                    public: self.gist_public,
                });
            }
            "pastebin" => return Ok(UploadTarget::Pastebin(self.pastebin_key.clone())),
            CUSTOM => Box::new(self.custom_uploaders.get(CUSTOM).cloned().ok_or_else(|| {
                CoreError::ConfigError("Validate and load the .sxcu first.".into())
            })?),
            _ => {
                return Err(CoreError::ConfigError(
                    "Select an upload destination.".into(),
                ));
            }
        };
        Ok(UploadTarget::Uploader(uploader))
    }

    fn start_upload(&mut self, path: &Path) -> Result<(), CoreError> {
        let target = self.upload_target()?;
        let path = path.to_path_buf();
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("sharex-upload".into())
            .spawn(move || {
                let _ = sender.send(target.upload(&path));
            })?;
        self.pending = Some(receiver);
        self.status = None;
        self.url = None;
        Ok(())
    }

    pub(crate) fn poll_upload(&mut self, clipboard: &dyn ClipboardSink) {
        let Some(receiver) = &self.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result.map_err(|err| format!("Upload failed: {err}")),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("The upload did not finish.".into()),
        };
        self.pending = None;
        self.status = Some(result.and_then(|result| {
            let copied = clipboard.copy_text(&result.url);
            self.url = Some(result.url);
            copied
                .map(|()| "Uploaded capture and copied its URL.".into())
                .map_err(|err| format!("Upload succeeded, but copying the URL failed: {err}"))
        }));
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &SharexPalette,
        s: &Strings,
        capture: Option<(&Path, bool)>,
    ) {
        ui.heading(s.uploaders);
        ui.separator();

        let local = LocalMirrorUploader::new(self.mirror_folder.trim());
        let destinations = [(LOCAL_MIRROR, local.name(), local.description())]
            .into_iter()
            .chain(
                list_builtin_presets()
                    .into_iter()
                    .map(|preset| (preset.id, preset.name, preset.description)),
            )
            .chain([(
                CUSTOM,
                "Custom .sxcu JSON",
                "Uploads using a ShareX .sxcu definition.",
            )])
            .collect::<Vec<_>>();
        let (_, name, description) = destinations
            .iter()
            .find(|(id, _, _)| *id == self.destination)
            .copied()
            .unwrap_or(destinations[0]);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_enabled_ui(self.pending.is_none(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("Destination");
                    egui::ComboBox::from_id_salt("upload_destination")
                        .selected_text(name)
                        .show_ui(ui, |ui| {
                            for (id, name, description) in destinations {
                                ui.selectable_value(&mut self.destination, id, name)
                                    .on_hover_text(description);
                            }
                        });
                });
                ui.label(RichText::new(description).color(p.text_muted));
                ui.add_space(8.0_f32);
                self.show_destination_settings(ui);
                ui.separator();

                if let Some((path, selected)) = capture {
                    ui.label(path.display().to_string());
                    let button = if selected {
                        "Upload Selected Capture"
                    } else {
                        "Upload Latest Capture"
                    };
                    if ui.button(button).clicked()
                        && let Err(err) = self.start_upload(path)
                    {
                        self.url = None;
                        self.status = Some(Err(format!("Upload failed: {err}")));
                    }
                } else {
                    ui.label(RichText::new("No saved capture available.").color(p.text_muted));
                }
            });

            if self.pending.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Uploading capture…");
                });
            }
            if let Some(status) = &self.status {
                let (message, color) = match status {
                    Ok(message) => (message, p.success),
                    Err(message) => (message, p.danger),
                };
                ui.label(RichText::new(message).color(color));
            }
            if let Some(url) = &self.url {
                ui.label(RichText::new(url).monospace());
            }
        });
    }

    fn show_destination_settings(&mut self, ui: &mut egui::Ui) {
        match self.destination {
            LOCAL_MIRROR => {
                ui.label("Mirror folder");
                ui.text_edit_singleline(&mut self.mirror_folder);
            }
            "imgur" => credential_input(ui, "Client ID", &mut self.imgur_client_id),
            "discord-webhook" => {
                credential_input(ui, "Webhook URL", &mut self.discord_webhook_url);
            }
            "github-gist" => {
                credential_input(ui, "GitHub token", &mut self.gist_token);
                ui.checkbox(&mut self.gist_public, "Public Gist");
            }
            "pastebin" => credential_input(ui, "API developer key", &mut self.pastebin_key),
            CUSTOM => {
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut self.sxcu_json)
                            .code_editor()
                            .hint_text("Paste .sxcu JSON")
                            .desired_rows(8)
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.custom_uploaders.remove(CUSTOM);
                    self.status = None;
                    self.url = None;
                }
                if ui.button("Validate & Load .sxcu").clicked() {
                    self.url = None;
                    self.status = Some(
                        self.load_custom_uploader()
                            .map(|()| "Custom uploader loaded.".into())
                            .map_err(|err| err.to_string()),
                    );
                }
            }
            _ => {}
        }
    }
}

fn credential_input(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).password(true));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn validates_and_loads_sxcu_json() {
        let mut panel = UploadersPanelState::new(Path::new("mirror"));
        panel.destination = CUSTOM;
        panel.sxcu_json = r#"{
            "Name": "Example",
            "RequestURL": "https://example.test/upload",
            "Body": "MultipartFormData",
            "FileFormName": "image"
        }"#
        .into();

        panel.load_custom_uploader().unwrap();

        let definition = panel.custom_uploaders.get(CUSTOM).unwrap().definition();
        assert_eq!(definition.name, "Example");
        assert_eq!(definition.file_form_name.as_deref(), Some("image"));
        assert!(panel.upload_target().is_ok());
    }

    #[test]
    fn rejects_malformed_and_invalid_sxcu_json() {
        let mut panel = UploadersPanelState::new(Path::new("mirror"));
        panel.destination = CUSTOM;

        for invalid in [
            "{",
            r#"{"Name": "Example", "RequestURL": "file:///capture.png"}"#,
        ] {
            panel.sxcu_json = invalid.into();
            assert!(panel.load_custom_uploader().is_err());
            assert!(panel.custom_uploaders.get(CUSTOM).is_none());
            assert!(panel.upload_target().is_err());
        }
    }

    #[test]
    fn uploads_capture_through_local_mirror() {
        let dir = tempdir().unwrap();
        let capture = dir.path().join("capture.png");
        let mirror = dir.path().join("mirror");
        fs::write(&capture, b"capture contents").unwrap();
        let mut panel = UploadersPanelState::new(&mirror);

        panel.start_upload(&capture).unwrap();
        let result = panel
            .pending
            .take()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();

        assert!(result.url.starts_with("file:///"));
        assert!(result.url.ends_with("/mirror/capture.png"));
        assert_eq!(
            fs::read(mirror.join("capture.png")).unwrap(),
            b"capture contents"
        );
    }
}
