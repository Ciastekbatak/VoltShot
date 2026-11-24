use eframe::egui::{
    self, Color32, CornerRadius, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use sharex_capture::XcapCapturer;
use sharex_core::{Region, ScreenCapturer};
use std::time::{Duration, Instant};

const TITLE: &str = "ShareX Recording HUD";
const HUD_SIZE: Vec2 = Vec2::new(364.0, 40.0);
const GAP: f32 = 8.0;

/// Control selected in the floating recording HUD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingHudAction {
    None,
    Stop,
    Abort,
}

/// Positions the HUD in egui points for a region specified in physical pixels.
/// Without a region, returns an offset from the primary display's top center.
pub fn compute_hud_position(region: Option<Region>, hud_size: Vec2, ppp: f32) -> [f32; 2] {
    hud_position(region, hud_size, ppp, None)
}

fn region_rect(region: Region) -> Rect {
    Rect::from_min_size(
        Pos2::new(region.x() as f32, region.y() as f32),
        Vec2::new(region.width() as f32, region.height() as f32),
    )
}

fn hud_position(
    region: Option<Region>,
    hud_size: Vec2,
    ppp: f32,
    display: Option<Rect>,
) -> [f32; 2] {
    let Some(region) = region else {
        let anchor = display.map_or(Pos2::ZERO, |bounds| bounds.center_top());
        return [anchor.x / ppp - hud_size.x / 2.0, anchor.y / ppp + GAP];
    };

    let region = region_rect(region);
    let above = region.top() / ppp - hud_size.y - GAP;
    let below = region.bottom() / ppp + GAP;
    let top = display.map_or(0.0, |bounds| bounds.top() / ppp);
    let mut x = region.center().x / ppp - hud_size.x / 2.0;
    let mut y = if above >= top { above } else { below };
    if let Some(bounds) = display {
        let left = bounds.left() / ppp;
        let bottom = bounds.bottom() / ppp;
        x = x.clamp(left, (bounds.right() / ppp - hud_size.x).max(left));
        if y + hud_size.y > bottom {
            y = (region.top() / ppp + GAP).clamp(top, (bottom - hud_size.y).max(top));
        }
    }
    [x, y]
}

#[derive(Clone, Copy)]
struct DisplayCache {
    region: Option<Region>,
    bounds: Option<Rect>,
    checked_at: Instant,
}

fn display_bounds(ctx: &egui::Context, id: egui::Id, region: Option<Region>) -> Option<Rect> {
    let cached = ctx.data(|data| data.get_temp::<DisplayCache>(id));
    if let Some(cached) = cached
        && cached.region == region
        && cached.checked_at.elapsed() < Duration::from_secs(5)
    {
        return cached.bounds;
    }

    let mut primary = None;
    let mut best = None;
    let mut best_area = 0.0;
    let target = region.map(region_rect);
    if let Ok(displays) = XcapCapturer::new().list_monitors() {
        for display in displays {
            let bounds = Rect::from_min_size(
                Pos2::new(display.x as f32, display.y as f32),
                Vec2::new(display.width as f32, display.height as f32),
            );
            if display.is_primary || primary.is_none() {
                primary = Some(bounds);
            }
            if let Some(target) = target {
                let overlap = target.intersect(bounds);
                let area = overlap.width().max(0.0) * overlap.height().max(0.0);
                if area > best_area {
                    best = Some(bounds);
                    best_area = area;
                }
            }
        }
    }
    let bounds = if region.is_some() { best } else { primary };
    ctx.data_mut(|data| {
        data.insert_temp(
            id,
            DisplayCache {
                region,
                bounds,
                checked_at: Instant::now(),
            },
        );
    });
    bounds
}

/// Shows the floating recording controls and returns the action selected this frame.
pub fn show_recording_hud(
    ctx: &egui::Context,
    region: Option<Region>,
    elapsed: Duration,
    format_label: &str,
    encoder_label: &str,
) -> RecordingHudAction {
    let viewport_id = egui::ViewportId::from_hash_of(TITLE);
    let ppp = ctx.pixels_per_point();
    let display = display_bounds(ctx, viewport_id.0, region);
    let position = if display.is_none() && region.is_none() {
        let width = ctx.input(|input| {
            input
                .viewport()
                .monitor_size
                .map_or(HUD_SIZE.x, |size| size.x)
        });
        let [offset_x, offset_y] = compute_hud_position(None, HUD_SIZE, ppp);
        [width / 2.0 + offset_x, offset_y]
    } else {
        hud_position(region, HUD_SIZE, ppp, display)
    };
    let builder = egui::ViewportBuilder::default()
        .with_title(TITLE)
        .with_decorations(false)
        .with_always_on_top()
        .with_resizable(false)
        .with_active(false)
        .with_taskbar(false)
        .with_inner_size(HUD_SIZE)
        .with_position(position);

    ctx.request_repaint_after(Duration::from_millis(100));
    ctx.show_viewport_immediate(viewport_id, builder, |ctx, class| {
        if class == egui::ViewportClass::Embedded {
            return egui::Window::new(TITLE)
                .title_bar(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, GAP])
                .show(ctx, |ui| {
                    hud_controls(ui, elapsed, format_label, encoder_label)
                })
                .and_then(|response| response.inner)
                .unwrap_or(RecordingHudAction::None);
        }

        if ctx.input(|input| input.viewport().close_requested()) {
            return RecordingHudAction::Stop;
        }

        if (display.is_some() || region.is_some())
            && let Some(outer_rect) = ctx.input(|input| input.viewport().outer_rect)
        {
            let position = Pos2::from(hud_position(
                region,
                HUD_SIZE,
                ctx.pixels_per_point(),
                display,
            ));
            if outer_rect.min.distance(position) > 1.0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
            }
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(16, 19, 26))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(48, 56, 72)))
                    .inner_margin(Margin::symmetric(10, 6)),
            )
            .show(ctx, |ui| {
                hud_controls(ui, elapsed, format_label, encoder_label)
            })
            .inner
    })
}

