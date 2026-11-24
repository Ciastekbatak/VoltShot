/// Live OS registration status for a configured hotkey binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyStatus {
    Active,
    Disabled,
    Unbound,
    Error(String),
}

#[cfg(target_os = "windows")]
mod platform {
    use super::HotkeyStatus;
    use eframe::egui;
    use sharex_core::{HotkeyAction, HotkeyBinding, HotkeyCombo, KeyCode, KeyModifiers};
    use std::collections::HashMap;
    use std::ptr;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use windows_sys::Win32::Foundation::ERROR_HOTKEY_ALREADY_REGISTERED;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
        VK_0, VK_1, VK_2, VK_3, VK_4, VK_5, VK_6, VK_7, VK_8, VK_9, VK_A, VK_B, VK_C, VK_D,
        VK_DELETE, VK_E, VK_END, VK_F, VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8,
        VK_F9, VK_F10, VK_F11, VK_F12, VK_G, VK_H, VK_HOME, VK_I, VK_INSERT, VK_J, VK_K, VK_L,
        VK_M, VK_N, VK_NEXT, VK_O, VK_P, VK_PRIOR, VK_Q, VK_R, VK_S, VK_SNAPSHOT, VK_T, VK_U, VK_V,
        VK_W, VK_X, VK_Y, VK_Z,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY, WM_QUIT,
    };

    const WM_SYNC_HOTKEYS: u32 = WM_APP + 1;

    pub struct HotkeyRuntime {
        thread_id: u32,
        desired: Arc<Mutex<Vec<HotkeyBinding>>>,
        statuses: Arc<Mutex<HashMap<String, HotkeyStatus>>>,
        action_rx: Receiver<HotkeyAction>,
    }

    impl HotkeyRuntime {
        pub fn new(ctx: egui::Context, bindings: &[HotkeyBinding]) -> Self {
            let desired = Arc::new(Mutex::new(bindings.to_vec()));
            let statuses = Arc::new(Mutex::new(HashMap::new()));
            let (action_tx, action_rx) = mpsc::channel();
            let (ready_tx, ready_rx) = mpsc::channel();

            let desired_bg = Arc::clone(&desired);
            let statuses_bg = Arc::clone(&statuses);

            thread::spawn(move || {
                run_win32_hotkey_loop(ctx, desired_bg, statuses_bg, action_tx, ready_tx);
            });

            let thread_id = ready_rx.recv().unwrap_or(0);
            Self {
                thread_id,
                desired,
                statuses,
                action_rx,
            }
        }

        pub fn sync(&mut self, bindings: &[HotkeyBinding]) {
            if let Ok(mut guard) = self.desired.lock() {
                *guard = bindings.to_vec();
            }
            if self.thread_id != 0 {
                unsafe {
                    PostThreadMessageW(self.thread_id, WM_SYNC_HOTKEYS, 0, 0);
                }
            }
        }

        pub fn retry_conflicts(&mut self, bindings: &[HotkeyBinding]) {
            let has_error = self
                .statuses
                .lock()
                .map(|map| map.values().any(|s| matches!(s, HotkeyStatus::Error(_))))
                .unwrap_or(false);
            if has_error {
                self.sync(bindings);
            }
        }

        pub fn poll_actions(&self) -> Vec<HotkeyAction> {
            let mut actions = Vec::new();
            while let Ok(action) = self.action_rx.try_recv() {
                actions.push(action);
            }
            actions
        }

        pub fn status_for(&self, id: &str) -> HotkeyStatus {
            self.statuses
                .lock()
                .ok()
                .and_then(|map| map.get(id).cloned())
                .unwrap_or(HotkeyStatus::Disabled)
        }

        pub fn active_count(&self) -> usize {
            self.statuses
                .lock()
                .map(|map| {
                    map.values()
                        .filter(|s| matches!(s, HotkeyStatus::Active))
                        .count()
                })
                .unwrap_or(0)
        }
    }

    impl Drop for HotkeyRuntime {
        fn drop(&mut self) {
            if self.thread_id != 0 {
                unsafe {
                    PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
                }
            }
        }
    }

    fn run_win32_hotkey_loop(
        ctx: egui::Context,
        desired: Arc<Mutex<Vec<HotkeyBinding>>>,
        statuses: Arc<Mutex<HashMap<String, HotkeyStatus>>>,
        action_tx: Sender<HotkeyAction>,
        ready_tx: Sender<u32>,
    ) {
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        unsafe {
            PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_NOREMOVE);
        }
        let tid = unsafe { GetCurrentThreadId() };

        let mut registered: HashMap<i32, HotkeyAction> = HashMap::new();
        apply_win32_bindings(&desired, &statuses, &mut registered);
        let _ = ready_tx.send(tid);

        loop {
            let ret = unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) };
            if ret <= 0 {
                break;
            }
            match msg.message {
                WM_HOTKEY => {
                    let id = msg.wParam as i32;
                    if let Some(&action) = registered.get(&id) {
                        let _ = action_tx.send(action);
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.request_repaint();
                    }
                }
                WM_SYNC_HOTKEYS => {
                    apply_win32_bindings(&desired, &statuses, &mut registered);
                    ctx.request_repaint();
                }
                _ => {}
            }
        }

        for id in registered.keys() {
            unsafe {
                UnregisterHotKey(ptr::null_mut(), *id);
            }
        }
    }

    fn apply_win32_bindings(
        desired: &Arc<Mutex<Vec<HotkeyBinding>>>,
        statuses: &Arc<Mutex<HashMap<String, HotkeyStatus>>>,
        registered: &mut HashMap<i32, HotkeyAction>,
    ) {
        for id in registered.keys() {
            unsafe {
                UnregisterHotKey(ptr::null_mut(), *id);
            }
        }
        registered.clear();

        let bindings = desired.lock().map(|g| g.clone()).unwrap_or_default();
        let mut next_statuses = HashMap::with_capacity(bindings.len());

        for (index, binding) in bindings.iter().enumerate() {
            if !binding.enabled {
                next_statuses.insert(binding.id.clone(), HotkeyStatus::Disabled);
                continue;
            }
            let Some(combo) = binding.combo else {
                next_statuses.insert(binding.id.clone(), HotkeyStatus::Unbound);
                continue;
            };

            let win_id = (index as i32) + 1;
            let (mods, vk) = to_win32_hotkey(combo);
            let ok = unsafe { RegisterHotKey(ptr::null_mut(), win_id, mods, vk) };
            if ok != 0 {
                registered.insert(win_id, binding.action);
                next_statuses.insert(binding.id.clone(), HotkeyStatus::Active);
            } else {
                let err = std::io::Error::last_os_error();
                let message = if err.raw_os_error() == Some(ERROR_HOTKEY_ALREADY_REGISTERED as i32)
                {
                    "Already claimed by another app (e.g. ShareX or Snipping Tool)".to_owned()
                } else {
                    err.to_string()
                };
                next_statuses.insert(binding.id.clone(), HotkeyStatus::Error(message));
            }
        }

        if let Ok(mut guard) = statuses.lock() {
            *guard = next_statuses;
        }
    }

    pub fn to_win32_hotkey(combo: HotkeyCombo) -> (u32, u32) {
        (to_win32_modifiers(combo.modifiers), to_win32_vk(combo.key))
    }

    fn to_win32_modifiers(mods: KeyModifiers) -> u32 {
        let mut flags = MOD_NOREPEAT;
        if mods.ctrl {
            flags |= MOD_CONTROL;
        }
        if mods.alt {
            flags |= MOD_ALT;
        }
        if mods.shift {
            flags |= MOD_SHIFT;
        }
        if mods.meta {
            flags |= MOD_WIN;
        }
        flags
    }

    fn to_win32_vk(key: KeyCode) -> u32 {
        let vk = match key {
            KeyCode::PrintScreen => VK_SNAPSHOT,
            KeyCode::F1 => VK_F1,
            KeyCode::F2 => VK_F2,
            KeyCode::F3 => VK_F3,
            KeyCode::F4 => VK_F4,
            KeyCode::F5 => VK_F5,
            KeyCode::F6 => VK_F6,
            KeyCode::F7 => VK_F7,
            KeyCode::F8 => VK_F8,
            KeyCode::F9 => VK_F9,
            KeyCode::F10 => VK_F10,
            KeyCode::F11 => VK_F11,
            KeyCode::F12 => VK_F12,
            KeyCode::KeyA => VK_A,
            KeyCode::KeyB => VK_B,
            KeyCode::KeyC => VK_C,
            KeyCode::KeyD => VK_D,
            KeyCode::KeyE => VK_E,
            KeyCode::KeyF => VK_F,
            KeyCode::KeyG => VK_G,
            KeyCode::KeyH => VK_H,
            KeyCode::KeyI => VK_I,
            KeyCode::KeyJ => VK_J,
            KeyCode::KeyK => VK_K,
            KeyCode::KeyL => VK_L,
            KeyCode::KeyM => VK_M,
            KeyCode::KeyN => VK_N,
            KeyCode::KeyO => VK_O,
            KeyCode::KeyP => VK_P,
            KeyCode::KeyQ => VK_Q,
            KeyCode::KeyR => VK_R,
            KeyCode::KeyS => VK_S,
            KeyCode::KeyT => VK_T,
            KeyCode::KeyU => VK_U,
            KeyCode::KeyV => VK_V,
            KeyCode::KeyW => VK_W,
            KeyCode::KeyX => VK_X,
            KeyCode::KeyY => VK_Y,
            KeyCode::KeyZ => VK_Z,
            KeyCode::Digit0 => VK_0,
            KeyCode::Digit1 => VK_1,
            KeyCode::Digit2 => VK_2,
            KeyCode::Digit3 => VK_3,
            KeyCode::Digit4 => VK_4,
            KeyCode::Digit5 => VK_5,
            KeyCode::Digit6 => VK_6,
            KeyCode::Digit7 => VK_7,
            KeyCode::Digit8 => VK_8,
            KeyCode::Digit9 => VK_9,
            KeyCode::Insert => VK_INSERT,
            KeyCode::Delete => VK_DELETE,
            KeyCode::Home => VK_HOME,
            KeyCode::End => VK_END,
            KeyCode::PageUp => VK_PRIOR,
            KeyCode::PageDown => VK_NEXT,
        };
        u32::from(vk)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn maps_sharex_printscreen_modifiers_to_distinct_win32_flags() {
            let (m_bare, vk_bare) =
                to_win32_hotkey(HotkeyCombo::new(KeyModifiers::NONE, KeyCode::PrintScreen));
            let (m_ctrl, vk_ctrl) =
                to_win32_hotkey(HotkeyCombo::new(KeyModifiers::ctrl(), KeyCode::PrintScreen));
            let (m_alt, vk_alt) =
                to_win32_hotkey(HotkeyCombo::new(KeyModifiers::alt(), KeyCode::PrintScreen));
            let (m_shift, vk_shift) = to_win32_hotkey(HotkeyCombo::new(
                KeyModifiers::shift(),
                KeyCode::PrintScreen,
            ));

            assert_eq!(vk_bare, u32::from(VK_SNAPSHOT));
            assert_eq!(vk_ctrl, u32::from(VK_SNAPSHOT));
            assert_eq!(vk_alt, u32::from(VK_SNAPSHOT));
            assert_eq!(vk_shift, u32::from(VK_SNAPSHOT));
            assert_ne!(m_bare, m_ctrl);
            assert_ne!(m_ctrl, m_alt);
            assert_ne!(m_alt, m_shift);
            assert_ne!(m_ctrl & MOD_CONTROL, 0);
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::HotkeyStatus;
    use eframe::egui;
    use global_hotkey::{
        GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
        hotkey::{Code, HotKey, Modifiers},
    };
    use sharex_core::{HotkeyAction, HotkeyBinding, HotkeyCombo, KeyCode, KeyModifiers};
    use std::collections::HashMap;
    use std::sync::mpsc::{self, Receiver};
    use std::thread;

    pub struct HotkeyRuntime {
        manager: Option<GlobalHotKeyManager>,
        init_error: Option<String>,
        registered: Vec<HotKey>,
        id_to_action: HashMap<u32, HotkeyAction>,
        statuses: HashMap<String, HotkeyStatus>,
        rx: Receiver<u32>,
    }

    impl HotkeyRuntime {
        pub fn new(ctx: egui::Context, bindings: &[HotkeyBinding]) -> Self {
            let (manager, init_error) = match GlobalHotKeyManager::new() {
                Ok(mgr) => (Some(mgr), None),
                Err(err) => (None, Some(err.to_string())),
            };

            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let receiver = GlobalHotKeyEvent::receiver();
                while let Ok(event) = receiver.recv() {
                    if event.state == HotKeyState::Pressed {
                        let _ = tx.send(event.id());
                        ctx.request_repaint();
                    }
                }
            });

            let mut runtime = Self {
                manager,
                init_error,
                registered: Vec::new(),
                id_to_action: HashMap::new(),
                statuses: HashMap::new(),
                rx,
            };
            runtime.sync(bindings);
            runtime
        }

        pub fn sync(&mut self, bindings: &[HotkeyBinding]) {
            if let Some(manager) = &self.manager {
                for existing in self.registered.drain(..) {
                    let _ = manager.unregister(existing);
                }
            }
            self.id_to_action.clear();
            self.statuses.clear();

            for binding in bindings {
                if !binding.enabled {
                    self.statuses
                        .insert(binding.id.clone(), HotkeyStatus::Disabled);
                    continue;
                }
                let Some(combo) = binding.combo else {
                    self.statuses
                        .insert(binding.id.clone(), HotkeyStatus::Unbound);
                    continue;
                };
                let Some(manager) = &self.manager else {
                    let reason = self
                        .init_error
                        .clone()
                        .unwrap_or_else(|| "global hotkey manager unavailable".into());
                    self.statuses
                        .insert(binding.id.clone(), HotkeyStatus::Error(reason));
                    continue;
                };

                let hotkey = to_global_hotkey(combo);
                match manager.register(hotkey) {
                    Ok(()) => {
                        self.id_to_action.insert(hotkey.id(), binding.action);
                        self.registered.push(hotkey);
                        self.statuses
                            .insert(binding.id.clone(), HotkeyStatus::Active);
                    }
                    Err(err) => {
                        self.statuses
                            .insert(binding.id.clone(), HotkeyStatus::Error(err.to_string()));
                    }
                }
            }
        }

        pub fn retry_conflicts(&mut self, bindings: &[HotkeyBinding]) {
            if self
                .statuses
                .values()
                .any(|s| matches!(s, HotkeyStatus::Error(_)))
            {
                self.sync(bindings);
            }
        }

        pub fn poll_actions(&self) -> Vec<HotkeyAction> {
            let mut actions = Vec::new();
            while let Ok(id) = self.rx.try_recv() {
                if let Some(action) = self.id_to_action.get(&id).copied() {
                    actions.push(action);
                }
            }
            actions
        }

        pub fn status_for(&self, id: &str) -> HotkeyStatus {
            self.statuses
                .get(id)
                .cloned()
                .unwrap_or(HotkeyStatus::Disabled)
        }

        pub fn active_count(&self) -> usize {
            self.statuses
                .values()
                .filter(|s| matches!(s, HotkeyStatus::Active))
                .count()
        }
    }

    pub fn to_global_hotkey(combo: HotkeyCombo) -> HotKey {
        let mut flags = Modifiers::empty();
        if combo.modifiers.ctrl {
            flags |= Modifiers::CONTROL;
        }
        if combo.modifiers.shift {
            flags |= Modifiers::SHIFT;
        }
        if combo.modifiers.alt {
            flags |= Modifiers::ALT;
        }
        if combo.modifiers.meta {
            flags |= Modifiers::META;
        }
        let code = match combo.key {
            KeyCode::PrintScreen => Code::PrintScreen,
            KeyCode::F1 => Code::F1,
            KeyCode::F2 => Code::F2,
            KeyCode::F3 => Code::F3,
            KeyCode::F4 => Code::F4,
            KeyCode::F5 => Code::F5,
            KeyCode::F6 => Code::F6,
            KeyCode::F7 => Code::F7,
            KeyCode::F8 => Code::F8,
            KeyCode::F9 => Code::F9,
            KeyCode::F10 => Code::F10,
            KeyCode::F11 => Code::F11,
            KeyCode::F12 => Code::F12,
            KeyCode::KeyA => Code::KeyA,
            KeyCode::KeyB => Code::KeyB,
            KeyCode::KeyC => Code::KeyC,
            KeyCode::KeyD => Code::KeyD,
            KeyCode::KeyE => Code::KeyE,
            KeyCode::KeyF => Code::KeyF,
            KeyCode::KeyG => Code::KeyG,
            KeyCode::KeyH => Code::KeyH,
            KeyCode::KeyI => Code::KeyI,
            KeyCode::KeyJ => Code::KeyJ,
            KeyCode::KeyK => Code::KeyK,
            KeyCode::KeyL => Code::KeyL,
            KeyCode::KeyM => Code::KeyM,
            KeyCode::KeyN => Code::KeyN,
            KeyCode::KeyO => Code::KeyO,
            KeyCode::KeyP => Code::KeyP,
            KeyCode::KeyQ => Code::KeyQ,
            KeyCode::KeyR => Code::KeyR,
            KeyCode::KeyS => Code::KeyS,
            KeyCode::KeyT => Code::KeyT,
            KeyCode::KeyU => Code::KeyU,
            KeyCode::KeyV => Code::KeyV,
            KeyCode::KeyW => Code::KeyW,
            KeyCode::KeyX => Code::KeyX,
            KeyCode::KeyY => Code::KeyY,
            KeyCode::KeyZ => Code::KeyZ,
            KeyCode::Digit0 => Code::Digit0,
            KeyCode::Digit1 => Code::Digit1,
            KeyCode::Digit2 => Code::Digit2,
            KeyCode::Digit3 => Code::Digit3,
            KeyCode::Digit4 => Code::Digit4,
            KeyCode::Digit5 => Code::Digit5,
            KeyCode::Digit6 => Code::Digit6,
            KeyCode::Digit7 => Code::Digit7,
            KeyCode::Digit8 => Code::Digit8,
            KeyCode::Digit9 => Code::Digit9,
            KeyCode::Insert => Code::Insert,
            KeyCode::Delete => Code::Delete,
            KeyCode::Home => Code::Home,
            KeyCode::End => Code::End,
            KeyCode::PageUp => Code::PageUp,
            KeyCode::PageDown => Code::PageDown,
        };
        HotKey::new((!flags.is_empty()).then_some(flags), code)
    }
}

pub use platform::HotkeyRuntime;
