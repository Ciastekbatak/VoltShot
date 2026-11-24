<img src="assets/icon.svg" width="64" height="64" alt="ShareX in Rust logo">

# ShareX in Rust

An open-source Rust rewrite of [ShareX](https://github.com/ShareX/ShareX) structured as an 8-crate Cargo workspace with a native **DirectX 12 (`wgpu`)** dark/light desktop interface, embedded Windows `.ico` without a background console window, native system tray, global screenshot/OCR/color-picker/recording hotkeys, multi-monitor region capture, Pro screenshot editor, rayon-accelerated image effects, `.sxcu` + AWS S3/R2/MinIO SigV4 uploaders, FFmpeg NVENC/QSV/AMF screen recording, native Windows WinRT OCR, workflows, and watch folders.

## Key capabilities

- **ShareX Desktop UI (`eframe` + `wgpu` DX12/Metal/Vulkan) with EN/PL localization**: Native DirectX 12 rendering on Windows (zero OpenGL flicker), dark and light themes, embedded multi-size Windows `.ico`, console-free release launch (`AttachConsole` preserved for CLI), ShareX bottom-right Toast notifications, optional shutter sound, Close-to-Tray, Start Minimized (`--minimized`), Windows startup (`HKCU\...\Run`), bounded LRU thumbnail cache (64 textures), and EN/PL UI switcher.
- **Global Win32 hotkeys (`hotkeys.rs` & `win_focus.rs`)**: Dedicated Win32 `RegisterHotKey` thread with automatic conflict retry and game-safe cursor clip/foreground handoff:
  - `PrintScreen` — Capture fullscreen
  - `Ctrl + PrintScreen` — Multi-monitor region capture overlay
  - `Alt + PrintScreen` — Capture active window
  - `Shift + PrintScreen` — Capture last region
  - `Ctrl + Shift + E` — Open image editor
  - `Ctrl + Shift + PrintScreen` — Toggle MP4 recording
  - `Alt + Shift + PrintScreen` — Toggle GIF recording
  - `Ctrl + Shift + O` — Capture text region (WinRT/Tesseract OCR -> clipboard)
  - `Ctrl + Shift + C` — Screen color picker overlay (HEX -> clipboard)
- **Multi-monitor capture & DXGI (`sharex-capture`)**: Virtual desktop stitching across negative-origin multi-monitor setups, Windows DXGI Desktop Duplication with automatic `xcap` fallback, window hit-testing/snapping, 11x11 pixel magnifier loupe, and pure-Rust vertical Scrolling Capture stitching.
- **Pro Screenshot Editor (`sharex-editor`)**: 18 tools including `SpeechBalloon`, `Magnifier` (composited canvas sampling, redaction-safe), `Spotlight`, `SmartEraser`, `ImageOverlay` (PNG stamps), opt-in `ab_glyph` TrueType text rendering, corner `ResizeHandle` dragging (single undo per gesture), undoable canvas resize, and `.sxed` JSON project export/import.
- **Rayon-accelerated Image Effects (`sharex-effects`)**: 13 pure-Rust filters (`Border`, `DropShadow` with O(1) 3-pass box blur, `RoundedCorners`, `Resize`, `CanvasPadding`, `Grayscale`, `Invert`, `BrightnessContrast`, `AutoCrop`, `WatermarkText`, `ImageWatermark`, `Colorize`, `Slice`), interactive 11-effect UI builder with TOML import/export, and ~5 ms 1080p `Clean Screenshot` execution.
- **Cloud Uploaders & URL Shorteners (`sharex-uploaders`)**: Full `.sxcu` engine (`ureq` + `rustls`), AWS S3 / Cloudflare R2 / Backblaze B2 / MinIO **SigV4** uploader, built-in Imgur / Discord Webhook / GitHub Gist / Pastebin / Local Mirror destinations, `is.gd` / `TinyURL` / `.sxcu` URL shorteners, persistent destination settings, and exponential-backoff `UploadQueue`.
- **Hardware-accelerated FFmpeg Recording (`sharex-media` & `recording_hud.rs`)**: MP4 (H.264 with auto-detected `NVENC` / `QSV` / `AMF` / `libx264`), WebM (VP9), and low-RAM two-stage disk-spooled palette GIF, optional audio device capture (`dshow` / `pulse` / `avfoundation`), region recording, and floating always-on-top Recording HUD.
- **Tools Suite & Native OCR (`sharex-tools`)**: Native Windows **WinRT OCR (`Windows.Media.Ocr`)** with automatic `TesseractCliOcr` fallback, QR code generator & decoder (`rqrr`), color converter (`HEX`/`RGB`/`HSL`/`CMYK`), pixel ruler, file hash checker (`MD5`/`SHA-1`/`SHA-256`/`SHA-512`), directory indexer (`HTML`/`Markdown`/`JSON`/`TXT`), image combiner + multi-row Contact Sheet grid, and Pin-to-Screen floating windows.
- **SQLite History, Workflows & Watch Folders (`sharex-core`)**: Shared CLI/GUI SQLite store with one-time JSONL migration, `#tag` search, per-hotkey `WorkflowRegistry` overrides, background `WatchFolderRuntime` daemon, and `--portable` mode.

## Workspace layout

| Rust crate | Original C# project | Responsibility |
| --- | --- | --- |
| `crates/sharex-core` | `ShareX.HelpersLib`, `ShareX.HistoryLib` | Domain types, 9 default hotkeys, filename patterns, TOML config + portable mode, SQLite/JSONL history, workflows, watch folder scanner, capture pipeline |
| `crates/sharex-capture` | `ShareX.ScreenCaptureLib` | Windows DXGI Desktop Duplication + `xcap` fallback, multi-monitor virtual desktop stitching, window hit-testing, scrolling capture stitcher |
| `crates/sharex-editor` | `ShareX.ImageEditor` | Non-destructive annotation document, 18 tools, `ab_glyph` TTF + bitmap fallback, corner resize handles, `.sxed` JSON, RGBA rasterizer |
| `crates/sharex-effects` | `ShareX.ImageEffectsLib` | Rayon-parallel 13-filter image pipeline, O(1) shadow blur, TOML/JSON presets, 1080p benchmark |
| `crates/sharex-uploaders` | `ShareX.UploadersLib` | `.sxcu` engine, AWS S3/R2/MinIO SigV4, Imgur/Discord/Gist/Pastebin/Local Mirror, URL shorteners, retry queue, persistent settings store |
| `crates/sharex-media` | `ShareX.ScreenRecordingLib` | FFmpeg discovery, NVENC/QSV/AMF detection, audio device enumeration, MP4/WebM/2-stage GIF recorder |
| `crates/sharex-tools` | `ShareX.Tools`, `ShareX.IndexerLib` | Windows WinRT OCR + Tesseract fallback, QR encode/decode, color converter, ruler, file hasher, directory indexer, image combiner + contact sheet |
| `crates/sharex-app` | `ShareX` | Console-free `sharex.exe` (`wgpu` DX12, `.ico`), tray, toasts, autostart, shutter sound, Recording HUD, multi-mode overlay, Workflows/Watchers UI, EN/PL i18n, CLI |

## Quick start

Install Rust 1.85 or newer with `rustup`. On Linux, install the system packages listed in [CONTRIBUTING.md](CONTRIBUTING.md).

```bash
cargo test --workspace
cargo build --release -p sharex-app
cargo run -p sharex-app -- --portable
cargo run --release -p sharex-effects --example bench_pipeline
```

## License

This project follows upstream ShareX under `GPL-3.0-or-later`.
