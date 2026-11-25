use crate::config::AppConfig;
use crate::domain::{CaptureId, CaptureSource, CapturedImage, CoreError, ImageFormat};
use crate::history::{HistoryEntry, HistoryRepository};
use crate::naming::{FileNameContext, FileNamePattern, resolve_unique_path};
use crate::ports::{ClipboardSink, ScreenCapturer, UploadRequest, UploaderLookup};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static CAPTURE_SEQUENCE: AtomicU32 = AtomicU32::new(1);

/// Observable outcome of running a capture task through the after-capture pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskReport {
    pub capture_id: CaptureId,
    pub timestamp: DateTime<Utc>,
    pub source: CaptureSource,
    pub image: CapturedImage,
    pub format: ImageFormat,
    pub saved_path: Option<PathBuf>,
    pub upload_url: Option<String>,
    pub copied_image: bool,
    pub copied_text: Option<String>,
    pub history_recorded: bool,
}

/// Injected system boundaries required to execute a capture task.
pub struct PipelineDeps<'a> {
    pub capturer: &'a dyn ScreenCapturer,
    pub clipboard: &'a dyn ClipboardSink,
    pub uploaders: &'a dyn UploaderLookup,
    pub history: &'a dyn HistoryRepository,
}

/// Pure orchestrator for VoltShot capture and after-capture actions.
pub struct CapturePipeline;

impl CapturePipeline {
    pub fn execute(
        source: CaptureSource,
        config: &AppConfig,
        deps: &PipelineDeps<'_>,
        now: DateTime<Utc>,
    ) -> Result<TaskReport, CoreError> {
        config.validate()?;
        let pattern = FileNamePattern::parse(&config.file_name_pattern)?;

        let image = deps.capturer.capture(&source)?;
        let seq = CAPTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let capture_id = CaptureId::from_timestamp(now, seq);

        let needs_encoded = config.pipeline.save_to_file || config.pipeline.uploader_id.is_some();
        let encoded_bytes = if needs_encoded {
            Some(image.encode(config.image_format)?)
        } else {
            None
        };

        let stem = pattern.expand(&FileNameContext {
            timestamp: now,
            app_name: "VoltShot",
            width: image.width(),
            height: image.height(),
        });
        let ext = config.image_format.extension();

        let saved_path = if config.pipeline.save_to_file {
            fs::create_dir_all(&config.output_dir)?;
            let target = resolve_unique_path(&config.output_dir, &stem, ext);
            let bytes = encoded_bytes
                .as_deref()
                .ok_or_else(|| CoreError::EncodeFailed {
                    format: config.image_format,
                    message: "missing encoded image bytes".into(),
                })?;
            fs::write(&target, bytes)?;
            Some(target)
        } else {
            None
        };

        let copied_image = if config.pipeline.copy_image_to_clipboard {
            deps.clipboard.copy_image(&image)?;
            true
        } else {
            false
        };

        let upload_url = if let Some(uploader_id) = config.pipeline.uploader_id.as_deref() {
            let uploader = deps
                .uploaders
                .get(uploader_id)
                .ok_or_else(|| CoreError::UnknownUploader(uploader_id.to_owned()))?;
            let bytes = encoded_bytes.ok_or_else(|| CoreError::EncodeFailed {
                format: config.image_format,
                message: "missing encoded image bytes for upload".into(),
            })?;
            let file_name = saved_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("{stem}.{ext}"));
            let result = uploader.upload(&UploadRequest {
                file_name,
                mime_type: config.image_format.mime_type().to_owned(),
                bytes,
            })?;
            Some(result.url)
        } else {
            None
        };

        let copied_text = if config.pipeline.copy_path_to_clipboard {
            let text_target = upload_url
                .clone()
                .or_else(|| saved_path.as_ref().map(|p| p.display().to_string()));
            if let Some(text) = text_target {
                deps.clipboard.copy_text(&text)?;
                Some(text)
            } else {
                None
            }
        } else {
            None
        };

        let history_recorded = if config.pipeline.record_history {
            deps.history.append(&HistoryEntry {
                id: capture_id.clone(),
                timestamp: now,
                source,
                width: image.width(),
                height: image.height(),
                format: config.image_format,
                file_path: saved_path.clone(),
                upload_url: upload_url.clone(),
                tags: Vec::new(),
                note: None,
            })?;
            true
        } else {
            false
        };

