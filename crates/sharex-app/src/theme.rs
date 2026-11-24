use eframe::egui::{
    self, Color32, CornerRadius, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2, Visuals,
};

/// ShareX minimalist color palette for dark and light themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharexPalette {
    pub sidebar_bg: Color32,
    pub panel_bg: Color32,
    pub card_bg: Color32,
    pub surface_elevated: Color32,
    pub hover_bg: Color32,
    pub border: Color32,
    pub divider: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_subtle: Color32,
    pub text_primary: Color32,
    pub text_muted: Color32,
    pub success: Color32,
    pub danger: Color32,
    pub warning: Color32,
}

/// Returns the ShareX dark or light color palette.
pub fn palette(dark: bool) -> SharexPalette {
    if dark {
        SharexPalette {
            sidebar_bg: Color32::from_rgb(17, 19, 24),
            panel_bg: Color32::from_rgb(22, 25, 32),
            card_bg: Color32::from_rgb(29, 33, 43),
            surface_elevated: Color32::from_rgb(34, 39, 51),
            hover_bg: Color32::from_rgb(38, 44, 57),
            border: Color32::from_rgb(42, 49, 64),
            divider: Color32::from_rgb(33, 38, 50),
            accent: Color32::from_rgb(59, 130, 246),
            accent_hover: Color32::from_rgb(96, 165, 250),
            accent_subtle: Color32::from_rgba_unmultiplied(59, 130, 246, 36),
            text_primary: Color32::from_rgb(241, 245, 249),
            text_muted: Color32::from_rgb(139, 149, 167),
            success: Color32::from_rgb(34, 197, 94),
            danger: Color32::from_rgb(239, 68, 68),
            warning: Color32::from_rgb(245, 158, 11),
        }
    } else {
        SharexPalette {
            sidebar_bg: Color32::from_rgb(244, 246, 250),
            panel_bg: Color32::from_rgb(250, 251, 253),
            card_bg: Color32::from_rgb(255, 255, 255),
            surface_elevated: Color32::from_rgb(248, 250, 252),
            hover_bg: Color32::from_rgb(236, 240, 248),
            border: Color32::from_rgb(226, 232, 240),
            divider: Color32::from_rgb(234, 238, 245),
            accent: Color32::from_rgb(37, 99, 235),
            accent_hover: Color32::from_rgb(59, 130, 246),
            accent_subtle: Color32::from_rgba_unmultiplied(37, 99, 235, 24),
            text_primary: Color32::from_rgb(15, 23, 42),
            text_muted: Color32::from_rgb(100, 116, 139),
            success: Color32::from_rgb(22, 163, 74),
            danger: Color32::from_rgb(220, 38, 38),
            warning: Color32::from_rgb(217, 119, 6),
        }
    }
}

/// Applies the sleek minimalist ShareX visual theme to an `egui::Context`.
pub fn apply_sharex_theme(ctx: &egui::Context, dark: bool) {
    let p = palette(dark);
    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    visuals.window_fill = p.panel_bg;
    visuals.panel_fill = p.panel_bg;
    visuals.faint_bg_color = p.surface_elevated;
    visuals.extreme_bg_color = if dark {
        Color32::from_rgb(19, 22, 29)
    } else {
        Color32::from_rgb(241, 245, 249)
    };
    visuals.override_text_color = Some(p.text_primary);
    visuals.selection.bg_fill = p.accent;
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.window_stroke = Stroke::new(1.0_f32, p.border);

    let radius = CornerRadius::same(6);

    visuals.widgets.noninteractive.bg_fill = p.card_bg;
    visuals.widgets.noninteractive.weak_bg_fill = p.card_bg;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.divider);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text_primary);
    visuals.widgets.noninteractive.corner_radius = radius;

    visuals.widgets.inactive.bg_fill = p.card_bg;
    visuals.widgets.inactive.weak_bg_fill = p.surface_elevated;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text_primary);
    visuals.widgets.inactive.corner_radius = radius;

    visuals.widgets.hovered.bg_fill = p.hover_bg;
    visuals.widgets.hovered.weak_bg_fill = p.hover_bg;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text_primary);
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.hovered.expansion = 0.0_f32;

    visuals.widgets.active.bg_fill = p.accent;
    visuals.widgets.active.weak_bg_fill = p.accent;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.accent_hover);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.active.expansion = 0.0_f32;

    visuals.widgets.open.bg_fill = p.hover_bg;
    visuals.widgets.open.weak_bg_fill = p.hover_bg;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, p.accent);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, p.text_primary);
    visuals.widgets.open.corner_radius = radius;

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(11.0, 6.0);
    style.spacing.window_margin = Margin::same(12);
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    ctx.set_style(style);
}

