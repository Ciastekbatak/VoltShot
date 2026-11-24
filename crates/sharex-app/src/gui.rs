use crate::autostart::AutostartManager;
use crate::clipboard::SystemClipboard;
use crate::editor_ui::{EditorAction, EditorViewState};
use crate::effects_ui::EffectsPanelState;
use crate::hotkeys::{HotkeyRuntime, HotkeyStatus, TriggeredHotkey};
use crate::i18n::{self, UiLanguage};
use crate::icons::{Icon, paint_icon, palette_icon_button};
use crate::notifications::{ToastAction, ToastManager};
use crate::pin_window::PinManager;
use crate::recording_ui::RecordingPanelState;
use crate::region_overlay::{OverlayMode, OverlayModeOutcome, RegionOverlayState};
use crate::theme::{
    SharexPalette, apply_sharex_theme, card_frame, hotkey_badge, paint_brand_mark, palette,
    primary_cta, section_label, status_dot, view_heading,
};
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

const THUMB_CACHE_CAPACITY: usize = 32;
const THUMB_MAX_SIDE: usize = 360;
const THUMBS_DECODED_PER_FRAME: usize = 2;
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
    let window_icon = egui::IconData {
        rgba: crate::tray::build_tray_icon_rgba(64),
        width: 64,
        height: 64,
    };
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration {
        desired_maximum_frame_latency: Some(1),
        ..Default::default()
    };
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(mut create) = wgpu_options.wgpu_setup {
        create.power_preference = eframe::egui_wgpu::wgpu::PowerPreference::LowPower;
        let base_desc = create.device_descriptor;
        create.device_descriptor = std::sync::Arc::new(move |adapter| {
            let mut desc = base_desc(adapter);
            desc.memory_hints = eframe::egui_wgpu::wgpu::MemoryHints::MemoryUsage;
            desc
        });
        wgpu_options.wgpu_setup = eframe::egui_wgpu::WgpuSetup::CreateNew(create);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ShareX (Rust)")
            .with_icon(std::sync::Arc::new(window_icon))
            .with_inner_size([1180.0, 740.0])
            .with_min_inner_size([880.0, 560.0])
            .with_visible(!start_minimized),
        wgpu_options,
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
    source_signature: Option<(u64, Option<std::time::SystemTime>)>,
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
    existing_files: HashMap<PathBuf, bool>,
    drag_candidate: Option<(PathBuf, egui::Pos2)>,
    extended_fonts_loaded: bool,
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

        crate::fonts::install_native_fonts(&cc.egui_ctx);
        let extended_fonts_loaded = if language.needs_extended_unicode_fonts() {
            crate::fonts::install_extended_unicode_fonts(&cc.egui_ctx);
            true
        } else {
            false
        };
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
            existing_files: HashMap::new(),
            drag_candidate: None,
            extended_fonts_loaded,
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

    fn set_language(&mut self, ctx: &egui::Context, language: UiLanguage) {
        if self.language != language {
            self.language = language;
            if language.needs_extended_unicode_fonts() && !self.extended_fonts_loaded {
                crate::fonts::install_extended_unicode_fonts(ctx);
                self.extended_fonts_loaded = true;
            }
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
        let file_name = completion
            .entry
            .file_path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                let stem = capture_filename_stem(config, image, completion.entry.timestamp)
                    .unwrap_or_else(|_| "ShareX".to_owned());
                format!("{stem}_{}.{}", completion.entry.id, format.extension())
            });
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
                let current_sig = fs::metadata(&watched.event.path)
                    .ok()
                    .map(|meta| (meta.len(), meta.modified().ok()));
                if watched.source_signature.is_some() && current_sig != watched.source_signature {
                    completion
                        .status
                        .push_str(". Skipped move: watched file changed while processing");
                } else {
                    match move_watched_file(&watched.event.path, folder) {
                        Ok(target) => {
                            if completion.entry.file_path.as_ref() == Some(&watched.event.path) {
                                completion.entry.file_path = Some(target);
                            }
                        }
                        Err(err) => completion.status.push_str(&format!(". Move failed: {err}")),
                    }
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
            let source_signature = fs::metadata(&event.path)
                .ok()
                .map(|meta| (meta.len(), meta.modified().ok()));
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
            if settings.config.pipeline.uploader_id.as_deref() == Some("local-mirror")
                && self
                    .uploaders
                    .mirror_folder_path()
                    .is_some_and(|dir| self.is_watched_directory(&dir))
            {
                self.status_line =
                    "Skipped Local Mirror upload: mirror folder is a watched directory.".into();
                settings.config.pipeline.uploader_id = None;
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
                Some(WatchedCapture {
                    event,
                    move_after,
                    source_signature,
                }),
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
        self.existing_files.clear();
        for entry in &self.history_entries {
            if let Some(path) = &entry.file_path {
                self.existing_files.insert(path.clone(), path.is_file());
            }
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
            let found = match self.history_repository().search(&query) {
                Ok(entries) => entries,
                Err(err) => {
                    self.status_line = format!("History search failed: {err}");
                    Vec::new()
                }
            };
            for entry in &found {
                if let Some(path) = &entry.file_path {
                    self.existing_files
                        .entry(path.clone())
                        .or_insert_with(|| path.is_file());
                }
            }
            Some(found)
        };
    }

    fn path_exists_cached(&self, path: &Path) -> bool {
        self.existing_files.get(path).copied().unwrap_or(false)
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

    fn ensure_thumbnail(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        decode_budget: &mut usize,
    ) -> Option<egui::TextureId> {
        let key = path.to_path_buf();
        if let Some(tex) = self.thumbnails.get(&key) {
            return Some(tex.id());
        }
        if self.failed_thumbs.contains_key(&key) || !self.path_exists_cached(path) {
            return None;
        }
        if *decode_budget == 0 {
            ctx.request_repaint();
            return None;
        }
        *decode_budget -= 1;
        match CapturedImage::load_from_file(path) {
            Ok(img) => {
                let color = crate::image_texture::bounded_color_image(&img, THUMB_MAX_SIDE);
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
        ui.add_space(2.0_f32);
        ui.horizontal(|ui| {
            paint_brand_mark(ui, 24.0_f32, p);
            ui.add_space(2.0_f32);
            ui.label(
                RichText::new("ShareX")
                    .size(17.0_f32)
                    .strong()
                    .color(p.text_primary),
            );
            egui::Frame::NONE
                .fill(p.accent_subtle)
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(5, 1))
                .show(ui, |ui| {
                    ui.label(RichText::new("RS").size(9.5_f32).strong().color(p.accent));
                });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut chosen = self.language;
                egui::ComboBox::from_id_salt("sidebar_lang_picker")
                    .selected_text(
                        RichText::new(self.language.short_code())
                            .size(10.5_f32)
                            .strong()
                            .color(p.text_primary),
                    )
                    .width(54.0_f32)
                    .show_ui(ui, |ui| {
                        for lang in UiLanguage::ALL {
                            let label = format!("{} ({})", lang.display_name(), lang.short_code());
                            ui.selectable_value(&mut chosen, lang, label);
                        }
                    });
                if chosen != self.language {
                    self.set_language(ctx, chosen);
                }
            });
        });

        ui.add_space(12.0_f32);

        let s = i18n::strings(self.language);

        if sidebar_primary_capture_button(
            ui,
            Icon::Region,
            s.capture_region_cta,
            &self.hotkey_hint(HotkeyAction::CaptureRegion),
            p,
        ) {
            self.start_region_overlay(ctx);
        }

        ui.add_space(6.0_f32);

        egui::ScrollArea::vertical()
            .id_salt("sharex_sidebar_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                if sidebar_action_button(
                    ui,
                    Icon::Fullscreen,
                    s.fullscreen,
                    &self.hotkey_hint(HotkeyAction::CaptureFullscreen),
                    p,
                ) {
                    self.capture_source(ctx, CaptureSource::PrimaryMonitor);
                }
                if sidebar_action_button(
                    ui,
                    Icon::Window,
                    s.active_window,
                    &self.hotkey_hint(HotkeyAction::CaptureActiveWindow),
                    p,
                ) {
                    self.capture_source(ctx, CaptureSource::ActiveWindow);
                }
                if sidebar_action_button(
                    ui,
                    Icon::LastRegion,
                    s.last_region,
                    &self.hotkey_hint(HotkeyAction::CaptureLastRegion),
                    p,
                ) {
                    self.capture_last_region(ctx, self.capture_settings());
                }
                if sidebar_action_button(
                    ui,
                    Icon::OcrText,
                    s.ocr_tool,
                    &self.hotkey_hint(HotkeyAction::CaptureOcr),
                    p,
                ) {
                    self.start_overlay_with_mode(ctx, OverlayMode::Ocr);
                }
                if sidebar_action_button(
                    ui,
                    Icon::EyeDropper,
                    s.color_tool,
                    &self.hotkey_hint(HotkeyAction::PickScreenColor),
                    p,
                ) {
                    self.start_overlay_with_mode(ctx, OverlayMode::ColorPicker);
                }

                ui.add_space(8.0_f32);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0_f32;
                    let selected_label = self
                        .monitors
                        .get(self.selected_monitor_idx)
                        .map(friendly_monitor_label)
                        .unwrap_or_else(|| s.no_monitor.to_owned());
                    let go_width = 40.0_f32;
                    let combo_width = (ui.available_width() - go_width - 8.0_f32).max(110.0_f32);
                    egui::ComboBox::from_id_salt("sidebar_monitor")
                        .selected_text(selected_label)
                        .width(combo_width)
                        .truncate()
                        .show_ui(ui, |ui| {
                            for (idx, m) in self.monitors.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.selected_monitor_idx,
                                    idx,
                                    friendly_monitor_label(m),
                                );
                            }
                        });
                    let go_btn = egui::Button::new(
                        RichText::new(s.capture_monitor_go)
                            .size(11.5_f32)
                            .color(p.text_primary),
                    )
                    .fill(p.card_bg)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .corner_radius(CornerRadius::same(6))
                    .min_size(Vec2::new(go_width, 24.0_f32));
                    if ui.add(go_btn).clicked()
                        && let Some(m) = self.monitors.get(self.selected_monitor_idx)
                    {
                        self.capture_source(ctx, CaptureSource::Monitor { id: m.id });
                    }
                });

                ui.add_space(10.0_f32);
                ui.separator();
                ui.add_space(4.0_f32);
                section_label(ui, s.workspace_section, p);

                if sidebar_nav_button(
                    ui,
                    Icon::Captures,
                    s.captures,
                    self.active_view == ActiveView::Captures,
                    None,
                    p,
                ) {
                    self.active_view = ActiveView::Captures;
                    self.refresh_history();
                }
                let editor_dirty = self.editor.as_ref().is_some_and(EditorViewState::is_dirty);
                let editor_label = match &self.editor {
                    Some(_) => s.image_editor,
                    None => s.image_editor_empty,
                };
                if sidebar_nav_button(
                    ui,
                    Icon::Editor,
                    editor_label,
                    self.active_view == ActiveView::Editor,
                    editor_dirty.then_some(p.accent),
                    p,
                ) {
                    self.active_view = ActiveView::Editor;
                }
                if sidebar_nav_button(
                    ui,
                    Icon::Effects,
                    s.effects,
                    self.active_view == ActiveView::Effects,
                    None,
                    p,
                ) {
                    self.active_view = ActiveView::Effects;
                }
                if sidebar_nav_button(
                    ui,
                    Icon::Tools,
                    s.tools,
                    self.active_view == ActiveView::Tools,
                    None,
                    p,
                ) {
                    self.active_view = ActiveView::Tools;
                }
                let is_recording = self.recording.is_recording();
                let recording_label = if is_recording {
                    format!("{} · REC", s.recording)
                } else {
                    s.recording.to_owned()
                };
                if sidebar_nav_button(
                    ui,
                    Icon::Recording,
                    &recording_label,
                    self.active_view == ActiveView::Recording,
                    is_recording.then_some(p.danger),
                    p,
                ) {
                    self.active_view = ActiveView::Recording;
                }
                if sidebar_nav_button(
                    ui,
                    Icon::Upload,
                    s.uploaders,
                    self.active_view == ActiveView::Uploaders,
                    None,
                    p,
                ) {
                    self.active_view = ActiveView::Uploaders;
                    self.refresh_history();
                }
                let watchers_active = self.watcher.is_active() && self.watcher.watched_count() > 0;
                if sidebar_nav_button(
                    ui,
                    Icon::Workflow,
                    s.workflows,
                    self.active_view == ActiveView::Workflows,
                    watchers_active.then_some(p.success),
                    p,
                ) {
                    self.active_view = ActiveView::Workflows;
                }
                if sidebar_nav_button(
                    ui,
                    Icon::Hotkeys,
                    s.hotkey_settings,
                    self.active_view == ActiveView::Hotkeys,
                    None,
                    p,
                ) {
                    self.active_view = ActiveView::Hotkeys;
                }
                if sidebar_nav_button(
                    ui,
                    Icon::Settings,
                    s.application_settings,
                    self.active_view == ActiveView::Settings,
                    None,
                    p,
                ) {
                    self.active_view = ActiveView::Settings;
                }

                ui.add_space(10.0_f32);
                ui.separator();
                ui.add_space(4.0_f32);

                let active_after_tasks = [
                    self.config.pipeline.save_to_file,
                    self.config.pipeline.copy_image_to_clipboard,
                    self.config.pipeline.open_in_editor,
                    self.config.pipeline.copy_path_to_clipboard,
                    self.effects.apply_on_capture,
                ]
                .into_iter()
                .filter(|enabled| *enabled)
                .count();
                let after_header = format!("{} ({active_after_tasks})", s.after_capture_tasks);

                egui::CollapsingHeader::new(
                    RichText::new(after_header)
                        .size(11.5_f32)
                        .strong()
                        .color(p.text_muted),
                )
                .id_salt("sidebar_after_capture")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_space(2.0_f32);
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
                });
            });
    }

    fn render_captures_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);
        let visible_len = self
            .search_results
            .as_ref()
            .map_or(self.history_entries.len(), Vec::len);

        ui.horizontal(|ui| {
            view_heading(ui, s.captures, p);
            if visible_len > 0 {
                egui::Frame::NONE
                    .fill(p.surface_elevated)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(7, 2))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(visible_len.to_string())
                                .size(11.0_f32)
                                .color(p.text_muted),
                        );
                    });
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(s.refresh).clicked() {
                    self.refresh_history();
                    self.refresh_monitors();
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
                if !self.search_filter.is_empty()
                    && ui
                        .add(
                            egui::Button::new(
                                RichText::new("×").size(13.0_f32).color(p.text_muted),
                            )
                            .fill(p.surface_elevated)
                            .stroke(Stroke::new(1.0_f32, p.border))
                            .corner_radius(CornerRadius::same(6))
                            .min_size(Vec2::new(24.0_f32, 24.0_f32)),
                        )
                        .on_hover_text(s.clear)
                        .clicked()
                {
                    self.search_filter.clear();
                    self.refresh_history_search();
                }
                let search_hint = if self.language == UiLanguage::En {
                    "Search captures or #tag..."
                } else {
                    s.filter_captures_hint
                };
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.search_filter)
                            .hint_text(search_hint)
                            .margin(Margin::symmetric(10, 6))
                            .desired_width(210.0_f32),
                    )
                    .on_hover_text("Use #tag or #tag followed by filter text.")
                    .changed()
                {
                    self.refresh_history_search();
                }
            });
        });
        ui.add_space(6.0_f32);
        ui.separator();
        ui.add_space(6.0_f32);

        if visible_len == 0 {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(56.0_f32);
                    card_frame(p)
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(Margin::symmetric(32, 28))
                        .show(ui, |ui| {
                            ui.set_max_width(400.0_f32);
                            ui.vertical_centered(|ui| {
                                paint_empty_viewfinder(ui, 52.0_f32, p);
                                ui.add_space(14.0_f32);
                                if self.search_results.is_some() {
                                    ui.label(
                                        RichText::new("No captures match this filter.")
                                            .size(16.0_f32)
                                            .strong()
                                            .color(p.text_primary),
                                    );
                                    ui.add_space(6.0_f32);
                                    ui.label(
                                        RichText::new("Try a different search query or #tag.")
                                            .size(12.5_f32)
                                            .color(p.text_muted),
                                    );
                                    ui.add_space(14.0_f32);
                                    if ui.button(s.clear).clicked() {
                                        self.search_filter.clear();
                                        self.refresh_history_search();
                                    }
                                    return;
                                }
                                ui.label(
                                    RichText::new(s.no_screenshots_yet)
                                        .size(17.0_f32)
                                        .strong()
                                        .color(p.text_primary),
                                );
                                ui.add_space(6.0_f32);
                                ui.label(
                                    RichText::new(
                                        "Capture a region, window, or monitor to see it here.",
                                    )
                                    .size(12.5_f32)
                                    .color(p.text_muted),
                                );
                                ui.add_space(16.0_f32);
                                if ui.add(primary_cta(s.capture_region_cta, p)).clicked() {
                                    self.start_region_overlay(ctx);
                                }
                                ui.add_space(16.0_f32);
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(s.region).size(11.5_f32).color(p.text_muted),
                                    );
                                    hotkey_badge(
                                        ui,
                                        &self.hotkey_hint(HotkeyAction::CaptureRegion),
                                        p,
                                    );
                                    ui.add_space(6.0_f32);
                                    ui.label(
                                        RichText::new(s.fullscreen)
                                            .size(11.5_f32)
                                            .color(p.text_muted),
                                    );
                                    hotkey_badge(
                                        ui,
                                        &self.hotkey_hint(HotkeyAction::CaptureFullscreen),
                                        p,
                                    );
                                });
                            });
                        });
                });
            });
            return;
        }

        let mut open_editor_path: Option<PathBuf> = None;
        let mut pin_file_path: Option<PathBuf> = None;
        let mut copy_img_path: Option<PathBuf> = None;
        let mut copy_text_val: Option<String> = None;
        let mut delete_file_path: Option<PathBuf> = None;
        let mut drag_out_path: Option<PathBuf> = None;
        let mut select_index: Option<usize> = None;
        let mut decode_budget = THUMBS_DECODED_PER_FRAME;
        let (pointer_primary_down, pointer_pos) =
            ctx.input(|input| (input.pointer.primary_down(), input.pointer.interact_pos()));

        if !pointer_primary_down {
            self.drag_candidate = None;
        } else if let (Some((candidate_path, start_pos)), Some(current_pos)) =
            (&self.drag_candidate, pointer_pos)
            && current_pos.distance(*start_pos) >= 6.0_f32
        {
            drag_out_path = Some(candidate_path.clone());
            self.drag_candidate = None;
        }

        egui::ScrollArea::vertical()
            .id_salt("captures_gallery_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let gap = 14.0_f32;
                let avail_w = (ui.available_width() - 4.0_f32).max(260.0_f32);
                let cols = (((avail_w + gap) / 292.0_f32).floor() as usize).clamp(1, 4);
                let card_outer_w = ((avail_w - gap * ((cols - 1) as f32)) / (cols as f32))
                    .floor()
                    .max(220.0_f32);
                let inner_w = (card_outer_w - 20.0_f32).max(180.0_f32);
                let thumb_h = (inner_w * 0.5625_f32).round().clamp(128.0_f32, 184.0_f32);

                let row_count = visible_len.div_ceil(cols);
                for row_idx in 0..row_count {
                    let row_start = row_idx * cols;
                    let row_end = (row_start + cols).min(visible_len);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for idx in row_start..row_end {
                            let (
                                entry_id,
                                entry_path,
                                entry_width,
                                entry_height,
                                entry_timestamp,
                                entry_has_tags,
                            ) = {
                                let entry = self
                                    .search_results
                                    .as_ref()
                                    .map_or(&self.history_entries[idx], |res| &res[idx]);
                                (
                                    entry.id.clone(),
                                    entry.file_path.clone(),
                                    entry.width,
                                    entry.height,
                                    entry.timestamp,
                                    !entry.tags.is_empty(),
                                )
                            };
                            let is_selected = self
                                .selected_capture
                                .as_ref()
                                .is_some_and(|selected| selected.id == entry_id);
                            let card_id =
                                ui.make_persistent_id(("capture_card", entry_id.as_str()));
                            let hovered_prev =
                                ui.data(|data| data.get_temp::<bool>(card_id).unwrap_or(false));
                            let stroke_width = if is_selected { 1.5_f32 } else { 1.0_f32 };
                            let stroke_color = if is_selected || hovered_prev {
                                p.accent
                            } else {
                                p.border
                            };

                            let card_resp = egui::Frame::NONE
                                .fill(p.card_bg)
                                .stroke(Stroke::new(stroke_width, stroke_color))
                                .corner_radius(CornerRadius::same(10))
                                .inner_margin(Margin::same(10))
                                .show(ui, |ui| {
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(inner_w, 0.0_f32),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.set_min_width(inner_w);
                                            ui.set_max_width(inner_w);

                                            let stage_size = Vec2::new(inner_w, thumb_h);
                                            let (stage_rect, stage_resp) = ui.allocate_exact_size(
                                                stage_size,
                                                egui::Sense::click_and_drag(),
                                            );
                                            let file_exists = entry_path
                                                .as_deref()
                                                .is_some_and(|path| self.path_exists_cached(path));
                                            if stage_resp
                                                .drag_started_by(egui::PointerButton::Primary)
                                                && file_exists
                                                && let (Some(path), Some(pos)) = (
                                                    &entry_path,
                                                    stage_resp.interact_pointer_pos(),
                                                )
                                            {
                                                self.drag_candidate = Some((path.clone(), pos));
                                            }
                                            if stage_resp.clicked() {
                                                select_index = Some(idx);
                                            }
                                            if stage_resp.double_clicked()
                                                && let Some(path) = &entry_path
                                            {
                                                open_editor_path = Some(path.clone());
                                            }
                                            if file_exists {
                                                stage_resp.on_hover_text(
                                                    "Click to select · Double-click to edit · Drag to drop into folders or apps",
                                                );
                                            }

                                            if ui.is_rect_visible(stage_rect) {
                                                ui.painter().rect_filled(
                                                    stage_rect,
                                                    CornerRadius::same(7),
                                                    p.sidebar_bg,
                                                );
                                                ui.painter().rect_stroke(
                                                    stage_rect,
                                                    CornerRadius::same(7),
                                                    Stroke::new(1.0_f32, p.divider),
                                                    egui::StrokeKind::Inside,
                                                );

                                                let thumb_tex =
                                                    entry_path.as_deref().and_then(|path| {
                                                        self.ensure_thumbnail(
                                                            ctx,
                                                            path,
                                                            &mut decode_budget,
                                                        )
                                                    });
                                                if let Some(tex_id) = thumb_tex {
                                                    let inner = stage_rect.shrink(3.0_f32);
                                                    let aspect = if entry_height > 0 {
                                                        entry_width as f32 / entry_height as f32
                                                    } else {
                                                        inner.width() / inner.height().max(1.0_f32)
                                                    };
                                                    let fit_size = if inner.width()
                                                        / inner.height().max(1.0_f32)
                                                        > aspect
                                                    {
                                                        Vec2::new(
                                                            (inner.height() * aspect)
                                                                .min(inner.width()),
                                                            inner.height(),
                                                        )
                                                    } else {
                                                        Vec2::new(
                                                            inner.width(),
                                                            (inner.width() / aspect.max(0.01_f32))
                                                                .min(inner.height()),
                                                        )
                                                    };
                                                    let img_rect = egui::Rect::from_center_size(
                                                        inner.center(),
                                                        fit_size,
                                                    );
                                                    egui::Image::new((tex_id, fit_size))
                                                        .corner_radius(CornerRadius::same(5))
                                                        .paint_at(ui, img_rect);
                                                } else {
                                                    ui.painter().text(
                                                        stage_rect.center(),
                                                        egui::Align2::CENTER_CENTER,
                                                        s.no_preview,
                                                        egui::FontId::proportional(11.5_f32),
                                                        p.text_muted,
                                                    );
                                                }
                                            }

                                            ui.add_space(8.0_f32);
                                            let title = entry_path
                                                .as_ref()
                                                .and_then(|fp| fp.file_name())
                                                .map(|n| n.to_string_lossy().into_owned())
                                                .unwrap_or_else(|| entry_id.as_str().to_owned());
                                            let dims_text =
                                                format!("{entry_width}×{entry_height}");
                                            let (meta_rect, meta_resp) = ui.allocate_exact_size(
                                                Vec2::new(inner_w, 18.0_f32),
                                                egui::Sense::click(),
                                            );
                                            if meta_resp.on_hover_text(&title).clicked() {
                                                select_index = Some(idx);
                                            }
                                            let dims_galley = ui.painter().layout_no_wrap(
                                                dims_text,
                                                egui::FontId::monospace(10.5_f32),
                                                p.text_muted,
                                            );
                                            let dims_w = dims_galley.size().x;
                                            let dims_pos = egui::pos2(
                                                meta_rect.max.x - dims_w,
                                                meta_rect.center().y
                                                    - dims_galley.size().y * 0.5_f32,
                                            );
                                            let title_max_w =
                                                (inner_w - dims_w - 10.0_f32).max(40.0_f32);
                                            let mut title_job = egui::text::LayoutJob::simple(
                                                title,
                                                egui::FontId::proportional(12.5_f32),
                                                p.text_primary,
                                                title_max_w,
                                            );
                                            title_job.wrap.max_rows = 1;
                                            title_job.wrap.break_anywhere = true;
                                            let title_galley = ui.painter().layout_job(title_job);
                                            let title_clip = egui::Rect::from_min_max(
                                                meta_rect.min,
                                                egui::pos2(
                                                    meta_rect.min.x + title_max_w,
                                                    meta_rect.max.y,
                                                ),
                                            );
                                            ui.painter().with_clip_rect(title_clip).galley(
                                                egui::pos2(
                                                    meta_rect.min.x,
                                                    meta_rect.center().y
                                                        - title_galley.size().y * 0.5_f32,
                                                ),
                                                title_galley,
                                                p.text_primary,
                                            );
                                            ui.painter().galley(
                                                dims_pos,
                                                dims_galley,
                                                p.text_muted,
                                            );

                                            if entry_has_tags {
                                                let entry_clone = self
                                                    .search_results
                                                    .as_ref()
                                                    .map_or(&self.history_entries[idx], |res| {
                                                        &res[idx]
                                                    })
                                                    .clone();
                                                ui.add_space(4.0_f32);
                                                self.render_capture_tags(ui, p, &entry_clone);
                                            }

                                            ui.add_space(4.0_f32);
                                            ui.horizontal(|ui| {
                                                ui.set_width(inner_w);
                                                ui.label(
                                                    RichText::new(
                                                        entry_timestamp
                                                            .format("%H:%M:%S")
                                                            .to_string(),
                                                    )
                                                    .size(11.0_f32)
                                                    .color(p.text_muted),
                                                );
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        ui.spacing_mut().item_spacing.x = 2.0_f32;
                                                        if let Some(path) = &entry_path {
                                                            if file_exists
                                                                && palette_icon_button(
                                                                    ui,
                                                                    Icon::Trash,
                                                                    s.delete_file,
                                                                    true,
                                                                    p,
                                                                )
                                                                .clicked()
                                                            {
                                                                delete_file_path =
                                                                    Some(path.clone());
                                                            }
                                                            if palette_icon_button(
                                                                ui,
                                                                Icon::Link,
                                                                s.copy_path,
                                                                false,
                                                                p,
                                                            )
                                                            .clicked()
                                                            {
                                                                copy_text_val = Some(
                                                                    path.display().to_string(),
                                                                );
                                                            }
                                                            if file_exists
                                                                && palette_icon_button(
                                                                    ui,
                                                                    Icon::Copy,
                                                                    s.copy_image,
                                                                    false,
                                                                    p,
                                                                )
                                                                .clicked()
                                                            {
                                                                copy_img_path = Some(path.clone());
                                                            }
                                                            if file_exists
                                                                && palette_icon_button(
                                                                    ui,
                                                                    Icon::Pin,
                                                                    s.pin_to_screen,
                                                                    false,
                                                                    p,
                                                                )
                                                                .clicked()
                                                            {
                                                                pin_file_path = Some(path.clone());
                                                            }
                                                            if file_exists
                                                                && palette_icon_button(
                                                                    ui,
                                                                    Icon::Editor,
                                                                    s.edit,
                                                                    false,
                                                                    p,
                                                                )
                                                                .clicked()
                                                            {
                                                                open_editor_path =
                                                                    Some(path.clone());
                                                            }
                                                        }
                                                    },
                                                );
                                            });
                                        },
                                    );
                                });

                            let is_hovered = card_resp.response.hovered();
                            if is_hovered != hovered_prev {
                                ui.data_mut(|data| data.insert_temp(card_id, is_hovered));
                            }
                        }
                    });
                    ui.add_space(gap);
                }
            });

        if let Some(idx) = select_index {
            let selected = self
                .search_results
                .as_ref()
                .map_or(&self.history_entries[idx], |res| &res[idx])
                .clone();
            self.select_capture(&selected);
        }
        if let Some(path) = drag_out_path
            && let Err(err) = crate::drag_out::start_file_drag(&path)
        {
            self.status_line = format!("Drag-and-drop failed: {err}");
        }

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
                ui.spacing_mut().item_spacing = Vec2::new(5.0_f32, 4.0_f32);
                for tag in &entry.tags {
                    egui::Frame::NONE
                        .fill(p.accent_subtle)
                        .stroke(Stroke::new(1.0_f32, p.border))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(7, 2))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 3.0_f32;
                                ui.label(
                                    RichText::new(format!("#{tag}"))
                                        .size(11.0_f32)
                                        .color(p.text_primary),
                                );
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("×").size(11.0_f32).color(p.text_muted),
                                        )
                                        .small()
                                        .frame(false),
                                    )
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
                        .desired_width(108.0_f32),
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
                    ui.add_space(64.0_f32);
                    card_frame(p)
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(Margin::symmetric(32, 28))
                        .show(ui, |ui| {
                            ui.set_max_width(400.0_f32);
                            ui.vertical_centered(|ui| {
                                paint_empty_viewfinder(ui, 48.0_f32, p);
                                ui.add_space(12.0_f32);
                                ui.label(
                                    RichText::new(s.no_image_in_editor)
                                        .size(17.0_f32)
                                        .strong()
                                        .color(p.text_primary),
                                );
                                ui.add_space(6.0_f32);
                                ui.label(
                                    RichText::new("Double-click any screenshot in Captures, or enable 'Open in image editor' before taking a screenshot.")
                                        .size(12.5_f32)
                                        .color(p.text_muted),
                                );
                            });
                        });
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
            view_heading(ui, s.hotkey_settings, p);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(primary_cta(s.apply_and_save, p)).clicked() {
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
        ui.add_space(4.0_f32);
        ui.label(
            RichText::new("Global hotkeys work even while ShareX is minimized or behind other applications. If Windows Snipping Tool owns PrintScreen, bind another key or disable 'Use the Print screen key to open screen capture' in Windows Accessibility settings.")
                .size(12.0_f32)
                .color(p.text_muted),
        );
        ui.add_space(6.0_f32);
        ui.separator();
        ui.add_space(6.0_f32);

        egui::ScrollArea::vertical()
            .id_salt("hotkeys_list_scroll")
            .show(ui, |ui| {
                for (idx, binding) in self.config.hotkeys.iter_mut().enumerate() {
                    let status = self.hotkeys.status_for(&binding.id);
                    let row_bg = if idx % 2 == 0 {
                        p.card_bg
                    } else {
                        p.surface_elevated
                    };
                    egui::Frame::NONE
                        .fill(row_bg)
                        .stroke(Stroke::new(1.0_f32, p.border))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut binding.enabled, "");
                                ui.add_sized(
                                    [166.0_f32, 22.0_f32],
                                    egui::Label::new(
                                        RichText::new(binding.action.label())
                                            .strong()
                                            .size(13.0_f32)
                                            .color(if binding.enabled {
                                                p.text_primary
                                            } else {
                                                p.text_muted
                                            }),
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
                                    .width(116.0_f32)
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

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let (badge_text, badge_color) = match status {
                                            HotkeyStatus::Active => {
                                                (s.status_active.to_owned(), p.success)
                                            }
                                            HotkeyStatus::Disabled => {
                                                (s.status_disabled.to_owned(), p.text_muted)
                                            }
                                            HotkeyStatus::Unbound => {
                                                (s.status_unbound.to_owned(), p.warning)
                                            }
                                            HotkeyStatus::Error(msg) => {
                                                (format!("Conflict: {msg}"), p.danger)
                                            }
                                        };
                                        ui.label(
                                            RichText::new(badge_text)
                                                .size(11.5_f32)
                                                .color(badge_color),
                                        );
                                        status_dot(ui, badge_color);
                                        if let Some(bound) = binding.combo {
                                            ui.add_space(6.0_f32);
                                            hotkey_badge(ui, &bound.to_string(), p);
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(5.0_f32);
                }
            });
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
        let mut save_clicked = false;

        ui.horizontal(|ui| {
            view_heading(ui, s.application_settings, p);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(primary_cta(s.save_settings, p)).clicked() {
                    save_clicked = true;
                }
            });
        });
        ui.add_space(6.0_f32);
        ui.separator();
        ui.add_space(8.0_f32);

        egui::ScrollArea::vertical()
            .id_salt("sharex_settings_scroll")
            .show(ui, |ui| {
                card_frame(p).show(ui, |ui| {
                    section_label(ui, "Capture & Storage", p);
                    ui.add_space(6.0_f32);
                    egui::Grid::new("sharex_settings_capture_grid")
                        .num_columns(2)
                        .spacing([18.0_f32, 10.0_f32])
                        .show(ui, |ui| {
                            ui.label(RichText::new(s.screenshots_folder).color(p.text_muted));
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.output_dir_input)
                                        .desired_width(360.0_f32),
                                );
                                if ui.button(s.open).clicked() {
                                    let target = PathBuf::from(self.output_dir_input.trim());
                                    let _ = fs::create_dir_all(&target);
                                    open_folder_in_os(&target);
                                }
                            });
                            ui.end_row();

                            ui.label(RichText::new(s.filename_pattern).color(p.text_muted));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.file_name_pattern)
                                    .desired_width(360.0_f32),
                            );
                            ui.end_row();

                            ui.label(RichText::new(s.pattern_preview).color(p.text_muted));
                            let preview =
                                match FileNamePattern::parse(&self.config.file_name_pattern) {
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
                            egui::Frame::NONE
                                .fill(p.sidebar_bg)
                                .stroke(Stroke::new(1.0_f32, p.divider))
                                .corner_radius(CornerRadius::same(5))
                                .inner_margin(Margin::symmetric(8, 3))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(preview)
                                            .monospace()
                                            .size(11.5_f32)
                                            .color(p.text_muted),
                                    );
                                });
                            ui.end_row();

                            ui.label(RichText::new(s.image_format).color(p.text_muted));
                            egui::ComboBox::from_id_salt("settings_format_picker")
                                .selected_text(
                                    self.config.image_format.extension().to_ascii_uppercase(),
                                )
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.config.image_format,
                                        ImageFormat::Png,
                                        "PNG",
                                    );
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
                        });
                });

                ui.add_space(10.0_f32);

                card_frame(p).show(ui, |ui| {
                    section_label(ui, "Desktop & Tray", p);
                    ui.add_space(6.0_f32);
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
                });

                ui.add_space(10.0_f32);

                card_frame(p).show(ui, |ui| {
                    section_label(ui, "Appearance & Language", p);
                    ui.add_space(6.0_f32);
                    egui::Grid::new("sharex_settings_appearance_grid")
                        .num_columns(2)
                        .spacing([18.0_f32, 10.0_f32])
                        .show(ui, |ui| {
                            ui.label(RichText::new(s.theme).color(p.text_muted));
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

                            ui.label(RichText::new(s.language).color(p.text_muted));
                            let mut chosen = self.language;
                            egui::ComboBox::from_id_salt("settings_lang_picker")
                                .selected_text(format!(
                                    "{} ({})",
                                    self.language.display_name(),
                                    self.language.short_code()
                                ))
                                .width(220.0_f32)
                                .show_ui(ui, |ui| {
                                    for lang in UiLanguage::ALL {
                                        let label = format!(
                                            "{} ({})",
                                            lang.display_name(),
                                            lang.short_code()
                                        );
                                        ui.selectable_value(&mut chosen, lang, label);
                                    }
                                });
                            if chosen != self.language {
                                self.set_language(ctx, chosen);
                            }
                            ui.end_row();
                        });
                });
            });

        if self.config.dark_theme != self.theme_applied_dark {
            apply_sharex_theme(ctx, self.config.dark_theme);
            self.theme_applied_dark = self.config.dark_theme;
        }

        if save_clicked {
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
            self.thumbnails.clear();
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        let pending_actions = self.hotkeys.poll_actions();
        for event in pending_actions {
            self.trigger_hotkey_event(ctx, event);
        }

        self.update_overlay(ctx);
        if self.overlay.is_some() {
            return;
        }
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
            .exact_height(28.0_f32)
            .frame(
                egui::Frame::NONE
                    .fill(p.sidebar_bg)
                    .inner_margin(Margin::symmetric(14, 0)),
            )
            .show(ctx, |ui| {
                let border_rect = ui.max_rect().expand2(Vec2::new(14.0_f32, 0.0_f32));
                ui.painter().with_clip_rect(border_rect).hline(
                    border_rect.x_range(),
                    border_rect.min.y + 0.5_f32,
                    Stroke::new(1.0_f32, p.border),
                );
                ui.centered_and_justified(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0_f32;
                        let active_hotkeys = self.hotkeys.active_count();
                        status_dot(
                            ui,
                            if active_hotkeys > 0 {
                                p.success
                            } else {
                                p.warning
                            },
                        );
                        ui.label(
                            RichText::new(format!(
                                "Hotkeys {active_hotkeys}/{}",
                                self.config.hotkeys.len()
                            ))
                            .size(11.5_f32)
                            .color(p.text_muted),
                        );

                        ui.add_space(6.0_f32);
                        let tray_on = self.tray.is_available();
                        status_dot(ui, if tray_on { p.success } else { p.text_muted });
                        ui.label(
                            RichText::new(if tray_on { "Tray" } else { "Tray off" })
                                .size(11.5_f32)
                                .color(p.text_muted),
                        );

                        if self.watcher.is_active() && self.watcher.watched_count() > 0 {
                            ui.add_space(6.0_f32);
                            status_dot(ui, p.success);
                            ui.label(
                                RichText::new(format!("Watchers {}", self.watcher.watched_count()))
                                    .size(11.5_f32)
                                    .color(p.text_muted),
                            );
                        }

                        if !self.pins.is_empty() {
                            ui.add_space(6.0_f32);
                            status_dot(ui, p.accent);
                            ui.label(
                                RichText::new(format!("Pins {}", self.pins.count()))
                                    .size(11.5_f32)
                                    .color(p.accent),
                            );
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&self.status_line)
                                        .size(11.5_f32)
                                        .color(p.text_muted),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
            });

        egui::SidePanel::left("sharex_sidebar")
            .resizable(false)
            .exact_width(252.0_f32)
            .frame(
                egui::Frame::NONE
                    .fill(p.sidebar_bg)
                    .inner_margin(Margin::symmetric(12, 12)),
            )
            .show(ctx, |ui| {
                let border_rect = ui.max_rect().expand2(Vec2::new(12.0_f32, 12.0_f32));
                ui.painter().with_clip_rect(border_rect).vline(
                    border_rect.max.x - 0.5_f32,
                    border_rect.y_range(),
                    Stroke::new(1.0_f32, p.border),
                );
                self.render_sidebar(ctx, ui, &p);
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(p.panel_bg)
                    .inner_margin(Margin::symmetric(20, 16)),
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

fn compact_hotkey_label(raw: &str) -> String {
    raw.replace("PrintScreen", "PrtSc")
        .replace("Control", "Ctrl")
        .replace(" + ", "+")
}

fn friendly_monitor_label(monitor: &MonitorInfo) -> String {
    let clean = monitor
        .name
        .strip_prefix(r"\\.\DISPLAY")
        .map(|num| format!("Display {num}"))
        .unwrap_or_else(|| monitor.name.clone());
    if monitor.is_primary {
        format!("{clean} · {}×{}", monitor.width, monitor.height)
    } else {
        format!("{clean} ({}×{})", monitor.width, monitor.height)
    }
}

fn sidebar_primary_capture_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    hotkey: &str,
    p: &SharexPalette,
) -> bool {
    let desired = Vec2::new(ui.available_width(), 36.0_f32);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    let bg = if resp.hovered() {
        p.accent
    } else {
        Color32::from_rgb(37, 99, 235)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(7), bg);
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.min.x + 17.0_f32, rect.center().y),
        Vec2::splat(14.0_f32),
    );
    paint_icon(painter, icon_rect, icon, Color32::WHITE);

    let short_key = compact_hotkey_label(hotkey);
    let key_galley = painter.layout_no_wrap(
        short_key,
        egui::FontId::monospace(9.5_f32),
        Color32::from_rgba_unmultiplied(255, 255, 255, 225),
    );
    let badge_w = key_galley.size().x + 10.0_f32;
    let badge_h = key_galley.size().y + 4.0_f32;
    let badge_rect = egui::Rect::from_min_size(
        egui::pos2(
            rect.max.x - badge_w - 8.0_f32,
            rect.center().y - badge_h * 0.5_f32,
        ),
        Vec2::new(badge_w, badge_h),
    );
    painter.rect_filled(
        badge_rect,
        CornerRadius::same(4),
        Color32::from_rgba_unmultiplied(0, 0, 0, 56),
    );
    painter.galley(
        egui::pos2(
            badge_rect.min.x + 5.0_f32,
            badge_rect.center().y - key_galley.size().y * 0.5_f32,
        ),
        key_galley,
        Color32::WHITE,
    );

    let text_x = rect.min.x + 31.0_f32;
    let max_label_w = (badge_rect.min.x - text_x - 8.0_f32).max(24.0_f32);
    let mut job = egui::text::LayoutJob::simple(
        label.trim_start_matches("+ ").to_owned(),
        egui::FontId::proportional(13.0_f32),
        Color32::WHITE,
        max_label_w,
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let label_galley = painter.layout_job(job);
    let clip = egui::Rect::from_min_max(
        egui::pos2(text_x, rect.min.y),
        egui::pos2(text_x + max_label_w, rect.max.y),
    );
    painter.with_clip_rect(clip).galley(
        egui::pos2(text_x, rect.center().y - label_galley.size().y * 0.5_f32),
        label_galley,
        Color32::WHITE,
    );
    resp.clicked()
}

fn sidebar_action_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    hotkey: &str,
    p: &SharexPalette,
) -> bool {
    let desired = Vec2::new(ui.available_width(), 29.0_f32);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), p.hover_bg);
    }
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.min.x + 15.0_f32, rect.center().y),
        Vec2::splat(13.5_f32),
    );
    paint_icon(
        painter,
        icon_rect,
        icon,
        if resp.hovered() {
            p.accent_hover
        } else {
            p.text_muted
        },
    );

    let short_key = compact_hotkey_label(hotkey);
    let galley = painter.layout_no_wrap(short_key, egui::FontId::monospace(9.5_f32), p.text_muted);
    let badge_w = galley.size().x + 8.0_f32;
    let badge_h = galley.size().y + 4.0_f32;
    let badge_rect = egui::Rect::from_min_size(
        egui::pos2(
            rect.max.x - badge_w - 6.0_f32,
            rect.center().y - badge_h * 0.5_f32,
        ),
        Vec2::new(badge_w, badge_h),
    );
    painter.rect_filled(badge_rect, CornerRadius::same(4), p.card_bg);
    painter.rect_stroke(
        badge_rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, p.border),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        egui::pos2(
            badge_rect.min.x + 4.0_f32,
            badge_rect.center().y - galley.size().y * 0.5_f32,
        ),
        galley,
        p.text_muted,
    );

    let text_x = rect.min.x + 29.0_f32;
    let max_label_w = (badge_rect.min.x - text_x - 8.0_f32).max(24.0_f32);
    let mut job = egui::text::LayoutJob::simple(
        label.to_owned(),
        egui::FontId::proportional(12.5_f32),
        p.text_primary,
        max_label_w,
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let label_galley = painter.layout_job(job);
    let clip = egui::Rect::from_min_max(
        egui::pos2(text_x, rect.min.y),
        egui::pos2(text_x + max_label_w, rect.max.y),
    );
    painter.with_clip_rect(clip).galley(
        egui::pos2(text_x, rect.center().y - label_galley.size().y * 0.5_f32),
        label_galley,
        p.text_primary,
    );
    resp.clicked()
}

