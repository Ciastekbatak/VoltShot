use crate::config::{CapturePlatform, RecordingConfig, RecordingFormat, VideoEncoder};
use crate::ffmpeg::{
    MediaError, build_gif_spool_args, build_gif_transcode_args, build_recording_args, find_ffmpeg,
};
use std::fs;
use std::io::{ErrorKind, Read, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(15);
const GIF_FINALIZE_TIMEOUT: Duration = Duration::from_secs(120);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(25);
const HARDWARE_STARTUP_PROBE: Duration = Duration::from_millis(120);
const MAX_CAPTURED_STDERR_BYTES: usize = 16 * 1024;

struct GifSpoolJob {
    ffmpeg_bin: PathBuf,
    spool_path: PathBuf,
    output_path: PathBuf,
    fps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingState {
    Idle,
    Recording {
        output_path: PathBuf,
        started_at: Instant,
    },
    Completed {
        output_path: PathBuf,
        duration: Duration,
    },
    Failed(String),
}

impl RecordingState {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Recording { .. } => "recording",
            Self::Completed { .. } => "completed",
            Self::Failed(_) => "failed",
        }
    }

    pub const fn is_recording(&self) -> bool {
        matches!(self, Self::Recording { .. })
    }

    pub fn output_path(&self) -> Option<&Path> {
        match self {
            Self::Recording { output_path, .. } | Self::Completed { output_path, .. } => {
                Some(output_path.as_path())
            }
            Self::Idle | Self::Failed(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingStateMachine {
    state: RecordingState,
}

impl Default for RecordingStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingStateMachine {
    pub fn new() -> Self {
        Self {
            state: RecordingState::Idle,
        }
    }

    pub fn state(&self) -> &RecordingState {
        &self.state
    }

    pub fn start(&mut self, output_path: PathBuf, started_at: Instant) -> Result<(), MediaError> {
        match self.state {
            RecordingState::Idle => {
                self.state = RecordingState::Recording {
                    output_path,
                    started_at,
                };
                Ok(())
            }
            _ => Err(MediaError::InvalidStateTransition {
                from: self.state.label(),
                action: "start",
            }),
        }
    }

    pub fn finish(&mut self, finished_at: Instant) -> Result<(PathBuf, Duration), MediaError> {
        match &self.state {
            RecordingState::Recording {
                output_path,
                started_at,
            } => {
                let output_path = output_path.clone();
                let duration = finished_at.saturating_duration_since(*started_at);
                self.state = RecordingState::Completed {
                    output_path: output_path.clone(),
                    duration,
                };
                Ok((output_path, duration))
            }
            _ => Err(MediaError::InvalidStateTransition {
                from: self.state.label(),
                action: "finish",
            }),
        }
    }

    pub fn abort(&mut self) -> Result<PathBuf, MediaError> {
        match &self.state {
            RecordingState::Recording { output_path, .. } => {
                let output_path = output_path.clone();
                self.state = RecordingState::Idle;
                Ok(output_path)
            }
            _ => Err(MediaError::InvalidStateTransition {
                from: self.state.label(),
                action: "abort",
            }),
        }
    }

    pub fn fail(&mut self, reason: impl Into<String>) -> Result<(), MediaError> {
        match self.state {
            RecordingState::Recording { .. } => {
                self.state = RecordingState::Failed(reason.into());
                Ok(())
            }
            _ => Err(MediaError::InvalidStateTransition {
                from: self.state.label(),
                action: "fail",
            }),
        }
    }

    pub fn reset(&mut self) -> Result<(), MediaError> {
        match self.state {
            RecordingState::Idle | RecordingState::Completed { .. } | RecordingState::Failed(_) => {
                self.state = RecordingState::Idle;
                Ok(())
            }
            RecordingState::Recording { .. } => Err(MediaError::InvalidStateTransition {
                from: self.state.label(),
                action: "reset",
            }),
        }
    }
}

pub struct ScreenRecorder {
    machine: RecordingStateMachine,
    encoder: VideoEncoder,
    child: Option<Child>,
    stderr_handle: Option<JoinHandle<String>>,
    gif_spool: Option<GifSpoolJob>,
    platform: CapturePlatform,
}

impl Default for ScreenRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenRecorder {
    pub fn new() -> Self {
        Self::with_platform(CapturePlatform::current())
    }

    pub fn with_platform(platform: CapturePlatform) -> Self {
        Self {
            machine: RecordingStateMachine::new(),
            encoder: VideoEncoder::Auto,
            child: None,
            stderr_handle: None,
            gif_spool: None,
            platform,
        }
    }

    pub fn state(&self) -> &RecordingState {
        self.machine.state()
    }

    /// Returns the encoder used by the active recording, including any startup fallback.
    pub fn active_encoder(&self) -> Option<VideoEncoder> {
        self.state().is_recording().then_some(self.encoder)
    }

    pub fn elapsed(&self) -> Option<Duration> {
        match self.machine.state() {
            RecordingState::Recording { started_at, .. } => Some(started_at.elapsed()),
            _ => None,
        }
    }

    pub fn start(&mut self, config: &RecordingConfig) -> Result<(), MediaError> {
        if !matches!(self.machine.state(), RecordingState::Idle) {
            return Err(MediaError::InvalidStateTransition {
                from: self.machine.state().label(),
                action: "start",
            });
        }

        let ffmpeg_bin = find_ffmpeg(config.ffmpeg_path.as_deref())?;

        if let Some(parent) = config.output_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }

        if config.format != RecordingFormat::Gif {
            let platform = self.platform;
            return self.start_capture(config, |attempt| {
                let mut command = Command::new(&ffmpeg_bin);
                command.args(build_recording_args(attempt, platform)?);
                Ok(command)
            });
        }

        let spool_path = make_gif_spool_path(&config.output_path);
        let mut command = Command::new(&ffmpeg_bin);
        command.args(build_gif_spool_args(config, self.platform, &spool_path)?);
        self.spawn_command(command, config.output_path.clone(), false)?;
        self.encoder = VideoEncoder::GifPalette;
        self.gif_spool = Some(GifSpoolJob {
            ffmpeg_bin,
            spool_path,
            output_path: config.output_path.clone(),
            fps: config.fps,
        });
        Ok(())
    }

    fn start_capture(
        &mut self,
        config: &RecordingConfig,
        mut command: impl FnMut(&RecordingConfig) -> Result<Command, MediaError>,
    ) -> Result<(), MediaError> {
        let mut encoder = config.resolved_encoder()?;
        let probe = config.fallback_to_libx264_on_hw_fail && encoder.is_hardware_h264();
        match self.spawn_command(command(config)?, config.output_path.clone(), probe) {
            Err(MediaError::ProcessFailed(_)) if probe => {
                let mut fallback = config.clone();
                encoder = VideoEncoder::Libx264;
                fallback.encoder = encoder;
                self.spawn_command(command(&fallback)?, fallback.output_path, true)?;
            }
            result => result?,
        }
        self.encoder = encoder;
        Ok(())
    }

    fn spawn_command(
        &mut self,
        mut command: Command,
        output_path: PathBuf,
        probe: bool,
    ) -> Result<(), MediaError> {
        if !matches!(self.machine.state(), RecordingState::Idle) {
            return Err(MediaError::InvalidStateTransition {
                from: self.machine.state().label(),
                action: "start",
            });
        }

        configure_no_window(&mut command);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;

        let started_at = Instant::now();
        let stderr_handle = child.stderr.take().map(spawn_stderr_drain);

        if probe {
            thread::sleep(HARDWARE_STARTUP_PROBE);
            match child.try_wait() {
                Ok(Some(status)) if !status.success() => {
                    let stderr = join_stderr(stderr_handle);
                    let detail = if stderr.is_empty() {
                        format!("ffmpeg exited with {status}")
                    } else {
                        stderr
                    };
                    return Err(MediaError::ProcessFailed(detail));
                }
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(err.into());
                }
                Ok(_) => {}
            }
        }

        self.machine.start(output_path, started_at)?;
        self.child = Some(child);
        self.stderr_handle = stderr_handle;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(PathBuf, Duration), MediaError> {
        if !self.machine.state().is_recording() {
            return Err(MediaError::InvalidStateTransition {
                from: self.machine.state().label(),
                action: "finish",
            });
        }

        let gif_spool = self.gif_spool.take();
        let Some(mut child) = self.child.take() else {
            if let Some(job) = gif_spool {
                let _ = fs::remove_file(&job.spool_path);
            }
            let reason = "recording process handle is missing".to_owned();
            self.machine.fail(reason.clone())?;
            return Err(MediaError::ProcessFailed(reason));
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(b"q\n");
            let _ = stdin.flush();
        }

        let status = wait_with_timeout(&mut child, GRACEFUL_STOP_TIMEOUT)?;
        let stderr = self.collect_stderr();

        if !status.success() {
            if let Some(job) = gif_spool {
                let _ = fs::remove_file(&job.spool_path);
            }
            let detail = if stderr.is_empty() {
                format!("exited with status {status}")
            } else {
                format!("exited with status {status}: {stderr}")
            };
            self.machine.fail(detail.clone())?;
            return Err(MediaError::ProcessFailed(detail));
        }

        if let Some(job) = gif_spool {
            let transcode_result = finalize_spooled_gif(&job);
            let _ = fs::remove_file(&job.spool_path);
            if let Err(err) = transcode_result {
                let detail = err.to_string();
                self.machine.fail(detail)?;
                return Err(err);
            }
        }

        self.machine.finish(Instant::now())
    }

    pub fn abort(&mut self) -> Result<PathBuf, MediaError> {
        if !self.machine.state().is_recording() {
            return Err(MediaError::InvalidStateTransition {
                from: self.machine.state().label(),
                action: "abort",
            });
        }

        let output_path = self.machine.abort()?;

        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = self.collect_stderr();
        if let Some(job) = self.gif_spool.take() {
            let _ = fs::remove_file(&job.spool_path);
        }

        if let Err(err) = fs::remove_file(&output_path)
            && err.kind() != ErrorKind::NotFound
        {
            return Err(MediaError::Io(err));
        }

        Ok(output_path)
    }

    pub fn reset(&mut self) -> Result<(), MediaError> {
        self.machine.reset()
    }

    fn collect_stderr(&mut self) -> String {
        join_stderr(self.stderr_handle.take())
    }
}

fn join_stderr(handle: Option<JoinHandle<String>>) -> String {
    handle
        .and_then(|handle| handle.join().ok())
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

impl Drop for ScreenRecorder {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = self.collect_stderr();
        if let Some(job) = self.gif_spool.take() {
            let _ = fs::remove_file(&job.spool_path);
        }
    }
}

fn make_gif_spool_path(output_path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let file_name = format!(".sharex-gif-spool-{pid}-{nanos}-{seq}.mkv");
    output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(
            || std::env::temp_dir().join(&file_name),
            |parent| parent.join(&file_name),
        )
}

fn finalize_spooled_gif(job: &GifSpoolJob) -> Result<(), MediaError> {
    let args = build_gif_transcode_args(&job.spool_path, &job.output_path, job.fps);
    let mut command = Command::new(&job.ffmpeg_bin);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    configure_no_window(&mut command);
    let mut child = command.spawn()?;
    let stderr_handle = child.stderr.take().map(spawn_stderr_drain);
    let status = wait_with_timeout(&mut child, GIF_FINALIZE_TIMEOUT)?;
    let stderr = stderr_handle
        .and_then(|handle| handle.join().ok())
        .map(|text| text.trim().to_owned())
        .unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        let detail = if stderr.is_empty() {
            format!("GIF transcode exited with status {status}")
        } else {
            format!("GIF transcode exited with status {status}: {stderr}")
        };
        Err(MediaError::ProcessFailed(detail))
    }
}

fn spawn_stderr_drain<R: Read + Send + 'static>(mut reader: R) -> JoinHandle<String> {
    thread::spawn(move || {
        let mut captured = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    captured.extend_from_slice(&chunk[..n]);
                    if captured.len() > MAX_CAPTURED_STDERR_BYTES {
                        let excess = captured.len() - MAX_CAPTURED_STDERR_BYTES;
                        captured.drain(..excess);
                    }
                }
            }
        }
        String::from_utf8_lossy(&captured).into_owned()
    })
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<ExitStatus, MediaError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Ok(child.wait()?);
        }
        thread::sleep(STOP_POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn state_machine_happy_path_start_finish_reset() {
        let mut machine = RecordingStateMachine::new();
        assert_eq!(machine.state(), &RecordingState::Idle);

        let t0 = Instant::now();
        let path = PathBuf::from("video.mp4");
        machine.start(path.clone(), t0).expect("start from Idle");
        assert!(machine.state().is_recording());
        assert_eq!(machine.state().output_path(), Some(path.as_path()));

        let t1 = t0 + Duration::from_secs(3);
        let (finished_path, duration) = machine.finish(t1).expect("finish from Recording");
        assert_eq!(finished_path, path);
        assert_eq!(duration, Duration::from_secs(3));
        assert_eq!(
            machine.state(),
            &RecordingState::Completed {
                output_path: path,
                duration: Duration::from_secs(3),
            }
        );

        machine.reset().expect("reset from Completed");
        assert_eq!(machine.state(), &RecordingState::Idle);
    }

    #[test]
    fn state_machine_abort_and_fail_transitions() {
        let mut machine = RecordingStateMachine::new();
        let path = PathBuf::from("clip.gif");

        machine
            .start(path.clone(), Instant::now())
            .expect("start succeeds");
        let aborted = machine.abort().expect("abort succeeds");
        assert_eq!(aborted, path);
        assert_eq!(machine.state(), &RecordingState::Idle);

        machine
            .start(PathBuf::from("clip2.mp4"), Instant::now())
            .expect("restart after abort");
        machine
            .fail("encoder crashed")
            .expect("fail from Recording");
        assert_eq!(
            machine.state(),
            &RecordingState::Failed("encoder crashed".to_owned())
        );

        machine.reset().expect("reset from Failed");
        assert_eq!(machine.state(), &RecordingState::Idle);
    }

    #[test]
    fn state_machine_rejects_illegal_transitions() {
        let mut machine = RecordingStateMachine::new();

        assert!(matches!(
            machine.finish(Instant::now()),
            Err(MediaError::InvalidStateTransition {
                from: "idle",
                action: "finish"
            })
        ));
        assert!(matches!(
            machine.abort(),
            Err(MediaError::InvalidStateTransition {
                from: "idle",
                action: "abort"
            })
        ));
        assert!(matches!(
            machine.fail("boom"),
            Err(MediaError::InvalidStateTransition {
                from: "idle",
                action: "fail"
            })
        ));

        let t0 = Instant::now();
        machine
            .start(PathBuf::from("a.mp4"), t0)
            .expect("initial start");

        assert!(matches!(
            machine.start(PathBuf::from("b.mp4"), t0),
            Err(MediaError::InvalidStateTransition {
                from: "recording",
                action: "start"
            })
        ));
        assert!(matches!(
            machine.reset(),
            Err(MediaError::InvalidStateTransition {
                from: "recording",
                action: "reset"
            })
        ));

        machine.finish(t0 + Duration::from_secs(1)).expect("finish");
        assert!(matches!(
            machine.start(PathBuf::from("c.mp4"), t0),
            Err(MediaError::InvalidStateTransition {
                from: "completed",
                action: "start"
            })
        ));
        assert!(matches!(
            machine.finish(t0 + Duration::from_secs(2)),
            Err(MediaError::InvalidStateTransition {
                from: "completed",
                action: "finish"
            })
        ));
    }

    #[test]
    fn screen_recorder_graceful_stop_sends_quit_to_stdin() {
        let dir = tempdir().expect("temp dir");
        let output = dir.path().join("recorded.mp4");
        let mut recorder = ScreenRecorder::new();

        #[cfg(windows)]
        let command = {
            let mut cmd = Command::new("cmd");
            cmd.args([
                "/V:ON",
                "/C",
                "set /p line=& if !line!==q (exit /b 0) else (exit /b 1)",
            ]);
            cmd
        };
        #[cfg(not(windows))]
        let command = {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "read line && [ \"$line\" = \"q\" ]"]);
            cmd
        };

        recorder
            .spawn_command(command, output.clone(), false)
            .expect("spawn child");
        assert!(recorder.state().is_recording());
        assert!(recorder.elapsed().is_some());

        let (finished_path, _) = recorder.stop().expect("graceful stop succeeds");
        assert_eq!(finished_path, output);
        assert!(matches!(recorder.state(), RecordingState::Completed { .. }));
    }

    #[test]
    fn screen_recorder_retries_cpu_if_hardware_exits_during_startup() {
        let dir = tempdir().expect("temp dir");
        let mut config = RecordingConfig::new(dir.path().join("recorded.mp4"));
        config.encoder = VideoEncoder::NvencH264;
        config.fallback_to_libx264_on_hw_fail = true;
        let mut recorder = ScreenRecorder::new();
        let mut attempts = Vec::new();

        recorder
            .start_capture(&config, |attempt| {
                attempts.push(attempt.encoder);
                let hardware = attempt.encoder == VideoEncoder::NvencH264;
                #[cfg(windows)]
                let mut command = Command::new("cmd");
                #[cfg(windows)]
                command.args([
                    "/C",
                    if hardware {
                        "exit /b 1"
                    } else {
                        "set /p line="
                    },
                ]);
                #[cfg(not(windows))]
                let mut command = Command::new("sh");
                #[cfg(not(windows))]
                command.args(["-c", if hardware { "exit 1" } else { "read line" }]);
                Ok(command)
            })
            .expect("software fallback starts");

        assert_eq!(attempts, [VideoEncoder::NvencH264, VideoEncoder::Libx264]);
        assert!(recorder.state().is_recording());
        assert_eq!(recorder.active_encoder(), Some(VideoEncoder::Libx264));
        assert_eq!(
            recorder.stop().expect("software recording stops").0,
            config.output_path
        );
    }

    #[test]
    fn screen_recorder_abort_kills_process_and_cleans_partial_output() {
        let dir = tempdir().expect("temp dir");
        let output = dir.path().join("partial.mp4");
        fs::write(&output, b"partial video bytes").expect("create partial output");

        let mut recorder = ScreenRecorder::new();

        #[cfg(windows)]
        let command = {
            let mut cmd = Command::new("ping");
            cmd.args(["-n", "30", "127.0.0.1"]);
            cmd
        };
        #[cfg(not(windows))]
        let command = {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "sleep 30"]);
            cmd
        };

        recorder
            .spawn_command(command, output.clone(), false)
            .expect("spawn long-running child");

        let aborted_path = recorder.abort().expect("abort succeeds");
        assert_eq!(aborted_path, output);
        assert_eq!(recorder.state(), &RecordingState::Idle);
        assert!(!output.exists());
    }

    #[test]
    fn screen_recorder_nonzero_exit_transitions_to_failed_with_stderr() {
        let dir = tempdir().expect("temp dir");
        let output = dir.path().join("broken.mp4");
        let mut recorder = ScreenRecorder::new();

        #[cfg(windows)]
        let command = {
            let mut cmd = Command::new("cmd");
            cmd.args(["/C", "echo simulated ffmpeg error 1>&2 && exit /b 1"]);
            cmd
        };
        #[cfg(not(windows))]
        let command = {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "echo 'simulated ffmpeg error' >&2; exit 1"]);
            cmd
        };

        recorder
            .spawn_command(command, output, false)
            .expect("spawn failing child");

        let err = recorder.stop().expect_err("non-zero exit must fail");
        assert!(matches!(
            err,
            MediaError::ProcessFailed(ref msg) if msg.contains("simulated ffmpeg error")
        ));
        assert!(matches!(
            recorder.state(),
            RecordingState::Failed(msg) if msg.contains("simulated ffmpeg error")
        ));
    }
}
