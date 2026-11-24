# ShareX rewrite roadmap

This roadmap tracks feature parity between the original C# [ShareX](https://github.com/ShareX/ShareX) solution and the Rust workspace. Pick any item marked Open and open an issue or pull request.

## Subsystem parity

| Subsystem | Original C# project | Rust crate | Status |
| --- | --- | --- | --- |
| Core types, naming, config | `ShareX.HelpersLib` | `sharex-core` | Foundation complete |
| Capture history | `ShareX.HistoryLib` | `sharex-core` | JSONL complete, SQLite optional |
| Screen capture | `ShareX.ScreenCaptureLib` | `sharex-capture` | Monitor and coordinate region complete |
| Uploader destinations | `ShareX.UploadersLib` | `sharex-uploaders` | Trait, registry, and local mirror complete |
| Desktop UI and CLI | `ShareX` | `sharex-app` | Basic window and CLI complete |
| Image effects pipeline | `ShareX.ImageEffectsLib` | Planned (`sharex-effects`) | Open |
| Annotation editor | `ShareX.ImageEditor` | Planned (`sharex-editor`) | Open |
| Screen and GIF recording | `ShareX.MediaLib` | Planned (`sharex-media`) | Open |
| Extra utilities (OCR, color picker, QR) | `ShareX.Tools` | Planned (`sharex-tools`) | Open |

## Priority milestones for contributors

### Milestone 1. Interactive desktop workflow

- Global hotkeys (`PrintScreen`, `Ctrl+PrintScreen`, `Alt+PrintScreen`) via a cross-platform hotkey crate.
- System tray icon and background menu.
- Interactive fullscreen overlay for drag-to-select region capture.
- Active-window capture on Windows, macOS, and Linux.

### Milestone 2. Uploaders and sharing

- Custom HTTP multipart uploader compatible with `.sxcu` destination definitions.
- Built-in uploaders for S3-compatible buckets, Imgur, GitHub Gist, and Pastebin.
- Upload retry queue and transfer progress reporting.

### Milestone 3. Editing and effects

- Non-destructive image effects pipeline (border, shadow, resize, watermark, blur).
- Interactive annotation canvas (arrows, rectangles, step numbers, pixelate, text).

### Milestone 4. Recording and tools

- Screen recording to MP4, WebM, and GIF via FFmpeg bindings or CLI wrapper.
- Color picker, screen ruler, QR code generator, and OCR text capture.
- Optional SQLite implementation of `HistoryRepository` with search and tag filtering.
