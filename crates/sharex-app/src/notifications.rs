use crate::theme::palette;
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};
use sharex_core::CapturedImage;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Maximum number of stacked toast viewports rendered simultaneously.
pub const MAX_VISIBLE_TOASTS: usize = 3;

/// Default lifetime for a capture toast notification before auto-dismissal.
pub const DEFAULT_TOAST_DURATION: Duration = Duration::from_secs(4);

/// Default logical width of a compact ShareX toast notification.
pub const TOAST_WIDTH: f32 = 340.0;

/// Default logical height of a compact ShareX toast notification.
pub const TOAST_HEIGHT: f32 = 86.0;

/// Vertical gap in logical pixels between stacked toast notifications.
pub const TOAST_GAP: f32 = 10.0;

/// Margin in logical pixels separating the toast stack from screen edges.
pub const TOAST_MARGIN: f32 = 16.0;

const FADE_OUT_SECS: f32 = 0.5;
const THUMB_BOX_SIZE: f32 = 62.0;

/// Action triggered when the user clicks an active toast notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastAction {
    None,
    OpenPath(PathBuf),
    OpenUrl(String),
}

/// Data backing a single ShareX capture toast notification.
pub struct ToastNotification {
    pub id: u64,
    pub title: String,
    pub subtitle: String,
    pub thumbnail: Option<egui::TextureHandle>,
    pub target_path: Option<PathBuf>,
    pub target_url: Option<String>,
    pub created_at: Instant,
    pub duration: Duration,
}

impl ToastNotification {
    fn click_action(&self) -> ToastAction {
        if let Some(url) = &self.target_url {
            ToastAction::OpenUrl(url.clone())
        } else if let Some(path) = &self.target_path {
            ToastAction::OpenPath(path.clone())
        } else {
            ToastAction::None
        }
    }

    fn remaining_fraction(&self, now: Instant) -> f32 {
        let total = self.duration.as_secs_f32();
        if total <= f32::EPSILON {
            return 0.0;
        }
        let elapsed = now.saturating_duration_since(self.created_at).as_secs_f32();
        (1.0 - elapsed / total).clamp(0.0, 1.0)
    }

    fn fade_alpha(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.created_at);
        let remaining = self.duration.saturating_sub(elapsed).as_secs_f32();
        (remaining / FADE_OUT_SECS).clamp(0.0, 1.0)
    }
}

/// Computes the top-left `(x, y)` position for the `index`-th stacked toast within a work area.
///
/// Index `0` sits at the bottom-right corner above the taskbar; higher indices stack upward with a
/// `TOAST_GAP` vertical gap and are clamped within `[0, work_area - toast]`.
pub fn compute_toast_rect(
    index: usize,
    work_area_w: f32,
    work_area_h: f32,
    toast_w: f32,
    toast_h: f32,
) -> (f32, f32) {
    let safe_area_w = work_area_w.max(0.0);
    let safe_area_h = work_area_h.max(0.0);
    let safe_toast_w = toast_w.max(0.0);
    let safe_toast_h = toast_h.max(0.0);

    let max_x = (safe_area_w - safe_toast_w).max(0.0);
    let max_y = (safe_area_h - safe_toast_h).max(0.0);

    let raw_x = safe_area_w - safe_toast_w - TOAST_MARGIN;
    let raw_y =
        safe_area_h - TOAST_MARGIN - safe_toast_h - (index as f32) * (safe_toast_h + TOAST_GAP);

    (raw_x.clamp(0.0, max_x), raw_y.clamp(0.0, max_y))
}

/// Manages active borderless ShareX capture toast notifications.
pub struct ToastManager {
    next_id: u64,
    toasts: Vec<ToastNotification>,
    default_duration: Duration,
}

impl Default for ToastManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ToastManager {
    /// Creates an empty toast manager with the default 4-second display duration.
    pub fn new() -> Self {
        Self {
            next_id: 1,
            toasts: Vec::new(),
            default_duration: DEFAULT_TOAST_DURATION,
        }
    }

    /// Enqueues a new capture toast notification, uploading an optional thumbnail texture.
    ///
    /// At most `MAX_VISIBLE_TOASTS` notifications are retained at once; older entries are dropped
    /// when capacity is exceeded.
    pub fn push_capture(
        &mut self,
        ctx: &egui::Context,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        thumb_img: Option<&CapturedImage>,
        path: Option<PathBuf>,
        url: Option<String>,
    ) -> u64 {
        let id = self.next_id.max(1);
        self.next_id = id.saturating_add(1);

        let thumbnail = thumb_img.map(|img| {
            let color_image = crate::image_texture::bounded_color_image(img, 128);
            ctx.load_texture(
                format!("sharex_toast_thumb_{id}"),
                color_image,
                egui::TextureOptions::LINEAR,
            )
        });

        self.toasts.push(ToastNotification {
            id,
            title: title.into(),
            subtitle: subtitle.into(),
            thumbnail,
            target_path: path,
            target_url: url,
            created_at: Instant::now(),
            duration: self.default_duration,
        });

        while self.toasts.len() > MAX_VISIBLE_TOASTS {
            self.toasts.remove(0);
        }

        id
    }

