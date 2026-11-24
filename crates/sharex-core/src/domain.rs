use chrono::{DateTime, Utc};
use image::{DynamicImage, ImageFormat as EncoderFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Cursor;
use std::num::NonZeroU32;
use std::str::FromStr;
use thiserror::Error;

/// Errors returned across the ShareX core domain and port boundaries.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid capture region: {0}")]
    InvalidRegion(String),

    #[error(
        "invalid RGBA image buffer: expected {expected} bytes for {width}x{height}, got {actual}"
    )]
    InvalidImageBuffer {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },

    #[error(
        "crop region ({x},{y},{width}x{height}) exceeds image bounds ({image_width}x{image_height})"
    )]
    RegionOutOfBounds {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        image_width: u32,
        image_height: u32,
    },

    #[error("unsupported image format: {0}")]
    UnsupportedImageFormat(String),

    #[error("failed to encode image as {format}: {message}")]
    EncodeFailed {
        format: ImageFormat,
        message: String,
    },

    #[error("invalid filename pattern: {0}")]
    InvalidFileNamePattern(String),

    #[error("screen capture failed: {0}")]
    CaptureFailed(String),

    #[error("clipboard operation failed: {0}")]
    ClipboardFailed(String),

    #[error("uploader `{0}` is not registered")]
    UnknownUploader(String),

    #[error("upload failed: {0}")]
    UploadFailed(String),

    #[error("config error: {0}")]
    ConfigError(String),

    #[error("history error: {0}")]
    HistoryError(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Unique identifier assigned to a capture task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CaptureId(String);

impl CaptureId {
    pub fn from_timestamp(now: DateTime<Utc>, discriminator: u32) -> Self {
        Self(format!(
            "cap-{}-{:06x}",
            now.format("%Y%m%d%H%M%S%3f"),
            discriminator & 0x00ff_ffff
        ))
    }

    pub fn from_raw(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CaptureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Operating-system identifier for a physical or virtual monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MonitorId(u32);

impl MonitorId {
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    pub const fn value(self) -> u32 {
        self.0
    }
}

impl fmt::Display for MonitorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Display metadata reported by the screen capture backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub id: MonitorId,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
}

/// Validated non-empty rectangular screen region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    x: i32,
    y: i32,
    width: NonZeroU32,
    height: NonZeroU32,
}

impl Region {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Result<Self, CoreError> {
        let width = NonZeroU32::new(width)
            .ok_or_else(|| CoreError::InvalidRegion("width must be greater than zero".into()))?;
        let height = NonZeroU32::new(height)
            .ok_or_else(|| CoreError::InvalidRegion("height must be greater than zero".into()))?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Parses a comma-separated `x,y,width,height` specification.
    pub fn parse(spec: &str) -> Result<Self, CoreError> {
        let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
        if parts.len() != 4 {
            return Err(CoreError::InvalidRegion(format!(
                "expected `x,y,width,height`, got `{spec}`"
            )));
        }
        let x = parts[0].parse::<i32>().map_err(|_| {
            CoreError::InvalidRegion(format!("invalid x coordinate `{}`", parts[0]))
        })?;
        let y = parts[1].parse::<i32>().map_err(|_| {
            CoreError::InvalidRegion(format!("invalid y coordinate `{}`", parts[1]))
        })?;
        let width = parts[2]
            .parse::<u32>()
            .map_err(|_| CoreError::InvalidRegion(format!("invalid width `{}`", parts[2])))?;
        let height = parts[3]
            .parse::<u32>()
            .map_err(|_| CoreError::InvalidRegion(format!("invalid height `{}`", parts[3])))?;
        Self::new(x, y, width, height)
    }

    pub const fn x(self) -> i32 {
        self.x
    }

    pub const fn y(self) -> i32 {
        self.y
    }

    pub const fn width(self) -> u32 {
        self.width.get()
    }

    pub const fn height(self) -> u32 {
        self.height.get()
    }
}

/// Target surface to capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureSource {
    PrimaryMonitor,
    Monitor { id: MonitorId },
    Region { region: Region },
}

/// Output encoding format for saved or uploaded captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    pub const fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }
}

impl fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

impl FromStr for ImageFormat {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "png" => Ok(Self::Png),
            "jpg" | "jpeg" => Ok(Self::Jpeg),
            "webp" => Ok(Self::Webp),
            other => Err(CoreError::UnsupportedImageFormat(other.to_owned())),
        }
    }
}

/// In-memory RGBA8 image whose buffer length is guaranteed to match `width * height * 4`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedImage {
    width: NonZeroU32,
    height: NonZeroU32,
    rgba: Vec<u8>,
}

