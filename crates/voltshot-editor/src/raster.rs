use crate::font::{TextFont, glyph_5x7};
use crate::model::{AnnotationKind, EditorDocument, Point, RectF, RgbaColor};
use voltshot_core::{CapturedImage, CoreError};

struct Canvas {
    width: i32,
    height: i32,
    rgba: Vec<u8>,
}

impl Canvas {
    fn from_image(image: &CapturedImage) -> Self {
        Self {
            width: image.width() as i32,
            height: image.height() as i32,
            rgba: image.rgba().to_vec(),
        }
    }

    fn blend_pixel(&mut self, x: i32, y: i32, color: RgbaColor) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height || color.a == 0 {
            return;
        }
        let idx = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        if color.a == 255 {
            self.rgba[idx] = color.r;
            self.rgba[idx + 1] = color.g;
            self.rgba[idx + 2] = color.b;
            self.rgba[idx + 3] = 255;
            return;
        }
        let alpha = f32::from(color.a) / 255.0;
        let remaining = f32::from(self.rgba[idx + 3]) / 255.0 * (1.0 - alpha);
        let combined = alpha + remaining;
        for (channel, source) in self.rgba[idx..idx + 3]
            .iter_mut()
            .zip([color.r, color.g, color.b])
        {
            *channel = ((f32::from(source) * alpha + f32::from(*channel) * remaining) / combined)
                .round() as u8;
        }
        self.rgba[idx + 3] = (combined * 255.0).round() as u8;
    }

    fn fill_rect(&mut self, rect: RectF, color: RgbaColor) {
        let x0 = (rect.x.floor() as i32).max(0);
        let y0 = (rect.y.floor() as i32).max(0);
        let x1 = (rect.right().ceil() as i32).min(self.width);
        let y1 = (rect.bottom().ceil() as i32).min(self.height);
        for y in y0..y1 {
            for x in x0..x1 {
                self.blend_pixel(x, y, color);
            }
        }
    }

    fn draw_image_overlay(
        &mut self,
        rect: RectF,
        width: u32,
        height: u32,
        rgba: &[u8],
        opacity: f32,
    ) {
        let (width, height) = (width as usize, height as usize);
        let expected = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4));
        if width == 0
            || height == 0
            || expected != Some(rgba.len())
            || !rect.is_valid()
            || rect.width <= 0.0
            || rect.height <= 0.0
            || !opacity.is_finite()
            || opacity <= 0.0
        {
            return;
        }
        let opacity = opacity.min(1.0);
        let left = ((rect.x - 0.5).ceil() as i32).max(0);
        let right = ((rect.right() - 0.5).ceil() as i32).min(self.width);
        let top = ((rect.y - 0.5).ceil() as i32).max(0);
        let bottom = ((rect.bottom() - 0.5).ceil() as i32).min(self.height);
        for y in top..bottom {
            let src_y = (((y as f32 + 0.5 - rect.y) / rect.height * height as f32) as usize)
                .min(height - 1);
            for x in left..right {
                let src_x = (((x as f32 + 0.5 - rect.x) / rect.width * width as f32) as usize)
                    .min(width - 1);
                let index = (src_y * width + src_x) * 4;
                let alpha = (f32::from(rgba[index + 3]) * opacity).round() as u8;
                self.blend_pixel(
                    x,
                    y,
                    RgbaColor::rgba(rgba[index], rgba[index + 1], rgba[index + 2], alpha),
                );
            }
        }
    }

    fn draw_smart_eraser(
        &mut self,
        points: &[Point],
        stroke_width: f32,
        sample_color: Option<RgbaColor>,
    ) {
        let Some(first) = points.first() else {
            return;
        };
        if !stroke_width.is_finite()
            || stroke_width <= 0.0
            || points.iter().any(|point| !point.is_finite())
            || self.width <= 0
            || self.height <= 0
        {
            return;
        }
        let color = sample_color.unwrap_or_else(|| {
            let x = (first.x.floor() as i32).clamp(0, self.width - 1) as usize;
            let y = (first.y.floor() as i32).clamp(0, self.height - 1) as usize;
            let index = (y * self.width as usize + x) * 4;
            if self.rgba[index + 3] == 0 {
                RgbaColor::BLACK
            } else {
                RgbaColor::rgb(self.rgba[index], self.rgba[index + 1], self.rgba[index + 2])
            }
        });
        self.draw_polyline(points, color.with_alpha(255), stroke_width.max(1.5));
    }

    fn stroke_rect(&mut self, rect: RectF, color: RgbaColor, stroke_width: f32) {
        let thickness = stroke_width.max(1.0);
        let top = RectF {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: thickness.min(rect.height),
        };
        let bottom = RectF {
            x: rect.x,
            y: (rect.bottom() - thickness).max(rect.y),
            width: rect.width,
            height: thickness.min(rect.height),
        };
        let left = RectF {
            x: rect.x,
            y: rect.y,
            width: thickness.min(rect.width),
            height: rect.height,
        };
        let right = RectF {
            x: (rect.right() - thickness).max(rect.x),
            y: rect.y,
            width: thickness.min(rect.width),
            height: rect.height,
        };
        self.fill_rect(top, color);
        self.fill_rect(bottom, color);
        self.fill_rect(left, color);
        self.fill_rect(right, color);
    }

    fn fill_circle(&mut self, center: Point, radius: f32, color: RgbaColor) {
        let r = radius.max(0.5);
        let r2 = r * r;
        let x0 = ((center.x - r).floor() as i32).max(0);
        let y0 = ((center.y - r).floor() as i32).max(0);
        let x1 = ((center.x + r).ceil() as i32).min(self.width - 1);
        let y1 = ((center.y + r).ceil() as i32).min(self.height - 1);
        for y in y0..=y1 {
            let dy = (y as f32 + 0.5) - center.y;
            for x in x0..=x1 {
                let dx = (x as f32 + 0.5) - center.x;
                if dx * dx + dy * dy <= r2 {
                    self.blend_pixel(x, y, color);
                }
            }
        }
    }

    fn stroke_ellipse(&mut self, rect: RectF, color: RgbaColor, stroke_width: f32) {
        let rx = (rect.width * 0.5).max(1.0);
        let ry = (rect.height * 0.5).max(1.0);
        let cx = rect.x + rx;
        let cy = rect.y + ry;
        let half_t = (stroke_width * 0.5).max(0.75);

        let x0 = ((rect.x - half_t).floor() as i32).max(0);
        let y0 = ((rect.y - half_t).floor() as i32).max(0);
        let x1 = ((rect.right() + half_t).ceil() as i32).min(self.width - 1);
        let y1 = ((rect.bottom() + half_t).ceil() as i32).min(self.height - 1);

        let rx_out = rx + half_t;
        let ry_out = ry + half_t;
        let rx_in = (rx - half_t).max(0.0);
        let ry_in = (ry - half_t).max(0.0);

        for y in y0..=y1 {
            let dy = (y as f32 + 0.5) - cy;
            for x in x0..=x1 {
                let dx = (x as f32 + 0.5) - cx;
                let outer = (dx * dx) / (rx_out * rx_out) + (dy * dy) / (ry_out * ry_out);
                if outer > 1.0 {
                    continue;
                }
                let inside_hole = if rx_in > 0.0 && ry_in > 0.0 {
                    (dx * dx) / (rx_in * rx_in) + (dy * dy) / (ry_in * ry_in) < 1.0
                } else {
                    false
                };
                if !inside_hole {
                    self.blend_pixel(x, y, color);
                }
            }
        }
    }

    fn stroke_segment(&mut self, a: Point, b: Point, color: RgbaColor, stroke_width: f32) {
        let radius = (stroke_width * 0.5).max(0.75);
        let r2 = radius * radius;
        let x0 = ((a.x.min(b.x) - radius).floor() as i32).max(0);
        let y0 = ((a.y.min(b.y) - radius).floor() as i32).max(0);
        let x1 = ((a.x.max(b.x) + radius).ceil() as i32).min(self.width - 1);
        let y1 = ((a.y.max(b.y) + radius).ceil() as i32).min(self.height - 1);

        let vx = b.x - a.x;
        let vy = b.y - a.y;
        let len2 = vx * vx + vy * vy;

        for y in y0..=y1 {
            let py = y as f32 + 0.5;
            for x in x0..=x1 {
                let px = x as f32 + 0.5;
                let t = if len2 > 1e-6 {
                    (((px - a.x) * vx + (py - a.y) * vy) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let cx = a.x + t * vx;
                let cy = a.y + t * vy;
                let dx = px - cx;
                let dy = py - cy;
                if dx * dx + dy * dy <= r2 {
                    self.blend_pixel(x, y, color);
                }
            }
        }
    }

    fn fill_triangle(&mut self, p0: Point, p1: Point, p2: Point, color: RgbaColor) {
        let x0 = (p0.x.min(p1.x).min(p2.x).floor() as i32).max(0);
        let y0 = (p0.y.min(p1.y).min(p2.y).floor() as i32).max(0);
        let x1 = (p0.x.max(p1.x).max(p2.x).ceil() as i32).min(self.width - 1);
        let y1 = (p0.y.max(p1.y).max(p2.y).ceil() as i32).min(self.height - 1);

        fn edge(a: Point, b: Point, p: Point) -> f32 {
            (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x)
        }

        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = Point::new(x as f32 + 0.5, y as f32 + 0.5);
                let e0 = edge(p0, p1, p);
                let e1 = edge(p1, p2, p);
                let e2 = edge(p2, p0, p);
                let has_neg = (e0 < 0.0) || (e1 < 0.0) || (e2 < 0.0);
                let has_pos = (e0 > 0.0) || (e1 > 0.0) || (e2 > 0.0);
                if !(has_neg && has_pos) {
                    self.blend_pixel(x, y, color);
                }
            }
        }
    }

    fn draw_arrow(&mut self, start: Point, end: Point, color: RgbaColor, stroke_width: f32) {
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1.0 {
            self.fill_circle(start, stroke_width * 0.5, color);
            return;
        }
        let ux = dx / len;
        let uy = dy / len;
        let head_len = (stroke_width * 3.5).max(14.0).min(len * 0.65);
        let head_half_w = head_len * 0.55;
        let base = Point::new(end.x - ux * head_len, end.y - uy * head_len);
        let left = Point::new(base.x - uy * head_half_w, base.y + ux * head_half_w);
        let right = Point::new(base.x + uy * head_half_w, base.y - ux * head_half_w);

        self.stroke_segment(start, base, color, stroke_width);
        self.fill_triangle(end, left, right, color);
    }

    fn draw_polyline(&mut self, points: &[Point], color: RgbaColor, stroke_width: f32) {
        match points {
            [] => {}
            [only] => self.fill_circle(*only, stroke_width * 0.5, color),
            many => {
                for pair in many.windows(2) {
                    self.stroke_segment(pair[0], pair[1], color, stroke_width);
                }
            }
        }
    }

    fn draw_text_line(&mut self, origin: Point, text: &str, color: RgbaColor, font_size: f32) {
        let scale = (font_size / 7.0).round().max(1.0);
        let mut cursor_x = origin.x.round();
        let start_y = origin.y.round();

        for ch in text.chars() {
            if cursor_x >= self.width as f32 {
                break;
            }
            let rows = glyph_5x7(ch);
            for (row_idx, bits) in rows.iter().enumerate() {
                for col_idx in 0..5 {
                    if (bits >> (4 - col_idx)) & 1 == 1 {
                        self.fill_rect(
                            RectF {
                                x: cursor_x + col_idx as f32 * scale,
                                y: start_y + row_idx as f32 * scale,
                                width: scale,
                                height: scale,
                            },
                            color,
                        );
                    }
                }
            }
            cursor_x += 6.0 * scale;
        }
    }

    fn draw_text_label(
        &mut self,
        origin: Point,
        text: &str,
        color: RgbaColor,
        size: f32,
        background: Option<RgbaColor>,
        font: &TextFont<'_>,
    ) {
        if !origin.is_finite() || !size.is_finite() {
            return;
        }
        let layout = font.layout(origin, text, size);
        if let Some(background) = background {
            let (width, height) = layout.as_ref().map_or_else(
                || {
                    let scale = (size / 7.0).round().max(1.0);
                    (
                        text.chars().count().max(1) as f32 * 6.0 * scale,
                        7.0 * scale,
                    )
                },
                |(width, height, _)| (*width, *height),
            );
            self.fill_rect(
                RectF {
                    x: origin.x - 5.0,
                    y: origin.y - 5.0,
                    width: width + 10.0,
                    height: height + 10.0,
                },
                background,
            );
        }
        if let Some((_, _, glyphs)) = layout {
            for glyph in glyphs {
                let bounds = glyph.px_bounds();
                if bounds.max.x <= 0.0
                    || bounds.max.y <= 0.0
                    || bounds.min.x >= self.width as f32
                    || bounds.min.y >= self.height as f32
                {
                    continue;
                }
                glyph.draw(|x, y, coverage| {
                    let alpha = (coverage * f32::from(color.a)).round() as u8;
                    self.blend_pixel(
                        bounds.min.x as i32 + x as i32,
                        bounds.min.y as i32 + y as i32,
                        color.with_alpha(alpha),
                    );
                });
            }
        } else {
            self.draw_text_line(origin, text, color, size);
        }
    }

    fn draw_balloon(&mut self, rect: RectF, tail: Point, fill: RgbaColor, border: RgbaColor) {
        let (outline, _) = speech_balloon_outline(rect, tail);
        let edges = || {
            outline
                .iter()
                .copied()
                .zip(outline.iter().copied().cycle().skip(1))
        };
        let top = rect.y.min(tail.y).floor().max(0.0) as i32;
        let bottom = rect.bottom().max(tail.y).ceil().min(self.height as f32) as i32;
        let mut crossings = Vec::with_capacity(4);
        for y in top..bottom {
            let py = y as f32 + 0.5;
            crossings.clear();
            for (a, b) in edges() {
                if (a.y > py) != (b.y > py) {
                    crossings.push(a.x + (b.x - a.x) * ((py - a.y) / (b.y - a.y)));
                }
            }
            crossings.sort_unstable_by(f32::total_cmp);
            for pair in crossings.chunks_exact(2) {
                let left = (pair[0] - 0.5).ceil().max(0.0) as i32;
                let right = (pair[1] - 0.5).ceil().min(self.width as f32) as i32;
                for x in left..right {
                    self.blend_pixel(x, y, fill);
                }
            }
        }
        for (a, b) in edges() {
            self.stroke_segment(a, b, border, 2.0);
        }
    }

    fn draw_balloon_text(&mut self, rect: RectF, text: &str, color: RgbaColor, font_size: f32) {
        let padding = 6.0;
        let inner_height = rect.height - 2.0 * padding;
        let max_scale = (inner_height / 7.0).floor();
        if !max_scale.is_finite() || max_scale < 1.0 {
            return;
        }
        let scale = (font_size / 7.0).round().clamp(1.0, max_scale);
        let max_chars = ((rect.width - 2.0 * padding + scale) / (6.0 * scale)) as usize;
        let line_height = 8.0 * scale;
        let lines = text
            .lines()
            .take(((inner_height + scale) / line_height) as usize);
        let text_height = lines.clone().count() as f32 * line_height - scale;
        let top = rect.y + (rect.height - text_height) * 0.5;
        for (index, line) in lines.enumerate() {
            let line: String = line.chars().take(max_chars).collect();
            let origin = Point::new(rect.x + padding, top + index as f32 * line_height);
            self.draw_text_line(origin, &line, color, 7.0 * scale);
        }
    }

    fn apply_magnifier(&mut self, center: Point, radius: f32, zoom: f32, border: RgbaColor) {
        if radius <= 0.0 || !radius.is_finite() {
            return;
        }
        let zoom = zoom.max(1.0);
        let border_radius = (radius - 2.0).max(0.0);
        let snapshot = self.rgba.clone();
        let x0 = (center.x - radius).floor().max(0.0) as i32;
        let y0 = (center.y - radius).floor().max(0.0) as i32;
        let x1 = (center.x + radius).ceil().min(self.width as f32) as i32;
        let y1 = (center.y + radius).ceil().min(self.height as f32) as i32;
        for y in y0..y1 {
            let dy = y as f32 + 0.5 - center.y;
            let sy = ((center.y + dy / zoom).floor() as i32).clamp(0, self.height - 1);
            for x in x0..x1 {
                let dx = x as f32 + 0.5 - center.x;
                let distance_squared = dx * dx + dy * dy;
                if distance_squared > radius * radius {
                    continue;
                }
                let sx = ((center.x + dx / zoom).floor() as i32).clamp(0, self.width - 1);
                let src = (sy as usize * self.width as usize + sx as usize) * 4;
                let dst = (y as usize * self.width as usize + x as usize) * 4;
                self.rgba[dst..dst + 4].copy_from_slice(&snapshot[src..src + 4]);
                if distance_squared >= border_radius * border_radius {
                    self.blend_pixel(x, y, border);
                }
            }
        }
    }

    fn apply_spotlight(&mut self, rect: RectF, dim_alpha: u8) {
        if dim_alpha == 0 {
            return;
        }
        let brightness = u16::from(255 - dim_alpha);
        for (y, row) in self
            .rgba
            .chunks_exact_mut(self.width as usize * 4)
            .enumerate()
        {
            for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
                if !rect.contains(Point::new(x as f32 + 0.5, y as f32 + 0.5), 0.0) {
                    for channel in &mut pixel[..3] {
                        *channel = ((u16::from(*channel) * brightness + 127) / 255) as u8;
                    }
                }
            }
        }
    }

    fn apply_pixelate(&mut self, rect: RectF, block_size: u32) {
        let step = block_size.clamp(2, i32::MAX as u32) as i32;
        let x0 = (rect.x.floor() as i32).max(0);
        let y0 = (rect.y.floor() as i32).max(0);
        let x1 = (rect.right().ceil() as i32).min(self.width);
        let y1 = (rect.bottom().ceil() as i32).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        let mut by = y0;
        while by < y1 {
            let block_y1 = by.saturating_add(step).min(y1);
            let mut bx = x0;
            while bx < x1 {
                let block_x1 = bx.saturating_add(step).min(x1);
                let mut r_sum: u64 = 0;
                let mut g_sum: u64 = 0;
                let mut b_sum: u64 = 0;
                let mut count: u64 = 0;

                for y in by..block_y1 {
                    for x in bx..block_x1 {
                        let idx = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
                        r_sum += u64::from(self.rgba[idx]);
                        g_sum += u64::from(self.rgba[idx + 1]);
                        b_sum += u64::from(self.rgba[idx + 2]);
                        count += 1;
                    }
                }

                if let Some(avg_r) = r_sum.checked_div(count) {
                    let avg = RgbaColor::rgb(
                        avg_r as u8,
                        g_sum.checked_div(count).unwrap_or(0) as u8,
                        b_sum.checked_div(count).unwrap_or(0) as u8,
                    );
                    for y in by..block_y1 {
                        for x in bx..block_x1 {
                            self.blend_pixel(x, y, avg);
                        }
                    }
                }

                bx = block_x1;
            }
            by = block_y1;
        }
    }

    fn apply_blur(&mut self, rect: RectF, radius: u32) {
        let r = (radius as i32).clamp(1, 32);
        let x0 = (rect.x.floor() as i32).max(0);
        let y0 = (rect.y.floor() as i32).max(0);
        let x1 = (rect.right().ceil() as i32).min(self.width);
        let y1 = (rect.bottom().ceil() as i32).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        let rw = (x1 - x0) as usize;
        let rh = (y1 - y0) as usize;
        let mut horiz = vec![0u8; rw * rh * 4];

        for ly in 0..rh {
            let gy = y0 + ly as i32;
            for lx in 0..rw {
                let gx = x0 + lx as i32;
                let mut r_sum: u32 = 0;
                let mut g_sum: u32 = 0;
                let mut b_sum: u32 = 0;
                let mut count: u32 = 0;
                for dx in -r..=r {
                    let sx = (gx + dx).clamp(x0, x1 - 1);
                    let idx = ((gy as usize) * (self.width as usize) + (sx as usize)) * 4;
                    r_sum += u32::from(self.rgba[idx]);
                    g_sum += u32::from(self.rgba[idx + 1]);
                    b_sum += u32::from(self.rgba[idx + 2]);
                    count += 1;
                }
                let out_idx = (ly * rw + lx) * 4;
                horiz[out_idx] = (r_sum / count) as u8;
                horiz[out_idx + 1] = (g_sum / count) as u8;
                horiz[out_idx + 2] = (b_sum / count) as u8;
                horiz[out_idx + 3] = 255;
            }
        }

        for ly in 0..rh {
            let gy = y0 + ly as i32;
            for lx in 0..rw {
                let gx = x0 + lx as i32;
                let mut r_sum: u32 = 0;
                let mut g_sum: u32 = 0;
                let mut b_sum: u32 = 0;
                let mut count: u32 = 0;
                for dy in -r..=r {
                    let sy = (ly as i32 + dy).clamp(0, (rh as i32) - 1) as usize;
                    let idx = (sy * rw + lx) * 4;
                    r_sum += u32::from(horiz[idx]);
                    g_sum += u32::from(horiz[idx + 1]);
                    b_sum += u32::from(horiz[idx + 2]);
                    count += 1;
                }
                self.blend_pixel(
                    gx,
                    gy,
                    RgbaColor::rgb(
                        (r_sum / count) as u8,
                        (g_sum / count) as u8,
                        (b_sum / count) as u8,
                    ),
                );
            }
        }
    }
}

