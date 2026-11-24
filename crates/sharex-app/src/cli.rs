use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use sharex_core::{
    AppConfig, CapturePipeline, CaptureSource, ClipboardSink, CoreError, HistoryRepository,
    ImageFormat, MonitorId, NoopClipboard, PipelineDeps, Region, ScreenCapturer,
};
use sharex_uploaders::UploaderRegistry;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// ShareX in Rust command-line interface.
#[derive(Debug, Parser)]
#[command(
    name = "sharex",
    version,
    about = "Screen capture and sharing tool in Rust"
)]
pub struct Cli {
    /// Override the TOML config file path.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Store config, screenshots, and history beside the executable.
    #[arg(long, global = true)]
    pub portable: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Launch the native desktop window.
    Gui,
    /// List available displays.
    Monitors,
    /// Capture a screen, monitor, or region and run the after-capture pipeline.
    Capture(CaptureArgs),
    /// Inspect or clear capture history.
    History {
        #[command(subcommand)]
        action: HistoryCommand,
    },
    /// Inspect application configuration.
    Config {
        #[command(subcommand)]
        action: ConfigCommand,
    },
    /// List configured global hotkeys.
    Hotkeys,
    /// List registered uploaders.
    Uploaders,
}

#[derive(Debug, Args)]
pub struct CaptureArgs {
    /// Capture the currently focused application window.
    #[arg(long, conflicts_with_all = ["active_monitor", "monitor", "region"])]
    pub active_window: bool,

    /// Capture the monitor that currently contains the mouse cursor.
    #[arg(long, conflicts_with_all = ["active_window", "monitor", "region"])]
    pub active_monitor: bool,

    /// Capture a specific monitor by numeric id from `sharex monitors`.
    #[arg(long, conflicts_with_all = ["region", "active_window", "active_monitor"])]
    pub monitor: Option<u32>,

    /// Capture a rectangular region specified as `x,y,width,height`.
    #[arg(long, conflicts_with_all = ["monitor", "active_window", "active_monitor"])]
    pub region: Option<String>,

    /// Override output directory for saved captures.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Override output image format (`png`, `jpg`, `webp`).
    #[arg(long)]
    pub format: Option<ImageFormat>,

    /// Override ShareX filename pattern.
    #[arg(long)]
    pub pattern: Option<String>,

    /// Skip writing the capture to disk.
    #[arg(long)]
    pub no_save: bool,

    /// Copy the captured image to the clipboard.
    #[arg(long)]
    pub copy_image: bool,

    /// Copy the saved file path or upload URL to the clipboard.
    #[arg(long)]
    pub copy_path: bool,

    /// Disable all clipboard writes for this run.
    #[arg(long)]
    pub no_clipboard: bool,

    /// Upload the capture using a registered uploader id.
    #[arg(long)]
    pub uploader: Option<String>,

    /// Skip appending an entry to capture history.
    #[arg(long)]
    pub no_history: bool,
}

#[derive(Debug, Subcommand)]
pub enum HistoryCommand {
    /// Print recent captures newest-first.
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Delete all recorded history entries.
    Clear,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the active configuration as TOML.
    Show,
    /// Print the resolved configuration file path.
    Path,
}

