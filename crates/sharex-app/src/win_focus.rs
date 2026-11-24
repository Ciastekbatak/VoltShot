/// Title of the main ShareX-RS desktop window used for Win32 window lookups.
pub const MAIN_WINDOW_TITLE: &str = "ShareX (Rust)";

#[cfg(target_os = "windows")]
mod imp {
    use super::MAIN_WINDOW_TITLE;
    use std::ptr;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND,
    };
    use windows_sys::Win32::System::Memory::SetProcessWorkingSetSizeEx;
    use windows_sys::Win32::System::Threading::{
        AttachThreadInput, CreateMutexW, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, BringWindowToTop, ClipCursor, FindWindowW, GetForegroundWindow,
        GetWindowThreadProcessId, HWND_TOPMOST, IsWindowVisible, PostMessageW, SW_RESTORE, SW_SHOW,
        SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowPos, ShowWindow,
        WM_PAINT,
    };

    const SINGLE_INSTANCE_MUTEX: &str = "Local\\ShareX_Rust_SingleInstance_v1";

    /// Holds the OS single-instance lock for the lifetime of the desktop GUI process.
    pub struct SingleInstanceGuard(HANDLE);

    impl Drop for SingleInstanceGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    pub fn acquire_single_instance() -> Option<SingleInstanceGuard> {
        let name: Vec<u16> = SINGLE_INSTANCE_MUTEX
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Some(SingleInstanceGuard(ptr::null_mut()));
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(handle);
            }
            focus_existing_main_window();
            return None;
        }
        Some(SingleInstanceGuard(handle))
    }

    fn focus_existing_main_window() {
        let wide: Vec<u16> = MAIN_WINDOW_TITLE
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let hwnd = unsafe { FindWindowW(ptr::null(), wide.as_ptr()) };
        if !hwnd.is_null() {
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
                PostMessageW(hwnd, WM_PAINT, 0, 0);
            }
        }
    }

    pub fn is_main_window_hidden() -> bool {
        let hwnd = find_own_window_by_title(MAIN_WINDOW_TITLE);
        !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } == 0
    }

    pub fn wake_hidden_main_window() {
        let hwnd = find_own_window_by_title(MAIN_WINDOW_TITLE);
        if !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } == 0 {
            unsafe {
                PostMessageW(hwnd, WM_PAINT, 0, 0);
            }
        }
    }

    pub fn allow_self_foreground() {
        unsafe {
            AllowSetForegroundWindow(GetCurrentProcessId());
        }
    }

    pub fn release_cursor_clip() {
        unsafe {
            ClipCursor(ptr::null());
        }
    }

    pub fn trim_working_set() {
        unsafe {
            SetProcessWorkingSetSizeEx(GetCurrentProcess(), usize::MAX, usize::MAX, 0);
        }
    }

    /// Holds the previously focused external window and keeps Win32 focus locked on the overlay.
    pub struct OverlayFocusGuard {
        previous_foreground: HWND,
        overlay_hwnd: HWND,
    }

    impl OverlayFocusGuard {
        pub fn capture_current_foreground() -> Self {
            let fg = unsafe { GetForegroundWindow() };
            let previous_foreground = if !fg.is_null() && window_pid(fg) != current_pid() {
                fg
            } else {
                ptr::null_mut()
            };
            release_cursor_clip();
            Self {
                previous_foreground,
                overlay_hwnd: ptr::null_mut(),
            }
        }

        pub fn enforce_overlay_focus(&mut self, window_title: &str) {
            release_cursor_clip();

            if self.overlay_hwnd.is_null() {
                self.overlay_hwnd = find_own_window_by_title(window_title);
            }
            if self.overlay_hwnd.is_null() {
                return;
            }

            let fg = unsafe { GetForegroundWindow() };
            if fg == self.overlay_hwnd {
                return;
            }

            unsafe {
                let current_tid = GetCurrentThreadId();
                let fg_tid = if !fg.is_null() {
                    GetWindowThreadProcessId(fg, ptr::null_mut())
                } else {
                    0
                };
                let attached = fg_tid != 0
                    && fg_tid != current_tid
                    && AttachThreadInput(current_tid, fg_tid, 1) != 0;

                SetWindowPos(
                    self.overlay_hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                );
                BringWindowToTop(self.overlay_hwnd);
                SetForegroundWindow(self.overlay_hwnd);
                SetActiveWindow(self.overlay_hwnd);
                SetFocus(self.overlay_hwnd);

                if attached {
                    AttachThreadInput(current_tid, fg_tid, 0);
                }
            }
        }

        pub fn restore_previous(&self) {
            if !self.previous_foreground.is_null() {
                unsafe {
                    SetForegroundWindow(self.previous_foreground);
                }
            }
        }
    }

    fn current_pid() -> u32 {
        unsafe { GetCurrentProcessId() }
    }

    fn window_pid(hwnd: HWND) -> u32 {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        pid
    }

    fn find_own_window_by_title(title: &str) -> HWND {
        let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let hwnd = unsafe { FindWindowW(ptr::null(), wide.as_ptr()) };
        if !hwnd.is_null() && window_pid(hwnd) == current_pid() {
            hwnd
        } else {
            ptr::null_mut()
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    pub struct SingleInstanceGuard;

    pub fn acquire_single_instance() -> Option<SingleInstanceGuard> {
        Some(SingleInstanceGuard)
    }

    pub fn is_main_window_hidden() -> bool {
        false
    }

    pub fn wake_hidden_main_window() {}

    pub fn allow_self_foreground() {}

    pub fn release_cursor_clip() {}

    pub fn trim_working_set() {}

    pub struct OverlayFocusGuard;

    impl OverlayFocusGuard {
        pub fn capture_current_foreground() -> Self {
            Self
        }

        pub fn enforce_overlay_focus(&mut self, _window_title: &str) {}

        pub fn restore_previous(&self) {}
    }
}

pub use imp::{
    OverlayFocusGuard, SingleInstanceGuard, acquire_single_instance, allow_self_foreground,
    is_main_window_hidden, release_cursor_clip, trim_working_set, wake_hidden_main_window,
};
