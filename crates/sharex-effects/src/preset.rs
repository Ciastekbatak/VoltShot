use serde::{Deserialize, Serialize};
use sharex_core::{CapturedImage, CoreError};

use crate::effects::{EffectColor, ImageEffect, WatermarkAnchor, apply_effects};

/// Named, serializable stack of image effects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectPreset {
    pub name: String,
    pub enabled: bool,
    pub effects: Vec<ImageEffect>,
}

impl EffectPreset {
    pub fn to_toml(&self) -> Result<String, CoreError> {
        toml::to_string_pretty(self).map_err(|err| CoreError::ConfigError(err.to_string()))
    }

    pub fn from_toml(input: &str) -> Result<Self, CoreError> {
        toml::from_str(input).map_err(|err| CoreError::ConfigError(err.to_string()))
    }

    pub fn to_json(&self) -> Result<String, CoreError> {
        serde_json::to_string_pretty(self).map_err(|err| CoreError::ConfigError(err.to_string()))
    }

    pub fn from_json(input: &str) -> Result<Self, CoreError> {
        serde_json::from_str(input).map_err(|err| CoreError::ConfigError(err.to_string()))
    }

    /// Applies the preset's effects when enabled, or returns an unchanged copy otherwise.
    pub fn apply(&self, image: &CapturedImage) -> Result<CapturedImage, CoreError> {
        if !self.enabled {
            return Ok(image.clone());
        }
        apply_effects(image, &self.effects)
    }
}

/// Returns built-in ShareX-style image effect presets.
pub fn builtin_presets() -> Vec<EffectPreset> {
    vec![
        EffectPreset {
            name: "Clean Screenshot".into(),
            enabled: true,
            effects: vec![
                ImageEffect::RoundedCorners { radius: 12 },
                ImageEffect::DropShadow {
                    offset_x: 0,
                    offset_y: 8,
                    blur_radius: 16,
                    color: EffectColor::rgba(0, 0, 0, 110),
                    background: EffectColor::TRANSPARENT,
                },
            ],
        },
        EffectPreset {
            name: "Docs Border".into(),
            enabled: true,
            effects: vec![
                ImageEffect::Border {
                    width: 2,
                    color: EffectColor::rgb(45, 55, 72),
                },
                ImageEffect::CanvasPadding {
                    top: 12,
                    right: 12,
                    bottom: 12,
                    left: 12,
                    color: EffectColor::WHITE,
                },
            ],
        },
        EffectPreset {
            name: "Grayscale Archive".into(),
            enabled: true,
            effects: vec![
                ImageEffect::Grayscale,
                ImageEffect::BrightnessContrast {
                    brightness: 0,
                    contrast: 15,
                },
            ],
        },
    ]
}

