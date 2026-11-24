pub mod effects;
pub mod preset;

pub use effects::{EffectColor, ImageEffect, apply_effect, apply_effects};
pub use preset::{EffectPreset, builtin_presets};
