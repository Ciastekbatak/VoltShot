# Contributing to VoltShot

Every pull request must keep `voltshot-core` free of OS capture, clipboard, and GUI dependencies. Put platform code in `voltshot-capture` or `voltshot-app`, and put destination plugins in `voltshot-uploaders`.

## Prerequisites

- Rust 1.85 or newer (`rustup update stable`)
- `rustfmt` and `clippy` components (`rustup component add rustfmt clippy`)

On Ubuntu or Debian, install the native display and windowing libraries before building:

```bash
sudo apt-get update
sudo apt-get install -y \
	pkg-config \
	libclang-dev \
	libxcb1-dev \
	libxrandr-dev \
	libdbus-1-dev \
	libpipewire-0.3-dev \
	libwayland-dev \
	libegl-dev \
	libxkbcommon-dev \
	libfontconfig-dev \
	libgtk-3-dev \
	libayatana-appindicator3-dev
```

Windows and macOS require no extra system packages.

## Run local checks

Run the same three checks that GitHub Actions runs before opening a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Add a new uploader

1. Create a module under `crates/voltshot-uploaders/src/`.
2. Implement the `voltshot_core::Uploader` trait (`id`, `name`, `description`, `upload`).
3. Register the uploader in `UploaderRegistry` inside `crates/voltshot-uploaders/src/lib.rs`.
4. Add unit tests that verify the observable behavior of `upload` against a local mock or temporary directory.

## Architecture rules

- Validate untrusted input at boundaries (CLI flags, TOML config, region strings, external API responses) and convert it into typed models in `voltshot-core`.
- Prefer enums and validated structs over loose booleans or stringly typed flags.
- Write tests against public crate interfaces so internal refactors do not break tests.
- Keep pull requests focused on one crate or one feature slice.
