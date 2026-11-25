use crate::i18n::Strings;
use crate::theme::{VoltShotPalette, view_heading};
use eframe::egui::{self, RichText};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use voltshot_core::{
    AppConfig, ClipboardSink, CoreError, ImageFormat, UploadRequest, UploadResult, Uploader,
};
use voltshot_uploaders::{
    CustomUploaderStore, IsGdShortener, LocalMirrorUploader, QueuedUploadStatus, RetryPolicy,
    S3Config, S3Uploader, SxcuUploader, TinyUrlShortener, UploadQueue, UploaderRegistry,
    UploaderSettingsFile, UrlShortener, discord_webhook_preset, github_gist_preset_for_request,
    imgur_preset, pastebin_preset_for_request,
};

const CUSTOM: &str = "custom-sxcu";
const LOCAL_MIRROR: &str = "local-mirror";
const S3: &str = "s3";
const DESTINATIONS: [&str; 7] = [
    LOCAL_MIRROR,
    "imgur",
    "discord-webhook",
    "github-gist",
    "pastebin",
    S3,
    CUSTOM,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShortenerChoice {
    None,
    IsGd,
    TinyUrl,
}

impl ShortenerChoice {
    const ALL: [Self; 3] = [Self::None, Self::IsGd, Self::TinyUrl];

    fn id(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::IsGd => Some("is-gd"),
            Self::TinyUrl => Some("tinyurl"),
        }
    }

    fn label(self, s: &Strings) -> &'static str {
        match self {
            Self::None => s.none,
            Self::IsGd => "is.gd",
            Self::TinyUrl => "TinyURL",
        }
    }

    fn from_id(id: Option<&str>) -> Result<Self, CoreError> {
        Self::ALL
            .into_iter()
            .find(|choice| choice.id() == id)
            .ok_or_else(|| {
                CoreError::ConfigError(format!("unknown URL shortener: {}", id.unwrap_or_default()))
            })
    }

    fn shorten(self, url: &str) -> Result<String, CoreError> {
        match self {
            Self::None => Err(CoreError::ConfigError("select a URL shortener".into())),
            Self::IsGd => IsGdShortener::new().shorten(url),
            Self::TinyUrl => TinyUrlShortener::new().shorten(url),
        }
    }
}

