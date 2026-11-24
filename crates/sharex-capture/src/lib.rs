use sharex_core::{
    CaptureSource, CapturedImage, CoreError, MonitorId, MonitorInfo, Region, ScreenCapturer,
};
use xcap::{Monitor, Window};

/// Captured virtual desktop frame paired with its global top-left screen origin.
#[derive(Debug, Clone)]
pub struct DesktopSnapshot {
    pub image: CapturedImage,
    pub origin_x: i32,
    pub origin_y: i32,
}

/// Cross-platform screen capturer backed by `xcap`.
#[derive(Debug, Default, Clone, Copy)]
pub struct XcapCapturer;

impl XcapCapturer {
    pub const fn new() -> Self {
        Self
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

    fn capture_monitor(monitor: &Monitor) -> Result<CapturedImage, CoreError> {
        let buffer = monitor
            .capture_image()
            .map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        let width = buffer.width();
        let height = buffer.height();
        CapturedImage::from_rgba(width, height, buffer.into_raw())
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

        let buffer = target
            .capture_image()
            .map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        CapturedImage::from_rgba(buffer.width(), buffer.height(), buffer.into_raw())
    }

    /// Captures the primary display together with its virtual screen origin for the region overlay.
    pub fn capture_primary_snapshot(&self) -> Result<DesktopSnapshot, CoreError> {
        let monitors = Monitor::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        if monitors.is_empty() {
            return Err(CoreError::CaptureFailed("no monitors detected".into()));
        }
        let target = monitors
            .iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .unwrap_or(&monitors[0]);
        let info = Self::monitor_to_info(target)?;
        let image = Self::capture_monitor(target)?;
        Ok(DesktopSnapshot {
            image,
            origin_x: info.x,
            origin_y: info.y,
        })
    }

    /// Captures and stitches all connected monitors into one virtual desktop snapshot.
    pub fn capture_virtual_desktop(&self) -> Result<DesktopSnapshot, CoreError> {
        let monitors = Monitor::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        if monitors.is_empty() {
            return Err(CoreError::CaptureFailed("no monitors detected".into()));
        }
        let mut frames = Vec::with_capacity(monitors.len());
        for monitor in &monitors {
            let info = Self::monitor_to_info(monitor)?;
            let image = Self::capture_monitor(monitor)?;
            frames.push((info, image));
        }
        stitch_monitor_frames(&frames)
    }
}

impl ScreenCapturer for XcapCapturer {
    fn list_monitors(&self) -> Result<Vec<MonitorInfo>, CoreError> {
        let monitors = Monitor::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
        monitors.iter().map(Self::monitor_to_info).collect()
    }