/// Executes non-GUI CLI subcommands against injected platform boundaries.
pub fn execute_headless(
    command: Command,
    config_path: &Path,
    mut config: AppConfig,
    capturer: &dyn ScreenCapturer,
    clipboard: &dyn ClipboardSink,
    uploaders: &UploaderRegistry,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
    match command {
        Command::Gui => Ok(String::new()),
        Command::Monitors => {
            let monitors = capturer.list_monitors()?;
            if monitors.is_empty() {
                return Ok("No monitors detected.".into());
            }
            let mut out = String::new();
            for m in monitors {
                let marker = if m.is_primary { " [primary]" } else { "" };
                let _ = writeln!(
                    out,
                    "id={} name=\"{}\" geometry={}x{}{:+}{:+}{}",
                    m.id, m.name, m.width, m.height, m.x, m.y, marker
                );
            }
            Ok(out.trim_end().to_owned())
        }
        Command::Capture(args) => {
            let source = if args.active_window {
                CaptureSource::ActiveWindow
            } else if args.active_monitor {
                CaptureSource::ActiveMonitor
            } else if let Some(id) = args.monitor {
                CaptureSource::Monitor {
                    id: MonitorId::new(id),
                }
            } else if let Some(spec) = args.region.as_deref() {
                CaptureSource::Region {
                    region: Region::parse(spec)?,
                }
            } else {
                CaptureSource::PrimaryMonitor
            };

            if let Some(dir) = args.output {
                config.output_dir = dir;
            }
            if let Some(fmt) = args.format {
                config.image_format = fmt;
            }
            if let Some(pattern) = args.pattern {
                config.file_name_pattern = pattern;
            }
            if args.no_save {
                config.pipeline.save_to_file = false;
            }
            if args.copy_image {
                config.pipeline.copy_image_to_clipboard = true;
            }
            if args.copy_path {
                config.pipeline.copy_path_to_clipboard = true;
            }
            if args.no_clipboard {
                config.pipeline.copy_image_to_clipboard = false;
                config.pipeline.copy_path_to_clipboard = false;
            }
            if let Some(uploader_id) = args.uploader {
                config.pipeline.uploader_id = Some(uploader_id);
            }
            if args.no_history {
                config.pipeline.record_history = false;
            }

            let noop = NoopClipboard;
            let active_clipboard: &dyn ClipboardSink =
                if args.no_clipboard { &noop } else { clipboard };
            let (history, _) = crate::AppHistoryStore::open(&config.history_path);

            let deps = PipelineDeps {
                capturer,
                clipboard: active_clipboard,
                uploaders,
                history: &history,
            };
            let report = CapturePipeline::execute(source, &config, &deps, now)?;

            let mut out = format!(
                "Captured {} ({}x{}, {})",
                report.capture_id,
                report.image.width(),
                report.image.height(),
                report.format
            );
            if let Some(path) = report.saved_path {
                let _ = write!(out, "\nSaved: {}", path.display());
            }
            if let Some(url) = report.upload_url {
                let _ = write!(out, "\nUploaded: {url}");
            }
            if report.copied_image {
                out.push_str("\nClipboard: image copied");
            }
            if let Some(text) = report.copied_text {
                let _ = write!(out, "\nClipboard text: {text}");
            }
            Ok(out)
        }
        Command::History { action } => {
            let (store, _) = crate::AppHistoryStore::open(&config.history_path);
            match action {
                HistoryCommand::List { limit } => {
                    let entries = store.list(Some(limit))?;
                    if entries.is_empty() {
                        return Ok("No captures recorded yet.".into());
                    }
                    let mut out = String::new();
                    for entry in entries {
                        let target = entry
                            .upload_url
                            .as_deref()
                            .map(str::to_owned)
                            .or_else(|| entry.file_path.map(|p| p.display().to_string()))
                            .unwrap_or_else(|| "(unsaved)".into());
                        let _ = writeln!(
                            out,
                            "{} | {} | {}x{} {} | {}",
                            entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
                            entry.id,
                            entry.width,
                            entry.height,
                            entry.format,
                            target
                        );
                    }
                    Ok(out.trim_end().to_owned())
                }
                HistoryCommand::Clear => {
                    store.clear()?;
                    Ok("Capture history cleared.".into())
                }
            }
        }
        Command::Config { action } => match action {
            ConfigCommand::Show => {
                let rendered = toml_preview(&config)?;
                Ok(rendered)
            }
            ConfigCommand::Path => Ok(config_path.display().to_string()),
        },
        Command::Hotkeys => {
            let mut out = String::new();
            for binding in &config.hotkeys {
                let key = binding
                    .combo
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "Unbound".into());
                let state = if binding.enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                let _ = writeln!(out, "{} | {} | {}", binding.action.label(), key, state);
            }
            Ok(out.trim_end().to_owned())
        }
        Command::Uploaders => {
            let entries = uploaders.list_metadata();
            if entries.is_empty() {
                return Ok("No uploaders registered.".into());
            }
            let mut out = String::new();
            for item in entries {
                let _ = writeln!(out, "{} ({}): {}", item.id, item.name, item.description);
            }
            Ok(out.trim_end().to_owned())
        }
    }
}

