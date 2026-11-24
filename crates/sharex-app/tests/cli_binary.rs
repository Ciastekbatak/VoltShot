use std::process::Command;
use tempfile::tempdir;

#[test]
fn compiled_binary_serves_help_uploaders_config_and_history_commands() {
    let bin = env!("CARGO_BIN_EXE_sharex");
    let dir = tempdir().expect("tempdir");
    let cfg_path = dir.path().join("config.toml");

    let help = Command::new(bin)
        .arg("--help")
        .output()
        .expect("run sharex --help");
    assert!(help.status.success());
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(help_text.contains("capture"));
    assert!(help_text.contains("monitors"));
    assert!(help_text.contains("history"));
    assert!(help_text.contains("gui"));

    let uploaders = Command::new(bin)
        .args(["--config", cfg_path.to_str().expect("utf8"), "uploaders"])
        .output()
        .expect("run sharex uploaders");
    assert!(uploaders.status.success());
    assert!(String::from_utf8_lossy(&uploaders.stdout).contains("local-mirror"));

    let cfg_out = Command::new(bin)
        .args([
            "--config",
            cfg_path.to_str().expect("utf8"),
            "config",
            "path",
        ])
        .output()
        .expect("run sharex config path");
    assert!(cfg_out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&cfg_out.stdout).trim(),
        cfg_path.display().to_string()
    );

    let history_out = Command::new(bin)
        .args([
            "--config",
            cfg_path.to_str().expect("utf8"),
            "history",
            "list",
        ])
        .output()
        .expect("run sharex history list");
    assert!(history_out.status.success());
    assert!(String::from_utf8_lossy(&history_out.stdout).contains("No captures recorded yet."));
}
