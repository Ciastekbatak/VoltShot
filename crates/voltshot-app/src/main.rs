#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    install_crash_log_hook();

    #[cfg(all(target_os = "windows", not(debug_assertions)))]
    if std::env::args_os().len() > 1 {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }

    if let Err(message) = voltshot_app::run() {
        eprintln!("voltshot: {message}");
        std::process::exit(1);
    }
}

fn install_crash_log_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let log_path = std::env::temp_dir().join("voltshot-crash.log");
        let backtrace = std::backtrace::Backtrace::force_capture();
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
        {
            use std::io::Write as _;
            let _ = writeln!(file, "=== VoltShot panic ===\n{info}\n{backtrace}\n");
        }
        default_hook(info);
    }));
}
