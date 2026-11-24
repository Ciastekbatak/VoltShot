use serde::{Deserialize, Serialize};
use sharex_core::{CapturedImage, CoreError};

use crate::effects::{EffectColor, ImageEffect, apply_effects};

/// Named, serializable stack of image effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
