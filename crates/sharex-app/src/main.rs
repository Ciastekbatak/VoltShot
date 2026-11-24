#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    #[cfg(all(target_os = "windows", not(debug_assertions)))]
    if std::env::args_os().len() > 1 {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }

    if let Err(message) = sharex_app::run() {
        eprintln!("sharex: {message}");
        std::process::exit(1);
    }
}
