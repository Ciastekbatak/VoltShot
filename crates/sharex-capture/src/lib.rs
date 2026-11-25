use sharex_core::{
    CaptureSource, CapturedImage, CoreError, MonitorId, MonitorInfo, Region, ScreenCapturer,
    WindowId, WindowInfo,
};
use xcap::{Monitor, Window};

#[cfg(target_os = "windows")]
/// Single-frame Windows Desktop Duplication capture.
pub mod dxgi;
/// Vertical stitching for sequences of scrolling screenshots.
pub mod scrolling;
#[cfg(target_os = "windows")]
mod win_fast;

pub use scrolling::{ScrollStitchOptions, ScrollStitchResult, stitch_scrolling_frames};

/// Monitor capture backend. Window captures always use `xcap`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    /// Try GDI on Windows, falling back to `xcap` on error.
    #[default]
    Auto,
    /// Require Windows Desktop Duplication without a fallback.
    Dxgi,
    /// Use `xcap` directly on all platforms.
    Xcap,
}

/// Captured virtual desktop frame paired with its global top-left screen origin.
#[derive(Debug, Clone)]
pub struct DesktopSnapshot {
    pub image: CapturedImage,
    pub origin_x: i32,
    pub origin_y: i32,
}

impl DesktopSnapshot {
    fn crop_region(&self, region: Region) -> Result<CapturedImage, CoreError> {
        let bounds = ScreenRect::new(
            self.origin_x,
            self.origin_y,
            self.image.width(),
            self.image.height(),
        );
        let intersection = bounds
            .intersection(ScreenRect::from(region))
            .ok_or_else(|| {
                CoreError::InvalidRegion("region is outside the virtual desktop".into())
            })?;
        self.image.crop(bounds.local_region(intersection)?)
    }
}

/// Cross-platform screen capturer with an optional Windows Desktop Duplication backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct XcapCapturer {
    backend: CaptureBackend,
}

impl XcapCapturer {
    /// Creates a screen capturer with automatic backend selection.
    pub const fn new() -> Self {
        Self::with_backend(CaptureBackend::Auto)
    }

    /// Creates a screen capturer with the given monitor backend.
    pub const fn with_backend(backend: CaptureBackend) -> Self {
        Self { backend }
    }

    fn monitor_to_info(monitor: &Monitor) -> Result<MonitorInfo, CoreError> {
        let id = monitor
            .id()
            .map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        let name = monitor
            .name()
            .map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        let x = monitor
            .x()
            .map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        let y = monitor
            .y()
            .map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        let width = monitor
            .width()
            .map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        let height = monitor
            .height()
            .map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        let is_primary = monitor
            .is_primary()
            .map_err(|e| CoreError::CaptureFailed(e.to_string()))?;

        Ok(MonitorInfo {
            id: MonitorId::new(id),
            name,
            x,
            y,
            width,
            height,
            is_primary,
        })
    }

    fn capture_monitor(&self, monitor: &Monitor) -> Result<CapturedImage, CoreError> {
        if self.backend == CaptureBackend::Dxgi {
            return Self::capture_monitor_dxgi(monitor);
        }
        #[cfg(target_os = "windows")]
        if self.backend == CaptureBackend::Auto
            && let (Ok(x), Ok(y), Ok(width), Ok(height)) =
                (monitor.x(), monitor.y(), monitor.width(), monitor.height())
            && let Ok(image) = win_fast::capture_screen_area(x, y, width, height)
        {
            return Ok(image);
        }
        let buffer = monitor
            .capture_image()
            .map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        let width = buffer.width();
        let height = buffer.height();
        CapturedImage::from_rgba(width, height, buffer.into_raw())
    }

    #[cfg(target_os = "windows")]
    fn capture_monitor_dxgi(monitor: &Monitor) -> Result<CapturedImage, CoreError> {
        let name = monitor
            .name()
            .map_err(|error| CoreError::CaptureFailed(error.to_string()))?;
        dxgi::capture_named_monitor(&name)
    }

    #[cfg(not(target_os = "windows"))]
    fn capture_monitor_dxgi(_: &Monitor) -> Result<CapturedImage, CoreError> {
        Err(CoreError::CaptureFailed("DXGI requires Windows".into()))
    }