    fn capture(&self, source: &CaptureSource) -> Result<CapturedImage, CoreError> {
        match source {
            CaptureSource::ActiveWindow => Self::capture_active_window(),
            CaptureSource::PrimaryMonitor => {
                let monitors =
                    Monitor::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
                if monitors.is_empty() {
                    return Err(CoreError::CaptureFailed("no monitors detected".into()));
                }
                let target = monitors
                    .iter()
                    .find(|m| m.is_primary().unwrap_or(false))
                    .unwrap_or(&monitors[0]);
                Self::capture_monitor(target)
            }
            CaptureSource::Monitor { id } => {
                let monitors =
                    Monitor::all().map_err(|e| CoreError::CaptureFailed(e.to_string()))?;
                let target = monitors
                    .iter()
                    .find(|m| m.id().ok() == Some(id.value()))
                    .ok_or_else(|| CoreError::CaptureFailed(format!("monitor `{id}` not found")))?;
                Self::capture_monitor(target)
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
                let (monitor_id, local_region) = resolve_region_on_monitors(*region, &infos)?;
                let target = monitors
                    .iter()
                    .find(|m| m.id().ok() == Some(monitor_id.value()))
                    .ok_or_else(|| {
                        CoreError::CaptureFailed(format!("monitor `{monitor_id}` disappeared"))
                    })?;
                let full = Self::capture_monitor(target)?;
                full.crop(local_region)
            }
        }
    }
}

/// Stitches per-monitor captures into a single virtual desktop RGBA image.
pub fn stitch_monitor_frames(
    frames: &[(MonitorInfo, CapturedImage)],
) -> Result<DesktopSnapshot, CoreError> {
    if frames.is_empty() {
        return Err(CoreError::CaptureFailed(
            "no monitor frames to stitch".into(),
        ));
    }
    let min_x = frames.iter().map(|(m, _)| m.x).min().unwrap_or(0);
    let min_y = frames.iter().map(|(m, _)| m.y).min().unwrap_or(0);
    let max_x = frames
        .iter()
        .map(|(m, img)| i64::from(m.x) + i64::from(img.width()))
        .max()
        .unwrap_or(0);
    let max_y = frames
        .iter()
        .map(|(m, img)| i64::from(m.y) + i64::from(img.height()))
        .max()
        .unwrap_or(0);

    let total_w = u32::try_from(max_x - i64::from(min_x))
        .map_err(|_| CoreError::CaptureFailed("invalid stitched width".into()))?;
    let total_h = u32::try_from(max_y - i64::from(min_y))
        .map_err(|_| CoreError::CaptureFailed("invalid stitched height".into()))?;

    let mut canvas = vec![0u8; (total_w as usize) * (total_h as usize) * 4];
    for (info, img) in frames {
        let dst_x = (info.x - min_x) as usize;
        let dst_y = (info.y - min_y) as usize;
        let src_w = img.width() as usize;
        let src_h = img.height() as usize;
        let src = img.rgba();
        for row in 0..src_h {
            let src_start = row * src_w * 4;
            let dst_start = ((dst_y + row) * (total_w as usize) + dst_x) * 4;
            canvas[dst_start..dst_start + src_w * 4]
                .copy_from_slice(&src[src_start..src_start + src_w * 4]);
        }
    }

    let image = CapturedImage::from_rgba(total_w, total_h, canvas)?;
    Ok(DesktopSnapshot {
        image,
        origin_x: min_x,
        origin_y: min_y,
    })
}

/// Maps a global screen region onto the best-matching monitor and returns monitor-local crop coordinates.
pub fn resolve_region_on_monitors(
    region: Region,
    monitors: &[MonitorInfo],
) -> Result<(MonitorId, Region), CoreError> {
    if monitors.is_empty() {
        return Err(CoreError::CaptureFailed("no monitors available".into()));
    }

    let rx1 = i64::from(region.x());
    let ry1 = i64::from(region.y());
    let rx2 = rx1 + i64::from(region.width());
    let ry2 = ry1 + i64::from(region.height());

    let best = monitors
        .iter()
        .filter_map(|monitor| {
            let mx1 = i64::from(monitor.x);
            let my1 = i64::from(monitor.y);
            let mx2 = mx1 + i64::from(monitor.width);
            let my2 = my1 + i64::from(monitor.height);

            let ix1 = rx1.max(mx1);
            let iy1 = ry1.max(my1);
            let ix2 = rx2.min(mx2);
            let iy2 = ry2.min(my2);

            if ix2 <= ix1 || iy2 <= iy1 {
                return None;
            }
            let area = (ix2 - ix1) * (iy2 - iy1);
            Some((monitor, ix1 - mx1, iy1 - my1, ix2 - ix1, iy2 - iy1, area))
        })
        .max_by_key(|entry| entry.5)
        .ok_or_else(|| {
            CoreError::InvalidRegion(format!(
                "region ({},{},{}x{}) does not intersect any monitor",
                region.x(),
                region.y(),
                region.width(),
                region.height()
            ))
        })?;

    let local = Region::new(best.1 as i32, best.2 as i32, best.3 as u32, best.4 as u32)?;
    Ok((best.0.id, local))
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
}
