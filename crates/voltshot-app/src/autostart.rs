use std::path::Path;

/// Registry subkey under `HKEY_CURRENT_USER` storing per-user startup commands on Windows.
pub const RUN_REGISTRY_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Cross-platform helper managing per-user Run-at-Startup registration (Windows Registry, Linux XDG, macOS LaunchAgents).
pub struct AutostartManager;

impl AutostartManager {
    /// Returns `true` when startup registration is supported on the current operating system.
    pub const fn is_supported() -> bool {
        cfg!(any(
            target_os = "windows",
            target_os = "linux",
            target_os = "macos"
        ))
    }

    /// Returns `true` when `app_name` is currently registered to launch at user sign-in.
    pub fn is_enabled(app_name: &str) -> bool {
        imp::is_enabled(app_name)
    }

    /// Registers or removes `app_name` in the OS per-user startup configuration.
    pub fn set_enabled(
        app_name: &str,
        exe_path: &Path,
        minimized: bool,
        enabled: bool,
    ) -> Result<(), String> {
        imp::set_enabled(app_name, exe_path, minimized, enabled)
    }

    /// Formats the quoted startup command line for `exe_path`, appending `--minimized` when requested.
    pub fn format_run_command(exe_path: &Path, minimized: bool) -> String {
        let quoted = format!("\"{}\"", exe_path.display());
        if minimized {
            format!("{quoted} --minimized")
        } else {
            quoted
        }
    }

