use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};
use voltshot_core::CapturedImage;

pub const MIN_PIN_SCALE: f32 = 0.25;
pub const MAX_PIN_SCALE: f32 = 4.0;
pub const MIN_PIN_OPACITY: f32 = 0.20;
pub const MAX_PIN_OPACITY: f32 = 1.0;
const MIN_PIN_DIMENSION: f32 = 48.0;
const TOGGLED_DIM_OPACITY: f32 = 0.55;

/// A single floating always-on-top screenshot window pinned to the desktop.
pub struct PinnedWindow {
    pub id: u64,
    pub title: String,
    pub texture: egui::TextureHandle,
    pub size: [f32; 2],
    pub scale: f32,
    pub opacity: f32,
    pub borderless: bool,
}

impl PinnedWindow {
    pub fn clamped_scale(scale: f32) -> f32 {
        scale.clamp(MIN_PIN_SCALE, MAX_PIN_SCALE)
    }

    pub fn clamped_opacity(opacity: f32) -> f32 {
        opacity.clamp(MIN_PIN_OPACITY, MAX_PIN_OPACITY)
    }

    pub fn display_size(&self) -> [f32; 2] {
        [
            (self.size[0] * self.scale).max(MIN_PIN_DIMENSION),
            (self.size[1] * self.scale).max(MIN_PIN_DIMENSION),
        ]
    }

    pub fn set_scale(&mut self, scale: f32) -> bool {
        let next = Self::clamped_scale(scale);
        if (next - self.scale).abs() > f32::EPSILON {
            self.scale = next;
            true
        } else {
            false
        }
    }

    pub fn apply_scroll_zoom(&mut self, scroll_delta_y: f32) -> bool {
        if scroll_delta_y.abs() <= f32::EPSILON {
            return false;
        }
        let factor = (1.0 + scroll_delta_y * 0.0025).clamp(0.75, 1.35);
        self.set_scale(self.scale * factor)
    }

    pub fn reset_zoom(&mut self) -> bool {
        self.set_scale(1.0)
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = Self::clamped_opacity(opacity);
    }

    pub fn toggle_opacity(&mut self) {
        if self.opacity >= 0.95 {
            self.opacity = TOGGLED_DIM_OPACITY;
        } else {
            self.opacity = MAX_PIN_OPACITY;
        }
    }

    pub fn toggle_borderless(&mut self) {
        self.borderless = !self.borderless;
    }

    fn tint_color(&self) -> Color32 {
        let alpha = (Self::clamped_opacity(self.opacity) * 255.0).round() as u8;
        Color32::from_white_alpha(alpha)
    }
}

/// Manages all active floating Pin-to-Screen screenshot viewports.
#[derive(Default)]
pub struct PinManager {
    next_id: u64,
    windows: Vec<PinnedWindow>,
}

