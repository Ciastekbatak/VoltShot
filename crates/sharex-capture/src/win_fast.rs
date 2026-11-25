use std::{
    mem::{self, MaybeUninit},
    ptr, slice,
};

use sharex_core::{CapturedImage, CoreError, WindowId, WindowInfo};
use windows_sys::Win32::{
    Foundation::{BOOL, HWND, LPARAM, RECT},
    Graphics::{
        Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
        Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC,
            CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, ERROR, GdiFlush, GetDC,
            GetWindowRgnBox, HDC, HGDIOBJ, MONITOR_DEFAULTTONULL, MonitorFromRect, NULLREGION,
            ReleaseDC, SRCCOPY, SelectObject,
        },
    },
    System::{
        StationsAndDesktops::{
            GetProcessWindowStation, GetUserObjectInformationW, UOI_FLAGS, USEROBJECTFLAGS,
        },
        Threading::GetCurrentProcessId,
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetLayeredWindowAttributes,
        GetSystemMetrics, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, LWA_ALPHA, SM_CXSCREEN,
        SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
        WS_EX_LAYERED, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WSF_VISIBLE,
    },
};

use super::{DesktopSnapshot, ScreenRect};

pub(super) fn capture_virtual_desktop_gdi() -> Result<DesktopSnapshot, CoreError> {
    let bounds = virtual_screen().ok_or_else(|| failure("no virtual desktop"))?;
    let origin_x = bounds.left as i32;
    let origin_y = bounds.top as i32;
    let image = capture_screen_area(origin_x, origin_y, bounds.width(), bounds.height())?;
    Ok(DesktopSnapshot {
        image,
        origin_x,
        origin_y,
    })
}

pub(super) fn capture_screen_area(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<CapturedImage, CoreError> {
    let dib_width = i32::try_from(width).map_err(|_| failure("invalid bitmap width"))?;
    let dib_height = i32::try_from(height).map_err(|_| failure("invalid bitmap height"))?;
    let len = usize::try_from(u64::from(width) * u64::from(height) * 4)
        .ok()
        .filter(|len| (1..=isize::MAX as usize).contains(len))
        .ok_or_else(|| failure("invalid bitmap size"))?;
    if !has_visible_window_station() {
        return Err(failure("no interactive desktop"));
    }

    let screen = ScreenDc(unsafe { GetDC(ptr::null_mut()) });
    if screen.0.is_null() {
        return Err(failure("GetDC failed"));
    }
    let dc = MemoryDc(unsafe { CreateCompatibleDC(screen.0) });
    if dc.0.is_null() {
        return Err(failure("CreateCompatibleDC failed"));
    }
    let mut info: BITMAPINFO = unsafe { mem::zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: dib_width,
        biHeight: -dib_height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..info.bmiHeader
    };
    let mut bits = ptr::null_mut();
    let bitmap = Bitmap(unsafe {
        CreateDIBSection(
            screen.0,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            ptr::null_mut(),
            0,
        )
    });
    if bitmap.0.is_null() || bits.is_null() {
        return Err(failure("CreateDIBSection failed"));
    }
    let previous = unsafe { SelectObject(dc.0, bitmap.0) };
    if previous.is_null() {
        return Err(failure("SelectObject failed"));
    }
    let selection = BitmapSelection { dc, previous };
    if unsafe {
        BitBlt(
            selection.dc.0,
            0,
            0,
            dib_width,
            dib_height,
            screen.0,
            x,
            y,
            SRCCOPY | CAPTUREBLT,
        )
    } == 0
        || unsafe { GdiFlush() } == 0
    {
        return Err(failure("desktop BitBlt failed"));
    }

    let src = unsafe { slice::from_raw_parts(bits.cast::<u8>(), len) };
    let rgba = bgra_to_rgba_opaque(src);
    CapturedImage::from_rgba(width, height, rgba)
}

pub(super) fn primary_monitor_rect_win32() -> Option<(i32, i32, u32, u32)> {
    let width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    (width > 0 && height > 0).then_some((0, 0, width as u32, height as u32))
}

fn bgra_to_rgba_opaque(src: &[u8]) -> Vec<u8> {
    let len = src.len();
    let mut rgba: Vec<u8> = Vec::with_capacity(len);
    convert_bgra_stripe(src, &mut rgba.spare_capacity_mut()[..len]);
    unsafe {
        rgba.set_len(len);
    }
    rgba
}

#[inline]
fn convert_bgra_stripe(src: &[u8], dst: &mut [MaybeUninit<u8>]) {
    for (dst_px, src_px) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let bgra = u32::from_le_bytes([src_px[0], src_px[1], src_px[2], src_px[3]]);
        let rgba = (bgra & 0x0000_FF00)
            | ((bgra & 0x00FF_0000) >> 16)
            | ((bgra & 0x0000_00FF) << 16)
            | 0xFF00_0000;
        let bytes = rgba.to_le_bytes();
        dst_px[0].write(bytes[0]);
        dst_px[1].write(bytes[1]);
        dst_px[2].write(bytes[2]);
        dst_px[3].write(bytes[3]);
    }
}

