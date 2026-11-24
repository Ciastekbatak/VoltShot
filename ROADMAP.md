# ShareX rewrite roadmap

This roadmap tracks feature parity between the original C# [ShareX](https://github.com/ShareX/ShareX) solution and the Rust workspace. Pick any item marked Open and open an issue or pull request.

## Subsystem parity

| Subsystem | Original C# project | Rust crate | Status |
| --- | --- | --- | --- |
| Core types, naming, config, hotkeys | `ShareX.HelpersLib` | `sharex-core` | Foundation complete |
| Capture history | `ShareX.HistoryLib` | `sharex-core` | JSONL complete, SQLite optional |
| Screen capture and region overlay | `ShareX.ScreenCaptureLib` | `sharex-capture`, `sharex-app` | Fullscreen, monitor, active window, and drag-to-select region complete |
| Annotation editor | `ShareX.ImageEditor` | `sharex-editor`, `sharex-app` | 13 core tools, pixelate, blur, crop, undo/redo, and save complete |
| Desktop UI and global hotkeys | `ShareX` | `sharex-app` | ShareX dark UI, keybind manager, and CLI complete |
| Uploader destinations | `ShareX.UploadersLib` | `sharex-uploaders` | Trait, registry, and local mirror complete; HTTP/`.sxcu` open |
| Image effects pipeline | `ShareX.ImageEffectsLib` | Planned (`sharex-effects`) | Open |
| Screen and GIF recording | `ShareX.MediaLib` | Planned (`sharex-media`) | Open |
| Extra utilities (OCR, color picker, QR) | `ShareX.Tools` | Planned (`sharex-tools`) | Open |

## Priority milestones for contributors

### Milestone 1. System tray and multi-monitor polish

- Minimize to Windows/macOS/Linux system tray with quick capture context menu.
- Multi-monitor spanning region overlay and live pixel magnifier loupe.
- Scrolling capture and window-element snapping during region selection.

### Milestone 2. Uploaders and sharing

- Custom HTTP multipart uploader compatible with `.sxcu` destination definitions.
- Built-in uploaders for S3-compatible buckets, Imgur, GitHub Gist, and Pastebin.
- Upload retry queue and transfer progress reporting.

### Milestone 3. Image effects and advanced editor tools

- Non-destructive automated after-capture image effects (border, shadow, resize, watermark).
- Speech balloons, stickers, magnifier circles, and TrueType font selection in `sharex-editor`.

### Milestone 4. Recording and tools

- Screen recording to MP4, WebM, and GIF via FFmpeg bindings or CLI wrapper.
- Color picker, screen ruler, QR code generator, and OCR text capture.
- Optional SQLite implementation of `HistoryRepository` with search and tag filtering.
