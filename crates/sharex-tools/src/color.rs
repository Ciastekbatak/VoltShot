use serde::{Deserialize, Serialize};
use sharex_core::{CapturedImage, CoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColorSample {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ColorSample {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn from_hex(hex: &str) -> Result<Self, CoreError> {
        let trimmed = hex.trim();
        let raw = trimmed.strip_prefix('#').unwrap_or(trimmed);
        if !raw.is_ascii() || (raw.len() != 6 && raw.len() != 8) {
            return Err(CoreError::ConfigError(format!(
                "invalid hex color `{hex}`: expected `#RRGGBB` or `#RRGGBBAA`"
            )));
        }
        let parse_channel = |slice: &str| {
            u8::from_str_radix(slice, 16)
                .map_err(|_| CoreError::ConfigError(format!("invalid hex color `{hex}`")))
        };
        let r = parse_channel(&raw[0..2])?;
        let g = parse_channel(&raw[2..4])?;
        let b = parse_channel(&raw[4..6])?;
        let a = if raw.len() == 8 {
            parse_channel(&raw[6..8])?
        } else {
            255
        };
        Ok(Self { r, g, b, a })
    }

    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }

    pub fn to_rgb_string(self) -> String {
        if self.a == 255 {
            format!("rgb({}, {}, {})", self.r, self.g, self.b)
        } else {
            format!("rgba({}, {}, {}, {})", self.r, self.g, self.b, self.a)
        }
    }

    pub fn to_hsl(self) -> (f32, f32, f32) {
        let r = f32::from(self.r) / 255.0;
        let g = f32::from(self.g) / 255.0;
        let b = f32::from(self.b) / 255.0;

        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let l = (max + min) * 0.5;

        if delta == 0.0 {
            return (0.0, 0.0, l * 100.0);
        }

        let s = delta / (1.0 - (2.0 * l - 1.0).abs());
        let raw_h = if (max - r).abs() <= f32::EPSILON {
            ((g - b) / delta).rem_euclid(6.0)
        } else if (max - g).abs() <= f32::EPSILON {
            ((b - r) / delta) + 2.0
        } else {
            ((r - g) / delta) + 4.0
        };
        let mut h = raw_h * 60.0;
        if h >= 360.0 || h < 0.0 {
            h = h.rem_euclid(360.0);
        }

        (h, s * 100.0, l * 100.0)
    }

    pub fn to_cmyk(self) -> (f32, f32, f32, f32) {
        let r = f32::from(self.r) / 255.0;
        let g = f32::from(self.g) / 255.0;
        let b = f32::from(self.b) / 255.0;

        let k = 1.0 - r.max(g).max(b);
        if k >= 1.0 - f32::EPSILON {
            return (0.0, 0.0, 0.0, 100.0);
        }

        let denom = 1.0 - k;
        let c = ((1.0 - r - k) / denom) * 100.0;
        let m = ((1.0 - g - k) / denom) * 100.0;
        let y = ((1.0 - b - k) / denom) * 100.0;
        (c, m, y, k * 100.0)
    }

    pub const fn to_decimal(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }
}

pub fn sample_image(img: &CapturedImage, x: u32, y: u32) -> Result<ColorSample, CoreError> {
    if x >= img.width() || y >= img.height() {
        return Err(CoreError::RegionOutOfBounds {
            x: i32::try_from(x).unwrap_or(i32::MAX),
            y: i32::try_from(y).unwrap_or(i32::MAX),
            width: 1,
            height: 1,
            image_width: img.width(),
            image_height: img.height(),
        });
    }
    let idx = ((y as usize) * (img.width() as usize) + (x as usize)) * 4;
    let rgba = img.rgba();
    Ok(ColorSample {
        r: rgba[idx],
        g: rgba[idx + 1],
        b: rgba[idx + 2],
        a: rgba[idx + 3],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_approx(actual: f32, expected: f32, epsilon: f32) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "expected {expected} +/- {epsilon}, got {actual}"
        );
    }

    #[test]
    fn hex_parsing_and_formatting_roundtrip_rgb_and_rgba() {
        let opaque = ColorSample::from_hex("#1a2b3c").expect("valid rgb hex");
        assert_eq!(opaque, ColorSample::new(0x1A, 0x2B, 0x3C, 0xFF));
        assert_eq!(opaque.to_hex(), "#1A2B3C");
        assert_eq!(opaque.to_rgb_string(), "rgb(26, 43, 60)");
        assert_eq!(opaque.to_decimal(), 0x1A2B3C);

        let translucent = ColorSample::from_hex("#FF804080").expect("valid rgba hex");
        assert_eq!(translucent, ColorSample::new(255, 128, 64, 128));
        assert_eq!(translucent.to_hex(), "#FF804080");
        assert_eq!(translucent.to_rgb_string(), "rgba(255, 128, 64, 128)");

        assert!(ColorSample::from_hex("#FFF").is_err());
        assert!(ColorSample::from_hex("#ZZZZZZ").is_err());
        assert!(ColorSample::from_hex("").is_err());
        assert!(ColorSample::from_hex("#ąćęłńó").is_err());
    }

    #[test]
    fn hsl_conversions_match_known_reference_colors() {
        let (h, s, l) = ColorSample::rgb(255, 0, 0).to_hsl();
        assert_approx(h, 0.0, 0.01);
        assert_approx(s, 100.0, 0.01);
        assert_approx(l, 50.0, 0.01);

        let (h, s, l) = ColorSample::rgb(0, 255, 0).to_hsl();
        assert_approx(h, 120.0, 0.01);
        assert_approx(s, 100.0, 0.01);
        assert_approx(l, 50.0, 0.01);

        let (h, s, l) = ColorSample::rgb(0, 0, 255).to_hsl();
        assert_approx(h, 240.0, 0.01);
        assert_approx(s, 100.0, 0.01);
        assert_approx(l, 50.0, 0.01);

        let (h, s, l) = ColorSample::rgb(0, 255, 255).to_hsl();
        assert_approx(h, 180.0, 0.01);
        assert_approx(s, 100.0, 0.01);
        assert_approx(l, 50.0, 0.01);

        let (h, s, l) = ColorSample::rgb(255, 255, 255).to_hsl();
        assert_approx(h, 0.0, 0.01);
        assert_approx(s, 0.0, 0.01);
        assert_approx(l, 100.0, 0.01);

        let (h, s, l) = ColorSample::rgb(0, 0, 0).to_hsl();
        assert_approx(h, 0.0, 0.01);
        assert_approx(s, 0.0, 0.01);
        assert_approx(l, 0.0, 0.01);
    }

    #[test]
    fn cmyk_conversions_match_known_reference_colors() {
        let (c, m, y, k) = ColorSample::rgb(255, 0, 0).to_cmyk();
        assert_approx(c, 0.0, 0.01);
        assert_approx(m, 100.0, 0.01);
        assert_approx(y, 100.0, 0.01);
        assert_approx(k, 0.0, 0.01);

        let (c, m, y, k) = ColorSample::rgb(0, 255, 255).to_cmyk();
        assert_approx(c, 100.0, 0.01);
        assert_approx(m, 0.0, 0.01);
        assert_approx(y, 0.0, 0.01);
        assert_approx(k, 0.0, 0.01);

        let (c, m, y, k) = ColorSample::rgb(0, 0, 0).to_cmyk();
        assert_approx(c, 0.0, 0.01);
        assert_approx(m, 0.0, 0.01);
        assert_approx(y, 0.0, 0.01);
        assert_approx(k, 100.0, 0.01);

        let (c, m, y, k) = ColorSample::rgb(51, 102, 153).to_cmyk();
        assert_approx(c, 66.67, 0.05);
        assert_approx(m, 33.33, 0.05);
        assert_approx(y, 0.0, 0.05);
        assert_approx(k, 40.0, 0.05);
    }

    #[test]
    fn sample_image_reads_pixel_and_rejects_out_of_bounds() {
        let rgba = vec![
            10, 20, 30, 255, 40, 50, 60, 200, 70, 80, 90, 150, 100, 110, 120, 100,
        ];
        let img = CapturedImage::from_rgba(2, 2, rgba).expect("valid 2x2 image");

        assert_eq!(
            sample_image(&img, 1, 0).expect("in bounds"),
            ColorSample::new(40, 50, 60, 200)
        );
        assert_eq!(
            sample_image(&img, 1, 1).expect("in bounds"),
            ColorSample::new(100, 110, 120, 100)
        );
        assert!(matches!(
            sample_image(&img, 2, 0),
            Err(CoreError::RegionOutOfBounds { .. })
        ));
        assert!(matches!(
            sample_image(&img, 0, 2),
            Err(CoreError::RegionOutOfBounds { .. })
        ));
    }
}
