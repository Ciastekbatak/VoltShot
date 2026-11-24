pub use crate::config::AudioDevice;
use crate::config::{CapturePlatform, RecordingConfig, RecordingFormat, VideoEncoder};
use std::env;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use thiserror::Error;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn configure_no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// Parses `ffmpeg -hide_banner -encoders` stdout and returns available H.264 encoders in priority order.
pub fn parse_available_h264_encoders(ffmpeg_encoders_stdout: &str) -> Vec<VideoEncoder> {
    let mut has_nvenc = false;
    let mut has_qsv = false;
    let mut has_amf = false;
    let mut has_libx264 = false;

    for line in ffmpeg_encoders_stdout.lines() {
        let mut parts = line.split_whitespace();
        let Some(flags) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        if flags.len() != 6
            || !flags.starts_with('V')
            || !flags
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte == b'.')
        {
            continue;
        }
        match name {
            "h264_nvenc" => has_nvenc = true,
            "h264_qsv" => has_qsv = true,
            "h264_amf" => has_amf = true,
            "libx264" => has_libx264 = true,
            _ => {}
        }
    }

    let mut encoders = Vec::new();
    if has_nvenc {
        encoders.push(VideoEncoder::NvencH264);
    }
    if has_qsv {
        encoders.push(VideoEncoder::QsvH264);
    }
    if has_amf {
        encoders.push(VideoEncoder::AmfH264);
    }
    if has_libx264 {
        encoders.push(VideoEncoder::Libx264);
    }
    encoders
}

