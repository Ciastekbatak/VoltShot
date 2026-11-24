use crate::ocr::{OcrEngine, OcrOptions, OcrResult};
use sharex_core::{CapturedImage, CoreError};

/// Normalizes a Tesseract or BCP-47 language tag into a single BCP-47 tag for WinRT OCR.
///
/// Multi-language specifications joined with `+` (such as `"eng+pol"`) return `None`,
/// signaling that the engine should fall back to Windows user-profile OCR languages.
pub fn normalize_bcp47_language_tag(tag: &str) -> Option<String> {
    let trimmed = tag.trim();
    if trimmed.is_empty() || trimmed.contains('+') {
        return None;
    }

    let lowered = trimmed.to_ascii_lowercase();
    let mapped = match lowered.as_str() {
        "eng" => Some("en"),
        "pol" => Some("pl"),
        "deu" | "ger" => Some("de"),
        "fra" | "fre" => Some("fr"),
        "spa" => Some("es"),
        "ita" => Some("it"),
        "por" => Some("pt"),
        "nld" | "dut" => Some("nl"),
        "rus" => Some("ru"),
        "ukr" => Some("uk"),
        "jpn" => Some("ja"),
        "kor" => Some("ko"),
        "chi_sim" | "chi-sim" => Some("zh-Hans"),
        "chi_tra" | "chi-tra" => Some("zh-Hant"),
        "ces" | "cze" => Some("cs"),
        "slk" | "slo" => Some("sk"),
        "swe" => Some("sv"),
        "nor" => Some("nb"),
        "dan" => Some("da"),
        "fin" => Some("fi"),
        "tur" => Some("tr"),
        "ara" => Some("ar"),
        "heb" => Some("he"),
        "hin" => Some("hi"),
        "vie" => Some("vi"),
        "tha" => Some("th"),
        "ell" | "gre" => Some("el"),
        "hun" => Some("hu"),
        "ron" | "rum" => Some("ro"),
        "bul" => Some("bg"),
        "hrv" => Some("hr"),
        "srp" => Some("sr"),
        _ => None,
    };

    if let Some(code) = mapped {
        return Some(code.to_owned());
    }

    Some(trimmed.replace('_', "-"))
}

/// Native Windows WinRT OCR engine backed by `Windows.Media.Ocr`.
#[derive(Debug, Clone, Copy, Default)]
pub struct WinRtOcr;

impl WinRtOcr {
    /// Creates a new Windows WinRT OCR engine instance.
    pub const fn new() -> Self {
        Self
    }
}