impl CapturedImage {
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, CoreError> {
        let width_nz = NonZeroU32::new(width).ok_or(CoreError::InvalidImageBuffer {
            width,
            height,
            expected: 0,
            actual: rgba.len(),
        })?;
        let height_nz = NonZeroU32::new(height).ok_or(CoreError::InvalidImageBuffer {
            width,
            height,
            expected: 0,
            actual: rgba.len(),
        })?;
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(4))
            .ok_or(CoreError::InvalidImageBuffer {
                width,
                height,
                expected: usize::MAX,
                actual: rgba.len(),
            })?;
        if rgba.len() != expected {
            return Err(CoreError::InvalidImageBuffer {
                width,
                height,
                expected,
                actual: rgba.len(),
            });
        }
        Ok(Self {
            width: width_nz,
            height: height_nz,
            rgba,
        })
    }

    pub const fn width(&self) -> u32 {
        self.width.get()
    }

    pub const fn height(&self) -> u32 {
        self.height.get()
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Returns a cropped sub-image using image-local non-negative coordinates.
    pub fn crop(&self, region: Region) -> Result<Self, CoreError> {
        if region.x() < 0 || region.y() < 0 {
            return Err(CoreError::RegionOutOfBounds {
                x: region.x(),
                y: region.y(),
                width: region.width(),
                height: region.height(),
                image_width: self.width(),
                image_height: self.height(),
            });
        }
        let start_x = region.x() as u32;
        let start_y = region.y() as u32;
        let end_x = start_x.checked_add(region.width());
        let end_y = start_y.checked_add(region.height());
        if end_x.is_none_or(|x| x > self.width()) || end_y.is_none_or(|y| y > self.height()) {
            return Err(CoreError::RegionOutOfBounds {
                x: region.x(),
                y: region.y(),
                width: region.width(),
                height: region.height(),
                image_width: self.width(),
                image_height: self.height(),
            });
        }

        let src_stride = (self.width() as usize) * 4;
        let row_bytes = (region.width() as usize) * 4;
        let mut out = Vec::with_capacity(row_bytes * (region.height() as usize));
        for row in start_y..(start_y + region.height()) {
            let offset = (row as usize) * src_stride + (start_x as usize) * 4;
            out.extend_from_slice(&self.rgba[offset..offset + row_bytes]);
        }
        Self::from_rgba(region.width(), region.height(), out)
    }

    /// Encodes the image into the requested container format.
    pub fn encode(&self, format: ImageFormat) -> Result<Vec<u8>, CoreError> {
        let rgba_img = RgbaImage::from_raw(self.width(), self.height(), self.rgba.clone())
            .ok_or_else(|| CoreError::EncodeFailed {
                format,
                message: "validated RGBA buffer could not construct RgbaImage".into(),
            })?;
        let dynamic = DynamicImage::ImageRgba8(rgba_img);
        let mut out = Vec::new();
        let mut cursor = Cursor::new(&mut out);

        let result = match format {
            ImageFormat::Png => dynamic.write_to(&mut cursor, EncoderFormat::Png),
            ImageFormat::Jpeg => DynamicImage::ImageRgb8(dynamic.to_rgb8())
                .write_to(&mut cursor, EncoderFormat::Jpeg),
            ImageFormat::Webp => dynamic.write_to(&mut cursor, EncoderFormat::WebP),
        };

        result.map_err(|err| CoreError::EncodeFailed {
            format,
            message: err.to_string(),
        })?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_rejects_zero_dimensions_and_parses_valid_spec() {
        assert!(matches!(
            Region::new(0, 0, 0, 10),
            Err(CoreError::InvalidRegion(_))
        ));
        assert!(matches!(
            Region::new(0, 0, 10, 0),
            Err(CoreError::InvalidRegion(_))
        ));

        let parsed = Region::parse("-10, 20, 640, 480").expect("valid region");
        assert_eq!(parsed.x(), -10);
        assert_eq!(parsed.y(), 20);
        assert_eq!(parsed.width(), 640);
        assert_eq!(parsed.height(), 480);

        assert!(matches!(
            Region::parse("10,20,30"),
            Err(CoreError::InvalidRegion(_))
        ));
    }

    #[test]
    fn captured_image_enforces_buffer_length_and_crops_accurately() {
        assert!(matches!(
            CapturedImage::from_rgba(2, 2, vec![0; 12]),
            Err(CoreError::InvalidImageBuffer {
                expected: 16,
                actual: 12,
                ..
            })
        ));

        let mut pixels = Vec::new();
        for y in 0..4u8 {
            for x in 0..4u8 {
                pixels.extend_from_slice(&[x * 10, y * 10, 100, 255]);
            }
        }
        let img = CapturedImage::from_rgba(4, 4, pixels).expect("valid 4x4 image");
        let cropped = img
            .crop(Region::new(1, 2, 2, 2).expect("valid crop"))
            .expect("crop succeeds");

        assert_eq!(cropped.width(), 2);
        assert_eq!(cropped.height(), 2);
        assert_eq!(&cropped.rgba()[0..4], &[10, 20, 100, 255]);
        assert_eq!(&cropped.rgba()[4..8], &[20, 20, 100, 255]);
        assert_eq!(&cropped.rgba()[8..12], &[10, 30, 100, 255]);
        assert_eq!(&cropped.rgba()[12..16], &[20, 30, 100, 255]);

        assert!(matches!(
            img.crop(Region::new(3, 0, 2, 2).expect("valid")),
            Err(CoreError::RegionOutOfBounds { .. })
        ));
    }

    #[test]
    fn captured_image_encodes_png_jpeg_and_webp_decodable_bytes() {
        let img =
            CapturedImage::from_rgba(2, 2, [255, 0, 0, 255].repeat(4)).expect("valid test image");

        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::Webp] {
            let encoded = img.encode(format).expect("encoding succeeds");
            let decoded = image::load_from_memory(&encoded).expect("encoded bytes decode cleanly");
            assert_eq!(decoded.width(), 2);
            assert_eq!(decoded.height(), 2);
        }
    }
}