/// Returns the clockwise balloon outline and the optional index of the tail tip.
pub fn speech_balloon_outline(rect: RectF, tail: Point) -> (Vec<Point>, Option<usize>) {
    let radius = (rect.width.min(rect.height) * 0.2).clamp(0.0, 12.0);
    let corners = [
        Point::new(rect.right() - radius, rect.y + radius),
        Point::new(rect.right() - radius, rect.bottom() - radius),
        Point::new(rect.x + radius, rect.bottom() - radius),
        Point::new(rect.x + radius, rect.y + radius),
    ];
    let side = if rect.contains(tail, 0.0) {
        None
    } else {
        let dx = ((tail.x - rect.x) - rect.width * 0.5) / rect.width.max(f32::EPSILON);
        let dy = ((tail.y - rect.y) - rect.height * 0.5) / rect.height.max(f32::EPSILON);
        Some(if dx.abs() > dy.abs() {
            if dx > 0.0 { 0 } else { 2 }
        } else if dy > 0.0 {
            1
        } else {
            3
        })
    };
    let mut outline = Vec::with_capacity(27);
    let mut tail_index = None;
    for (index, center) in corners.into_iter().enumerate() {
        let quarter = std::f32::consts::FRAC_PI_2;
        for step in 0..=5 {
            let angle = (index as f32 - 1.0 + step as f32 / 5.0) * quarter;
            let (sin, cos) = angle.sin_cos();
            outline.push(Point::new(center.x + cos * radius, center.y + sin * radius));
        }
        if side == Some(index) {
            let a = corners[index];
            let b = corners[(index + 1) % corners.len()];
            let length = (b.x - a.x).hypot(b.y - a.y);
            if length <= 0.0 {
                continue;
            }
            let axis = Point::new((b.x - a.x) / length, (b.y - a.y) / length);
            let normal = Point::new(axis.y * radius, -axis.x * radius);
            let half_base = (length * 0.25).min(9.0);
            let projection = (tail.x - a.x) * axis.x + (tail.y - a.y) * axis.y;
            let mid = projection.clamp(half_base, length - half_base);
            let point_on_side = |offset: f32| {
                Point::new(
                    a.x + axis.x * offset + normal.x,
                    a.y + axis.y * offset + normal.y,
                )
            };
            outline.push(point_on_side(mid - half_base));
            tail_index = Some(outline.len());
            outline.push(tail);
            outline.push(point_on_side(mid + half_base));
        }
    }
    (outline, tail_index)
}

