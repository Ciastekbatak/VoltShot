use eframe::egui;
use std::collections::HashMap;
use std::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    CaptureFullscreen,
    CaptureRegion,
    CaptureActiveWindow,
    OpenMainWindow,
    OpenScreenshotsFolder,
    Quit,
}

#[derive(Default)]
pub struct TrayMenuMap {
    actions: HashMap<String, TrayAction>,
}

impl TrayMenuMap {
    pub fn insert(&mut self, id: impl AsRef<str>, action: TrayAction) {
        self.actions.insert(id.as_ref().to_owned(), action);
    }

    pub fn action_for(&self, id: impl AsRef<str>) -> Option<TrayAction> {
        self.actions.get(id.as_ref()).copied()
    }
}

pub fn build_tray_icon_rgba(size: u32) -> Vec<u8> {
    if size == 0 {
        return Vec::new();
    }

    const BOLT: [(f64, f64); 6] = [
        (70.0, 28.0),
        (43.0, 68.0),
        (61.0, 68.0),
        (56.0, 100.0),
        (85.0, 58.0),
        (67.0, 58.0),
    ];
    const SS: usize = 3;

    let mut rgba = Vec::with_capacity(size as usize * size as usize * 4);
    let scale = 128.0 / f64::from(size);
    let inv_ss = 1.0 / (SS as f64);
    let inv_n = 1.0 / ((SS * SS) as f64);

    for row in 0..size {
        for col in 0..size {
            let mut pr = 0.0_f64;
            let mut pg = 0.0_f64;
            let mut pb = 0.0_f64;
            let mut pa = 0.0_f64;

            for sy in 0..SS {
                let y = (f64::from(row) + (sy as f64 + 0.5) * inv_ss) * scale;
                for sx in 0..SS {
                    let x = (f64::from(col) + (sx as f64 + 0.5) * inv_ss) * scale;
                    let (r, g, b, a) = sample_voltshot_mark(x, y, &BOLT);
                    pr += r * a;
                    pg += g * a;
                    pb += b * a;
                    pa += a;
                }
            }

            pa *= inv_n;
            if pa <= 1e-6 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let r = ((pr * inv_n) / pa).clamp(0.0, 255.0).round() as u8;
                let g = ((pg * inv_n) / pa).clamp(0.0, 255.0).round() as u8;
                let b = ((pb * inv_n) / pa).clamp(0.0, 255.0).round() as u8;
                let a = (pa * 255.0).clamp(0.0, 255.0).round() as u8;
                rgba.extend_from_slice(&[r, g, b, a]);
            }
        }
    }

    rgba
}

fn sample_voltshot_mark(x: f64, y: f64, bolt: &[(f64, f64); 6]) -> (f64, f64, f64, f64) {
    let qx = (x - 64.0).abs() - 32.0;
    let qy = (y - 64.0).abs() - 32.0;
    let sd_rect = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - 28.0;

    let mut pixel = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    if sd_rect <= 0.0 {
        let t = ((x - 4.0) + (y - 4.0)) / 240.0;
        let (r, g, b) = lerp_rgb((15.0, 23.0, 42.0), (23.0, 37.0, 84.0), t);
        pixel = (r, g, b, 1.0);
    }
    if sd_rect.abs() <= 1.5 {
        pixel = blend_over(pixel, (56.0, 189.0, 248.0, 0.32));
    }

    let fx = 64.0 - (x - 64.0).abs();
    let fy = 64.0 - (y - 64.0).abs();
    let d_v = dist_to_segment(fx, fy, 26.0, 34.0, 26.0, 46.0);
    let d_h = dist_to_segment(fx, fy, 34.0, 26.0, 46.0, 26.0);
    let d_arc = if fx <= 34.0 && fy <= 34.0 {
        ((fx - 34.0).hypot(fy - 34.0) - 8.0).abs()
    } else {
        (fx - 26.0)
            .hypot(fy - 34.0)
            .min((fx - 34.0).hypot(fy - 26.0))
    };
    if d_v.min(d_h).min(d_arc) <= 3.5 {
        let t = ((x - 26.0) + (y - 26.0)) / 152.0;
        let (r, g, b) = lerp_rgb((125.0, 211.0, 252.0), (56.0, 189.0, 248.0), t);
        pixel = blend_over(pixel, (r, g, b, 1.0));
    }

    if point_in_polygon(x, y, bolt) {
        let t = ((x - 43.0) + (y - 28.0)) / 114.0;
        let (r, g, b) = lerp_rgb((56.0, 189.0, 248.0), (2.0, 132.0, 199.0), t);
        pixel = blend_over(pixel, (r, g, b, 1.0));
    }

    pixel
}

