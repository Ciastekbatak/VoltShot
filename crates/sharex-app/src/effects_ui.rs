use crate::i18n::Strings;
use crate::theme::SharexPalette;
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use sharex_core::CapturedImage;
use sharex_effects::{EffectPreset, ImageEffect, apply_effects, builtin_presets};
use std::path::{Path, PathBuf};

/// Interactive Image Effects preset selector and live preview panel state.
pub struct EffectsPanelState {
    presets: Vec<EffectPreset>,
    selected_preset_idx: Option<usize>,
    pub apply_on_capture: bool,
    use_latest_capture: bool,
    preview_texture: Option<egui::TextureHandle>,
    preview_dims: Option<(u32, u32)>,
    preview_key: Option<(Option<usize>, Option<PathBuf>)>,
    preview_error: Option<String>,
}

impl Default for EffectsPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectsPanelState {
    pub fn new() -> Self {
        Self {
            presets: builtin_presets(),
            selected_preset_idx: Some(0),
            apply_on_capture: false,
            use_latest_capture: false,
            preview_texture: None,
            preview_dims: None,
            preview_key: None,
            preview_error: None,
        }
    }

    pub fn presets(&self) -> &[EffectPreset] {
        &self.presets
    }

    pub fn selected_preset_idx(&self) -> Option<usize> {
        self.selected_preset_idx
    }

    pub fn set_selected_preset_idx(&mut self, idx: Option<usize>) {
        self.selected_preset_idx = idx.filter(|&i| i < self.presets.len());
    }

    pub fn active_preset(&self) -> Option<&EffectPreset> {
        self.selected_preset_idx
            .and_then(|idx| self.presets.get(idx))
    }

    pub fn active_preset_name(&self) -> &str {
        self.active_preset()
            .map(|preset| preset.name.as_str())
            .unwrap_or("None")
    }

