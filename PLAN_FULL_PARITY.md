# ShareX in Rust - Phase 2 Full Parity & Optimization Plan (PStack)

Start baseline: commit `65d5776` (8 crates, 166 passing tests, 0 clippy warnings, wgpu DX12 renderer).

## Exit predicate

Phase 2 succeeds when Waves A through F are merged cleanly into `master`, `cargo fmt --all -- --check` exits 0, `cargo clippy --workspace --all-targets -- -D warnings` exits 0, `cargo test --workspace` passes 100%, `0` non-doc comments and `0` `#[allow(...)]` attributes exist across `crates/`, the 1080p `Clean Screenshot` benchmark improves substantially via `rayon`, `cargo build --release -p sharex-app` produces a console-free Windows `sharex.exe` with embedded `.ico`, and a cross-vendor audit (`codex/gpt-6.2-sol`) completes with zero unresolved blockers.

## Wave breakdown

### Wave A - Low-level performance and capture/media core (3 parallel agents)
- [x] **A1 (`crates/sharex-capture`)**: Full virtual-desktop multi-monitor snapshot (`capture_virtual_desktop`), Windows DXGI Desktop Duplication path with `xcap` fallback, and pure-Rust vertical Scrolling Capture stitcher (`stitch_scrolling_frames`) (`e6c6dc7`).
- [x] **A2 (`crates/sharex-effects`)**: Parallelized image filters with `rayon` + O(1) 3-pass box shadow blur (`235.04 ms -> 5.36 ms`, 43.9x faster on 1080p `Clean Screenshot`), plus `WatermarkText`, `ImageWatermark`, `Colorize`, and `Slice` (`ca38849`).
- [x] **A3 (`crates/sharex-media`)**: Hardware encoder auto-detection (`h264_nvenc`, `h264_qsv`, `h264_amf` via `ffmpeg -encoders` with `libx264` fallback), audio input enumeration/recording (`dshow`/`pulse`/`avfoundation`), and region recording support (`5b1f351`).

### Wave B - Editor Pro 2.0, Cloud S3/URL Shorteners, and Native WinRT OCR (3 parallel agents)
- [x] **B1 (`crates/sharex-editor`)**: Opt-in TrueType font rendering via `ab_glyph` with deterministic bitmap fallback, `ImageOverlay` (PNG stickers/stamps), `SmartEraser` (composited canvas color sampling), and corner `ResizeHandle` hit-testing + undoable resize (`4110ebe`).
- [x] **B2 (`crates/sharex-uploaders`)**: AWS S3 / Cloudflare R2 / MinIO SigV4 `S3Uploader`, URL shorteners (`is.gd`, `TinyURL`, custom `.sxcu`), and atomic persistent destination configuration store (`c0d9f07`).
- [x] **B3 (`crates/sharex-tools`)**: Native Windows `WinRtOcr` (`Windows.Media.Ocr`) ahead of `TesseractCliOcr` in `FallbackOcrChain`, multi-row `ContactSheet` thumbnail grid generator, and pure-Rust QR code decoder (`decode_qr_image`) (`93961fe`).

### Wave C - Native Screen Overlays, Recording HUD, and Windows Shell Integration (3 parallel agents)
- [x] **C1 (`crates/sharex-app/src/region_overlay.rs`)**: Multi-monitor virtual desktop spanning overlay with primary-monitor toolbar anchoring and 4 modes: Capture, OCR, SelectRecordingRegion, and ColorPicker (`995a2bc`).
- [x] **C2 (`crates/sharex-app/src/recording_hud.rs` + `recording_ui.rs`)**: Always-on-top floating Recording HUD (elapsed timer, Stop/Abort, format/encoder pill) + cached hardware encoder selector, audio input dropdown, and region selection (`808665b`).
- [x] **C3 (`crates/sharex-app/src/{notifications.rs, autostart.rs, sound.rs}`)**: ShareX bottom-right Toast notifications (thumbnail, click-to-open, right-click dismiss, auto-hide), Win32 `HKCU\...\Run` `AutostartManager`, and in-memory PCM shutter sound (`0e7007d`).

### Wave D - Workflows Runtime, Effects Builder, Tools/S3 UI, and 100% PL/EN (3 parallel agents)
- [x] **D1 (`crates/sharex-app/src/{workflows_ui.rs, watcher_runtime.rs}`)**: Background `WatchFolderRuntime` daemon and TOML-persisted Workflows & Watch Folders management panel (`646415a`).
- [x] **D2 (`crates/sharex-app/src/{effects_ui.rs, editor_ui.rs}`)**: Interactive 11-effect pipeline builder with live preview & TOML import/export + Editor Pro 2.0 UI (`SmartEraser`, PNG/sample stamps, corner resize handles with single-undo-per-drag, `.sxed` I/O, canvas resize) (`db2b161`).
- [x] **D3 (`crates/sharex-app/src/{tools_ui.rs, uploaders_ui.rs, i18n.rs}`)**: Tools UI for QR Decode, Pixel Ruler, Image Combiner + Contact Sheet, WinRT-first background OCR, Color Picker hook; Uploaders UI for S3/R2/MinIO, URL Shorteners, persistent settings, and Retry Queue; expanded EN/PL localization (`2051398`).

### Wave E - Main Integration, Console-Free Windows Binary, and `.ico` Packaging (2 parallel agents)
- [x] **E1 (`crates/sharex-app/src/{gui.rs, cli.rs, main.rs, hotkeys.rs}` + `sharex-core`)**: Integrated all Phase 2 modules, new global hotkeys (`CaptureOcr` `Ctrl+Shift+O`, `PickScreenColor` `Ctrl+Shift+C`), `--minimized`, Close-to-Tray, Autostart, Toasts, WatchFolderRuntime, and `windows_subsystem = "windows"` + `AttachConsole` (`33bc269`).
- [x] **E2 (`crates/sharex-app/build.rs`, `assets/sharex.ico`)**: Multi-resolution `16/32/48/256` Windows `.ico` resource embedding and PE metadata via `winresource` (`6a284e8`).

### Wave F - Cross-Model Audit and Final Release Verification
- [ ] Cross-vendor audit (`codex/gpt-6.2-sol`) over `decisions.tsv` and cumulative diff `65d5776..HEAD`.
- [ ] Remediate any blockers and notable risks.
- [ ] Full `fmt` + `clippy -D warnings` + `test` + `0` comments/`allow` audit + 1080p benchmark + release `sharex.exe` build & live launch.