/// Paints a vector ShareX rounded-square brand mark with viewfinder corners and a geometric `S`.
pub fn paint_brand_mark(ui: &mut egui::Ui, size: f32, p: &SharexPalette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    let corner = (size * 0.25_f32).round().clamp(3.0_f32, 12.0_f32) as u8;

    painter.rect_filled(rect, CornerRadius::same(corner), p.accent);
    painter.rect_stroke(
        rect,
        CornerRadius::same(corner),
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 36)),
        egui::StrokeKind::Inside,
    );

    let inset = size * 0.17_f32;
    let arm = size * 0.16_f32;
    let inner = Rect::from_min_max(
        Pos2::new(rect.min.x + inset, rect.min.y + inset),
        Pos2::new(rect.max.x - inset, rect.max.y - inset),
    );
    let corner_stroke = Stroke::new(
        (size * 0.065_f32).max(1.1_f32),
        Color32::from_rgba_unmultiplied(255, 255, 255, 165),
    );

    painter.line_segment(
        [inner.left_top(), Pos2::new(inner.min.x + arm, inner.min.y)],
        corner_stroke,
    );
    painter.line_segment(
        [inner.left_top(), Pos2::new(inner.min.x, inner.min.y + arm)],
        corner_stroke,
    );
    painter.line_segment(
        [inner.right_top(), Pos2::new(inner.max.x - arm, inner.min.y)],
        corner_stroke,
    );
    painter.line_segment(
        [inner.right_top(), Pos2::new(inner.max.x, inner.min.y + arm)],
        corner_stroke,
    );
    painter.line_segment(
        [
            inner.left_bottom(),
            Pos2::new(inner.min.x + arm, inner.max.y),
        ],
        corner_stroke,
    );
    painter.line_segment(
        [
            inner.left_bottom(),
            Pos2::new(inner.min.x, inner.max.y - arm),
        ],
        corner_stroke,
    );
    painter.line_segment(
        [
            inner.right_bottom(),
            Pos2::new(inner.max.x - arm, inner.max.y),
        ],
        corner_stroke,
    );
    painter.line_segment(
        [
            inner.right_bottom(),
            Pos2::new(inner.max.x, inner.max.y - arm),
        ],
        corner_stroke,
    );

    let sx0 = rect.center().x - size * 0.16_f32;
    let sx1 = rect.center().x + size * 0.16_f32;
    let sy_top = rect.center().y - size * 0.21_f32;
    let sy_mid = rect.center().y;
    let sy_bot = rect.center().y + size * 0.21_f32;
    let glyph_stroke = Stroke::new((size * 0.105_f32).max(1.6_f32), Color32::WHITE);

    painter.add(egui::Shape::line(
        vec![
            Pos2::new(sx1, sy_top),
            Pos2::new(sx0, sy_top),
            Pos2::new(sx0, sy_mid),
            Pos2::new(sx1, sy_mid),
            Pos2::new(sx1, sy_bot),
            Pos2::new(sx0, sy_bot),
        ],
        glyph_stroke,
    ));
}

/// Returns the standard elevated surface card frame.
pub fn card_frame(p: &SharexPalette) -> egui::Frame {
    egui::Frame::NONE
        .fill(p.card_bg)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(14))
}

/// Renders a subtle uppercase muted section heading with clean breathing room.
pub fn section_label(ui: &mut egui::Ui, text: &str, p: &SharexPalette) {
    ui.add_space(2.0_f32);
    ui.label(
        RichText::new(text.to_uppercase())
            .size(10.5_f32)
            .strong()
            .color(p.text_muted),
    );
    ui.add_space(2.0_f32);
}

/// Renders a consistent primary view heading.
pub fn view_heading(ui: &mut egui::Ui, text: &str, p: &SharexPalette) {
    ui.label(
        RichText::new(text)
            .size(20.0_f32)
            .strong()
            .color(p.text_primary),
    );
}

/// Renders a smooth 7px antialiased status dot.
pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(7.0_f32), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5_f32, color);
}

/// Renders a compact monospace keycap badge.
pub fn hotkey_badge(ui: &mut egui::Ui, text: &str, p: &SharexPalette) {
    egui::Frame::NONE
        .fill(p.sidebar_bg)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .monospace()
                    .size(10.5_f32)
                    .color(p.text_muted),
            );
        });
}

/// Constructs a rounded primary accent call-to-action button.
pub fn primary_cta<'a>(label: &'a str, p: &SharexPalette) -> egui::Button<'a> {
    egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
        .fill(p.accent)
        .stroke(Stroke::new(1.0_f32, p.accent))
        .corner_radius(CornerRadius::same(6))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_and_light_palettes_have_distinct_surfaces_and_accent() {
        let dark = palette(true);
        let light = palette(false);
        assert_eq!(dark.sidebar_bg, Color32::from_rgb(17, 19, 24));
        assert_eq!(dark.panel_bg, Color32::from_rgb(22, 25, 32));
        assert_eq!(dark.card_bg, Color32::from_rgb(29, 33, 43));
        assert_eq!(dark.accent, Color32::from_rgb(59, 130, 246));
        assert_eq!(light.sidebar_bg, Color32::from_rgb(244, 246, 250));
        assert_eq!(light.card_bg, Color32::from_rgb(255, 255, 255));
        assert_ne!(dark.panel_bg, light.panel_bg);
        assert_ne!(dark.accent_subtle, Color32::TRANSPARENT);
    }

    #[test]
    fn apply_sharex_theme_sets_floating_scrollbars_and_rounded_widgets() {
        let ctx = egui::Context::default();
        apply_sharex_theme(&ctx, true);
        let style = ctx.style();
        assert_eq!(
            style.visuals.widgets.inactive.corner_radius,
            CornerRadius::same(6)
        );
        assert!(style.spacing.scroll.floating);
        assert_eq!(style.spacing.window_margin, Margin::same(12));
    }
}
