<img src="assets/icon.svg" width="64" height="64" alt="ShareX in Rust logo">

# ShareX in Rust

An open-source Rust rewrite of [ShareX](https://github.com/ShareX/ShareX) with a modular Cargo workspace, cross-platform screen capture, after-capture workflows, a CLI, and a native desktop UI.

## What works today

- Fullscreen, single-monitor, and rectangular region screen capture through `xcap`.
- Configurable after-capture pipeline that saves PNG, JPEG, or WebP files, copies images or paths to the system clipboard, uploads through a plugin trait, and logs capture history.
- ShareX filename pattern expansion (`%y`, `%mo`, `%d`, `%h`, `%mi`, `%s`, `%ms`, `%pn`, `%width`, `%height`) with collision-safe file naming.
- Append-only JSON Lines capture history and TOML configuration.
- Native desktop window (`eframe` and `egui`) with live preview, capture controls, history browser, and settings editor.
- Scriptable CLI (`sharex monitors`, `sharex capture`, `sharex history`, `sharex config`, `sharex gui`).

## Workspace layout

Each crate maps to a subsystem in the original C# ShareX solution so contributors can own one area without stepping on others.

| Rust crate | Original C# project | Responsibility |
| --- | --- | --- |
| `crates/sharex-core` | `ShareX.HelpersLib`, `ShareX.HistoryLib` | Pure domain types, filename patterns, TOML config, JSONL history store, and after-capture pipeline |
| `crates/sharex-capture` | `ShareX.ScreenCaptureLib` | Cross-platform monitor enumeration and region screen capture via `xcap` |
| `crates/sharex-uploaders` | `ShareX.UploadersLib` | `UploaderRegistry` and concrete uploader implementations |
| `crates/sharex-app` | `ShareX` | `sharex` binary containing the `clap` CLI, `arboard` clipboard adapter, and `eframe` desktop GUI |

## Quick start

Install Rust 1.85 or newer with `rustup`. On Linux, install the system packages listed in [CONTRIBUTING.md](CONTRIBUTING.md).

Build and test the workspace:

```bash
cargo test --workspace
```

Launch the desktop app:

```bash
cargo run -p sharex-app
```

Run headless CLI commands:

```bash
cargo run -p sharex-app -- monitors
cargo run -p sharex-app -- capture --output ./captures
cargo run -p sharex-app -- capture --region 0,0,640,480 --format webp --output ./captures
cargo run -p sharex-app -- history list --limit 10
```

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for local build checks and how to add a new uploader or capture mode. Read [ROADMAP.md](ROADMAP.md) to pick an open milestone.

## License

This project follows upstream ShareX under `GPL-3.0-or-later`.
