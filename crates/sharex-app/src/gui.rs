use crate::autostart::AutostartManager;
use crate::clipboard::SystemClipboard;
use crate::editor_ui::{EditorAction, EditorViewState};
use crate::effects_ui::EffectsPanelState;
use crate::hotkeys::{HotkeyRuntime, HotkeyStatus, TriggeredHotkey};
use crate::i18n::{self, UiLanguage};
use crate::notifications::{ToastAction, ToastManager};
use crate::pin_window::PinManager;
use crate::recording_ui::RecordingPanelState;
use crate::region_overlay::{OverlayMode, OverlayModeOutcome, RegionOverlayState};
use crate::theme::{SharexPalette, apply_sharex_theme, palette};
use crate::thumb_cache::LruCache;
use crate::tools_ui::{ToolsContext, ToolsPanelState};
use crate::tray::{TrayAction, TrayRuntime};
use crate::uploaders_ui::UploadersPanelState;
use crate::watcher_runtime::WatchFolderRuntime;
use crate::workflows_ui::{
    WorkflowsPanelState, WorkflowsUiAction, resolve_config_for_hotkey, workflow_for_hotkey,
};
use chrono::Utc;
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use sharex_capture::{DesktopSnapshot, XcapCapturer};
use sharex_core::{
    AppConfig, CaptureId, CaptureSource, CapturedImage, ClipboardSink, FileNameContext,
    FileNamePattern, HistoryEntry, HistoryQuery, HistoryRepository, HotkeyAction, HotkeyCombo,
    ImageFormat, JsonlHistoryStore, KeyCode, KeyModifiers, MonitorInfo, ScreenCapturer,
    SqliteHistoryStore, UploadRequest, Uploader, WatchedFileEvent, default_sharex_hotkeys,
    resolve_effective_config, resolve_unique_path,
};
use sharex_media::RecordingFormat;
use sharex_tools::{FallbackOcrChain, OcrEngine, OcrOptions};
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

const THUMB_CACHE_CAPACITY: usize = 64;
const HISTORY_LIMIT: usize = 100;
const AUTOSTART_APP_NAME: &str = "ShareX-RS";

/// Starts the ShareX-style native desktop application window with global hotkeys.
pub fn launch_gui(config_path: PathBuf) -> Result<(), eframe::Error> {
    launch_gui_with_fallback(config_path, AppConfig::default_for_os(), false)
}

