use eframe::egui;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use sharex_core::{HotkeyAction, HotkeyBinding, HotkeyCombo, KeyCode, KeyModifiers};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::thread;

/// Live OS registration status for a configured hotkey binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyStatus {
    Active,
    Disabled,
    Unbound,
    Error(String),
}

/// Manages OS-level global hotkeys and wakes the `eframe` UI loop on keypress.
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

/// Converts a `sharex-core` key combo into a `global-hotkey` registration handle.
pub fn to_global_hotkey(combo: HotkeyCombo) -> HotKey {
    let mods = to_modifiers(combo.modifiers);
    let code = to_code(combo.key);
    HotKey::new(mods, code)
}

fn to_modifiers(mods: KeyModifiers) -> Option<Modifiers> {
    let mut flags = Modifiers::empty();
    if mods.ctrl {
        flags |= Modifiers::CONTROL;
    }
    if mods.shift {
        flags |= Modifiers::SHIFT;
    }
    if mods.alt {
        flags |= Modifiers::ALT;
    }
    if mods.meta {
        flags |= Modifiers::META;
    }
    (!flags.is_empty()).then_some(flags)
}

fn to_code(key: KeyCode) -> Code {
    match key {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_distinct_sharex_combos_to_distinct_hotkey_ids() {
        let bare = to_global_hotkey(HotkeyCombo::new(KeyModifiers::NONE, KeyCode::PrintScreen));
        let ctrl = to_global_hotkey(HotkeyCombo::new(KeyModifiers::ctrl(), KeyCode::PrintScreen));
        let alt = to_global_hotkey(HotkeyCombo::new(KeyModifiers::alt(), KeyCode::PrintScreen));
        let shift = to_global_hotkey(HotkeyCombo::new(
            KeyModifiers::shift(),
            KeyCode::PrintScreen,
        ));

        assert_ne!(bare.id(), ctrl.id());
        assert_ne!(ctrl.id(), alt.id());
        assert_ne!(alt.id(), shift.id());
    }
}