        Ok(TaskReport {
            capture_id,
            timestamp: now,
            source,
            image,
            format: config.image_format,
            saved_path,
            upload_url,
            copied_image,
            copied_text,
            history_recorded,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::MonitorInfo;
    use crate::history::JsonlHistoryStore;
    use crate::ports::{UploadResult, Uploader};
    use chrono::TimeZone;
    use std::sync::Mutex;
    use tempfile::tempdir;

    struct FakeCapturer {
        image: CapturedImage,
    }

    impl ScreenCapturer for FakeCapturer {
        fn list_monitors(&self) -> Result<Vec<MonitorInfo>, CoreError> {
            Ok(Vec::new())
        }

        fn capture(&self, _source: &CaptureSource) -> Result<CapturedImage, CoreError> {
            Ok(self.image.clone())
        }
    }

    #[derive(Default)]
    struct RecordingClipboard {
        images: Mutex<usize>,
        texts: Mutex<Vec<String>>,
    }

    impl ClipboardSink for RecordingClipboard {
        fn copy_image(&self, _image: &CapturedImage) -> Result<(), CoreError> {
            *self.images.lock().expect("lock") += 1;
            Ok(())
        }

        fn copy_text(&self, text: &str) -> Result<(), CoreError> {
            self.texts.lock().expect("lock").push(text.to_owned());
            Ok(())
        }
    }

    struct FakeUploader;

    impl Uploader for FakeUploader {
        fn id(&self) -> &'static str {
            "fake"
        }

        fn name(&self) -> &'static str {
            "Fake"
        }

        fn description(&self) -> &'static str {
            "Test uploader"
        }

        fn upload(&self, request: &UploadRequest) -> Result<UploadResult, CoreError> {
            Ok(UploadResult {
                url: format!("https://example.test/{}", request.file_name),
            })
        }
    }

    struct SingleUploaderLookup(FakeUploader);

    impl UploaderLookup for SingleUploaderLookup {
        fn get(&self, id: &str) -> Option<&dyn Uploader> {
            (id == self.0.id()).then_some(&self.0)
        }
    }

    #[test]
    fn pipeline_saves_copies_uploads_and_records_history() {
        let dir = tempdir().expect("tempdir");
        let mut config = AppConfig::default_for_os();
        config.output_dir = dir.path().join("captures");
        config.history_path = dir.path().join("history.jsonl");
        config.file_name_pattern = "Shot_%y%mo%d_%widthx%height".into();
        config.image_format = ImageFormat::Png;
        config.pipeline.save_to_file = true;
        config.pipeline.copy_image_to_clipboard = true;
        config.pipeline.copy_path_to_clipboard = true;
        config.pipeline.record_history = true;
        config.pipeline.uploader_id = Some("fake".into());

        let capturer = FakeCapturer {
            image: CapturedImage::from_rgba(3, 2, [12, 34, 56, 255].repeat(6))
                .expect("valid image"),
        };
        let clipboard = RecordingClipboard::default();
        let uploaders = SingleUploaderLookup(FakeUploader);
        let history = JsonlHistoryStore::new(&config.history_path);
        let now = Utc
            .with_ymd_and_hms(2026, 11, 24, 10, 0, 0)
            .single()
            .expect("valid timestamp");

        let deps = PipelineDeps {
            capturer: &capturer,
            clipboard: &clipboard,
            uploaders: &uploaders,
            history: &history,
        };
        let report = CapturePipeline::execute(CaptureSource::PrimaryMonitor, &config, &deps, now)
            .expect("pipeline succeeds");

        let saved_path = report.saved_path.expect("file saved");
        assert!(saved_path.exists());
        assert_eq!(
            saved_path.file_name().and_then(|n| n.to_str()),
            Some("Shot_20261124_3x2.png")
        );
        let disk_bytes = fs::read(&saved_path).expect("read saved png");
        let decoded = image::load_from_memory(&disk_bytes).expect("valid png on disk");
        assert_eq!((decoded.width(), decoded.height()), (3, 2));

        assert!(report.copied_image);
        assert_eq!(*clipboard.images.lock().expect("lock"), 1);
        assert_eq!(
            report.upload_url.as_deref(),
            Some("https://example.test/Shot_20261124_3x2.png")
        );
        assert_eq!(
            report.copied_text.as_deref(),
            Some("https://example.test/Shot_20261124_3x2.png")
        );
        assert_eq!(
            clipboard.texts.lock().expect("lock").as_slice(),
            ["https://example.test/Shot_20261124_3x2.png"]
        );

        let entries = history.list(None).expect("read history");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, report.capture_id);
        assert_eq!(entries[0].upload_url, report.upload_url);
    }

    #[test]
    fn pipeline_returns_unknown_uploader_error_for_unregistered_id() {
        let dir = tempdir().expect("tempdir");
        let mut config = AppConfig::default_for_os();
        config.output_dir = dir.path().join("captures");
        config.history_path = dir.path().join("history.jsonl");
        config.pipeline.save_to_file = false;
        config.pipeline.copy_image_to_clipboard = false;
        config.pipeline.uploader_id = Some("missing".into());

        let capturer = FakeCapturer {
            image: CapturedImage::from_rgba(1, 1, vec![0, 0, 0, 255]).expect("valid"),
        };
        let clipboard = RecordingClipboard::default();
        let uploaders = crate::ports::EmptyUploaderLookup;
        let history = JsonlHistoryStore::new(&config.history_path);

        let deps = PipelineDeps {
            capturer: &capturer,
            clipboard: &clipboard,
            uploaders: &uploaders,
            history: &history,
        };
        let err =
            CapturePipeline::execute(CaptureSource::PrimaryMonitor, &config, &deps, Utc::now())
                .expect_err("must fail on unknown uploader");
        assert!(matches!(err, CoreError::UnknownUploader(id) if id == "missing"));
    }
}
