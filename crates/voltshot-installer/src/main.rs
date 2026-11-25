#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

#[cfg(any(target_os = "windows", test))]
use flate2::read::DeflateDecoder;
use std::env;
#[cfg(any(target_os = "windows", test))]
use std::fs::{self, File};
#[cfg(any(target_os = "windows", test))]
use std::io::{self, Read};
#[cfg(any(target_os = "windows", test))]
use std::path::Path;
use std::path::PathBuf;

/// User-facing application name registered in Windows Start Menu, Search, and Installed Apps.
pub const APP_NAME: &str = "VoltShot";
/// Semantic version baked into the installer binary from Cargo metadata.
#[cfg(target_os = "windows")]
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Installed executable file name placed inside the per-user application directory.
#[cfg(any(target_os = "windows", test))]
const APP_EXE_NAME: &str = "VoltShot.exe";
/// Installed uninstaller executable file name referenced by Windows Settings.
#[cfg(target_os = "windows")]
const UNINSTALL_EXE_NAME: &str = "Uninstall.exe";
/// Publisher metadata displayed in Windows Installed Apps.
#[cfg(target_os = "windows")]
const PUBLISHER: &str = "VoltShot Community";
/// Community Discord invite linked from Windows Installed Apps metadata.
#[cfg(target_os = "windows")]
const COMMUNITY_URL: &str = "https://discord.gg/cRHrPusNGW";
/// Shortcut tooltip and description indexed by Windows Search.
#[cfg(target_os = "windows")]
const APP_DESCRIPTION: &str = "VoltShot - Next-Gen Screenshot & Capture App";

#[cfg(all(target_os = "windows", has_embedded_payload))]
static EMBEDDED_PAYLOAD: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/voltshot_payload.deflate"));

#[cfg(all(target_os = "windows", not(has_embedded_payload)))]
static EMBEDDED_PAYLOAD: &[u8] = &[];

#[derive(Clone, Debug, PartialEq, Eq)]
struct CliOptions {
    uninstall: bool,
    silent: bool,
    autostart: Option<bool>,
    launch_after_install: bool,
    install_dir: Option<PathBuf>,
    payload_override: Option<PathBuf>,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            uninstall: false,
            silent: false,
            autostart: None,
            launch_after_install: true,
            install_dir: None,
            payload_override: None,
        }
    }
}

fn parse_args<I, S>(args: I) -> Result<CliOptions, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut opts = CliOptions::default();
    let mut iter = args.into_iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_ref() {
            "--uninstall" | "/uninstall" | "/U" => opts.uninstall = true,
            "--silent" | "/S" | "/quiet" => opts.silent = true,
            "--autostart" => opts.autostart = Some(true),
            "--no-autostart" => opts.autostart = Some(false),
            "--no-launch" => opts.launch_after_install = false,
            "--dir" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "Missing path argument after --dir.".to_owned())?;
                opts.install_dir = Some(PathBuf::from(value.as_ref()));
            }
            "--payload" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "Missing path argument after --payload.".to_owned())?;
                opts.payload_override = Some(PathBuf::from(value.as_ref()));
            }
            other => {
                return Err(format!("Unknown installer option: {other}"));
            }
        }
    }
    Ok(opts)
}

#[cfg(any(target_os = "windows", test))]
fn default_install_dir_from(local_app_data: &Path) -> PathBuf {
    local_app_data.join("Programs").join(APP_NAME)
}

#[cfg(any(target_os = "windows", test))]
fn format_quoted_command(exe: &Path, suffix: &str) -> String {
    if suffix.is_empty() {
        format!("\"{}\"", exe.display())
    } else {
        format!("\"{}\" {suffix}", exe.display())
    }
}

#[cfg(any(target_os = "windows", test))]
fn decompress_payload_to_file(compressed: &[u8], destination: &Path) -> io::Result<u64> {
    if compressed.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "No embedded VoltShot binary payload found in this installer.",
        ));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid destination path."))?;
    fs::create_dir_all(parent)?;
    let temp_path = parent.join(format!("{APP_EXE_NAME}.tmp"));
    let mut decoder = DeflateDecoder::new(compressed);
    let mut out = File::create(&temp_path)?;
    let bytes = io::copy(&mut decoder, &mut out)?;
    out.sync_all()?;
    drop(out);
    if destination.exists() {
        let _ = fs::remove_file(destination);
    }
    fs::rename(&temp_path, destination)?;
    Ok(bytes)
}

