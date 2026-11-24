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

/// Installs crisp native OS UI and monospace fonts into `egui` with automatic fallback.
pub fn install_native_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    if let Some(bytes) = load_first_existing(PROPORTIONAL_CANDIDATES) {
        fonts.font_data.insert(
            "native_ui".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "native_ui".to_owned());
    }

    if let Some(bytes) = load_first_existing(MONOSPACE_CANDIDATES) {
        fonts.font_data.insert(
            "native_mono".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "native_mono".to_owned());
    }

    ctx.set_fonts(fonts);
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
}