fn toml_preview(config: &AppConfig) -> Result<String, CoreError> {
    let mut out = String::new();
    let _ = writeln!(out, "output_dir = \"{}\"", config.output_dir.display());
    let _ = writeln!(out, "file_name_pattern = \"{}\"", config.file_name_pattern);
    let _ = writeln!(out, "image_format = \"{}\"", config.image_format);
    let _ = writeln!(out, "history_path = \"{}\"", config.history_path.display());
    let _ = writeln!(out, "save_to_file = {}", config.pipeline.save_to_file);
    let _ = writeln!(
        out,
        "copy_image_to_clipboard = {}",
        config.pipeline.copy_image_to_clipboard
    );
    let _ = writeln!(
        out,
        "copy_path_to_clipboard = {}",
        config.pipeline.copy_path_to_clipboard
    );
    let _ = writeln!(out, "record_history = {}", config.pipeline.record_history);
    if let Some(uploader_id) = &config.pipeline.uploader_id {
        let _ = writeln!(out, "uploader_id = \"{uploader_id}\"");
    }
    Ok(out.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use sharex_core::{CapturedImage, MonitorInfo};
    use tempfile::tempdir;

    struct StubCapturer;

    impl ScreenCapturer for StubCapturer {
        fn list_monitors(&self) -> Result<Vec<MonitorInfo>, CoreError> {
            Ok(vec![MonitorInfo {
                id: MonitorId::new(7),
                name: "Display 1".into(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                is_primary: true,
            }])
        }

        fn capture(&self, _source: &CaptureSource) -> Result<CapturedImage, CoreError> {
            CapturedImage::from_rgba(4, 3, [20, 40, 60, 255].repeat(12))
        }
    }

    #[test]
    fn cli_capture_saves_uploads_and_appears_in_history_list() {
        let dir = tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        let mut config = AppConfig::default_for_os();
        config.output_dir = dir.path().join("shots");
        config.history_path = dir.path().join("history.jsonl");
        config.file_name_pattern = "CliTest_%y%mo%d".into();

        let uploaders = UploaderRegistry::with_local_mirror(dir.path().join("mirror"));
        let capturer = StubCapturer;
        let clipboard = NoopClipboard;
        let now = Utc
            .with_ymd_and_hms(2026, 11, 24, 12, 30, 0)
            .single()
            .expect("valid ts");

        let parsed = Cli::try_parse_from([
            "sharex",
            "capture",
            "--format",
            "png",
            "--no-clipboard",
            "--uploader",
            "local-mirror",
        ])
        .expect("parse capture cli");

        let capture_out = execute_headless(
            parsed.command.expect("capture subcommand"),
            &config_path,
            config.clone(),
            &capturer,
            &clipboard,
            &uploaders,
            now,
        )
        .expect("capture command succeeds");

        assert!(capture_out.contains("4x3, png"));
        assert!(capture_out.contains("CliTest_20261124.png"));
        assert!(capture_out.contains("Uploaded: file:///"));

        let list_out = execute_headless(
            Command::History {
                action: HistoryCommand::List { limit: 10 },
            },
            &config_path,
            config,
            &capturer,
            &clipboard,
            &uploaders,
            now,
        )
        .expect("history list succeeds");

        assert!(list_out.contains("4x3 png"));
        assert!(list_out.contains("file:///"));
    }

    #[test]
    fn cli_monitors_prints_primary_display_metadata() {
        let dir = tempdir().expect("tempdir");
        let config = AppConfig::default_for_os();
        let uploaders = UploaderRegistry::new();
        let out = execute_headless(
            Command::Monitors,
            &dir.path().join("config.toml"),
            config,
            &StubCapturer,
            &NoopClipboard,
            &uploaders,
            Utc::now(),
        )
        .expect("monitors command succeeds");

        assert!(out.contains("id=7"));
        assert!(out.contains("1920x1080"));
        assert!(out.contains("[primary]"));
    }

    #[test]
    fn cli_parses_portable_and_active_monitor_flags() {
        let parsed = Cli::try_parse_from(["sharex", "--portable", "capture", "--active-monitor"])
            .expect("parse portable active-monitor cli");
        assert!(parsed.portable);
        match parsed.command {
            Some(Command::Capture(args)) => {
                assert!(args.active_monitor);
                assert!(!args.active_window);
                assert!(args.monitor.is_none());
                assert!(args.region.is_none());
            }
            other => panic!("expected Capture command, got {other:?}"),
        }

        assert!(
            Cli::try_parse_from(["sharex", "capture", "--active-monitor", "--active-window"])
                .is_err()
        );
    }

    #[test]
    fn cli_active_monitor_executes_with_active_monitor_source() {
        use std::sync::Mutex;

        struct RecordingCapturer {
            seen: Mutex<Vec<CaptureSource>>,
        }

        impl ScreenCapturer for RecordingCapturer {
            fn list_monitors(&self) -> Result<Vec<MonitorInfo>, CoreError> {
                Ok(Vec::new())
            }

            fn capture(&self, source: &CaptureSource) -> Result<CapturedImage, CoreError> {
                self.seen.lock().expect("lock").push(*source);
                CapturedImage::from_rgba(2, 2, [10, 20, 30, 255].repeat(4))
            }
        }

        let dir = tempdir().expect("tempdir");
        let mut config = AppConfig::portable_for_dir(dir.path());
        config.pipeline.save_to_file = false;
        config.pipeline.copy_image_to_clipboard = false;
        config.pipeline.record_history = false;

        let capturer = RecordingCapturer {
            seen: Mutex::new(Vec::new()),
        };
        let uploaders = UploaderRegistry::new();
        let parsed = Cli::try_parse_from(["sharex", "capture", "--active-monitor", "--no-save"])
            .expect("parse active monitor");

        let out = execute_headless(
            parsed.command.expect("capture"),
            &AppConfig::portable_config_path(dir.path()),
            config,
            &capturer,
            &NoopClipboard,
            &uploaders,
            Utc::now(),
        )
        .expect("active monitor capture succeeds");

        assert!(out.contains("2x2"));
        assert_eq!(
            capturer.seen.lock().expect("lock").as_slice(),
            &[CaptureSource::ActiveMonitor]
        );
    }

    #[test]
    fn cli_history_clear_wipes_sqlite_and_prevents_jsonl_resurrection() {
        let dir = tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        let mut config = AppConfig::portable_for_dir(dir.path());
        config.pipeline.save_to_file = false;
        config.pipeline.copy_image_to_clipboard = false;
        config.pipeline.record_history = true;

        let uploaders = UploaderRegistry::new();
        let now = Utc::now();
        execute_headless(
            Command::Capture(CaptureArgs {
                monitor: None,
                active_monitor: false,
                active_window: false,
                region: None,
                format: None,
                output: None,
                pattern: None,
                uploader: None,
                no_save: true,
                copy_image: false,
                copy_path: false,
                no_clipboard: true,
                no_history: false,
            }),
            &config_path,
            config.clone(),
            &StubCapturer,
            &NoopClipboard,
            &uploaders,
            now,
        )
        .expect("capture records to shared sqlite");

        let (reopened, _) = crate::AppHistoryStore::open(&config.history_path);
        assert_eq!(reopened.list(None).expect("list").len(), 1);

        let cleared = execute_headless(
            Command::History {
                action: HistoryCommand::Clear,
            },
            &config_path,
            config.clone(),
            &StubCapturer,
            &NoopClipboard,
            &uploaders,
            now,
        )
        .expect("history clear succeeds");
        assert!(cleared.contains("cleared"));

        let (after_clear, _) = crate::AppHistoryStore::open(&config.history_path);
        assert!(after_clear.list(None).expect("list after clear").is_empty());
    }
}