impl OcrEngine for WinRtOcr {
    fn name(&self) -> &'static str {
        "windows-winrt-ocr"
    }

    fn is_available(&self) -> bool {
        platform::is_available()
    }

    fn recognize(&self, image: &CapturedImage, opts: &OcrOptions) -> Result<OcrResult, CoreError> {
        platform::recognize(self.name(), image, opts)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::normalize_bcp47_language_tag;
    use crate::ocr::{OcrOptions, OcrResult};
    use sharex_core::{CapturedImage, CoreError};
    use std::sync::OnceLock;
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine as WinRtOcrEngine;
    use windows::Storage::Streams::DataWriter;
    use windows::Win32::System::Com::CoIncrementMTAUsage;
    use windows::core::HSTRING;

    fn ensure_mta() -> Result<(), CoreError> {
        static MTA_INIT: OnceLock<Result<(), String>> = OnceLock::new();
        match MTA_INIT.get_or_init(|| {
            unsafe { CoIncrementMTAUsage() }
                .map(|_cookie| ())
                .map_err(|err| format!("failed to initialize COM MTA for WinRT OCR: {err}"))
        }) {
            Ok(()) => Ok(()),
            Err(msg) => Err(CoreError::CaptureFailed(msg.clone())),
        }
    }

    fn try_create_engine(language_tag: &str) -> Result<WinRtOcrEngine, CoreError> {
        if let Some(bcp47) = normalize_bcp47_language_tag(language_tag) {
            if let Ok(language) = Language::CreateLanguage(&HSTRING::from(bcp47.as_str()))
                && let Ok(engine) = WinRtOcrEngine::TryCreateFromLanguage(&language)
            {
                return Ok(engine);
            }

            if let Ok(available) = WinRtOcrEngine::AvailableRecognizerLanguages() {
                let target_lower = bcp47.to_ascii_lowercase();
                let prefix = format!("{target_lower}-");
                for lang in available {
                    if let Ok(tag) = lang.LanguageTag() {
                        let candidate = tag.to_string().to_ascii_lowercase();
                        if (candidate == target_lower || candidate.starts_with(&prefix))
                            && let Ok(engine) = WinRtOcrEngine::TryCreateFromLanguage(&lang)
                        {
                            return Ok(engine);
                        }
                    }
                }
            }
        }

        WinRtOcrEngine::TryCreateFromUserProfileLanguages().map_err(|err| {
            CoreError::CaptureFailed(format!(
                "failed to create Windows WinRT OCR engine for `{language_tag}`: {err}"
            ))
        })
    }

    pub(super) fn is_available() -> bool {
        if ensure_mta().is_err() {
            return false;
        }
        WinRtOcrEngine::TryCreateFromUserProfileLanguages().is_ok()
    }

    pub(super) fn recognize(
        engine_name: &'static str,
        image: &CapturedImage,
        opts: &OcrOptions,
    ) -> Result<OcrResult, CoreError> {
        opts.validate()?;
        ensure_mta()?;

        let max_dim = WinRtOcrEngine::MaxImageDimension().map_err(|err| {
            CoreError::CaptureFailed(format!("failed to query WinRT OCR max dimension: {err}"))
        })?;
        if image.width() > max_dim || image.height() > max_dim {
            return Err(CoreError::InvalidRegion(format!(
                "image dimensions {}x{} exceed WinRT OCR maximum dimension {max_dim}",
                image.width(),
                image.height()
            )));
        }

        let width = i32::try_from(image.width()).map_err(|_| {
            CoreError::InvalidRegion("image width exceeds WinRT SoftwareBitmap i32 bounds".into())
        })?;
        let height = i32::try_from(image.height()).map_err(|_| {
            CoreError::InvalidRegion("image height exceeds WinRT SoftwareBitmap i32 bounds".into())
        })?;

        let engine = try_create_engine(&opts.language)?;

        let writer = DataWriter::new().map_err(|err| {
            CoreError::CaptureFailed(format!("failed to allocate WinRT DataWriter: {err}"))
        })?;
        writer.WriteBytes(image.rgba()).map_err(|err| {
            CoreError::CaptureFailed(format!("failed to write RGBA bytes to WinRT buffer: {err}"))
        })?;
        let buffer = writer.DetachBuffer().map_err(|err| {
            CoreError::CaptureFailed(format!("failed to detach WinRT IBuffer: {err}"))
        })?;

        let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
            &buffer,
            BitmapPixelFormat::Rgba8,
            width,
            height,
            BitmapAlphaMode::Premultiplied,
        )
        .map_err(|err| {
            CoreError::CaptureFailed(format!("failed to create WinRT SoftwareBitmap: {err}"))
        })?;

        let ocr_result = engine
            .RecognizeAsync(&bitmap)
            .map_err(|err| {
                CoreError::CaptureFailed(format!("failed to start WinRT OCR recognition: {err}"))
            })?
            .get()
            .map_err(|err| {
                CoreError::CaptureFailed(format!("WinRT OCR recognition failed: {err}"))
            })?;

        let lines = ocr_result.Lines().map_err(|err| {
            CoreError::CaptureFailed(format!("failed to read WinRT OCR result lines: {err}"))
        })?;
        let mut collected = Vec::new();
        for line in lines {
            let line_text = line.Text().map_err(|err| {
                CoreError::CaptureFailed(format!("failed to read WinRT OCR line text: {err}"))
            })?;
            collected.push(line_text.to_string());
        }

        let resolved_language = engine
            .RecognizerLanguage()
            .ok()
            .and_then(|lang| lang.LanguageTag().ok())
            .map(|tag| tag.to_string())
            .filter(|tag| !tag.is_empty())
            .unwrap_or_else(|| opts.language.clone());

        Ok(OcrResult {
            text: collected.join("\n"),
            language: resolved_language,
            engine_name,
        })
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use crate::ocr::{OcrOptions, OcrResult};
    use sharex_core::{CapturedImage, CoreError};

    pub(super) fn is_available() -> bool {
        false
    }

    pub(super) fn recognize(
        _engine_name: &'static str,
        _image: &CapturedImage,
        _opts: &OcrOptions,
    ) -> Result<OcrResult, CoreError> {
        Err(CoreError::CaptureFailed(
            "Windows WinRT OCR is only available on Windows".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::FallbackOcrChain;

    #[test]
    fn normalizes_tesseract_and_bcp47_language_tags() {
        assert_eq!(normalize_bcp47_language_tag("eng").as_deref(), Some("en"));
        assert_eq!(normalize_bcp47_language_tag("ENG").as_deref(), Some("en"));
        assert_eq!(normalize_bcp47_language_tag("pol").as_deref(), Some("pl"));
        assert_eq!(normalize_bcp47_language_tag("deu").as_deref(), Some("de"));
        assert_eq!(normalize_bcp47_language_tag("fra").as_deref(), Some("fr"));
        assert_eq!(
            normalize_bcp47_language_tag("chi_sim").as_deref(),
            Some("zh-Hans")
        );
        assert_eq!(
            normalize_bcp47_language_tag("en-US").as_deref(),
            Some("en-US")
        );
        assert_eq!(
            normalize_bcp47_language_tag("pl_PL").as_deref(),
            Some("pl-PL")
        );
        assert_eq!(normalize_bcp47_language_tag("eng+pol"), None);
        assert_eq!(normalize_bcp47_language_tag(""), None);
        assert_eq!(normalize_bcp47_language_tag("   "), None);
    }

    #[test]
    fn fallback_ocr_chain_default_orders_winrt_before_tesseract() {
        let chain = FallbackOcrChain::default();
        assert_eq!(
            chain.engine_names(),
            vec!["windows-winrt-ocr", "tesseract-cli"]
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn winrt_ocr_reports_unavailable_on_non_windows() {
        let engine = WinRtOcr::new();
        assert_eq!(engine.name(), "windows-winrt-ocr");
        assert!(!engine.is_available());
        let img = CapturedImage::from_rgba(2, 2, [255, 255, 255, 255].repeat(4)).expect("2x2");
        assert!(matches!(
            engine.recognize(&img, &OcrOptions::default()),
            Err(CoreError::CaptureFailed(_))
        ));
    }

    #[cfg(target_os = "windows")]
    fn glyph_5x7(ch: char) -> [u8; 7] {
        match ch {
            'H' => [
                0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
            ],
            'E' => [
                0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
            ],
            'L' => [
                0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
            ],
            'O' => [
                0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
            ],
            '2' => [
                0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111,
            ],
            '0' => [
                0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
            ],
            '6' => [
                0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
            ],
            _ => [0; 7],
        }
    }

    #[cfg(target_os = "windows")]
    fn render_banner_text(text: &str, scale: u32, margin: u32) -> CapturedImage {
        let char_w = 5 * scale;
        let char_h = 7 * scale;
        let spacing = 2 * scale;
        let chars: Vec<char> = text.chars().collect();
        let content_w =
            (chars.len() as u32) * char_w + (chars.len().saturating_sub(1) as u32) * spacing;
        let width = content_w + margin * 2;
        let height = char_h + margin * 2;
        let mut rgba = vec![255u8; (width * height * 4) as usize];

        for (ci, &ch) in chars.iter().enumerate() {
            let glyph = glyph_5x7(ch);
            let origin_x = margin + (ci as u32) * (char_w + spacing);
            for (gy, row_bits) in glyph.iter().enumerate() {
                for gx in 0..5u32 {
                    if (row_bits >> (4 - gx)) & 1 == 1 {
                        let px0 = origin_x + gx * scale;
                        let py0 = margin + (gy as u32) * scale;
                        for py in py0..py0 + scale {
                            for px in px0..px0 + scale {
                                let idx = ((py * width + px) * 4) as usize;
                                rgba[idx] = 0;
                                rgba[idx + 1] = 0;
                                rgba[idx + 2] = 0;
                                rgba[idx + 3] = 255;
                            }
                        }
                    }
                }
            }
        }

        CapturedImage::from_rgba(width, height, rgba).expect("valid synthetic OCR banner")
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn winrt_ocr_smoke_test_on_synthetic_high_contrast_text() {
        let engine = WinRtOcr::new();
        assert_eq!(engine.name(), "windows-winrt-ocr");
        if !engine.is_available() {
            return;
        }

        let image = render_banner_text("HELLO 2026", 8, 48);
        let result = engine
            .recognize(&image, &OcrOptions::default())
            .expect("WinRT OCR should succeed on synthetic banner");
        assert_eq!(result.engine_name, "windows-winrt-ocr");
        assert!(!result.language.is_empty());
        let normalized = result.text.to_ascii_uppercase();
        assert!(
            normalized.contains("HELLO") || normalized.contains("2026"),
            "expected OCR text to recognize banner content, got `{}`",
            result.text
        );
    }
}
