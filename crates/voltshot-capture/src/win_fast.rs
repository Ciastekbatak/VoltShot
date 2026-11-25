use std::{
    cell::RefCell,
    mem::{self, MaybeUninit},
    ptr, slice,
};

use voltshot_core::{CapturedImage, CoreError, WindowId, WindowInfo};
use windows_sys::Win32::{
    Foundation::{BOOL, HWND, LPARAM, POINT, RECT},
    Graphics::{
        Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
        Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC,
            CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, ERROR, GdiFlush, GetDC,
            GetMonitorInfoW, GetWindowRgnBox, HDC, HGDIOBJ, MONITOR_DEFAULTTONULL,
            MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromRect, NULLREGION,
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
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, LWA_ALPHA, SM_CXVIRTUALSCREEN,
        SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WS_EX_LAYERED,
        WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WSF_VISIBLE,
    },
};

use super::{DesktopSnapshot, ScreenRect};

struct CachedDib {
    width: i32,
    height: i32,
    len: usize,
    dc: MemoryDc,
    bitmap: Bitmap,
    previous: HGDIOBJ,
    bits: *const u8,
}

impl Drop for CachedDib {
    fn drop(&mut self) {
        if !self.dc.0.is_null() && !self.previous.is_null() {
            unsafe {
                SelectObject(self.dc.0, self.previous);
            }
        }
    }
}

impl CachedDib {
    fn create(screen: HDC, width: i32, height: i32, len: usize) -> Result<Self, CoreError> {
        let dc = MemoryDc(unsafe { CreateCompatibleDC(screen) });
        if dc.0.is_null() {
            return Err(failure("CreateCompatibleDC failed"));
        }
        let mut info: BITMAPINFO = unsafe { mem::zeroed() };
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..info.bmiHeader
        };
        let mut bits = ptr::null_mut();
        let bitmap = Bitmap(unsafe {
            CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0)
        });
        if bitmap.0.is_null() || bits.is_null() {
            return Err(failure("CreateDIBSection failed"));
        }
        let previous = unsafe { SelectObject(dc.0, bitmap.0) };
        if previous.is_null() {
            return Err(failure("SelectObject failed"));
        }
        Ok(Self {
            width,
            height,
            len,
            dc,
            bitmap,
            previous,
            bits: bits.cast::<u8>(),
        })
    }

    fn matches(&self, width: i32, height: i32) -> bool {
        self.width == width && self.height == height && !self.bitmap.0.is_null()
    }
}

thread_local! {
    static THREAD_DIB: RefCell<Option<CachedDib>> = const { RefCell::new(None) };
}

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

pub(super) fn virtual_desktop_bounds_win32() -> Option<(i32, i32, u32, u32)> {
    let bounds = virtual_screen()?;
    Some((
        bounds.left as i32,
        bounds.top as i32,
        bounds.width(),
        bounds.height(),
    ))
}

pub(super) fn primary_monitor_rect_win32() -> Option<(i32, i32, u32, u32)> {
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    if monitor.is_null() {
        return None;
    }
    let mut info: MONITORINFO = unsafe { mem::zeroed() };
    info.cbSize = mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }
    let rect = info.rcMonitor;
    let width = u32::try_from(rect.right - rect.left).ok()?;
    let height = u32::try_from(rect.bottom - rect.top).ok()?;
    (width > 0 && height > 0).then_some((rect.left, rect.top, width, height))
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

    let rgba = THREAD_DIB.with(|cell| -> Result<Vec<u8>, CoreError> {
        let mut slot = cell.borrow_mut();
        let reused = slot
            .as_ref()
            .is_some_and(|dib| dib.matches(dib_width, dib_height));
        if !reused {
            *slot = Some(CachedDib::create(screen.0, dib_width, dib_height, len)?);
        }
        let blitted = {
            let dib = slot.as_ref().ok_or_else(|| failure("cached DIB missing"))?;
            unsafe {
                BitBlt(
                    dib.dc.0,
                    0,
                    0,
                    dib_width,
                    dib_height,
                    screen.0,
                    x,
                    y,
                    SRCCOPY | CAPTUREBLT,
                ) != 0
                    && GdiFlush() != 0
            }
        };
        if !blitted {
            if reused {
                *slot = Some(CachedDib::create(screen.0, dib_width, dib_height, len)?);
                let dib = slot.as_ref().ok_or_else(|| failure("cached DIB missing"))?;
                if unsafe {
                    BitBlt(
                        dib.dc.0,
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
            } else {
                return Err(failure("desktop BitBlt failed"));
            }
        }
        let dib = slot.as_ref().ok_or_else(|| failure("cached DIB missing"))?;
        let src = unsafe { slice::from_raw_parts(dib.bits, dib.len) };
        Ok(bgra_to_rgba_opaque(src))
    })?;

    CapturedImage::from_rgba(width, height, rgba)
}

fn bgra_to_rgba_opaque(src: &[u8]) -> Vec<u8> {
    let len = src.len();
    let mut rgba: Vec<u8> = Vec::with_capacity(len);
    let dst = &mut rgba.spare_capacity_mut()[..len];
    convert_bgra_to_rgba_opaque(src, dst);
    unsafe {
        rgba.set_len(len);
    }
    rgba
}

fn convert_bgra_to_rgba_opaque(src: &[u8], dst: &mut [MaybeUninit<u8>]) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            unsafe {
                convert_bgra_to_rgba_avx2(src, dst);
            }
            return;
        }
        if std::arch::is_x86_feature_detected!("ssse3") {
            unsafe {
                convert_bgra_to_rgba_ssse3(src, dst);
            }
            return;
        }
    }
    convert_bgra_to_rgba_scalar(src, dst);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn convert_bgra_to_rgba_avx2(src: &[u8], dst: &mut [MaybeUninit<u8>]) {
    use std::arch::x86_64::{
        __m256i, _mm256_loadu_si256, _mm256_or_si256, _mm256_set1_epi32, _mm256_setr_epi8,
        _mm256_shuffle_epi8, _mm256_storeu_si256,
    };

    let shuffle = _mm256_setr_epi8(
        2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15, 2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11,
        14, 13, 12, 15,
    );
    let alpha_mask = _mm256_set1_epi32(0xFF00_0000_u32 as i32);

    let len = src.len();
    let mut offset = 0usize;
    let src_ptr = src.as_ptr();
    let dst_ptr = dst.as_mut_ptr().cast::<u8>();

    while offset + 32 <= len {
        unsafe {
            let chunk = _mm256_loadu_si256(src_ptr.add(offset).cast::<__m256i>());
            let swapped = _mm256_shuffle_epi8(chunk, shuffle);
            let opaque = _mm256_or_si256(swapped, alpha_mask);
            _mm256_storeu_si256(dst_ptr.add(offset).cast::<__m256i>(), opaque);
        }
        offset += 32;
    }

    if offset < len {
        convert_bgra_to_rgba_scalar(&src[offset..], &mut dst[offset..]);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "ssse3")]
