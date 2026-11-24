pub mod config;
pub mod domain;
pub mod history;
#[cfg(feature = "sqlite")]
pub mod history_sqlite;
pub mod hotkeys;
pub mod naming;
pub mod pipeline;
pub mod ports;
pub mod workflows;

pub use config::{AppConfig, PipelineConfig};
pub use domain::{
    CaptureId, CaptureSource, CapturedImage, CoreError, ImageFormat, MonitorId, MonitorInfo,
    Region, WindowId, WindowInfo,
};
pub use history::{HistoryEntry, HistoryQuery, HistoryRepository, JsonlHistoryStore};
#[cfg(feature = "sqlite")]
pub use history_sqlite::SqliteHistoryStore;
pub use hotkeys::{
    HotkeyAction, HotkeyBinding, HotkeyCombo, KeyCode, KeyModifiers, default_sharex_hotkeys,
};
pub use naming::{FileNameContext, FileNamePattern, resolve_unique_path};
pub use pipeline::{CapturePipeline, PipelineDeps, TaskReport};
pub use ports::{
    ClipboardSink, EmptyUploaderLookup, NoopClipboard, ScreenCapturer, UploadRequest, UploadResult,
    Uploader, UploaderLookup,
};
pub use workflows::{
    TaskOverrides, WatchFolderRule, WatchFolderScanner, WatchedFileEvent, WorkflowPreset,
    WorkflowRegistry, resolve_effective_config,
};
