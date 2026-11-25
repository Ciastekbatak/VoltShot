use crate::ocr_winrt::WinRtOcr;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use voltshot_core::{CapturedImage, CoreError, ImageFormat};

const TESSERACT_BINARY: &str = if cfg!(windows) {
    "tesseract.exe"
} else {
    "tesseract"
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrOptions {
    pub language: String,
    pub psm: Option<u8>,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            language: "eng".to_owned(),
            psm: None,
        }
    }
}

impl OcrOptions {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.language.is_empty() {
            return Err(CoreError::ConfigError(
                "OCR language tag must not be empty".into(),
            ));
        }
        if self.language.chars().any(char::is_whitespace) {
            return Err(CoreError::ConfigError(format!(
                "OCR language tag `{}` must not contain whitespace",
                self.language
            )));
        }
        for segment in self.language.split('+') {
            let mut chars = segment.chars();
            let Some(first) = chars.next() else {
                return Err(CoreError::ConfigError(format!(
                    "OCR language tag `{}` contains an empty segment",
                    self.language
                )));
            };
            if !first.is_ascii_alphanumeric()
                || !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            {
                return Err(CoreError::ConfigError(format!(
                    "OCR language tag `{}` contains invalid characters",
                    self.language
                )));
            }
        }
        if let Some(psm) = self.psm.filter(|&psm| psm > 13) {
            return Err(CoreError::ConfigError(format!(
                "OCR page segmentation mode {psm} is out of range (expected 0..=13)"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcrResult {
    pub text: String,
    pub language: String,
    pub engine_name: &'static str,
}

pub trait OcrEngine: Send + Sync {
    fn name(&self) -> &'static str;
    fn is_available(&self) -> bool;
    fn recognize(&self, image: &CapturedImage, opts: &OcrOptions) -> Result<OcrResult, CoreError>;
}

pub fn find_tesseract(override_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = override_path {
        if path.is_file() {
            return Some(path.to_path_buf());
        }
        if path.is_dir() {
            let candidate = path.join(TESSERACT_BINARY);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        return None;
    }

    let path_var = std::env::var_os("PATH")?;
    find_in_paths(std::env::split_paths(&path_var))
}

fn find_in_paths(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    for dir in paths {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(TESSERACT_BINARY);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn build_tesseract_args(input_png: &Path, opts: &OcrOptions) -> Result<Vec<String>, CoreError> {
    opts.validate()?;
    let input = input_png
        .to_str()
        .ok_or_else(|| CoreError::ConfigError("OCR input image path is not valid UTF-8".into()))?;
    if input.is_empty() {
        return Err(CoreError::ConfigError(
            "OCR input image path must not be empty".into(),
        ));
    }

    let mut args = vec![
        input.to_owned(),
        "stdout".to_owned(),
        "-l".to_owned(),
        opts.language.clone(),
    ];
    if let Some(psm) = opts.psm {
        args.push("--psm".to_owned());
        args.push(psm.to_string());
    }
    Ok(args)
}

#[derive(Debug, Clone, Default)]
pub struct TesseractCliOcr {
    binary_override: Option<PathBuf>,
}

impl TesseractCliOcr {
    pub fn new(binary_override: Option<PathBuf>) -> Self {
        Self { binary_override }
    }

    pub fn find_tesseract(override_path: Option<&Path>) -> Option<PathBuf> {
        find_tesseract(override_path)
    }

    pub fn binary_override(&self) -> Option<&Path> {
        self.binary_override.as_deref()
    }
}

impl OcrEngine for TesseractCliOcr {
    fn name(&self) -> &'static str {
        "tesseract-cli"
    }

    fn is_available(&self) -> bool {
        find_tesseract(self.binary_override.as_deref()).is_some()
    }

    fn recognize(&self, image: &CapturedImage, opts: &OcrOptions) -> Result<OcrResult, CoreError> {
        opts.validate()?;
        let binary = find_tesseract(self.binary_override.as_deref()).ok_or_else(|| match &self
            .binary_override
        {
            Some(path) => CoreError::CaptureFailed(format!(
                "tesseract binary override not found at `{}`",
                path.display()
            )),
            None => CoreError::CaptureFailed("tesseract executable not found on PATH".into()),
        })?;

        let png_bytes = image.encode(ImageFormat::Png)?;
        let temp_png = TempPngFile::create(&png_bytes)?;
        let args = build_tesseract_args(temp_png.path(), opts)?;

        let mut command = Command::new(&binary);
        command.args(&args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let output = command.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let status = output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated by signal".to_owned());
            let detail = if stderr.is_empty() {
                format!("tesseract exited with status {status}")
            } else {
                format!("tesseract exited with status {status}: {stderr}")
            };
            return Err(CoreError::CaptureFailed(detail));
        }

        let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok(OcrResult {
            text,
            language: opts.language.clone(),
            engine_name: self.name(),
        })
    }
}

pub struct FallbackOcrChain {
    engines: Vec<Box<dyn OcrEngine>>,
}

impl FallbackOcrChain {
    pub fn new(engines: Vec<Box<dyn OcrEngine>>) -> Self {
        Self { engines }
    }

    /// Returns the ordered list of configured OCR engine names.
    pub fn engine_names(&self) -> Vec<&'static str> {
        self.engines.iter().map(|engine| engine.name()).collect()
    }
}

impl Default for FallbackOcrChain {
    fn default() -> Self {
        Self::new(vec![
            Box::new(WinRtOcr::new()),
            Box::new(TesseractCliOcr::new(None)),
        ])
    }
}

impl OcrEngine for FallbackOcrChain {
    fn name(&self) -> &'static str {
        "fallback-ocr-chain"
    }

    fn is_available(&self) -> bool {
        self.engines.iter().any(|engine| engine.is_available())
    }

    fn recognize(&self, image: &CapturedImage, opts: &OcrOptions) -> Result<OcrResult, CoreError> {
        opts.validate()?;
        for engine in &self.engines {
            if engine.is_available() {
                return engine.recognize(image, opts);
            }
        }
        Err(CoreError::CaptureFailed(
            "no available OCR engine installed".into(),
        ))
    }
}

struct TempPngFile {
    path: PathBuf,
}

impl TempPngFile {
    fn create(png_bytes: &[u8]) -> Result<Self, CoreError> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let temp_dir = std::env::temp_dir();
        let pid = std::process::id();

        for _ in 0..32 {
            let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = temp_dir.join(format!("voltshot-ocr-{pid}-{nanos}-{seq}.png"));
            let mut open_options = OpenOptions::new();
            open_options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                open_options.mode(0o600);
            }
            match open_options.open(&path) {
                Ok(mut file) => {
                    let guard = Self { path };
                    file.write_all(png_bytes)?;
                    file.flush()?;
                    return Ok(guard);
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(CoreError::Io(err)),
            }
        }

        Err(CoreError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "failed to allocate unique temporary PNG path for OCR",
        )))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPngFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use tempfile::tempdir;

    fn sample_image() -> CapturedImage {
        CapturedImage::from_rgba(2, 2, [255, 255, 255, 255].repeat(4)).expect("valid 2x2 image")
    }

    struct StubOcrEngine {
        engine_name: &'static str,
        available: bool,
        output_text: String,
        calls: Arc<AtomicUsize>,
    }

    impl StubOcrEngine {
        fn new(
            engine_name: &'static str,
            available: bool,
            output_text: &str,
            calls: Arc<AtomicUsize>,
        ) -> Self {
            Self {
                engine_name,
                available,
                output_text: output_text.to_owned(),
                calls,
            }
        }
    }

    impl OcrEngine for StubOcrEngine {
        fn name(&self) -> &'static str {
            self.engine_name
        }

        fn is_available(&self) -> bool {
            self.available
        }

        fn recognize(
            &self,
            _image: &CapturedImage,
            opts: &OcrOptions,
        ) -> Result<OcrResult, CoreError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(OcrResult {
                text: self.output_text.clone(),
                language: opts.language.clone(),
                engine_name: self.engine_name,
            })
        }
    }

    #[test]
    fn ocr_options_validate_accepts_valid_tags_and_psm_bounds() {
        let default_opts = OcrOptions::default();
        assert_eq!(default_opts.language, "eng");
        assert_eq!(default_opts.psm, None);
        assert!(default_opts.validate().is_ok());

        for language in ["eng", "pol", "eng+pol", "chi_sim+eng", "srp-latn"] {
            let opts = OcrOptions {
                language: language.to_owned(),
                psm: Some(0),
            };
            assert!(opts.validate().is_ok(), "expected `{language}` to be valid");
        }

        let max_psm = OcrOptions {
            language: "eng".to_owned(),
            psm: Some(13),
        };
        assert!(max_psm.validate().is_ok());
    }

    #[test]
    fn ocr_options_validate_rejects_invalid_language_tags_and_psm() {
        for language in [
            "",
            "   ",
            " eng",
            "eng ",
            "eng pol",
            "eng+",
            "+pol",
            "eng++pol",
            "-eng",
            "--psm",
            "eng;whoami",
            "../eng",
        ] {
            let opts = OcrOptions {
                language: language.to_owned(),
                psm: None,
            };
            assert!(
                matches!(opts.validate(), Err(CoreError::ConfigError(_))),
                "expected `{language}` to be rejected"
            );
        }

        for psm in [14u8, 99, 255] {
            let opts = OcrOptions {
                language: "eng".to_owned(),
                psm: Some(psm),
            };
            assert!(matches!(opts.validate(), Err(CoreError::ConfigError(_))));
        }
    }

    #[test]
    fn build_tesseract_args_produces_exact_cli_arguments_with_and_without_psm() {
        let input = Path::new("/tmp/capture.png");

        let without_psm = OcrOptions {
            language: "eng".to_owned(),
            psm: None,
        };
        assert_eq!(
            build_tesseract_args(input, &without_psm).expect("args without psm"),
            vec!["/tmp/capture.png", "stdout", "-l", "eng"]
        );

        let with_psm = OcrOptions {
            language: "eng+pol".to_owned(),
            psm: Some(6),
        };
        assert_eq!(
            build_tesseract_args(input, &with_psm).expect("args with psm"),
            vec!["/tmp/capture.png", "stdout", "-l", "eng+pol", "--psm", "6"]
        );

        assert!(matches!(
            build_tesseract_args(Path::new(""), &without_psm),
            Err(CoreError::ConfigError(_))
        ));
        assert!(matches!(
            build_tesseract_args(
                input,
                &OcrOptions {
                    language: "eng".to_owned(),
                    psm: Some(14),
                }
            ),
            Err(CoreError::ConfigError(_))
        ));
    }

    #[test]
    fn fallback_ocr_chain_skips_unavailable_and_returns_first_available_result() {
        let first_calls = Arc::new(AtomicUsize::new(0));
        let second_calls = Arc::new(AtomicUsize::new(0));
        let third_calls = Arc::new(AtomicUsize::new(0));

        let chain = FallbackOcrChain::new(vec![
            Box::new(StubOcrEngine::new(
                "unavailable-primary",
                false,
                "unused",
                Arc::clone(&first_calls),
            )),
            Box::new(StubOcrEngine::new(
                "available-secondary",
                true,
                "Hello OCR",
                Arc::clone(&second_calls),
            )),
            Box::new(StubOcrEngine::new(
                "available-tertiary",
                true,
                "Should not run",
                Arc::clone(&third_calls),
            )),
        ]);

        assert!(chain.is_available());
        let opts = OcrOptions {
            language: "eng+pol".to_owned(),
            psm: Some(3),
        };
        let result = chain
            .recognize(&sample_image(), &opts)
            .expect("second engine succeeds");

        assert_eq!(
            result,
            OcrResult {
                text: "Hello OCR".to_owned(),
                language: "eng+pol".to_owned(),
                engine_name: "available-secondary",
            }
        );
        assert_eq!(first_calls.load(Ordering::SeqCst), 0);
        assert_eq!(second_calls.load(Ordering::SeqCst), 1);
        assert_eq!(third_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn fallback_ocr_chain_returns_error_when_all_engines_unavailable() {
        let calls = Arc::new(AtomicUsize::new(0));
        let chain = FallbackOcrChain::new(vec![
            Box::new(StubOcrEngine::new(
                "engine-a",
                false,
                "",
                Arc::clone(&calls),
            )),
            Box::new(StubOcrEngine::new(
                "engine-b",
                false,
                "",
                Arc::clone(&calls),
            )),
        ]);

        assert!(!chain.is_available());
        assert!(matches!(
            chain.recognize(&sample_image(), &OcrOptions::default()),
            Err(CoreError::CaptureFailed(_))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let empty_chain = FallbackOcrChain::new(Vec::new());
        assert!(!empty_chain.is_available());
        assert!(matches!(
            empty_chain.recognize(&sample_image(), &OcrOptions::default()),
            Err(CoreError::CaptureFailed(_))
        ));
    }

    #[test]
    fn tesseract_cli_ocr_reports_unavailable_and_errors_for_nonexistent_override() {
        let dir = tempdir().expect("tempdir");
        let missing_bin = dir.path().join("definitely-missing-tesseract-binary");
        let engine = TesseractCliOcr::new(Some(missing_bin.clone()));

        assert_eq!(engine.name(), "tesseract-cli");
        assert_eq!(engine.binary_override(), Some(missing_bin.as_path()));
        assert!(!engine.is_available());
        assert!(TesseractCliOcr::find_tesseract(Some(&missing_bin)).is_none());

        let err = engine
            .recognize(&sample_image(), &OcrOptions::default())
            .expect_err("nonexistent binary should fail cleanly");
        assert!(matches!(err, CoreError::CaptureFailed(_)));
    }

    #[test]
    fn find_tesseract_discovers_override_file_directory_and_path_entries() {
        let dir = tempdir().expect("tempdir");
        let empty_dir = dir.path().join("empty");
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&empty_dir).expect("create empty dir");
        fs::create_dir_all(&bin_dir).expect("create bin dir");

        let fake_bin = bin_dir.join(TESSERACT_BINARY);
        fs::write(&fake_bin, b"stub").expect("write fake binary");

        assert_eq!(find_tesseract(Some(&fake_bin)), Some(fake_bin.clone()));
        assert_eq!(find_tesseract(Some(&bin_dir)), Some(fake_bin.clone()));
        assert_eq!(find_tesseract(Some(&empty_dir)), None);
        assert_eq!(find_in_paths([empty_dir, bin_dir]), Some(fake_bin));
    }

    #[test]
    fn temp_png_file_is_written_and_deleted_on_drop() {
        let png = sample_image().encode(ImageFormat::Png).expect("encode png");
        let created_path = {
            let temp = TempPngFile::create(&png).expect("create temp png");
            let path = temp.path().to_path_buf();
            assert!(path.is_file());
            assert_eq!(fs::read(&path).expect("read temp png"), png);
            path
        };
        assert!(!created_path.exists());
    }
}
