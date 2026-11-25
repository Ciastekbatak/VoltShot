use flate2::Compression;
use flate2::write::DeflateEncoder;
use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_embedded_payload)");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/voltshot.ico");
    println!("cargo:rerun-if-env-changed=VOLTSHOT_EMBED_PAYLOAD");
    println!("cargo:rerun-if-env-changed=VOLTSHOT_PAYLOAD_PATH");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR missing"));
    let compressed_path = out_dir.join("voltshot_payload.deflate");

    if let Some(payload_src) = resolve_payload_source() {
        println!("cargo:rerun-if-changed={}", payload_src.display());
        compress_payload(&payload_src, &compressed_path)
            .expect("failed to compress VoltShot executable payload");
        println!("cargo:rustc-cfg=has_embedded_payload");
    } else if !compressed_path.exists() {
        fs::write(&compressed_path, []).expect("failed to write empty payload placeholder");
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        compile_windows_resources();
    }
}

fn resolve_payload_source() -> Option<PathBuf> {
    if let Ok(explicit) = env::var("VOLTSHOT_PAYLOAD_PATH") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    if env::var("VOLTSHOT_EMBED_PAYLOAD").as_deref() == Ok("1") {
        let default_exe = PathBuf::from("../../target/release/voltshot.exe");
        if default_exe.is_file() {
            return Some(default_exe);
        }
    }
    None
}

fn compress_payload(src: &Path, dst: &Path) -> io::Result<()> {
    let mut input = File::open(src)?;
    let output = File::create(dst)?;
    let mut encoder = DeflateEncoder::new(output, Compression::best());
    io::copy(&mut input, &mut encoder)?;
    let mut finished = encoder.finish()?;
    finished.flush()?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn compile_windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/voltshot.ico")
        .set("ProductName", "VoltShot Setup")
        .set(
            "FileDescription",
            "VoltShot Installer - Next-Gen Screen Capture App",
        )
        .set("LegalCopyright", "GPL-3.0-or-later");
    res.compile()
        .expect("failed to compile VoltShot installer Windows resources");
}

#[cfg(not(target_os = "windows"))]
fn compile_windows_resources() {}