fn virtual_screen() -> Option<ScreenRect> {
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    (width > 0 && height > 0).then(|| {
        let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        ScreenRect::new(x, y, width as u32, height as u32)
    })
}

fn has_visible_window_station() -> bool {
    let station = unsafe { GetProcessWindowStation() };
    let mut flags: USEROBJECTFLAGS = unsafe { mem::zeroed() };
    let mut needed = 0;
    !station.is_null()
        && unsafe {
            GetUserObjectInformationW(
                station,
                UOI_FLAGS,
                ptr::from_mut(&mut flags).cast(),
                mem::size_of::<USEROBJECTFLAGS>() as u32,
                &mut needed,
            )
        } != 0
        && flags.dwFlags & WSF_VISIBLE as u32 != 0
}

fn failure(message: &str) -> CoreError {
    CoreError::CaptureFailed(format!("GDI: {message}"))
}

struct ScreenDc(HDC);

impl Drop for ScreenDc {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ReleaseDC(ptr::null_mut(), self.0) };
        }
    }
}

struct MemoryDc(HDC);

impl Drop for MemoryDc {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DeleteDC(self.0) };
        }
    }
}

struct Bitmap(HGDIOBJ);

impl Drop for Bitmap {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DeleteObject(self.0) };
        }
    }
}

struct BitmapSelection {
    dc: MemoryDc,
    previous: HGDIOBJ,
}

impl Drop for BitmapSelection {
    fn drop(&mut self) {
        unsafe { SelectObject(self.dc.0, self.previous) };
    }
}

pub(super) fn list_snap_windows_win32() -> Vec<WindowInfo> {
    let Some(desktop) = virtual_screen() else {
        return Vec::new();
    };
    let mut enumeration = WindowEnumeration {
        desktop,
        process_id: unsafe { GetCurrentProcessId() },
        foreground: unsafe { GetForegroundWindow() },
        opaque_windows: Vec::new(),
        windows: Vec::new(),
    };
    unsafe {
        EnumWindows(
            Some(collect_snap_window),
            ptr::from_mut(&mut enumeration) as LPARAM,
        )
    };
    enumeration.windows
}

struct WindowEnumeration {
    desktop: ScreenRect,
    process_id: u32,
    foreground: HWND,
    opaque_windows: Vec<ScreenRect>,
    windows: Vec<WindowInfo>,
}

unsafe extern "system" fn collect_snap_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let enumeration = unsafe { &mut *(data as *mut WindowEnumeration) };
    enumeration.visit(hwnd);
    1
}

