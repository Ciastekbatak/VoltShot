use crate::i18n::Strings;
use crate::recording_hud::{RecordingHudAction, show_recording_hud};
use crate::theme::{VoltShotPalette, card_frame, status_dot, view_heading};
use chrono::Utc;
use eframe::egui::{self, Color32, RichText};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;
use voltshot_core::{ClipboardSink, Region, resolve_unique_path};
use voltshot_media::{
    AudioDevice, CapturePlatform, MediaError, RecordingConfig, RecordingFormat, RecordingState,
    ScreenRecorder, VideoEncoder, detect_available_h264_encoders, find_ffmpeg,
    hardware_encoder_supported_for_region, list_audio_devices, preferred_mp4_encoder,
};

pub use crate::recording_hud::format_elapsed;

type FinalizeResult = (ScreenRecorder, Result<(PathBuf, Duration), MediaError>);

/// Screen & GIF recording workspace panel state backed by `voltshot_media::ScreenRecorder`.
pub struct RecordingPanelState {
    recorder: ScreenRecorder,
    finalizing: Option<Receiver<FinalizeResult>>,
    active_config: Option<RecordingConfig>,
    format: RecordingFormat,
    mp4_encoder: VideoEncoder,
    available_encoders: Vec<VideoEncoder>,
    audio_device: Option<AudioDevice>,
    audio_devices: Vec<AudioDevice>,
    region: Option<Region>,
    select_region_requested: bool,
    fps: u32,
    crf: u8,
    draw_cursor: bool,
    ffmpeg_path: Option<PathBuf>,
    ffmpeg_checked: bool,
    last_message: Option<String>,
}