fn sidebar_nav_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    active: bool,
    dot_color: Option<Color32>,
    p: &SharexPalette,
) -> bool {
    let desired = Vec2::new(ui.available_width(), 30.0_f32);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, CornerRadius::same(6), p.accent_subtle);
        let indicator = egui::Rect::from_min_size(
            egui::pos2(rect.min.x, rect.min.y + 5.0_f32),
            Vec2::new(3.0_f32, rect.height() - 10.0_f32),
        );
        painter.rect_filled(indicator, CornerRadius::same(2), p.accent);
    } else if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), p.hover_bg);
    }
    let text_color = if active || resp.hovered() {
        p.text_primary
    } else {
        p.text_muted
    };
    let icon_color = if active {
        p.accent
    } else if resp.hovered() {
        p.text_primary
    } else {
        p.text_muted
    };
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.min.x + 17.0_f32, rect.center().y),
        Vec2::splat(14.0_f32),
    );
    paint_icon(painter, icon_rect, icon, icon_color);
    let font = egui::FontId::proportional(13.0_f32);
    let galley = painter.layout_no_wrap(label.to_owned(), font, text_color);
    let text_pos = egui::pos2(
        rect.min.x + 31.0_f32,
        rect.center().y - galley.size().y * 0.5_f32,
    );
    if active {
        painter.galley(
            egui::pos2(text_pos.x + 0.35_f32, text_pos.y),
            galley.clone(),
            text_color,
        );
    }
    painter.galley(text_pos, galley, text_color);

    if let Some(color) = dot_color {
        painter.circle_filled(
            egui::pos2(rect.max.x - 10.0_f32, rect.center().y),
            3.5_f32,
            color,
        );
    }
    resp.clicked()
}

