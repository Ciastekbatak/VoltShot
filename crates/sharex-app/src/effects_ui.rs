use crate::i18n::Strings;
use crate::theme::SharexPalette;
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use sharex_core::CapturedImage;
use sharex_effects::{
    EffectColor, EffectPreset, ImageEffect, WatermarkAnchor, apply_effects, builtin_presets,
    extended_presets,
};
use std::path::{Path, PathBuf};

/// Interactive Image Effects preset selector and live preview panel state.
pub struct EffectsPanelState {
    presets: Vec<EffectPreset>,
    extended_presets: Vec<EffectPreset>,
    custom: EffectPreset,
    selected_preset_idx: Option<usize>,
    preset_toml: String,
    toml_status: Option<String>,
    pub apply_on_capture: bool,
    use_latest_capture: bool,
    preview_texture: Option<egui::TextureHandle>,
    preview_dims: Option<(u32, u32)>,
    preview_key: Option<(Option<usize>, Option<PathBuf>)>,
    preview_source: Option<(Option<PathBuf>, CapturedImage)>,
    preview_error: Option<String>,
}

impl Default for EffectsPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectsPanelState {
    pub fn new() -> Self {
        let presets = builtin_presets();
        Self {
            extended_presets: extended_presets().into_iter().skip(presets.len()).collect(),
            presets,
            custom: EffectPreset {
                name: "Custom Pipeline".into(),
                enabled: true,
                effects: Vec::new(),
            },
            selected_preset_idx: Some(0),
            preset_toml: String::new(),
            toml_status: None,
            apply_on_capture: false,
            use_latest_capture: false,
            preview_texture: None,
            preview_dims: None,
            preview_key: None,
            preview_source: None,
            preview_error: None,
        }
    }

    /// Returns the original built-in catalog; the panel also shows supplemental and custom presets.
    pub fn presets(&self) -> &[EffectPreset] {
        &self.presets
    }

    pub fn selected_preset_idx(&self) -> Option<usize> {
        self.selected_preset_idx
    }

    pub fn set_selected_preset_idx(&mut self, idx: Option<usize>) {
        self.selected_preset_idx = idx.filter(|&index| index <= self.custom_index());
    }

    pub fn active_preset(&self) -> Option<&EffectPreset> {
        let index = self.selected_preset_idx?;
        if index == self.custom_index() {
            Some(&self.custom)
        } else {
            self.presets()
                .iter()
                .chain(&self.extended_presets)
                .nth(index)
        }
    }

    fn custom_index(&self) -> usize {
        self.presets.len() + self.extended_presets.len()
    }

    fn editable_pipeline(&mut self) -> &mut EffectPreset {
        if self.selected_preset_idx != Some(self.custom_index()) {
            self.custom.effects = self
                .active_preset()
                .map_or_else(Vec::new, |preset| preset.effects.clone());
            self.custom.name = "Custom Pipeline".into();
            self.custom.enabled = true;
            self.selected_preset_idx = Some(self.custom_index());
        }
        self.preview_key = None;
        &mut self.custom
    }

    fn edit_pipeline(&mut self, edit: PipelineEdit) {
        let effects = &mut self.editable_pipeline().effects;
        match edit {
            PipelineEdit::Add(effect) => effects.push(effect),
            PipelineEdit::Move { from, to } => effects.swap(from, to),
            PipelineEdit::Remove(index) => {
                effects.remove(index);
            }
        }
    }

    fn export_preset_toml(&self) -> Result<String, String> {
        self.active_preset()
            .ok_or_else(|| "Select a preset to export.".to_owned())?
            .to_toml()
            .map_err(|err| err.to_string())
    }