    fn primary_monitor(monitors: &[Monitor]) -> Result<&Monitor, CoreError> {
        monitors
            .iter()
            .find(|monitor| monitor.is_primary().unwrap_or(false))
            .or_else(|| monitors.first())
            .ok_or_else(|| CoreError::CaptureFailed("no monitors detected".into()))
    }

    fn window_to_info(window: &Window) -> Option<WindowInfo> {
        let width = window.width().ok()?;
        let height = window.height().ok()?;
        if width == 0 || height == 0 {
            return None;
        }

        Some(WindowInfo {
            id: WindowId::new(window.id().ok()?),
            x: window.x().ok()?,
            y: window.y().ok()?,
            width,
            height,
            title: window.title().unwrap_or_default(),
            app_name: window.app_name().unwrap_or_default(),
            is_minimized: window.is_minimized().unwrap_or(true),
            is_focused: window.is_focused().unwrap_or(false),
        })
    }

    fn capture_window(window: &Window) -> Result<CapturedImage, CoreError> {
        let buffer = window
            .capture_image()
            .map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        CapturedImage::from_rgba(buffer.width(), buffer.height(), buffer.into_raw())
    }

    fn capture_window_by_id(id: WindowId) -> Result<CapturedImage, CoreError> {
        let windows = Window::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        let target = windows
            .iter()
            .find(|window| window.id().ok() == Some(id.value()))
            .ok_or_else(|| CoreError::CaptureFailed(format!("window `{id}` not found")))?;
        Self::capture_window(target)
    }

    fn capture_active_window() -> Result<CapturedImage, CoreError> {
        let windows = Window::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        let target = windows
            .iter()
            .find(|w| {
                w.is_focused().unwrap_or(false)
                    && !w.is_minimized().unwrap_or(true)
                    && w.width().unwrap_or(0) > 0
                    && w.height().unwrap_or(0) > 0
            })
            .or_else(|| {
                windows.iter().find(|w| {
                    !w.is_minimized().unwrap_or(true)
                        && w.width().unwrap_or(0) > 0
                        && w.height().unwrap_or(0) > 0
                })
            })
            .ok_or_else(|| CoreError::CaptureFailed("no visible active window found".into()))?;

        Self::capture_window(target)
    }

    /// Captures the primary display together with its virtual screen origin for the region overlay.
    pub fn capture_primary_snapshot(&self) -> Result<DesktopSnapshot, CoreError> {
        let monitors = Monitor::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        self.capture_primary_from_monitors(&monitors)
    }

    fn capture_primary_from_monitors(
        &self,
        monitors: &[Monitor],
    ) -> Result<DesktopSnapshot, CoreError> {
        let target = Self::primary_monitor(monitors)?;
        let info = Self::monitor_to_info(target)?;
        let image = self.capture_monitor(target)?;
        if (image.width(), image.height()) != (info.width, info.height) {
            return stitch_monitor_frames(&[(info, image)]);
        }
        Ok(DesktopSnapshot {
            image,
            origin_x: info.x,
            origin_y: info.y,
        })
    }

    /// Captures all connected monitors into one virtual desktop snapshot.
    pub fn capture_virtual_desktop(&self) -> Result<DesktopSnapshot, CoreError> {
        #[cfg(target_os = "windows")]
        if self.backend == CaptureBackend::Auto
            && let Ok(snapshot) = win_fast::capture_virtual_desktop_gdi()
        {
            return Ok(snapshot);
        }
        let monitors = Monitor::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        if monitors.len() == 1 {
            return self.capture_primary_from_monitors(&monitors);
        }
        let infos = monitors
            .iter()
            .map(Self::monitor_to_info)
            .collect::<Result<Vec<_>, _>>()?;
        self.capture_virtual_desktop_from_monitors(monitors.iter().zip(infos))
    }

    /// Lists windows suitable for overlay snapping, in front-to-back order on Windows.
    pub fn list_snap_windows(&self) -> Result<Vec<WindowInfo>, CoreError> {
        #[cfg(target_os = "windows")]
        return Ok(win_fast::list_snap_windows_win32());

        #[cfg(not(target_os = "windows"))]
        self.list_windows()
    }

