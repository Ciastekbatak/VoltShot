# ShareX rewrite roadmap

This roadmap tracks feature parity between the original C# [ShareX](https://github.com/ShareX/ShareX) solution and the 8-crate Rust workspace.

## Subsystem parity

| Subsystem | Original C# project | Rust crate | Status |
| --- | --- | --- | --- |
| Core types, naming, config, hotkeys, workflows | `ShareX.HelpersLib` | `sharex-core` | Complete (config, portable mode, 9 default hotkeys, `WorkflowRegistry`, `WatchFolderScanner`) |
| Capture history | `ShareX.HistoryLib` | `sharex-core` | Complete (shared CLI/GUI `SqliteHistoryStore`, tags, `#tag` search, one-time JSONL migration) |
| Screen capture & multi-monitor overlays | `ShareX.ScreenCaptureLib` | `sharex-capture`, `sharex-app` | Complete (DXGI Desktop Duplication + `xcap` fallback, multi-monitor virtual desktop, scrolling stitcher, 4 overlay modes: Capture/OCR/RecordingRegion/ColorPicker) |
| Annotation editor | `ShareX.ImageEditor` | `sharex-editor`, `sharex-app` | Complete (18 tools incl. SpeechBalloon, Magnifier, Spotlight, SmartEraser, ImageOverlay stamps, `ab_glyph` TTF, corner resize handles, `.sxed` JSON, canvas resize) |
| Image effects pipeline | `ShareX.ImageEffectsLib` | `sharex-effects`, `sharex-app` | Complete (13 rayon-parallel filters, O(1) shadow blur ~5ms @ 1080p, interactive 11-effect UI builder, TOML import/export) |
| Uploader destinations & URL shorteners | `ShareX.UploadersLib` | `sharex-uploaders`, `sharex-app` | Complete (`.sxcu`, AWS S3/R2/MinIO SigV4, Imgur/Discord/Gist/Pastebin/Local Mirror, `is.gd`/`TinyURL`/`.sxcu` shorteners, persistent settings, retry queue) |
| Screen & GIF recording | `ShareX.ScreenRecordingLib` | `sharex-media`, `sharex-app` | Complete (NVENC/QSV/AMF auto-detection, audio devices, region recording, 2-stage disk-spooled GIF, floating Recording HUD, global hotkeys) |
| Utilities & native OCR | `ShareX.Tools`, `ShareX.IndexerLib` | `sharex-tools`, `sharex-app` | Complete (native Windows WinRT OCR + Tesseract fallback, QR encode/decode, Color, Ruler, Hash, Indexer, Combiner + Contact Sheet, Pin-to-Screen) |
| Desktop shell, Tray, Toasts, i18n, Packaging | `ShareX` | `sharex-app`, `.github/workflows/release.yml` | Complete (`wgpu` DX12, console-free Windows GUI, embedded `.ico`, Tray, Close-to-Tray, Toasts, Autostart, shutter sound, Workflows/Watchers UI, EN/PL i18n, CI releases) |

## Future stretch enhancements

- Interactive Scrolling Capture auto-scroll driver (`SendInput` mouse wheel loop wired into UI).
- Native Linux Wayland (`xdg-desktop-portal` ScreenCast) and macOS ScreenCaptureKit backends.
- Signed Windows MSI / winget installer package alongside portable ZIP archives.