/// Starts the ShareX-style native desktop window with an explicit fallback config (e.g. portable mode).
pub fn launch_gui_with_fallback(
    config_path: PathBuf,
    fallback_config: AppConfig,
    minimized: bool,
) -> Result<(), eframe::Error> {
    let (config, config_warning) =
        match crate::load_config_with_fallback(&config_path, fallback_config.clone()) {
            Ok(config) => (config, None),
            Err(err) => (
                fallback_config,
                Some(format!("Loaded fallback config after error: {err}")),
            ),
        };
    let start_minimized = minimized || config.start_minimized;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ShareX (Rust)")
            .with_inner_size([1160.0, 720.0])
            .with_min_inner_size([860.0, 540.0])
            .with_visible(!start_minimized),
        ..Default::default()
    };

    eframe::run_native(
        "ShareX (Rust)",
        options,
        Box::new(move |cc| {
            Ok(Box::new(SharexGuiApp::new(
                cc,
                config_path,
                config,
                config_warning,
                start_minimized,
            )))
        }),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveView {
    Captures,
    Editor,
    Effects,
    Tools,
    Recording,
    Uploaders,
    Workflows,
    Hotkeys,
    Settings,
}

struct CaptureSettings {
    config: AppConfig,
    effect_preset: Option<String>,
}

#[derive(Clone)]
struct WatchedCapture {
    event: WatchedFileEvent,
    move_after: Option<PathBuf>,
}

#[derive(Clone)]
struct CaptureCompletion {
    entry: HistoryEntry,
    status: String,
    record_history: bool,
    copy_path: bool,
    watched: Option<WatchedCapture>,
}

struct CaptureUpload {
    completion: CaptureCompletion,
    result: Result<String, String>,
}

type OcrOutcome = Result<(String, &'static str), String>;

struct SharexGuiApp {
    config_path: PathBuf,
    config: AppConfig,
    output_dir_input: String,
    monitors: Vec<MonitorInfo>,
    selected_monitor_idx: usize,
    active_view: ActiveView,
    language: UiLanguage,
    search_filter: String,
    sqlite_history: Option<SqliteHistoryStore>,
    jsonl_history: JsonlHistoryStore,
    history_entries: Vec<HistoryEntry>,
    search_results: Option<Vec<HistoryEntry>>,
    older_capture_path: Option<PathBuf>,
    selected_capture: Option<HistoryEntry>,
    tag_input: String,
    thumbnails: LruCache<PathBuf, egui::TextureHandle>,
    failed_thumbs: HashMap<PathBuf, bool>,
    editor: Option<EditorViewState>,
    effects: EffectsPanelState,
    tools: ToolsPanelState,
    recording: RecordingPanelState,
    uploaders: UploadersPanelState,
    toasts: ToastManager,
    workflows: WorkflowsPanelState,
    watcher: WatchFolderRuntime,
    capture_upload_tx: Sender<CaptureUpload>,
    capture_upload_rx: Receiver<CaptureUpload>,
    overlay: Option<RegionOverlayState>,
    overlay_capture_settings: Option<CaptureSettings>,
    ocr_pending: Option<Receiver<OcrOutcome>>,
    pins: PinManager,
    tray: TrayRuntime,
    force_quit: bool,
    run_at_startup: bool,
    hotkeys: HotkeyRuntime,
    last_hotkey_retry: Instant,
    capturer: XcapCapturer,
    clipboard: SystemClipboard,
    status_line: String,
    theme_applied_dark: bool,
}

impl SharexGuiApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        config_path: PathBuf,
        mut config: AppConfig,
        config_warning: Option<String>,
        start_minimized: bool,
    ) -> Self {
        let language = config.ui_language.parse::<UiLanguage>().unwrap_or_default();
        let mut initial_status =
            config_warning.unwrap_or_else(|| i18n::strings(language).ready_status.to_owned());

        if !config.hotkeys.is_empty() {
            for binding in default_sharex_hotkeys().into_iter().filter(|binding| {
                matches!(
                    binding.action,
                    HotkeyAction::RecordVideo
                        | HotkeyAction::RecordGif
                        | HotkeyAction::CaptureOcr
                        | HotkeyAction::PickScreenColor
                )
            }) {
                if !config
                    .hotkeys
                    .iter()
                    .any(|saved| saved.action == binding.action || saved.id == binding.id)
                {
                    config.hotkeys.push(binding);
                }
            }
        }

        apply_sharex_theme(&cc.egui_ctx, config.dark_theme);
        let hotkeys = HotkeyRuntime::new(cc.egui_ctx.clone(), &config.hotkeys);
        let tray = TrayRuntime::new(cc.egui_ctx.clone());
        if start_minimized {
            if tray.is_available() {
                cc.egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Visible(false));
            } else {
                cc.egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Visible(true));
                cc.egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
        let capturer = XcapCapturer::new();
        let monitors = capturer.list_monitors().unwrap_or_default();
        let sqlite_path = config.history_path.with_extension("sqlite");
        let sqlite_history = match SqliteHistoryStore::open(&sqlite_path) {
            Ok(store) => {
                if sqlite_path != config.history_path
                    && let Err(err) = store.migrate_from_jsonl_once(&config.history_path)
                {
                    initial_status.push_str(&format!(" History import failed: {err}"));
                }
                Some(store)
            }
            Err(err) => {
                initial_status
                    .push_str(&format!(" Using JSONL history. SQLite open failed: {err}"));
                None
            }
        };
        let jsonl_history = JsonlHistoryStore::new(&config.history_path);
        let mut uploaders = UploadersPanelState::new(&crate::default_mirror_dir(&config));
        uploaders.configure_settings_path(config_path.with_file_name("uploaders.json"));
        let workflows_path = WorkflowsPanelState::default_path_for_config(&config_path);
        let workflows =
            WorkflowsPanelState::load_or_default(&workflows_path).unwrap_or_else(|err| {
                initial_status.push_str(&format!(" Failed to load workflows: {err}"));
                WorkflowsPanelState::new()
            });
        let watcher =
            WatchFolderRuntime::new(cc.egui_ctx.clone(), workflows.watch_rules().to_vec());
        let (capture_upload_tx, capture_upload_rx) = mpsc::channel();
        let output_dir_input = config.output_dir.display().to_string();
        let cache_cap = NonZeroUsize::new(THUMB_CACHE_CAPACITY).expect("non-zero cache capacity");

        let mut app = Self {
            config_path,
            theme_applied_dark: config.dark_theme,
            config,
            output_dir_input,
            monitors,
            selected_monitor_idx: 0,
            active_view: ActiveView::Captures,
            language,
            search_filter: String::new(),
            sqlite_history,
            jsonl_history,
            history_entries: Vec::new(),
            search_results: None,
            older_capture_path: None,
            selected_capture: None,
            tag_input: String::new(),
            thumbnails: LruCache::new(cache_cap),
            failed_thumbs: HashMap::new(),
            editor: None,
            effects: EffectsPanelState::new(),
            tools: ToolsPanelState::new(),
            recording: RecordingPanelState::new(),
            uploaders,
            toasts: ToastManager::new(),
            workflows,
            watcher,
            capture_upload_tx,
            capture_upload_rx,
            overlay: None,
            overlay_capture_settings: None,
            ocr_pending: None,
            pins: PinManager::new(),
            tray,
            force_quit: false,
            run_at_startup: AutostartManager::is_enabled(AUTOSTART_APP_NAME),
            hotkeys,
            last_hotkey_retry: Instant::now(),
            capturer,
            clipboard: SystemClipboard::new(),
            status_line: initial_status,
        };
        app.refresh_history();
        app
    }

    fn persist_config_silent(&mut self) {
        if let Err(err) = self.config.save(&self.config_path) {
            self.status_line = format!("Failed to save config: {err}");
        }
    }

    fn set_language(&mut self, language: UiLanguage) {
        if self.language != language {
            self.language = language;
            self.config.ui_language = language.code().to_owned();
            self.persist_config_silent();
        }
    }

    fn latest_capture_path(&self) -> Option<&Path> {
        first_existing_capture_path(&self.history_entries).or_else(|| {
            self.older_capture_path
                .as_deref()
                .filter(|path| path.is_file())
        })
    }

    fn select_capture(&mut self, entry: &HistoryEntry) {
        if !self.is_capture_selected(entry) {
            self.tag_input.clear();
        }
        self.selected_capture = Some(entry.clone());
    }

    fn is_capture_selected(&self, entry: &HistoryEntry) -> bool {
        self.selected_capture
            .as_ref()
            .is_some_and(|selected| selected.id == entry.id)
    }

    fn hotkey_hint(&self, action: HotkeyAction) -> String {
        self.config
            .hotkeys
            .iter()
            .find(|b| b.action == action && b.enabled)
            .and_then(|b| b.combo)
            .map(|c| c.to_string())
            .unwrap_or_else(|| i18n::strings(self.language).status_unbound.to_owned())
    }

    fn handle_tray_action(&mut self, ctx: &egui::Context, action: TrayAction) {
        match action {
            TrayAction::CaptureFullscreen => {
                self.capture_source(ctx, CaptureSource::PrimaryMonitor);
            }
            TrayAction::CaptureRegion => {
                self.start_region_overlay(ctx);
            }
            TrayAction::CaptureActiveWindow => {
                self.capture_source(ctx, CaptureSource::ActiveWindow);
            }
            TrayAction::OpenMainWindow => {
                show_main_window(ctx);
            }
            TrayAction::OpenScreenshotsFolder => {
                let _ = fs::create_dir_all(&self.config.output_dir);
                open_folder_in_os(&self.config.output_dir);
            }
            TrayAction::Quit => {
                self.force_quit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn trigger_hotkey_event(&mut self, ctx: &egui::Context, event: TriggeredHotkey) {
        match event.action {
            HotkeyAction::CaptureFullscreen => {
                let settings = self.capture_settings_for_hotkey(&event.id);
                self.capture_source_with_settings(ctx, CaptureSource::PrimaryMonitor, settings);
            }
            HotkeyAction::CaptureRegion => {
                let settings = self.capture_settings_for_hotkey(&event.id);
                self.start_overlay(
                    ctx,
                    OverlayMode::Capture,
                    event.pre_snapshot,
                    Some(settings),
                );
            }
            HotkeyAction::CaptureActiveWindow => {
                let settings = self.capture_settings_for_hotkey(&event.id);
                self.capture_source_with_settings(ctx, CaptureSource::ActiveWindow, settings);
            }
            HotkeyAction::CaptureLastRegion => {
                let settings = self.capture_settings_for_hotkey(&event.id);
                self.capture_last_region(ctx, settings);
            }
            HotkeyAction::OpenEditor => {
                if self.editor.is_some() {
                    self.active_view = ActiveView::Editor;
                    show_main_window(ctx);
                } else if let Some(first_path) = self
                    .history_entries
                    .iter()
                    .find_map(|e| e.file_path.clone())
                {
                    self.open_file_in_editor(ctx, &first_path);
                    show_main_window(ctx);
                } else {
                    self.status_line =
                        "No screenshot available to open in the image editor.".into();
                }
            }
            HotkeyAction::RecordVideo | HotkeyAction::RecordGif => {
                let settings = self.capture_settings_for_hotkey(&event.id);
                let format = if event.action == HotkeyAction::RecordVideo {
                    RecordingFormat::Mp4H264
                } else {
                    RecordingFormat::Gif
                };
                self.status_line = self
                    .recording
                    .toggle_hotkey_recording(format, &settings.config.output_dir);
                self.active_view = ActiveView::Recording;
            }
            HotkeyAction::CaptureOcr => {
                self.start_overlay(ctx, OverlayMode::Ocr, event.pre_snapshot, None);
            }
            HotkeyAction::PickScreenColor => {
                self.start_overlay(ctx, OverlayMode::ColorPicker, event.pre_snapshot, None);
            }
        }
    }

    fn capture_settings(&self) -> CaptureSettings {
        CaptureSettings {
            config: self.config.clone(),
            effect_preset: None,
        }
    }

    fn capture_settings_for_hotkey(&self, hotkey_id: &str) -> CaptureSettings {
        let registry = self.workflows.registry();
        CaptureSettings {
            config: resolve_config_for_hotkey(&self.config, registry, hotkey_id),
            effect_preset: workflow_for_hotkey(&self.config, registry, hotkey_id)
                .and_then(|preset| preset.overrides.effect_preset_name.clone()),
        }
    }

    fn start_overlay(
        &mut self,
        ctx: &egui::Context,
        mode: OverlayMode,
        pre_snapshot: Option<DesktopSnapshot>,
        capture_settings: Option<CaptureSettings>,
    ) {
        if self.overlay.is_some() || (mode == OverlayMode::Ocr && self.ocr_pending.is_some()) {
            return;
        }
        let snapshot = pre_snapshot.map_or_else(|| capture_overlay_snapshot(&self.capturer), Ok);
        match snapshot {
            Ok(snapshot) => {
                self.overlay = Some(RegionOverlayState::open_with_mode(ctx, snapshot, mode));
                self.overlay_capture_settings = capture_settings;
                self.status_line = match mode {
                    OverlayMode::Capture => "Select a region to capture. Esc to cancel.",
                    OverlayMode::Ocr => "Select a region containing text. Esc to cancel.",
                    OverlayMode::SelectRecordingRegion => {
                        "Select a region to record. Esc to cancel."
                    }
                    OverlayMode::ColorPicker => "Click a pixel to copy its color. Esc to cancel.",
                }
                .into();
                ctx.request_repaint();
            }
            Err(err) => self.status_line = format!("Could not capture the desktop: {err}"),
        }
    }

    fn start_region_overlay(&mut self, ctx: &egui::Context) {
        self.start_overlay_with_mode(ctx, OverlayMode::Capture);
    }

    fn start_overlay_with_mode(&mut self, ctx: &egui::Context, mode: OverlayMode) {
        self.start_overlay(ctx, mode, None, None);
    }

    fn capture_last_region(&mut self, ctx: &egui::Context, settings: CaptureSettings) {
        if let Some(region) = self.config.last_region {
            self.capture_source_with_settings(ctx, CaptureSource::Region { region }, settings);
        } else {
            self.start_overlay(ctx, OverlayMode::Capture, None, Some(settings));
        }
    }

    fn capture_source(&mut self, ctx: &egui::Context, source: CaptureSource) {
        self.capture_source_with_settings(ctx, source, self.capture_settings());
    }

    fn capture_source_with_settings(
        &mut self,
        ctx: &egui::Context,
        source: CaptureSource,
        settings: CaptureSettings,
    ) {
        match self.capturer.capture(&source) {
            Ok(image) => {
                self.commit_captured_image(ctx, image, source, settings, false, None);
            }
            Err(err) => {
                self.status_line = format!("Capture failed: {err}");
            }
        }
    }

    fn commit_captured_image(
        &mut self,
        ctx: &egui::Context,
        image: CapturedImage,
        source: CaptureSource,
        settings: CaptureSettings,
        force_open_in_editor: bool,
        watched: Option<WatchedCapture>,
    ) {
        let config = settings.config;
        let preset = match settings.effect_preset.as_deref() {
            Some(name) => self.effects.preset_by_name(name),
            None if self.effects.apply_on_capture => self.effects.active_preset(),
            None => None,
        };
        let (image, effect_note) = match preset {
            Some(preset) => match preset.apply(&image) {
                Ok(processed) => (processed, format!(" [{}]", preset.name)),
                Err(err) => (image, format!(" [effect skipped: {err}]")),
            },
            None => (
                image,
                settings
                    .effect_preset
                    .map_or_else(String::new, |name| format!(" [unknown effect: {name}]")),
            ),
        };

        let now = Utc::now();
        let format = config.image_format;
        let saved_path = if config.pipeline.save_to_file {
            match write_capture_file(&config, &image, format, now) {
                Ok(path) => {
                    self.thumbnails.remove(&path);
                    self.failed_thumbs.remove(&path);
                    Some(path)
                }
                Err(err) => {
                    self.status_line = format!("Failed to save capture: {err}");
                    return;
                }
            }
        } else {
            None
        };

        let target = saved_path
            .clone()
            .or_else(|| watched.as_ref().map(|watched| watched.event.path.clone()));
        let title = if watched.is_some() {
            "Watched image processed"
        } else {
            "Screenshot captured"
        };
        let dimensions = format!(
            "{}x{} ({format}){effect_note}",
            image.width(),
            image.height()
        );
        let mut status = format!("{title}: {dimensions}");
        if let Some(path) = &target {
            status.push_str(&format!(" -> {}", path.display()));
        }
        if config.pipeline.copy_image_to_clipboard
            && let Err(err) = self.clipboard.copy_image(&image)
        {
            status.push_str(&format!(". Clipboard copy failed: {err}"));
        }
        if watched.is_none() && self.config.play_capture_sound {
            crate::sound::play_capture_sound();
        }
        if self.config.show_toasts {
            self.toasts
                .push_capture(ctx, title, dimensions, Some(&image), target.clone(), None);
        }

        let mut completion = CaptureCompletion {
            entry: HistoryEntry {
                id: CaptureId::from_timestamp(now, image.width() ^ image.height()),
                timestamp: now,
                source,
                width: image.width(),
                height: image.height(),
                format,
                file_path: target,
                upload_url: None,
                tags: Vec::new(),
                note: watched
                    .as_ref()
                    .map(|watched| format!("Watch folder: {}", watched.event.path.display())),
            },
            status,
            record_history: config.pipeline.record_history,
            copy_path: config.pipeline.copy_path_to_clipboard,
            watched,
        };

        let upload_started = config.pipeline.uploader_id.as_deref().is_some_and(|id| {
            match self.start_capture_upload(ctx, &config, &image, &completion, id) {
                Ok(()) => true,
                Err(err) => {
                    completion
                        .status
                        .push_str(&format!(". Upload failed: {err}"));
                    if let Some(watched) = &mut completion.watched {
                        watched.move_after = None;
                    }
                    false
                }
            }
        });
        if upload_started {
            self.status_line = format!("{}. Uploading…", completion.status);
        } else {
            self.finish_capture(completion);
        }

        if force_open_in_editor || config.pipeline.open_in_editor {
            self.editor = Some(EditorViewState::open(ctx, image, saved_path, format));
            self.active_view = ActiveView::Editor;
            show_main_window(ctx);
        }
    }

    fn start_capture_upload(
        &self,
        ctx: &egui::Context,
        config: &AppConfig,
        image: &CapturedImage,
        completion: &CaptureCompletion,
        id: &str,
    ) -> Result<(), String> {
        let uploader = self
            .uploaders
            .workflow_uploader(id)
            .map_err(|err| err.to_string())?;
        let format = config.image_format;
        let stem = capture_filename_stem(config, image, completion.entry.timestamp)?;
        let file_name = format!("{stem}.{}", format.extension());
        let completion = completion.clone();
        let image = image.clone();
        let sender = self.capture_upload_tx.clone();
        let ctx = ctx.clone();
        thread::Builder::new()
            .name("sharex-workflow-upload".into())
            .spawn(move || {
                let result = image.encode(format).and_then(|bytes| {
                    uploader.upload(&UploadRequest {
                        file_name,
                        mime_type: format.mime_type().into(),
                        bytes,
                    })
                });
                let _ = sender.send(CaptureUpload {
                    completion,
                    result: result
                        .map(|report| report.url)
                        .map_err(|err| err.to_string()),
                });
                ctx.request_repaint();
            })
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    fn finish_capture(&mut self, mut completion: CaptureCompletion) {
        if let Some(watched) = completion.watched {
            if let Some(folder) = watched.move_after.as_deref() {
                match move_watched_file(&watched.event.path, folder) {
                    Ok(target) => {
                        if completion.entry.file_path.as_ref() == Some(&watched.event.path) {
                            completion.entry.file_path = Some(target);
                        }
                    }
                    Err(err) => completion.status.push_str(&format!(". Move failed: {err}")),
                }
            }
            self.workflows.record_processed_event(watched.event);
        }

        if completion.copy_path
            && let Some(text) = completion.entry.upload_url.clone().or_else(|| {
                completion
                    .entry
                    .file_path
                    .as_ref()
                    .map(|path| path.display().to_string())
            })
            && let Err(err) = self.clipboard.copy_text(&text)
        {
            completion
                .status
                .push_str(&format!(". Clipboard copy failed: {err}"));
        }
        if completion.record_history {
            match self.history_repository().append(&completion.entry) {
                Ok(()) => self.select_capture(&completion.entry),
                Err(err) => completion
                    .status
                    .push_str(&format!(". History update failed: {err}")),
            }
            self.refresh_history();
        }
        self.status_line = completion.status;
    }

    fn poll_capture_uploads(&mut self, ctx: &egui::Context) {
        while let Ok(mut upload) = self.capture_upload_rx.try_recv() {
            match upload.result {
                Ok(url) => {
                    upload
                        .completion
                        .status
                        .push_str(&format!(". Uploaded to {url}"));
                    if self.config.show_toasts {
                        self.toasts.push_capture(
                            ctx,
                            "Upload complete",
                            &url,
                            None,
                            upload.completion.entry.file_path.clone(),
                            Some(url.clone()),
                        );
                    }
                    upload.completion.entry.upload_url = Some(url);
                }
                Err(err) => {
                    upload
                        .completion
                        .status
                        .push_str(&format!(". Upload failed: {err}"));
                    if let Some(watched) = &mut upload.completion.watched {
                        watched.move_after = None;
                    }
                }
            }
            self.finish_capture(upload.completion);
        }
    }

    fn poll_watcher(&mut self, ctx: &egui::Context) {
        for event in self.watcher.poll_events() {
            let Some(rule) = self
                .workflows
                .watch_rules()
                .iter()
                .find(|rule| rule.id == event.rule_id && rule.enabled)
            else {
                continue;
            };
            let Ok(image) = CapturedImage::load_from_file(&event.path) else {
                continue;
            };
            let mut settings = self.capture_settings();
            if let Some(id) = event.workflow_id.as_deref() {
                let Some(preset) = self
                    .workflows
                    .registry()
                    .find_by_id(id)
                    .filter(|p| p.enabled)
                else {
                    self.status_line = format!("Watch folder workflow '{id}' is unavailable.");
                    continue;
                };
                match resolve_effective_config(&self.config, &preset.overrides) {
                    Ok(config) => settings.config = config,
                    Err(err) => {
                        self.status_line = format!("Watch folder workflow '{id}' failed: {err}");
                        continue;
                    }
                }
                settings
                    .effect_preset
                    .clone_from(&preset.overrides.effect_preset_name);
            }
            while self.is_watched_directory(&settings.config.output_dir) {
                settings.config.output_dir.push("Processed");
            }
            let move_after = rule.move_after_process.clone();
            if let Some(folder) = &move_after
                && self.is_watched_directory(folder)
            {
                self.status_line =
                    format!("Move destination is a watched folder: {}", folder.display());
                continue;
            }
            self.commit_captured_image(
                ctx,
                image,
                CaptureSource::WatchedFile,
                settings,
                false,
                Some(WatchedCapture { event, move_after }),
            );
        }
    }

    fn is_watched_directory(&self, path: &Path) -> bool {
        let canonical = path.canonicalize().ok();
        self.workflows.watch_rules().iter().any(|rule| {
            rule.enabled
                && (rule.folder == path
                    || canonical.as_ref().is_some_and(|candidate| {
                        rule.folder
                            .canonicalize()
                            .as_ref()
                            .is_ok_and(|root| root == candidate)
                    }))
        })
    }

    fn update_overlay(&mut self, ctx: &egui::Context) {
        let Some(overlay) = self.overlay.as_mut() else {
            return;
        };
        let outcome = overlay.show_mode(ctx);
        if matches!(outcome, OverlayModeOutcome::Pending) {
            return;
        }
        let closing = self.overlay.take();
        let capture_settings = self.overlay_capture_settings.take();
        if let Some(closing) = closing.as_ref()
            && !matches!(outcome, OverlayModeOutcome::Captured { .. })
        {
            closing.restore_previous_focus();
        }

        match outcome {
            OverlayModeOutcome::Pending => {}
            OverlayModeOutcome::Cancelled => {
                self.status_line = "Region selection cancelled.".into()
            }
            OverlayModeOutcome::Captured {
                image,
                global_region,
                open_in_editor,
            } => {
                self.config.last_region = Some(global_region);
                self.persist_config_silent();
                let settings = capture_settings.unwrap_or_else(|| self.capture_settings());
                let will_open_editor = open_in_editor || settings.config.pipeline.open_in_editor;
                self.commit_captured_image(
                    ctx,
                    image,
                    CaptureSource::Region {
                        region: global_region,
                    },
                    settings,
                    open_in_editor,
                    None,
                );
                if !will_open_editor && let Some(closing) = closing {
                    closing.restore_previous_focus();
                }
            }
            OverlayModeOutcome::OcrRegion { image, .. } => self.start_ocr(ctx, image),
            OverlayModeOutcome::RecordingRegion(region) => {
                self.recording.set_region(Some(region));
                self.active_view = ActiveView::Recording;
                self.status_line = format!(
                    "Recording region selected: {} × {}",
                    region.width(),
                    region.height()
                );
            }
            OverlayModeOutcome::ColorPicked { hex, .. } => {
                self.status_line = match self.clipboard.copy_text(&hex) {
                    Ok(()) => format!("Copied {hex} to clipboard."),
                    Err(err) => format!("Picked {hex}. Clipboard copy failed: {err}"),
                };
                self.tools.set_picked_color(hex.clone());
                self.active_view = ActiveView::Tools;
                if self.config.show_toasts {
                    self.toasts
                        .push_capture(ctx, "Screen color picked", hex, None, None, None);
                }
            }
        }
    }

    fn start_ocr(&mut self, ctx: &egui::Context, image: CapturedImage) {
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        match thread::Builder::new()
            .name("sharex-region-ocr".into())
            .spawn(move || {
                let result = FallbackOcrChain::default()
                    .recognize(&image, &OcrOptions::default())
                    .map(|result| (result.text, result.engine_name))
                    .map_err(|err| err.to_string());
                let _ = sender.send(result);
                repaint.request_repaint();
            }) {
            Ok(_) => {
                self.ocr_pending = Some(receiver);
                self.status_line = i18n::strings(self.language).running_ocr.to_owned();
            }
            Err(err) => self.status_line = format!("Could not start OCR: {err}"),
        }
    }

    fn poll_ocr(&mut self, ctx: &egui::Context) {
        let Some(receiver) = self.ocr_pending.as_ref() else {
            return;
        };
        let s = i18n::strings(self.language);
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(s.ocr_interrupted.to_owned()),
        };
        self.ocr_pending = None;
        match result {
            Ok((text, engine)) => {
                self.status_line = if text.trim().is_empty() {
                    s.ocr_no_text.to_owned()
                } else {
                    match self.clipboard.copy_text(&text) {
                        Ok(()) => {
                            format!("{} ({engine}). Text copied to clipboard.", s.ocr_completed)
                        }
                        Err(err) => format!(
                            "{} ({engine}). Clipboard copy failed: {err}",
                            s.ocr_completed
                        ),
                    }
                };
                self.tools.set_ocr_result(text, engine);
                self.active_view = ActiveView::Tools;
            }
            Err(err) => self.status_line = format!("OCR failed: {err}"),
        }
        if self.config.show_toasts {
            self.toasts
                .push_capture(ctx, s.ocr_tool, &self.status_line, None, None, None);
        }
    }

    fn write_new_capture_file(
        &self,
        image: &CapturedImage,
        format: ImageFormat,
        now: chrono::DateTime<Utc>,
    ) -> Result<PathBuf, String> {
        write_capture_file(&self.config, image, format, now)
    }

    fn open_file_in_editor(&mut self, ctx: &egui::Context, path: &Path) {
        match CapturedImage::load_from_file(path) {
            Ok(img) => {
                let format = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(|e| e.parse::<ImageFormat>().ok())
                    .unwrap_or(self.config.image_format);
                self.editor = Some(EditorViewState::open(
                    ctx,
                    img,
                    Some(path.to_path_buf()),
                    format,
                ));
                self.active_view = ActiveView::Editor;
                self.status_line = format!("Opened {} in image editor.", path.display());
            }
            Err(err) => {
                self.status_line = format!("Could not open {}: {err}", path.display());
            }
        }
    }

    fn pin_file_to_screen(&mut self, ctx: &egui::Context, path: &Path) {
        match CapturedImage::load_from_file(path) {
            Ok(img) => {
                let title = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Pinned Capture".to_owned());
                let id = self.pins.pin_image(ctx, title.clone(), &img);
                self.status_line = format!("Pinned {title} (#{id}) to screen.");
            }
            Err(err) => {
                self.status_line = format!("Failed to pin {}: {err}", path.display());
            }
        }
    }

    fn refresh_monitors(&mut self) {
        match self.capturer.list_monitors() {
            Ok(list) => {
                self.monitors = list;
                if self.selected_monitor_idx >= self.monitors.len() {
                    self.selected_monitor_idx = 0;
                }
            }
            Err(err) => self.status_line = format!("Monitor refresh failed: {err}"),
        }
    }

    fn history_repository(&self) -> &dyn HistoryRepository {
        match &self.sqlite_history {
            Some(store) => store,
            None => &self.jsonl_history,
        }
    }

    fn refresh_history(&mut self) {
        match self.history_repository().list(Some(HISTORY_LIMIT)) {
            Ok(entries) => self.history_entries = entries,
            Err(err) => self.status_line = format!("History load failed: {err}"),
        }
        self.older_capture_path = None;
        if self.history_entries.len() == HISTORY_LIMIT && self.latest_capture_path().is_none() {
            match self.history_repository().list(None) {
                Ok(entries) => {
                    self.older_capture_path =
                        first_existing_capture_path(&entries).map(Path::to_path_buf);
                }
                Err(err) => self.status_line = format!("History load failed: {err}"),
            }
        }
        self.refresh_history_search();
    }

    fn refresh_history_search(&mut self) {
        let query = capture_history_query(&self.search_filter);
        self.search_results = if query.text.is_none() && query.tag.is_none() {
            None
        } else {
            Some(match self.history_repository().search(&query) {
                Ok(entries) => entries,
                Err(err) => {
                    self.status_line = format!("History search failed: {err}");
                    Vec::new()
                }
            })
        };
    }

    fn save_capture_tags(&mut self, id: &CaptureId, tags: &[String]) -> bool {
        match self.history_repository().set_tags(id, tags) {
            Ok(true) => {
                self.status_line = "Capture tags updated.".into();
                self.refresh_history();
                true
            }
            Ok(false) => {
                self.status_line = "This capture is no longer in history.".into();
                self.refresh_history();
                false
            }
            Err(err) => {
                self.status_line = format!("Could not update capture tags: {err}");
                false
            }
        }
    }

    fn ensure_thumbnail(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureId> {
        let key = path.to_path_buf();
        if let Some(tex) = self.thumbnails.get(&key) {
            return Some(tex.id());
        }
        if self.failed_thumbs.contains_key(&key) || !path.exists() {
            return None;
        }
        match CapturedImage::load_from_file(path) {
            Ok(img) => {
                let color = crate::image_texture::bounded_color_image(&img, 256);
                let tex = ctx.load_texture(
                    format!("thumb_{}", path.display()),
                    color,
                    egui::TextureOptions::LINEAR,
                );
                let id = tex.id();
                self.thumbnails.insert(key, tex);
                Some(id)
            }
            Err(_) => {
                self.failed_thumbs.insert(key, true);
                None
            }
        }
    }

    fn render_sidebar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("ShareX")
                    .size(20.0)
                    .strong()
                    .color(p.text_primary),
            );
            ui.label(RichText::new("Rust").size(12.0).color(p.accent));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .selectable_label(self.language == UiLanguage::Pl, "PL")
                    .on_hover_text(UiLanguage::Pl.display_name())
                    .clicked()
                {
                    self.set_language(UiLanguage::Pl);
                }
                if ui
                    .selectable_label(self.language == UiLanguage::En, "EN")
                    .on_hover_text(UiLanguage::En.display_name())
                    .clicked()
                {
                    self.set_language(UiLanguage::En);
                }
            });
        });
        ui.add_space(6.0);
        ui.separator();

        let s = i18n::strings(self.language);

        egui::ScrollArea::vertical()
            .id_salt("sharex_sidebar_scroll")
            .show(ui, |ui| {
                ui.label(
                    RichText::new(s.capture_section)
                        .size(11.0)
                        .strong()
                        .color(p.text_muted),
                );
                if sidebar_action_button(
                    ui,
                    s.fullscreen,
                    &self.hotkey_hint(HotkeyAction::CaptureFullscreen),
                    p,
                ) {
                    self.capture_source(ctx, CaptureSource::PrimaryMonitor);
                }
                if sidebar_action_button(
                    ui,
                    s.region,
                    &self.hotkey_hint(HotkeyAction::CaptureRegion),
                    p,
                ) {
                    self.start_region_overlay(ctx);
                }
                if sidebar_action_button(
                    ui,
                    s.active_window,
                    &self.hotkey_hint(HotkeyAction::CaptureActiveWindow),
                    p,
                ) {
                    self.capture_source(ctx, CaptureSource::ActiveWindow);
                }
                if sidebar_action_button(
                    ui,
                    s.last_region,
                    &self.hotkey_hint(HotkeyAction::CaptureLastRegion),
                    p,
                ) {
                    self.capture_last_region(ctx, self.capture_settings());
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let selected_label = self
                        .monitors
                        .get(self.selected_monitor_idx)
                        .map(|m| format!("{} ({}x{})", m.name, m.width, m.height))
                        .unwrap_or_else(|| s.no_monitor.to_owned());
                    egui::ComboBox::from_id_salt("sidebar_monitor")
                        .selected_text(selected_label)
                        .width(142.0)
                        .show_ui(ui, |ui| {
                            for (idx, m) in self.monitors.iter().enumerate() {
                                let label = format!("{} ({}x{})", m.name, m.width, m.height);
                                ui.selectable_value(&mut self.selected_monitor_idx, idx, label);
                            }
                        });
                    if ui.small_button(s.capture_monitor_go).clicked()
                        && let Some(m) = self.monitors.get(self.selected_monitor_idx)
                    {
                        self.capture_source(ctx, CaptureSource::Monitor { id: m.id });
                    }
                });

                ui.add_space(6.0);
                ui.separator();
                ui.label(
                    RichText::new(s.after_capture_section)
                        .size(11.0)
                        .strong()
                        .color(p.text_muted),
                );
                let mut changed = false;
                changed |= ui
                    .checkbox(&mut self.config.pipeline.save_to_file, s.save_image_to_file)
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.config.pipeline.copy_image_to_clipboard,
                        s.copy_image_to_clipboard,
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.config.pipeline.open_in_editor,
                        s.open_in_image_editor,
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.config.pipeline.copy_path_to_clipboard,
                        s.copy_file_path,
                    )
                    .changed();
                let effects_toggle_label =
                    format!("{}: {}", s.effects, self.effects.active_preset_name());
                ui.checkbox(&mut self.effects.apply_on_capture, effects_toggle_label);
                if changed {
                    self.persist_config_silent();
                }

                ui.add_space(6.0);
                ui.separator();
                ui.label(
                    RichText::new(s.workspace_section)
                        .size(11.0)
                        .strong()
                        .color(p.text_muted),
                );

                if sidebar_nav_button(ui, s.captures, self.active_view == ActiveView::Captures, p) {
                    self.active_view = ActiveView::Captures;
                    self.refresh_history();
                }
                let editor_label = match &self.editor {
                    Some(ed) if ed.is_dirty() => s.image_editor_dirty,
                    Some(_) => s.image_editor,
                    None => s.image_editor_empty,
                };
                if sidebar_nav_button(ui, editor_label, self.active_view == ActiveView::Editor, p) {
                    self.active_view = ActiveView::Editor;
                }
                if sidebar_nav_button(ui, s.effects, self.active_view == ActiveView::Effects, p) {
                    self.active_view = ActiveView::Effects;
                }
                if sidebar_nav_button(ui, s.tools, self.active_view == ActiveView::Tools, p) {
                    self.active_view = ActiveView::Tools;
                }
                let recording_label = if self.recording.is_recording() {
                    format!("{} (REC)", s.recording)
                } else {
                    s.recording.to_owned()
                };
                if sidebar_nav_button(
                    ui,
                    &recording_label,
                    self.active_view == ActiveView::Recording,
                    p,
                ) {
                    self.active_view = ActiveView::Recording;
                }
                if sidebar_nav_button(
                    ui,
                    s.uploaders,
                    self.active_view == ActiveView::Uploaders,
                    p,
                ) {
                    self.active_view = ActiveView::Uploaders;
                    self.refresh_history();
                }
                if sidebar_nav_button(
                    ui,
                    s.workflows,
                    self.active_view == ActiveView::Workflows,
                    p,
                ) {
                    self.active_view = ActiveView::Workflows;
                }
                if sidebar_nav_button(
                    ui,
                    s.hotkey_settings,
                    self.active_view == ActiveView::Hotkeys,
                    p,
                ) {
                    self.active_view = ActiveView::Hotkeys;
                }
                if sidebar_nav_button(
                    ui,
                    s.application_settings,
                    self.active_view == ActiveView::Settings,
                    p,
                ) {
                    self.active_view = ActiveView::Settings;
                }
            });
    }

    fn render_captures_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);

        ui.horizontal(|ui| {
            ui.heading(s.captures);
            ui.add_space(8.0);
            let region_btn =
                egui::Button::new(RichText::new(s.capture_region_cta).color(Color32::WHITE))
                    .fill(p.accent);
            if ui.add(region_btn).clicked() {
                self.start_region_overlay(ctx);
            }
            if ui.button(s.fullscreen).clicked() {
                self.capture_source(ctx, CaptureSource::PrimaryMonitor);
            }
            if ui.button(s.open_screenshots_folder).clicked() {
                let _ = fs::create_dir_all(&self.config.output_dir);
                open_folder_in_os(&self.config.output_dir);
            }
            if !self.pins.is_empty()
                && ui
                    .button(format!("{} ({})", s.pin_close_all, self.pins.count()))
                    .clicked()
            {
                self.pins.close_all();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(s.refresh).clicked() {
                    self.refresh_history();
                    self.refresh_monitors();
                }
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.search_filter)
                            .hint_text(s.filter_captures_hint)
                            .desired_width(180.0_f32),
                    )
                    .on_hover_text("Use #tag or #tag followed by filter text.")
                    .changed()
                {
                    self.refresh_history_search();
                }
            });
        });
        ui.separator();

        let visible = self
            .search_results
            .as_ref()
            .unwrap_or(&self.history_entries)
            .clone();

        if visible.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    if self.search_results.is_some() {
                        ui.label(
                            RichText::new("No captures match this filter.").color(p.text_muted),
                        );
                        return;
                    }
                    ui.label(
                        RichText::new(s.no_screenshots_yet)
                            .size(18.0)
                            .strong()
                            .color(p.text_primary),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "{}: {} | {}: {} | {}: {}",
                            s.fullscreen,
                            self.hotkey_hint(HotkeyAction::CaptureFullscreen),
                            s.region,
                            self.hotkey_hint(HotkeyAction::CaptureRegion),
                            s.active_window,
                            self.hotkey_hint(HotkeyAction::CaptureActiveWindow),
                        ))
                        .color(p.text_muted),
                    );
                });
            });
            return;
        }

        let mut open_editor_path: Option<PathBuf> = None;
        let mut pin_file_path: Option<PathBuf> = None;
        let mut copy_img_path: Option<PathBuf> = None;
        let mut copy_text_val: Option<String> = None;
        let mut delete_file_path: Option<PathBuf> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in &visible {
                let is_selected = self.is_capture_selected(entry);
                let frame = egui::Frame::NONE
                    .fill(p.card_bg)
                    .stroke(Stroke::new(
                        if is_selected { 1.5_f32 } else { 1.0_f32 },
                        if is_selected { p.accent } else { p.border },
                    ))
                    .corner_radius(CornerRadius::same(5))
                    .inner_margin(Margin::same(10));

                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let thumb_size = Vec2::new(128.0, 76.0);
                        let thumb_tex = entry
                            .file_path
                            .as_deref()
                            .and_then(|path| self.ensure_thumbnail(ctx, path));
                        if let Some(tex_id) = thumb_tex {
                            let img_resp = ui.add(
                                egui::Image::new((tex_id, thumb_size))
                                    .corner_radius(CornerRadius::same(3))
                                    .sense(egui::Sense::click()),
                            );
                            if img_resp.clicked() {
                                self.select_capture(entry);
                            }
                            if img_resp.double_clicked()
                                && let Some(path) = &entry.file_path
                            {
                                open_editor_path = Some(path.clone());
                            }
                        } else {
                            let (rect, preview) =
                                ui.allocate_exact_size(thumb_size, egui::Sense::click());
                            if preview.clicked() {
                                self.select_capture(entry);
                            }
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(3), p.sidebar_bg);
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                s.no_preview,
                                egui::FontId::proportional(11.0),
                                p.text_muted,
                            );
                        }

                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            let title = entry
                                .file_path
                                .as_ref()
                                .and_then(|fp| fp.file_name())
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| entry.id.as_str().to_owned());
                            let title = egui::Label::new(
                                RichText::new(title)
                                    .strong()
                                    .size(14.0_f32)
                                    .color(p.text_primary),
                            )
                            .sense(egui::Sense::click());
                            if ui.add(title).on_hover_text("Select capture").clicked() {
                                self.select_capture(entry);
                            }
                            ui.label(
                                RichText::new(format!(
                                    "{}x{} {} | {}",
                                    entry.width,
                                    entry.height,
                                    entry.format.extension().to_ascii_uppercase(),
                                    entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
                                ))
                                .size(12.0)
                                .color(p.text_muted),
                            );
                            if let Some(path) = &entry.file_path {
                                ui.label(
                                    RichText::new(path.display().to_string())
                                        .size(11.0)
                                        .color(p.text_muted),
                                );
                            }
                            self.render_capture_tags(ui, p, entry);
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if let Some(path) = &entry.file_path {
                                    let exists = path.exists();
                                    if ui.add_enabled(exists, egui::Button::new(s.edit)).clicked() {
                                        open_editor_path = Some(path.clone());
                                    }
                                    if ui
                                        .add_enabled(exists, egui::Button::new("Pin"))
                                        .on_hover_text(s.pin_to_screen)
                                        .clicked()
                                    {
                                        pin_file_path = Some(path.clone());
                                    }
                                    if ui
                                        .add_enabled(exists, egui::Button::new(s.copy_image))
                                        .clicked()
                                    {
                                        copy_img_path = Some(path.clone());
                                    }
                                    if ui.button(s.copy_path).clicked() {
                                        copy_text_val = Some(path.display().to_string());
                                    }
                                    if ui
                                        .add_enabled(exists, egui::Button::new(s.delete_file))
                                        .clicked()
                                    {
                                        delete_file_path = Some(path.clone());
                                    }
                                }
                            });
                        });
                    });
                });
                ui.add_space(6.0);
            }
        });

        if let Some(path) = open_editor_path {
            self.open_file_in_editor(ctx, &path);
        }
        if let Some(path) = pin_file_path {
            self.pin_file_to_screen(ctx, &path);
        }
        if let Some(path) = copy_img_path {
            self.status_line = match CapturedImage::load_from_file(&path)
                .and_then(|img| self.clipboard.copy_image(&img))
            {
                Ok(()) => format!("Copied image {} to clipboard.", path.display()),
                Err(err) => format!("Copy image failed: {err}"),
            };
        }
        if let Some(text) = copy_text_val {
            self.status_line = match self.clipboard.copy_text(&text) {
                Ok(()) => format!("Copied path: {text}"),
                Err(err) => format!("Clipboard error: {err}"),
            };
        }
        if let Some(path) = delete_file_path {
            self.status_line = match fs::remove_file(&path) {
                Ok(()) => {
                    self.thumbnails.remove(&path);
                    self.failed_thumbs.remove(&path);
                    self.refresh_history();
                    format!("Deleted {}", path.display())
                }
                Err(err) => format!("Delete failed: {err}"),
            };
        }
    }

    fn render_capture_tags(&mut self, ui: &mut egui::Ui, p: &SharexPalette, entry: &HistoryEntry) {
        if !entry.tags.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for tag in &entry.tags {
                    egui::Frame::NONE
                        .fill(p.sidebar_bg)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(5, 1))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("#{tag}")).small().color(p.text_muted),
                                );
                                if ui
                                    .add(egui::Button::new("×").small().frame(false))
                                    .on_hover_text(format!("Remove #{tag}"))
                                    .clicked()
                                {
                                    let tags = entry
                                        .tags
                                        .iter()
                                        .filter(|existing| *existing != tag)
                                        .cloned()
                                        .collect::<Vec<_>>();
                                    self.save_capture_tags(&entry.id, &tags);
                                }
                            });
                        });
                }
            });
        }

        if self.is_capture_selected(entry) {
            ui.horizontal(|ui| {
                let input = ui.add(
                    egui::TextEdit::singleline(&mut self.tag_input)
                        .id_salt(("capture_tag", entry.id.as_str()))
                        .hint_text("Add tag")
                        .desired_width(100.0_f32),
                );
                let tag = self.tag_input.trim().trim_start_matches('#').trim();
                let can_add = !tag.is_empty() && !entry.tags.iter().any(|existing| existing == tag);
                let submitted = ui
                    .add_enabled(can_add, egui::Button::new("+").small())
                    .on_hover_text("Add tag")
                    .clicked()
                    || (input.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                if submitted && can_add {
                    let mut tags = entry.tags.clone();
                    tags.push(tag.to_owned());
                    if self.save_capture_tags(&entry.id, &tags) {
                        self.tag_input.clear();
                    }
                }
            });
        }
    }

    fn render_editor_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);
        let Some(editor) = self.editor.as_mut() else {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.label(RichText::new(s.no_image_in_editor).size(18.0).strong());
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Double-click any screenshot in Captures, or enable 'Open in image editor' before taking a screenshot.")
                            .color(p.text_muted),
                    );
                });
            });
            return;
        };

        let action = editor.show(ctx, ui, p);
        match action {
            EditorAction::None => {}
            EditorAction::CopyImage(img) => {
                self.status_line = match self.clipboard.copy_image(&img) {
                    Ok(()) => "Copied edited image to clipboard.".into(),
                    Err(err) => format!("Clipboard error: {err}"),
                };
            }
            EditorAction::Save {
                image,
                target_path,
                format,
            } => {
                let save_result = match target_path {
                    Some(path) => image
                        .encode(format)
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| fs::write(&path, bytes).map_err(|e| e.to_string()))
                        .map(|()| path),
                    None => self.write_new_capture_file(&image, format, Utc::now()),
                };
                match save_result {
                    Ok(path) => {
                        self.thumbnails.remove(&path);
                        self.failed_thumbs.remove(&path);
                        if let Some(ed) = self.editor.as_mut() {
                            ed.set_saved(path.clone());
                        }
                        self.status_line = format!("Saved edited screenshot to {}", path.display());
                        self.refresh_history();
                    }
                    Err(err) => self.status_line = format!("Save failed: {err}"),
                }
            }
            EditorAction::SaveAsCopy { image, format } => {
                match self.write_new_capture_file(&image, format, Utc::now()) {
                    Ok(path) => {
                        let now = Utc::now();
                        let history_result = self.history_repository().append(&HistoryEntry {
                            id: CaptureId::from_timestamp(now, image.width()),
                            timestamp: now,
                            source: CaptureSource::PrimaryMonitor,
                            width: image.width(),
                            height: image.height(),
                            format,
                            file_path: Some(path.clone()),
                            upload_url: None,
                            tags: Vec::new(),
                            note: None,
                        });
                        if let Some(ed) = self.editor.as_mut() {
                            ed.set_saved(path.clone());
                        }
                        self.status_line = format!("Saved edited copy to {}", path.display());
                        if let Err(err) = history_result {
                            self.status_line
                                .push_str(&format!(" History update failed: {err}"));
                        }
                        self.refresh_history();
                    }
                    Err(err) => self.status_line = format!("Save copy failed: {err}"),
                }
            }
            EditorAction::Close => {
                self.editor = None;
                self.active_view = ActiveView::Captures;
            }
        }
    }

    fn render_effects_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);
        let latest = self.latest_capture_path().map(Path::to_path_buf);
        if let Some(msg) = self.effects.show(ctx, ui, p, s, latest.as_deref()) {
            self.status_line = msg;
        }
    }

    fn render_tools_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);
        let latest = self.latest_capture_path().map(Path::to_path_buf);
        let output_dir = self.config.output_dir.clone();
        if let Some(msg) = self.tools.show(
            ctx,
            ui,
            p,
            s,
            ToolsContext {
                output_dir: &output_dir,
                latest_capture: latest.as_deref(),
                clipboard: &self.clipboard,
            },
        ) {
            self.status_line = msg;
        }
    }

    fn render_recording_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);
        let output_dir = self.config.output_dir.clone();
        if let Some(msg) = self
            .recording
            .show(ctx, ui, p, s, &output_dir, &self.clipboard)
        {
            self.status_line = msg;
        }
    }

    fn render_uploaders_view(&mut self, ui: &mut egui::Ui, p: &SharexPalette) {
        let selected = self
            .selected_capture
            .as_ref()
            .and_then(|entry| entry.file_path.as_deref())
            .filter(|path| path.is_file());
        let capture = selected.map(|path| (path.to_path_buf(), true)).or_else(|| {
            self.latest_capture_path()
                .map(|path| (path.to_path_buf(), false))
        });
        self.uploaders.show(
            ui,
            p,
            i18n::strings(self.language),
            capture
                .as_ref()
                .map(|(path, selected)| (path.as_path(), *selected)),
        );
    }

    fn render_hotkeys_view(&mut self, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);

        ui.horizontal(|ui| {
            ui.heading(s.hotkey_settings);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let apply_btn =
                    egui::Button::new(RichText::new(s.apply_and_save).color(Color32::WHITE))
                        .fill(p.accent);
                if ui.add(apply_btn).clicked() {
                    self.hotkeys.sync(&self.config.hotkeys);
                    self.persist_config_silent();
                    self.status_line =
                        format!("Applied hotkeys ({} active).", self.hotkeys.active_count());
                }
                if ui.button(s.reset_defaults).clicked() {
                    self.config.hotkeys = default_sharex_hotkeys();
                    self.hotkeys.sync(&self.config.hotkeys);
                    self.persist_config_silent();
                    self.status_line = "Restored default ShareX keybinds.".into();
                }
            });
        });
        ui.label(
            RichText::new("Global hotkeys work even while ShareX is minimized or behind other applications. If Windows Snipping Tool owns PrintScreen, bind another key or disable 'Use the Print screen key to open screen capture' in Windows Accessibility settings.")
                .size(12.0)
                .color(p.text_muted),
        );
        ui.separator();

        for binding in &mut self.config.hotkeys {
            let status = self.hotkeys.status_for(&binding.id);
            egui::Frame::NONE
                .fill(p.card_bg)
                .stroke(Stroke::new(1.0_f32, p.border))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut binding.enabled, "");
                        ui.add_sized(
                            [170.0, 22.0],
                            egui::Label::new(
                                RichText::new(binding.action.label()).strong().size(13.0),
                            ),
                        );

                        let mut combo = binding.combo.unwrap_or_else(|| {
                            HotkeyCombo::new(KeyModifiers::NONE, KeyCode::PrintScreen)
                        });
                        let mut edited = false;

                        if ui.selectable_label(combo.modifiers.ctrl, "Ctrl").clicked() {
                            combo.modifiers.ctrl = !combo.modifiers.ctrl;
                            edited = true;
                        }
                        if ui.selectable_label(combo.modifiers.alt, "Alt").clicked() {
                            combo.modifiers.alt = !combo.modifiers.alt;
                            edited = true;
                        }
                        if ui
                            .selectable_label(combo.modifiers.shift, "Shift")
                            .clicked()
                        {
                            combo.modifiers.shift = !combo.modifiers.shift;
                            edited = true;
                        }

                        egui::ComboBox::from_id_salt(format!("hotkey_key_{}", binding.id))
                            .selected_text(match binding.combo {
                                Some(c) => c.key.display_name(),
                                None => s.status_unbound,
                            })
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                for key in KeyCode::ALL {
                                    if ui
                                        .selectable_label(
                                            binding.combo.is_some_and(|c| c.key == key),
                                            key.display_name(),
                                        )
                                        .clicked()
                                    {
                                        combo.key = key;
                                        edited = true;
                                    }
                                }
                            });

                        if edited {
                            binding.combo = Some(combo);
                        }
                        if ui.small_button(s.clear).clicked() {
                            binding.combo = None;
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (badge_text, badge_color) = match status {
                                HotkeyStatus::Active => (s.status_active.to_owned(), p.success),
                                HotkeyStatus::Disabled => {
                                    (s.status_disabled.to_owned(), p.text_muted)
                                }
                                HotkeyStatus::Unbound => (s.status_unbound.to_owned(), p.warning),
                                HotkeyStatus::Error(msg) => (format!("Conflict: {msg}"), p.danger),
                            };
                            ui.label(RichText::new(badge_text).size(12.0).color(badge_color));
                        });
                    });
                });
            ui.add_space(4.0);
        }
    }

    fn render_workflows_view(&mut self, ui: &mut egui::Ui, p: &SharexPalette) {
        if let Some(err) = self.watcher.last_error() {
            ui.colored_label(p.warning, err);
        }
        let path = WorkflowsPanelState::default_path_for_config(&self.config_path);
        match self.workflows.show(
            ui,
            p,
            &path,
            self.watcher.watched_count(),
            self.watcher.is_active(),
        ) {
            WorkflowsUiAction::None => {}
            WorkflowsUiAction::RulesChanged(rules) => self.watcher.sync_rules(rules),
            WorkflowsUiAction::Saved(message) => {
                self.watcher
                    .sync_rules(self.workflows.watch_rules().to_vec());
                self.status_line = message;
            }
        }
    }

    fn set_autostart(&mut self, enabled: bool) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        AutostartManager::set_enabled(
            AUTOSTART_APP_NAME,
            &exe,
            self.config.start_minimized,
            enabled,
        )?;
        self.run_at_startup = enabled;
        Ok(())
    }

    fn render_settings_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);

        ui.heading(s.application_settings);
        ui.separator();

        egui::Grid::new("sharex_settings_grid")
            .num_columns(2)
            .spacing([14.0, 10.0])
            .show(ui, |ui| {
                ui.label(s.screenshots_folder);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.output_dir_input).desired_width(380.0),
                    );
                    if ui.button(s.open).clicked() {
                        let target = PathBuf::from(self.output_dir_input.trim());
                        let _ = fs::create_dir_all(&target);
                        open_folder_in_os(&target);
                    }
                });
                ui.end_row();

                ui.label(s.filename_pattern);
                ui.add(
                    egui::TextEdit::singleline(&mut self.config.file_name_pattern)
                        .desired_width(380.0),
                );
                ui.end_row();

                ui.label(s.pattern_preview);
                let preview = match FileNamePattern::parse(&self.config.file_name_pattern) {
                    Ok(pattern) => format!(
                        "{}.{}",
                        pattern.expand(&FileNameContext {
                            timestamp: Utc::now(),
                            app_name: "ShareX",
                            width: 1920,
                            height: 1080,
                        }),
                        self.config.image_format.extension()
                    ),
                    Err(err) => format!("Invalid pattern: {err}"),
                };
                ui.label(RichText::new(preview).monospace().color(p.text_muted));
                ui.end_row();

                ui.label(s.image_format);
                egui::ComboBox::from_id_salt("settings_format_picker")
                    .selected_text(self.config.image_format.extension().to_ascii_uppercase())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.config.image_format, ImageFormat::Png, "PNG");
                        ui.selectable_value(
                            &mut self.config.image_format,
                            ImageFormat::Jpeg,
                            "JPEG",
                        );
                        ui.selectable_value(
                            &mut self.config.image_format,
                            ImageFormat::Webp,
                            "WEBP",
                        );
                    });
                ui.end_row();

                ui.label(s.theme);
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.config.dark_theme, s.theme_dark)
                        .clicked()
                    {
                        self.config.dark_theme = true;
                    }
                    if ui
                        .selectable_label(!self.config.dark_theme, s.theme_light)
                        .clicked()
                    {
                        self.config.dark_theme = false;
                    }
                });
                ui.end_row();

                ui.label(s.language);
                ui.horizontal(|ui| {
                    for lang in UiLanguage::ALL {
                        if ui
                            .selectable_label(self.language == lang, lang.display_name())
                            .clicked()
                        {
                            self.set_language(lang);
                        }
                    }
                });
                ui.end_row();
            });

        if self.config.dark_theme != self.theme_applied_dark {
            apply_sharex_theme(ctx, self.config.dark_theme);
            self.theme_applied_dark = self.config.dark_theme;
        }

        ui.add_space(8.0);
        ui.checkbox(&mut self.config.show_toasts, s.toast_notifications);
        ui.checkbox(&mut self.config.play_capture_sound, s.capture_sound);
        ui.add_enabled(
            self.tray.is_available(),
            egui::Checkbox::new(&mut self.config.close_to_tray, s.close_to_tray),
        );
        ui.checkbox(&mut self.config.start_minimized, s.start_minimized);
        let mut next = self.run_at_startup;
        if ui
            .add_enabled(
                cfg!(target_os = "windows"),
                egui::Checkbox::new(&mut next, s.run_at_startup),
            )
            .changed()
            && let Err(err) = self.set_autostart(next)
        {
            self.status_line = format!("Could not change startup registration: {err}");
        }

        ui.add_space(14.0);
        let save_btn =
            egui::Button::new(RichText::new(s.save_settings).color(Color32::WHITE)).fill(p.accent);
        if ui.add(save_btn).clicked() {
            self.config.output_dir = PathBuf::from(self.output_dir_input.trim());
            self.status_line = match self.config.save(&self.config_path) {
                Ok(()) => {
                    if self.run_at_startup
                        && let Err(err) = self.set_autostart(true)
                    {
                        format!(
                            "Settings saved, but startup registration could not be updated: {err}"
                        )
                    } else {
                        format!("Saved settings to {}", self.config_path.display())
                    }
                }
                Err(err) => format!("Failed to save settings: {err}"),
            };
        }
    }
}

