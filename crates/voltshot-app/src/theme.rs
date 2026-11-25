use eframe::egui::{
    self, Color32, CornerRadius, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2, Visuals,
};

/// VoltShot minimalist color palette for dark and light themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoltShotPalette {
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

/// Returns the VoltShot dark or light color palette.
pub fn palette(dark: bool) -> VoltShotPalette {
    if dark {
        VoltShotPalette {
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
        VoltShotPalette {
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

/// Applies the sleek minimalist VoltShot visual theme to an `egui::Context`.
pub fn apply_voltshot_theme(ctx: &egui::Context, dark: bool) {
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

/// Paints a vector VoltShot rounded-square brand mark with viewfinder corners and a lightning bolt.
pub fn paint_brand_mark(ui: &mut egui::Ui, size: f32, _p: &VoltShotPalette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_brand_mark_in_rect(ui.painter(), rect, 1.0_f32);
}

/// Paints the VoltShot brand mark inside `rect` with an optional opacity multiplier.
pub fn paint_brand_mark_in_rect(painter: &egui::Painter, rect: Rect, alpha: f32) {
    let alpha = alpha.clamp(0.0_f32, 1.0_f32);
    let with_a = |r: u8, g: u8, b: u8, base_a: f32| {
        Color32::from_rgba_unmultiplied(r, g, b, (base_a * alpha * 255.0_f32).round() as u8)
    };
    let size = rect.width().min(rect.height());
    let corner = (size * (28.0_f32 / 128.0_f32))
        .round()
        .clamp(3.0_f32, 16.0_f32) as u8;

    let bg_rect = rect.shrink(size * (4.0_f32 / 128.0_f32));
    painter.rect_filled(bg_rect, CornerRadius::same(corner), with_a(19, 30, 63, 1.0));
    painter.rect_stroke(
        bg_rect,
        CornerRadius::same(corner),
        Stroke::new(
            (size * (3.0_f32 / 128.0_f32)).max(1.0_f32),
            with_a(56, 189, 248, 0.35),
        ),
        egui::StrokeKind::Inside,
    );

    let map = |nx: f32, ny: f32| {
        Pos2::new(
            rect.min.x + rect.width() * (nx / 128.0_f32),
            rect.min.y + rect.height() * (ny / 128.0_f32),
        )
    };
    let frame_stroke = Stroke::new(
        (size * (7.0_f32 / 128.0_f32)).max(1.25_f32),
        with_a(96, 202, 250, 1.0),
    );
    let corners: [[(f32, f32); 3]; 4] = [
        [(26.0, 46.0), (26.0, 26.0), (46.0, 26.0)],
        [(82.0, 26.0), (102.0, 26.0), (102.0, 46.0)],
        [(102.0, 82.0), (102.0, 102.0), (82.0, 102.0)],
        [(46.0, 102.0), (26.0, 102.0), (26.0, 82.0)],
    ];
    for pts in corners {
        painter.add(egui::Shape::line(
            pts.into_iter().map(|(x, y)| map(x, y)).collect(),
            frame_stroke,
        ));
    }

    let bolt_top = with_a(56, 189, 248, 1.0);
    let bolt_bot = with_a(14, 165, 233, 1.0);
    let v0 = map(70.0, 28.0);
    let v1 = map(43.0, 68.0);
    let v2 = map(61.0, 68.0);
    let v3 = map(56.0, 100.0);
    let v4 = map(85.0, 58.0);
    let v5 = map(67.0, 58.0);

    painter.add(egui::Shape::convex_polygon(
        vec![v0, v1, v2, v5],
        bolt_top,
        Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![v5, v2, v3, v4],
        bolt_bot,
        Stroke::NONE,
    ));
}

/// Returns the standard elevated surface card frame.
pub fn card_frame(p: &VoltShotPalette) -> egui::Frame {
    egui::Frame::NONE
        .fill(p.card_bg)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(14))
}

/// Renders a subtle uppercase muted section heading with clean breathing room.
pub fn section_label(ui: &mut egui::Ui, text: &str, p: &VoltShotPalette) {
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
pub fn view_heading(ui: &mut egui::Ui, text: &str, p: &VoltShotPalette) {
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
pub fn hotkey_badge(ui: &mut egui::Ui, text: &str, p: &VoltShotPalette) {
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
pub fn primary_cta<'a>(label: &'a str, p: &VoltShotPalette) -> egui::Button<'a> {
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
    fn apply_voltshot_theme_sets_floating_scrollbars_and_rounded_widgets() {
        let ctx = egui::Context::default();
        apply_voltshot_theme(&ctx, true);
        let style = ctx.style();
        assert_eq!(
            style.visuals.widgets.inactive.corner_radius,
            CornerRadius::same(6)
        );
        assert!(style.spacing.scroll.floating);
        assert_eq!(style.spacing.window_margin, Margin::same(12));
    }
}
