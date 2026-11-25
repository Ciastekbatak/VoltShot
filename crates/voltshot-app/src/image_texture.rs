use eframe::egui::{Color32, ColorImage};
use voltshot_core::CapturedImage;

pub(crate) fn bounded_color_image(image: &CapturedImage, max_side: usize) -> ColorImage {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let largest = width.max(height);
    let max_side = max_side.max(1);
    if largest <= max_side {
        return ColorImage::from_rgba_unmultiplied([width, height], image.rgba());
    }

    let target_width = (width * max_side / largest).max(1);
    let target_height = (height * max_side / largest).max(1);
    let mut output = ColorImage::new([target_width, target_height], Color32::TRANSPARENT);
    for (row_index, row) in output.pixels.chunks_exact_mut(target_width).enumerate() {
        let source_row = row_index * height / target_height * width;
        for (column, pixel) in row.iter_mut().enumerate() {
            let offset = (source_row + column * width / target_width) * 4;
            let rgba = &image.rgba()[offset..offset + 4];
            *pixel = Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
        }
    }
    output
}

pub(crate) fn bounded_opaque_color_image(image: &CapturedImage, max_side: usize) -> ColorImage {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let largest = width.max(height);
    let max_side = max_side.max(1);
    if largest <= max_side {
        let pixels = image
            .rgba()
            .chunks_exact(4)
            .map(|px| Color32::from_rgba_premultiplied(px[0], px[1], px[2], 255))
            .collect();
        return ColorImage {
            size: [width, height],
            pixels,
        };
    }

    let target_width = (width * max_side / largest).max(1);
    let target_height = (height * max_side / largest).max(1);
    let mut pixels = Vec::with_capacity(target_width * target_height);
    let rgba_buf = image.rgba();
    for row_index in 0..target_height {
        let source_row = row_index * height / target_height * width;
        for column in 0..target_width {
            let offset = (source_row + column * width / target_width) * 4;
            pixels.push(Color32::from_rgba_premultiplied(
                rgba_buf[offset],
                rgba_buf[offset + 1],
                rgba_buf[offset + 2],
                255,
            ));
        }
    }
    ColorImage {
        size: [target_width, target_height],
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_desktop_preview_respects_texture_limit_without_cropping() {
        let row = [
            [255, 0, 0, 255],
            [0, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 0, 255],
        ];
        let full = CapturedImage::from_rgba(4, 2, row.concat().repeat(2)).unwrap();
        let preview = bounded_color_image(&full, 2);
        assert_eq!(preview.size, [2, 1]);
        assert_eq!(preview.pixels, [Color32::RED, Color32::GREEN]);
        assert_eq!([full.width(), full.height()], [4, 2]);
    }

    #[test]
    fn opaque_color_image_matches_full_and_downscaled_pixels() {
        let row = [
            [255, 0, 0, 255],
            [12, 34, 56, 255],
            [0, 255, 0, 255],
            [78, 90, 12, 255],
        ];
        let full = CapturedImage::from_rgba(4, 2, row.concat().repeat(2)).unwrap();
        let exact = bounded_opaque_color_image(&full, 8);
        assert_eq!(exact.size, [4, 2]);
        assert_eq!(exact.pixels[0], Color32::from_rgb(255, 0, 0));
        assert_eq!(exact.pixels[1], Color32::from_rgb(12, 34, 56));
        let scaled = bounded_opaque_color_image(&full, 2);
        assert_eq!(scaled.size, [2, 1]);
        assert_eq!(scaled.pixels, [Color32::RED, Color32::GREEN]);
    }
}
