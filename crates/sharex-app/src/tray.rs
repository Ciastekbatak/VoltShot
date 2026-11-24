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

    let mut rgba = Vec::with_capacity(size as usize * size as usize * 4);
    let pixel_width = 32.0 / f64::from(size);

    for row in 0..size {
        let y = ((f64::from(row) + 0.5) * pixel_width - 16.0).abs();
        for col in 0..size {
            let x = ((f64::from(col) + 0.5) * pixel_width - 16.0).abs();
            let distance_from_corner_center = (x - 8.0).max(0.0).hypot((y - 8.0).max(0.0));
            let coverage =
                (0.5 + (7.0 - distance_from_corner_center) / pixel_width).clamp(0.0, 1.0);
            let alpha = (coverage * 255.0).round() as u8;

            let bracket = (4.0..9.0).contains(&x) && (4.0..9.0).contains(&y) && x.max(y) >= 7.0;
            let lens = (3.0..5.0).contains(&x.hypot(y));
            let center = x < 1.0 && y < 1.0;
            let [r, g, b] = if bracket || lens || center {
                [48, 177, 255]
            } else {
                [25, 35, 51]
            };
            rgba.extend_from_slice(&[r, g, b, alpha]);
        }
    }

    rgba
}

pub struct TrayRuntime {
    actions: mpsc::Receiver<TrayAction>,
    init_error: Option<String>,
    #[cfg(target_os = "windows")]
    _native: Option<windows::NativeTray>,
}

impl TrayRuntime {
    /// On Windows, create and drop the runtime on the thread running the GUI event loop.
    pub fn new(ctx: egui::Context) -> Self {
        let (action_tx, actions) = mpsc::channel();
        #[cfg(target_os = "windows")]
        let (native, init_error) = match windows::NativeTray::new(ctx, action_tx) {
            Ok(native) => (Some(native), None),
            Err(error) => (None, Some(error)),
        };
        #[cfg(not(target_os = "windows"))]
        let init_error = {
            drop((ctx, action_tx));
            Some("The native ShareX system tray is available on Windows only".to_owned())
        };

        Self {
            actions,
            init_error,
            #[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
mod windows {
    use super::{TrayAction, TrayMenuMap, build_tray_icon_rgba, egui, mpsc};
    use std::sync::{Mutex, MutexGuard};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetShellWindow;

    const ICON_SIZE: u32 = 32;
    const POLL_INTERVAL: Duration = Duration::from_millis(16);
    static ACTIVE_TRAY: Mutex<()> = Mutex::new(());

    pub(super) struct NativeTray {
        _icon: TrayIcon,
        stop: mpsc::Sender<()>,
        worker: Option<JoinHandle<()>>,
        _active: MutexGuard<'static, ()>,
    }

    impl NativeTray {
        pub(super) fn new(
            ctx: egui::Context,
            action_tx: mpsc::Sender<TrayAction>,
        ) -> Result<Self, String> {
            if unsafe { GetShellWindow() }.is_null() {
                return Err("The Windows desktop shell is unavailable".to_owned());
            }

            let active = ACTIVE_TRAY
                .try_lock()
                .map_err(|_| "The ShareX system tray is already active".to_owned())?;
            let (menu, menu_map) = build_menu().map_err(|err| err.to_string())?;
            let icon = Icon::from_rgba(build_tray_icon_rgba(ICON_SIZE), ICON_SIZE, ICON_SIZE)
                .map_err(|err| err.to_string())?;
            let icon = TrayIconBuilder::new()
                .with_tooltip("ShareX")
                .with_icon(icon)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()
                .map_err(|err| err.to_string())?;

            let icon_id = icon.id().clone();
            let (stop, stop_rx) = mpsc::channel();
            let worker = thread::Builder::new()
                .name("sharex-tray-events".to_owned())
                .spawn(move || {
                    let menu_events = MenuEvent::receiver();
                    let icon_events = TrayIconEvent::receiver();
                    loop {
                        let menu_actions = menu_events
                            .try_iter()
                            .filter_map(|event| menu_map.action_for(event.id()));
                        let icon_actions = icon_events.try_iter().filter_map(|event| match event {
                            TrayIconEvent::DoubleClick {
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
                        }

                        if stop_rx.recv_timeout(POLL_INTERVAL)
                            != Err(mpsc::RecvTimeoutError::Timeout)
                        {
                            return;
                        }
                    }
                })
                .map_err(|err| err.to_string())?;

            Ok(Self {
                _icon: icon,
                stop,
                worker: Some(worker),
                _active: active,
            })
        }
    }

    impl Drop for NativeTray {
        fn drop(&mut self) {
            let _ = self.stop.send(());
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
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
            &[("Quit", TrayAction::Quit)],
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
        assert_eq!(pixel(16, 3), [25, 35, 51, 255]);
        assert_eq!(pixel(8, 8), [48, 177, 255, 255]);
        assert_eq!(pixel(16, 16), pixel(8, 8));
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
        #[cfg(target_os = "windows")]
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

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn unsupported_platform_reports_an_error_without_actions() {
        let tray = TrayRuntime::new(egui::Context::default());
        assert!(!tray.is_available());
        assert!(tray.init_error().is_some());
        assert!(tray.poll_actions().is_empty());
    }
}