    fn import_preset_toml(&mut self, toml: &str) -> Result<(), String> {
        self.custom = EffectPreset::from_toml(toml).map_err(|err| err.to_string())?;
        self.selected_preset_idx = Some(self.custom_index());
        self.preview_key = None;
        Ok(())
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
        preset.apply(image).map_err(|err| err.to_string())
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
            egui::ScrollArea::vertical().show(&mut columns[0], |ui| {
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
                    .presets()
                    .iter()
                    .chain(&self.extended_presets)
                    .map(|preset| (preset.name.clone(), summarize_preset(preset)))
                    .collect();

                for (idx, (name, summary)) in preset_summaries.iter().enumerate() {
                    let is_selected = self.selected_preset_idx == Some(idx);
                    if preset_row(ui, name, summary, is_selected, p) {
                        self.selected_preset_idx = Some(idx);
                    }
                    ui.add_space(4.0);
                }

                let custom_index = self.custom_index();
                if preset_row(
                    ui,
                    &self.custom.name,
                    "Editable custom pipeline",
                    self.selected_preset_idx == Some(custom_index),
                    p,
                ) {
                    self.selected_preset_idx = Some(custom_index);
                }

                ui.add_space(10.0);
                ui.label(
                    RichText::new("CUSTOM PIPELINE BUILDER")
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
                        self.show_pipeline(ui);
                    });

                self.show_preset_toml(ui, p);
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
                                egui::Button::new("Latest capture")
                                    .selected(self.use_latest_capture && has_latest),
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

    fn show_pipeline(&mut self, ui: &mut egui::Ui) {
        let editing = self.selected_preset_idx == Some(self.custom_index());
        if !editing
            && self.active_preset().is_some()
            && ui
                .button("Edit a copy")
                .on_hover_text("Copy this preset over the current custom pipeline")
                .clicked()
        {
            self.editable_pipeline();
        }
        let mut edit = None;
        let add = ui.menu_button("+ Add Effect", |ui| {
            for effect in default_effects() {
                if ui.button(effect_label(&effect)).clicked() {
                    edit = Some(PipelineEdit::Add(effect));
                    ui.close_menu();
                }
            }
        });
        if !editing && self.active_preset().is_some() {
            add.response
                .on_hover_text("Start a custom pipeline from this preset, then add a step");
        }

        if self.selected_preset_idx == Some(self.custom_index()) {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.custom.name)
                        .hint_text("Preset name")
                        .desired_width(130.0),
                );
                if ui.checkbox(&mut self.custom.enabled, "Enabled").changed() {
                    self.preview_key = None;
                }
            });
            let len = self.custom.effects.len();
            for (index, effect) in self.custom.effects.iter_mut().enumerate() {
                ui.separator();
                ui.push_id(index, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(format!("{}. {}", index + 1, effect_label(effect)));
                        if ui
                            .add_enabled(index > 0, egui::Button::new("^"))
                            .on_hover_text("Move up")
                            .clicked()
                        {
                            edit = Some(PipelineEdit::Move {
                                from: index,
                                to: index - 1,
                            });
                        }
                        if ui
                            .add_enabled(index + 1 < len, egui::Button::new("v"))
                            .on_hover_text("Move down")
                            .clicked()
                        {
                            edit = Some(PipelineEdit::Move {
                                from: index,
                                to: index + 1,
                            });
                        }
                        if ui
                            .small_button("x")
                            .on_hover_text("Remove effect")
                            .clicked()
                        {
                            edit = Some(PipelineEdit::Remove(index));
                        }
                    });
                    if edit_effect(ui, effect) {
                        self.preview_key = None;
                    }
                });
            }
        } else if let Some(preset) = self.active_preset() {
            for (index, effect) in preset.effects.iter().enumerate() {
                ui.label(format!("{}. {}", index + 1, effect_label(effect)));
            }
        } else {
            ui.label("No effects will be applied.");
        }

        if let Some(edit) = edit {
            self.edit_pipeline(edit);
        }
    }

    fn show_preset_toml(&mut self, ui: &mut egui::Ui, palette: &SharexPalette) {
        ui.collapsing("Export / Import TOML", |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.active_preset().is_some(),
                        egui::Button::new("Export TOML"),
                    )
                    .clicked()
                {
                    match self.export_preset_toml() {
                        Ok(toml) => {
                            ui.ctx().copy_text(toml.clone());
                            self.preset_toml = toml;
                            self.toml_status = Some("Preset TOML copied to the clipboard.".into());
                        }
                        Err(err) => self.toml_status = Some(err),
                    }
                }
                if ui.button("Load TOML Preset").clicked() {
                    let toml = std::mem::take(&mut self.preset_toml);
                    self.toml_status = Some(match self.import_preset_toml(&toml) {
                        Ok(()) => format!("Loaded {}.", self.custom.name),
                        Err(err) => format!("Import failed: {err}"),
                    });
                    self.preset_toml = toml;
                }
            });
            ui.add(
                egui::TextEdit::multiline(&mut self.preset_toml)
                    .code_editor()
                    .desired_width(ui.available_width())
                    .desired_rows(6)
                    .hint_text("Paste preset TOML here"),
            );
            if let Some(status) = &self.toml_status {
                ui.label(RichText::new(status).color(palette.text_muted));
            }
        });
    }

    fn ensure_preview_texture(&mut self, ctx: &egui::Context, source_path: Option<PathBuf>) {
        let desired_key = (self.selected_preset_idx, source_path.clone());
        if self.preview_key.as_ref() == Some(&desired_key)
            && (self.preview_texture.is_some() || self.preview_error.is_some())
        {
            return;
        }

        if self
            .preview_source
            .as_ref()
            .is_none_or(|(path, _)| *path != source_path)
        {
            let image = source_path
                .as_ref()
                .and_then(|path| CapturedImage::load_from_file(path).ok())
                .map_or_else(build_sample_preview_image, |image| {
                    downscale_for_preview(&image, 420, 260)
                });
            self.preview_source = Some((source_path, image));
        }
        let Some((_, base_image)) = &self.preview_source else {
            return;
        };
        let rendered = self
            .active_preset()
            .map_or_else(|| Ok(base_image.clone()), |preset| preset.apply(base_image));

        match rendered {
            Ok(img) => {
                let texture_limit = ctx.input(|input| input.max_texture_side) as u32;
                let preview = downscale_for_preview(&img, texture_limit, texture_limit);
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [preview.width() as usize, preview.height() as usize],
                    preview.rgba(),
                );
                if let Some(texture) = &mut self.preview_texture {
                    texture.set(color, egui::TextureOptions::LINEAR);
                } else {
                    self.preview_texture = Some(ctx.load_texture(
                        "sharex_effects_preview",
                        color,
                        egui::TextureOptions::LINEAR,
                    ));
                }
                self.preview_dims = Some((img.width(), img.height()));
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
    let label_painter = ui.painter().with_clip_rect(rect);
    label_painter.text(
        egui::pos2(rect.min.x + 10.0, rect.min.y + 14.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(13.5),
        title_color,
    );
    label_painter.text(
        egui::pos2(rect.min.x + 10.0, rect.min.y + 32.0),
        egui::Align2::LEFT_CENTER,
        subtitle,
        egui::FontId::proportional(11.0),
        sub_color,
    );
    resp.clicked()
}

enum PipelineEdit {
    Add(ImageEffect),
    Move { from: usize, to: usize },
    Remove(usize),
}

fn summarize_preset(preset: &EffectPreset) -> String {
    let labels: Vec<_> = preset.effects.iter().map(effect_label).collect();
    if labels.is_empty() {
        "No steps".to_owned()
    } else {
        labels.join(" + ")
    }
}

fn effect_label(effect: &ImageEffect) -> &'static str {
    match effect {
        ImageEffect::Border { .. } => "Border",
        ImageEffect::CanvasPadding { .. } => "Canvas padding",
        ImageEffect::RoundedCorners { .. } => "Rounded corners",
        ImageEffect::DropShadow { .. } => "Drop shadow",
        ImageEffect::Resize { .. } => "Resize",
        ImageEffect::Grayscale => "Grayscale",
        ImageEffect::Invert => "Invert colors",
        ImageEffect::BrightnessContrast { .. } => "Brightness/Contrast",
        ImageEffect::AutoCrop { .. } => "Auto-crop",
        ImageEffect::WatermarkText { .. } => "Text watermark",
        ImageEffect::Colorize { .. } => "Colorize",
        ImageEffect::Slice { .. } => "Slice",
        ImageEffect::ImageWatermark { .. } => "Image watermark",
    }
}

fn default_effects() -> [ImageEffect; 11] {
    [
        ImageEffect::Border {
            width: 2,
            color: EffectColor::BLACK,
        },
        ImageEffect::DropShadow {
            offset_x: 0,
            offset_y: 8,
            blur_radius: 12,
            color: EffectColor::rgba(0, 0, 0, 120),
            background: EffectColor::TRANSPARENT,
        },
        ImageEffect::RoundedCorners { radius: 12 },
        ImageEffect::CanvasPadding {
            top: 12,
            right: 12,
            bottom: 12,
            left: 12,
            color: EffectColor::WHITE,
        },
        ImageEffect::WatermarkText {
            text: "ShareX".into(),
            position: WatermarkAnchor::BottomRight,
            color: EffectColor::WHITE,
            opacity: 0.8,
            scale: 2,
            margin: 12,
            shadow: true,
        },
        ImageEffect::Colorize {
            color: EffectColor::rgb(80, 140, 220),
            strength: 0.5,
        },
        ImageEffect::BrightnessContrast {
            brightness: 0,
            contrast: 15,
        },
        ImageEffect::Grayscale,
        ImageEffect::Invert,
        ImageEffect::Slice {
            min_height: 4,
            max_height: 16,
            shift: 8,
        },
        ImageEffect::AutoCrop { tolerance: 8 },
    ]
}

fn edit_effect(ui: &mut egui::Ui, effect: &mut ImageEffect) -> bool {
    match effect {
        ImageEffect::Border { width, color } => {
            effect_slider(ui, width, 0..=64, "Width") | effect_color(ui, color, "Color")
        }
        ImageEffect::DropShadow {
            offset_x,
            offset_y,
            blur_radius,
            color,
            background,
        } => {
            effect_slider(ui, blur_radius, 0..=64, "Blur radius")
                | effect_slider(ui, offset_x, -64..=64, "Horizontal offset")
                | effect_slider(ui, offset_y, -64..=64, "Vertical offset")
                | effect_color(ui, color, "Shadow")
                | effect_color(ui, background, "Background")
        }
        ImageEffect::RoundedCorners { radius } => effect_slider(ui, radius, 0..=128, "Radius"),
        ImageEffect::CanvasPadding {
            top,
            right,
            bottom,
            left,
            color,
        } => {
            effect_slider(ui, top, 0..=128, "Top")
                | effect_slider(ui, right, 0..=128, "Right")
                | effect_slider(ui, bottom, 0..=128, "Bottom")
                | effect_slider(ui, left, 0..=128, "Left")
                | effect_color(ui, color, "Color")
        }
        ImageEffect::WatermarkText {
            text,
            position,
            color,
            opacity,
            scale,
            margin,
            shadow,
        } => {
            ui.add(egui::TextEdit::singleline(text).hint_text("Watermark text"))
                .changed()
                | edit_anchor(ui, position)
                | effect_color(ui, color, "Text color")
                | effect_slider(ui, opacity, 0.0..=1.0, "Opacity")
                | effect_slider(ui, scale, 1..=12, "Text scale")
                | effect_slider(ui, margin, 0..=128, "Margin")
                | ui.checkbox(shadow, "Text shadow").changed()
        }
        ImageEffect::Colorize { color, strength } => {
            effect_color(ui, color, "Tint") | effect_slider(ui, strength, 0.0..=1.0, "Strength")
        }
        ImageEffect::BrightnessContrast {
            brightness,
            contrast,
        } => {
            effect_slider(ui, brightness, -100..=100, "Brightness")
                | effect_slider(ui, contrast, -100..=100, "Contrast")
        }
        ImageEffect::Slice {
            min_height,
            max_height,
            shift,
        } => {
            let minimum_changed = effect_slider(ui, min_height, 1..=128, "Min band height");
            let maximum_changed = effect_slider(ui, max_height, 1..=128, "Max band height");
            if minimum_changed && *min_height > *max_height {
                *max_height = *min_height;
            } else if maximum_changed && *max_height < *min_height {
                *min_height = *max_height;
            }
            minimum_changed | maximum_changed | effect_slider(ui, shift, -128..=128, "Shift")
        }
        ImageEffect::AutoCrop { tolerance } => effect_slider(ui, tolerance, 0..=255, "Tolerance"),
        ImageEffect::Resize {
            width,
            height,
            keep_aspect,
        } => {
            effect_slider(ui, width, 1..=4096, "Width")
                | effect_slider(ui, height, 1..=4096, "Height")
                | ui.checkbox(keep_aspect, "Keep aspect ratio").changed()
        }
        ImageEffect::ImageWatermark {
            width,
            height,
            position,
            opacity,
            margin,
            ..
        } => {
            ui.label(format!("Embedded stamp: {width} x {height}"));
            edit_anchor(ui, position)
                | effect_slider(ui, opacity, 0.0..=1.0, "Opacity")
                | effect_slider(ui, margin, 0..=128, "Margin")
        }
        ImageEffect::Grayscale | ImageEffect::Invert => false,
    }
}

fn effect_slider<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    label: &str,
) -> bool {
    ui.add(
        egui::Slider::new(value, range)
            .text(label)
            .clamping(egui::SliderClamping::Edits),
    )
    .changed()
}