impl Default for RecordingPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingPanelState {
    pub fn new() -> Self {
        Self {
            recorder: ScreenRecorder::new(),
            finalizing: None,
            active_config: None,
            format: RecordingFormat::Mp4H264,
            mp4_encoder: VideoEncoder::Auto,
            available_encoders: Vec::new(),
            audio_device: None,
            audio_devices: Vec::new(),
            region: None,
            select_region_requested: false,
            fps: 30,
            crf: 23,
            draw_cursor: true,
            ffmpeg_path: None,
            ffmpeg_checked: false,
            last_message: None,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.recorder.state().is_recording()
    }

    pub fn is_finalizing(&self) -> bool {
        self.finalizing.is_some()
    }

    /// Sets the region for the next recording, or selects fullscreen with `None`.
    pub fn set_region(&mut self, region: Option<Region>) {
        self.region = region;
    }

    /// Returns the region selected for the next recording.
    pub fn selected_region(&self) -> Option<Region> {
        self.region
    }

    /// Consumes a pending request to open the recording-region selector.
    pub fn take_select_region_request(&mut self) -> bool {
        std::mem::take(&mut self.select_region_requested)
    }

    pub fn poll_finalize(&mut self) -> Option<String> {
        let receiver = self.finalizing.as_ref()?;
        match receiver.try_recv() {
            Ok((recorder, result)) => {
                self.finalizing = None;
                self.recorder = recorder;
                let msg = match result {
                    Ok((path, duration)) => format!(
                        "Saved recording ({}) to {}",
                        format_elapsed(duration),
                        path.display()
                    ),
                    Err(err) => format!("Recording stop failed: {err}"),
                };
                self.last_message = Some(msg.clone());
                Some(msg)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.finalizing = None;
                let msg = "Recording finalization worker terminated unexpectedly.".to_owned();
                self.last_message = Some(msg.clone());
                Some(msg)
            }
        }
    }

    pub fn build_config(&self, output_dir: &Path) -> RecordingConfig {
        let stem = format!("VoltShot_Rec_{}", Utc::now().format("%Y-%m-%d_%H-%M-%S"));
        let output_path = resolve_unique_path(output_dir, &stem, self.format.extension());
        let mut config = RecordingConfig::new(output_path);
        config.format = self.format;
        config.region = self.selected_region();
        config.encoder = match self.format {
            RecordingFormat::Mp4H264 if self.mp4_encoder == VideoEncoder::Auto => {
                let encoder = self.auto_mp4_encoder();
                config.fallback_to_libx264_on_hw_fail = encoder != VideoEncoder::Libx264;
                encoder
            }
            RecordingFormat::Mp4H264 => self.mp4_encoder,
            RecordingFormat::WebmVp9 => VideoEncoder::LibvpxVp9,
            RecordingFormat::Gif => VideoEncoder::GifPalette,
        };
        if self.format != RecordingFormat::Gif {
            config.audio_device = self.audio_device.as_ref().map(|device| device.name.clone());
        }
        config.fps = self.fps;
        config.crf = self.crf;
        config.draw_cursor = self.draw_cursor;
        config.ffmpeg_path = self.ffmpeg_path.clone();
        config
    }

    fn auto_mp4_encoder(&self) -> VideoEncoder {
        if hardware_encoder_supported_for_region(self.selected_region()) {
            preferred_mp4_encoder(&self.available_encoders)
        } else {
            VideoEncoder::Libx264
        }
    }

    /// Stops the active recording (off the UI thread) or starts a recording in the requested format.
    pub fn toggle_hotkey_recording(
        &mut self,
        format: RecordingFormat,
        output_dir: &Path,
    ) -> String {
        let message = if self.is_finalizing() {
            "Recording is already finalizing...".to_owned()
        } else if self.is_recording() {
            self.stop_recording_async()
        } else {
            if !self.ffmpeg_checked {
                self.check_ffmpeg();
            }
            self.format = format;
            self.start_recording(output_dir)
        };
        self.last_message = Some(message.clone());
        message
    }

    fn check_ffmpeg(&mut self) {
        self.ffmpeg_path = find_ffmpeg(None).ok();
        self.ffmpeg_checked = true;
        if let Some(path) = &self.ffmpeg_path {
            self.available_encoders = detect_available_h264_encoders(path);
            self.audio_devices = list_audio_devices(path, CapturePlatform::current());
        } else {
            self.available_encoders.clear();
            self.audio_devices.clear();
        }
        if self
            .audio_device
            .as_ref()
            .is_some_and(|device| !self.audio_devices.contains(device))
        {
            self.audio_device = None;
        }
        if !matches!(self.mp4_encoder, VideoEncoder::Auto | VideoEncoder::Libx264)
            && !self.available_encoders.contains(&self.mp4_encoder)
        {
            self.mp4_encoder = VideoEncoder::Auto;
        }
    }

    fn start_recording(&mut self, output_dir: &Path) -> String {
        if self.is_finalizing() {
            return "Wait for the previous recording to finish finalizing.".into();
        }
        if self.ffmpeg_path.is_none() {
            return "FFmpeg was not found. Install FFmpeg, then select Re-check.".into();
        }
        if !matches!(self.recorder.state(), RecordingState::Idle)
            && let Err(err) = self.recorder.reset()
        {
            return format!("Failed to reset recorder: {err}");
        }
        let config = self.build_config(output_dir);
        match self.recorder.start(&config) {
            Ok(()) => {
                let message = format!(
                    "Recording {} at {} FPS to {}",
                    self.format.extension().to_ascii_uppercase(),
                    self.fps,
                    config.output_path.display()
                );
                self.active_config = Some(config);
                message
            }
            Err(err) => format!("Failed to start recording: {err}"),
        }
    }

    fn stop_recording_async(&mut self) -> String {
        if !self.is_recording() {
            return "No active recording to stop.".into();
        }
        self.active_config = None;
        let mut recorder = std::mem::take(&mut self.recorder);
        let (tx, rx) = mpsc::channel();
        match thread::Builder::new()
            .name("voltshot-record-finalize".into())
            .spawn(move || {
                let result = recorder.stop();
                let _ = tx.send((recorder, result));
            }) {
            Ok(_) => {
                self.finalizing = Some(rx);
                "Stopping and finalizing recording...".into()
            }
            Err(err) => format!("Failed to spawn finalizer thread: {err}"),
        }
    }

    fn abort_recording(&mut self) -> String {
        self.active_config = None;
        match self.recorder.abort() {
            Ok(_) => "Recording aborted and partial file discarded.".into(),
            Err(err) => format!("Abort failed: {err}"),
        }
    }

    /// Renders the active recording's floating controls and reports a stop or abort message.
    pub fn show_hud(&mut self, ctx: &egui::Context) -> Option<String> {
        let elapsed = self.recorder.elapsed()?;
        let config = self.active_config.as_ref()?;
        let format = match config.format {
            RecordingFormat::Mp4H264 => "MP4",
            RecordingFormat::WebmVp9 => "WebM",
            RecordingFormat::Gif => "GIF",
        };
        let message = match show_recording_hud(
            ctx,
            config.region,
            elapsed,
            format,
            encoder_badge(self.recorder.active_encoder()?),
        ) {
            RecordingHudAction::None => return None,
            RecordingHudAction::Stop => self.stop_recording_async(),
            RecordingHudAction::Abort => self.abort_recording(),
        };
        self.last_message = Some(message.clone());
        ctx.request_repaint();
        Some(message)
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        s: &Strings,
        output_dir: &Path,
        clipboard: &dyn ClipboardSink,
    ) -> Option<String> {
        let mut status_msg = self.poll_finalize();

        if !self.ffmpeg_checked {
            self.check_ffmpeg();
        }

        let is_recording = self.recorder.state().is_recording();
        let is_finalizing = self.is_finalizing();
        if is_recording || is_finalizing {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        ui.horizontal(|ui| {
            view_heading(ui, s.recording, p);
            ui.add_space(10.0_f32);
            if is_recording {
                let elapsed = self.recorder.elapsed().unwrap_or_default();
                status_dot(ui, p.danger);
                ui.label(
                    RichText::new(format!("REC {}", format_elapsed(elapsed)))
                        .strong()
                        .color(p.danger),
                );
            } else if is_finalizing {
                ui.spinner();
                ui.label(RichText::new("Finalizing...").strong().color(p.warning));
            }
        });
        ui.add_space(6.0_f32);
        ui.separator();
        ui.add_space(6.0_f32);

        card_frame(p).show(ui, |ui| {
            ui.horizontal(|ui| {
                status_dot(
                    ui,
                    if self.ffmpeg_path.is_some() {
                        p.success
                    } else {
                        p.warning
                    },
                );
                ui.label(RichText::new("FFmpeg:").strong());
                if let Some(path) = &self.ffmpeg_path {
                    ui.label(
                        RichText::new(format!("Available ({})", path.display())).color(p.success),
                    );
                } else {
                    ui.label(
                        RichText::new("Not found in PATH (install FFmpeg to record MP4/WebM/GIF)")
                            .color(p.warning),
                    );
                }
                if ui
                    .add_enabled(
                        !is_recording && !is_finalizing,
                        egui::Button::new("Re-check").small(),
                    )
                    .clicked()
                {
                    self.check_ffmpeg();
                }
            });
        });

        ui.add_space(10.0);
        ui.add_enabled_ui(!is_recording && !is_finalizing, |ui| {
            egui::Grid::new("recording_settings_grid")
                .num_columns(2)
                .spacing([14.0, 10.0])
                .show(ui, |ui| {
                    ui.label("Format");
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.format,
                            RecordingFormat::Mp4H264,
                            "MP4 (H.264)",
                        );
                        ui.selectable_value(
                            &mut self.format,
                            RecordingFormat::WebmVp9,
                            "WebM (VP9)",
                        );
                        ui.selectable_value(&mut self.format, RecordingFormat::Gif, "GIF");
                    });
                    ui.end_row();

                    if self.format == RecordingFormat::Mp4H264 {
                        ui.label("Encoder");
                        let label = if self.mp4_encoder == VideoEncoder::Auto {
                            format!("Auto ({})", encoder_badge(self.auto_mp4_encoder()))
                        } else {
                            encoder_option(self.mp4_encoder).to_owned()
                        };
                        egui::ComboBox::from_id_salt("recording_encoder")
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.mp4_encoder,
                                    VideoEncoder::Auto,
                                    "Auto",
                                );
                                for &encoder in &self.available_encoders {
                                    if encoder != VideoEncoder::Libx264 {
                                        ui.selectable_value(
                                            &mut self.mp4_encoder,
                                            encoder,
                                            encoder_option(encoder),
                                        );
                                    }
                                }
                                ui.selectable_value(
                                    &mut self.mp4_encoder,
                                    VideoEncoder::Libx264,
                                    encoder_option(VideoEncoder::Libx264),
                                );
                            });
                        ui.end_row();
                    }

                    if self.format != RecordingFormat::Gif {
                        ui.label("Audio input");
                        let label = self
                            .audio_device
                            .as_ref()
                            .map_or("None (Silent)", |device| &device.name);
                        egui::ComboBox::from_id_salt("recording_audio_device")
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.audio_device, None, "None (Silent)");
                                for device in &self.audio_devices {
                                    if ui
                                        .selectable_label(
                                            self.audio_device.as_ref() == Some(device),
                                            &device.name,
                                        )
                                        .clicked()
                                    {
                                        self.audio_device = Some(device.clone());
                                    }
                                }
                            });
                        ui.end_row();
                    }

                    ui.label("Capture region");
                    ui.horizontal(|ui| {
                        if ui.button("Select Region...").clicked() {
                            self.select_region_requested = true;
                        }
                        if let Some(region) = self.selected_region() {
                            ui.label(format!("{} × {}", region.width(), region.height()));
                            if ui.button("Clear Region (Fullscreen)").clicked() {
                                self.set_region(None);
                            }
                        } else {
                            ui.label("Fullscreen");
                        }
                    });
                    ui.end_row();

                    ui.label("Frame rate (FPS)");
                    ui.horizontal(|ui| {
                        for fps in [15_u32, 30_u32, 60_u32] {
                            ui.selectable_value(&mut self.fps, fps, format!("{fps} FPS"));
                        }
                    });
                    ui.end_row();

                    if self.format != RecordingFormat::Gif {
                        ui.label("Quality (CRF)");
                        ui.add(egui::Slider::new(&mut self.crf, 15..=35));
                        ui.end_row();
                    }

                    ui.label("Cursor");
                    ui.checkbox(&mut self.draw_cursor, "Include mouse cursor");
                    ui.end_row();
                });
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let can_start = !is_recording && !is_finalizing && self.ffmpeg_path.is_some();
            let start_btn =
                egui::Button::new(RichText::new("Start Recording").color(Color32::WHITE))
                    .fill(p.accent);
            if ui.add_enabled(can_start, start_btn).clicked() {
                let message = self.start_recording(output_dir);
                self.last_message = Some(message.clone());
                status_msg = Some(message);
            }

            let stop_btn = egui::Button::new(RichText::new("Stop & Save").color(Color32::WHITE))
                .fill(p.success);
            if ui
                .add_enabled(is_recording && !is_finalizing, stop_btn)
                .clicked()
            {
                let message = self.stop_recording_async();
                self.last_message = Some(message.clone());
                status_msg = Some(message);
            }

            let abort_btn =
                egui::Button::new(RichText::new("Abort").color(Color32::WHITE)).fill(p.danger);
            if ui
                .add_enabled(is_recording && !is_finalizing, abort_btn)
                .clicked()
            {
                let message = self.abort_recording();
                self.last_message = Some(message.clone());
                status_msg = Some(message);
            }
        });

        ui.add_space(12.0_f32);
        card_frame(p).show(ui, |ui| match self.recorder.state() {
            RecordingState::Idle => {
                ui.label(RichText::new("Status: Idle — ready to record.").color(p.text_muted));
            }
            RecordingState::Recording { output_path, .. } => {
                let elapsed = self.recorder.elapsed().unwrap_or_default();
                ui.label(
                    RichText::new(format!(
                        "Status: Recording ({} / {:.1}s)",
                        format_elapsed(elapsed),
                        elapsed.as_secs_f32()
                    ))
                    .strong()
                    .color(p.danger),
                );
                ui.label(
                    RichText::new(format!("Output: {}", output_path.display()))
                        .size(12.0)
                        .color(p.text_muted),
                );
            }
            RecordingState::Completed {
                output_path,
                duration,
            } => {
                let path_text = output_path.display().to_string();
                ui.label(
                    RichText::new(format!("Status: Completed ({})", format_elapsed(*duration)))
                        .strong()
                        .color(p.success),
                );
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&path_text).monospace().size(12.0));
                    if ui.small_button("Copy Path").clicked() {
                        status_msg = Some(match clipboard.copy_text(&path_text) {
                            Ok(()) => format!("Copied recording path: {path_text}"),
                            Err(err) => format!("Clipboard error: {err}"),
                        });
                    }
                });
            }
            RecordingState::Failed(reason) => {
                ui.label(
                    RichText::new(format!("Status: Failed — {reason}"))
                        .strong()
                        .color(p.danger),
                );
            }
        });

        if let Some(msg) = &self.last_message {
            ui.add_space(6.0);
            ui.label(RichText::new(msg).size(12.0).color(p.text_muted));
        }

        status_msg
    }
}

