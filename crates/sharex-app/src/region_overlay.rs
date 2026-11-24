use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use sharex_capture::DesktopSnapshot;
use sharex_core::{CapturedImage, Region};

/// Result produced by one frame of the interactive ShareX region capture overlay.
pub enum OverlayOutcome {
    Pending,
    Cancelled,
    Captured {
        image: CapturedImage,
        global_region: Region,
    },
}

/// Interactive borderless fullscreen overlay for ShareX-style drag-to-select screenshots.
pub struct RegionOverlayState {
    snapshot: DesktopSnapshot,
    texture: egui::TextureHandle,
    drag_start: Option<Pos2>,
    drag_current: Option<Pos2>,
}

impl RegionOverlayState {
    pub fn open(ctx: &egui::Context, snapshot: DesktopSnapshot) -> Self {
        let color_img = egui::ColorImage::from_rgba_unmultiplied(
            [
                snapshot.image.width() as usize,
                snapshot.image.height() as usize,
            ],
            snapshot.image.rgba(),
        );
        let texture = ctx.load_texture(
            "sharex_region_overlay_backdrop",
            color_img,
            egui::TextureOptions::NEAREST,
        );
        Self {
            snapshot,
            texture,
            drag_start: None,
            drag_current: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> OverlayOutcome {
        let viewport_id = egui::ViewportId::from_hash_of("sharex_region_capture_overlay");
        let w = self.snapshot.image.width() as f32;
        let h = self.snapshot.image.height() as f32;
        let builder = egui::ViewportBuilder::default()
            .with_title("ShareX Region Capture")
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_position([self.snapshot.origin_x as f32, self.snapshot.origin_y as f32])
            .with_inner_size([w, h])
            .with_active(true);

        let mut outcome = OverlayOutcome::Pending;

        ctx.show_viewport_immediate(viewport_id, builder, |overlay_ctx, _class| {
            if overlay_ctx.input(|i| i.viewport().close_requested())
                || overlay_ctx.input(|i| i.key_pressed(egui::Key::Escape))
            {
                outcome = OverlayOutcome::Cancelled;
                return;
            }

            if overlay_ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Ok(global_region) = Region::new(
                    self.snapshot.origin_x,
                    self.snapshot.origin_y,
                    self.snapshot.image.width(),
                    self.snapshot.image.height(),
                ) {
                    outcome = OverlayOutcome::Captured {
                        image: self.snapshot.image.clone(),
                        global_region,
                    };
                }
                return;
            }

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(overlay_ctx, |ui| {
                    let screen_rect = ui.max_rect();
                    let response = ui.allocate_rect(screen_rect, Sense::click_and_drag());
                    overlay_ctx.set_cursor_icon(egui::CursorIcon::Crosshair);

                    if response.secondary_clicked() {
                        outcome = OverlayOutcome::Cancelled;
                        return;
                    }

                    if response.drag_started_by(egui::PointerButton::Primary) {
                        self.drag_start = response.interact_pointer_pos();
                        self.drag_current = self.drag_start;
                    }
                    if response.dragged_by(egui::PointerButton::Primary) {
                        self.drag_current = response.interact_pointer_pos();
                    }

                    let pointer = overlay_ctx.input(|i| i.pointer.hover_pos());
                    let painter = ui.painter_at(screen_rect);

                    painter.image(
                        self.texture.id(),
                        screen_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );

                    let scrim = Color32::from_black_alpha(135);
                    if let (Some(start), Some(curr)) = (self.drag_start, self.drag_current) {
                        let sel = Rect::from_two_pos(start, curr).intersect(screen_rect);

                        painter.rect_filled(
                            Rect::from_min_max(
                                screen_rect.min,
                                Pos2::new(screen_rect.max.x, sel.min.y),
                            ),
                            0.0,
                            scrim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(screen_rect.min.x, sel.max.y),
                                screen_rect.max,
                            ),
                            0.0,
                            scrim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(screen_rect.min.x, sel.min.y),
                                Pos2::new(sel.min.x, sel.max.y),
                            ),
                            0.0,
                            scrim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(sel.max.x, sel.min.y),
                                Pos2::new(screen_rect.max.x, sel.max.y),
                            ),
                            0.0,
                            scrim,
                        );

                        painter.rect_stroke(
                            sel,
                            0.0,
                            Stroke::new(2.0_f32, Color32::from_rgb(59, 130, 246)),
                            egui::StrokeKind::Outside,
                        );
                        painter.rect_stroke(
                            sel,
                            0.0,
                            Stroke::new(1.0_f32, Color32::WHITE),
                            egui::StrokeKind::Inside,
                        );

                        if response.drag_stopped_by(egui::PointerButton::Primary) {
                            if let Some((cropped, global_region)) =
                                self.crop_selection(screen_rect, sel)
                            {
                                outcome = OverlayOutcome::Captured {
                                    image: cropped,
                                    global_region,
                                };
                                return;
                            }
                            self.drag_start = None;
                            self.drag_current = None;
                        }
                    } else {
                        painter.rect_filled(screen_rect, 0.0, scrim);
                    }

                    if let Some(pos) = pointer {
                        let guide = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(96, 165, 250, 175));
                        painter.line_segment(
                            [Pos2::new(screen_rect.min.x, pos.y), Pos2::new(screen_rect.max.x, pos.y)],
                            guide,
                        );
                        painter.line_segment(
                            [Pos2::new(pos.x, screen_rect.min.y), Pos2::new(pos.x, screen_rect.max.y)],
                            guide,
                        );

                        let hud_text = if let (Some(start), Some(curr)) =
                            (self.drag_start, self.drag_current)
                        {
                            let sel = Rect::from_two_pos(start, curr).intersect(screen_rect);
                            let (px, py, pw, ph) = self.selection_to_pixels(screen_rect, sel);
                            format!(
                                "X: {}  Y: {}  W: {}  H: {}",
                                self.snapshot.origin_x + px as i32,
                                self.snapshot.origin_y + py as i32,
                                pw,
                                ph
                            )
                        } else {
                            let sx = self.snapshot.image.width() as f32 / screen_rect.width().max(1.0);
                            let sy = self.snapshot.image.height() as f32 / screen_rect.height().max(1.0);
                            let px = ((pos.x - screen_rect.min.x) * sx).round() as i32;
                            let py = ((pos.y - screen_rect.min.y) * sy).round() as i32;
                            format!(
                                "X: {}  Y: {}  |  Drag to select  |  Enter: fullscreen  |  Esc: cancel",
                                self.snapshot.origin_x + px,
                                self.snapshot.origin_y + py
                            )
                        };

                        let badge_pos = Pos2::new(
                            (pos.x + 16.0).min(screen_rect.max.x - 320.0).max(12.0),
                            (pos.y + 18.0).min(screen_rect.max.y - 38.0).max(12.0),
                        );
                        let galley = painter.layout_no_wrap(
                            hud_text,
                            FontId::monospace(13.0),
                            Color32::WHITE,
                        );
                        let badge_rect =
                            Rect::from_min_size(badge_pos, galley.size() + Vec2::new(14.0, 8.0));
                        painter.rect_filled(
                            badge_rect,
                            CornerRadius::same(4),
                            Color32::from_rgba_unmultiplied(20, 24, 30, 230),
                        );
                        painter.rect_stroke(
                            badge_rect,
                            CornerRadius::same(4),
                            Stroke::new(1.0_f32, Color32::from_rgb(59, 130, 246)),
                            egui::StrokeKind::Inside,
                        );
                        painter.galley(badge_pos + Vec2::new(7.0, 4.0), galley, Color32::WHITE);
                    }
                });
        });

        outcome
    }

    fn selection_to_pixels(&self, screen_rect: Rect, sel: Rect) -> (u32, u32, u32, u32) {
        let sx = self.snapshot.image.width() as f32 / screen_rect.width().max(1.0);
        let sy = self.snapshot.image.height() as f32 / screen_rect.height().max(1.0);

        let x = (((sel.min.x - screen_rect.min.x) * sx).floor().max(0.0) as u32)
            .min(self.snapshot.image.width().saturating_sub(1));
        let y = (((sel.min.y - screen_rect.min.y) * sy).floor().max(0.0) as u32)
            .min(self.snapshot.image.height().saturating_sub(1));
        let w =
            ((sel.width() * sx).round() as u32).min(self.snapshot.image.width().saturating_sub(x));
        let h = ((sel.height() * sy).round() as u32)
            .min(self.snapshot.image.height().saturating_sub(y));
        (x, y, w, h)
    }

    fn crop_selection(&self, screen_rect: Rect, sel: Rect) -> Option<(CapturedImage, Region)> {
        let (x, y, w, h) = self.selection_to_pixels(screen_rect, sel);
        if w < 4 || h < 4 {
            return None;
        }
        let local = Region::new(x as i32, y as i32, w, h).ok()?;
        let cropped = self.snapshot.image.crop(local).ok()?;
        let global = Region::new(
            self.snapshot.origin_x + x as i32,
            self.snapshot.origin_y + y as i32,
            w,
            h,
        )
        .ok()?;
        Some((cropped, global))
    }
}
