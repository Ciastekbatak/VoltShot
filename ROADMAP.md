# ShareX rewrite roadmap

This roadmap tracks feature parity between the original C# [ShareX](https://github.com/ShareX/ShareX) solution and the 8-crate Rust workspace.

## Subsystem parity

| Subsystem | Original C# project | Rust crate | Status |
| --- | --- | --- | --- |
| Core types, naming, config, hotkeys, workflows | `ShareX.HelpersLib` | `sharex-core` | Complete (config, portable mode, 7 default hotkeys, `WorkflowRegistry`, `WatchFolderScanner`) |
| Capture history | `ShareX.HistoryLib` | `sharex-core` | Complete (bundled `SqliteHistoryStore`, tags, search queries, JSONL auto-migration) |
| Screen capture and region overlay | `ShareX.ScreenCaptureLib` | `sharex-capture`, `sharex-app` | Complete (fullscreen, active monitor, active window, multi-monitor crop, 11x11 pixel loupe, hover window snapping) |
| Annotation editor | `ShareX.ImageEditor` | `sharex-editor`, `sharex-app` | Complete (16 tools incl. Balloon/Magnifier/Spotlight, resize, `.sxed` JSON, undo/redo, PNG/JPEG/WebP save) |
| Image effects pipeline | `ShareX.ImageEffectsLib` | `sharex-effects`, `sharex-app` | Complete (9 pure-Rust filters, TOML/JSON presets, live after-capture execution, 1080p benchmark) |
| Uploader destinations | `ShareX.UploadersLib` | `sharex-uploaders`, `sharex-app` | Complete (`.sxcu` engine, template evaluator, Imgur/Discord/Gist/Pastebin presets, Local Mirror, retry queue, Uploaders UI) |
| Screen and GIF recording | `ShareX.ScreenRecordingLib` | `sharex-media`, `sharex-app` | Complete (FFmpeg builder for MP4/WebM/2-pass GIF, Recording panel, `Ctrl+Shift+PrtSc` / `Alt+Shift+PrtSc` hotkeys) |
| Extra utilities (Color, Ruler, QR, Hash, Indexer, Combiner, OCR) | `ShareX.Tools`, `ShareX.IndexerLib` | `sharex-tools`, `sharex-app` | Complete (5-tab Tools UI, Pin-to-Screen floating windows, `OcrEngine` + Tesseract CLI) |
| Desktop UI, Tray, i18n, Release CI | `ShareX` | `sharex-app`, `.github/workflows/release.yml` | Complete (dark/light UI, `tray-icon`, EN/PL localization, LRU cache, `--portable`, multi-OS release workflow) |

## Remaining follow-up milestones for contributors

- **Native Windows WinRT OCR adapter**: Implement `OcrEngine` against `Windows.Media.Ocr` as primary backend ahead of `TesseractCliOcr` in `FallbackOcrChain`.
- **S3 / R2 / MinIO SigV4 uploader**: Add AWS Signature Version 4 signing inside `sharex-uploaders`.
- **Editor TrueType font picker and sticker overlays**: Add `ab_glyph` system font rendering and `ImageOverlay` stamp annotations to `sharex-editor`.
- **Scrolling capture and screen-recording region HUD**: Add auto-scroll stitching and a translucent live recording border overlay.
- **Workflow & Watch Folder UI editor**: Expose `WorkflowRegistry` and `WatchFolderScanner` configuration screens inside `sharex-app`.
- **Windows `.ico` executable resource**: Embed a multi-size `.ico` in the Windows binary via `build.rs` / `winresource`.
