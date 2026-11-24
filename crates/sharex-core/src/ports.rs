use crate::domain::{CaptureSource, CapturedImage, CoreError, MonitorInfo, WindowInfo};

/// Screen capture backend boundary implemented by OS adapters.
pub trait ScreenCapturer: Send + Sync {
    fn list_monitors(&self) -> Result<Vec<MonitorInfo>, CoreError>;
    fn list_windows(&self) -> Result<Vec<WindowInfo>, CoreError> {
        Ok(Vec::new())
    }
    fn capture(&self, source: &CaptureSource) -> Result<CapturedImage, CoreError>;
}

/// System clipboard boundary for copying captured images or text.
pub trait ClipboardSink: Send + Sync {
    fn copy_image(&self, image: &CapturedImage) -> Result<(), CoreError>;
    fn copy_text(&self, text: &str) -> Result<(), CoreError>;
}

/// No-op clipboard used when clipboard integration is disabled or unavailable.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopClipboard;

impl ClipboardSink for NoopClipboard {
    fn copy_image(&self, _image: &CapturedImage) -> Result<(), CoreError> {
        Ok(())
    }

    fn copy_text(&self, _text: &str) -> Result<(), CoreError> {
        Ok(())
    }
}

/// Payload handed to an uploader plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRequest {
    pub file_name: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// Destination URL returned by a completed upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResult {
    pub url: String,
}

/// Extension point for community-contributed image and file uploaders.
pub trait Uploader: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn upload(&self, request: &UploadRequest) -> Result<UploadResult, CoreError>;
}

/// Registry lookup boundary used by `CapturePipeline`.
pub trait UploaderLookup: Send + Sync {
    fn get(&self, id: &str) -> Option<&dyn Uploader>;
}

/// Empty uploader registry for configurations with no active uploaders.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyUploaderLookup;

impl UploaderLookup for EmptyUploaderLookup {
    fn get(&self, _id: &str) -> Option<&dyn Uploader> {
        None
    }
}
