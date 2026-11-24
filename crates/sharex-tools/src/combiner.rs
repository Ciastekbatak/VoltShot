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
}
