# ShareX in Rust - 20-Hour Autonomous Rewrite Plan (PStack)

Start baseline: commit `a269d98` (5 crates: `sharex-core`, `sharex-capture`, `sharex-editor`, `sharex-uploaders`, `sharex-app`; 25 passing tests; 0 clippy warnings).

## Exit predicate

The 20-hour rewrite succeeds when all 8 waves below are merged into `master`, `cargo fmt --all -- --check` exits 0, `cargo clippy --workspace --all-targets -- -D warnings` exits 0, `cargo test --workspace` passes 100%, `cargo build --release -p sharex-app` produces a verified standalone `sharex.exe`, every wave is logged in `decisions.tsv`, and a cross-vendor audit completes with zero unresolved blockers.

## Dependency graph and wave schedule

```text
Wave 1 (0-2.5h):  W1-A (SQLite history) || W1-B (Multi-mon/Windows capture) || W1-C (sharex-effects)
                         \                        |                         /
Wave 2 (2.5-5.5h): W2-A (System tray)   || W2-B (Editor Pro: balloon/mag/spot) || W2-C (Region loupe + window snap)
                         \                        |                         /
Wave 3 (5.5-8.5h): W3-A (.sxcu HTTP engine)       || W3-B (Imgur, Discord, Gist, Pastebin, retry queue)
                               \                  /
Wave 4 (8.5-11.5h): W4-A (sharex-media FFmpeg)    || W4-B (Recording UI + global hotkeys)
                               \                  /
Wave 5 (11.5-14.5h): W5-A (Color picker + Ruler)  || W5-B (QR + Hash + Indexer) || W5-C (Combiner + Pin-to-Screen)
                               \                  |                  /
Wave 6 (14.5-17.0h): W6-A (OcrEngine + Tesseract) || W6-B (Per-hotkey workflows + Watch folders)
                               \                  /
Wave 7 (17.0-19.0h): W7-A/C (Full ShareX UI + PL/EN + SQLite + Uploaders) || W7-B (LRU cache, benchmark, --portable, release CI)
                               \                  /
Wave 8 (19.0-20.0h): Cross-model audit (Codex) + Comment Sicko + full release gauntlet
```

## Wave breakdown

### Wave 1 (Hours 0.0 to 2.5) - Core storage, capture depth, and image effects

- [x] **W1-A (`crates/sharex-core`)**: Added `SqliteHistoryStore` (`rusqlite` bundled behind default `sqlite` feature) implementing `HistoryRepository`, `HistoryQuery` (text, format, date range, tag filters), `set_tags`, and one-time JSONL-to-SQLite migration (`6fd77c2`, unified CLI/GUI store in Wave 8 fix).
- [x] **W1-B (`crates/sharex-capture`)**: Added `WindowInfo` enumeration (`list_windows`, `find_window_at_point`), cursor-based active monitor capture (`CaptureSource::ActiveMonitor`, Win32 cursor lookup with primary fallback on Linux/macOS), `CaptureSource::Window { id }`, and multi-monitor region crop (`eab9b0a`).
- [x] **W1-C (`crates/sharex-effects`)**: Added pure Rust composable `ImageEffect` pipeline (9 filters: `Border`, `DropShadow`, `RoundedCorners`, `Resize`, `CanvasPadding`, `Grayscale`, `Invert`, `BrightnessContrast`, `AutoCrop`) and TOML/JSON `EffectPreset` serialization (`28617fd`). (Follow-up: `WatermarkText` remains open.)

### Wave 2 (Hours 2.5 to 5.5) - System tray, pro editor, and smart region overlay

- [x] **W2-A (`crates/sharex-app` tray)**: Added native system tray (`tray-icon 0.21.3`) with programmatic 32x32 RGBA ShareX icon and 6 quick actions (Fullscreen, Region, Active Window, Open Main Window, Open Screenshots Folder, Quit) (`515f9f8`).
- [x] **W2-B (`crates/sharex-editor`)**: Added `SpeechBalloon`, `Magnifier` (circular pixel zoom), `Spotlight` (dim outside region), undoable resize, and `.sxed` JSON annotation import/export (`c03c013`). (Follow-up: TrueType `ab_glyph` font picker and `ImageOverlay` sticker stamps remain open.)
- [x] **W2-C (`crates/sharex-app` region overlay)**: Added live 11x11 ShareX pixel magnifier loupe with HEX/RGB readout (`C` to copy hex) and smart hover window-bounds snapping (`c479d4d`).

### Wave 3 (Hours 5.5 to 8.5) - Custom `.sxcu` engine and built-in uploaders