    /// Returns the number of currently active toast notifications.
    pub fn active_count(&self) -> usize {
        self.toasts.len()
    }

    /// Dismisses a single toast notification by its `id`, returning `true` if it existed.
    pub fn dismiss(&mut self, id: u64) -> bool {
        let before = self.toasts.len();
        self.toasts.retain(|toast| toast.id != id);
        self.toasts.len() != before
    }

    /// Dismisses all active toast notifications.
    pub fn dismiss_all(&mut self) {
        self.toasts.clear();
    }

    /// Removes all notifications whose lifetime has elapsed as of `now`, returning how many expired.
    pub fn prune_expired(&mut self, now: Instant) -> usize {
        let before = self.toasts.len();
        self.toasts
            .retain(|toast| now.saturating_duration_since(toast.created_at) < toast.duration);
        before - self.toasts.len()
    }

    /// Renders up to 3 stacked borderless always-on-top toast viewports and returns any clicked action.
    pub fn show_all(&mut self, ctx: &egui::Context, dark: bool) -> ToastAction {
        let now = Instant::now();
        self.prune_expired(now);
        if self.toasts.is_empty() {
            return ToastAction::None;
        }

        ctx.request_repaint_after(Duration::from_millis(50));

        let (origin_x, origin_y, work_w, work_h) = primary_work_area(ctx);
        let p = palette(dark);
        let mut to_dismiss: Vec<u64> = Vec::new();
        let mut action = ToastAction::None;

        for (slot, toast) in self
            .toasts
            .iter()
            .rev()
            .take(MAX_VISIBLE_TOASTS)
            .enumerate()
        {
            let (rel_x, rel_y) =
                compute_toast_rect(slot, work_w, work_h, TOAST_WIDTH, TOAST_HEIGHT);
            let pos = Pos2::new(origin_x + rel_x, origin_y + rel_y);

            let alpha = toast.fade_alpha(now);
            let progress = toast.remaining_fraction(now);
            let viewport_id = egui::ViewportId::from_hash_of(("sharex_toast_viewport", toast.id));
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("ShareX Notification {}", toast.id))
                .with_always_on_top()
                .with_decorations(false)
                .with_resizable(false)
                .with_active(false)
                .with_transparent(true)
                .with_taskbar(false)
                .with_inner_size([TOAST_WIDTH, TOAST_HEIGHT])
                .with_position([pos.x, pos.y]);

            let mut dismissed = false;
            let mut triggered_action = ToastAction::None;

            ctx.show_viewport_immediate(viewport_id, builder, |vp_ctx, _class| {
                vp_ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));

                if vp_ctx.input(|i| i.viewport().close_requested()) {
                    dismissed = true;
                    return;
                }

                let bg_fill = with_alpha(p.card_bg, alpha * 0.96);
                let border_color = with_alpha(p.border, alpha);
                let accent_color = with_alpha(p.accent, alpha);
                let primary_text = with_alpha(p.text_primary, alpha);
                let muted_text = with_alpha(p.text_muted, alpha);
                let thumb_bg = with_alpha(p.sidebar_bg, alpha);

                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(vp_ctx, |ui| {
                        let full_rect = ui.max_rect();
                        let card_response = ui.interact(
                            full_rect,
                            egui::Id::new(("sharex_toast_card", toast.id)),
                            Sense::click(),
                        );

                        let painter = ui.painter();
                        painter.rect_filled(full_rect, CornerRadius::same(8), bg_fill);
                        painter.rect_stroke(
                            full_rect,
                            CornerRadius::same(8),
                            Stroke::new(1.0_f32, border_color),
                            egui::StrokeKind::Inside,
                        );

                        let accent_rect = Rect::from_min_max(
                            full_rect.min,
                            Pos2::new(full_rect.min.x + 4.0, full_rect.max.y),
                        );
                        painter.rect_filled(
                            accent_rect,
                            CornerRadius {
                                nw: 8,
                                sw: 8,
                                ne: 0,
                                se: 0,
                            },
                            accent_color,
                        );

                        let progress_height = 2.0_f32;
                        let progress_width = (full_rect.width() - 4.0).max(0.0) * progress;
                        let progress_rect = Rect::from_min_max(
                            Pos2::new(full_rect.min.x + 4.0, full_rect.max.y - progress_height),
                            Pos2::new(full_rect.min.x + 4.0 + progress_width, full_rect.max.y),
                        );
                        painter.rect_filled(progress_rect, CornerRadius::ZERO, accent_color);

                        let mut close_clicked = false;
                        let content_rect = full_rect.shrink2(Vec2::new(10.0, 8.0));
                        let mut content_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(content_rect)
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );

                        content_ui.add_space(4.0);

                        let (thumb_rect, _) = content_ui
                            .allocate_exact_size(Vec2::splat(THUMB_BOX_SIZE), Sense::hover());
                        content_ui.painter().rect_filled(
                            thumb_rect,
                            CornerRadius::same(5),
                            thumb_bg,
                        );
                        if let Some(tex) = &toast.thumbnail {
                            let img_rect = fit_aspect_rect(tex.size_vec2(), thumb_rect);
                            let tint = Color32::from_white_alpha((alpha * 255.0).round() as u8);
                            content_ui.painter().image(
                                tex.id(),
                                img_rect,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                tint,
                            );
                        } else {
                            content_ui.painter().text(
                                thumb_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "SX",
                                egui::FontId::proportional(16.0),
                                accent_color,
                            );
                        }
                        content_ui.painter().rect_stroke(
                            thumb_rect,
                            CornerRadius::same(5),
                            Stroke::new(1.0_f32, border_color),
                            egui::StrokeKind::Inside,
                        );

                        content_ui.add_space(8.0);

                        let text_width = (content_ui.available_width() - 26.0).max(40.0);
                        content_ui.allocate_ui_with_layout(
                            Vec2::new(text_width, THUMB_BOX_SIZE),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&toast.title)
                                            .strong()
                                            .size(13.0)
                                            .color(primary_text),
                                    )
                                    .truncate(),
                                );
                                ui.add_space(2.0);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&toast.subtitle)
                                            .size(11.5)
                                            .color(muted_text),
                                    )
                                    .truncate(),
                                );
                                ui.add_space(4.0);
                                let hint = if toast.target_url.is_some() {
                                    "Click to open URL"
                                } else if toast.target_path.is_some() {
                                    "Click to open file"
                                } else {
                                    "Right-click to dismiss"
                                };
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(hint).size(10.0).color(accent_color),
                                    )
                                    .truncate(),
                                );
                            },
                        );

                        content_ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                            let close_btn = egui::Button::new(
                                egui::RichText::new("x").size(11.0).color(muted_text),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(4))
                            .min_size(Vec2::splat(20.0));
                            if ui.add(close_btn).on_hover_text("Dismiss").clicked() {
                                close_clicked = true;
                            }
                        });

                        if close_clicked || card_response.secondary_clicked() {
                            dismissed = true;
                        } else if card_response.clicked() {
                            dismissed = true;
                            triggered_action = toast.click_action();
                        }
                    });
            });

            if dismissed {
                to_dismiss.push(toast.id);
                if action == ToastAction::None && triggered_action != ToastAction::None {
                    action = triggered_action;
                }
            }
        }

        if !to_dismiss.is_empty() {
            self.toasts.retain(|toast| !to_dismiss.contains(&toast.id));
        }

        action
    }
}