fn lerp_rgb(c0: (f64, f64, f64), c1: (f64, f64, f64), t: f64) -> (f64, f64, f64) {
    let t = t.clamp(0.0, 1.0);
    (
        c0.0 + (c1.0 - c0.0) * t,
        c0.1 + (c1.1 - c0.1) * t,
        c0.2 + (c1.2 - c0.2) * t,
    )
}

fn blend_over(dst: (f64, f64, f64, f64), src: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let inv = 1.0 - src.3;
    let out_a = src.3 + dst.3 * inv;
    if out_a <= 1e-9 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    (
        (src.0 * src.3 + dst.0 * dst.3 * inv) / out_a,
        (src.1 * src.3 + dst.1 * dst.3 * inv) / out_a,
        (src.2 * src.3 + dst.2 * dst.3 * inv) / out_a,
        out_a,
    )
}

fn dist_to_segment(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let vx = bx - ax;
    let vy = by - ay;
    let wx = px - ax;
    let wy = py - ay;
    let c1 = vx * wx + vy * wy;
    if c1 <= 0.0 {
        return (px - ax).hypot(py - ay);
    }
    let c2 = vx * vx + vy * vy;
    if c2 <= c1 {
        return (px - bx).hypot(py - by);
    }
    let t = c1 / c2;
    (px - (ax + t * vx)).hypot(py - (ay + t * vy))
}

fn point_in_polygon(x: f64, y: f64, poly: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut prev = *poly.last().expect("non-empty polygon");
    for &curr in poly {
        if (prev.1 > y) != (curr.1 > y) {
            let x_int = prev.0 + (y - prev.1) * (curr.0 - prev.0) / (curr.1 - prev.1);
            if x < x_int {
                inside = !inside;
            }
        }
        prev = curr;
    }
    inside
}

pub struct TrayRuntime {
    actions: mpsc::Receiver<TrayAction>,
    init_error: Option<String>,
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    _native: Option<native::NativeTray>,
}

impl TrayRuntime {
    /// Creates the OS-native system tray runtime on the GUI thread.
    pub fn new(ctx: egui::Context) -> Self {
        let (action_tx, actions) = mpsc::channel();
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        let (native, init_error) = match native::NativeTray::new(ctx, action_tx) {
            Ok(native) => (Some(native), None),
            Err(error) => (None, Some(error)),
        };
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        let init_error = {
            drop((ctx, action_tx));
            Some("The native VoltShot system tray is unavailable on this platform".to_owned())
        };

        Self {
            actions,
            init_error,
            #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
            _native: native,
        }
    }

    pub fn poll_actions(&self) -> Vec<TrayAction> {
        self.actions.try_iter().collect()
    }

    pub fn is_available(&self) -> bool {
        self.init_error.is_none()
    }

    pub fn init_error(&self) -> Option<&str> {
        self.init_error.as_deref()
    }
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
mod native {
    use super::{TrayAction, TrayMenuMap, build_tray_icon_rgba, egui, mpsc};
    use std::sync::{Mutex, MutexGuard};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    const ICON_SIZE: u32 = 32;
    const POLL_INTERVAL: Duration = Duration::from_millis(16);
    static ACTIVE_TRAY: Mutex<()> = Mutex::new(());

    pub(super) struct NativeTray {
        #[cfg(not(target_os = "linux"))]
        _icon: TrayIcon,
        stop: mpsc::Sender<()>,
        worker: Option<JoinHandle<()>>,
        #[cfg(target_os = "linux")]
        gtk_thread: Option<JoinHandle<()>>,
        _active: MutexGuard<'static, ()>,
    }

