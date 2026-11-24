pub mod config;
pub mod ffmpeg;
pub mod recorder;

pub use config::{CapturePlatform, RecordingConfig, RecordingFormat, VideoEncoder};
pub use ffmpeg::{MediaError, build_recording_args, find_ffmpeg};
pub use recorder::{RecordingState, RecordingStateMachine, ScreenRecorder};
