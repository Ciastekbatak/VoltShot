#[cfg(target_os = "windows")]
mod imp {
    use std::ptr;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Threading::{
        AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, BringWindowToTop, ClipCursor, FindWindowW, GetForegroundWindow,
        GetWindowThreadProcessId, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
        SetForegroundWindow, SetWindowPos,
    };

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
    pub fn allow_self_foreground() {}

    pub fn release_cursor_clip() {}

    pub struct OverlayFocusGuard;

    impl OverlayFocusGuard {
        pub fn capture_current_foreground() -> Self {
            Self
        }

        pub fn enforce_overlay_focus(&mut self, _window_title: &str) {}

        pub fn restore_previous(&self) {}
    }
}

pub use imp::{OverlayFocusGuard, allow_self_foreground, release_cursor_clip};