/// Probes `ffmpeg_path` for available H.264 hardware and software encoders in priority order.
pub fn detect_available_h264_encoders(ffmpeg_path: &Path) -> Vec<VideoEncoder> {
    let mut command = Command::new(ffmpeg_path);
    command
        .args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_no_window(&mut command);

    let Ok(output) = command.output() else {
        return vec![VideoEncoder::Libx264];
    };
    if !output.status.success() {
        return vec![VideoEncoder::Libx264];
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = parse_available_h264_encoders(&stdout);
    if parsed.is_empty() {
        vec![VideoEncoder::Libx264]
    } else {
        parsed
    }
}

/// Selects the highest-priority MP4 H.264 encoder from `available`, falling back to `Libx264`.
pub fn preferred_mp4_encoder(available: &[VideoEncoder]) -> VideoEncoder {
    const PRIORITY: [VideoEncoder; 4] = [
        VideoEncoder::NvencH264,
        VideoEncoder::QsvH264,
        VideoEncoder::AmfH264,
        VideoEncoder::Libx264,
    ];
    for candidate in PRIORITY {
        if available.contains(&candidate) {
            return candidate;
        }
    }
    VideoEncoder::Libx264
}

/// Parses Windows `ffmpeg -list_devices true -f dshow -i dummy` stderr for DirectShow audio devices.
pub fn parse_dshow_audio_devices(stderr: &str) -> Vec<AudioDevice> {
    let mut devices = Vec::new();
    let mut in_audio_section = false;

    for line in stderr.lines() {
        let Some(dshow_pos) = line.find("[dshow") else {
            continue;
        };
        let Some(bracket_end) = line[dshow_pos..].find(']') else {
            continue;
        };
        let body = line[dshow_pos + bracket_end + 1..].trim();

        if body.starts_with("DirectShow video devices") {
            in_audio_section = false;
            continue;
        }
        if body.starts_with("DirectShow audio devices") {
            in_audio_section = true;
            continue;
        }
        if body.starts_with("Alternative name") || !body.starts_with('"') {
            continue;
        }
        let Some(end_quote) = body.rfind('"') else {
            continue;
        };
        if end_quote == 0 {
            continue;
        }

        let name = &body[1..end_quote];
        if name.is_empty() {
            continue;
        }

        let suffix = body[end_quote + 1..].trim();
        let is_audio = if let Some(inner) = suffix
            .strip_prefix('(')
            .and_then(|rest| rest.strip_suffix(')'))
        {
            inner.split(',').any(|tag| tag.trim() == "audio")
        } else {
            in_audio_section && suffix.is_empty()
        };

        if is_audio && !devices.iter().any(|dev: &AudioDevice| dev.name == name) {
            devices.push(AudioDevice {
                name: name.to_owned(),
                backend: "dshow",
            });
        }
    }

    devices
}

/// Parses macOS `ffmpeg -f avfoundation -list_devices true -i ""` stderr for AVFoundation audio devices.
pub fn parse_avfoundation_audio_devices(stderr: &str) -> Vec<AudioDevice> {
    let mut devices = Vec::new();
    let mut in_audio_section = false;

    for line in stderr.lines() {
        let Some(avf_pos) = line.find("[AVFoundation") else {
            continue;
        };
        let Some(bracket_end) = line[avf_pos..].find(']') else {
            continue;
        };
        let body = line[avf_pos + bracket_end + 1..].trim();

        if body.starts_with("AVFoundation video devices:") {
            in_audio_section = false;
            continue;
        }
        if body.starts_with("AVFoundation audio devices:") {
            in_audio_section = true;
            continue;
        }
        if !in_audio_section || !body.starts_with('[') {
            continue;
        }
        let Some(close_bracket) = body.find(']') else {
            continue;
        };
        let index_str = &body[1..close_bracket];
        if index_str.is_empty() || !index_str.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let name = body[close_bracket + 1..].trim();
        if !name.is_empty() && !devices.iter().any(|dev: &AudioDevice| dev.name == name) {
            devices.push(AudioDevice {
                name: name.to_owned(),
                backend: "avfoundation",
            });
        }
    }

    devices
}

/// Probes `ffmpeg_path` for available platform audio input devices, returning an empty list on failure.
pub fn list_audio_devices(ffmpeg_path: &Path, platform: CapturePlatform) -> Vec<AudioDevice> {
    let probe_args: &[&str] = match platform {
        CapturePlatform::WindowsGdiGrab => &[
            "-hide_banner",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ],
        CapturePlatform::MacOsAvFoundation => &[
            "-hide_banner",
            "-f",
            "avfoundation",
            "-list_devices",
            "true",
            "-i",
            "",
        ],
        CapturePlatform::LinuxX11Grab => return Vec::new(),
    };

    let mut command = Command::new(ffmpeg_path);
    command
        .args(probe_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    configure_no_window(&mut command);

    let Ok(output) = command.output() else {
        return Vec::new();
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    match platform {
        CapturePlatform::WindowsGdiGrab => parse_dshow_audio_devices(&stderr),
        CapturePlatform::MacOsAvFoundation => parse_avfoundation_audio_devices(&stderr),
        CapturePlatform::LinuxX11Grab => Vec::new(),
    }
}

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("invalid recording configuration: {0}")]
    InvalidConfig(String),

    #[error("ffmpeg executable not found")]
    FfmpegNotFound,

    #[error("invalid recording state transition: cannot `{action}` from `{from}`")]
    InvalidStateTransition {
        from: &'static str,
        action: &'static str,
    },

    #[error("ffmpeg process failed: {0}")]
    ProcessFailed(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

const fn ffmpeg_binary_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

pub fn find_ffmpeg(override_path: Option<&Path>) -> Result<PathBuf, MediaError> {
    let path_env = env::var_os("PATH");
    let search_dirs: Vec<PathBuf> = path_env
        .as_deref()
        .map(|paths| env::split_paths(paths).collect())
        .unwrap_or_default();
    find_ffmpeg_in_dirs(override_path, &search_dirs)
}

fn find_ffmpeg_in_dirs(
    override_path: Option<&Path>,
    search_dirs: &[PathBuf],
) -> Result<PathBuf, MediaError> {
    if let Some(explicit) = override_path {
        if explicit.is_file() {
            return Ok(explicit.to_path_buf());
        }
        return Err(MediaError::FfmpegNotFound);
    }

    let binary_name = ffmpeg_binary_name();
    for dir in search_dirs {
        let candidate = dir.join(binary_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(MediaError::FfmpegNotFound)
}

pub fn build_recording_args(
    config: &RecordingConfig,
    platform: CapturePlatform,
) -> Result<Vec<String>, MediaError> {
    config.validate()?;
    let encoder = config.resolved_encoder()?;
    let cursor_flag = if config.draw_cursor { "1" } else { "0" };
    let active_audio = if config.format == RecordingFormat::Gif {
        None
    } else {
        config
            .audio_device
            .as_deref()
            .filter(|dev| !dev.trim().is_empty())
    };

    let mut args = vec!["-y".to_owned()];

    match platform {
        CapturePlatform::WindowsGdiGrab => {
            args.push("-f".to_owned());
            args.push("gdigrab".to_owned());
            args.push("-framerate".to_owned());
            args.push(config.fps.to_string());
            args.push("-draw_mouse".to_owned());
            args.push(cursor_flag.to_owned());
            if let Some(region) = config.region {
                args.push("-offset_x".to_owned());
                args.push(region.x().to_string());
                args.push("-offset_y".to_owned());
                args.push(region.y().to_string());
                args.push("-video_size".to_owned());
                args.push(format!("{}x{}", region.width(), region.height()));
            }
            args.push("-i".to_owned());
            args.push("desktop".to_owned());
            if let Some(dev) = active_audio {
                args.push("-f".to_owned());
                args.push("dshow".to_owned());
                args.push("-i".to_owned());
                args.push(format!("audio={dev}"));
            }
        }
        CapturePlatform::LinuxX11Grab => {
            args.push("-f".to_owned());
            args.push("x11grab".to_owned());
            args.push("-framerate".to_owned());
            args.push(config.fps.to_string());
            args.push("-draw_mouse".to_owned());
            args.push(cursor_flag.to_owned());
            if let Some(region) = config.region {
                if region.x() < 0 || region.y() < 0 {
                    return Err(MediaError::InvalidConfig(
                        "x11grab region offsets must be non-negative".to_owned(),
                    ));
                }
                args.push("-video_size".to_owned());
                args.push(format!("{}x{}", region.width(), region.height()));
                args.push("-i".to_owned());
                args.push(format!(":0.0+{},{}", region.x(), region.y()));
            } else {
                args.push("-i".to_owned());
                args.push(":0.0".to_owned());
            }
            if let Some(dev) = active_audio {
                args.push("-f".to_owned());
                args.push("pulse".to_owned());
                args.push("-i".to_owned());
                args.push(dev.to_owned());
            }
        }
        CapturePlatform::MacOsAvFoundation => {
            if let Some(region) = config.region {
                if region.x() < 0 || region.y() < 0 {
                    return Err(MediaError::InvalidConfig(
                        "avfoundation crop offsets must be non-negative".to_owned(),
                    ));
                }
            }
            args.push("-f".to_owned());
            args.push("avfoundation".to_owned());
            args.push("-framerate".to_owned());
            args.push(config.fps.to_string());
            args.push("-capture_cursor".to_owned());
            args.push(cursor_flag.to_owned());
            args.push("-i".to_owned());
            let avf_input = match active_audio {
                Some(dev) => format!("1:{dev}"),
                None => "1:none".to_owned(),
            };
            args.push(avf_input);
        }
    }

    let macos_crop = match (platform, config.region) {
        (CapturePlatform::MacOsAvFoundation, Some(region)) => Some(format!(
            "crop={}:{}:{}:{}",
            region.width(),
            region.height(),
            region.x(),
            region.y()
        )),
        _ => None,
    };

    match config.format {
        RecordingFormat::Mp4H264 => {
            match encoder {
                VideoEncoder::Libx264 => {
                    args.push("-c:v".to_owned());
                    args.push("libx264".to_owned());
                    args.push("-preset".to_owned());
                    args.push("veryfast".to_owned());
                    args.push("-crf".to_owned());
                    args.push(config.crf.to_string());
                }
                VideoEncoder::NvencH264 => {
                    args.push("-c:v".to_owned());
                    args.push("h264_nvenc".to_owned());
                    args.push("-preset".to_owned());
                    args.push("p4".to_owned());
                    args.push("-cq".to_owned());
                    args.push(config.crf.to_string());
                }
                VideoEncoder::QsvH264 => {
                    args.push("-c:v".to_owned());
                    args.push("h264_qsv".to_owned());
                    args.push("-global_quality".to_owned());
                    args.push(config.crf.to_string());
                }
                VideoEncoder::AmfH264 => {
                    args.push("-c:v".to_owned());
                    args.push("h264_amf".to_owned());
                    args.push("-rc".to_owned());
                    args.push("cqp".to_owned());
                    args.push("-qp_i".to_owned());
                    args.push(config.crf.to_string());
                    args.push("-qp_p".to_owned());
                    args.push(config.crf.to_string());
                }
                _ => unreachable!("validated by VideoEncoder::resolve"),
            }
            let vf = match macos_crop {
                Some(crop) => format!("{crop},scale=trunc(iw/2)*2:trunc(ih/2)*2"),
                None => "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_owned(),
            };
            args.push("-vf".to_owned());
            args.push(vf);
            args.push("-pix_fmt".to_owned());
            args.push("yuv420p".to_owned());
            if active_audio.is_some() {
                args.push("-c:a".to_owned());
                args.push("aac".to_owned());
                args.push("-b:a".to_owned());
                args.push("128k".to_owned());
            }
            args.push("-movflags".to_owned());
            args.push("+faststart".to_owned());
        }
        RecordingFormat::WebmVp9 => {
            args.push("-c:v".to_owned());
            args.push("libvpx-vp9".to_owned());
            args.push("-crf".to_owned());
            args.push(config.crf.to_string());
            args.push("-b:v".to_owned());
            args.push("0".to_owned());
            args.push("-deadline".to_owned());
            args.push("realtime".to_owned());
            args.push("-cpu-used".to_owned());
            args.push("4".to_owned());
            if let Some(crop) = macos_crop {
                args.push("-vf".to_owned());
                args.push(crop);
            }
            args.push("-pix_fmt".to_owned());
            args.push("yuv420p".to_owned());
            if active_audio.is_some() {
                args.push("-c:a".to_owned());
                args.push("libopus".to_owned());
                args.push("-b:a".to_owned());
                args.push("128k".to_owned());
            }
        }
        RecordingFormat::Gif => {
            let filtergraph = match macos_crop {
                Some(crop) => format!(
                    "{crop},fps={},split[s0][s1];[s0]palettegen=stats_mode=single[p];[s1][p]paletteuse=new=1",
                    config.fps
                ),
                None => format!(
                    "fps={},split[s0][s1];[s0]palettegen=stats_mode=single[p];[s1][p]paletteuse=new=1",
                    config.fps
                ),
            };
            args.push("-vf".to_owned());
            args.push(filtergraph);
            args.push("-loop".to_owned());
            args.push("0".to_owned());
        }
    }

    args.push(config.output_path.to_string_lossy().into_owned());
    Ok(args)
}

/// Builds low-RAM disk-spooled capture arguments for Stage 1 of a two-stage GIF recording.
pub fn build_gif_spool_args(
    config: &RecordingConfig,
    platform: CapturePlatform,
    spool_path: &Path,
) -> Result<Vec<String>, MediaError> {
    config.validate()?;
    let mut spool_config = config.clone();
    spool_config.format = RecordingFormat::Mp4H264;
    spool_config.encoder = VideoEncoder::Libx264;
    spool_config.crf = 18;
    spool_config.audio_device = None;
    spool_config.output_path = spool_path.to_path_buf();

    let mut args = build_recording_args(&spool_config, platform)?;
    if let Some(pos) = args.iter().position(|arg| arg == "veryfast") {
        args[pos] = "ultrafast".to_owned();
    }
    if let Some(pos) = args.iter().position(|arg| arg == "-movflags") {
        args.drain(pos..=pos + 1);
    }
    Ok(args)
}

/// Builds Stage 2 offline palette-optimized GIF transcode arguments from a spooled recording.
pub fn build_gif_transcode_args(spool_path: &Path, output_path: &Path, fps: u32) -> Vec<String> {
    vec![
        "-y".to_owned(),
        "-i".to_owned(),
        spool_path.to_string_lossy().into_owned(),
        "-vf".to_owned(),
        format!("fps={fps},split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse"),
        "-loop".to_owned(),
        "0".to_owned(),
        output_path.to_string_lossy().into_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use sharex_core::Region;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn build_args_windows_gdigrab_mp4_with_region_and_nvenc() {
        let mut config = RecordingConfig::new("out/recording.mp4");
        config.region = Some(Region::new(-100, 50, 1280, 720).expect("valid region"));
        config.fps = 60;
        config.crf = 18;
        config.draw_cursor = false;
        config.encoder = VideoEncoder::NvencH264;

        let args = build_recording_args(&config, CapturePlatform::WindowsGdiGrab)
            .expect("args build succeeds");

        assert_eq!(
            args,
            vec![
                "-y",
                "-f",
                "gdigrab",
                "-framerate",
                "60",
                "-draw_mouse",
                "0",
                "-offset_x",
                "-100",
                "-offset_y",
                "50",
                "-video_size",
                "1280x720",
                "-i",
                "desktop",
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p4",
                "-cq",
                "18",
                "-vf",
                "scale=trunc(iw/2)*2:trunc(ih/2)*2",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
                "out/recording.mp4",
            ]
        );
    }

    #[test]
    fn build_args_uses_libx264_when_either_region_side_is_too_small() {
        let mut config = RecordingConfig::new("screen.mp4");
        config.encoder = VideoEncoder::NvencH264;

        for (width, height) in [(64, 64), (159, 160), (160, 159), (160, 160)] {
            config.region = Some(Region::new(0, 0, width, height).expect("valid region"));
            let args = build_recording_args(&config, CapturePlatform::WindowsGdiGrab)
                .expect("args build succeeds");
            let expected = if (width, height) == (160, 160) {
                "h264_nvenc"
            } else {
                "libx264"
            };
            assert!(args.windows(2).any(|pair| pair == ["-c:v", expected]));
        }
    }

    #[test]
    fn build_args_windows_gdigrab_fullscreen_mp4_auto_uses_libx264() {
        let config = RecordingConfig::new("screen.mp4");
        let args = build_recording_args(&config, CapturePlatform::WindowsGdiGrab)
            .expect("args build succeeds");

        assert_eq!(
            args,
            vec![
                "-y",
                "-f",
                "gdigrab",
                "-framerate",
                "30",
                "-draw_mouse",
                "1",
                "-i",
                "desktop",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "23",
                "-vf",
                "scale=trunc(iw/2)*2:trunc(ih/2)*2",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
                "screen.mp4",
            ]
        );
    }

    #[test]
    fn build_args_linux_x11grab_webm_with_region() {
        let mut config = RecordingConfig::new("clip.webm");
        config.format = RecordingFormat::WebmVp9;
        config.encoder = VideoEncoder::Auto;
        config.fps = 30;
        config.crf = 30;
        config.region = Some(Region::new(10, 20, 800, 600).expect("valid region"));

        let args = build_recording_args(&config, CapturePlatform::LinuxX11Grab)
            .expect("args build succeeds");

        assert_eq!(
            args,
            vec![
                "-y",
                "-f",
                "x11grab",
                "-framerate",
                "30",
                "-draw_mouse",
                "1",
                "-video_size",
                "800x600",
                "-i",
                ":0.0+10,20",
                "-c:v",
                "libvpx-vp9",
                "-crf",
                "30",
                "-b:v",
                "0",
                "-deadline",
                "realtime",
                "-cpu-used",
                "4",
                "-pix_fmt",
                "yuv420p",
                "clip.webm",
            ]
        );
    }

    #[test]
    fn build_args_macos_avfoundation_gif_with_region_and_palette_filtergraph() {
        let mut config = RecordingConfig::new("demo.gif");
        config.format = RecordingFormat::Gif;
        config.encoder = VideoEncoder::GifPalette;
        config.fps = 15;
        config.region = Some(Region::new(40, 80, 640, 480).expect("valid region"));

        let args = build_recording_args(&config, CapturePlatform::MacOsAvFoundation)
            .expect("args build succeeds");

        assert_eq!(
            args,
            vec![
                "-y",
                "-f",
                "avfoundation",
                "-framerate",
                "15",
                "-capture_cursor",
                "1",
                "-i",
                "1:none",
                "-vf",
                "crop=640:480:40:80,fps=15,split[s0][s1];[s0]palettegen=stats_mode=single[p];[s1][p]paletteuse=new=1",
                "-loop",
                "0",
                "demo.gif",
            ]
        );
    }

    #[test]
    fn build_args_windows_gif_uses_palette_filtergraph() {
        let mut config = RecordingConfig::new("demo.gif");
        config.format = RecordingFormat::Gif;
        config.fps = 12;

        let args = build_recording_args(&config, CapturePlatform::WindowsGdiGrab)
            .expect("args build succeeds");

        assert_eq!(
            args,
            vec![
                "-y",
                "-f",
                "gdigrab",
                "-framerate",
                "12",
                "-draw_mouse",
                "1",
                "-i",
                "desktop",
                "-vf",
                "fps=12,split[s0][s1];[s0]palettegen=stats_mode=single[p];[s1][p]paletteuse=new=1",
                "-loop",
                "0",
                "demo.gif",
            ]
        );
    }

    #[test]
    fn gif_spool_and_transcode_args_avoid_live_ram_buffering_and_force_even_dimensions() {
        let mut config = RecordingConfig::new("clip.gif");
        config.format = RecordingFormat::Gif;
        config.fps = 15;
        config.region = Some(Region::new(0, 0, 65, 64).expect("odd width region"));

        let spool = Path::new("tmp_spool.mkv");
        let spool_args = build_gif_spool_args(&config, CapturePlatform::WindowsGdiGrab, spool)
            .expect("spool args");
        assert!(spool_args.contains(&"ultrafast".to_owned()));
        assert!(spool_args.contains(&"scale=trunc(iw/2)*2:trunc(ih/2)*2".to_owned()));
        assert!(!spool_args.contains(&"-movflags".to_owned()));
        assert_eq!(spool_args.last().map(String::as_str), Some("tmp_spool.mkv"));

        let transcode_args = build_gif_transcode_args(spool, Path::new("clip.gif"), 15);
        assert_eq!(
            transcode_args,
            vec![
                "-y",
                "-i",
                "tmp_spool.mkv",
                "-vf",
                "fps=15,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse",
                "-loop",
                "0",
                "clip.gif",
            ]
        );
    }

    #[test]
    fn build_args_rejects_negative_offsets_on_x11_and_macos() {
        let mut config = RecordingConfig::new("out.mp4");
        config.region = Some(Region::new(-10, 0, 100, 100).expect("valid region"));

        assert!(matches!(
            build_recording_args(&config, CapturePlatform::LinuxX11Grab),
            Err(MediaError::InvalidConfig(_))
        ));
        assert!(matches!(
            build_recording_args(&config, CapturePlatform::MacOsAvFoundation),
            Err(MediaError::InvalidConfig(_))
        ));
    }

    #[test]
    fn find_ffmpeg_honors_explicit_existing_override_and_rejects_missing_path() {
        let dir = tempdir().expect("temp dir");
        let fake_bin = dir.path().join(ffmpeg_binary_name());
        fs::write(&fake_bin, b"stub").expect("write fake ffmpeg");

        let found = find_ffmpeg(Some(&fake_bin)).expect("explicit override found");
        assert_eq!(found, fake_bin);

        let missing = dir.path().join("does-not-exist-ffmpeg");
        assert!(matches!(
            find_ffmpeg(Some(&missing)),
            Err(MediaError::FfmpegNotFound)
        ));
    }

    #[test]
    fn find_ffmpeg_scans_search_dirs_when_no_override_given() {
        let empty_dir = tempdir().expect("empty dir");
        let bin_dir = tempdir().expect("bin dir");
        let fake_bin = bin_dir.path().join(ffmpeg_binary_name());
        fs::write(&fake_bin, b"stub").expect("write fake ffmpeg");

        let found = find_ffmpeg_in_dirs(
            None,
            &[empty_dir.path().to_path_buf(), bin_dir.path().to_path_buf()],
        )
        .expect("found in search dirs");
        assert_eq!(found, fake_bin);

        assert!(matches!(
            find_ffmpeg_in_dirs(None, &[empty_dir.path().to_path_buf()]),
            Err(MediaError::FfmpegNotFound)
        ));
    }

    #[test]
    fn parse_available_h264_encoders_orders_by_priority_and_ignores_non_matches() {
        let stdout = "\
Encoders:
 V..... = Video
 A..... = Audio
 ------
 VFS..D libx264              libx264 H.264 / AVC (codec h264)
 VFS..D libx264rgb           libx264 RGB (codec h264)
 V..... h264_amf             AMD AMF H.264 Encoder (codec h264)
 V..... h264_qsv             Intel Quick Sync Video (codec h264)
 V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
 A....D aac                  AAC (Advanced Audio Coding)
";
        let encoders = parse_available_h264_encoders(stdout);
        assert_eq!(
            encoders,
            vec![
                VideoEncoder::NvencH264,
                VideoEncoder::QsvH264,
                VideoEncoder::AmfH264,
                VideoEncoder::Libx264,
            ]
        );
        assert_eq!(preferred_mp4_encoder(&encoders), VideoEncoder::NvencH264);

        let qsv_and_amf = parse_available_h264_encoders(
            " V..... h264_amf AMD\n V..... h264_qsv Intel\n VFS..D libx264rgb RGB only\n",
        );
        assert_eq!(
            qsv_and_amf,
            vec![VideoEncoder::QsvH264, VideoEncoder::AmfH264]
        );
        assert_eq!(preferred_mp4_encoder(&qsv_and_amf), VideoEncoder::QsvH264);

        let amf_only = parse_available_h264_encoders(" V..... h264_amf AMD\n");
        assert_eq!(amf_only, vec![VideoEncoder::AmfH264]);
        assert_eq!(preferred_mp4_encoder(&amf_only), VideoEncoder::AmfH264);

        assert_eq!(preferred_mp4_encoder(&[]), VideoEncoder::Libx264);
        assert_eq!(
            preferred_mp4_encoder(&[VideoEncoder::Libx264, VideoEncoder::AmfH264]),
            VideoEncoder::AmfH264
        );
    }

    #[test]
    fn detect_available_h264_encoders_falls_back_to_libx264_on_error() {
        let missing = Path::new("definitely-missing-ffmpeg-binary-for-test");
        assert_eq!(
            detect_available_h264_encoders(missing),
            vec![VideoEncoder::Libx264]
        );
    }

    #[test]
    fn build_args_supports_qsv_and_amf_hardware_encoders() {
        let mut qsv_config = RecordingConfig::new("qsv.mp4");
        qsv_config.encoder = VideoEncoder::QsvH264;
        qsv_config.crf = 21;

        let qsv_args = build_recording_args(&qsv_config, CapturePlatform::WindowsGdiGrab)
            .expect("qsv args succeed");
        assert_eq!(
            qsv_args,
            vec![
                "-y",
                "-f",
                "gdigrab",
                "-framerate",
                "30",
                "-draw_mouse",
                "1",
                "-i",
                "desktop",
                "-c:v",
                "h264_qsv",
                "-global_quality",
                "21",
                "-vf",
                "scale=trunc(iw/2)*2:trunc(ih/2)*2",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
                "qsv.mp4",
            ]
        );

        let mut amf_config = RecordingConfig::new("amf.mp4");
        amf_config.encoder = VideoEncoder::AmfH264;
        amf_config.crf = 20;

        let amf_args = build_recording_args(&amf_config, CapturePlatform::WindowsGdiGrab)
            .expect("amf args succeed");
        assert_eq!(
            amf_args,
            vec![
                "-y",
                "-f",
                "gdigrab",
                "-framerate",
                "30",
                "-draw_mouse",
                "1",
                "-i",
                "desktop",
                "-c:v",
                "h264_amf",
                "-rc",
                "cqp",
                "-qp_i",
                "20",
                "-qp_p",
                "20",
                "-vf",
                "scale=trunc(iw/2)*2:trunc(ih/2)*2",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
                "amf.mp4",
            ]
        );
    }

    #[test]
    fn parse_dshow_audio_devices_supports_legacy_and_ffmpeg8_formats() {
        let legacy_stderr = "\
[dshow @ 000001e2b3c4d5e0] DirectShow video devices (some may be both video and audio devices)
[dshow @ 000001e2b3c4d5e0]  \"Integrated Webcam\"
[dshow @ 000001e2b3c4d5e0]     Alternative name \"@device_pnp_usb\"
[dshow @ 000001e2b3c4d5e0] DirectShow audio devices
[dshow @ 000001e2b3c4d5e0]  \"Microphone (Realtek(R) Audio)\"
[dshow @ 000001e2b3c4d5e0]     Alternative name \"@device_cm_1\"
[dshow @ 000001e2b3c4d5e0]  \"Stereo Mix (Realtek(R) Audio)\"
[dshow @ 000001e2b3c4d5e0]     Alternative name \"@device_cm_2\"
dummy: Immediate exit requested
";
        assert_eq!(
            parse_dshow_audio_devices(legacy_stderr),
            vec![
                AudioDevice {
                    name: "Microphone (Realtek(R) Audio)".to_owned(),
                    backend: "dshow",
                },
                AudioDevice {
                    name: "Stereo Mix (Realtek(R) Audio)".to_owned(),
                    backend: "dshow",
                },
            ]
        );

        let modern_stderr = "\
[dshow @ 0000024a1b2c3d40] \"HD Pro Webcam\" (video)
[dshow @ 0000024a1b2c3d40]   Alternative name \"@device_pnp_cam\"
[dshow @ 0000024a1b2c3d40] \"Microphone (Blue Yeti)\" (audio)
[dshow @ 0000024a1b2c3d40]   Alternative name \"@device_cm_yeti\"
[dshow @ 0000024a1b2c3d40] \"Elgato Capture\" (video, audio)
[dshow @ 0000024a1b2c3d40] \"Unused Device\" (none)
[dshow @ 0000024a1b2c3d40] \"Microphone (Blue Yeti)\" (audio)
Error opening input file dummy.
";
        assert_eq!(
            parse_dshow_audio_devices(modern_stderr),
            vec![
                AudioDevice {
                    name: "Microphone (Blue Yeti)".to_owned(),
                    backend: "dshow",
                },
                AudioDevice {
                    name: "Elgato Capture".to_owned(),
                    backend: "dshow",
                },
            ]
        );
    }

    #[test]
    fn parse_avfoundation_audio_devices_extracts_audio_section_only() {
        let stderr = "\
[AVFoundation indev @ 0x7f8b1c004080] AVFoundation video devices:
[AVFoundation indev @ 0x7f8b1c004080] [0] FaceTime HD Camera
[AVFoundation indev @ 0x7f8b1c004080] [1] Capture screen 0
[AVFoundation indev @ 0x7f8b1c004080] AVFoundation audio devices:
[AVFoundation indev @ 0x7f8b1c004080] [0] MacBook Pro Microphone
[AVFoundation indev @ 0x7f8b1c004080] [1] BlackHole 2ch
: Input/output error
";
        assert_eq!(
            parse_avfoundation_audio_devices(stderr),
            vec![
                AudioDevice {
                    name: "MacBook Pro Microphone".to_owned(),
                    backend: "avfoundation",
                },
                AudioDevice {
                    name: "BlackHole 2ch".to_owned(),
                    backend: "avfoundation",
                },
            ]
        );
    }

    #[test]
    fn list_audio_devices_returns_empty_on_failure_and_linux() {
        let missing = Path::new("definitely-missing-ffmpeg-binary-for-test");
        assert!(list_audio_devices(missing, CapturePlatform::WindowsGdiGrab).is_empty());
        assert!(list_audio_devices(missing, CapturePlatform::MacOsAvFoundation).is_empty());
        assert!(list_audio_devices(missing, CapturePlatform::LinuxX11Grab).is_empty());
    }

    #[test]
    fn build_args_adds_audio_input_and_codecs_for_mp4_and_webm_and_omits_for_gif() {
        let mut win_mp4 = RecordingConfig::new("voice.mp4");
        win_mp4.audio_device = Some("Microphone (USB)".to_owned());
        let win_mp4_args =
            build_recording_args(&win_mp4, CapturePlatform::WindowsGdiGrab).expect("win mp4");
        assert_eq!(
            win_mp4_args,
            vec![
                "-y",
                "-f",
                "gdigrab",
                "-framerate",
                "30",
                "-draw_mouse",
                "1",
                "-i",
                "desktop",
                "-f",
                "dshow",
                "-i",
                "audio=Microphone (USB)",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "23",
                "-vf",
                "scale=trunc(iw/2)*2:trunc(ih/2)*2",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
                "voice.mp4",
            ]
        );

        let mut linux_webm = RecordingConfig::new("voice.webm");
        linux_webm.format = RecordingFormat::WebmVp9;
        linux_webm.audio_device = Some("default".to_owned());
        let linux_webm_args =
            build_recording_args(&linux_webm, CapturePlatform::LinuxX11Grab).expect("linux webm");
        assert_eq!(
            linux_webm_args,
            vec![
                "-y",
                "-f",
                "x11grab",
                "-framerate",
                "30",
                "-draw_mouse",
                "1",
                "-i",
                ":0.0",
                "-f",
                "pulse",
                "-i",
                "default",
                "-c:v",
                "libvpx-vp9",
                "-crf",
                "23",
                "-b:v",
                "0",
                "-deadline",
                "realtime",
                "-cpu-used",
                "4",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "libopus",
                "-b:a",
                "128k",
                "voice.webm",
            ]
        );

        let mut mac_mp4 = RecordingConfig::new("mac.mp4");
        mac_mp4.audio_device = Some("0".to_owned());
        let mac_mp4_args =
            build_recording_args(&mac_mp4, CapturePlatform::MacOsAvFoundation).expect("mac mp4");
        assert_eq!(
            mac_mp4_args,
            vec![
                "-y",
                "-f",
                "avfoundation",
                "-framerate",
                "30",
                "-capture_cursor",
                "1",
                "-i",
                "1:0",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "23",
                "-vf",
                "scale=trunc(iw/2)*2:trunc(ih/2)*2",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
                "mac.mp4",
            ]
        );

        let mut gif_with_audio = RecordingConfig::new("silent.gif");
        gif_with_audio.format = RecordingFormat::Gif;
        gif_with_audio.audio_device = Some("Microphone (USB)".to_owned());
        for platform in [
            CapturePlatform::WindowsGdiGrab,
            CapturePlatform::LinuxX11Grab,
            CapturePlatform::MacOsAvFoundation,
        ] {
            let gif_args = build_recording_args(&gif_with_audio, platform).expect("gif args");
            assert!(!gif_args.contains(&"-c:a".to_owned()));
            assert!(!gif_args.contains(&"dshow".to_owned()));
            assert!(!gif_args.contains(&"pulse".to_owned()));
            if platform == CapturePlatform::MacOsAvFoundation {
                assert!(gif_args.contains(&"1:none".to_owned()));
            }
        }

        let spool_args = build_gif_spool_args(
            &gif_with_audio,
            CapturePlatform::WindowsGdiGrab,
            Path::new("tmp.mkv"),
        )
        .expect("gif spool args");
        assert!(!spool_args.contains(&"-c:a".to_owned()));
        assert!(!spool_args.contains(&"dshow".to_owned()));
    }

    #[test]
    fn build_args_rejects_newline_and_nul_injection_in_audio_device() {
        for malicious in ["mic\n-f lavfi", "mic\r-i evil", "mic\0injected"] {
            let mut config = RecordingConfig::new("out.mp4");
            config.audio_device = Some(malicious.to_owned());
            assert!(matches!(
                build_recording_args(&config, CapturePlatform::WindowsGdiGrab),
                Err(MediaError::InvalidConfig(msg)) if msg.contains("audio_device")
            ));
        }
    }
}
