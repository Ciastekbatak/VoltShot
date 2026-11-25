use std::path::{Path, PathBuf};

/// Initiates a native OS outbound file drag-and-drop operation for `path`.
pub fn start_file_drag(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("File does not exist: {}", path.display()));
    }
    let normalized = normalize_drag_path(path)?;
    platform::start_file_drag(&normalized)
}

fn normalize_drag_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| err.to_string())?
            .join(path)
    };
    let canonical = absolute.canonicalize().unwrap_or(absolute);
    Ok(strip_verbatim_prefix(&canonical))
}

fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::Win32::Foundation::{
        BOOL, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK,
    };
    use windows::Win32::System::Com::IDataObject;
    use windows::Win32::System::Ole::{
        DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DoDragDrop, IDropSource, IDropSource_Impl,
        OleInitialize, OleUninitialize,
    };
    use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        ILClone, ILCreateFromPathW, ILFindLastID, ILFree, ILRemoveLastID, SHCreateDataObject,
    };
    use windows::core::{HRESULT, PCWSTR, implement};

    struct PidlGuard(*mut ITEMIDLIST);

    impl Drop for PidlGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    ILFree(Some(self.0.cast_const()));
                }
            }
        }
    }

    struct OleGuard(bool);

    impl Drop for OleGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    OleUninitialize();
                }
            }
        }
    }

    #[implement(IDropSource)]
    struct FileDropSource;

    impl IDropSource_Impl for FileDropSource_Impl {
        fn QueryContinueDrag(
            &self,
            escape_pressed: BOOL,
            key_state: MODIFIERKEYS_FLAGS,
        ) -> HRESULT {
            if escape_pressed.as_bool() {
                DRAGDROP_S_CANCEL
            } else if (key_state.0 & MK_LBUTTON.0) == 0 {
                DRAGDROP_S_DROP
            } else {
                S_OK
            }
        }

        fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    pub(super) fn start_file_drag(path: &Path) -> Result<(), String> {
        let ole_ok = unsafe { OleInitialize(None) }.is_ok();
        let _ole = OleGuard(ole_ok);

        let wide = encode_wide_nul(path.as_os_str());
        let full_pidl = PidlGuard(unsafe { ILCreateFromPathW(PCWSTR(wide.as_ptr())) });
        if full_pidl.0.is_null() {
            return Err(format!(
                "Could not resolve Shell item for {}",
                path.display()
            ));
        }
        let child_pidl = unsafe { ILFindLastID(full_pidl.0.cast_const()) };
        if child_pidl.is_null() {
            return Err(format!(
                "Could not locate Shell child item for {}",
                path.display()
            ));
        }
        let parent_pidl = PidlGuard(unsafe { ILClone(full_pidl.0.cast_const()) });
        if parent_pidl.0.is_null() || !unsafe { ILRemoveLastID(Some(parent_pidl.0)) }.as_bool() {
            return Err(format!(
                "Could not resolve parent folder for {}",
                path.display()
            ));
        }

        let children = [child_pidl as *const ITEMIDLIST];
        let data_object: IDataObject =
            unsafe { SHCreateDataObject(Some(parent_pidl.0.cast_const()), Some(&children), None) }
                .map_err(|err| format!("SHCreateDataObject failed: {err}"))?;

        let drop_source: IDropSource = FileDropSource.into();
        let mut effect = DROPEFFECT(0);
        let hr = unsafe {
            DoDragDrop(
                &data_object,
                &drop_source,
                DROPEFFECT_COPY | DROPEFFECT_LINK,
                &mut effect,
            )
        };
        if hr == DRAGDROP_S_DROP || hr == DRAGDROP_S_CANCEL || hr == S_OK {
            Ok(())
        } else {
            Err(format!("DoDragDrop failed with HRESULT 0x{:08X}", hr.0))
        }
    }

    fn encode_wide_nul(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use std::path::Path;

    pub(super) fn start_file_drag(_path: &Path) -> Result<(), String> {
        Err("Outbound file drag-and-drop is currently supported on Windows.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_windows_verbatim_prefixes_for_shell_drag_targets() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\C:\Users\Demo\shot.png")),
            PathBuf::from(r"C:\Users\Demo\shot.png")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\UNC\server\share\shot.png")),
            PathBuf::from(r"\\server\share\shot.png")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"C:\Users\Demo\shot.png")),
            PathBuf::from(r"C:\Users\Demo\shot.png")
        );
    }

    #[test]
    fn start_file_drag_rejects_missing_files() {
        let result = start_file_drag(Path::new("definitely_missing_drag_capture_file.png"));
        assert!(result.is_err());
    }
}