    impl NativeTray {
        pub(super) fn new(
            ctx: egui::Context,
            action_tx: mpsc::Sender<TrayAction>,
        ) -> Result<Self, String> {
            #[cfg(target_os = "windows")]
            if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetShellWindow() }.is_null() {
                return Err("The Windows desktop shell is unavailable".to_owned());
            }

            #[cfg(target_os = "linux")]
            if std::env::var_os("DISPLAY").is_none()
                && std::env::var_os("WAYLAND_DISPLAY").is_none()
            {
                return Err("No Linux display server is active for the system tray".to_owned());
            }

            let active = ACTIVE_TRAY
                .try_lock()
                .map_err(|_| "The VoltShot system tray is already active".to_owned())?;

            #[cfg(target_os = "linux")]
            {
                let (init_tx, init_rx) =
                    mpsc::channel::<Result<(TrayMenuMap, tray_icon::TrayIconId), String>>();
                let gtk_thread = thread::Builder::new()
                    .name("voltshot-linux-tray".to_owned())
                    .spawn(move || {
                        if let Err(err) = gtk::init() {
                            let _ = init_tx.send(Err(format!("GTK init failed: {err}")));
                            return;
                        }
                        let created = (|| -> Result<(TrayIcon, TrayMenuMap), String> {
                            let (menu, menu_map) = build_menu().map_err(|err| err.to_string())?;
                            let icon = Icon::from_rgba(
                                build_tray_icon_rgba(ICON_SIZE),
                                ICON_SIZE,
                                ICON_SIZE,
                            )
                            .map_err(|err| err.to_string())?;
                            let tray = TrayIconBuilder::new()
                                .with_tooltip("VoltShot")
                                .with_icon(icon)
                                .with_menu(Box::new(menu))
                                .build()
                                .map_err(|err| err.to_string())?;
                            Ok((tray, menu_map))
                        })();
                        match created {
                            Ok((tray, menu_map)) => {
                                let icon_id = tray.id().clone();
                                let _ = init_tx.send(Ok((menu_map, icon_id)));
                                gtk::main();
                                drop(tray);
                            }
                            Err(err) => {
                                let _ = init_tx.send(Err(err));
                            }
                        }
                    })
                    .map_err(|err| err.to_string())?;

                let (menu_map, icon_id) = init_rx
                    .recv()
                    .map_err(|_| "Linux tray thread exited during initialization".to_owned())??;

                let (stop, worker) = spawn_event_worker(ctx, action_tx, menu_map, icon_id)?;
                Ok(Self {
                    stop,
                    worker: Some(worker),
                    gtk_thread: Some(gtk_thread),
                    _active: active,
                })
            }

            #[cfg(not(target_os = "linux"))]
            {
                let (menu, menu_map) = build_menu().map_err(|err| err.to_string())?;
                let icon = Icon::from_rgba(build_tray_icon_rgba(ICON_SIZE), ICON_SIZE, ICON_SIZE)
                    .map_err(|err| err.to_string())?;
                let builder = TrayIconBuilder::new()
                    .with_tooltip("VoltShot")
                    .with_icon(icon)
                    .with_menu(Box::new(menu));
                #[cfg(target_os = "windows")]
                let builder = builder.with_menu_on_left_click(false);
                let icon = builder.build().map_err(|err| err.to_string())?;
                let icon_id = icon.id().clone();
                let (stop, worker) = spawn_event_worker(ctx, action_tx, menu_map, icon_id)?;

                Ok(Self {
                    _icon: icon,
                    stop,
                    worker: Some(worker),
                    _active: active,
                })
            }
        }
    }