    /// Applies the active preset via `sharex_effects::apply_effects` when enabled.
    pub fn apply_if_enabled(&self, image: &CapturedImage) -> Result<CapturedImage, String> {
        if !self.apply_on_capture {
            return Ok(image.clone());
        }
        let Some(preset) = self.active_preset() else {
            return Ok(image.clone());
        };
        apply_effects(image, &preset.effects).map_err(|err| err.to_string())
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        p: &SharexPalette,
        s: &Strings,
        latest_capture: Option<&Path>,
    ) -> Option<String> {
        let mut status_msg = None;

        ui.horizontal(|ui| {
            ui.heading(s.effects);
            ui.add_space(12.0);
            if ui
                .checkbox(
                    &mut self.apply_on_capture,
                    "Apply selected preset after capture",
                )
                .changed()
            {
                status_msg = Some(if self.apply_on_capture {
                    format!("Image effects enabled ({}).", self.active_preset_name())
                } else {
                    "Image effects on capture disabled.".to_owned()
                });
            }
        });
        ui.separator();

        ui.columns(2, |columns| {
            columns[0].vertical(|ui| {
                ui.label(
                    RichText::new("PRESETS")
                        .size(11.0)
                        .strong()
                        .color(p.text_muted),
                );
                ui.add_space(4.0);

                let none_selected = self.selected_preset_idx.is_none();
                if preset_row(ui, "None", "Original unmodified image", none_selected, p) {
                    self.selected_preset_idx = None;
                }
                ui.add_space(4.0);

                let preset_summaries: Vec<(String, String)> = self
                    .presets
                    .iter()
                    .map(|preset| (preset.name.clone(), summarize_preset(preset)))
                    .collect();

                for (idx, (name, summary)) in preset_summaries.iter().enumerate() {
                    let is_selected = self.selected_preset_idx == Some(idx);
                    if preset_row(ui, name, summary, is_selected, p) {
                        self.selected_preset_idx = Some(idx);
                    }
                    ui.add_space(4.0);
                }

                ui.add_space(10.0);
                ui.label(
                    RichText::new("EFFECT PIPELINE")
                        .size(11.0)
                        .strong()
                        .color(p.text_muted),
                );
                ui.add_space(4.0);

                egui::Frame::NONE
                    .fill(p.card_bg)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .corner_radius(CornerRadius::same(5))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        if let Some(preset) = self.active_preset() {
                            for (step, effect) in preset.effects.iter().enumerate() {
                                ui.label(
                                    RichText::new(format!(
                                        "{}. {}",
                                        step + 1,
                                        describe_effect(effect)
                                    ))
                                    .size(12.5)
                                    .color(p.text_primary),
                                );
                            }
                        } else {
                            ui.label(
                                RichText::new("No effects will be applied.")
                                    .size(12.5)
                                    .color(p.text_muted),
                            );
                        }
                    });
            });

            columns[1].vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("LIVE PREVIEW")
                            .size(11.0)
                            .strong()
                            .color(p.text_muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let has_latest = latest_capture.is_some_and(Path::exists);
                        if ui
                            .add_enabled(
                                has_latest,
                                egui::SelectableLabel::new(
                                    self.use_latest_capture && has_latest,
                                    "Latest capture",
                                ),
                            )
                            .clicked()
                        {
                            self.use_latest_capture = true;
                        }
                        if ui
                            .selectable_label(!self.use_latest_capture || !has_latest, "Sample")
                            .clicked()
                        {
                            self.use_latest_capture = false;
                        }
                    });
                });
                ui.add_space(4.0);

                let source_path = if self.use_latest_capture {
                    latest_capture
                        .filter(|path| path.exists())
                        .map(Path::to_path_buf)
                } else {
                    None
                };

                self.ensure_preview_texture(ctx, source_path);

                egui::Frame::NONE
                    .fill(p.card_bg)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        if let Some(err) = &self.preview_error {
                            ui.label(RichText::new(err).color(p.danger));
                        } else if let Some(tex) = &self.preview_texture {
                            let max_w = ui.available_width().clamp(160.0, 460.0);
                            let max_h = 300.0_f32;
                            let tex_size = tex.size_vec2();
                            let scale = (max_w / tex_size.x.max(1.0))
                                .min(max_h / tex_size.y.max(1.0))
                                .min(1.0);
                            let display_size = tex_size * scale;
                            ui.vertical_centered(|ui| {
                                ui.add(
                                    egui::Image::new((tex.id(), display_size))
                                        .corner_radius(CornerRadius::same(4)),
                                );
                                ui.add_space(6.0);
                                if let Some((w, h)) = self.preview_dims {
                                    ui.label(
                                        RichText::new(format!(
                                            "{} — {}x{} px",
                                            self.active_preset_name(),
                                            w,
                                            h
                                        ))
                                        .size(12.0)
                                        .color(p.text_muted),
                                    );
                                }
                            });
                        }
                    });
            });
        });

        status_msg
    }

    fn ensure_preview_texture(&mut self, ctx: &egui::Context, source_path: Option<PathBuf>) {
        let desired_key = (self.selected_preset_idx, source_path.clone());
        if self.preview_key.as_ref() == Some(&desired_key)
            && (self.preview_texture.is_some() || self.preview_error.is_some())
        {
            return;
        }

        let base_image = match source_path {
            Some(ref path) => match CapturedImage::load_from_file(path) {
                Ok(img) => downscale_for_preview(&img, 420, 260),
                Err(_) => build_sample_preview_image(),
            },
            None => build_sample_preview_image(),
        };

        let rendered = match self.active_preset() {
            Some(preset) => apply_effects(&base_image, &preset.effects),
            None => Ok(base_image),
        };

        match rendered {
            Ok(img) => {
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [img.width() as usize, img.height() as usize],
                    img.rgba(),
                );
                let tex = ctx.load_texture(
                    "sharex_effects_preview",
                    color,
                    egui::TextureOptions::LINEAR,
                );
                self.preview_dims = Some((img.width(), img.height()));
                self.preview_texture = Some(tex);
                self.preview_error = None;
            }
            Err(err) => {
                self.preview_texture = None;
                self.preview_dims = None;
                self.preview_error = Some(format!("Preview error: {err}"));
            }
        }
        self.preview_key = Some(desired_key);
    }
}

