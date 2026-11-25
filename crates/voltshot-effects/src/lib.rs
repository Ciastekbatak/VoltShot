pub mod effects;
pub mod preset;
mod resize;
mod watermark;

pub use effects::{
    EffectColor, ImageEffect, RgbaColor, WatermarkAnchor, apply_effect, apply_effects,
};
pub use preset::{EffectPreset, builtin_presets, extended_presets};
