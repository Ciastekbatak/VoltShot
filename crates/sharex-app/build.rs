fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/sharex.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        compile_windows_resources();
    }
}

#[cfg(target_os = "windows")]
fn compile_windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/sharex.ico")
        .set("ProductName", "ShareX (Rust)")
        .set(
            "FileDescription",
            "ShareX in Rust - Screen Capture, Editor & Sharing",
        )
        .set("LegalCopyright", "GPL-3.0-or-later");
    res.compile()
        .expect("failed to compile Windows icon and executable metadata resources");
}

#[cfg(not(target_os = "windows"))]
fn compile_windows_resources() {}
