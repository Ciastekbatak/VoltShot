pub mod config;
pub mod ffmpeg;
pub mod recorder;

pub use config::{AudioDevice, CapturePlatform, RecordingConfig, RecordingFormat, VideoEncoder};
pub use ffmpeg::{
    MediaError, build_gif_spool_args, build_gif_transcode_args, build_recording_args,
    detect_available_h264_encoders, find_ffmpeg, list_audio_devices, parse_available_h264_encoders,
    parse_avfoundation_audio_devices, parse_dshow_audio_devices, preferred_mp4_encoder,
};
pub use recorder::{RecordingState, RecordingStateMachine, ScreenRecorder};
