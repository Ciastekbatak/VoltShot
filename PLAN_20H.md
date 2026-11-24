# ShareX in Rust — 20-Hour Autonomous Rewrite Plan (PStack)

Start baseline: commit `a269d98` (5 crates: `sharex-core`, `sharex-capture`, `sharex-editor`, `sharex-uploaders`, `sharex-app`; 25 passing tests; 0 clippy warnings).

## Exit predicate

The 20-hour rewrite succeeds when all 8 waves below are merged into `master`, `cargo fmt --all -- --check` exits 0, `cargo clippy --workspace --all-targets -- -D warnings` exits 0, `cargo test --workspace` passes 100%, `cargo build --release -p sharex-app` produces a verified standalone `sharex.exe`, every wave is logged in `decisions.tsv`, and a cross-vendor audit completes with zero unresolved blockers.

## Dependency graph and wave schedule

```text
Wave 1 (0-2.5h):  W1-A (SQLite history) || W1-B (Multi-mon/Windows capture) || W1-C (sharex-effects)
                         \                        |                         /
Wave 2 (2.5-5.5h): W2-A (System tray)   || W2-B (Editor Pro: loupe/balloon/TTF) || W2-C (Region loupe + window snap)
                         \                        |                         /
Wave 3 (5.5-8.5h): W3-A (.sxcu HTTP engine)       || W3-B (Imgur, Discord, Gist, S3, retry queue)
                               \                  /
Wave 4 (8.5-11.5h): W4-A (sharex-media FFmpeg)    || W4-B (Recording HUD + hotkeys)
                               \                  /
Wave 5 (11.5-14.5h): W5-A (Color picker + Ruler)  || W5-B (QR + Hash + Indexer) || W5-C (Combiner + Pin-to-Screen)
                               \                  |                  /
Wave 6 (14.5-17.0h): W6-A (WinRT + CLI OCR)       || W6-B (Per-hotkey workflows + Watch folders)
                               \                  /
Wave 7 (17.0-19.0h): W7-A (Full ShareX UI + PL/EN)|| W7-B (LRU cache, benchmarks, icon, release CI)
                               \                  /
Wave 8 (19.0-20.0h): Cross-model audit (Codex/Gemini) + Comment Sicko + full release gauntlet
```

## Wave breakdown

### Wave 1 (Hours 0.0 to 2.5) — Core storage, capture depth, and image effects

- [ ] **W1-A (`crates/sharex-core`)**: Add `SqliteHistoryStore` (`rusqlite` bundled behind a `sqlite` default feature) implementing `HistoryRepository`, plus `HistoryQuery` (text search, format filter, date range, tag filter), tag add/remove, and one-way JSONL-to-SQLite migration.
- [ ] **W1-B (`crates/sharex-capture`)**: Add `WindowInfo` enumeration (`list_windows`), cursor-based active monitor capture (`CaptureSource::ActiveMonitor` via Win32 `GetCursorPos` + `MonitorFromPoint`), `CaptureSource::Window { id }`, and `capture_virtual_desktop` fallback tests.
- [ ] **W1-C (`crates/sharex-effects`)**: New pure crate implementing composable `ImageEffect` pipeline (`Border`, `DropShadow`, `RoundedCorners`, `Resize`, `CanvasPadding`, `WatermarkText`, `Grayscale`, `Invert`, `BrightnessContrast`, `AutoCrop`), `EffectPreset` JSON/TOML serialization, and pixel-level unit tests.

### Wave 2 (Hours 2.5 to 5.5) — System tray, pro editor, and smart region overlay

- [ ] **W2-A (`crates/sharex-app` tray)**: Native system tray (`tray-icon`) with ShareX quick menu (Fullscreen, Region, Active Window, Open Main Window, Open Screenshots Folder, Quit) and close-to-tray preference.
- [ ] **W2-B (`crates/sharex-editor`)**: Add `SpeechBalloon`, `Magnifier` (2x/3x circular pixel zoom), `Spotlight` (dim outside region), `ImageOverlay`, TrueType font rendering via `ab_glyph`, corner resize handles on selected annotations, and `.sxed` JSON annotation serialization.
- [ ] **W2-C (`crates/sharex-app` region overlay)**: Multi-monitor spanning overlay, live ShareX pixel magnifier loupe with RGB/HEX readout under cursor, and smart hover window-bounds snapping.