impl eframe::App for SharexGuiApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(palette(self.config.dark_theme).panel_bg).to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.uploaders.poll_upload(&self.clipboard);

        if self.hotkeys.has_conflicts() {
            if self.last_hotkey_retry.elapsed() >= Duration::from_secs(2) {
                self.hotkeys.retry_conflicts(&self.config.hotkeys);
                self.last_hotkey_retry = Instant::now();
            }
            ctx.request_repaint_after(Duration::from_secs(2));
        }

        for action in self.tray.poll_actions() {
            self.handle_tray_action(ctx, action);
        }
        if ctx.input(|input| input.viewport().close_requested())
            && self.config.close_to_tray
            && !self.force_quit
            && self.tray.is_available()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        let pending_actions = self.hotkeys.poll_actions();
        for event in pending_actions {
            self.trigger_hotkey_event(ctx, event);
        }

        self.update_overlay(ctx);
        self.poll_ocr(ctx);
        self.poll_capture_uploads(ctx);
        self.poll_watcher(ctx);

        if let Some(msg) = self.recording.poll_finalize() {
            self.status_line = msg;
        }
        if self.recording.is_recording()
            && let Some(msg) = self.recording.show_hud(ctx)
        {
            self.status_line = msg;
        }

        self.pins.show_all(ctx);

        let p = palette(self.config.dark_theme);

        egui::TopBottomPanel::bottom("sharex_status_bar")
            .frame(
                egui::Frame::NONE
                    .fill(p.sidebar_bg)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .inner_margin(Margin::symmetric(10, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Hotkeys: {}/{} active",
                            self.hotkeys.active_count(),
                            self.config.hotkeys.len()
                        ))
                        .size(12.0)
                        .color(if self.hotkeys.active_count() > 0 {
                            p.success
                        } else {
                            p.warning
                        }),
                    );
                    ui.separator();
                    let tray_label = if self.tray.is_available() {
                        "Tray: on"
                    } else {
                        "Tray: off"
                    };
                    ui.label(RichText::new(tray_label).size(12.0).color(
                        if self.tray.is_available() {
                            p.success
                        } else {
                            p.text_muted
                        },
                    ));
                    if !self.pins.is_empty() {
                        ui.separator();
                        ui.label(
                            RichText::new(format!("Pins: {}", self.pins.count()))
                                .size(12.0)
                                .color(p.accent),
                        );
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(&self.status_line)
                            .size(12.0)
                            .color(p.text_muted),
                    );
                });
            });

        egui::SidePanel::left("sharex_sidebar")
            .resizable(false)
            .exact_width(228.0)
            .frame(
                egui::Frame::NONE
                    .fill(p.sidebar_bg)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .inner_margin(Margin::same(10)),
            )
            .show(ctx, |ui| {
                self.render_sidebar(ctx, ui, &p);
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(p.panel_bg)
                    .inner_margin(Margin::same(12)),
            )
            .show(ctx, |ui| match self.active_view {
                ActiveView::Captures => self.render_captures_view(ctx, ui, &p),
                ActiveView::Editor => self.render_editor_view(ctx, ui, &p),
                ActiveView::Effects => self.render_effects_view(ctx, ui, &p),
                ActiveView::Tools => self.render_tools_view(ctx, ui, &p),
                ActiveView::Recording => self.render_recording_view(ctx, ui, &p),
                ActiveView::Uploaders => self.render_uploaders_view(ui, &p),
                ActiveView::Workflows => self.render_workflows_view(ui, &p),
                ActiveView::Hotkeys => self.render_hotkeys_view(ui, &p),
                ActiveView::Settings => self.render_settings_view(ctx, ui, &p),
            });

        if self.tools.take_pick_color_request() {
            self.start_overlay_with_mode(ctx, OverlayMode::ColorPicker);
        }
        if self.tools.take_ocr_region_request() {
            self.start_overlay_with_mode(ctx, OverlayMode::Ocr);
        }
        if self.recording.take_select_region_request() {
            self.start_overlay_with_mode(ctx, OverlayMode::SelectRecordingRegion);
        }

        if !self.config.show_toasts {
            self.toasts.dismiss_all();
        }
        match self.toasts.show_all(ctx, self.config.dark_theme) {
            ToastAction::None => {}
            ToastAction::OpenPath(path) => {
                open_folder_in_os(if path.is_file() {
                    path.parent().unwrap_or(&path)
                } else {
                    &path
                });
            }
            ToastAction::OpenUrl(url) => {
                if url.starts_with("https://") || url.starts_with("http://") {
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                }
            }
        }

        if self.recording.is_recording()
            || self.recording.is_finalizing()
            || self.uploaders.is_uploading()
            || self.ocr_pending.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

pub(crate) fn capture_overlay_snapshot(
    capturer: &XcapCapturer,
) -> Result<DesktopSnapshot, sharex_core::CoreError> {
    capturer
        .capture_virtual_desktop()
        .or_else(|_| capturer.capture_primary_snapshot())
}

fn show_main_window(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

fn capture_filename_stem(
    config: &AppConfig,
    image: &CapturedImage,
    now: chrono::DateTime<Utc>,
) -> Result<String, String> {
    Ok(FileNamePattern::parse(&config.file_name_pattern)
        .map_err(|err| err.to_string())?
        .expand(&FileNameContext {
            timestamp: now,
            app_name: "ShareX",
            width: image.width(),
            height: image.height(),
        }))
}

fn write_capture_file(
    config: &AppConfig,
    image: &CapturedImage,
    format: ImageFormat,
    now: chrono::DateTime<Utc>,
) -> Result<PathBuf, String> {
    let stem = capture_filename_stem(config, image, now)?;
    fs::create_dir_all(&config.output_dir).map_err(|err| err.to_string())?;
    let path = resolve_unique_path(&config.output_dir, &stem, format.extension());
    let bytes = image.encode(format).map_err(|err| err.to_string())?;
    fs::write(&path, bytes).map_err(|err| err.to_string())?;
    Ok(path)
}

fn move_watched_file(path: &Path, folder: &Path) -> Result<PathBuf, std::io::Error> {
    let name = path.file_name().ok_or(std::io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(folder)?;
    let target = folder.join(name);
    if target.exists() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    fs::rename(path, &target)?;
    Ok(target)
}

fn first_existing_capture_path(entries: &[HistoryEntry]) -> Option<&Path> {
    entries
        .iter()
        .find_map(|entry| entry.file_path.as_deref().filter(|path| path.is_file()))
}

fn capture_history_query(filter: &str) -> HistoryQuery {
    let filter = filter.trim();
    let mut query = HistoryQuery {
        limit: Some(HISTORY_LIMIT),
        ..HistoryQuery::default()
    };
    if let Some(tag_filter) = filter.strip_prefix('#') {
        let mut parts = tag_filter.splitn(2, char::is_whitespace);
        if let Some(tag) = parts.next().filter(|tag| !tag.is_empty()) {
            query.tag = Some(tag.to_owned());
            query.text = parts
                .next()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned);
            return query;
        }
    }
    query.text = (!filter.is_empty()).then(|| filter.to_owned());
    query
}

fn sidebar_action_button(ui: &mut egui::Ui, label: &str, hotkey: &str, p: &SharexPalette) -> bool {
    let desired = Vec2::new(ui.available_width(), 28.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    let bg = if resp.hovered() {
        p.hover_bg
    } else {
        p.card_bg
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, if resp.hovered() { p.accent } else { p.border }),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        egui::pos2(rect.min.x + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        p.text_primary,
    );
    ui.painter().text(
        egui::pos2(rect.max.x - 8.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        hotkey,
        egui::FontId::monospace(10.5),
        p.text_muted,
    );
    resp.clicked()
}

fn sidebar_nav_button(ui: &mut egui::Ui, label: &str, active: bool, p: &SharexPalette) -> bool {
    let desired = Vec2::new(ui.available_width(), 28.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    let bg = if active {
        p.accent
    } else if resp.hovered() {
        p.hover_bg
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    ui.painter().text(
        egui::pos2(rect.min.x + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        if active {
            Color32::WHITE
        } else {
            p.text_primary
        },
    );
    resp.clicked()
}

fn open_folder_in_os(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_history_filter_supports_plain_text_and_tag_prefix() {
        for (filter, text, tag) in [
            ("   ", None, None),
            (" Step #2 ", Some("Step #2"), None),
            (" #Bug ", None, Some("Bug")),
            (" #Bug   Login screen ", Some("Login screen"), Some("Bug")),
        ] {
            let query = capture_history_query(filter);
            assert_eq!(query.text.as_deref(), text);
            assert_eq!(query.tag.as_deref(), tag);
            assert_eq!(query.limit, Some(HISTORY_LIMIT));
        }
    }
}
