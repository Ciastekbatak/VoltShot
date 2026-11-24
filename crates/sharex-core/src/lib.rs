pub mod config;
pub mod domain;
pub mod history;
pub mod naming;
pub mod pipeline;
pub mod ports;

pub use config::{AppConfig, PipelineConfig};
pub use domain::{
    CaptureId, CaptureSource, CapturedImage, CoreError, ImageFormat, MonitorId, MonitorInfo, Region,
};
pub use history::{HistoryEntry, HistoryRepository, JsonlHistoryStore};
pub use naming::{FileNameContext, FileNamePattern, resolve_unique_path};
pub use pipeline::{CapturePipeline, PipelineDeps, TaskReport};
pub use ports::{
    ClipboardSink, EmptyUploaderLookup, NoopClipboard, ScreenCapturer, UploadRequest, UploadResult,
    Uploader, UploaderLookup,
};
