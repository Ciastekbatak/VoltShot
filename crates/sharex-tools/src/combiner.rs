use serde::{Deserialize, Serialize};
use sharex_core::{CapturedImage, CoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CombineOrientation {
    Horizontal,
    #[default]
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CombineAlignment {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombineOptions {
    pub orientation: CombineOrientation,
    pub alignment: CombineAlignment,
    pub gap: u32,
    pub background: [u8; 4],
}

impl Default for CombineOptions {
    fn default() -> Self {
        Self {
            orientation: CombineOrientation::Vertical,
            alignment: CombineAlignment::Start,
            gap: 0,
            background: [0, 0, 0, 0],
        }
    }
}

pub fn combine_images(
    images: &[&CapturedImage],
    opts: &CombineOptions,
) -> Result<CapturedImage, CoreError> {
    if images.is_empty() {
        return Err(CoreError::InvalidRegion(
            "cannot combine an empty list of images".into(),
        ));
    }

    let gap_count = u32::try_from(images.len() - 1)
        .map_err(|_| CoreError::InvalidRegion("too many images to combine".into()))?;
    let total_gap = opts
        .gap
        .checked_mul(gap_count)
        .ok_or_else(|| CoreError::InvalidRegion("combined image gap overflows u32".into()))?;

    let (total_width, total_height) = match opts.orientation {
        CombineOrientation::Horizontal => {
            let mut sum_w = 0u32;
            let mut max_h = 0u32;
            for img in images {
                sum_w = sum_w.checked_add(img.width()).ok_or_else(|| {
                    CoreError::InvalidRegion("combined image width overflows u32".into())
                })?;
                max_h = max_h.max(img.height());
            }
            let width = sum_w.checked_add(total_gap).ok_or_else(|| {
                CoreError::InvalidRegion("combined image width overflows u32".into())
            })?;
            (width, max_h)
        }
        CombineOrientation::Vertical => {
            let mut max_w = 0u32;
            let mut sum_h = 0u32;
            for img in images {
                max_w = max_w.max(img.width());
                sum_h = sum_h.checked_add(img.height()).ok_or_else(|| {
                    CoreError::InvalidRegion("combined image height overflows u32".into())
                })?;
            }
            let height = sum_h.checked_add(total_gap).ok_or_else(|| {
                CoreError::InvalidRegion("combined image height overflows u32".into())
            })?;
            (max_w, height)
        }
    };

    let pixel_count = (total_width as usize)
        .checked_mul(total_height as usize)
        .ok_or_else(|| CoreError::InvalidRegion("combined image dimensions overflow".into()))?;
    let byte_len = pixel_count
        .checked_mul(4)
        .ok_or_else(|| CoreError::InvalidRegion("combined image buffer size overflows".into()))?;

    let mut out = Vec::with_capacity(byte_len);
    for _ in 0..pixel_count {
        out.extend_from_slice(&opts.background);
    }

    let dst_stride = (total_width as usize) * 4;
    let mut cursor = 0u32;

    for img in images {
        let (dst_x, dst_y) = match opts.orientation {
            CombineOrientation::Horizontal => (
                cursor,
                align_offset(total_height, img.height(), opts.alignment),
            ),
            CombineOrientation::Vertical => (
                align_offset(total_width, img.width(), opts.alignment),
                cursor,
            ),
        };

        let src_stride = (img.width() as usize) * 4;
        let src_rgba = img.rgba();
        for row in 0..img.height() as usize {
            let src_start = row * src_stride;
            let dst_start = ((dst_y as usize) + row) * dst_stride + (dst_x as usize) * 4;
            out[dst_start..dst_start + src_stride]
                .copy_from_slice(&src_rgba[src_start..src_start + src_stride]);
        }

        let advance = match opts.orientation {
            CombineOrientation::Horizontal => img.width(),
            CombineOrientation::Vertical => img.height(),
        };
        cursor = cursor.saturating_add(advance).saturating_add(opts.gap);
    }

    CapturedImage::from_rgba(total_width, total_height, out)
}

const fn align_offset(container: u32, item: u32, alignment: CombineAlignment) -> u32 {
    match alignment {
        CombineAlignment::Start => 0,
        CombineAlignment::Center => (container - item) / 2,
        CombineAlignment::End => container - item,
    }
}

/// Configuration for multi-row thumbnail contact sheet generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactSheetOptions {
    pub columns: u32,
    pub thumb_width: u32,
    pub thumb_height: u32,
    pub gap: u32,
    pub padding: u32,
    pub background: [u8; 4],
}

impl Default for ContactSheetOptions {
    fn default() -> Self {
        Self {
            columns: 4,
            thumb_width: 320,
            thumb_height: 180,
            gap: 8,
            padding: 16,
            background: [24, 24, 24, 255],
        }
    }
}

impl ContactSheetOptions {
    /// Validates that contact sheet grid dimensions are non-zero.
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.columns == 0 {
            return Err(CoreError::InvalidRegion(
                "contact sheet columns must be greater than zero".into(),
            ));
        }
        if self.thumb_width == 0 {
            return Err(CoreError::InvalidRegion(
                "contact sheet thumb_width must be greater than zero".into(),
            ));
        }
        if self.thumb_height == 0 {
            return Err(CoreError::InvalidRegion(
                "contact sheet thumb_height must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}

/// Renders a multi-row thumbnail contact sheet by proportionally letterboxing each image into its cell.
pub fn generate_contact_sheet(
    images: &[CapturedImage],
    opts: &ContactSheetOptions,
) -> Result<CapturedImage, CoreError> {
    opts.validate()?;
    if images.is_empty() {
        return Err(CoreError::InvalidRegion(
            "cannot generate a contact sheet from an empty list of images".into(),
        ));
    }

    let count = u32::try_from(images.len())
        .map_err(|_| CoreError::InvalidRegion("too many images for contact sheet".into()))?;
    let rows = count.div_ceil(opts.columns);

    let pad_both = opts
        .padding
        .checked_mul(2)
        .ok_or_else(|| CoreError::InvalidRegion("contact sheet padding overflows u32".into()))?;
    let col_thumbs = opts
        .columns
        .checked_mul(opts.thumb_width)
        .ok_or_else(|| CoreError::InvalidRegion("contact sheet width overflows u32".into()))?;
    let col_gaps = opts
        .columns
        .saturating_sub(1)
        .checked_mul(opts.gap)
        .ok_or_else(|| CoreError::InvalidRegion("contact sheet width gap overflows u32".into()))?;
    let total_width = pad_both
        .checked_add(col_thumbs)
        .and_then(|v| v.checked_add(col_gaps))
        .ok_or_else(|| {
            CoreError::InvalidRegion("contact sheet total width overflows u32".into())
        })?;

    let row_thumbs = rows
        .checked_mul(opts.thumb_height)
        .ok_or_else(|| CoreError::InvalidRegion("contact sheet height overflows u32".into()))?;
    let row_gaps = rows
        .saturating_sub(1)
        .checked_mul(opts.gap)
        .ok_or_else(|| CoreError::InvalidRegion("contact sheet height gap overflows u32".into()))?;
    let total_height = pad_both
        .checked_add(row_thumbs)
        .and_then(|v| v.checked_add(row_gaps))
        .ok_or_else(|| {
            CoreError::InvalidRegion("contact sheet total height overflows u32".into())
        })?;

    let pixel_count = (total_width as usize)
        .checked_mul(total_height as usize)
        .ok_or_else(|| {
            CoreError::InvalidRegion("contact sheet canvas pixel count overflows".into())
        })?;
    let byte_len = pixel_count.checked_mul(4).ok_or_else(|| {
        CoreError::InvalidRegion("contact sheet canvas buffer size overflows".into())
    })?;

    let mut out = Vec::with_capacity(byte_len);
    for _ in 0..pixel_count {
        out.extend_from_slice(&opts.background);
    }

    let cell_step_x = opts.thumb_width.saturating_add(opts.gap);
    let cell_step_y = opts.thumb_height.saturating_add(opts.gap);

    for (index, img) in images.iter().enumerate() {
        let idx_u32 = index as u32;
        let col = idx_u32 % opts.columns;
        let row = idx_u32 / opts.columns;
        let cell_x = opts.padding + col * cell_step_x;
        let cell_y = opts.padding + row * cell_step_y;

        let (scaled_w, scaled_h) = fit_dimensions(
            img.width(),
            img.height(),
            opts.thumb_width,
            opts.thumb_height,
        );
        let dst_x0 = cell_x + (opts.thumb_width - scaled_w) / 2;
        let dst_y0 = cell_y + (opts.thumb_height - scaled_h) / 2;

        blit_scaled_bilinear(
            img,
            &mut out,
            total_width,
            dst_x0,
            dst_y0,
            scaled_w,
            scaled_h,
        );
    }

    CapturedImage::from_rgba(total_width, total_height, out)
}

fn fit_dimensions(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let scale_x = f64::from(max_w) / f64::from(src_w);
    let scale_y = f64::from(max_h) / f64::from(src_h);
    let scale = scale_x.min(scale_y);
    let scaled_w = ((f64::from(src_w) * scale).round() as u32).clamp(1, max_w);
    let scaled_h = ((f64::from(src_h) * scale).round() as u32).clamp(1, max_h);
    (scaled_w, scaled_h)
}

fn blit_scaled_bilinear(
    src: &CapturedImage,
    dst: &mut [u8],
    dst_width: u32,
    dst_x0: u32,
    dst_y0: u32,
    scaled_w: u32,
    scaled_h: u32,
) {
    let src_w = src.width();
    let src_h = src.height();
    let src_rgba = src.rgba();
    let dst_stride = (dst_width as usize) * 4;

    if src_w == scaled_w && src_h == scaled_h {
        let src_stride = (src_w as usize) * 4;
        for row in 0..src_h as usize {
            let src_start = row * src_stride;
            let dst_start = ((dst_y0 as usize) + row) * dst_stride + (dst_x0 as usize) * 4;
            dst[dst_start..dst_start + src_stride]
                .copy_from_slice(&src_rgba[src_start..src_start + src_stride]);
        }
        return;
    }

    let max_src_x = f64::from(src_w - 1);
    let max_src_y = f64::from(src_h - 1);
    let ratio_x = f64::from(src_w) / f64::from(scaled_w);
    let ratio_y = f64::from(src_h) / f64::from(scaled_h);

    for dy in 0..scaled_h {
        let sy = ((f64::from(dy) + 0.5) * ratio_y - 0.5).clamp(0.0, max_src_y);
        let y0 = sy.floor() as u32;
        let y1 = (y0 + 1).min(src_h - 1);
        let fy = sy - f64::from(y0);
        let wy0 = 1.0 - fy;
        let wy1 = fy;

        let dst_row_offset = ((dst_y0 + dy) as usize) * dst_stride;

        for dx in 0..scaled_w {
            let sx = ((f64::from(dx) + 0.5) * ratio_x - 0.5).clamp(0.0, max_src_x);
            let x0 = sx.floor() as u32;
            let x1 = (x0 + 1).min(src_w - 1);
            let fx = sx - f64::from(x0);
            let wx0 = 1.0 - fx;
            let wx1 = fx;

            let i00 = ((y0 * src_w + x0) * 4) as usize;
            let i10 = ((y0 * src_w + x1) * 4) as usize;
            let i01 = ((y1 * src_w + x0) * 4) as usize;
            let i11 = ((y1 * src_w + x1) * 4) as usize;

            let dst_idx = dst_row_offset + ((dst_x0 + dx) as usize) * 4;
            for ch in 0..4 {
                let top = f64::from(src_rgba[i00 + ch]) * wx0 + f64::from(src_rgba[i10 + ch]) * wx1;
                let bot = f64::from(src_rgba[i01 + ch]) * wx0 + f64::from(src_rgba[i11 + ch]) * wx1;
                let value = (top * wy0 + bot * wy1).round().clamp(0.0, 255.0) as u8;
                dst[dst_idx + ch] = value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> CapturedImage {
        let mut buf = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            buf.extend_from_slice(&rgba);
        }
        CapturedImage::from_rgba(width, height, buf).expect("valid solid image")
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
    fn rejects_empty_image_slice() {
        assert!(matches!(
            combine_images(&[], &CombineOptions::default()),
            Err(CoreError::InvalidRegion(_))
        ));
    }

    #[test]
    fn combines_horizontally_with_gap_and_center_alignment() {
        let red = solid(2, 2, [255, 0, 0, 255]);
        let blue = solid(3, 4, [0, 0, 255, 255]);
        let bg = [10, 20, 30, 255];

        let combined = combine_images(
            &[&red, &blue],
            &CombineOptions {
                orientation: CombineOrientation::Horizontal,
                alignment: CombineAlignment::Center,
                gap: 1,
                background: bg,
            },
        )
        .expect("horizontal combine");

        assert_eq!(combined.width(), 6);
        assert_eq!(combined.height(), 4);
        assert_eq!(pixel_at(&combined, 0, 0), bg);
        assert_eq!(pixel_at(&combined, 0, 1), [255, 0, 0, 255]);
        assert_eq!(pixel_at(&combined, 1, 2), [255, 0, 0, 255]);
        assert_eq!(pixel_at(&combined, 0, 3), bg);
        assert_eq!(pixel_at(&combined, 2, 1), bg);
        assert_eq!(pixel_at(&combined, 3, 0), [0, 0, 255, 255]);
        assert_eq!(pixel_at(&combined, 5, 3), [0, 0, 255, 255]);
    }

    #[test]
    fn combines_vertically_with_end_alignment_and_gap() {
        let green = solid(2, 1, [0, 255, 0, 255]);
        let white = solid(4, 2, [255, 255, 255, 255]);
        let bg = [5, 5, 5, 255];

        let combined = combine_images(
            &[&green, &white],
            &CombineOptions {
                orientation: CombineOrientation::Vertical,
                alignment: CombineAlignment::End,
                gap: 2,
                background: bg,
            },
        )
        .expect("vertical combine");

        assert_eq!(combined.width(), 4);
        assert_eq!(combined.height(), 5);
        assert_eq!(pixel_at(&combined, 0, 0), bg);
        assert_eq!(pixel_at(&combined, 1, 0), bg);
        assert_eq!(pixel_at(&combined, 2, 0), [0, 255, 0, 255]);
        assert_eq!(pixel_at(&combined, 3, 0), [0, 255, 0, 255]);
        assert_eq!(pixel_at(&combined, 2, 1), bg);
        assert_eq!(pixel_at(&combined, 2, 2), bg);
        assert_eq!(pixel_at(&combined, 0, 3), [255, 255, 255, 255]);
        assert_eq!(pixel_at(&combined, 3, 4), [255, 255, 255, 255]);
    }

    #[test]
    fn contact_sheet_validates_empty_input_and_zero_dimensions() {
        let img = solid(4, 4, [255, 0, 0, 255]);

        assert!(matches!(
            generate_contact_sheet(&[], &ContactSheetOptions::default()),
            Err(CoreError::InvalidRegion(_))
        ));

        let zero_cols = ContactSheetOptions {
            columns: 0,
            ..ContactSheetOptions::default()
        };
        assert!(matches!(
            zero_cols.validate(),
            Err(CoreError::InvalidRegion(_))
        ));
        assert!(matches!(
            generate_contact_sheet(std::slice::from_ref(&img), &zero_cols),
            Err(CoreError::InvalidRegion(_))
        ));

        let zero_width = ContactSheetOptions {
            thumb_width: 0,
            ..ContactSheetOptions::default()
        };
        assert!(matches!(
            zero_width.validate(),
            Err(CoreError::InvalidRegion(_))
        ));

        let zero_height = ContactSheetOptions {
            thumb_height: 0,
            ..ContactSheetOptions::default()
        };
        assert!(matches!(
            zero_height.validate(),
            Err(CoreError::InvalidRegion(_))
        ));
    }

    #[test]
    fn generates_five_image_two_column_contact_sheet_with_exact_layout_and_letterboxing() {
        let c0 = [255, 0, 0, 255];
        let c1 = [0, 255, 0, 255];
        let c2 = [0, 0, 255, 255];
        let c3 = [255, 255, 0, 255];
        let c4 = [255, 0, 255, 255];
        let bg = [12, 34, 56, 255];

        let images = vec![
            solid(10, 10, c0),
            solid(20, 10, c1),
            solid(10, 20, c2),
            solid(4, 4, c3),
            solid(8, 8, c4),
        ];

        let opts = ContactSheetOptions {
            columns: 2,
            thumb_width: 10,
            thumb_height: 10,
            gap: 2,
            padding: 3,
            background: bg,
        };

        let sheet = generate_contact_sheet(&images, &opts).expect("contact sheet");

        assert_eq!(sheet.width(), 3 * 2 + 2 * 10 + 2);
        assert_eq!(sheet.height(), 3 * 2 + 3 * 10 + 2 * 2);

        assert_eq!(pixel_at(&sheet, 0, 0), bg);
        assert_eq!(pixel_at(&sheet, 2, 2), bg);
        assert_eq!(pixel_at(&sheet, sheet.width() - 1, sheet.height() - 1), bg);

        assert_eq!(pixel_at(&sheet, 3, 3), c0);
        assert_eq!(pixel_at(&sheet, 12, 12), c0);

        assert_eq!(pixel_at(&sheet, 13, 5), bg);
        assert_eq!(pixel_at(&sheet, 14, 5), bg);

        assert_eq!(pixel_at(&sheet, 15, 3), bg);
        assert_eq!(pixel_at(&sheet, 15, 4), bg);
        assert_eq!(pixel_at(&sheet, 15, 5), c1);
        assert_eq!(pixel_at(&sheet, 24, 9), c1);
        assert_eq!(pixel_at(&sheet, 15, 10), bg);
        assert_eq!(pixel_at(&sheet, 15, 12), bg);

        assert_eq!(pixel_at(&sheet, 3, 13), bg);
        assert_eq!(pixel_at(&sheet, 3, 14), bg);

        assert_eq!(pixel_at(&sheet, 3, 15), bg);
        assert_eq!(pixel_at(&sheet, 4, 15), bg);
        assert_eq!(pixel_at(&sheet, 5, 15), c2);
        assert_eq!(pixel_at(&sheet, 9, 24), c2);
        assert_eq!(pixel_at(&sheet, 10, 15), bg);
        assert_eq!(pixel_at(&sheet, 12, 15), bg);

        assert_eq!(pixel_at(&sheet, 15, 15), c3);
        assert_eq!(pixel_at(&sheet, 24, 24), c3);

        assert_eq!(pixel_at(&sheet, 3, 27), c4);
        assert_eq!(pixel_at(&sheet, 12, 36), c4);

        assert_eq!(pixel_at(&sheet, 15, 27), bg);
        assert_eq!(pixel_at(&sheet, 24, 36), bg);
    }
}