fn encoder_badge(encoder: VideoEncoder) -> &'static str {
    match encoder {
        VideoEncoder::NvencH264 => "NVENC",
        VideoEncoder::QsvH264 => "QSV",
        VideoEncoder::AmfH264 => "AMF",
        VideoEncoder::Libx264 => "libx264",
        VideoEncoder::LibvpxVp9 => "VP9",
        VideoEncoder::Auto | VideoEncoder::GifPalette => "",
    }
}

fn encoder_option(encoder: VideoEncoder) -> &'static str {
    match encoder {
        VideoEncoder::NvencH264 => "NVENC (NVIDIA)",
        VideoEncoder::QsvH264 => "QSV (Intel)",
        VideoEncoder::AmfH264 => "AMF (AMD)",
        VideoEncoder::Libx264 => "CPU (libx264)",
        _ => encoder_badge(encoder),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn recording_panel_builds_valid_configs_for_all_formats() {
        let dir = tempdir().expect("tempdir");
        let mut state = RecordingPanelState::new();
        assert!(!state.is_recording());

        for (format, fps, ext) in [
            (RecordingFormat::Mp4H264, 30, "mp4"),
            (RecordingFormat::WebmVp9, 60, "webm"),
            (RecordingFormat::Gif, 15, "gif"),
        ] {
            state.format = format;
            state.fps = fps;
            let cfg = state.build_config(dir.path());
            assert!(cfg.validate().is_ok());
            assert_eq!(cfg.fps, fps);
            assert_eq!(
                cfg.output_path.extension().and_then(|e| e.to_str()),
                Some(ext)
            );
        }
    }

    #[test]
    fn build_config_applies_region_encoder_and_audio_but_keeps_gif_silent() {
        let dir = tempdir().expect("tempdir");
        let mut state = RecordingPanelState::new();
        let region = Region::new(-640, 20, 640, 480).expect("valid region");
        state.set_region(Some(region));
        state.mp4_encoder = VideoEncoder::NvencH264;
        state.audio_device = Some(AudioDevice {
            name: "USB Microphone".into(),
            backend: "dshow",
        });

        let mp4 = state.build_config(dir.path());
        assert_eq!(mp4.region, state.selected_region());
        assert_eq!(mp4.region, Some(region));
        assert_eq!(mp4.encoder, VideoEncoder::NvencH264);
        assert_eq!(mp4.audio_device.as_deref(), Some("USB Microphone"));
        assert!(mp4.validate().is_ok());

        state.format = RecordingFormat::Gif;
        let gif = state.build_config(dir.path());
        assert_eq!(gif.region, Some(region));
        assert_eq!(gif.encoder, VideoEncoder::GifPalette);
        assert_eq!(gif.audio_device, None);
        assert!(gif.validate().is_ok());

        state.format = RecordingFormat::WebmVp9;
        state.set_region(None);
        let webm = state.build_config(dir.path());
        assert_eq!(webm.region, None);
        assert_eq!(webm.encoder, VideoEncoder::LibvpxVp9);
        assert_eq!(webm.audio_device, mp4.audio_device);
        assert!(webm.validate().is_ok());
    }

    #[test]
    fn build_config_resolves_auto_using_the_detected_encoders() {
        let dir = tempdir().expect("tempdir");
        let mut state = RecordingPanelState::new();
        assert_eq!(
            state.build_config(dir.path()).encoder,
            VideoEncoder::Libx264
        );
        state.available_encoders = vec![VideoEncoder::AmfH264, VideoEncoder::QsvH264];
        let fullscreen = state.build_config(dir.path());
        assert_eq!(fullscreen.encoder, VideoEncoder::QsvH264);
        assert!(fullscreen.fallback_to_libx264_on_hw_fail);

        state.available_encoders = vec![VideoEncoder::NvencH264];
        state.set_region(Some(Region::new(0, 0, 64, 64).expect("valid region")));
        let small_region = state.build_config(dir.path());
        assert_eq!(small_region.encoder, VideoEncoder::Libx264);
        assert!(!small_region.fallback_to_libx264_on_hw_fail);
    }

    #[test]
    fn select_region_request_is_consumed_once() {
        let mut state = RecordingPanelState::new();
        assert!(!state.take_select_region_request());
        state.select_region_requested = true;
        assert!(state.take_select_region_request());
        assert!(!state.take_select_region_request());
    }

    #[test]
    fn hotkey_reports_missing_ffmpeg_without_starting_recording() {
        let dir = tempdir().expect("tempdir");
        let mut state = RecordingPanelState::new();
        state.ffmpeg_checked = true;
        let status = state.toggle_hotkey_recording(RecordingFormat::Gif, dir.path());

        assert!(status.contains("FFmpeg was not found"));
        assert!(!state.is_recording());
        assert_eq!(state.format, RecordingFormat::Gif);
        assert_eq!(state.last_message.as_deref(), Some(status.as_str()));
    }

    #[test]
    fn format_elapsed_renders_minutes_seconds_and_tenths() {
        assert_eq!(format_elapsed(Duration::from_millis(0)), "00:00.0");
        assert_eq!(format_elapsed(Duration::from_millis(65_400)), "01:05.4");
    }
}
