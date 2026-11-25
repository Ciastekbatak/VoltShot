/// Title of the main VoltShot desktop window used for Win32 window lookups.
pub const MAIN_WINDOW_TITLE: &str = "VoltShot";

#[cfg(target_os = "windows")]
mod imp {
    use super::MAIN_WINDOW_TITLE;
    use std::ptr;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, RECT,
    };
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Memory::SetProcessWorkingSetSizeEx;
    use windows_sys::Win32::System::Threading::{
        AttachThreadInput, CreateMutexW, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, BringWindowToTop, ClipCursor, FindWindowW, GetForegroundWindow,
        GetSystemMetrics, GetWindowRect, GetWindowThreadProcessId, HWND_TOPMOST, ICON_BIG,
        ICON_SMALL, IMAGE_ICON, IsWindow, IsWindowVisible, LR_DEFAULTSIZE, LR_SHARED, LoadImageW,
        PostMessageW, SM_CXSMICON, SM_CYSMICON, SW_HIDE, SW_RESTORE, SW_SHOW, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_SHOWWINDOW, SendMessageW,
        SetForegroundWindow, SetWindowPos, ShowWindow, WM_CLOSE, WM_PAINT, WM_SETICON,
    };

    const SINGLE_INSTANCE_MUTEX: &str = "Local\\VoltShot_SingleInstance_v1";
    static CACHED_MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
    static CACHED_OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);

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

    fn cached_main_hwnd() -> HWND {
        let cached = CACHED_MAIN_HWND.load(Ordering::Relaxed) as HWND;
        if !cached.is_null() && unsafe { IsWindow(cached) } != 0 {
            return cached;
        }
        let found = find_own_window_by_title(MAIN_WINDOW_TITLE);
        if !found.is_null() {
            CACHED_MAIN_HWND.store(found as isize, Ordering::Relaxed);
        }
        found
    }

    fn cached_overlay_hwnd(window_title: &str) -> HWND {
        let cached = CACHED_OVERLAY_HWND.load(Ordering::Relaxed) as HWND;
        if !cached.is_null() && unsafe { IsWindow(cached) } != 0 {
            return cached;
        }
        let found = find_own_window_by_title(window_title);
        if !found.is_null() {
            CACHED_OVERLAY_HWND.store(found as isize, Ordering::Relaxed);
        }
        found
    }

    pub fn is_main_window_hidden() -> bool {
        let hwnd = cached_main_hwnd();
        !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } == 0
    }

    pub fn wake_hidden_main_window() {
        let hwnd = cached_main_hwnd();
        if !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } == 0 {
            unsafe {
                PostMessageW(hwnd, WM_PAINT, 0, 0);
            }
        }
    }

    pub fn apply_main_window_icon() -> bool {
        let hwnd = cached_main_hwnd();
        if hwnd.is_null() {
            return false;
        }
        unsafe {
            let module = GetModuleHandleW(ptr::null());
            let big = LoadImageW(
                module,
                1 as *const u16,
                IMAGE_ICON,
                0,
                0,
                LR_DEFAULTSIZE | LR_SHARED,
            );
            if !big.is_null() {
                SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big as isize);
            }
            let small_w = GetSystemMetrics(SM_CXSMICON);
            let small_h = GetSystemMetrics(SM_CYSMICON);
            let small = LoadImageW(
                module,
                1 as *const u16,
                IMAGE_ICON,
                small_w,
                small_h,
                LR_SHARED,
            );
            if !small.is_null() {
                SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small as isize);
            }
        }
        true
    }

    pub fn restore_main_window() {
        let hwnd = cached_main_hwnd();
        if !hwnd.is_null() {
            apply_main_window_icon();
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
                PostMessageW(hwnd, WM_PAINT, 0, 0);
            }
        }
    }

    pub fn close_main_window_now() {
        let hwnd = cached_main_hwnd();
        if !hwnd.is_null() {
            unsafe {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
                PostMessageW(hwnd, WM_PAINT, 0, 0);
            }
        }
    }

    pub fn overlay_window_matches(
        window_title: &str,
        origin_x: i32,
        origin_y: i32,
        width: i32,
        height: i32,
    ) -> bool {
        let hwnd = cached_overlay_hwnd(window_title);
        if hwnd.is_null() {
            return false;
        }
        let mut rect: RECT = unsafe { std::mem::zeroed() };
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return false;
        }
        rect.left == origin_x
            && rect.top == origin_y
            && (rect.right - rect.left) == width.max(1)
            && (rect.bottom - rect.top) == height.max(1)
    }

    pub fn hide_overlay_window(window_title: &str) -> bool {
        let hwnd = cached_overlay_hwnd(window_title);
        if hwnd.is_null() {
            return false;
        }
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        true
    }

    pub fn wake_overlay_window(window_title: &str) -> bool {
        let hwnd = cached_overlay_hwnd(window_title);
        if hwnd.is_null() {
            return false;
        }
        unsafe {
            PostMessageW(hwnd, WM_PAINT, 0, 0);
        }
        true
    }

    pub fn prepare_overlay_window(
        window_title: &str,
        origin_x: i32,
        origin_y: i32,
        width: i32,
        height: i32,
    ) -> bool {
        let hwnd = cached_overlay_hwnd(window_title);
        if hwnd.is_null() {
            return false;
        }
        let disable_transitions: i32 = 1;
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_TRANSITIONS_FORCEDISABLED as u32,
                (&raw const disable_transitions).cast(),
                size_of::<i32>() as u32,
            );
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                origin_x,
                origin_y,
                width.max(1),
                height.max(1),
                SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
        true
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
        previous_foreground: isize,
        overlay_hwnd: isize,
    }

    impl OverlayFocusGuard {
        pub fn capture_current_foreground() -> Self {
            let fg = unsafe { GetForegroundWindow() };
            let previous_foreground = if !fg.is_null() && window_pid(fg) != current_pid() {
                fg as isize
            } else {
                0
            };
            release_cursor_clip();
            Self {
                previous_foreground,
                overlay_hwnd: 0,
            }
        }

        pub fn enforce_overlay_focus(&mut self, window_title: &str) {
            release_cursor_clip();

            let mut overlay_hwnd = self.overlay_hwnd as HWND;
            if overlay_hwnd.is_null() || unsafe { IsWindow(overlay_hwnd) } == 0 {
                overlay_hwnd = cached_overlay_hwnd(window_title);
                self.overlay_hwnd = overlay_hwnd as isize;
            }
            if overlay_hwnd.is_null() {
                return;
            }

            let fg = unsafe { GetForegroundWindow() };
            if fg == overlay_hwnd && unsafe { IsWindowVisible(overlay_hwnd) } != 0 {
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
                    overlay_hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                );
                BringWindowToTop(overlay_hwnd);
                SetForegroundWindow(overlay_hwnd);
                SetActiveWindow(overlay_hwnd);
                SetFocus(overlay_hwnd);

                if attached {
                    AttachThreadInput(current_tid, fg_tid, 0);
                }
            }
        }

        pub fn restore_previous(&self) {
            let prev = self.previous_foreground as HWND;
            if !prev.is_null() && unsafe { IsWindow(prev) } != 0 {
                unsafe {
                    SetForegroundWindow(prev);
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

    pub fn apply_main_window_icon() -> bool {
        true
    }

    pub fn restore_main_window() {}

    pub fn close_main_window_now() {}

    pub fn overlay_window_matches(
        _window_title: &str,
        _origin_x: i32,
        _origin_y: i32,
        _width: i32,
        _height: i32,
    ) -> bool {
        false
    }

    pub fn hide_overlay_window(_window_title: &str) -> bool {
        false
    }

    pub fn wake_overlay_window(_window_title: &str) -> bool {
        false
    }

    pub fn prepare_overlay_window(
        _window_title: &str,
        _origin_x: i32,
        _origin_y: i32,
        _width: i32,
        _height: i32,
    ) -> bool {
        false
    }

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
    apply_main_window_icon, close_main_window_now, hide_overlay_window, is_main_window_hidden,
    overlay_window_matches, prepare_overlay_window, release_cursor_clip, restore_main_window,
    trim_working_set, wake_hidden_main_window, wake_overlay_window,
};