/// Flattens an `EditorDocument` into an RGBA `CapturedImage`.
pub fn render_document(doc: &EditorDocument) -> Result<CapturedImage, CoreError> {
    render_document_with_font(doc, &TextFont::default())
}

/// Flattens all annotation layers using the chosen font for text labels.
pub fn render_document_with_font(
    doc: &EditorDocument,
    font: &TextFont<'_>,
) -> Result<CapturedImage, CoreError> {
    let base = doc.base_image();
    let mut canvas = Canvas::from_image(base);

    for annotation in doc.annotations() {
        match &annotation.kind {
            AnnotationKind::Rectangle {
                rect,
                color,
                stroke_width,
                filled,
            } => {
                if *filled {
                    canvas.fill_rect(*rect, *color);
                } else {
                    canvas.stroke_rect(*rect, *color, *stroke_width);
                }
            }
            AnnotationKind::Ellipse {
                rect,
                color,
                stroke_width,
            } => {
                canvas.stroke_ellipse(*rect, *color, *stroke_width);
            }
            AnnotationKind::Line {
                start,
                end,
                color,
                stroke_width,
            } => {
                canvas.stroke_segment(*start, *end, *color, *stroke_width);
            }
            AnnotationKind::Arrow {
                start,
                end,
                color,
                stroke_width,
            } => {
                canvas.draw_arrow(*start, *end, *color, *stroke_width);
            }
            AnnotationKind::Freehand {
                points,
                color,
                stroke_width,
            } => {
                canvas.draw_polyline(points, *color, *stroke_width);
            }
            AnnotationKind::Highlighter {
                points,
                color,
                stroke_width,
            } => {
                canvas.draw_polyline(points, color.with_alpha(96), *stroke_width);
            }
            AnnotationKind::Text {
                position,
                text,
                color,
                font_size,
                background,
            } => {
                canvas.draw_text_label(*position, text, *color, *font_size, *background, font);
            }
            AnnotationKind::StepNumber {
                center,
                number,
                color,
                radius,
            } => {
                canvas.fill_circle(*center, *radius + 2.0, RgbaColor::WHITE);
                canvas.fill_circle(*center, *radius, *color);
                let label = number.to_string();
                let font_size = (*radius * 0.95).max(12.0);
                let scale = ((font_size / 7.0).round()).max(1.0);
                let text_w = (label.len() as f32 * 6.0 - 1.0) * scale;
                let text_h = 7.0 * scale;
                let origin = Point::new(center.x - text_w * 0.5, center.y - text_h * 0.5);
                canvas.draw_text_line(origin, &label, RgbaColor::WHITE, font_size);
            }
            AnnotationKind::Pixelate { rect, block_size } => {
                canvas.apply_pixelate(*rect, *block_size);
            }
            AnnotationKind::Blur { rect, radius } => {
                canvas.apply_blur(*rect, *radius);
            }
            AnnotationKind::SpeechBalloon {
                rect,
                tail,
                text,
                fill,
                border,
                text_color,
                font_size,
            } => {
                canvas.draw_balloon(*rect, *tail, *fill, *border);
                canvas.draw_balloon_text(*rect, text, *text_color, *font_size);
            }
            AnnotationKind::Magnifier {
                center,
                radius,
                zoom,
                border_color,
            } => {
                canvas.apply_magnifier(*center, *radius, *zoom, *border_color);
            }
            AnnotationKind::Spotlight { rect, dim_alpha } => {
                canvas.apply_spotlight(*rect, *dim_alpha);
            }
            AnnotationKind::ImageOverlay {
                rect,
                width,
                height,
                rgba,
                opacity,
            } => {
                canvas.draw_image_overlay(*rect, *width, *height, rgba, *opacity);
            }
            AnnotationKind::SmartEraser {
                points,
                stroke_width,
                sample_color,
            } => {
                canvas.draw_smart_eraser(points, *stroke_width, *sample_color);
            }
        }
    }

    CapturedImage::from_rgba(base.width(), base.height(), canvas.rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use voltshot_core::Region;

    fn solid_image(width: u32, height: u32, color: RgbaColor) -> CapturedImage {
        let px = [color.r, color.g, color.b, color.a];
        CapturedImage::from_rgba(width, height, px.repeat((width * height) as usize))
            .expect("valid image")
    }

    fn pixel_at(img: &CapturedImage, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * img.width() + x) * 4) as usize;
        [
            img.rgba()[idx],
            img.rgba()[idx + 1],
            img.rgba()[idx + 2],
            img.rgba()[idx + 3],
        ]
    }

    #[test]
    fn renders_rectangle_arrow_step_number_and_text_onto_pixels() {
        let mut doc = EditorDocument::new(solid_image(120, 100, RgbaColor::BLACK));
        doc.push_annotation(AnnotationKind::Rectangle {
            rect: RectF {
                x: 5.0,
                y: 5.0,
                width: 30.0,
                height: 20.0,
            },
            color: RgbaColor::RED,
            stroke_width: 3.0,
            filled: false,
        });
        doc.push_annotation(AnnotationKind::Arrow {
            start: Point::new(10.0, 50.0),
            end: Point::new(70.0, 50.0),
            color: RgbaColor::GREEN,
            stroke_width: 4.0,
        });
        doc.push_annotation(AnnotationKind::StepNumber {
            center: Point::new(95.0, 25.0),
            number: 1,
            color: RgbaColor::BLUE,
            radius: 14.0,
        });
        doc.push_annotation(AnnotationKind::Text {
            position: Point::new(10.0, 75.0),
            text: "HI".into(),
            color: RgbaColor::WHITE,
            font_size: 14.0,
            background: None,
        });

        let flat = doc.flatten().expect("flatten succeeds");
        assert_eq!(pixel_at(&flat, 6, 6), [235, 64, 52, 255]);
        assert_eq!(pixel_at(&flat, 15, 15), [20, 22, 26, 255]);
        assert_eq!(pixel_at(&flat, 40, 50), [52, 199, 89, 255]);
        assert_eq!(pixel_at(&flat, 95, 25), [245, 247, 250, 255]);
        assert_eq!(pixel_at(&flat, 85, 25), [59, 130, 246, 255]);
        assert_eq!(pixel_at(&flat, 10, 75), [245, 247, 250, 255]);
    }

    #[test]
    fn truetype_renders_partial_coverage_and_invalid_font_uses_bitmap() {
        let mut doc = EditorDocument::new(solid_image(96, 48, RgbaColor::rgba(0, 0, 0, 0)));
        doc.push_annotation(AnnotationKind::Text {
            position: Point::new(4.25, 4.25),
            text: "AVg".into(),
            color: RgbaColor::WHITE.with_alpha(200),
            font_size: 27.0,
            background: None,
        });

        let invalid = TextFont::from_bytes(b"not a font");
        assert!(!invalid.is_truetype());
        let fallback = render_document_with_font(&doc, &invalid).unwrap();
        assert_eq!(fallback.rgba(), doc.flatten().unwrap().rgba());
        assert!(
            fallback
                .rgba()
                .chunks_exact(4)
                .all(|pixel| [0, 200].contains(&pixel[3]))
        );
        assert!(fallback.rgba().chunks_exact(4).any(|pixel| pixel[3] == 200));

        let system = TextFont::system();
        let rendered = render_document_with_font(&doc, &system).unwrap();
        if system.is_truetype() {
            assert!(rendered.rgba().chunks_exact(4).any(|pixel| pixel[3] == 200));
            assert!(
                rendered
                    .rgba()
                    .chunks_exact(4)
                    .any(|pixel| { pixel[..3] == [245, 247, 250] && (1..200).contains(&pixel[3]) })
            );
        } else {
            assert_eq!(rendered.rgba(), fallback.rgba());
        }
    }

    #[test]
    fn image_overlay_scales_composites_and_round_trips() {
        let base = solid_image(5, 4, RgbaColor::rgb(0, 0, 255));
        let mut doc = EditorDocument::new(base.clone());
        doc.push_annotation(AnnotationKind::ImageOverlay {
            rect: RectF::from_points(Point::new(-1.0, 1.0), Point::new(3.0, 3.0)),
            width: 2,
            height: 2,
            rgba: vec![
                255, 0, 0, 128, 0, 255, 0, 255, 255, 0, 0, 0, 255, 255, 255, 255,
            ],
            opacity: 0.5,
        });
        let flat = doc.flatten().unwrap();
        assert_eq!(pixel_at(&flat, 0, 1), [64, 0, 191, 255]);
        assert_eq!(pixel_at(&flat, 1, 1), [0, 128, 127, 255]);
        assert_eq!(pixel_at(&flat, 2, 1), [0, 128, 127, 255]);
        assert_eq!(pixel_at(&flat, 0, 2), [0, 0, 255, 255]);
        assert_eq!(pixel_at(&flat, 1, 2), [128, 128, 255, 255]);
        assert_eq!(pixel_at(&flat, 3, 2), [0, 0, 255, 255]);

        let mut imported = EditorDocument::new(base);
        let json = doc.export_annotations_json().unwrap();
        assert_eq!(imported.import_annotations_json(&json).unwrap(), 1);
        assert_eq!(imported.annotations(), doc.annotations());
        assert_eq!(imported.flatten().unwrap().rgba(), flat.rgba());
    }

    #[test]
    fn malformed_image_overlay_buffers_are_ignored() {
        let base = solid_image(4, 4, RgbaColor::WHITE);
        let mut doc = EditorDocument::new(base.clone());
        for (width, height, rgba) in [
            (2, 2, vec![255; 15]),
            (1, 1, vec![255; 5]),
            (u32::MAX, u32::MAX, vec![]),
            (0, 3, vec![]),
        ] {
            doc.push_annotation(AnnotationKind::ImageOverlay {
                rect: RectF::from_points(Point::new(0.0, 0.0), Point::new(4.0, 4.0)),
                width,
                height,
                rgba,
                opacity: 1.0,
            });
        }
        assert_eq!(doc.flatten().unwrap().rgba(), base.rgba());
    }

    #[test]
    fn smart_eraser_samples_prior_layers_and_paints_opaque_pixels() {
        let base = solid_image(12, 8, RgbaColor::rgba(0, 0, 255, 80));
        let mut doc = EditorDocument::new(base.clone());
        doc.push_annotation(AnnotationKind::Rectangle {
            rect: RectF::from_points(Point::new(0.0, 2.0), Point::new(4.0, 6.0)),
            color: RgbaColor::rgba(255, 0, 0, 128),
            stroke_width: 1.0,
            filled: true,
        });
        let blemish = RgbaColor::rgb(0, 255, 0);
        doc.push_annotation(AnnotationKind::Rectangle {
            rect: RectF::from_points(Point::new(5.0, 3.0), Point::new(9.0, 5.0)),
            color: blemish,
            stroke_width: 1.0,
            filled: true,
        });
        let before = doc.flatten().unwrap();
        let [red, green, blue, alpha] = pixel_at(&before, 2, 4);
        assert!(red > 0 && blue > 0 && alpha < 255);
        doc.push_annotation(AnnotationKind::SmartEraser {
            points: vec![Point::new(2.5, 4.0), Point::new(9.5, 4.0)],
            stroke_width: 2.0,
            sample_color: None,
        });
        doc.push_annotation(AnnotationKind::SmartEraser {
            points: vec![Point::new(11.5, 0.5)],
            stroke_width: 1.0,
            sample_color: Some(RgbaColor::rgba(123, 45, 67, 0)),
        });
        let flat = doc.flatten().unwrap();
        for y in 3..5 {
            for x in 5..9 {
                assert_eq!(pixel_at(&flat, x, y), [red, green, blue, 255]);
            }
        }
        assert_eq!(pixel_at(&flat, 11, 0), [123, 45, 67, 255]);
        assert_eq!(pixel_at(&flat, 6, 2), pixel_at(&before, 6, 2));

        let mut imported = EditorDocument::new(base);
        assert_eq!(
            imported
                .import_annotations_json(&doc.export_annotations_json().unwrap())
                .unwrap(),
            4
        );
        assert_eq!(imported.annotations(), doc.annotations());
        assert_eq!(imported.flatten().unwrap().rgba(), flat.rgba());
    }

    #[test]
    fn pixelate_and_blur_obscure_high_contrast_checkerboard() {
        let mut rgba = Vec::with_capacity(64 * 64 * 4);
        for y in 0..64u32 {
            for x in 0..64u32 {
                let c = if (x + y) % 2 == 0 { 255 } else { 0 };
                rgba.extend_from_slice(&[c, c, c, 255]);
            }
        }
        let mut doc =
            EditorDocument::new(CapturedImage::from_rgba(64, 64, rgba).expect("checkerboard"));
        doc.push_annotation(AnnotationKind::Pixelate {
            rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
            },
            block_size: 8,
        });
        doc.push_annotation(AnnotationKind::Blur {
            rect: RectF {
                x: 32.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
            },
            radius: 3,
        });

        let flat = doc.flatten().expect("flatten succeeds");
        let p_pix = pixel_at(&flat, 4, 4)[0];
        assert!((110..=145).contains(&p_pix));
        let p_blur = pixel_at(&flat, 48, 16)[0];
        assert!((100..=155).contains(&p_blur));
        assert_eq!(pixel_at(&flat, 0, 40)[0], 255);
        assert_eq!(pixel_at(&flat, 1, 40)[0], 0);
    }

    #[test]
    fn spotlight_dims_only_pixels_outside_the_opening() {
        let mut doc = EditorDocument::new(solid_image(8, 6, RgbaColor::rgba(200, 100, 50, 180)));
        doc.push_annotation(AnnotationKind::Spotlight {
            rect: RectF::from_points(Point::new(2.0, 1.0), Point::new(6.0, 5.0)),
            dim_alpha: 128,
        });

        let flat = doc.flatten().unwrap();
        for y in 0..6 {
            for x in 0..8 {
                let expected = if (2..6).contains(&x) && (1..5).contains(&y) {
                    [200, 100, 50, 180]
                } else {
                    [100, 50, 25, 180]
                };
                assert_eq!(pixel_at(&flat, x, y), expected, "pixel ({x}, {y})");
            }
        }
    }

    #[test]
    fn magnifier_enlarges_a_base_pixel_and_draws_a_border() {
        let background = [20, 22, 26, 255];
        let feature = [235, 64, 52, 255];
        let mut rgba = background.repeat(25 * 25);
        let offset = (12 * 25 + 12) * 4;
        rgba[offset..offset + 4].copy_from_slice(&feature);
        let mut doc = EditorDocument::new(CapturedImage::from_rgba(25, 25, rgba).unwrap());
        doc.push_annotation(AnnotationKind::Magnifier {
            center: Point::new(12.5, 12.5),
            radius: 7.0,
            zoom: 3.0,
            border_color: RgbaColor::BLUE,
        });

        let flat = doc.flatten().unwrap();
        assert_eq!(pixel_at(&flat, 11, 12), feature);
        assert_eq!(pixel_at(&flat, 12, 12), feature);
        assert_eq!(pixel_at(&flat, 13, 13), feature);
        assert_eq!(pixel_at(&flat, 16, 12), background);
        assert_eq!(pixel_at(&flat, 18, 12), [59, 130, 246, 255]);
        assert_eq!(pixel_at(&flat, 20, 12), background);
    }

    #[test]
    fn magnifier_over_redaction_never_leaks_original_base_pixels() {
        let secret = [235, 64, 52, 255];
        let cover = [52, 199, 89, 255];
        let mut doc = EditorDocument::new(solid_image(25, 25, RgbaColor::RED));
        doc.push_annotation(AnnotationKind::Rectangle {
            rect: RectF::from_points(Point::new(0.0, 0.0), Point::new(25.0, 25.0)),
            color: RgbaColor::GREEN,
            stroke_width: 1.0,
            filled: true,
        });
        doc.push_annotation(AnnotationKind::Magnifier {
            center: Point::new(12.5, 12.5),
            radius: 7.0,
            zoom: 3.0,
            border_color: RgbaColor::BLUE,
        });

        let flat = doc.flatten().unwrap();
        assert_eq!(pixel_at(&flat, 12, 12), cover);
        assert_ne!(pixel_at(&flat, 12, 12), secret);
        assert_eq!(pixel_at(&flat, 18, 12), [59, 130, 246, 255]);
    }

    #[test]
    fn speech_balloon_draws_rounded_body_outline_tail_and_text() {
        let mut doc = EditorDocument::new(solid_image(50, 50, RgbaColor::BLACK));
        doc.push_annotation(AnnotationKind::SpeechBalloon {
            rect: RectF::from_points(Point::new(8.0, 6.0), Point::new(42.0, 30.0)),
            tail: Point::new(25.0, 44.0),
            text: "A".into(),
            fill: RgbaColor::YELLOW,
            border: RgbaColor::BLUE,
            text_color: RgbaColor::RED,
            font_size: 7.0,
        });

        let flat = doc.flatten().unwrap();
        let fill = [250, 214, 48, 255];
        let border = [59, 130, 246, 255];
        assert_eq!(pixel_at(&flat, 8, 6), [20, 22, 26, 255]);
        assert_eq!(pixel_at(&flat, 36, 18), fill);
        assert_eq!(pixel_at(&flat, 25, 6), border);
        assert_eq!(pixel_at(&flat, 25, 30), fill);
        assert_eq!(pixel_at(&flat, 25, 35), fill);
        assert_eq!(pixel_at(&flat, 25, 43), border);
        assert_eq!(pixel_at(&flat, 15, 15), [235, 64, 52, 255]);
    }

    #[test]
    fn undo_redo_and_crop_preserve_document_invariants() {
        let mut doc = EditorDocument::new(solid_image(100, 80, RgbaColor::BLACK));
        let id = doc.push_annotation(AnnotationKind::Rectangle {
            rect: RectF {
                x: 20.0,
                y: 20.0,
                width: 10.0,
                height: 10.0,
            },
            color: RgbaColor::RED,
            stroke_width: 2.0,
            filled: true,
        });
        assert_eq!(doc.annotations().len(), 1);
        assert!(doc.can_undo());

        doc.crop(Region::new(10, 10, 50, 40).expect("valid crop"))
            .expect("crop succeeds");
        assert_eq!(doc.base_image().width(), 50);
        assert_eq!(doc.base_image().height(), 40);
        assert_eq!(doc.annotations()[0].bounds().x, 10.0);

        assert!(doc.undo());
        assert_eq!(doc.base_image().width(), 100);
        assert_eq!(doc.annotations()[0].bounds().x, 20.0);

        assert!(doc.redo());
        assert_eq!(doc.base_image().width(), 50);

        doc.select(Some(id));
        assert!(doc.delete_selected());
        assert!(doc.annotations().is_empty());
        assert!(doc.undo());
        assert_eq!(doc.annotations().len(), 1);
    }
}