fn effect_color(ui: &mut egui::Ui, color: &mut EffectColor, label: &str) -> bool {
    let mut rgba = color.to_array();
    let changed = ui
        .horizontal(|ui| {
            ui.label(label);
            ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed()
        })
        .inner;
    if changed {
        let [red, green, blue, alpha] = rgba;
        *color = EffectColor::rgba(red, green, blue, alpha);
    }
    changed
}

fn edit_anchor(ui: &mut egui::Ui, position: &mut WatermarkAnchor) -> bool {
    let original = *position;
    let anchors = [
        (WatermarkAnchor::TopLeft, "Top left"),
        (WatermarkAnchor::TopCenter, "Top center"),
        (WatermarkAnchor::TopRight, "Top right"),
        (WatermarkAnchor::MiddleLeft, "Middle left"),
        (WatermarkAnchor::Center, "Center"),
        (WatermarkAnchor::MiddleRight, "Middle right"),
        (WatermarkAnchor::BottomLeft, "Bottom left"),
        (WatermarkAnchor::BottomCenter, "Bottom center"),
        (WatermarkAnchor::BottomRight, "Bottom right"),
    ];
    let selected = anchors
        .iter()
        .find_map(|&(anchor, label)| (anchor == original).then_some(label));
    egui::ComboBox::from_id_salt("watermark_anchor")
        .selected_text(selected.unwrap_or_default())
        .show_ui(ui, |ui| {
            for (anchor, label) in anchors {
                ui.selectable_value(position, anchor, label);
            }
        });
    *position != original
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
    fn custom_pipeline_edits_preserve_builtins_and_update_the_preview() {
        let mut state = EffectsPanelState::new();
        state.set_selected_preset_idx(Some(state.presets().len()));
        assert_eq!(state.active_preset_name(), "Watermarked Share");
        let original = state.active_preset().unwrap().clone();
        state.edit_pipeline(PipelineEdit::Add(ImageEffect::Invert));
        state.edit_pipeline(PipelineEdit::Move { from: 1, to: 0 });
        let custom = state.active_preset().unwrap();
        assert_eq!(
            custom.effects,
            [ImageEffect::Invert, original.effects[0].clone()]
        );

        let ctx = egui::Context::default();
        state.ensure_preview_texture(&ctx, None);
        assert_eq!(state.preview_dims, Some((280, 168)));
        state.edit_pipeline(PipelineEdit::Remove(1));
        state.edit_pipeline(PipelineEdit::Add(default_effects()[0].clone()));
        state.ensure_preview_texture(&ctx, None);
        assert_eq!(state.preview_dims, Some((284, 172)));
        state.set_selected_preset_idx(Some(state.presets().len()));
        assert_eq!(state.active_preset(), Some(&original));
    }

    #[test]
    fn custom_pipeline_toml_round_trips_and_failed_import_keeps_the_active_preset() {
        let mut source = EffectsPanelState::new();
        source.set_selected_preset_idx(None);
        for effect in default_effects() {
            source.edit_pipeline(PipelineEdit::Add(effect));
        }
        source.custom.name = "Shared preset".into();
        let toml = source.export_preset_toml().unwrap();
        let mut destination = EffectsPanelState::new();
        destination.import_preset_toml(&toml).unwrap();
        assert_eq!(destination.active_preset(), source.active_preset());
        assert_eq!(destination.export_preset_toml().unwrap(), toml);
        assert!(destination.import_preset_toml("unparsable").is_err());
        assert_eq!(destination.active_preset(), source.active_preset());
    }

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