    fn spawn_event_worker(
        ctx: egui::Context,
        action_tx: mpsc::Sender<TrayAction>,
        menu_map: TrayMenuMap,
        icon_id: tray_icon::TrayIconId,
    ) -> Result<(mpsc::Sender<()>, JoinHandle<()>), String> {
        let (stop, stop_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("voltshot-tray-events".to_owned())
            .spawn(move || {
                let menu_events = MenuEvent::receiver();
                let icon_events = TrayIconEvent::receiver();
                loop {
                    let menu_actions = menu_events
                        .try_iter()
                        .filter_map(|event| menu_map.action_for(event.id()));
                    let icon_actions = icon_events.try_iter().filter_map(|event| match event {
                        TrayIconEvent::Click {
                            id,
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                        | TrayIconEvent::DoubleClick {
                            id,
                            button: MouseButton::Left,
                            ..
                        } if id == icon_id => Some(TrayAction::OpenMainWindow),
                        _ => None,
                    });

                    for action in menu_actions.chain(icon_actions) {
                        if action_tx.send(action).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                        crate::win_focus::wake_hidden_main_window();
                    }

                    if stop_rx.recv_timeout(POLL_INTERVAL) != Err(mpsc::RecvTimeoutError::Timeout) {
                        return;
                    }
                }
            })
            .map_err(|err| err.to_string())?;
        Ok((stop, worker))
    }

    impl Drop for NativeTray {
        fn drop(&mut self) {
            let _ = self.stop.send(());
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            #[cfg(target_os = "linux")]
            {
                gtk::glib::idle_add(|| {
                    gtk::main_quit();
                    gtk::glib::ControlFlow::Break
                });
                if let Some(gtk_thread) = self.gtk_thread.take() {
                    let _ = gtk_thread.join();
                }
            }
        }
    }

    fn build_menu() -> tray_icon::menu::Result<(Menu, TrayMenuMap)> {
        let menu = Menu::new();
        let mut map = TrayMenuMap::default();
        let groups = [
            &[
                ("Fullscreen", TrayAction::CaptureFullscreen),
                ("Region", TrayAction::CaptureRegion),
                ("Active Window", TrayAction::CaptureActiveWindow),
            ][..],
            &[
                ("Open Main Window", TrayAction::OpenMainWindow),
                ("Open Screenshots Folder", TrayAction::OpenScreenshotsFolder),
            ],
            &[("Quit VoltShot", TrayAction::Quit)],
        ];
        for (index, group) in groups.into_iter().enumerate() {
            if index != 0 {
                menu.append(&PredefinedMenuItem::separator())?;
            }
            for &(label, action) in group {
                let item = MenuItem::new(label, true, None);
                menu.append(&item)?;
                map.insert(item.id(), action);
            }
        }
        Ok((menu, map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icon_has_transparent_corners_and_a_blue_emblem() {
        let rgba = build_tray_icon_rgba(32);
        assert_eq!(rgba.len(), 32 * 32 * 4);

        let pixel = |x: usize, y: usize| {
            let offset = (y * 32 + x) * 4;
            &rgba[offset..offset + 4]
        };
        assert_eq!(pixel(0, 0)[3], 0);

        let bg = pixel(16, 3);
        assert_eq!(bg[3], 255);
        assert!(bg[0] < 35 && bg[1] < 50 && bg[2] < 95);

        let bracket = pixel(7, 7);
        assert_eq!(bracket[3], 255);
        assert!(bracket[2] > 200 && bracket[1] > 170);

        let bolt = pixel(16, 16);
        assert_eq!(bolt[3], 255);
        assert!(bolt[2] > 190 && bolt[1] > 140);
    }

    #[test]
    fn menu_ids_round_trip_all_tray_actions() {
        let entries = [
            ("capture-fullscreen", TrayAction::CaptureFullscreen),
            ("capture-region", TrayAction::CaptureRegion),
            ("capture-active-window", TrayAction::CaptureActiveWindow),
            ("open-main-window", TrayAction::OpenMainWindow),
            ("open-screenshots-folder", TrayAction::OpenScreenshotsFolder),
            ("quit", TrayAction::Quit),
        ];
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        let entries = entries.map(|(id, action)| (tray_icon::menu::MenuId::new(id), action));

        let mut map = TrayMenuMap::default();
        for (id, action) in &entries {
            map.insert(id, *action);
        }
        for (id, action) in entries {
            assert_eq!(map.action_for(id), Some(action));
        }
        assert_eq!(map.action_for("unrelated-action"), None);
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    #[test]
    fn unsupported_platform_reports_an_error_without_actions() {
        let tray = TrayRuntime::new(egui::Context::default());
        assert!(!tray.is_available());
        assert!(tray.init_error().is_some());
        assert!(tray.poll_actions().is_empty());
    }
}