fn web_url_input(input: &str) -> Option<&str> {
    let url = input.trim();
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    ((scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http"))
        && !authority.is_empty()
        && !url.chars().any(|character| {
            character.is_whitespace() || character.is_control() || character == '\\'
        }))
    .then_some(url)
}

enum UploadTarget {
    Uploader(Box<dyn Uploader>),
    Gist { token: String, public: bool },
    Pastebin(String),
}

enum UploadPayload {
    File(PathBuf),
    Text { file_name: String, contents: String },
}

impl UploadPayload {
    fn filename(&self) -> String {
        match self {
            Self::File(path) => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Self::Text { file_name, .. } if !file_name.trim().is_empty() => {
                file_name.trim().to_owned()
            }
            Self::Text { .. } => "snippet.txt".to_owned(),
        }
    }

    fn into_request(self) -> Result<UploadRequest, CoreError> {
        let file_name = self.filename();
        match self {
            Self::File(path) => {
                if file_name.is_empty() {
                    return Err(CoreError::UploadFailed(
                        "The capture has no file name.".into(),
                    ));
                }
                let mime_type = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .and_then(|ext| ext.parse::<ImageFormat>().ok())
                    .map_or("application/octet-stream", ImageFormat::mime_type);
                Ok(UploadRequest {
                    file_name,
                    mime_type: mime_type.into(),
                    bytes: fs::read(&path)?,
                })
            }
            Self::Text { contents, .. } => Ok(UploadRequest {
                file_name,
                mime_type: "text/plain; charset=utf-8".into(),
                bytes: contents.into_bytes(),
            }),
        }
    }
}

struct SelectedUploader {
    id: &'static str,
    target: UploadTarget,
}

impl Uploader for SelectedUploader {
    fn id(&self) -> &'static str {
        self.id
    }

    fn name(&self) -> &'static str {
        self.id
    }

    fn description(&self) -> &'static str {
        self.id
    }

    fn upload(&self, request: &UploadRequest) -> Result<UploadResult, CoreError> {
        let result = match &self.target {
            UploadTarget::Uploader(uploader) => uploader.upload(request),
            UploadTarget::Gist { token, public } => {
                SxcuUploader::from_sxcu(github_gist_preset_for_request(token, *public, request)?)?
                    .upload(request)
            }
            UploadTarget::Pastebin(key) => {
                SxcuUploader::from_sxcu(pastebin_preset_for_request(key, None, request)?)?
                    .upload(request)
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

struct Notice {
    label: fn(&Strings) -> &'static str,
    detail: Option<String>,
}

impl Notice {
    fn new(label: fn(&Strings) -> &'static str) -> Self {
        Self {
            label,
            detail: None,
        }
    }

    fn with_detail(label: fn(&Strings) -> &'static str, detail: impl ToString) -> Self {
        Self {
            label,
            detail: Some(detail.to_string()),
        }
    }

    fn text(&self, s: &Strings) -> String {
        let label = (self.label)(s);
        self.detail
            .as_ref()
            .map_or_else(|| label.to_owned(), |detail| format!("{label}: {detail}"))
    }
}

struct CompletedUpload {
    url: String,
    shortener_error: Option<CoreError>,
}

struct UploadWorkerOutput {
    queue: UploadQueue,
    result: Result<CompletedUpload, String>,
}

type QueueRow = (String, QueuedUploadStatus, u32);

/// Configures and runs manual uploads of saved captures and text snippets.
pub struct UploadersPanelState {
    destination: &'static str,
    mirror_folder: String,
    imgur_client_id: String,
    discord_webhook_url: String,
    gist_token: String,
    gist_public: bool,
    pastebin_key: String,
    text_file_name: String,
    text_payload: String,
    sxcu_json: String,
    custom_uploaders: CustomUploaderStore,
    additional_sxcu_json: Vec<String>,
    s3: S3Config,
    settings_path: PathBuf,
    queue: UploadQueue,
    queue_rows: Vec<QueueRow>,
    pending: Option<Receiver<UploadWorkerOutput>>,
    status: Option<Result<Notice, Notice>>,
    settings_status: Option<Result<Notice, Notice>>,
    url: Option<String>,
    shortener: ShortenerChoice,
    auto_shorten: bool,
    shortener_input: String,
    shortened_url: Option<String>,
    shortener_status: Option<Notice>,
    pending_shortening: Option<Receiver<Result<String, CoreError>>>,
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
            text_file_name: "snippet.txt".into(),
            text_payload: String::new(),
            sxcu_json: String::new(),
            custom_uploaders: CustomUploaderStore::new(),
            additional_sxcu_json: Vec::new(),
            s3: default_s3_config(),
            settings_path: AppConfig::default_config_path().with_file_name("uploaders.json"),
            queue: UploadQueue::new(),
            queue_rows: Vec::new(),
            pending: None,
            status: None,
            settings_status: None,
            url: None,
            shortener: ShortenerChoice::None,
            auto_shorten: false,
            shortener_input: String::new(),
            shortened_url: None,
            shortener_status: None,
            pending_shortening: None,
        }
    }

    /// Set the location used by the Save and Load Uploader Settings controls.
    pub fn configure_settings_path(&mut self, path: PathBuf) {
        self.settings_path = path;
    }

    pub(crate) fn mirror_folder_path(&self) -> Option<PathBuf> {
        let trimmed = self.mirror_folder.trim();
        (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
    }

    fn is_text_destination(&self) -> bool {
        matches!(self.destination, "github-gist" | "pastebin")
    }

    fn load_custom_uploader(&mut self) -> Result<(), CoreError> {
        self.custom_uploaders.load(CUSTOM, &self.sxcu_json)
    }

    fn upload_target(&self) -> Result<UploadTarget, CoreError> {
        self.upload_target_for(self.destination)
    }

    fn upload_target_for(&self, destination: &str) -> Result<UploadTarget, CoreError> {
        let uploader: Box<dyn Uploader> = match destination {
            LOCAL_MIRROR => {
                let folder = self.mirror_folder.trim();
                if folder.is_empty() {
                    return Err(CoreError::ConfigError("Enter a mirror folder.".into()));
                }
                Box::new(LocalMirrorUploader::new(folder))
            }
            S3 => Box::new(S3Uploader::new(self.s3.clone())?),
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

    pub(crate) fn workflow_uploader(&self, id: &str) -> Result<impl Uploader + use<>, CoreError> {
        let id = DESTINATIONS
            .into_iter()
            .find(|destination| *destination == id)
            .ok_or_else(|| CoreError::UnknownUploader(id.to_owned()))?;
        Ok(SelectedUploader {
            id,
            target: self.upload_target_for(id)?,
        })
    }

    fn start_upload(&mut self, path: &Path) -> Result<(), CoreError> {
        self.start_upload_payload(UploadPayload::File(path.to_path_buf()))
    }

    fn start_text_upload(&mut self) -> Result<(), CoreError> {
        if self.text_payload.trim().is_empty() {
            return Err(CoreError::ConfigError(
                "Enter UTF-8 text to upload to a text destination.".into(),
            ));
        }
        self.start_upload_payload(UploadPayload::Text {
            file_name: self.text_file_name.clone(),
            contents: self.text_payload.clone(),
        })
    }

    fn start_upload_payload(&mut self, payload: UploadPayload) -> Result<(), CoreError> {
        let uploader = SelectedUploader {
            id: self.destination,
            target: self.upload_target()?,
        };
        let shortener = if self.auto_shorten {
            self.shortener
        } else {
            ShortenerChoice::None
        };
        let filename = payload.filename();
        let (queue_sender, queue_receiver) = mpsc::channel();
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("voltshot-upload".into())
            .spawn(move || {
                if let Ok(mut queue) = queue_receiver.recv() {
                    let result = payload
                        .into_request()
                        .map_err(|error| error.to_string())
                        .and_then(|request| {
                            process_upload(&mut queue, uploader, request, shortener)
                        });
                    let _ = sender.send(UploadWorkerOutput { queue, result });
                }
            })?;
        if let Err(error) = queue_sender.send(std::mem::take(&mut self.queue)) {
            self.queue = error.0;
            return Err(CoreError::UploadFailed("upload worker disconnected".into()));
        }
        self.queue_rows
            .push((filename, QueuedUploadStatus::Pending, 0));
        self.pending = Some(receiver);
        self.status = None;
        self.url = None;
        Ok(())
    }

    fn start_shortening(&mut self) -> Result<(), Notice> {
        let url = web_url_input(&self.shortener_input)
            .ok_or_else(|| Notice::new(|s| s.shortener_invalid_url))?
            .to_owned();
        let shortener = self.shortener;
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("voltshot-url-shortener".into())
            .spawn(move || {
                let _ = sender.send(shortener.shorten(&url));
            })
            .map_err(|error| Notice::with_detail(|s| s.shortening_failed, error))?;
        self.pending_shortening = Some(receiver);
        self.shortened_url = None;
        self.shortener_status = None;
        Ok(())
    }

    fn save_settings(&self) -> Result<(), CoreError> {
        let mut custom_sxcu_json = vec![self.sxcu_json.clone()];
        custom_sxcu_json.extend(self.additional_sxcu_json.iter().cloned());
        if custom_sxcu_json.len() == 1 && self.sxcu_json.trim().is_empty() {
            custom_sxcu_json.clear();
        }
        UploaderSettingsFile {
            selected_destination: self.destination.to_owned(),
            mirror_folder: Some(self.mirror_folder.clone()),
            s3: Some(self.s3.clone()),
            custom_sxcu_json,
            active_shortener: self
                .shortener
                .id()
                .filter(|_| self.auto_shorten)
                .map(str::to_owned),
        }
        .save(&self.settings_path)
    }

    fn load_settings(&mut self) -> Result<(), CoreError> {
        let settings = UploaderSettingsFile::load_or_default(&self.settings_path)?;
        let destination = DESTINATIONS
            .into_iter()
            .find(|id| *id == settings.selected_destination)
            .ok_or_else(|| CoreError::UnknownUploader(settings.selected_destination.clone()))?;
        let shortener = ShortenerChoice::from_id(settings.active_shortener.as_deref())?;
        let mut definitions = settings.custom_sxcu_json.into_iter();
        let sxcu_json = definitions.next().unwrap_or_default();
        let mut custom_uploaders = CustomUploaderStore::new();
        if !sxcu_json.trim().is_empty() {
            let _ = custom_uploaders.load(CUSTOM, &sxcu_json);
        }
        self.destination = destination;
        if let Some(folder) = settings.mirror_folder {
            self.mirror_folder = folder;
        }
        self.s3 = settings.s3.unwrap_or_else(default_s3_config);
        self.sxcu_json = sxcu_json;
        self.custom_uploaders = custom_uploaders;
        self.additional_sxcu_json = definitions.collect();
        self.shortener = shortener;
        self.auto_shorten = shortener != ShortenerChoice::None;
        Ok(())
    }

    fn sync_queue_rows(&mut self) {
        self.queue_rows = self
            .queue
            .items()
            .iter()
            .map(|item| {
                (
                    item.request.file_name.clone(),
                    item.status.clone(),
                    item.attempts,
                )
            })
            .collect();
    }

    pub(crate) fn is_uploading(&self) -> bool {
        self.pending.is_some() || self.pending_shortening.is_some()
    }

    pub(crate) fn poll_upload(&mut self, clipboard: &dyn ClipboardSink) {
        self.poll_shortening();
        let Some(receiver) = &self.pending else {
            return;
        };
        let completion = match receiver.try_recv() {
            Ok(completion) => completion,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                if let Some((_, status, _)) = self.queue_rows.last_mut() {
                    *status = QueuedUploadStatus::Failed {
                        attempts: 0,
                        last_error: String::new(),
                    };
                }
                self.status = Some(Err(Notice::new(|s| s.upload_interrupted)));
                return;
            }
        };
        self.pending = None;
        self.queue = completion.queue;
        self.sync_queue_rows();
        self.status = Some(
            completion
                .result
                .map_err(|error| Notice::with_detail(|s| s.upload_failed, error))
                .and_then(|result| {
                    if let Some(error) = result.shortener_error {
                        self.shortener_status =
                            Some(Notice::with_detail(|s| s.shortening_failed, error));
                    }
                    let copied = clipboard.copy_text(&result.url);
                    self.url = Some(result.url);
                    copied
                        .map(|()| Notice::new(|s| s.upload_finished))
                        .map_err(|error| Notice::with_detail(|s| s.upload_copy_failed, error))
                }),
        );
    }

    fn poll_shortening(&mut self) {
        let Some(receiver) = &self.pending_shortening else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(url)) => self.shortened_url = Some(url),
            Ok(Err(error)) => {
                self.shortener_status = Some(Notice::with_detail(|s| s.shortening_failed, error))
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.shortener_status = Some(Notice::new(|s| s.shortener_interrupted))
            }
        }
        self.pending_shortening = None;
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        s: &Strings,
        capture: Option<(&Path, bool)>,
    ) {
        view_heading(ui, s.uploaders, p);
        ui.add_space(6.0_f32);
        ui.separator();
        ui.add_space(6.0_f32);
        egui::ScrollArea::vertical()
            .id_salt("uploaders_panel")
            .show(ui, |ui| {
                ui.add_enabled_ui(!self.is_uploading(), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(s.destination);
                        egui::ComboBox::from_id_salt("upload_destination")
                            .selected_text(destination_text(self.destination, s).0)
                            .show_ui(ui, |ui| {
                                for id in DESTINATIONS {
                                    let (name, description) = destination_text(id, s);
                                    ui.selectable_value(&mut self.destination, id, name)
                                        .on_hover_text(description);
                                }
                            });
                    });
                    ui.label(
                        RichText::new(destination_text(self.destination, s).1).color(p.text_muted),
                    );
                    ui.add_space(8.0_f32);
                    self.show_destination_settings(ui, s);
                    ui.separator();
                    self.show_upload_controls(ui, p, s, capture);
                });
                if self.pending.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(s.uploading_capture);
                    });
                }
                if let Some(status) = &self.status {
                    let (notice, color) = match status {
                        Ok(notice) => (notice, p.success),
                        Err(notice) => (notice, p.danger),
                    };
                    ui.label(RichText::new(notice.text(s)).color(color));
                }
                if let Some(url) = &self.url {
                    ui.add(egui::Label::new(RichText::new(url).monospace()).selectable(true));
                }

                ui.separator();
                self.show_shortener(ui, p, s);
                ui.separator();
                self.show_settings_buttons(ui, p, s);
                ui.separator();
                self.show_queue(ui, p, s);
            });
    }

    fn show_upload_controls(
        &mut self,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        s: &Strings,
        capture: Option<(&Path, bool)>,
    ) {
        let start = if self.is_text_destination() {
            ui.label(s.uploader_filename);
            ui.text_edit_singleline(&mut self.text_file_name);
            ui.label(s.text_snippet);
            ui.add(
                egui::TextEdit::multiline(&mut self.text_payload)
                    .code_editor()
                    .hint_text(s.text_snippet_hint)
                    .desired_rows(6)
                    .desired_width(f32::INFINITY),
            );
            ui.add_enabled(
                !self.text_payload.trim().is_empty(),
                egui::Button::new(s.upload_text_snippet),
            )
            .clicked()
            .then(|| self.start_text_upload())
        } else if let Some((path, selected)) = capture {
            ui.label(path.display().to_string());
            let button = if selected {
                s.upload_selected
            } else {
                s.upload_latest
            };
            ui.button(button).clicked().then(|| self.start_upload(path))
        } else {
            ui.label(RichText::new(s.no_screenshots_yet).color(p.text_muted));
            None
        };
        if let Some(Err(error)) = start {
            self.url = None;
            self.status = Some(Err(Notice::with_detail(|s| s.upload_failed, error)));
        }
    }

    fn show_destination_settings(&mut self, ui: &mut egui::Ui, s: &Strings) {
        match self.destination {
            LOCAL_MIRROR => text_input(ui, s.mirror_folder, &mut self.mirror_folder, false),
            "imgur" => text_input(ui, s.client_id, &mut self.imgur_client_id, true),
            "discord-webhook" => text_input(ui, s.webhook_url, &mut self.discord_webhook_url, true),
            "github-gist" => {
                text_input(ui, s.github_token, &mut self.gist_token, true);
                ui.checkbox(&mut self.gist_public, s.public_gist);
            }
            "pastebin" => text_input(ui, s.developer_key, &mut self.pastebin_key, true),
            S3 => {
                let cfg = &mut self.s3;
                text_input(ui, s.s3_endpoint, &mut cfg.endpoint, false);
                text_input(ui, s.s3_region, &mut cfg.region, false);
                text_input(ui, s.s3_bucket, &mut cfg.bucket, false);
                text_input(ui, s.s3_access_key_id, &mut cfg.access_key_id, false);
                text_input(ui, s.s3_secret_access_key, &mut cfg.secret_access_key, true);
                text_input(ui, s.s3_key_prefix, &mut cfg.key_prefix, false);
                let domain = cfg.custom_domain.get_or_insert_with(String::new);
                text_input(ui, s.s3_custom_domain, domain, false);
                if domain.trim().is_empty() {
                    cfg.custom_domain = None;
                }
                ui.checkbox(&mut cfg.use_path_style, s.s3_use_path_style);
                ui.checkbox(&mut cfg.public_read_acl, s.s3_public_read_acl);
            }
            CUSTOM => {
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut self.sxcu_json)
                            .code_editor()
                            .hint_text(s.paste_sxcu)
                            .desired_rows(8)
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.custom_uploaders.remove(CUSTOM);
                    self.status = None;
                    self.url = None;
                }
                if ui.button(s.validate_sxcu).clicked() {
                    self.url = None;
                    self.status = Some(
                        self.load_custom_uploader()
                            .map(|()| Notice::new(|s| s.custom_uploader_loaded))
                            .map_err(|error| {
                                Notice::with_detail(|s| s.uploader_settings_error, error)
                            }),
                    );
                }
            }
            _ => {}
        }
    }

    fn show_shortener(&mut self, ui: &mut egui::Ui, p: &VoltShotPalette, s: &Strings) {
        ui.heading(s.url_shortener);
        ui.add_enabled_ui(!self.is_uploading(), |ui| {
            egui::ComboBox::from_id_salt("url_shortener_choice")
                .selected_text(self.shortener.label(s))
                .show_ui(ui, |ui| {
                    for choice in ShortenerChoice::ALL {
                        ui.selectable_value(&mut self.shortener, choice, choice.label(s));
                    }
                });
            let available = self.shortener != ShortenerChoice::None;
            if !available {
                self.auto_shorten = false;
            }
            ui.add_enabled(
                available,
                egui::Checkbox::new(&mut self.auto_shorten, s.shorten_uploads),
            );
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.shortener_input)
                        .hint_text(s.shortener_input_hint)
                        .desired_width(320.0),
                );
                if ui
                    .add_enabled(available, egui::Button::new(s.shorten_url))
                    .clicked()
                    && let Err(notice) = self.start_shortening()
                {
                    self.shortened_url = None;
                    self.shortener_status = Some(notice);
                }
            });
        });
        if self.pending_shortening.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(s.shortening_url);
            });
        }
        if let Some(notice) = &self.shortener_status {
            ui.label(RichText::new(notice.text(s)).color(p.danger));
        }
        if let Some(url) = &self.shortened_url {
            ui.horizontal_wrapped(|ui| {
                ui.label(s.shortened_url);
                ui.add(egui::Label::new(RichText::new(url).monospace()).selectable(true));
                if ui.small_button(s.copy).clicked() {
                    ui.ctx().copy_text(url.clone());
                }
            });
        }
    }

    fn show_settings_buttons(&mut self, ui: &mut egui::Ui, p: &VoltShotPalette, s: &Strings) {
        ui.label(s.uploader_settings_secret_notice);
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(!self.is_uploading(), |ui| {
                let action = if ui.button(s.save_uploader_settings).clicked() {
                    Some(
                        self.save_settings()
                            .map(|()| Notice::new(|s| s.uploader_settings_saved)),
                    )
                } else if ui.button(s.load_uploader_settings).clicked() {
                    Some(
                        self.load_settings()
                            .map(|()| Notice::new(|s| s.uploader_settings_loaded)),
                    )
                } else {
                    None
                };
                if let Some(action) = action {
                    self.settings_status = Some(action.map_err(|error| {
                        Notice::with_detail(|s| s.uploader_settings_error, error)
                    }));
                }
            });
        });
        if let Some(status) = &self.settings_status {
            let (notice, color) = match status {
                Ok(notice) => (notice, p.success),
                Err(notice) => (notice, p.danger),
            };
            ui.label(RichText::new(notice.text(s)).color(color));
        }
    }

    fn show_queue(&mut self, ui: &mut egui::Ui, p: &VoltShotPalette, s: &Strings) {
        ui.horizontal_wrapped(|ui| {
            ui.heading(s.upload_queue);
            let can_clear = self.pending.is_none() && !self.queue_rows.is_empty();
            if ui
                .add_enabled(can_clear, egui::Button::new(s.clear_completed))
                .clicked()
            {
                self.queue.clear_finished();
                self.sync_queue_rows();
            }
        });
        if self.queue_rows.is_empty() {
            ui.label(s.queue_empty);
        }
        for (name, status, attempts) in &self.queue_rows {
            let (label, color) = match status {
                QueuedUploadStatus::Pending => (s.queue_pending, p.text_muted),
                QueuedUploadStatus::InProgress => (s.queue_uploading, p.text_primary),
                QueuedUploadStatus::Succeeded { .. } => (s.queue_succeeded, p.success),
                QueuedUploadStatus::Failed { .. } => (s.queue_failed, p.danger),
            };
            ui.horizontal_wrapped(|ui| {
                ui.strong(name);
                ui.label(RichText::new(label).color(color));
                if *attempts > 0 {
                    ui.label(format!("{}: {attempts}", s.queue_attempts));
                }
            });
            if let QueuedUploadStatus::Failed { last_error, .. } = status
                && !last_error.is_empty()
            {
                ui.label(RichText::new(last_error).color(p.danger));
            }
        }
    }
}