    fn capture_virtual_desktop_from_monitors<'a>(
        &self,
        monitors: impl ExactSizeIterator<Item = (&'a Monitor, MonitorInfo)>,
    ) -> Result<DesktopSnapshot, CoreError> {
        if monitors.len() == 0 {
            return Err(CoreError::CaptureFailed("no monitors detected".into()));
        }
        let mut frames = Vec::with_capacity(monitors.len());
        for (monitor, info) in monitors {
            frames.push((info, self.capture_monitor(monitor)?));
        }
        stitch_monitor_frames(&frames)
    }
}

#[cfg(target_os = "windows")]
fn cursor_position() -> Option<(i32, i32)> {
    use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

    let mut point = POINT { x: 0, y: 0 };
    let success = unsafe { GetCursorPos(&mut point) };
    (success != 0).then_some((point.x, point.y))
}

#[cfg(not(target_os = "windows"))]
fn cursor_position() -> Option<(i32, i32)> {
    None
}

impl ScreenCapturer for XcapCapturer {
    fn list_monitors(&self) -> Result<Vec<MonitorInfo>, CoreError> {
        let monitors = Monitor::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        monitors.iter().map(Self::monitor_to_info).collect()
    }

    fn list_windows(&self) -> Result<Vec<WindowInfo>, CoreError> {
        let windows = Window::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        Ok(windows.iter().filter_map(Self::window_to_info).collect())
    }

