<img src="assets/icon.svg" width="64" height="64" alt="ShareX in Rust logo">

# ShareX in Rust

An open-source Rust rewrite of [ShareX](https://github.com/ShareX/ShareX) with a ShareX-style dark desktop interface, global screenshot hotkeys, interactive drag-to-select region capture, and a built-in screenshot annotation editor.

## What works today

- **ShareX desktop UI (`eframe` and `egui`)**: Dark theme sidebar rail with quick capture buttons, after-capture checkboxes, visual capture history thumbnails, hotkey manager, and settings editor.
- **Global screenshot hotkeys (`global-hotkey`)**: System-wide ShareX default binds (`PrintScreen` fullscreen, `Ctrl + PrintScreen` interactive region, `Alt + PrintScreen` active window, `Shift + PrintScreen` last region, `Ctrl + Shift + E` image editor) with live rebinding and conflict status.
- **Interactive region capture overlay**: Freezes the display, dims outside the selection, shows live crosshair guides and pixel dimensions, and crops on mouse release.
- **Built-in screenshot editor (`sharex-editor`)**: Select and move, rectangle, filled rectangle, ellipse, arrow, line, freehand pen, highlighter, text, auto-incrementing step numbers, pixelate, blur, crop, undo and redo (`Ctrl+Z` / `Ctrl+Y`), copy to clipboard (`Ctrl+C`), and save to PNG, JPEG, or WebP (`Ctrl+S`).
- **ShareX filename patterns and history**: Expands `%y`, `%mo`, `%d`, `%h`, `%mi`, `%s`, `%ms`, `%pn`, `%width`, `%height` with collision-safe file naming and append-only JSON Lines history.
- **Scriptable CLI**: `sharex monitors`, `sharex capture`, `sharex hotkeys`, `sharex history`, `sharex config`, and `sharex gui`.

## Workspace layout

Each crate maps to a subsystem in the original C# ShareX solution so contributors can own one area without stepping on others.

| Rust crate | Original C# project | Responsibility |
| --- | --- | --- |
| `crates/sharex-core` | `ShareX.HelpersLib`, `ShareX.HistoryLib` | Pure domain types, hotkey models, filename patterns, TOML config, JSONL history store, and after-capture pipeline |
| `crates/sharex-capture` | `ShareX.ScreenCaptureLib` | Cross-platform monitor enumeration, active window capture, and region screen capture via `xcap` |
| `crates/sharex-editor` | `ShareX.ImageEditor` | Pure non-destructive annotation document, undo/redo stack, and RGBA software rasterizer |
| `crates/sharex-uploaders` | `ShareX.UploadersLib` | `UploaderRegistry` and concrete uploader implementations |
| `crates/sharex-app` | `ShareX` | `sharex` binary containing the ShareX dark GUI, global hotkey runtime, region overlay, editor canvas, clipboard adapter, and `clap` CLI |

## Quick start

Install Rust 1.85 or newer with `rustup`. On Linux, install the system packages listed in [CONTRIBUTING.md](CONTRIBUTING.md).

Build and test the workspace:

```bash
cargo test --workspace
```

Launch the ShareX desktop app:

```bash
cargo run -p sharex-app
```

Build an optimized release executable:

```bash
cargo build --release -p sharex-app
```

Run headless CLI commands:

```bash
cargo run -p sharex-app -- monitors
cargo run -p sharex-app -- hotkeys
cargo run -p sharex-app -- capture --output ./captures
cargo run -p sharex-app -- capture --region 0,0,640,480 --format webp --output ./captures
cargo run -p sharex-app -- history list --limit 10
```

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for local build checks and crate boundary rules. Read [ROADMAP.md](ROADMAP.md) to pick an open milestone.

## License

This project follows upstream ShareX under `GPL-3.0-or-later`.