fn default_s3_config() -> S3Config {
    S3Config {
        endpoint: "https://s3.amazonaws.com".to_owned(),
        region: "us-east-1".to_owned(),
        bucket: String::new(),
        access_key_id: String::new(),
        secret_access_key: String::new(),
        key_prefix: String::new(),
        custom_domain: None,
        use_path_style: false,
        public_read_acl: false,
    }
}

fn destination_text(id: &str, s: &Strings) -> (&'static str, &'static str) {
    match id {
        "imgur" => ("Imgur", s.imgur_description),
        "discord-webhook" => ("Discord Webhook", s.discord_description),
        "github-gist" => ("GitHub Gist", s.gist_description),
        "pastebin" => ("Pastebin", s.pastebin_description),
        S3 => (s.s3_destination, s.s3_description),
        CUSTOM => (s.custom_uploader, s.custom_uploader_description),
        _ => (s.local_mirror, s.local_mirror_description),
    }
}

fn text_input(ui: &mut egui::Ui, label: &str, value: &mut String, masked: bool) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).password(masked));
}

fn process_upload(
    queue: &mut UploadQueue,
    uploader: SelectedUploader,
    request: UploadRequest,
    shortener: ShortenerChoice,
) -> Result<CompletedUpload, String> {
    let mut registry = UploaderRegistry::new();
    let id = uploader.id;
    registry.register(Box::new(uploader));
    queue.enqueue(id, request);
    let policy = RetryPolicy {
        max_attempts: 3,
        base_delay_ms: 250,
        max_delay_ms: 1000,
    };
    let item = queue
        .process_next(&registry, &policy)
        .ok_or("no pending upload")?;
    match &item.status {
        QueuedUploadStatus::Succeeded { url } => {
            let mut completed = CompletedUpload {
                url: url.clone(),
                shortener_error: None,
            };
            if shortener != ShortenerChoice::None && web_url_input(url).is_some() {
                match shortener.shorten(url) {
                    Ok(url) => completed.url = url,
                    Err(error) => completed.shortener_error = Some(error),
                }
            }
            Ok(completed)
        }
        QueuedUploadStatus::Failed { last_error, .. } => Err(last_error.clone()),
        _ => Err("upload did not finish".into()),
    }
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
        let completion = panel
            .pending
            .take()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let result = completion.result.unwrap();

        assert!(result.url.starts_with("file:///"));
        assert!(result.url.ends_with("/mirror/capture.png"));
        assert_eq!(completion.queue.items()[0].attempts, 1);
        assert!(matches!(
            completion.queue.items()[0].status,
            QueuedUploadStatus::Succeeded { .. }
        ));
        assert_eq!(
            fs::read(mirror.join("capture.png")).unwrap(),
            b"capture contents"
        );
    }

    #[test]
    fn text_destinations_require_non_empty_utf8_snippet() {
        let mut panel = UploadersPanelState::new(Path::new("mirror"));
        panel.destination = "github-gist";
        panel.gist_token = "ghp_test".into();
        assert!(panel.is_text_destination());
        assert!(panel.start_text_upload().is_err());

        panel.destination = "pastebin";
        panel.pastebin_key = "dev_key".into();
        assert!(panel.is_text_destination());
        assert!(panel.start_text_upload().is_err());

        panel.destination = LOCAL_MIRROR;
        assert!(!panel.is_text_destination());
    }

    fn s3_fixture() -> S3Config {
        S3Config {
            endpoint: "https://storage.example.test".into(),
            region: "auto".into(),
            bucket: "captures".into(),
            access_key_id: "test-id".into(),
            secret_access_key: "test-secret".into(),
            key_prefix: "screenshots".into(),
            custom_domain: Some("https://cdn.example.test".into()),
            use_path_style: true,
            public_read_acl: true,
        }
    }

    #[test]
    fn validates_s3_target_before_uploading() {
        let mut panel = UploadersPanelState::new(Path::new("mirror"));
        panel.destination = S3;
        assert!(panel.upload_target().is_err());
        panel.s3 = s3_fixture();
        assert!(
            matches!(panel.upload_target(), Ok(UploadTarget::Uploader(uploader)) if uploader.id() == S3)
        );
        panel.s3.endpoint = "file:///tmp/storage".into();
        assert!(panel.upload_target().is_err());
    }

    #[test]
    fn shortener_selection_and_validation_require_web_urls() {
        let s = crate::i18n::UiLanguage::En.strings();
        assert_eq!(ShortenerChoice::from_id(None).unwrap().label(s), s.none);
        assert_eq!(
            ShortenerChoice::from_id(Some(IsGdShortener::new().id())).unwrap(),
            ShortenerChoice::IsGd
        );
        assert_eq!(
            ShortenerChoice::from_id(Some(TinyUrlShortener::new().id())).unwrap(),
            ShortenerChoice::TinyUrl
        );
        assert!(ShortenerChoice::from_id(Some("invalid")).is_err());
        for invalid in [
            "",
            "example.test",
            "file:///capture.png",
            "https:///missing-host",
            "https://host/path with spaces",
        ] {
            assert!(web_url_input(invalid).is_none(), "accepted {invalid:?}");
        }
        let mut panel = UploadersPanelState::new(Path::new("mirror"));
        panel.shortener = ShortenerChoice::IsGd;
        panel.shortener_input = "file:///capture.png".into();
        assert!(panel.start_shortening().is_err());
        assert!(!panel.is_uploading());
        assert_eq!(
            web_url_input(" https://example.test/image.png "),
            Some("https://example.test/image.png")
        );
    }

    #[test]
    fn settings_round_trip_through_panel_state() {
        let folder = tempdir().unwrap();
        let path = folder.path().join("nested/uploaders.json");
        let custom = r#"{"Name":"Example","RequestURL":"https://example.test/upload","Body":"MultipartFormData","FileFormName":"image"}"#;
        let mut panel = UploadersPanelState::new(Path::new("my-mirror"));
        panel.configure_settings_path(path.clone());
        panel.destination = S3;
        panel.s3 = s3_fixture();
        panel.sxcu_json = custom.into();
        panel.shortener = ShortenerChoice::TinyUrl;
        panel.auto_shorten = true;
        panel.save_settings().unwrap();

        let file = UploaderSettingsFile::load_or_default(&path).unwrap();
        assert_eq!(file.s3.as_ref(), Some(&s3_fixture()));
        assert_eq!(file.active_shortener.as_deref(), Some("tinyurl"));
        let mut restored = UploadersPanelState::new(Path::new("default"));
        restored.configure_settings_path(path);
        restored.load_settings().unwrap();
        assert_eq!(restored.destination, S3);
        assert_eq!(restored.mirror_folder, "my-mirror");
        assert_eq!(restored.s3, s3_fixture());
        assert_eq!(restored.sxcu_json, custom);
        assert!(restored.custom_uploaders.get(CUSTOM).is_some());
        assert_eq!(restored.shortener, ShortenerChoice::TinyUrl);
        assert!(restored.auto_shorten);
    }

    #[test]
    fn settings_restore_unfinished_sxcu_without_enabling_it() {
        let folder = tempdir().unwrap();
        let path = folder.path().join("uploaders.json");
        let mut panel = UploadersPanelState::new(Path::new("mirror"));
        panel.configure_settings_path(path.clone());
        panel.destination = CUSTOM;
        panel.sxcu_json = "{".into();
        panel.save_settings().unwrap();

        let mut restored = UploadersPanelState::new(Path::new("mirror"));
        restored.configure_settings_path(path);
        restored.load_settings().unwrap();
        assert_eq!(restored.destination, CUSTOM);
        assert_eq!(restored.sxcu_json, "{");
        assert!(restored.upload_target().is_err());
    }
}
