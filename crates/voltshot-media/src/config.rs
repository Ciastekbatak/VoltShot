use crate::ffmpeg::MediaError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use voltshot_core::Region;

pub const MAX_RECORDING_FPS: u32 = 240;
pub const MAX_CRF: u8 = 51;
/// Minimum region width and height for hardware H.264 encoding.
pub const MIN_HARDWARE_ENCODER_SIDE: u32 = 160;

/// Returns whether the region meets the hardware encoder size threshold.
pub fn hardware_encoder_supported_for_region(region: Option<Region>) -> bool {
    region.is_none_or(|region| {
        region.width() >= MIN_HARDWARE_ENCODER_SIDE && region.height() >= MIN_HARDWARE_ENCODER_SIDE
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingFormat {
    #[default]
    Mp4H264,
    WebmVp9,
    Gif,
}

impl RecordingFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp4H264 => "mp4",
            Self::WebmVp9 => "webm",
            Self::Gif => "gif",
        }
    }
}

impl fmt::Display for RecordingFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoEncoder {
    #[default]
    Auto,
    Libx264,
    NvencH264,
    QsvH264,
    AmfH264,
    LibvpxVp9,
    GifPalette,
}

impl VideoEncoder {
    pub(crate) const fn is_hardware_h264(self) -> bool {
        matches!(self, Self::NvencH264 | Self::QsvH264 | Self::AmfH264)
    }

    /// Resolves `Auto` to a concrete encoder and validates compatibility with `format`.
    pub fn resolve(self, format: RecordingFormat) -> Result<Self, MediaError> {
        match (format, self) {
            (RecordingFormat::Mp4H264, Self::Auto | Self::Libx264) => Ok(Self::Libx264),
            (RecordingFormat::Mp4H264, Self::NvencH264) => Ok(Self::NvencH264),
            (RecordingFormat::Mp4H264, Self::QsvH264) => Ok(Self::QsvH264),
            (RecordingFormat::Mp4H264, Self::AmfH264) => Ok(Self::AmfH264),
            (RecordingFormat::WebmVp9, Self::Auto | Self::LibvpxVp9) => Ok(Self::LibvpxVp9),
            (RecordingFormat::Gif, Self::Auto | Self::GifPalette) => Ok(Self::GifPalette),
            _ => Err(MediaError::InvalidConfig(format!(
                "encoder `{self:?}` is incompatible with format `{format}`"
            ))),
        }
    }
}

/// Audio input device discovered from a platform FFmpeg capture backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct AudioDevice {
    /// Human-readable device name passed to FFmpeg.
    pub name: String,
    /// Capture backend identifier (for example `"dshow"` or `"avfoundation"`).
    pub backend: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturePlatform {
    WindowsGdiGrab,
    LinuxX11Grab,
    MacOsAvFoundation,
}