fn hud_controls(
    ui: &mut egui::Ui,
    elapsed: Duration,
    format_label: &str,
    encoder_label: &str,
) -> RecordingHudAction {
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0_f32;
        let pulse = (elapsed.as_secs_f32() * std::f32::consts::TAU).cos();
        let red = Color32::from_rgba_unmultiplied(248, 81, 73, (185.0 + 70.0 * pulse) as u8);
        let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(10.0_f32), Sense::hover());
        ui.painter().circle_filled(dot_rect.center(), 4.2_f32, red);
        ui.label(
            RichText::new("REC")
                .size(11.5_f32)
                .strong()
                .color(Color32::from_rgb(248, 113, 113)),
        );
        ui.label(
            RichText::new(format_elapsed(elapsed))
                .size(12.5_f32)
                .monospace()
                .strong()
                .color(Color32::from_rgb(241, 245, 249)),
        );

        let label = if encoder_label.is_empty() {
            format_label.to_owned()
        } else {
            format!("{format_label} · {encoder_label}")
        };
        egui::Frame::NONE
            .fill(Color32::from_rgb(29, 34, 45))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(48, 56, 72)))
            .corner_radius(CornerRadius::same(5))
            .inner_margin(Margin::symmetric(7, 2))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(label)
                        .size(10.5_f32)
                        .color(Color32::from_rgb(186, 198, 216)),
                )
            });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0_f32;
            let abort = egui::Button::new(
                RichText::new("Abort")
                    .size(11.5_f32)
                    .strong()
                    .color(Color32::from_rgb(252, 165, 165)),
            )
            .fill(Color32::from_rgba_unmultiplied(220, 38, 38, 42))
            .stroke(Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(239, 68, 68, 120),
            ))
            .corner_radius(CornerRadius::same(5))
            .min_size(Vec2::new(52.0_f32, 24.0_f32));
            let stop = egui::Button::new(
                RichText::new("Stop")
                    .size(11.5_f32)
                    .strong()
                    .color(Color32::WHITE),
            )
            .fill(Color32::from_rgb(22, 163, 74))
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(5))
            .min_size(Vec2::new(48.0_f32, 24.0_f32));

            let abort_clicked = ui.add(abort).clicked();
            let stop_clicked = ui.add(stop).clicked();
            if stop_clicked {
                RecordingHudAction::Stop
            } else if abort_clicked {
                RecordingHudAction::Abort
            } else {
                RecordingHudAction::None
            }
        })
        .inner
    })
    .inner
}

/// Formats recording duration as minutes, seconds, and tenths of a second.
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

    const SIZE: Vec2 = Vec2::new(200.0, 40.0);

    #[test]
    fn fullscreen_anchors_to_the_primary_display_top_center() {
        assert_eq!(compute_hud_position(None, SIZE, 2.0), [-100.0, 8.0]);
        let primary = Rect::from_min_size(Pos2::ZERO, Vec2::new(2560.0, 1440.0));
        assert_eq!(hud_position(None, SIZE, 2.0, Some(primary)), [540.0, 8.0]);
    }

    #[test]
    fn hud_centers_above_a_region_at_scaled_coordinates() {
        let region = Region::new(400, 200, 800, 300).expect("valid region");
        assert_eq!(compute_hud_position(Some(region), SIZE, 2.0), [300.0, 52.0]);
    }

    #[test]
    fn hud_flips_below_a_region_near_the_top_edge() {
        let region = Region::new(400, 24, 800, 300).expect("valid region");
        assert_eq!(
            compute_hud_position(Some(region), SIZE, 2.0),
            [300.0, 170.0]
        );
    }

    #[test]
    fn hud_keeps_negative_x_on_a_left_monitor() {
        let region = Region::new(-1000, 200, 400, 300).expect("valid region");
        assert_eq!(
            compute_hud_position(Some(region), SIZE, 2.0),
            [-500.0, 52.0]
        );
    }
}
