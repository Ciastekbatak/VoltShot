use crate::config::{CapturePlatform, RecordingConfig, RecordingFormat, VideoEncoder};
use std::env;
use std::path::{Path, PathBuf};
use thiserror::Error;

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
    let encoder = config.encoder.resolve(config.format)?;
    let cursor_flag = if config.draw_cursor { "1" } else { "0" };

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
            args.push("1:none".to_owned());
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
    let mut spool_config = config.clone();
    spool_config.format = RecordingFormat::Mp4H264;
    spool_config.encoder = VideoEncoder::Libx264;
    spool_config.crf = 18;
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
}