### Wave 3 (Hours 5.5 to 8.5) — Custom `.sxcu` engine and built-in uploaders

- [ ] **W3-A (`crates/sharex-uploaders` `.sxcu`)**: Full `.sxcu` JSON schema parser, HTTP multipart/form-data/binary uploader (`reqwest` rustls), response syntax evaluator (`$json:...$`, `$regex:...$`, `$response$`, `$header:...$`), and local mock HTTP server integration tests.
- [ ] **W3-B (`crates/sharex-uploaders` destinations)**: Built-in `ImgurUploader`, `DiscordWebhookUploader`, `GitHubGistUploader`, `PastebinUploader`, `S3Uploader` (SigV4 for AWS S3 / R2 / MinIO), and persistent upload retry queue.

### Wave 4 (Hours 8.5 to 11.5) — Screen and GIF recording

- [ ] **W4-A (`crates/sharex-media`)**: New crate owning `RecordingConfig`, `RecordingState` state machine, FFmpeg command builder (`gdigrab`/x11grab/avfoundation), MP4/WebM/2-pass palette GIF encoding, audio source listing, and FFmpeg discovery.
- [ ] **W4-B (`crates/sharex-app` recording UI)**: `Ctrl + Shift + PrintScreen` (MP4) and `Alt + Shift + PrintScreen` (GIF) hotkeys, floating red recording frame with elapsed timer, Stop/Abort controls, and history recording.

### Wave 5 (Hours 11.5 to 14.5) — ShareX tools suite

- [ ] **W5-A (`crates/sharex-tools` color & ruler)**: Screen color picker (`HEX`, `RGB`, `HSL`, `CMYK`, swatch history) and pixel ruler (`dx`, `dy`, distance, angle, area).
- [ ] **W5-B (`crates/sharex-tools` utilities)**: QR code generator/decoder, file hash checker (`MD5`, `SHA-1`, `SHA-256`, `SHA-512`), and directory indexer (`HTML`, `Markdown`, `JSON`, `TXT`).
- [ ] **W5-C (`crates/sharex-tools` + `sharex-app` pin)**: Image combiner (vertical/horizontal stitch with gap/alignment), batch thumbnail sheet generator, and borderless always-on-top Pin-to-Screen floating windows.

### Wave 6 (Hours 14.5 to 17.0) — OCR, custom workflows, and watch folders

- [ ] **W6-A (`crates/sharex-tools` OCR)**: `OcrEngine` trait with Windows WinRT OCR (`Windows.Media.Ocr`) and Tesseract CLI fallback, wired to a **Capture Text (OCR)** action and hotkey.
- [ ] **W6-B (`crates/sharex-core` + `sharex-app` workflows)**: Per-hotkey `TaskSettings` overrides (custom capture mode, effects preset, filename pattern, uploader per hotkey) and watch folder auto-processor.

### Wave 7 (Hours 17.0 to 19.0) — Full UI parity, PL/EN localization, and packaging

- [ ] **W7-A (`crates/sharex-app` UI + i18n)**: Integrate Image Effects manager, Destinations/Uploader `.sxcu` UI, Tools launcher, History grid/list search and tags, and Polish (`pl`) / English (`en`) UI switcher.
- [ ] **W7-B (Performance & Release)**: Bounded thumbnail LRU cache, capture/rasterizer benchmarks, embedded Windows `.ico` icon, `--portable` mode, and GitHub Release binary workflow.

### Wave 8 (Hours 19.0 to 20.0) — Cross-model audit and final verification

- [ ] Cross-vendor review of `decisions.tsv` and cumulative diff (`codex` + `antigravity`).
- [ ] Full `no-comments` audit across all workspace crates.
- [ ] Full `fmt` + `clippy -D warnings` + `test` + `release` build + live CLI/hotkey verification.