impl PinManager {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            windows: Vec::new(),
        }
    }

    /// Uploads `image` as a texture and registers a new floating always-on-top pinned window.
    pub fn pin_image(
        &mut self,
        ctx: &egui::Context,
        title: impl Into<String>,
        image: &CapturedImage,
    ) -> u64 {
        let id = self.next_id.max(1);
        self.next_id = id.saturating_add(1);

        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.rgba(),
        );
        let texture = ctx.load_texture(
            format!("voltshot_pin_window_{id}"),
            color_image,
            egui::TextureOptions::LINEAR,
        );

        let ppp = ctx.pixels_per_point().max(1.0);
        let logical_size = [
            (image.width() as f32 / ppp).max(MIN_PIN_DIMENSION),
            (image.height() as f32 / ppp).max(MIN_PIN_DIMENSION),
        ];

        self.windows.push(PinnedWindow {
            id,
            title: title.into(),
            texture,
            size: logical_size,
            scale: 1.0,
            opacity: MAX_PIN_OPACITY,
            borderless: true,
        });
        id
    }

    /// Renders every active pinned window in its own always-on-top immediate viewport.
    pub fn show_all(&mut self, ctx: &egui::Context) {
        let mut to_close: Vec<u64> = Vec::new();

        for pin in &mut self.windows {
            let viewport_id = egui::ViewportId::from_hash_of(("voltshot_pin_viewport", pin.id));
            let pin_icon = egui::IconData {
                rgba: crate::tray::build_tray_icon_rgba(64),
                width: 64,
                height: 64,
            };
            let builder = egui::ViewportBuilder::default()
                .with_title(pin.title.clone())
                .with_icon(std::sync::Arc::new(pin_icon))
                .with_always_on_top()
                .with_decorations(!pin.borderless)
                .with_inner_size(pin.display_size())
                .with_resizable(true)
                .with_transparent(true);

            let mut should_close = false;

            ctx.show_viewport_immediate(viewport_id, builder, |vp_ctx, _class| {
                if vp_ctx
                    .input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
                {
                    should_close = true;
                    return;
                }

                let (scroll_y, ctrl_held, reset_key, zoom_in_key, zoom_out_key) =
                    vp_ctx.input(|i| {
                        (
                            i.smooth_scroll_delta.y,
                            i.modifiers.ctrl,
                            i.key_pressed(egui::Key::Num0),
                            i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals),
                            i.key_pressed(egui::Key::Minus),
                        )
                    });

                let mut size_changed = false;
                if scroll_y.abs() > f32::EPSILON {
                    if ctrl_held {
                        pin.set_opacity(pin.opacity + scroll_y * 0.0015);
                    } else {
                        size_changed |= pin.apply_scroll_zoom(scroll_y);
                    }
                }
                if reset_key {
                    size_changed |= pin.reset_zoom();
                }
                if zoom_in_key {
                    size_changed |= pin.set_scale(pin.scale * 1.15);
                }
                if zoom_out_key {
                    size_changed |= pin.set_scale(pin.scale / 1.15);
                }

                let prev_borderless = pin.borderless;

                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(vp_ctx, |ui| {
                        let screen_rect = ui.max_rect();
                        let response = ui.allocate_rect(screen_rect, Sense::click_and_drag());

                        if response.drag_started_by(egui::PointerButton::Primary) {
                            vp_ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }

                        let painter = ui.painter_at(screen_rect);
                        painter.image(
                            pin.texture.id(),
                            screen_rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            pin.tint_color(),
                        );

                        if pin.borderless {
                            painter.rect_stroke(
                                screen_rect,
                                CornerRadius::ZERO,
                                Stroke::new(
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(59, 130, 246, 175),
                                ),
                                egui::StrokeKind::Inside,
                            );
                        }

                        response.context_menu(|ui| {
                            ui.label(format!("{} ({:.0}%)", pin.title, pin.scale * 100.0));
                            ui.separator();
                            if ui.button("Reset zoom (100%)").clicked() {
                                size_changed |= pin.reset_zoom();
                                ui.close_menu();
                            }
                            ui.horizontal(|ui| {
                                ui.label("Opacity");
                                let mut pct = pin.opacity * 100.0;
                                if ui
                                    .add(
                                        egui::Slider::new(&mut pct, 20.0..=100.0)
                                            .fixed_decimals(0)
                                            .suffix("%"),
                                    )
                                    .changed()
                                {
                                    pin.set_opacity(pct / 100.0);
                                }
                            });
                            if ui.button("Toggle opacity").clicked() {
                                pin.toggle_opacity();
                                ui.close_menu();
                            }
                            if ui.checkbox(&mut pin.borderless, "Borderless").changed() {
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Close (Esc)").clicked() {
                                should_close = true;
                                ui.close_menu();
                            }
                        });

                        let pointer_inside = vp_ctx.input(|i| i.pointer.has_pointer());
                        if pointer_inside {
                            egui::Area::new(egui::Id::new(("pin_hud", pin.id)))
                                .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-6.0, 6.0))
                                .show(vp_ctx, |ui| {
                                    egui::Frame::NONE
                                        .fill(Color32::from_rgba_unmultiplied(24, 28, 34, 215))
                                        .stroke(Stroke::new(
                                            1.0_f32,
                                            Color32::from_rgba_unmultiplied(90, 102, 120, 200),
                                        ))
                                        .corner_radius(CornerRadius::same(4))
                                        .inner_margin(egui::Margin::symmetric(6, 3))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = 4.0;
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "{:.0}%",
                                                        pin.scale * 100.0
                                                    ))
                                                    .size(11.0)
                                                    .color(Color32::from_rgb(200, 210, 224)),
                                                );
                                                if ui
                                                    .small_button("α")
                                                    .on_hover_text("Toggle opacity")
                                                    .clicked()
                                                {
                                                    pin.toggle_opacity();
                                                }
                                                if ui
                                                    .small_button("×")
                                                    .on_hover_text("Close (Esc)")
                                                    .clicked()
                                                {
                                                    should_close = true;
                                                }
                                            });
                                        });
                                });
                        }
                    });

                if pin.borderless != prev_borderless {
                    vp_ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(!pin.borderless));
                }
                if size_changed {
                    let [w, h] = pin.display_size();
                    vp_ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(w, h)));
                }
            });

            if should_close {
                to_close.push(pin.id);
            }
        }

        if !to_close.is_empty() {
            self.windows.retain(|w| !to_close.contains(&w.id));
        }
    }

    pub fn count(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn windows(&self) -> &[PinnedWindow] {
        &self.windows
    }

    pub fn get(&self, id: u64) -> Option<&PinnedWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut PinnedWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn close(&mut self, id: u64) -> bool {
        let before = self.windows.len();
        self.windows.retain(|w| w.id != id);
        self.windows.len() != before
    }

    pub fn close_all(&mut self) {
        self.windows.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_image(width: u32, height: u32) -> CapturedImage {
        let pixels = vec![255_u8; (width * height * 4) as usize];
        CapturedImage::from_rgba(width, height, pixels).expect("valid image")
    }

    #[test]
    fn pin_manager_tracks_multiple_windows_and_closes_individually_or_all() {
        let ctx = egui::Context::default();
        let mut manager = PinManager::new();
        assert_eq!(manager.count(), 0);
        assert!(manager.is_empty());

        let img = sample_image(320, 180);
        let id1 = manager.pin_image(&ctx, "Shot 1", &img);
        let id2 = manager.pin_image(&ctx, "Shot 2", &img);
        let id3 = manager.pin_image(&ctx, "Shot 3", &img);

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_eq!(manager.count(), 3);
        assert!(!manager.is_empty());

        let pin = manager.get(id1).expect("first pin exists");
        assert_eq!(pin.title, "Shot 1");
        assert_eq!(pin.scale, 1.0);
        assert_eq!(pin.opacity, MAX_PIN_OPACITY);
        assert!(pin.borderless);

        assert!(manager.close(id2));
        assert!(!manager.close(id2));
        assert_eq!(manager.count(), 2);
        assert!(manager.get(id2).is_none());

        manager.close_all();
        assert_eq!(manager.count(), 0);
        assert!(manager.is_empty());
    }

    #[test]
    fn zoom_opacity_and_borderless_controls_clamp_correctly() {
        let ctx = egui::Context::default();
        let mut manager = PinManager::new();
        let img = sample_image(400, 200);
        let id = manager.pin_image(&ctx, "Zoom Test", &img);
        let pin = manager.get_mut(id).expect("pin exists");

        assert!(pin.set_scale(10.0));
        assert_eq!(pin.scale, MAX_PIN_SCALE);
        assert!(!pin.set_scale(12.0));

        assert!(pin.set_scale(0.01));
        assert_eq!(pin.scale, MIN_PIN_SCALE);

        assert!(pin.apply_scroll_zoom(120.0));
        assert!(pin.scale > MIN_PIN_SCALE);
        assert!(pin.reset_zoom());
        assert_eq!(pin.scale, 1.0);
        assert_eq!(pin.display_size(), [400.0, 200.0]);

        pin.set_opacity(0.0);
        assert_eq!(pin.opacity, MIN_PIN_OPACITY);
        pin.set_opacity(2.0);
        assert_eq!(pin.opacity, MAX_PIN_OPACITY);

        pin.toggle_opacity();
        assert!(pin.opacity < 0.95);
        pin.toggle_opacity();
        assert_eq!(pin.opacity, MAX_PIN_OPACITY);

        assert!(pin.borderless);
        pin.toggle_borderless();
        assert!(!pin.borderless);
    }
}
