<img src="assets/icon.svg" width="64" height="64" alt="ShareX in Rust logo">

# ShareX in Rust

An open-source Rust rewrite of [ShareX](https://github.com/ShareX/ShareX) structured as an 8-crate Cargo workspace with a ShareX-style dark desktop interface, native Windows system tray, global screenshot and recording hotkeys, smart region capture with pixel loupe, built-in screenshot editor, automated image effects, `.sxcu` uploaders, FFmpeg screen recording, and developer utilities.

## What works today

- **ShareX desktop UI (`eframe` and `egui`) with EN/PL localization**: Dark and light themes, left sidebar rail, native system tray (`tray-icon`) with 6 quick actions, bounded O(1) LRU thumbnail cache, and live English (`EN`) / Polish (`PL`) UI switching.
- **Global Win32 hotkeys (`hotkeys.rs` & `win_focus.rs`)**: Dedicated Win32 `RegisterHotKey` message thread with automatic conflict retry and game-safe cursor clip/foreground handoff (`PrintScreen` fullscreen, `Ctrl + PrintScreen` region, `Alt + PrintScreen` active window, `Shift + PrintScreen` last region, `Ctrl + Shift + E` image editor, `Ctrl + Shift + PrintScreen` MP4 recording, `Alt + Shift + PrintScreen` GIF recording).
- **Smart region capture overlay**: Freezes the display, provides an 11x11 zoomed pixel magnifier loupe with live `HEX`/`RGB` readout (`C` copies hex), smart hover window-bounds snapping, and inline annotation tools before finalizing capture.
- **Built-in screenshot editor (`sharex-editor`)**: Select and move, rectangle, filled rectangle, ellipse, arrow, line, freehand pen, highlighter, text, step numbers, speech balloon, circular magnifier, spotlight, pixelate, blur, crop, undoable resize, `.sxed` JSON project export/import, undo/redo (`Ctrl+Z` / `Ctrl+Y`), copy (`Ctrl+C`), and save (`Ctrl+S`).
- **Automated image effects (`sharex-effects`)**: Pure Rust filter pipeline (`Border`, `DropShadow`, `RoundedCorners`, `Resize`, `CanvasPadding`, `WatermarkText`, `Grayscale`, `Invert`, `BrightnessContrast`, `AutoCrop`) with TOML/JSON presets applied automatically on capture.
- **SQLite capture history and tags (`sharex-core`)**: Bundled SQLite store (`SqliteHistoryStore`) with automatic migration from `history.jsonl`, full text and `#tag` search, inline tag management, per-hotkey workflow overrides, and watch folder scanning.
- **Custom `.sxcu` and built-in uploaders (`sharex-uploaders`)**: Full `.sxcu` parser, `$json:...$`/`$regex:...$`/`$header:...$`/`$response$` response template evaluator, `ureq`/`rustls` HTTP engine, built-in presets for Imgur, Discord Webhook, GitHub Gist, Pastebin, Local Mirror, and persistent exponential-backoff `UploadQueue`.
- **FFmpeg screen and GIF recording (`sharex-media`)**: MP4 (H.264), WebM (VP9), and 2-pass palette GIF recording with live elapsed timer, Stop/Abort controls, and global recording hotkeys.
- **Tools suite and Pin-to-Screen (`sharex-tools`, `pin_window.rs`)**: QR code generator, color converter (`HEX`/`RGB`/`HSL`/`CMYK`), pixel ruler, file hash checker (`MD5`/`SHA-1`/`SHA-256`/`SHA-512`), directory indexer (`HTML`/`Markdown`/`JSON`/`TXT`), image combiner, `OcrEngine` + Tesseract CLI adapter, and borderless always-on-top Pin-to-Screen viewports.
- **Portable mode and CLI**: `--portable` keeps config, screenshots, and history next to the executable. Scriptable CLI supports `monitors`, `capture` (including `--active-monitor` and `--region`), `hotkeys`, `uploaders`, `history`, `config`, and `gui`.

## Workspace layout

Each crate maps to a subsystem in the original C# ShareX solution so contributors can own one area without coupling UI, OS, or network details into core logic.

| Rust crate | Original C# project | Responsibility |
| --- | --- | --- |
| `crates/sharex-core` | `ShareX.HelpersLib`, `ShareX.HistoryLib` | Domain types, hotkeys, filename patterns, TOML config + portable mode, SQLite/JSONL history, workflows, watch folder scanner, and capture pipeline |
| `crates/sharex-capture` | `ShareX.ScreenCaptureLib` | Monitor/window enumeration, cursor-based active monitor capture, window hit-testing, and multi-monitor region crop |
| `crates/sharex-editor` | `ShareX.ImageEditor` | Non-destructive annotation document, Pro tools (Balloon, Magnifier, Spotlight), undo/redo, `.sxed` JSON serialization, and RGBA software rasterizer |
| `crates/sharex-effects` | `ShareX.ImageEffectsLib` | Composable pure-Rust image filter pipeline, built-in presets, TOML/JSON serialization, and 1080p benchmark example |
| `crates/sharex-uploaders` | `ShareX.UploadersLib` | `.sxcu` parser, response template engine, `ureq`/`rustls` HTTP uploader, Imgur/Discord/Gist/Pastebin presets, Local Mirror, and retry queue |
| `crates/sharex-media` | `ShareX.ScreenRecordingLib` | FFmpeg discovery, command builder (`gdigrab`/`x11grab`/`avfoundation`), MP4/WebM/GIF recording state machine |
| `crates/sharex-tools` | `ShareX.Tools`, `ShareX.IndexerLib` | Color converter, ruler, file hasher, directory indexer, image combiner, QR generator, and `OcrEngine` + Tesseract CLI fallback chain |
| `crates/sharex-app` | `ShareX` | `sharex` desktop binary with dark/light `egui` UI, system tray, EN/PL i18n, Pin-to-Screen, Win32 global hotkeys, region overlay, and `clap` CLI |

## Quick start

Install Rust 1.85 or newer with `rustup`. On Linux, install the system packages listed in [CONTRIBUTING.md](CONTRIBUTING.md).

Build and test the workspace:

```bash
cargo test --workspace
```

Launch the ShareX desktop app (or portable mode):

```bash
cargo run -p sharex-app
cargo run -p sharex-app -- --portable
```

Build an optimized release executable:

```bash
cargo build --release -p sharex-app
```

Run headless CLI commands and the 1080p effects benchmark:

```bash
cargo run -p sharex-app -- monitors
cargo run -p sharex-app -- hotkeys
cargo run -p sharex-app -- uploaders
cargo run -p sharex-app -- capture --active-monitor --output ./captures
cargo run -p sharex-app -- capture --region 0,0,640,480 --format webp --output ./captures
cargo run -p sharex-app -- history list --limit 10
cargo run --release -p sharex-effects --example bench_pipeline
```

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for local build checks and crate boundary rules. Read [ROADMAP.md](ROADMAP.md) and [PLAN_20H.md](PLAN_20H.md) for architecture status and open follow-ups.

## License

This project follows upstream ShareX under `GPL-3.0-or-later`.
