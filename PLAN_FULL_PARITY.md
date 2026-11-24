# ShareX in Rust - Phase 2 Full Parity & Optimization Plan (PStack)

Start baseline: commit `65d5776` (8 crates, 166 passing tests, 0 clippy warnings, wgpu DX12 renderer).

## Exit predicate

Phase 2 succeeds when Waves A through F are merged cleanly into `master`, `cargo fmt --all -- --check` exits 0, `cargo clippy --workspace --all-targets -- -D warnings` exits 0, `cargo test --workspace` passes 100%, `0` non-doc comments and `0` `#[allow(...)]` attributes exist across `crates/`, the 1080p `Clean Screenshot` benchmark improves substantially via `rayon`, `cargo build --release -p sharex-app` produces a console-free Windows `sharex.exe` with embedded `.ico`, and a cross-vendor audit (`codex/gpt-6.2-sol`) completes with zero unresolved blockers.

## Wave breakdown

### Wave A - Low-level performance and capture/media core (3 parallel agents)
- [ ] **A1 (`crates/sharex-capture`)**: Full virtual-desktop multi-monitor snapshot (`capture_virtual_desktop`), Windows DXGI Desktop Duplication path with `xcap` fallback, and pure-Rust vertical Scrolling Capture stitcher (`stitch_scrolling_frames`).
- [ ] **A2 (`crates/sharex-effects`)**: Parallelize image filters with `rayon` (`DropShadow`, `RoundedCorners`, `Resize`, `BrightnessContrast`, `Grayscale`, `Invert`) and add `WatermarkText`, `ImageWatermark`, `Colorize`, and `Slice`.
- [ ] **A3 (`crates/sharex-media`)**: Hardware encoder auto-detection (`h264_nvenc`, `h264_qsv`, `h264_amf` via `ffmpeg -encoders` with `libx264` fallback), audio input enumeration/recording (`dshow`/`pulse`/`avfoundation`), and region recording support.

### Wave B - Editor Pro 2.0, Cloud S3/URL Shorteners, and Native WinRT OCR (3 parallel agents)
- [ ] **B1 (`crates/sharex-editor`)**: TrueType font rendering via `ab_glyph` (embedded open sans-serif font), `ImageOverlay` (PNG stickers/stamps), `SmartEraser` (background color sampling), and corner `ResizeHandle` hit-testing + resize operations.
- [ ] **B2 (`crates/sharex-uploaders`)**: AWS S3 / Cloudflare R2 / MinIO SigV4 `S3Uploader`, URL shorteners (`is.gd`, `TinyURL`, custom `.sxcu`), and persistent destination/queue configuration storage.
- [ ] **B3 (`crates/sharex-tools`)**: Native Windows `WinRtOcr` (`Windows.Media.Ocr`) ahead of `TesseractCliOcr` in `FallbackOcrChain`, multi-row `ContactSheet` thumbnail grid generator, and QR code decoder (`decode_qr_image`).

### Wave C - Native Screen Overlays, Recording HUD, and Windows Shell Integration (3 parallel agents)
- [ ] **C1 (`crates/sharex-app/src/region_overlay.rs`)**: Multi-monitor virtual desktop spanning overlay with 4 modes: Region Capture, Capture Text (OCR), Select Recording Region, and Screen Color Picker overlay.
- [ ] **C2 (`crates/sharex-app/src/recording_hud.rs` + `recording_ui.rs`)**: Translucent always-on-top red recording frame around captured region + floating controls (elapsed timer, Stop/Abort, encoder selector, audio selector, Select Region button).
- [ ] **C3 (`crates/sharex-app/src/{notifications.rs, tray.rs, autostart.rs}`)**: ShareX bottom-right Toast notifications (thumbnail, click-to-open, right-click dismiss, auto-hide), Close-to-Tray / Start-minimized, Windows startup registry (`HKCU\...\Run`), and optional shutter sound.

### Wave D - Workflows Runtime, Effects Builder, Tools/S3 UI, and 100% PL/EN (3 parallel agents)
- [ ] **D1 (`crates/sharex-app/src/{workflows_ui.rs, watcher_runtime.rs}`)**: Wire `WorkflowRegistry` per-hotkey overrides and background `WatchFolderScanner` daemon + Workflows & Watch Folders UI tab.
- [ ] **D2 (`crates/sharex-app/src/{effects_ui.rs, editor_ui.rs}`)**: Interactive custom effect chain builder (add/remove/reorder filters, parameter sliders, TOML/JSON preset save/import) + Editor Pro UI (`ImageOverlay`, `SmartEraser`, resize handles, `.sxed` export/import).
- [ ] **D3 (`crates/sharex-app/src/{tools_ui.rs, uploaders_ui.rs, i18n.rs}`)**: Tools UI for Color Picker, Pixel Ruler, Image Combiner + Contact Sheet, QR Decoder, WinRT OCR, Scrolling Capture; Uploaders UI for S3/R2/MinIO, URL Shortener, Retry Queue; 100% EN/PL localization across all views.

### Wave E - Main Integration, Console-Free Windows Binary, and `.ico` Packaging (2 parallel agents)
- [ ] **E1 (`crates/sharex-app/src/{gui.rs, cli.rs, main.rs}`)**: Integrate all Wave C/D modules, new global hotkeys (`CaptureOcr`, `ColorPicker`, `ScrollingCapture`), and `windows_subsystem = "windows"` + `AttachConsole` for silent GUI launch with working CLI output.
- [ ] **E2 (`crates/sharex-app/build.rs`, `assets/sharex.ico`, `.github/workflows/release.yml`, docs)**: Multi-size Windows `.ico` resource embedding (`winresource`), updated 1080p benchmark docs, `README.md`, `ROADMAP.md`, and CI updates.

### Wave F - Cross-Model Audit and Final Release Verification
- [ ] Cross-vendor audit (`codex/gpt-6.2-sol`) over `decisions.tsv` and cumulative diff `65d5776..HEAD`.
- [ ] Remediate all blockers and notable risks.
- [ ] Full `fmt` + `clippy -D warnings` + `test` + `0` comments/`allow` audit + 1080p benchmark + release `sharex.exe` build & live launch.