fn with_alpha(color: Color32, factor: f32) -> Color32 {
    let a = ((color.a() as f32) * factor.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

fn fit_aspect_rect(src: Vec2, bounds: Rect) -> Rect {
    if src.x <= 0.0 || src.y <= 0.0 {
        return bounds;
    }
    let scale = (bounds.width() / src.x).min(bounds.height() / src.y);
    let size = Vec2::new(src.x * scale, src.y * scale);
    Rect::from_center_size(bounds.center(), size)
}

#[cfg(target_os = "windows")]
fn primary_work_area(ctx: &egui::Context) -> (f32, f32, f32, f32) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETWORKAREA, SystemParametersInfoW};

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETWORKAREA, 0, std::ptr::from_mut(&mut rect).cast(), 0)
    };
    if ok != 0 && rect.right > rect.left && rect.bottom > rect.top {
        let ppp = ctx.pixels_per_point().max(1.0);
        return (
            rect.left as f32 / ppp,
            rect.top as f32 / ppp,
            (rect.right - rect.left) as f32 / ppp,
            (rect.bottom - rect.top) as f32 / ppp,
        );
    }

    fallback_work_area(ctx)
}

#[cfg(not(target_os = "windows"))]
fn primary_work_area(ctx: &egui::Context) -> (f32, f32, f32, f32) {
    fallback_work_area(ctx)
}