fn preset_row(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    selected: bool,
    p: &SharexPalette,
) -> bool {
    let desired = Vec2::new(ui.available_width(), 46.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    let bg = if selected {
        p.accent
    } else if resp.hovered() {
        p.hover_bg
    } else {
        p.card_bg
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(5),
        Stroke::new(
            if selected { 1.5_f32 } else { 1.0_f32 },
            if selected || resp.hovered() {
                p.accent
            } else {
                p.border
            },
        ),
        egui::StrokeKind::Inside,
    );
    let title_color = if selected {
        Color32::WHITE
    } else {
        p.text_primary
    };
    let sub_color = if selected {
        Color32::from_rgba_unmultiplied(240, 245, 255, 220)
    } else {
        p.text_muted
    };
    ui.painter().text(
        egui::pos2(rect.min.x + 10.0, rect.min.y + 14.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(13.5),
        title_color,
    );
    ui.painter().text(
        egui::pos2(rect.min.x + 10.0, rect.min.y + 32.0),
        egui::Align2::LEFT_CENTER,
        subtitle,
        egui::FontId::proportional(11.0),
        sub_color,
    );
    resp.clicked()
}

fn summarize_preset(preset: &EffectPreset) -> String {
    let labels: Vec<&'static str> = preset
        .effects
        .iter()
        .map(|effect| match effect {
            ImageEffect::Border { .. } => "Border",
            ImageEffect::CanvasPadding { .. } => "Padding",
            ImageEffect::RoundedCorners { .. } => "Rounded corners",
            ImageEffect::DropShadow { .. } => "Drop shadow",
            ImageEffect::Resize { .. } => "Resize",
            ImageEffect::Grayscale => "Grayscale",
            ImageEffect::Invert => "Invert",
            ImageEffect::BrightnessContrast { .. } => "Brightness/Contrast",
            ImageEffect::AutoCrop { .. } => "Auto-crop",
        })
        .collect();
    if labels.is_empty() {
        "No steps".to_owned()
    } else {
        labels.join(" + ")
    }
}

fn describe_effect(effect: &ImageEffect) -> String {
    match effect {
        ImageEffect::Border { width, color } => {
            format!(
                "Border ({width}px, #{:02X}{:02X}{:02X})",
                color.r, color.g, color.b
            )
        }
        ImageEffect::CanvasPadding {
            top,
            right,
            bottom,
            left,
            ..
        } => format!("Canvas padding ({top}/{right}/{bottom}/{left} px)"),
        ImageEffect::RoundedCorners { radius } => format!("Rounded corners (radius {radius}px)"),
        ImageEffect::DropShadow {
            offset_x,
            offset_y,
            blur_radius,
            ..
        } => format!("Drop shadow (offset {offset_x},{offset_y}, blur {blur_radius}px)"),
        ImageEffect::Resize {
            width,
            height,
            keep_aspect,
        } => format!(
            "Resize ({width}x{height}{})",
            if *keep_aspect { ", keep aspect" } else { "" }
        ),
        ImageEffect::Grayscale => "Grayscale".to_owned(),
        ImageEffect::Invert => "Invert colors".to_owned(),
        ImageEffect::BrightnessContrast {
            brightness,
            contrast,
        } => format!("Brightness {brightness:+} / Contrast {contrast:+}"),
        ImageEffect::AutoCrop { tolerance } => format!("Auto-crop (tolerance {tolerance})"),
    }
}

fn downscale_for_preview(img: &CapturedImage, max_w: u32, max_h: u32) -> CapturedImage {
    if img.width() <= max_w && img.height() <= max_h {
        return img.clone();
    }
    apply_effects(
        img,
        &[ImageEffect::Resize {
            width: max_w,
            height: max_h,
            keep_aspect: true,
        }],
    )
    .unwrap_or_else(|_| img.clone())
}

fn build_sample_preview_image() -> CapturedImage {
    let width = 280u32;
    let height = 168u32;
    let mut rgba = vec![0u8; (width * height * 4) as usize];

    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let [r, g, b] = if y < 28 {
                if (12..22).contains(&x) && (9..19).contains(&y) {
                    [235, 87, 87]
                } else if (28..38).contains(&x) && (9..19).contains(&y) {
                    [245, 166, 35]
                } else if (44..54).contains(&x) && (9..19).contains(&y) {
                    [52, 199, 89]
                } else {
                    [36, 44, 56]
                }
            } else if (20..124).contains(&x) && (46..144).contains(&y) {
                let t = ((x - 20) * 255 / 104) as u8;
                [59, 110_u8.saturating_add(t / 4), 246]
            } else if (142..256).contains(&x)
                && ((52..66).contains(&y) || (78..90).contains(&y) || (102..114).contains(&y))
            {
                [196, 208, 224]
            } else if (142..212).contains(&x) && (126..144).contains(&y) {
                [52, 199, 89]
            } else {
                let gx = (x * 24 / width) as u8;
                let gy = (y * 20 / height) as u8;
                [26 + gx, 32 + gy, 44 + gx]
            };
            rgba[idx] = r;
            rgba[idx + 1] = g;
            rgba[idx + 2] = b;
            rgba[idx + 3] = 255;
        }
    }

    CapturedImage::from_rgba(width, height, rgba).expect("valid sample preview image")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_state_loads_builtin_presets_and_applies_only_when_enabled() {
        let mut state = EffectsPanelState::new();
        assert_eq!(state.presets().len(), 3);
        assert_eq!(state.active_preset_name(), "Clean Screenshot");

        let img = CapturedImage::from_rgba(24, 16, [80, 140, 220, 255].repeat(24 * 16))
            .expect("valid test image");

        let unchanged = state.apply_if_enabled(&img).expect("disabled returns copy");
        assert_eq!(unchanged, img);

        state.apply_on_capture = true;
        let transformed = state
            .apply_if_enabled(&img)
            .expect("enabled applies preset");
        assert!(transformed.width() > img.width());
        assert!(transformed.height() > img.height());

        state.set_selected_preset_idx(None);
        assert_eq!(state.active_preset_name(), "None");
        assert_eq!(
            state
                .apply_if_enabled(&img)
                .expect("none preset returns copy"),
            img
        );
    }
}