    fn capture(&self, source: &CaptureSource) -> Result<CapturedImage, CoreError> {
        match source {
            CaptureSource::WatchedFile => Err(CoreError::CaptureFailed(
                "watched files must be loaded from disk".into(),
            )),
            CaptureSource::ActiveWindow => Self::capture_active_window(),
            CaptureSource::Window { id } => Self::capture_window_by_id(*id),
            CaptureSource::PrimaryMonitor | CaptureSource::ActiveMonitor => {
                let monitors =
                    Monitor::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
                let position = match source {
                    CaptureSource::ActiveMonitor => cursor_position(),
                    _ => None,
                };
                if let Some((x, y)) = position {
                    let infos = monitors
                        .iter()
                        .map(Self::monitor_to_info)
                        .collect::<Result<Vec<_>, _>>()?;
                    let index = resolve_monitor_for_point(x, y, &infos)
                        .and_then(|id| infos.iter().position(|info| info.id == id));
                    if let Some(index) = index {
                        return self.capture_monitor(&monitors[index]);
                    }
                }
                self.capture_monitor(Self::primary_monitor(&monitors)?)
            }
            CaptureSource::Monitor { id } => {
                let monitors =
                    Monitor::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
                let target = monitors
                    .iter()
                    .find(|m| m.id().ok() == Some(id.value()))
                    .ok_or_else(|| CoreError::CaptureFailed(format!("monitor `{id}` not found")))?;
                self.capture_monitor(target)
            }
            CaptureSource::Region { region } => {
                let monitors =
                    Monitor::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
                if monitors.is_empty() {
                    return Err(CoreError::CaptureFailed("no monitors detected".into()));
                }
                let infos: Vec<MonitorInfo> = monitors
                    .iter()
                    .map(Self::monitor_to_info)
                    .collect::<Result<_, _>>()?;
                if region_across_monitors(*region, &infos) {
                    let snapshot =
                        self.capture_virtual_desktop_from_monitors(monitors.iter().zip(infos))?;
                    return snapshot.crop_region(*region);
                }
                let (monitor_id, local_region) = resolve_region_on_monitors(*region, &infos)?;
                let target = monitors
                    .iter()
                    .find(|m| m.id().ok() == Some(monitor_id.value()))
                    .ok_or_else(|| {
                        CoreError::CaptureFailed(format!("monitor `{monitor_id}` disappeared"))
                    })?;
                let full = self.capture_monitor(target)?;
                full.crop(local_region)
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ScreenRect {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl ScreenRect {
    fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        let left = i64::from(x);
        let top = i64::from(y);
        Self {
            left,
            top,
            right: left + i64::from(width),
            bottom: top + i64::from(height),
        }
    }

    fn contains_point(self, x: i32, y: i32) -> bool {
        (self.left..self.right).contains(&i64::from(x))
            && (self.top..self.bottom).contains(&i64::from(y))
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let overlap = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (overlap.left < overlap.right && overlap.top < overlap.bottom).then_some(overlap)
    }

    fn width(self) -> u32 {
        (self.right - self.left) as u32
    }

    fn height(self) -> u32 {
        (self.bottom - self.top) as u32
    }

    fn local_region(self, intersection: Self) -> Result<Region, CoreError> {
        let x = i32::try_from(intersection.left - self.left).map_err(|_| {
            CoreError::InvalidRegion("crop x offset exceeds coordinate range".into())
        })?;
        let y = i32::try_from(intersection.top - self.top).map_err(|_| {
            CoreError::InvalidRegion("crop y offset exceeds coordinate range".into())
        })?;
        Region::new(x, y, intersection.width(), intersection.height())
    }
}

impl From<Region> for ScreenRect {
    fn from(region: Region) -> Self {
        Self::new(region.x(), region.y(), region.width(), region.height())
    }
}

/// Returns the monitor at the point, or the primary monitor if the point is outside all displays.
pub fn resolve_monitor_for_point(x: i32, y: i32, monitors: &[MonitorInfo]) -> Option<MonitorId> {
    monitors
        .iter()
        .find(|monitor| {
            ScreenRect::new(monitor.x, monitor.y, monitor.width, monitor.height)
                .contains_point(x, y)
        })
        .or_else(|| monitors.iter().find(|monitor| monitor.is_primary))
        .map(|monitor| monitor.id)
}

/// Returns the first non-minimized window whose screen bounds contain the point.
pub fn find_window_at_point(x: i32, y: i32, windows: &[WindowInfo]) -> Option<&WindowInfo> {
    windows.iter().find(|window| {
        !window.is_minimized
            && ScreenRect::new(window.x, window.y, window.width, window.height).contains_point(x, y)
    })
}

/// Returns whether a region intersects at least two monitors with positive area.
pub fn region_across_monitors(region: Region, monitors: &[MonitorInfo]) -> bool {
    let bounds = ScreenRect::from(region);
    monitors
        .iter()
        .filter(|monitor| {
            bounds
                .intersection(ScreenRect::new(
                    monitor.x,
                    monitor.y,
                    monitor.width,
                    monitor.height,
                ))
                .is_some()
        })
        .nth(1)
        .is_some()
}

/// Stitches captures in monitor coordinates, resampling scaled frames and leaving gaps transparent.
pub fn stitch_monitor_frames(
    frames: &[(MonitorInfo, CapturedImage)],
) -> Result<DesktopSnapshot, CoreError> {
    if frames.is_empty() {
        return Err(CoreError::CaptureFailed(
            "no monitor frames to stitch".into(),
        ));
    }
    if frames
        .iter()
        .any(|(info, _)| info.width == 0 || info.height == 0)
    {
        return Err(CoreError::CaptureFailed(
            "monitor frame has empty desktop bounds".into(),
        ));
    }
    let min_x = frames.iter().map(|(m, _)| m.x).min().unwrap_or(0);
    let min_y = frames.iter().map(|(m, _)| m.y).min().unwrap_or(0);
    let max_x = frames
        .iter()
        .map(|(m, _)| i64::from(m.x) + i64::from(m.width))
        .max()
        .unwrap_or(0);
    let max_y = frames
        .iter()
        .map(|(m, _)| i64::from(m.y) + i64::from(m.height))
        .max()
        .unwrap_or(0);

    let total_w = u32::try_from(max_x - i64::from(min_x))
        .map_err(|_| CoreError::CaptureFailed("invalid stitched width".into()))?;
    let total_h = u32::try_from(max_y - i64::from(min_y))
        .map_err(|_| CoreError::CaptureFailed("invalid stitched height".into()))?;

    let canvas_len = (total_w as usize)
        .checked_mul(total_h as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| CoreError::CaptureFailed("stitched image is too large".into()))?;
    let mut canvas = Vec::new();
    canvas.try_reserve_exact(canvas_len).map_err(|error| {
        CoreError::CaptureFailed(format!("could not allocate stitched image: {error}"))
    })?;
    canvas.resize(canvas_len, 0);
    for (info, img) in frames {
        let dst_x = (i64::from(info.x) - i64::from(min_x)) as usize;
        let dst_y = (i64::from(info.y) - i64::from(min_y)) as usize;
        let src_w = img.width() as usize;
        let dst_w = info.width as usize;
        let src = img.rgba();
        for row in 0..info.height {
            let source_row = sample_coordinate(row, img.height(), info.height);
            let src_start = source_row * src_w * 4;
            let src_row = &src[src_start..src_start + src_w * 4];
            let row = row as usize;
            let dst_start = ((dst_y + row) * (total_w as usize) + dst_x) * 4;
            let dst_row = &mut canvas[dst_start..dst_start + dst_w * 4];
            if src_w == dst_w {
                dst_row.copy_from_slice(src_row);
            } else {
                for (column, pixel) in dst_row.chunks_exact_mut(4).enumerate() {
                    let offset = sample_coordinate(column as u32, img.width(), info.width) * 4;
                    pixel.copy_from_slice(&src_row[offset..offset + 4]);
                }
            }
        }
    }

    let image = CapturedImage::from_rgba(total_w, total_h, canvas)?;
    Ok(DesktopSnapshot {
        image,
        origin_x: min_x,
        origin_y: min_y,
    })
}

fn sample_coordinate(coordinate: u32, source_extent: u32, dest_extent: u32) -> usize {
    if source_extent == dest_extent {
        coordinate as usize
    } else {
        (u64::from(coordinate) * u64::from(source_extent) / u64::from(dest_extent)) as usize
    }
}

/// Maps a global screen region onto the best-matching monitor and returns monitor-local crop coordinates.
pub fn resolve_region_on_monitors(
    region: Region,
    monitors: &[MonitorInfo],
) -> Result<(MonitorId, Region), CoreError> {
    if monitors.is_empty() {
        return Err(CoreError::CaptureFailed("no monitors available".into()));
    }

    let bounds = ScreenRect::from(region);
    let (id, monitor_bounds, intersection) = monitors
        .iter()
        .filter_map(|monitor| {
            let monitor_bounds =
                ScreenRect::new(monitor.x, monitor.y, monitor.width, monitor.height);
            let intersection = bounds.intersection(monitor_bounds)?;
            Some((monitor.id, monitor_bounds, intersection))
        })
        .max_by_key(|(_, _, intersection)| {
            u64::from(intersection.width()) * u64::from(intersection.height())
        })
        .ok_or_else(|| {
            CoreError::InvalidRegion(format!(
                "region ({},{},{}x{}) does not intersect any monitor",
                region.x(),
                region.y(),
                region.width(),
                region.height()
            ))
        })?;

    Ok((id, monitor_bounds.local_region(intersection)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_monitors() -> Vec<MonitorInfo> {
        vec![
            MonitorInfo {
                id: MonitorId::new(1),
                name: "Primary".into(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                is_primary: true,
            },
            MonitorInfo {
                id: MonitorId::new(2),
                name: "Left".into(),
                x: -1280,
                y: 0,
                width: 1280,
                height: 720,
                is_primary: false,
            },
        ]
    }

    #[test]
    fn resolves_points_on_secondary_monitor_and_shared_edge() {
        let mut monitors = sample_monitors();
        monitors[1].y = -100;
        let (primary, left) = (&monitors[0], &monitors[1]);

        assert_eq!(
            resolve_monitor_for_point(-1280, -100, &monitors),
            Some(left.id)
        );
        assert_eq!(resolve_monitor_for_point(-1, 619, &monitors), Some(left.id));
        assert_eq!(resolve_monitor_for_point(0, 0, &monitors), Some(primary.id));
        assert_eq!(
            resolve_monitor_for_point(-1, 620, &monitors),
            Some(primary.id)
        );
    }

    #[test]
    fn falls_back_only_to_a_reported_primary_monitor() {
        let monitors = sample_monitors();
        assert_eq!(
            resolve_monitor_for_point(1920, 0, &monitors),
            Some(monitors[0].id)
        );
        assert_eq!(resolve_monitor_for_point(1920, 0, &monitors[1..]), None);
        assert_eq!(resolve_monitor_for_point(0, 0, &[]), None);

        let large = MonitorInfo {
            x: i32::MAX - 1,
            y: i32::MAX - 1,
            width: 4,
            height: 4,
            ..monitors[1].clone()
        };
        let id = large.id;
        assert_eq!(
            resolve_monitor_for_point(i32::MAX, i32::MAX, &[large]),
            Some(id)
        );
    }

    #[test]
    fn finds_first_containing_visible_window_in_list_order() {
        let front = WindowInfo {
            id: WindowId::new(1),
            title: "Front".into(),
            app_name: "Example".into(),
            x: -10,
            y: -20,
            width: 10,
            height: 10,
            is_minimized: false,
            is_focused: false,
        };
        let windows = [
            WindowInfo {
                id: WindowId::new(2),
                is_minimized: true,
                ..front.clone()
            },
            WindowInfo {
                id: WindowId::new(3),
                width: 0,
                ..front.clone()
            },
            front.clone(),
            WindowInfo {
                id: WindowId::new(4),
                is_focused: true,
                width: 20,
                height: 20,
                ..front
            },
        ];

        assert_eq!(find_window_at_point(-10, -20, &windows), Some(&windows[2]));
        assert_eq!(find_window_at_point(-1, -11, &windows), Some(&windows[2]));
        assert_eq!(find_window_at_point(0, -11, &windows), Some(&windows[3]));
        assert_eq!(find_window_at_point(-1, -10, &windows), Some(&windows[3]));
        assert_eq!(find_window_at_point(10, 0, &windows), None);
        assert_eq!(find_window_at_point(-10, -20, &windows[..2]), None);
        assert_eq!(find_window_at_point(-10, -20, &[]), None);
    }

    #[test]
    fn detects_positive_area_overlap_with_multiple_monitors() {
        let mut monitors = sample_monitors();
        let cases = [
            ((-1, 0, 2, 1), true),
            ((-1, 719, 2, 1), true),
            ((-1, 720, 2, 1), false),
            ((-100, 0, 100, 100), false),
            ((0, 0, 100, 100), false),
            ((-2000, -2000, 50, 50), false),
        ];
        for ((x, y, width, height), expected) in cases {
            let region = Region::new(x, y, width, height).expect("valid region");
            assert_eq!(
                region_across_monitors(region, &monitors),
                expected,
                "{region:?}"
            );
        }

        let seam = Region::new(-1, 0, 2, 1).expect("valid region");
        assert!(!region_across_monitors(seam, &[]));
        assert!(!region_across_monitors(seam, &monitors[..1]));
        monitors[1].width = 0;
        assert!(!region_across_monitors(seam, &monitors));
    }

    #[test]
    fn resolves_negative_coordinate_region_on_secondary_monitor() {
        let monitors = sample_monitors();
        let region = Region::new(-1000, 100, 300, 200).expect("valid region");
        let (monitor_id, local) =
            resolve_region_on_monitors(region, &monitors).expect("must intersect left monitor");

        assert_eq!(monitor_id, MonitorId::new(2));
        assert_eq!(local.x(), 280);
        assert_eq!(local.y(), 100);
        assert_eq!(local.width(), 300);
        assert_eq!(local.height(), 200);
    }

    #[test]
    fn clamps_region_extending_past_monitor_edge() {
        let monitors = sample_monitors();
        let region = Region::new(1800, 1000, 400, 200).expect("valid region");
        let (monitor_id, local) =
            resolve_region_on_monitors(region, &monitors).expect("clamps to primary");

        assert_eq!(monitor_id, MonitorId::new(1));
        assert_eq!(local.x(), 1800);
        assert_eq!(local.y(), 1000);
        assert_eq!(local.width(), 120);
        assert_eq!(local.height(), 80);
    }

    #[test]
    fn rejects_region_outside_all_monitors() {
        let monitors = sample_monitors();
        let region = Region::new(5000, 5000, 100, 100).expect("valid region");
        assert!(matches!(
            resolve_region_on_monitors(region, &monitors),
            Err(CoreError::InvalidRegion(_))
        ));
    }

    #[test]
    fn stitches_two_monitors_with_negative_left_origin() {
        let left_info = MonitorInfo {
            id: MonitorId::new(1),
            name: "Left".into(),
            x: -2,
            y: 0,
            width: 2,
            height: 2,
            is_primary: false,
        };
        let right_info = MonitorInfo {
            id: MonitorId::new(2),
            name: "Right".into(),
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            is_primary: true,
        };
        let left_img = CapturedImage::from_rgba(2, 2, [255, 0, 0, 255].repeat(4)).expect("left");
        let right_img = CapturedImage::from_rgba(2, 2, [0, 0, 255, 255].repeat(4)).expect("right");

        let snapshot = stitch_monitor_frames(&[(left_info, left_img), (right_info, right_img)])
            .expect("stitch succeeds");

        assert_eq!(snapshot.origin_x, -2);
        assert_eq!(snapshot.origin_y, 0);
        assert_eq!(snapshot.image.width(), 4);
        assert_eq!(snapshot.image.height(), 2);
        assert_eq!(&snapshot.image.rgba()[0..4], &[255, 0, 0, 255]);
        assert_eq!(&snapshot.image.rgba()[8..12], &[0, 0, 255, 255]);
    }

    #[test]
    fn crops_stitched_frames_in_global_coordinates_across_a_display_gap() {
        let monitors = [
            MonitorInfo {
                id: MonitorId::new(1),
                name: "Upper left".into(),
                x: -3,
                y: -2,
                width: 2,
                height: 2,
                is_primary: false,
            },
            MonitorInfo {
                id: MonitorId::new(2),
                name: "Primary".into(),
                x: 0,
                y: 0,
                width: 2,
                height: 2,
                is_primary: true,
            },
        ];
        let left = [
            [1, 0, 0, 255],
            [2, 0, 0, 255],
            [3, 0, 0, 255],
            [4, 0, 0, 255],
        ];
        let right = [
            [0, 0, 1, 255],
            [0, 0, 2, 255],
            [0, 0, 3, 255],
            [0, 0, 4, 255],
        ];
        let region = Region::new(-2, -1, 3, 3).expect("valid region");
        assert!(region_across_monitors(region, &monitors));

        let frames: Vec<_> = monitors
            .into_iter()
            .zip([left, right])
            .map(|(monitor, pixels)| {
                let image = CapturedImage::from_rgba(2, 2, pixels.concat()).expect("valid frame");
                (monitor, image)
            })
            .collect();
        let snapshot = stitch_monitor_frames(&frames).expect("stitch succeeds");
        let crop = snapshot.crop_region(region).expect("crop succeeds");
        let clear = [0, 0, 0, 0];
        let expected = [
            left[3], clear, clear, clear, clear, right[0], clear, clear, right[2],
        ];

        assert_eq!((snapshot.origin_x, snapshot.origin_y), (-3, -2));
        assert_eq!((crop.width(), crop.height()), (3, 3));
        assert_eq!(crop.rgba(), expected.concat());

        let beyond_edges = Region::new(-4, -3, 7, 6).expect("valid region");
        assert_eq!(
            snapshot
                .crop_region(beyond_edges)
                .expect("crop clips to desktop"),
            snapshot.image
        );
        let outside = Region::new(2, 0, 1, 1).expect("valid region");
        assert!(matches!(
            snapshot.crop_region(outside),
            Err(CoreError::InvalidRegion(_))
        ));
    }

    #[test]
    fn stitches_different_heights_and_scales_frames_to_desktop_bounds() {
        let short = MonitorInfo {
            id: MonitorId::new(1),
            name: "Short".into(),
            x: -2,
            y: -1,
            width: 2,
            height: 1,
            is_primary: false,
        };
        let tall = MonitorInfo {
            id: MonitorId::new(2),
            name: "Tall".into(),
            x: 0,
            y: -1,
            width: 1,
            height: 3,
            is_primary: true,
        };
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        let first = [0, 0, 1, 255];
        let second = [0, 0, 2, 255];
        let third = [0, 0, 3, 255];
        let clear = [0; 4];
        let short_image = CapturedImage::from_rgba(
            4,
            2,
            [red, clear, green, clear, clear, clear, clear, clear].concat(),
        )
        .expect("valid scaled capture");
        let tall_image =
            CapturedImage::from_rgba(1, 3, [first, second, third].concat()).expect("valid capture");

        let snapshot = stitch_monitor_frames(&[(short, short_image), (tall, tall_image)])
            .expect("stitch succeeds");
        assert_eq!((snapshot.origin_x, snapshot.origin_y), (-2, -1));
        assert_eq!((snapshot.image.width(), snapshot.image.height()), (3, 3));
        assert_eq!(
            snapshot.image.rgba(),
            [red, green, first, clear, clear, second, clear, clear, third].concat()
        );
    }

    #[test]
    fn rejects_empty_desktop_frames() {
        assert!(matches!(
            stitch_monitor_frames(&[]),
            Err(CoreError::CaptureFailed(_))
        ));
    }
}