#[cfg(target_os = "windows")]
fn copy_override_payload_to_file(source: &Path, destination: &Path) -> io::Result<u64> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid destination path."))?;
    fs::create_dir_all(parent)?;
    let mut input = File::open(source)?;
    let mut header = [0u8; 2];
    let _ = input.read(&mut header)?;
    drop(input);
    fs::copy(source, destination)
}

fn main() {
    let opts = match parse_args(env::args()) {
        Ok(opts) => opts,
        Err(err) => {
            platform::show_error(&format!("{APP_NAME} Setup"), &err, false);
            std::process::exit(2);
        }
    };

    if let Err(err) = platform::run(&opts) {
        platform::show_error(&format!("{APP_NAME} Setup"), &err, opts.silent);
        std::process::exit(1);
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{
        APP_DESCRIPTION, APP_EXE_NAME, APP_NAME, APP_VERSION, COMMUNITY_URL, CliOptions,
        EMBEDDED_PAYLOAD, PUBLISHER, UNINSTALL_EXE_NAME, copy_override_payload_to_file,
        decompress_payload_to_file, default_install_dir_from, format_quoted_command,
    };
    use std::env;
    use std::fs;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::ptr;
    use std::thread;
    use std::time::Duration;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize, IPersistFile,
    };
    use windows::Win32::UI::Shell::{
        IShellLinkW, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify, ShellLink,
    };
    use windows_core::{HSTRING, Interface};
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD, REG_SZ, RegCloseKey, RegCreateKeyW,
        RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDCANCEL, IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO,
        MB_YESNOCANCEL, MessageBoxW,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const UNINSTALL_REG_SUBKEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\VoltShot";
    const APP_PATHS_REG_SUBKEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\App Paths\VoltShot.exe";
    const RUN_REG_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const LEGACY_RUN_VALUE: &str = "ShareX-RS";

    struct RegKeyGuard(HKEY);

    impl Drop for RegKeyGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    RegCloseKey(self.0);
                }
            }
        }
    }

    fn wide_nul(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn show_error(title: &str, message: &str, silent: bool) {
        if silent {
            eprintln!("{title}: {message}");
            return;
        }
        let wide_title = wide_nul(title);
        let wide_msg = wide_nul(message);
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                wide_msg.as_ptr(),
                wide_title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }

    fn show_info(title: &str, message: &str) {
        let wide_title = wide_nul(title);
        let wide_msg = wide_nul(message);
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                wide_msg.as_ptr(),
                wide_title.as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }

    fn ask_install_choice(install_dir: &Path) -> Option<bool> {
        let title = wide_nul(&format!("{APP_NAME} Setup v{APP_VERSION}"));
        let prompt = format!(
            "Install {APP_NAME} v{APP_VERSION} (Next-Gen Screenshot App)?\n\n\
             Install folder:\n{}\n\n\
             This adds {APP_NAME} to Windows Search, Start Menu, and Installed Apps (no admin rights required).\n\n\
             • Yes — Install + Start automatically with Windows (Recommended)\n\
             • No — Install only (manual launch)\n\
             • Cancel — Exit setup",
            install_dir.display()
        );
        let wide_msg = wide_nul(&prompt);
        let response = unsafe {
            MessageBoxW(
                ptr::null_mut(),
                wide_msg.as_ptr(),
                title.as_ptr(),
                MB_YESNOCANCEL | MB_ICONINFORMATION,
            )
        };
        if response == IDCANCEL {
            None
        } else {
            Some(response == IDYES)
        }
    }

    fn ask_uninstall_confirm() -> bool {
        let title = wide_nul(&format!("Uninstall {APP_NAME}"));
        let prompt = wide_nul(&format!(
            "Remove {APP_NAME} from Windows Search, Start Menu, Startup, and Installed Apps?\n\n\
             Your saved screenshots and configuration files will NOT be deleted."
        ));
        let response = unsafe {
            MessageBoxW(
                ptr::null_mut(),
                prompt.as_ptr(),
                title.as_ptr(),
                MB_YESNO | MB_ICONQUESTION,
            )
        };
        response == IDYES
    }

    pub fn run(opts: &CliOptions) -> Result<(), String> {
        let install_dir = resolve_install_dir(opts)?;
        if opts.uninstall {
            uninstall(&install_dir, opts.silent)
        } else {
            install(&install_dir, opts)
        }
    }

    fn resolve_install_dir(opts: &CliOptions) -> Result<PathBuf, String> {
        if let Some(dir) = &opts.install_dir {
            return Ok(dir.clone());
        }
        let local_app_data = env::var_os("LOCALAPPDATA")
            .ok_or_else(|| "LOCALAPPDATA environment variable is unavailable.".to_owned())?;
        Ok(default_install_dir_from(Path::new(&local_app_data)))
    }

    fn start_menu_shortcut_path() -> Result<PathBuf, String> {
        let app_data = env::var_os("APPDATA")
            .ok_or_else(|| "APPDATA environment variable is unavailable.".to_owned())?;
        Ok(PathBuf::from(app_data)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join(format!("{APP_NAME}.lnk")))
    }

    fn desktop_shortcut_path() -> Option<PathBuf> {
        let profile = env::var_os("USERPROFILE")?;
        let desktop = PathBuf::from(profile).join("Desktop");
        desktop
            .is_dir()
            .then(|| desktop.join(format!("{APP_NAME}.lnk")))
    }

    fn stop_running_app() {
        let _ = Command::new("taskkill")
            .args(["/IM", APP_EXE_NAME, "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        thread::sleep(Duration::from_millis(250));
    }

    fn install(install_dir: &Path, opts: &CliOptions) -> Result<(), String> {
        let enable_autostart = if opts.silent {
            opts.autostart.unwrap_or(true)
        } else if let Some(explicit) = opts.autostart {
            explicit
        } else {
            match ask_install_choice(install_dir) {
                Some(choice) => choice,
                None => return Ok(()),
            }
        };

        stop_running_app();

        let exe_path = install_dir.join(APP_EXE_NAME);
        let installed_bytes = if let Some(override_src) = &opts.payload_override {
            copy_override_payload_to_file(override_src, &exe_path)
                .map_err(|err| format!("Failed to install {APP_EXE_NAME}: {err}"))?
        } else {
            decompress_payload_to_file(EMBEDDED_PAYLOAD, &exe_path)
                .map_err(|err| format!("Failed to extract {APP_EXE_NAME}: {err}"))?
        };

        let uninstall_path = install_dir.join(UNINSTALL_EXE_NAME);
        let current_exe = env::current_exe()
            .map_err(|err| format!("Failed to locate setup executable: {err}"))?;
        if current_exe != uninstall_path {
            fs::copy(&current_exe, &uninstall_path)
                .map_err(|err| format!("Failed to write uninstaller: {err}"))?;
        }

        let start_shortcut = start_menu_shortcut_path()?;
        create_shortcut(&start_shortcut, &exe_path, install_dir, APP_DESCRIPTION)?;
        if let Some(desktop_shortcut) = desktop_shortcut_path() {
            let _ = create_shortcut(&desktop_shortcut, &exe_path, install_dir, APP_DESCRIPTION);
        }

        let size_kb = u32::try_from((installed_bytes / 1024).max(1)).unwrap_or(u32::MAX);
        register_uninstall_entry(install_dir, &exe_path, &uninstall_path, size_kb)?;
        register_app_paths(&exe_path, install_dir)?;
        set_autostart(&exe_path, enable_autostart)?;
        notify_shell();

        if opts.launch_after_install {
            let _ = Command::new(&exe_path).spawn();
        }

        if !opts.silent {
            show_info(
                &format!("{APP_NAME} Installed"),
                &format!(
                    "{APP_NAME} v{APP_VERSION} is installed and ready!\n\n\
                     • Press Win and search \"{APP_NAME}\" anytime\n\
                     • Press Ctrl + PrintScreen for instant region capture\n\
                     • Manage or uninstall anytime in Windows Settings → Apps → Installed apps\n\n\
                     Community Discord: {COMMUNITY_URL}"
                ),
            );
        }
        Ok(())
    }

    fn uninstall(install_dir: &Path, silent: bool) -> Result<(), String> {
        if !silent && !ask_uninstall_confirm() {
            return Ok(());
        }

        stop_running_app();
        remove_autostart_entry(APP_NAME);
        remove_autostart_entry(LEGACY_RUN_VALUE);
        delete_registry_tree(UNINSTALL_REG_SUBKEY);
        delete_registry_tree(APP_PATHS_REG_SUBKEY);

        if let Ok(start_shortcut) = start_menu_shortcut_path() {
            let _ = fs::remove_file(start_shortcut);
        }
        if let Some(desktop_shortcut) = desktop_shortcut_path() {
            let _ = fs::remove_file(desktop_shortcut);
        }

        let exe_path = install_dir.join(APP_EXE_NAME);
        let _ = fs::remove_file(&exe_path);
        notify_shell();

        let uninstall_path = install_dir.join(UNINSTALL_EXE_NAME);
        let current_exe = env::current_exe().ok();
        if current_exe.as_deref() == Some(uninstall_path.as_path()) {
            schedule_self_cleanup(&uninstall_path, install_dir);
        } else {
            let _ = fs::remove_file(&uninstall_path);
            let _ = fs::remove_dir(install_dir);
        }

        if !silent {
            show_info(
                &format!("{APP_NAME} Uninstalled"),
                &format!(
                    "{APP_NAME} has been removed from your system.\n\n\
                     Your saved screenshots and settings were preserved."
                ),
            );
        }
        Ok(())
    }

    fn schedule_self_cleanup(uninstall_exe: &Path, install_dir: &Path) {
        let script = format!(
            "ping -n 3 127.0.0.1 >nul & del /f /q \"{}\" & rmdir /q \"{}\"",
            uninstall_exe.display(),
            install_dir.display()
        );
        let _ = Command::new("cmd.exe")
            .args(["/C", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }

    fn create_shortcut(
        shortcut_path: &Path,
        target: &Path,
        work_dir: &Path,
        description: &str,
    ) -> Result<(), String> {
        if let Some(parent) = shortcut_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create shortcut folder: {err}"))?;
        }
        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            if hr.is_err() {
                return Err(format!("COM initialization failed: {hr:?}"));
            }
            let result = (|| -> windows_core::Result<()> {
                let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
                link.SetPath(&HSTRING::from(target.as_os_str()))?;
                link.SetWorkingDirectory(&HSTRING::from(work_dir.as_os_str()))?;
                link.SetDescription(&HSTRING::from(description))?;
                link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)?;
                let persist: IPersistFile = link.cast()?;
                persist.Save(&HSTRING::from(shortcut_path.as_os_str()), true)?;
                Ok(())
            })();
            CoUninitialize();
            result.map_err(|err| format!("Failed to create Windows shortcut: {err}"))
        }
    }

    fn create_registry_key(subkey: &str) -> Result<RegKeyGuard, String> {
        let wide_subkey = wide_nul(subkey);
        let mut hkey: HKEY = ptr::null_mut();
        let status = unsafe { RegCreateKeyW(HKEY_CURRENT_USER, wide_subkey.as_ptr(), &mut hkey) };
        if status == ERROR_SUCCESS {
            Ok(RegKeyGuard(hkey))
        } else {
            Err(format!(
                "Failed to create registry key {subkey} (error {status})."
            ))
        }
    }

    fn set_reg_sz(key: &RegKeyGuard, name: &str, value: &str) -> Result<(), String> {
        let wide_name = wide_nul(name);
        let wide_value = wide_nul(value);
        let byte_len = u32::try_from(wide_value.len().saturating_mul(size_of::<u16>()))
            .map_err(|_| "Registry string value exceeds maximum size.".to_owned())?;
        let name_ptr = if name.is_empty() {
            ptr::null()
        } else {
            wide_name.as_ptr()
        };
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name_ptr,
                0,
                REG_SZ,
                wide_value.as_ptr().cast::<u8>(),
                byte_len,
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!(
                "Failed to write registry value {name} (error {status})."
            ))
        }
    }

    fn set_reg_dword(key: &RegKeyGuard, name: &str, value: u32) -> Result<(), String> {
        let wide_name = wide_nul(name);
        let bytes = value.to_le_bytes();
        let status = unsafe {
            RegSetValueExW(
                key.0,
                wide_name.as_ptr(),
                0,
                REG_DWORD,
                bytes.as_ptr(),
                u32::try_from(bytes.len()).unwrap_or(4),
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!(
                "Failed to write DWORD registry value {name} (error {status})."
            ))
        }
    }

    fn delete_registry_tree(subkey: &str) {
        let wide_subkey = wide_nul(subkey);
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, wide_subkey.as_ptr());
        }
    }

    fn register_uninstall_entry(
        install_dir: &Path,
        exe_path: &Path,
        uninstall_path: &Path,
        size_kb: u32,
    ) -> Result<(), String> {
        let key = create_registry_key(UNINSTALL_REG_SUBKEY)?;
        set_reg_sz(&key, "DisplayName", APP_NAME)?;
        set_reg_sz(&key, "DisplayVersion", APP_VERSION)?;
        set_reg_sz(&key, "Publisher", PUBLISHER)?;
        set_reg_sz(&key, "InstallLocation", &install_dir.display().to_string())?;
        set_reg_sz(&key, "DisplayIcon", &format!("{},0", exe_path.display()))?;
        set_reg_sz(
            &key,
            "UninstallString",
            &format_quoted_command(uninstall_path, "--uninstall"),
        )?;
        set_reg_sz(
            &key,
            "QuietUninstallString",
            &format_quoted_command(uninstall_path, "--uninstall --silent"),
        )?;
        set_reg_sz(&key, "URLInfoAbout", COMMUNITY_URL)?;
        set_reg_dword(&key, "EstimatedSize", size_kb)?;
        set_reg_dword(&key, "NoModify", 1)?;
        set_reg_dword(&key, "NoRepair", 1)?;
        Ok(())
    }

    fn register_app_paths(exe_path: &Path, install_dir: &Path) -> Result<(), String> {
        let key = create_registry_key(APP_PATHS_REG_SUBKEY)?;
        set_reg_sz(&key, "", &exe_path.display().to_string())?;
        set_reg_sz(&key, "Path", &install_dir.display().to_string())?;
        Ok(())
    }

    fn set_autostart(exe_path: &Path, enabled: bool) -> Result<(), String> {
        remove_autostart_entry(LEGACY_RUN_VALUE);
        if enabled {
            let key = create_registry_key(RUN_REG_SUBKEY)?;
            set_reg_sz(
                &key,
                APP_NAME,
                &format_quoted_command(exe_path, "--minimized"),
            )?;
        } else {
            remove_autostart_entry(APP_NAME);
        }
        Ok(())
    }

    fn remove_autostart_entry(name: &str) {
        let wide_subkey = wide_nul(RUN_REG_SUBKEY);
        let mut hkey: HKEY = ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide_subkey.as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut hkey,
            )
        };
        if status != ERROR_SUCCESS {
            return;
        }
        let guard = RegKeyGuard(hkey);
        let wide_name = wide_nul(name);
        let delete_status = unsafe { RegDeleteValueW(guard.0, wide_name.as_ptr()) };
        if delete_status != ERROR_SUCCESS && delete_status != ERROR_FILE_NOT_FOUND {
            eprintln!("Warning: could not remove startup entry {name} (error {delete_status})");
        }
    }

    fn notify_shell() {
        unsafe {
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::CliOptions;

    pub fn show_error(title: &str, message: &str, _silent: bool) {
        eprintln!("{title}: {message}");
    }

    pub fn run(_opts: &CliOptions) -> Result<(), String> {
        Err("The VoltShot GUI installer is intended for Windows.".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::DeflateEncoder;
    use std::io::Write;

    #[test]
    fn parse_args_supports_install_and_uninstall_flags() {
        let install_opts = parse_args([
            "voltshot-setup.exe",
            "--silent",
            "--no-autostart",
            "--no-launch",
            "--dir",
            r"C:\Apps\VoltShot",
        ])
        .expect("valid install flags");
        assert!(!install_opts.uninstall);
        assert!(install_opts.silent);
        assert_eq!(install_opts.autostart, Some(false));
        assert!(!install_opts.launch_after_install);
        assert_eq!(
            install_opts.install_dir,
            Some(PathBuf::from(r"C:\Apps\VoltShot"))
        );

        let uninstall_opts =
            parse_args(["Uninstall.exe", "--uninstall", "/S"]).expect("valid uninstall flags");
        assert!(uninstall_opts.uninstall);
        assert!(uninstall_opts.silent);
    }

    #[test]
    fn parse_args_rejects_unknown_flag_and_missing_dir_value() {
        assert!(parse_args(["voltshot-setup.exe", "--unknown"]).is_err());
        assert!(parse_args(["voltshot-setup.exe", "--dir"]).is_err());
    }

    #[test]
    fn default_install_dir_and_quoted_commands_format_cleanly() {
        let dir = default_install_dir_from(Path::new(r"C:\Users\Alex\AppData\Local"));
        assert_eq!(
            dir,
            PathBuf::from(r"C:\Users\Alex\AppData\Local\Programs\VoltShot")
        );
        let exe = dir.join(APP_EXE_NAME);
        assert_eq!(
            format_quoted_command(&exe, "--minimized"),
            format!("\"{}\" --minimized", exe.display())
        );
        assert_eq!(
            format_quoted_command(&exe, ""),
            format!("\"{}\"", exe.display())
        );
    }

    #[test]
    fn decompress_payload_round_trips_binary_bytes() {
        let sample = b"MZ-voltshot-payload-verification-bytes";
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(sample).expect("compress sample");
        let compressed = encoder.finish().expect("finish deflate");

        let temp_dir =
            env::temp_dir().join(format!("voltshot-installer-test-{}", std::process::id()));
        let target = temp_dir.join(APP_EXE_NAME);
        let written = decompress_payload_to_file(&compressed, &target).expect("extract payload");
        assert_eq!(written, sample.len() as u64);
        assert_eq!(fs::read(&target).expect("read extracted"), sample);
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
