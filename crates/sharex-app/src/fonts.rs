use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::fs;
use std::path::Path;
use std::sync::Arc;

const PROPORTIONAL_CANDIDATES: &[&str] = &[
    #[cfg(target_os = "windows")]
    r"C:\Windows\Fonts\segoeui.ttf",
    #[cfg(target_os = "windows")]
    r"C:\Windows\Fonts\arial.ttf",
    #[cfg(target_os = "macos")]
    "/System/Library/Fonts/SFNS.ttf",
    #[cfg(target_os = "macos")]
    "/System/Library/Fonts/Helvetica.ttc",
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
];

const MONOSPACE_CANDIDATES: &[&str] = &[
    #[cfg(target_os = "windows")]
    r"C:\Windows\Fonts\CascadiaMono.ttf",
    #[cfg(target_os = "windows")]
    r"C:\Windows\Fonts\consola.ttf",
    #[cfg(target_os = "macos")]
    "/System/Library/Fonts/SFNSMono.ttf",
    #[cfg(target_os = "macos")]
    "/System/Library/Fonts/Menlo.ttc",
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
];

#[cfg(target_os = "windows")]
const EXTENDED_FALLBACKS: &[(&str, &str)] = &[
    ("fallback_cjk_sc", r"C:\Windows\Fonts\msyh.ttc"),
    ("fallback_cjk_jp", r"C:\Windows\Fonts\YuGothR.ttc"),
    ("fallback_cjk_kr", r"C:\Windows\Fonts\malgun.ttf"),
    ("fallback_thai_ar", r"C:\Windows\Fonts\tahoma.ttf"),
];

#[cfg(target_os = "macos")]
const EXTENDED_FALLBACKS: &[(&str, &str)] =
    &[("fallback_cjk", "/System/Library/Fonts/PingFang.ttc")];

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const EXTENDED_FALLBACKS: &[(&str, &str)] = &[(
    "fallback_cjk",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
)];

/// Installs compact native OS UI and monospace fonts into `egui`.
pub fn install_native_fonts(ctx: &egui::Context) {
    install_fonts_with_fallbacks(ctx, false);
}

/// Loads extended CJK, Thai, and Arabic fallback fonts lazily when needed by the selected language.
pub fn install_extended_unicode_fonts(ctx: &egui::Context) {
    install_fonts_with_fallbacks(ctx, true);
}

fn install_fonts_with_fallbacks(ctx: &egui::Context, include_extended: bool) {
    ctx.set_fonts(build_font_definitions(include_extended));
}

fn build_font_definitions(include_extended: bool) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();

    if let Some(bytes) = load_first_existing(PROPORTIONAL_CANDIDATES) {
        fonts.font_data.insert(
            "native_ui".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        for list in fonts.families.values_mut() {
            list.retain(|name| name != "Ubuntu-Light");
        }
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "native_ui".to_owned());
        fonts.font_data.remove("Ubuntu-Light");
    }

    if let Some(bytes) = load_first_existing(MONOSPACE_CANDIDATES) {
        fonts.font_data.insert(
            "native_mono".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        for list in fonts.families.values_mut() {
            list.retain(|name| name != "Hack");
        }
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "native_mono".to_owned());
        fonts.font_data.remove("Hack");
    }

    if include_extended {
        for &(key, path) in EXTENDED_FALLBACKS {
            if let Some(bytes) = load_first_existing(&[path]) {
                fonts
                    .font_data
                    .insert(key.to_owned(), Arc::new(FontData::from_owned(bytes)));
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .push(key.to_owned());
            }
        }
    }

    fonts
}

fn load_first_existing(candidates: &[&str]) -> Option<Vec<u8>> {
    candidates
        .iter()
        .map(Path::new)
        .find_map(|path| fs::read(path).ok().filter(|bytes| !bytes.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_first_existing_skips_missing_paths() {
        assert!(load_first_existing(&["/definitely/missing/font.ttf"]).is_none());
    }

    #[test]
    fn every_referenced_family_font_exists_in_font_data() {
        for extended in [false, true] {
            let defs = build_font_definitions(extended);
            for (family, names) in &defs.families {
                assert!(!names.is_empty(), "empty font family {family:?}");
                for name in names {
                    assert!(
                        defs.font_data.contains_key(name),
                        "family {family:?} references missing font '{name}'"
                    );
                }
            }
        }
    }
}
