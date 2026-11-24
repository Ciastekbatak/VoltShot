pub mod cli;
pub mod clipboard;
pub mod editor_ui;
pub mod gui;
pub mod hotkeys;
pub mod region_overlay;
pub mod theme;
pub mod win_focus;

use chrono::Utc;
use clap::Parser;
use cli::{Cli, Command, execute_headless};
use clipboard::SystemClipboard;
use sharex_capture::XcapCapturer;
use sharex_core::{AppConfig, CoreError};
use sharex_uploaders::UploaderRegistry;
use std::path::PathBuf;

/// Entry point shared by the `sharex` binary.
pub fn run() -> Result<(), String> {
    let parsed = Cli::parse();
    let config_path = parsed
        .config
        .clone()
        .unwrap_or_else(AppConfig::default_config_path);

    let command = parsed.command.unwrap_or(Command::Gui);
    match command {
        Command::Gui => gui::launch_gui(config_path).map_err(|err| err.to_string()),
        other => {
            let config = AppConfig::load_or_default(&config_path).map_err(format_core_error)?;
            let mirror_dir = default_mirror_dir(&config);
            let capturer = XcapCapturer::new();
            let clipboard = SystemClipboard::new();
            let uploaders = UploaderRegistry::with_local_mirror(mirror_dir);
            let output = execute_headless(
                other,
                &config_path,
                config,
                &capturer,
                &clipboard,
                &uploaders,
                Utc::now(),
            )
            .map_err(format_core_error)?;
            if !output.is_empty() {
                println!("{output}");
            }
            Ok(())
        }
    }
}

pub(crate) fn default_mirror_dir(config: &AppConfig) -> PathBuf {
    config.output_dir.join("uploaded-mirror")
}

fn format_core_error(err: CoreError) -> String {
    err.to_string()
}