impl WindowEnumeration {
    fn visit(&mut self, hwnd: HWND) {
        if unsafe { IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 } {
            return;
        }
        let style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
        if style & WS_EX_TRANSPARENT != 0 || is_cloaked(hwnd) {
            return;
        }
        let layered = style & WS_EX_LAYERED != 0;
        let mut alpha = u8::MAX;
        let mut flags = 0;
        let known_layer = layered
            && unsafe { GetLayeredWindowAttributes(hwnd, ptr::null_mut(), &mut alpha, &mut flags) }
                != 0;
        if known_layer && flags & LWA_ALPHA != 0 && alpha == 0 {
            return;
        }

        let mut class = [0; 256];
        let class_len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
        let class = &class[..class_len as usize];
        let is_class = |name: &str| class.iter().copied().eq(name.encode_utf16());
        if is_class("Progman") || is_class("WorkerW") {
            return;
        }
        let mut rect: RECT = unsafe { mem::zeroed() };
        let has_frame = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                ptr::from_mut(&mut rect).cast(),
                mem::size_of::<RECT>() as u32,
            )
        } >= 0
            && rect.left < rect.right
            && rect.top < rect.bottom;
        if !has_frame
            && (is_class("Windows.UI.Core.CoreWindow")
                || unsafe { GetWindowRect(hwnd, &mut rect) } == 0)
        {
            return;
        }
        let bounds = ScreenRect {
            left: i64::from(rect.left),
            top: i64::from(rect.top),
            right: i64::from(rect.right),
            bottom: i64::from(rect.bottom),
        };
        let Some(bounds) = bounds.intersection(self.desktop) else {
            return;
        };
        if unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }.is_null()
            || self.opaque_windows.iter().any(|above| {
                above.left <= bounds.left
                    && above.top <= bounds.top
                    && above.right >= bounds.right
                    && above.bottom >= bounds.bottom
            })
        {
            return;
        }

        let mut region_bounds = rect;
        let region = unsafe { GetWindowRgnBox(hwnd, &mut region_bounds) };
        if region == NULLREGION {
            return;
        }
        if region == ERROR
            && style & WS_EX_NOREDIRECTIONBITMAP == 0
            && (!layered || (known_layer && flags == LWA_ALPHA && alpha == u8::MAX))
        {
            self.opaque_windows.push(bounds);
        }
        if style & WS_EX_TOOLWINDOW != 0 || bounds.width() < 12 || bounds.height() < 12 {
            return;
        }
        let mut pid = 0;
        if unsafe { GetWindowThreadProcessId(hwnd, &mut pid) } == 0 || pid == self.process_id {
            return;
        }
        let mut title = vec![0; unsafe { GetWindowTextLengthW(hwnd) }.saturating_add(1) as usize];
        let title_len = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
        let title = String::from_utf16_lossy(&title[..title_len as usize]);
        if title.contains("ShareX") || title.contains("VoltShot") {
            return;
        }
        self.windows.push(WindowInfo {
            id: WindowId::new(hwnd as usize as u32),
            title,
            app_name: String::new(),
            x: bounds.left as i32,
            y: bounds.top as i32,
            width: bounds.width(),
            height: bounds.height(),
            is_minimized: false,
            is_focused: hwnd == self.foreground,
        });
    }
}

fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            ptr::from_mut(&mut cloaked).cast(),
            mem::size_of::<u32>() as u32,
        )
    };
    result >= 0 && cloaked != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_to_rgba_opaque_swizzles_small_and_multithreaded_buffers() {
        let small = [10u8, 20, 30, 0, 100, 150, 200, 42];
        assert_eq!(
            bgra_to_rgba_opaque(&small),
            vec![30, 20, 10, 255, 200, 150, 100, 255]
        );

        let pixel_count = 100_000usize;
        let mut large = Vec::with_capacity(pixel_count * 4);
        for i in 0..pixel_count {
            large.extend_from_slice(&[
                (i % 251) as u8,
                ((i * 3) % 251) as u8,
                ((i * 7) % 251) as u8,
                0,
            ]);
        }
        let converted = bgra_to_rgba_opaque(&large);
        assert_eq!(converted.len(), large.len());
        for i in [0, 1, 12345, pixel_count / 2, pixel_count - 1] {
            let s = &large[i * 4..i * 4 + 4];
            let d = &converted[i * 4..i * 4 + 4];
            assert_eq!(d, &[s[2], s[1], s[0], 255]);
        }
    }
}