    /// Formats a Freedesktop XDG autostart `.desktop` entry for Linux desktop environments.
    pub fn format_linux_desktop_entry(app_name: &str, exe_path: &Path, minimized: bool) -> String {
        let escaped_path = exe_path
            .display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let exec = if minimized {
            format!("\"{escaped_path}\" --minimized")
        } else {
            format!("\"{escaped_path}\"")
        };
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Version=1.0\n\
             Name={app_name}\n\
             Comment=Next-gen screen capture, annotation editor, and sharing\n\
             Exec={exec}\n\
             Icon=voltshot\n\
             Terminal=false\n\
             Categories=Graphics;Utility;\n\
             X-GNOME-Autostart-enabled=true\n"
        )
    }

    /// Formats a macOS `launchd` user LaunchAgent `.plist` XML document.
    pub fn format_macos_launch_agent_plist(
        label: &str,
        exe_path: &Path,
        minimized: bool,
    ) -> String {
        let escaped_label = escape_xml(label);
        let escaped_exe = escape_xml(&exe_path.display().to_string());
        let minimized_arg = if minimized {
            "\n        <string>--minimized</string>"
        } else {
            ""
        };
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
                 <key>Label</key>\n\
                 <string>{escaped_label}</string>\n\
                 <key>ProgramArguments</key>\n\
                 <array>\n\
                     <string>{escaped_exe}</string>{minimized_arg}\n\
                 </array>\n\
                 <key>RunAtLoad</key>\n\
                 <true/>\n\
             </dict>\n\
             </plist>\n"
        )
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Encodes `value` as a NUL-terminated UTF-16 code unit sequence for Win32 wide-string APIs.
pub fn encode_utf16_nul(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
mod imp {
    use super::{AutostartManager, RUN_REGISTRY_SUBKEY, encode_utf16_nul};
    use std::path::Path;
    use std::ptr;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, REG_VALUE_TYPE,
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };

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

    fn open_run_key(access: u32) -> Result<RegKeyGuard, u32> {
        let subkey = encode_utf16_nul(RUN_REGISTRY_SUBKEY);
        let mut hkey: HKEY = ptr::null_mut();
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, access, &mut hkey) };
        if status == ERROR_SUCCESS {
            Ok(RegKeyGuard(hkey))
        } else {
            Err(status)
        }
    }

    pub fn is_enabled(app_name: &str) -> bool {
        if app_name.is_empty() {
            return false;
        }
        let Ok(key) = open_run_key(KEY_QUERY_VALUE) else {
            return false;
        };
        let value_name = encode_utf16_nul(app_name);
        let mut value_type: REG_VALUE_TYPE = 0;
        let mut data_len: u32 = 0;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                value_name.as_ptr(),
                ptr::null(),
                &mut value_type,
                ptr::null_mut(),
                &mut data_len,
            )
        };
        status == ERROR_SUCCESS && data_len > 0
    }

    pub fn set_enabled(
        app_name: &str,
        exe_path: &Path,
        minimized: bool,
        enabled: bool,
    ) -> Result<(), String> {
        if app_name.is_empty() {
            return Err("Application name must not be empty.".to_string());
        }

        let key = open_run_key(KEY_SET_VALUE)
            .map_err(|code| format!("Failed to open Windows Run registry key (error {code})."))?;
        let value_name = encode_utf16_nul(app_name);

        if enabled {
            let command = AutostartManager::format_run_command(exe_path, minimized);
            let wide_command = encode_utf16_nul(&command);
            let byte_len = u32::try_from(wide_command.len().saturating_mul(size_of::<u16>()))
                .map_err(|_| "Startup command exceeds maximum registry value size.".to_string())?;
            let status = unsafe {
                RegSetValueExW(
                    key.0,
                    value_name.as_ptr(),
                    0,
                    REG_SZ,
                    wide_command.as_ptr().cast::<u8>(),
                    byte_len,
                )
            };
            if status == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(format!(
                    "Failed to write Windows Run registry value (error {status})."
                ))
            }
        } else {
            let status = unsafe { RegDeleteValueW(key.0, value_name.as_ptr()) };
            if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(format!(
                    "Failed to remove Windows Run registry value (error {status})."
                ))
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::AutostartManager;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn desktop_entry_path(app_name: &str) -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        let slug = app_name.to_ascii_lowercase().replace(' ', "-");
        Some(base.join("autostart").join(format!("{slug}.desktop")))
    }

    pub fn is_enabled(app_name: &str) -> bool {
        if app_name.is_empty() {
            return false;
        }
        desktop_entry_path(app_name).is_some_and(|path| path.is_file())
    }

    pub fn set_enabled(
        app_name: &str,
        exe_path: &Path,
        minimized: bool,
        enabled: bool,
    ) -> Result<(), String> {
        if app_name.is_empty() {
            return Err("Application name must not be empty.".to_string());
        }
        let path = desktop_entry_path(app_name)
            .ok_or_else(|| "Could not determine Linux XDG autostart directory.".to_string())?;
        if enabled {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("Failed to create autostart directory: {err}"))?;
            }
            let content =
                AutostartManager::format_linux_desktop_entry(app_name, exe_path, minimized);
            fs::write(&path, content)
                .map_err(|err| format!("Failed to write {}: {err}", path.display()))
        } else {
            match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(format!("Failed to remove {}: {err}", path.display())),
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::AutostartManager;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn launch_agent_label(app_name: &str) -> String {
        let slug = app_name.to_ascii_lowercase().replace(' ', "-");
        format!("io.{slug}.app")
    }

    fn launch_agent_path(app_name: &str) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        let label = launch_agent_label(app_name);
        Some(
            home.join("Library")
                .join("LaunchAgents")
                .join(format!("{label}.plist")),
        )
    }

    pub fn is_enabled(app_name: &str) -> bool {
        if app_name.is_empty() {
            return false;
        }
        launch_agent_path(app_name).is_some_and(|path| path.is_file())
    }

    pub fn set_enabled(
        app_name: &str,
        exe_path: &Path,
        minimized: bool,
        enabled: bool,
    ) -> Result<(), String> {
        if app_name.is_empty() {
            return Err("Application name must not be empty.".to_string());
        }
        let path = launch_agent_path(app_name)
            .ok_or_else(|| "Could not determine macOS LaunchAgents directory.".to_string())?;
        if enabled {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("Failed to create LaunchAgents directory: {err}"))?;
            }
            let label = launch_agent_label(app_name);
            let content =
                AutostartManager::format_macos_launch_agent_plist(&label, exe_path, minimized);
            fs::write(&path, content)
                .map_err(|err| format!("Failed to write {}: {err}", path.display()))
        } else {
            match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(format!("Failed to remove {}: {err}", path.display())),
            }
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod imp {
    use std::path::Path;

    pub fn is_enabled(_app_name: &str) -> bool {
        false
    }

    pub fn set_enabled(
        _app_name: &str,
        _exe_path: &Path,
        _minimized: bool,
        enabled: bool,
    ) -> Result<(), String> {
        if enabled {
            Err("Startup registration is not supported on this platform.".to_string())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn format_run_command_quotes_paths_with_spaces_and_handles_minimized_flag() {
        let path_with_spaces = PathBuf::from(r"C:\Program Files\VoltShot\voltshot.exe");
        assert_eq!(
            AutostartManager::format_run_command(&path_with_spaces, false),
            r#""C:\Program Files\VoltShot\voltshot.exe""#
        );
        assert_eq!(
            AutostartManager::format_run_command(&path_with_spaces, true),
            r#""C:\Program Files\VoltShot\voltshot.exe" --minimized"#
        );

        let simple_path = PathBuf::from(r"C:\Tools\voltshot.exe");
        assert_eq!(
            AutostartManager::format_run_command(&simple_path, false),
            r#""C:\Tools\voltshot.exe""#
        );
        assert_eq!(
            AutostartManager::format_run_command(&simple_path, true),
            r#""C:\Tools\voltshot.exe" --minimized"#
        );
    }

    #[test]
    fn encode_utf16_nul_terminates_ascii_empty_and_unicode_strings() {
        assert_eq!(encode_utf16_nul(""), vec![0]);
        assert_eq!(
            encode_utf16_nul("VoltShot"),
            vec![
                b'V' as u16,
                b'o' as u16,
                b'l' as u16,
                b't' as u16,
                b'S' as u16,
                b'h' as u16,
                b'o' as u16,
                b't' as u16,
                0
            ]
        );

        let unicode = encode_utf16_nul("Śx📷");
        assert_eq!(unicode.last(), Some(&0));
        let decoded = String::from_utf16(&unicode[..unicode.len() - 1]).expect("valid utf-16");
        assert_eq!(decoded, "Śx📷");
    }

    #[test]
    fn formats_linux_xdg_desktop_entry_and_macos_launch_agent_plist() {
        let linux_entry = AutostartManager::format_linux_desktop_entry(
            "VoltShot",
            Path::new("/usr/local/bin/voltshot"),
            true,
        );
        assert!(linux_entry.contains("[Desktop Entry]"));
        assert!(linux_entry.contains("Name=VoltShot"));
        assert!(linux_entry.contains("Exec=\"/usr/local/bin/voltshot\" --minimized"));
        assert!(linux_entry.contains("Icon=voltshot"));
        assert!(linux_entry.contains("X-GNOME-Autostart-enabled=true"));

        let mac_plist = AutostartManager::format_macos_launch_agent_plist(
            "io.voltshot.app",
            Path::new("/Applications/VoltShot & Tools/VoltShot.app/Contents/MacOS/voltshot"),
            true,
        );
        assert!(mac_plist.contains("<string>io.voltshot.app</string>"));
        assert!(mac_plist.contains(
            "<string>/Applications/VoltShot &amp; Tools/VoltShot.app/Contents/MacOS/voltshot</string>"
        ));
        assert!(mac_plist.contains("<string>--minimized</string>"));
        assert!(mac_plist.contains("<key>RunAtLoad</key>"));
        assert!(AutostartManager::is_supported());
    }
}
