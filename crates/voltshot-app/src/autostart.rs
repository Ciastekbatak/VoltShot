use std::path::Path;

/// Registry subkey under `HKEY_CURRENT_USER` storing per-user startup commands on Windows.
pub const RUN_REGISTRY_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Cross-platform helper managing Windows per-user Run-at-Startup registration.
pub struct AutostartManager;

impl AutostartManager {
    /// Returns `true` when `app_name` is currently registered to launch at user sign-in.
    pub fn is_enabled(app_name: &str) -> bool {
        imp::is_enabled(app_name)
    }

    /// Registers or removes `app_name` in the per-user startup registry key.
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

#[cfg(not(target_os = "windows"))]
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
            Err("Windows startup registration is only supported on Windows.".to_string())
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
}