/// Returns the original presets and the opt-in Watermarked Share preset.
pub fn extended_presets() -> Vec<EffectPreset> {
    let mut presets = builtin_presets();
    presets.push(EffectPreset {
        name: "Watermarked Share".into(),
        enabled: true,
        effects: vec![ImageEffect::WatermarkText {
            text: "ShareX".into(),
            position: WatermarkAnchor::BottomRight,
            color: EffectColor::WHITE,
            opacity: 0.8,
            scale: 2,
            margin: 12,
            shadow: true,
        }],
    });
    presets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_variant_preset() -> EffectPreset {
        EffectPreset {
            name: "All Effects".into(),
            enabled: true,
            effects: vec![
                ImageEffect::Border {
                    width: 2,
                    color: EffectColor::rgb(30, 40, 50),
                },
                ImageEffect::CanvasPadding {
                    top: 4,
                    right: 6,
                    bottom: 8,
                    left: 10,
                    color: EffectColor::rgba(255, 255, 255, 240),
                },
                ImageEffect::RoundedCorners { radius: 8 },
                ImageEffect::DropShadow {
                    offset_x: -2,
                    offset_y: 6,
                    blur_radius: 10,
                    color: EffectColor::rgba(0, 0, 0, 128),
                    background: EffectColor::TRANSPARENT,
                },
                ImageEffect::Resize {
                    width: 320,
                    height: 240,
                    keep_aspect: true,
                },
                ImageEffect::Grayscale,
                ImageEffect::Invert,
                ImageEffect::BrightnessContrast {
                    brightness: -15,
                    contrast: 25,
                },
                ImageEffect::AutoCrop { tolerance: 3 },
                ImageEffect::WatermarkText {
                    text: "ShareX\n2026".into(),
                    position: WatermarkAnchor::MiddleRight,
                    color: EffectColor::WHITE,
                    opacity: 0.75,
                    scale: 2,
                    margin: 4,
                    shadow: true,
                },
                ImageEffect::Colorize {
                    color: EffectColor::rgb(30, 80, 255),
                    strength: 0.625,
                },
                ImageEffect::Slice {
                    min_height: 2,
                    max_height: 5,
                    shift: -3,
                },
                ImageEffect::ImageWatermark {
                    rgba: vec![255, 128, 32, 200, 12, 34, 56, 78],
                    width: 2,
                    height: 1,
                    position: WatermarkAnchor::BottomCenter,
                    opacity: 0.5,
                    margin: 6,
                },
            ],
        }
    }

    #[test]
    fn toml_and_json_round_trip_preserve_all_variants() {
        let preset = full_variant_preset();

        let toml_text = preset.to_toml().expect("TOML serialization succeeds");
        let from_toml = EffectPreset::from_toml(&toml_text).expect("TOML deserialization succeeds");
        assert_eq!(from_toml, preset);

        let json_text = preset.to_json().expect("JSON serialization succeeds");
        let from_json = EffectPreset::from_json(&json_text).expect("JSON deserialization succeeds");
        assert_eq!(from_json, preset);
        for tag in ["watermark_text", "colorize", "slice", "image_watermark"] {
            assert!(toml_text.contains(&format!("type = \"{tag}\"")));
            assert!(json_text.contains(&format!("\"type\": \"{tag}\"")));
        }
        assert!(toml_text.contains("position = \"middle_right\""));
        assert!(json_text.contains("\"position\": \"bottom_center\""));
    }

    #[test]
    fn extended_presets_add_an_opt_in_watermark_without_changing_builtins() {
        let presets = extended_presets();
        assert_eq!(presets.len(), 4);
        assert_eq!(&presets[..3], builtin_presets());
        let preset = &presets[3];
        assert_eq!(preset.name, "Watermarked Share");
        let image = CapturedImage::from_rgba(128, 60, vec![0; 128 * 60 * 4]).unwrap();
        let watermarked = preset.apply(&image).unwrap();
        assert_eq!((watermarked.width(), watermarked.height()), (128, 60));
        assert!(watermarked.rgba().chunks_exact(4).any(|pixel| pixel[3] > 0));
    }

    #[test]
    fn builtin_presets_serialize_and_apply_cleanly() {
        let presets = builtin_presets();
        assert_eq!(presets.len(), 3);
        assert_eq!(presets[0].name, "Clean Screenshot");
        assert_eq!(presets[1].name, "Docs Border");
        assert_eq!(presets[2].name, "Grayscale Archive");

        let img = CapturedImage::from_rgba(24, 16, [120, 180, 240, 255].repeat(24 * 16))
            .expect("valid test image");

        for preset in &presets {
            let round_tripped =
                EffectPreset::from_toml(&preset.to_toml().expect("to_toml")).expect("from_toml");
            assert_eq!(&round_tripped, preset);

            let rendered = preset.apply(&img).expect("preset applies cleanly");
            assert!(rendered.width() > 0);
            assert!(rendered.height() > 0);
        }

        let mut disabled = presets[1].clone();
        disabled.enabled = false;
        assert_eq!(disabled.apply(&img).expect("disabled preset"), img);
    }

    #[test]
    fn invalid_preset_markup_returns_config_error() {
        assert!(matches!(
            EffectPreset::from_toml("not valid = [toml"),
            Err(CoreError::ConfigError(_))
        ));
        assert!(matches!(
            EffectPreset::from_json("{invalid json"),
            Err(CoreError::ConfigError(_))
        ));
    }
}