impl CapturePlatform {
    pub const fn current() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::WindowsGdiGrab
        }
        #[cfg(target_os = "macos")]
        {
            Self::MacOsAvFoundation
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            Self::LinuxX11Grab
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingConfig {
    pub region: Option<Region>,
    pub fps: u32,
    pub format: RecordingFormat,
    pub encoder: VideoEncoder,
    pub crf: u8,
    pub draw_cursor: bool,
    pub output_path: PathBuf,
    pub ffmpeg_path: Option<PathBuf>,
    #[serde(default)]
    pub audio_device: Option<String>,
    /// Retry with libx264 if the selected hardware encoder fails during startup.
    #[serde(default)]
    pub fallback_to_libx264_on_hw_fail: bool,
}

impl RecordingConfig {
    pub fn new(output_path: impl Into<PathBuf>) -> Self {
        Self {
            region: None,
            fps: 30,
            format: RecordingFormat::Mp4H264,
            encoder: VideoEncoder::Auto,
            crf: 23,
            draw_cursor: true,
            output_path: output_path.into(),
            ffmpeg_path: None,
            audio_device: None,
            fallback_to_libx264_on_hw_fail: false,
        }
    }

    pub(crate) fn resolved_encoder(&self) -> Result<VideoEncoder, MediaError> {
        let encoder = self.encoder.resolve(self.format)?;
        if encoder.is_hardware_h264() && !hardware_encoder_supported_for_region(self.region) {
            Ok(VideoEncoder::Libx264)
        } else {
            Ok(encoder)
        }
    }

    pub fn validate(&self) -> Result<(), MediaError> {
        if self.fps == 0 || self.fps > MAX_RECORDING_FPS {
            return Err(MediaError::InvalidConfig(format!(
                "fps must be between 1 and {MAX_RECORDING_FPS}, got {}",
                self.fps
            )));
        }
        if self.crf > MAX_CRF {
            return Err(MediaError::InvalidConfig(format!(
                "crf must be between 0 and {MAX_CRF}, got {}",
                self.crf
            )));
        }
        if self.output_path.as_os_str().is_empty()
            || self
                .output_path
                .to_str()
                .is_some_and(|text| text.trim().is_empty())
        {
            return Err(MediaError::InvalidConfig(
                "output_path must not be empty".to_owned(),
            ));
        }
        if let Some(device) = &self.audio_device
            && device.chars().any(|ch| matches!(ch, '\0' | '\n' | '\r'))
        {
            return Err(MediaError::InvalidConfig(
                "audio_device must not contain NUL or newline characters".to_owned(),
            ));
        }
        self.encoder.resolve(self.format)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_format_extension_display_and_serde_round_trip() {
        assert_eq!(RecordingFormat::Mp4H264.extension(), "mp4");
        assert_eq!(RecordingFormat::WebmVp9.extension(), "webm");
        assert_eq!(RecordingFormat::Gif.extension(), "gif");

        assert_eq!(RecordingFormat::Mp4H264.to_string(), "mp4");
        assert_eq!(RecordingFormat::WebmVp9.to_string(), "webm");
        assert_eq!(RecordingFormat::Gif.to_string(), "gif");

        for format in [
            RecordingFormat::Mp4H264,
            RecordingFormat::WebmVp9,
            RecordingFormat::Gif,
        ] {
            let encoded = serde_json::to_string(&format).expect("serialize format");
            let decoded: RecordingFormat =
                serde_json::from_str(&encoded).expect("deserialize format");
            assert_eq!(decoded, format);
        }
    }

    #[test]
    fn recording_config_validate_accepts_valid_boundaries() {
        let mut config = RecordingConfig::new("capture.mp4");
        config.fps = 1;
        config.crf = 0;
        assert!(config.validate().is_ok());

        config.fps = MAX_RECORDING_FPS;
        config.crf = MAX_CRF;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn recording_config_validate_rejects_invalid_fps() {
        let mut zero_fps = RecordingConfig::new("capture.mp4");
        zero_fps.fps = 0;
        assert!(matches!(
            zero_fps.validate(),
            Err(MediaError::InvalidConfig(msg)) if msg.contains("fps")
        ));

        let mut high_fps = RecordingConfig::new("capture.mp4");
        high_fps.fps = 241;
        assert!(matches!(
            high_fps.validate(),
            Err(MediaError::InvalidConfig(msg)) if msg.contains("fps")
        ));
    }

    #[test]
    fn recording_config_validate_rejects_invalid_crf() {
        let mut config = RecordingConfig::new("capture.mp4");
        config.crf = 52;
        assert!(matches!(
            config.validate(),
            Err(MediaError::InvalidConfig(msg)) if msg.contains("crf")
        ));
    }

    #[test]
    fn recording_config_validate_rejects_empty_output_path() {
        let empty = RecordingConfig::new("");
        assert!(matches!(
            empty.validate(),
            Err(MediaError::InvalidConfig(msg)) if msg.contains("output_path")
        ));

        let blank = RecordingConfig::new("   ");
        assert!(matches!(
            blank.validate(),
            Err(MediaError::InvalidConfig(msg)) if msg.contains("output_path")
        ));
    }

    #[test]
    fn recording_config_validate_rejects_incompatible_encoder_and_format() {
        let mut config = RecordingConfig::new("clip.gif");
        config.format = RecordingFormat::Gif;
        config.encoder = VideoEncoder::Libx264;
        assert!(matches!(
            config.validate(),
            Err(MediaError::InvalidConfig(msg)) if msg.contains("incompatible")
        ));
    }

    #[test]
    fn capture_platform_current_matches_target_os() {
        let platform = CapturePlatform::current();
        #[cfg(target_os = "windows")]
        assert_eq!(platform, CapturePlatform::WindowsGdiGrab);
        #[cfg(target_os = "macos")]
        assert_eq!(platform, CapturePlatform::MacOsAvFoundation);
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        assert_eq!(platform, CapturePlatform::LinuxX11Grab);
    }

    #[test]
    fn hardware_h264_encoders_resolve_for_mp4_and_round_trip_serde() {
        for encoder in [
            VideoEncoder::NvencH264,
            VideoEncoder::QsvH264,
            VideoEncoder::AmfH264,
        ] {
            assert_eq!(
                encoder
                    .resolve(RecordingFormat::Mp4H264)
                    .expect("valid mp4 encoder"),
                encoder
            );
            assert!(encoder.resolve(RecordingFormat::WebmVp9).is_err());
            assert!(encoder.resolve(RecordingFormat::Gif).is_err());

            let encoded = serde_json::to_string(&encoder).expect("serialize encoder");
            let decoded: VideoEncoder = serde_json::from_str(&encoded).expect("deserialize");
            assert_eq!(decoded, encoder);
        }
    }

    #[test]
    fn recording_config_audio_device_defaults_to_none_and_rejects_control_chars() {
        let config = RecordingConfig::new("capture.mp4");
        assert_eq!(config.audio_device, None);

        let legacy_json = r#"{
            "region": null,
            "fps": 30,
            "format": "mp4_h264",
            "encoder": "auto",
            "crf": 23,
            "draw_cursor": true,
            "output_path": "capture.mp4",
            "ffmpeg_path": null
        }"#;
        let decoded: RecordingConfig =
            serde_json::from_str(legacy_json).expect("deserialize without audio_device");
        assert_eq!(decoded.audio_device, None);

        for bad in ["mic\n-f lavfi", "mic\r-i test", "mic\0null"] {
            let mut invalid = RecordingConfig::new("capture.mp4");
            invalid.audio_device = Some(bad.to_owned());
            assert!(matches!(
                invalid.validate(),
                Err(MediaError::InvalidConfig(msg)) if msg.contains("audio_device")
            ));
        }
    }
}
