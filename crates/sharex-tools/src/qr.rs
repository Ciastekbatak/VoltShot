use qrcode::{Color, QrCode};
use sharex_core::{CapturedImage, CoreError};

pub fn generate_qr_image(
    text: &str,
    module_px: u32,
    quiet_zone: u32,
) -> Result<CapturedImage, CoreError> {
    if text.is_empty() {
        return Err(CoreError::InvalidRegion(
            "QR code text must not be empty".into(),
        ));
    }
    if module_px == 0 {
        return Err(CoreError::InvalidRegion(
            "QR code module_px must be greater than zero".into(),
        ));
    }

    let code = QrCode::new(text.as_bytes())
        .map_err(|err| CoreError::InvalidRegion(format!("failed to encode QR code: {err}")))?;
    let qr_modules = u32::try_from(code.width())
        .map_err(|_| CoreError::InvalidRegion("QR code width exceeds u32".into()))?;
    let border_modules = quiet_zone
        .checked_mul(2)
        .ok_or_else(|| CoreError::InvalidRegion("QR quiet_zone overflows u32".into()))?;
    let total_modules = qr_modules
        .checked_add(border_modules)
        .ok_or_else(|| CoreError::InvalidRegion("QR total module count overflows u32".into()))?;
    let size = total_modules
        .checked_mul(module_px)
        .ok_or_else(|| CoreError::InvalidRegion("QR image dimensions overflow u32".into()))?;

    let pixel_count = (size as usize)
        .checked_mul(size as usize)
        .ok_or_else(|| CoreError::InvalidRegion("QR image pixel count overflows".into()))?;
    let byte_len = pixel_count
        .checked_mul(4)
        .ok_or_else(|| CoreError::InvalidRegion("QR image buffer size overflows".into()))?;

    let mut rgba = vec![255u8; byte_len];
    let stride = (size as usize) * 4;

    for my in 0..qr_modules {
        for mx in 0..qr_modules {
            if code[(mx as usize, my as usize)] == Color::Dark {
                let start_x = ((mx + quiet_zone) * module_px) as usize;
                let start_y = ((my + quiet_zone) * module_px) as usize;
                for py in start_y..start_y + (module_px as usize) {
                    let row_offset = py * stride;
                    for px in start_x..start_x + (module_px as usize) {
                        let idx = row_offset + px * 4;
                        rgba[idx] = 0;
                        rgba[idx + 1] = 0;
                        rgba[idx + 2] = 0;
                        rgba[idx + 3] = 255;
                    }
                }
            }
        }
    }

    CapturedImage::from_rgba(size, size, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn rejects_empty_text_or_zero_module_size() {
        assert!(matches!(
            generate_qr_image("", 4, 2),
            Err(CoreError::InvalidRegion(_))
        ));
        assert!(matches!(
            generate_qr_image("https://getsharex.com", 0, 2),
            Err(CoreError::InvalidRegion(_))
        ));
    }

    #[test]
    fn renders_expected_dimensions_quiet_zone_and_finder_pattern() {
        let text = "https://getsharex.com";
        let module_px = 4;
        let quiet_zone = 2;
        let expected_modules = QrCode::new(text.as_bytes()).expect("valid qr").width() as u32;
        let expected_size = (expected_modules + quiet_zone * 2) * module_px;

        let img = generate_qr_image(text, module_px, quiet_zone).expect("generate qr");
        assert_eq!(img.width(), expected_size);
        assert_eq!(img.height(), expected_size);

        let white = [255, 255, 255, 255];
        let black = [0, 0, 0, 255];

        assert_eq!(pixel_at(&img, 0, 0), white);
        assert_eq!(pixel_at(&img, expected_size - 1, expected_size - 1), white);

        let origin = quiet_zone * module_px;
        assert_eq!(pixel_at(&img, origin, origin), black);
        assert_eq!(
            pixel_at(&img, origin + module_px, origin + module_px),
            white
        );
        assert_eq!(
            pixel_at(&img, origin + 2 * module_px, origin + 2 * module_px),
            black
        );
        assert_eq!(
            pixel_at(&img, origin + 6 * module_px, origin + 6 * module_px),
            black
        );

        for chunk in img.rgba().chunks_exact(4) {
            assert!(chunk == black || chunk == white);
        }
    }
}