fn paint_empty_viewfinder(ui: &mut egui::Ui, size: f32, p: &SharexPalette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(12), p.surface_elevated);
    painter.rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0_f32, p.border),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink(size * 0.24_f32);
    let arm = size * 0.16_f32;
    let stroke = Stroke::new(1.8_f32, p.accent);
    painter.line_segment(
        [inner.left_top(), egui::pos2(inner.min.x + arm, inner.min.y)],
        stroke,
    );
    painter.line_segment(
        [inner.left_top(), egui::pos2(inner.min.x, inner.min.y + arm)],
        stroke,
    );
    painter.line_segment(
        [
            inner.right_top(),
            egui::pos2(inner.max.x - arm, inner.min.y),
        ],
        stroke,
    );
    painter.line_segment(
        [
            inner.right_top(),
            egui::pos2(inner.max.x, inner.min.y + arm),
        ],
        stroke,
    );
    painter.line_segment(
        [
            inner.left_bottom(),
            egui::pos2(inner.min.x + arm, inner.max.y),
        ],
        stroke,
    );
    painter.line_segment(
        [
            inner.left_bottom(),
            egui::pos2(inner.min.x, inner.max.y - arm),
        ],
        stroke,
    );
    painter.line_segment(
        [
            inner.right_bottom(),
            egui::pos2(inner.max.x - arm, inner.max.y),
        ],
        stroke,
    );
    painter.line_segment(
        [
            inner.right_bottom(),
            egui::pos2(inner.max.x, inner.max.y - arm),
        ],
        stroke,
    );
    painter.circle_stroke(
        rect.center(),
        size * 0.08_f32,
        Stroke::new(1.5_f32, p.text_muted),
    );
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

    #[test]
    fn compact_hotkey_and_monitor_labels_stay_short() {
        assert_eq!(compact_hotkey_label("Ctrl + PrintScreen"), "Ctrl+PrtSc");
        assert_eq!(compact_hotkey_label("Shift + PrintScreen"), "Shift+PrtSc");
        let monitor = MonitorInfo {
            id: sharex_core::MonitorId::new(1),
            name: r"\\.\DISPLAY2".into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            is_primary: true,
        };
        assert_eq!(friendly_monitor_label(&monitor), "Display 2 · 1920×1080");
    }
}