fn fallback_work_area(ctx: &egui::Context) -> (f32, f32, f32, f32) {
    if let Some(size) = ctx.input(|i| i.viewport().monitor_size) {
        return (0.0, 0.0, size.x, (size.y - 48.0).max(0.0));
    }
    (0.0, 0.0, 1280.0, 672.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_image(width: u32, height: u32) -> CapturedImage {
        let pixels = vec![200_u8; (width * height * 4) as usize];
        CapturedImage::from_rgba(width, height, pixels).expect("valid image")
    }

    #[test]
    fn compute_toast_rect_stacks_upward_from_bottom_right() {
        let (x0, y0) = compute_toast_rect(0, 1920.0, 1040.0, TOAST_WIDTH, TOAST_HEIGHT);
        let (x1, y1) = compute_toast_rect(1, 1920.0, 1040.0, TOAST_WIDTH, TOAST_HEIGHT);
        let (x2, y2) = compute_toast_rect(2, 1920.0, 1040.0, TOAST_WIDTH, TOAST_HEIGHT);

        assert_eq!(x0, 1920.0 - TOAST_WIDTH - TOAST_MARGIN);
        assert_eq!(x1, x0);
        assert_eq!(x2, x0);

        assert_eq!(y0, 1040.0 - TOAST_MARGIN - TOAST_HEIGHT);
        assert_eq!(y1, y0 - (TOAST_HEIGHT + TOAST_GAP));
        assert_eq!(y2, y1 - (TOAST_HEIGHT + TOAST_GAP));
        assert!(y2 < y1 && y1 < y0);
    }

    #[test]
    fn compute_toast_rect_clamps_to_work_area_bounds() {
        let (x_small, y_small) = compute_toast_rect(0, 200.0, 60.0, TOAST_WIDTH, TOAST_HEIGHT);
        assert_eq!(x_small, 0.0);
        assert_eq!(y_small, 0.0);

        let (x_high, y_high) = compute_toast_rect(50, 1920.0, 1040.0, TOAST_WIDTH, TOAST_HEIGHT);
        assert_eq!(x_high, 1920.0 - TOAST_WIDTH - TOAST_MARGIN);
        assert_eq!(y_high, 0.0);

        let (x_neg, y_neg) = compute_toast_rect(0, -100.0, -50.0, TOAST_WIDTH, TOAST_HEIGHT);
        assert_eq!(x_neg, 0.0);
        assert_eq!(y_neg, 0.0);
    }

    #[test]
    fn manager_tracks_caps_dismisses_and_prunes_expired_toasts() {
        let ctx = egui::Context::default();
        let mut manager = ToastManager::new();
        assert_eq!(manager.active_count(), 0);

        let img = sample_image(64, 48);
        let id1 = manager.push_capture(
            &ctx,
            "Capture 1",
            "saved.png",
            Some(&img),
            Some(PathBuf::from("saved.png")),
            None,
        );
        let id2 = manager.push_capture(
            &ctx,
            "Capture 2",
            "https://example.com/2",
            None,
            Some(PathBuf::from("saved2.png")),
            Some("https://example.com/2".to_string()),
        );
        let id3 = manager.push_capture(&ctx, "Capture 3", "copied", None, None, None);
        assert_eq!(manager.active_count(), 3);

        let id4 = manager.push_capture(&ctx, "Capture 4", "evicts oldest", None, None, None);
        assert_eq!(manager.active_count(), MAX_VISIBLE_TOASTS);
        assert!(!manager.dismiss(id1));
        assert!(manager.dismiss(id2));
        assert_eq!(manager.active_count(), 2);

        let created = manager.toasts[0].created_at;
        assert_eq!(manager.prune_expired(created + Duration::from_secs(2)), 0);
        assert_eq!(manager.active_count(), 2);

        assert_eq!(
            manager.prune_expired(created + DEFAULT_TOAST_DURATION + Duration::from_millis(1)),
            2
        );
        assert_eq!(manager.active_count(), 0);

        let _ = (id3, id4);
        let id5 = manager.push_capture(&ctx, "Capture 5", "dismiss all", None, None, None);
        assert!(id5 > id4);
        manager.dismiss_all();
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn click_action_prefers_url_then_path_then_none() {
        let now = Instant::now();
        let with_both = ToastNotification {
            id: 1,
            title: "Uploaded".into(),
            subtitle: "ok".into(),
            thumbnail: None,
            target_path: Some(PathBuf::from("shot.png")),
            target_url: Some("https://cdn.example/shot.png".into()),
            created_at: now,
            duration: DEFAULT_TOAST_DURATION,
        };
        assert_eq!(
            with_both.click_action(),
            ToastAction::OpenUrl("https://cdn.example/shot.png".into())
        );

        let path_only = ToastNotification {
            target_url: None,
            ..with_both
        };
        assert_eq!(
            path_only.click_action(),
            ToastAction::OpenPath(PathBuf::from("shot.png"))
        );

        let empty = ToastNotification {
            target_path: None,
            ..path_only
        };
        assert_eq!(empty.click_action(), ToastAction::None);
    }
}
