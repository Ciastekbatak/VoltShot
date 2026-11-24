use crate::font::glyph_5x7;
use crate::model::{AnnotationKind, EditorDocument, Point, RectF, RgbaColor};
use sharex_core::{CapturedImage, CoreError};

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
        let inv = 1.0 - alpha;
        self.rgba[idx] =
            (f32::from(color.r) * alpha + f32::from(self.rgba[idx]) * inv).round() as u8;
        self.rgba[idx + 1] =
            (f32::from(color.g) * alpha + f32::from(self.rgba[idx + 1]) * inv).round() as u8;
        self.rgba[idx + 2] =
            (f32::from(color.b) * alpha + f32::from(self.rgba[idx + 2]) * inv).round() as u8;
        self.rgba[idx + 3] = 255;
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
        let scale = ((font_size / 7.0).round() as i32).max(1);
        let mut cursor_x = origin.x.round() as i32;
        let start_y = origin.y.round() as i32;

        for ch in text.chars() {
            let rows = glyph_5x7(ch);
            for (row_idx, bits) in rows.iter().enumerate() {
                for col_idx in 0..5 {
                    if (bits >> (4 - col_idx)) & 1 == 1 {
                        let px = cursor_x + col_idx * scale;
                        let py = start_y + (row_idx as i32) * scale;
                        for dy in 0..scale {
                            for dx in 0..scale {
                                self.blend_pixel(px + dx, py + dy, color);
                            }
                        }
                    }
                }
            }
            cursor_x += 6 * scale;
        }
    }

    fn apply_pixelate(&mut self, rect: RectF, block_size: u32) {
        let step = (block_size as i32).max(2);
        let x0 = (rect.x.floor() as i32).max(0);
        let y0 = (rect.y.floor() as i32).max(0);
        let x1 = (rect.right().ceil() as i32).min(self.width);
        let y1 = (rect.bottom().ceil() as i32).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        let mut by = y0;
        while by < y1 {
            let block_y1 = (by + step).min(y1);
            let mut bx = x0;
            while bx < x1 {
                let block_x1 = (bx + step).min(x1);
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

                bx += step;
            }
            by += step;
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

/// Flattens an `EditorDocument` into an RGBA `CapturedImage`.
pub fn render_document(doc: &EditorDocument) -> Result<CapturedImage, CoreError> {
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
                if let Some(bg) = background {
                    let scale = ((*font_size / 7.0).round()).max(1.0);
                    let w = (text.chars().count().max(1) as f32) * (6.0 * scale) + 10.0;
                    let h = 7.0 * scale + 10.0;
                    canvas.fill_rect(
                        RectF {
                            x: position.x - 5.0,
                            y: position.y - 5.0,
                            width: w,
                            height: h,
                        },
                        *bg,
                    );
                }
                canvas.draw_text_line(*position, text, *color, *font_size);
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
        }
    }

    CapturedImage::from_rgba(base.width(), base.height(), canvas.rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sharex_core::Region;

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
