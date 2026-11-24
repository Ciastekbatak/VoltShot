use crate::i18n::Strings;
use crate::theme::SharexPalette;
use chrono::Utc;
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke};
use sharex_core::{ClipboardSink, resolve_unique_path};
use sharex_media::{RecordingConfig, RecordingFormat, RecordingState, ScreenRecorder, find_ffmpeg};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Screen & GIF recording workspace panel state backed by `sharex_media::ScreenRecorder`.
pub struct RecordingPanelState {
    recorder: ScreenRecorder,
    format: RecordingFormat,
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
            format: RecordingFormat::Mp4H264,
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

    pub fn build_config(&self, output_dir: &Path) -> RecordingConfig {
        let stem = format!("ShareX_Rec_{}", Utc::now().format("%Y-%m-%d_%H-%M-%S"));
        let output_path = resolve_unique_path(output_dir, &stem, self.format.extension());
        let mut config = RecordingConfig::new(output_path);
        config.format = self.format;
        config.fps = self.fps;
        config.crf = self.crf;
        config.draw_cursor = self.draw_cursor;
        config.ffmpeg_path = self.ffmpeg_path.clone();
        config
    }

    /// Stops the active recording or starts a recording in the requested format.
    pub fn toggle_hotkey_recording(
        &mut self,
        format: RecordingFormat,
        output_dir: &Path,
    ) -> String {
        let message = if self.is_recording() {
            self.stop_recording()
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
    }

    fn start_recording(&mut self, output_dir: &Path) -> String {
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
            Ok(()) => format!(
                "Recording {} at {} FPS to {}",
                self.format.extension().to_ascii_uppercase(),
                self.fps,
                config.output_path.display()
            ),
            Err(err) => format!("Failed to start recording: {err}"),
        }
    }

    fn stop_recording(&mut self) -> String {
        match self.recorder.stop() {
            Ok((path, duration)) => format!(
                "Saved recording ({}) to {}",
                format_elapsed(duration),
                path.display()
            ),
            Err(err) => format!("Recording stop failed: {err}"),
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        p: &SharexPalette,
        s: &Strings,
        output_dir: &Path,
        clipboard: &dyn ClipboardSink,
    ) -> Option<String> {
        let mut status_msg = None;

        if !self.ffmpeg_checked {
            self.check_ffmpeg();
        }

        let is_recording = self.recorder.state().is_recording();
        if is_recording {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        ui.horizontal(|ui| {
            ui.heading(s.recording);
            ui.add_space(10.0);
            if is_recording {
                let elapsed = self.recorder.elapsed().unwrap_or_default();
                ui.label(
                    RichText::new(format!("REC {}", format_elapsed(elapsed)))
                        .strong()
                        .color(p.danger),
                );
            }
        });
        ui.separator();

        egui::Frame::NONE
            .fill(p.card_bg)
            .stroke(Stroke::new(1.0_f32, p.border))
            .corner_radius(CornerRadius::same(5))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("FFmpeg:").strong());
                    if let Some(path) = &self.ffmpeg_path {
                        ui.label(
                            RichText::new(format!("Available ({})", path.display()))
                                .color(p.success),
                        );
                    } else {
                        ui.label(
                            RichText::new(
                                "Not found in PATH (install FFmpeg to record MP4/WebM/GIF)",
                            )
                            .color(p.warning),
                        );
                    }
                    if ui.small_button("Re-check").clicked() {
                        self.check_ffmpeg();
                    }
                });
            });

        ui.add_space(10.0);
        ui.add_enabled_ui(!is_recording, |ui| {
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
            let can_start = !is_recording && self.ffmpeg_path.is_some();
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
            if ui.add_enabled(is_recording, stop_btn).clicked() {
                let message = self.stop_recording();
                self.last_message = Some(message.clone());
                status_msg = Some(message);
            }

            let abort_btn =
                egui::Button::new(RichText::new("Abort").color(Color32::WHITE)).fill(p.danger);
            if ui.add_enabled(is_recording, abort_btn).clicked() {
                match self.recorder.abort() {
                    Ok(_) => {
                        let msg = "Recording aborted and partial file discarded.".to_owned();
                        self.last_message = Some(msg.clone());
                        status_msg = Some(msg);
                    }
                    Err(err) => {
                        let msg = format!("Abort failed: {err}");
                        self.last_message = Some(msg.clone());
                        status_msg = Some(msg);
                    }
                }
            }
        });

        ui.add_space(12.0);
        egui::Frame::NONE
            .fill(p.card_bg)
            .stroke(Stroke::new(1.0_f32, p.border))
            .corner_radius(CornerRadius::same(5))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| match self.recorder.state() {
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

pub fn format_elapsed(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    let tenths = duration.subsec_millis() / 100;
    format!("{mins:02}:{secs:02}.{tenths}")
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