unsafe fn convert_bgra_to_rgba_ssse3(src: &[u8], dst: &mut [MaybeUninit<u8>]) {
    use std::arch::x86_64::{
        __m128i, _mm_loadu_si128, _mm_or_si128, _mm_set1_epi32, _mm_setr_epi8, _mm_shuffle_epi8,
        _mm_storeu_si128,
    };

    let shuffle = _mm_setr_epi8(2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15);
    let alpha_mask = _mm_set1_epi32(0xFF00_0000_u32 as i32);

    let len = src.len();
    let mut offset = 0usize;
    let src_ptr = src.as_ptr();
    let dst_ptr = dst.as_mut_ptr().cast::<u8>();

    while offset + 16 <= len {
        unsafe {
            let chunk = _mm_loadu_si128(src_ptr.add(offset).cast::<__m128i>());
            let swapped = _mm_shuffle_epi8(chunk, shuffle);
            let opaque = _mm_or_si128(swapped, alpha_mask);
            _mm_storeu_si128(dst_ptr.add(offset).cast::<__m128i>(), opaque);
        }
        offset += 16;
    }

    if offset < len {
        convert_bgra_to_rgba_scalar(&src[offset..], &mut dst[offset..]);
    }
}

fn convert_bgra_to_rgba_scalar(src: &[u8], dst: &mut [MaybeUninit<u8>]) {
    for (dst_px, src_px) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        dst_px[0].write(src_px[2]);
        dst_px[1].write(src_px[1]);
        dst_px[2].write(src_px[0]);
        dst_px[3].write(255);
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
        if title.contains("VoltShot") {
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
    fn simd_bgra_to_rgba_opaque_matches_scalar_for_all_tail_sizes() {
        for pixels in [1usize, 3, 4, 7, 8, 9, 15, 16, 17, 33, 64] {
            let mut src = Vec::with_capacity(pixels * 4);
            for i in 0..pixels {
                src.push((i.wrapping_mul(37) & 0xFF) as u8);
                src.push((i.wrapping_mul(73) & 0xFF) as u8);
                src.push((i.wrapping_mul(113) & 0xFF) as u8);
                src.push((i & 0xFF) as u8);
            }
            let fast = bgra_to_rgba_opaque(&src);
            let mut expected = Vec::with_capacity(src.len());
            for px in src.chunks_exact(4) {
                expected.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
            assert_eq!(fast, expected, "mismatch for {pixels} pixels");
        }
    }

    #[test]
    fn cached_dib_supports_repeated_gdi_captures_without_panicking() {
        let Ok(first) = capture_virtual_desktop_gdi() else {
            return;
        };
        let second = capture_virtual_desktop_gdi().expect("second GDI capture");
        let third = capture_virtual_desktop_gdi().expect("third GDI capture");
        assert_eq!(first.image.width(), second.image.width());
        assert_eq!(second.image.height(), third.image.height());
    }
}