- [x] **W3-A (`crates/sharex-uploaders` `.sxcu`)**: Added `.sxcu` JSON schema parser, HTTP multipart/form/JSON/binary uploader (`ureq 3.5` with `rustls` for smaller sync footprint than `reqwest`), response syntax evaluator (`$json:...$`, `$regex:...$`, `$response$`, `$header:...$`), and local loopback HTTP integration tests (`e7511e2`).
- [x] **W3-B (`crates/sharex-uploaders` destinations)**: Added built-in `.sxcu` presets for Imgur, Discord Webhook, GitHub Gist, and Pastebin plus FIFO exponential-backoff `UploadQueue` with JSON persistence (`b8e5aa1`). (Follow-up: AWS S3 / R2 SigV4 signing remains open.)

### Wave 4 (Hours 8.5 to 11.5) - Screen and GIF recording

- [x] **W4-A (`crates/sharex-media`)**: Added `RecordingConfig`, `RecordingState` state machine, FFmpeg command builder (`gdigrab`/`x11grab`/`avfoundation`), MP4/WebM/2-pass palette GIF encoding, and FFmpeg discovery (`3b1e652`).
- [x] **W4-B (`crates/sharex-app` recording UI)**: Wired `RecordingPanelState` with live timer + Stop/Abort controls (`c4c1516`) and `Ctrl + Shift + PrintScreen` (MP4) / `Alt + Shift + PrintScreen` (GIF) global hotkeys (`ef5d188`).

### Wave 5 (Hours 11.5 to 14.5) - ShareX tools suite

- [x] **W5-A (`crates/sharex-tools` color & ruler)**: Added color conversions (`HEX`, `RGB`, `HSL`, `CMYK`, decimal) and pixel ruler (`dx`, `dy`, distance, angle, area) (`984d4cf`).
- [x] **W5-B (`crates/sharex-tools` utilities)**: Added QR code RGBA generator, file hash checker (`MD5`, `SHA-1`, `SHA-256`, `SHA-512`), and directory indexer (`HTML`, `Markdown`, `JSON`, `TXT`) (`984d4cf`).
- [x] **W5-C (`crates/sharex-tools` + `sharex-app` pin)**: Added horizontal/vertical `ImageCombiner` strip stitching (`984d4cf`) and borderless always-on-top Pin-to-Screen floating viewports (`e836ebd`, `c4c1516`). (Follow-up: multi-row contact-sheet grid generator remains open.)

### Wave 6 (Hours 14.5 to 17.0) - OCR, custom workflows, and watch folders

- [x] **W6-A (`crates/sharex-tools` OCR)**: Added `OcrEngine` trait, `TesseractCliOcr` adapter with owner-only `0600` Unix temp PNG cleanup, `FallbackOcrChain` (`243943f`), and integrated OCR tab inside Tools panel (`c4c1516`). (Follow-up: native Windows WinRT `Windows.Media.Ocr` adapter can plug directly into `OcrEngine`.)
- [x] **W6-B (`crates/sharex-core` workflows)**: Added library APIs for per-hotkey `TaskOverrides`, `WorkflowPreset`, `WorkflowRegistry`, `resolve_effective_config`, and atomic `WatchFolderScanner` (`66daae4`). (Follow-up: runtime hotkey/watcher background loop and settings UI remain open.)

### Wave 7 (Hours 17.0 to 19.0) - Full UI parity, PL/EN localization, and packaging

- [x] **W7-A / W7-C (`crates/sharex-app` UI + i18n)**: Integrated System Tray, Pin-to-Screen, EN/PL core UI & sidebar localization (`i18n.rs`, with some advanced sub-panel labels still in English), Image Effects panel, 5-tab Tools panel, Recording panel + hotkeys (with background disk-spooled GIF finalization), shared CLI/GUI SQLite history with search and tags, and Uploaders `.sxcu` + presets + text snippet panel (`e836ebd`, `c4c1516`, `ef5d188`).
- [x] **W7-B (Performance & Release)**: Added O(1) bounded LRU thumbnail cache (`e836ebd`), `--portable` config mode, 1080p effects/PNG benchmark example (`bench_pipeline.rs`), and multi-platform GitHub Release workflow (`.github/workflows/release.yml`) (`86f88c3`).

### Wave 8 (Hours 19.0 to 20.0) - Cross-model audit and final verification

- [x] Cross-vendor review of `decisions.tsv` and cumulative diff (`codex/gpt-6.2-sol`) completed and all 4 blockers + notable risks remediated.
- [x] Full `no-comments` and `#[allow(...)]` audit across all workspace crates (`0` non-doc comments, `0` suppressions).
- [x] Full `fmt` + `clippy -D warnings` + `test` + `release` build + live CLI verification and fresh `sharex.exe` launch.
