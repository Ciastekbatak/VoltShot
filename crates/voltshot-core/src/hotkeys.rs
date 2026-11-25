use crate::domain::CoreError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Action triggered by a global hotkey binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    CaptureFullscreen,
    CaptureRegion,
    CaptureActiveWindow,
    CaptureLastRegion,
    OpenEditor,
    RecordVideo,
    RecordGif,
    CaptureOcr,
    PickScreenColor,
}

impl HotkeyAction {
    pub const ALL: [Self; 9] = [
        Self::CaptureFullscreen,
        Self::CaptureRegion,
        Self::CaptureActiveWindow,
        Self::CaptureLastRegion,
        Self::OpenEditor,
        Self::RecordVideo,
        Self::RecordGif,
        Self::CaptureOcr,
        Self::PickScreenColor,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::CaptureFullscreen => "Capture fullscreen",
            Self::CaptureRegion => "Capture region",
            Self::CaptureActiveWindow => "Capture active window",
            Self::CaptureLastRegion => "Capture last region",
            Self::OpenEditor => "Open image editor",
            Self::RecordVideo => "Record screen (MP4)",
            Self::RecordGif => "Record screen (GIF)",
            Self::CaptureOcr => "Capture text (OCR)",
            Self::PickScreenColor => "Screen color picker",
        }
    }
}

/// Modifier flags for a hotkey combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct KeyModifiers {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub meta: bool,
}

impl KeyModifiers {
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
        meta: false,
    };

    pub const fn ctrl() -> Self {
        Self {
            ctrl: true,
            ..Self::NONE
        }
    }

    pub const fn alt() -> Self {
        Self {
            alt: true,
            ..Self::NONE
        }
    }

    pub const fn shift() -> Self {
        Self {
            shift: true,
            ..Self::NONE
        }
    }
}

/// Supported primary keys for global hotkey bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyCode {
    PrintScreen,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
}

impl KeyCode {
    pub const ALL: [Self; 55] = [
        Self::PrintScreen,
        Self::F1,
        Self::F2,
        Self::F3,
        Self::F4,
        Self::F5,
        Self::F6,
        Self::F7,
        Self::F8,
        Self::F9,
        Self::F10,
        Self::F11,
        Self::F12,
        Self::KeyA,
        Self::KeyB,
        Self::KeyC,
        Self::KeyD,
        Self::KeyE,
        Self::KeyF,
        Self::KeyG,
        Self::KeyH,
        Self::KeyI,
        Self::KeyJ,
        Self::KeyK,
        Self::KeyL,
        Self::KeyM,
        Self::KeyN,
        Self::KeyO,
        Self::KeyP,
        Self::KeyQ,
        Self::KeyR,
        Self::KeyS,
        Self::KeyT,
        Self::KeyU,
        Self::KeyV,
        Self::KeyW,
        Self::KeyX,
        Self::KeyY,
        Self::KeyZ,
        Self::Digit0,
        Self::Digit1,
        Self::Digit2,
        Self::Digit3,
        Self::Digit4,
        Self::Digit5,
        Self::Digit6,
        Self::Digit7,
        Self::Digit8,
        Self::Digit9,
        Self::Insert,
        Self::Delete,
        Self::Home,
        Self::End,
        Self::PageUp,
        Self::PageDown,
    ];

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::PrintScreen => "PrintScreen",
            Self::F1 => "F1",
            Self::F2 => "F2",
            Self::F3 => "F3",
            Self::F4 => "F4",
            Self::F5 => "F5",
            Self::F6 => "F6",
            Self::F7 => "F7",
            Self::F8 => "F8",
            Self::F9 => "F9",
            Self::F10 => "F10",
            Self::F11 => "F11",
            Self::F12 => "F12",
            Self::KeyA => "A",
            Self::KeyB => "B",
            Self::KeyC => "C",
            Self::KeyD => "D",
            Self::KeyE => "E",
            Self::KeyF => "F",
            Self::KeyG => "G",
            Self::KeyH => "H",
            Self::KeyI => "I",
            Self::KeyJ => "J",
            Self::KeyK => "K",
            Self::KeyL => "L",
            Self::KeyM => "M",
            Self::KeyN => "N",
            Self::KeyO => "O",
            Self::KeyP => "P",
            Self::KeyQ => "Q",
            Self::KeyR => "R",
            Self::KeyS => "S",
            Self::KeyT => "T",
            Self::KeyU => "U",
            Self::KeyV => "V",
            Self::KeyW => "W",
            Self::KeyX => "X",
            Self::KeyY => "Y",
            Self::KeyZ => "Z",
            Self::Digit0 => "0",
            Self::Digit1 => "1",
            Self::Digit2 => "2",
            Self::Digit3 => "3",
            Self::Digit4 => "4",
            Self::Digit5 => "5",
            Self::Digit6 => "6",
            Self::Digit7 => "7",
            Self::Digit8 => "8",
            Self::Digit9 => "9",
            Self::Insert => "Insert",
            Self::Delete => "Delete",
            Self::Home => "Home",
            Self::End => "End",
            Self::PageUp => "PageUp",
            Self::PageDown => "PageDown",
        }
    }
}

impl fmt::Display for KeyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

impl FromStr for KeyCode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_uppercase();
        match normalized.as_str() {
            "PRINTSCREEN" | "PRTSC" | "PRINT" | "SNAPSHOT" => Ok(Self::PrintScreen),
            "F1" => Ok(Self::F1),
            "F2" => Ok(Self::F2),
            "F3" => Ok(Self::F3),
            "F4" => Ok(Self::F4),
            "F5" => Ok(Self::F5),
            "F6" => Ok(Self::F6),
            "F7" => Ok(Self::F7),
            "F8" => Ok(Self::F8),
            "F9" => Ok(Self::F9),
            "F10" => Ok(Self::F10),
            "F11" => Ok(Self::F11),
            "F12" => Ok(Self::F12),
            "A" | "KEYA" => Ok(Self::KeyA),
            "B" | "KEYB" => Ok(Self::KeyB),
            "C" | "KEYC" => Ok(Self::KeyC),
            "D" | "KEYD" => Ok(Self::KeyD),
            "E" | "KEYE" => Ok(Self::KeyE),
            "F" | "KEYF" => Ok(Self::KeyF),
            "G" | "KEYG" => Ok(Self::KeyG),
            "H" | "KEYH" => Ok(Self::KeyH),
            "I" | "KEYI" => Ok(Self::KeyI),
            "J" | "KEYJ" => Ok(Self::KeyJ),
            "K" | "KEYK" => Ok(Self::KeyK),
            "L" | "KEYL" => Ok(Self::KeyL),
            "M" | "KEYM" => Ok(Self::KeyM),
            "N" | "KEYN" => Ok(Self::KeyN),
            "O" | "KEYO" => Ok(Self::KeyO),
            "P" | "KEYP" => Ok(Self::KeyP),
            "Q" | "KEYQ" => Ok(Self::KeyQ),
            "R" | "KEYR" => Ok(Self::KeyR),
            "S" | "KEYS" => Ok(Self::KeyS),
            "T" | "KEYT" => Ok(Self::KeyT),
            "U" | "KEYU" => Ok(Self::KeyU),
            "V" | "KEYV" => Ok(Self::KeyV),
            "W" | "KEYW" => Ok(Self::KeyW),
            "X" | "KEYX" => Ok(Self::KeyX),
            "Y" | "KEYY" => Ok(Self::KeyY),
            "Z" | "KEYZ" => Ok(Self::KeyZ),
            "0" | "DIGIT0" => Ok(Self::Digit0),
            "1" | "DIGIT1" => Ok(Self::Digit1),
            "2" | "DIGIT2" => Ok(Self::Digit2),
            "3" | "DIGIT3" => Ok(Self::Digit3),
            "4" | "DIGIT4" => Ok(Self::Digit4),
            "5" | "DIGIT5" => Ok(Self::Digit5),
            "6" | "DIGIT6" => Ok(Self::Digit6),
            "7" | "DIGIT7" => Ok(Self::Digit7),
            "8" | "DIGIT8" => Ok(Self::Digit8),
            "9" | "DIGIT9" => Ok(Self::Digit9),
            "INSERT" | "INS" => Ok(Self::Insert),
            "DELETE" | "DEL" => Ok(Self::Delete),
            "HOME" => Ok(Self::Home),
            "END" => Ok(Self::End),
            "PAGEUP" | "PGUP" => Ok(Self::PageUp),
            "PAGEDOWN" | "PGDN" => Ok(Self::PageDown),
            _ => Err(CoreError::ConfigError(format!("unsupported key `{s}`"))),
        }
    }
}

/// Complete key combination (modifiers + primary key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HotkeyCombo {
    pub modifiers: KeyModifiers,
    pub key: KeyCode,
}

impl HotkeyCombo {
    pub const fn new(modifiers: KeyModifiers, key: KeyCode) -> Self {
        Self { modifiers, key }
    }

    pub fn parse(spec: &str) -> Result<Self, CoreError> {
        let mut modifiers = KeyModifiers::NONE;
        let mut primary: Option<KeyCode> = None;

        for token in spec.split('+').map(str::trim).filter(|t| !t.is_empty()) {
            match token.to_ascii_uppercase().as_str() {
                "CTRL" | "CONTROL" => modifiers.ctrl = true,
                "SHIFT" => modifiers.shift = true,
                "ALT" => modifiers.alt = true,
                "WIN" | "META" | "SUPER" | "CMD" => modifiers.meta = true,
                other => {
                    if primary.is_some() {
                        return Err(CoreError::ConfigError(format!(
                            "multiple primary keys in hotkey `{spec}`"
                        )));
                    }
                    primary = Some(KeyCode::from_str(other)?);
                }
            }
        }

        let key = primary.ok_or_else(|| {
            CoreError::ConfigError(format!("hotkey `{spec}` is missing a primary key"))
        })?;
        Ok(Self { modifiers, key })
    }
}

impl fmt::Display for HotkeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<&str> = Vec::with_capacity(5);
        if self.modifiers.ctrl {
            parts.push("Ctrl");
        }
        if self.modifiers.alt {
            parts.push("Alt");
        }
        if self.modifiers.shift {
            parts.push("Shift");
        }
        if self.modifiers.meta {
            parts.push("Win");
        }
        parts.push(self.key.display_name());
        f.write_str(&parts.join(" + "))
    }
}

/// Persisted hotkey binding connecting a key combination to a capture or editor action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyBinding {
    pub id: String,
    pub action: HotkeyAction,
    pub combo: Option<HotkeyCombo>,
    pub enabled: bool,
}

/// Returns the default VoltShot global hotkey set for the current operating system.
pub fn default_voltshot_hotkeys() -> Vec<HotkeyBinding> {
    default_voltshot_hotkeys_for_os(cfg!(target_os = "macos"))
}

/// Returns the default VoltShot global hotkey set for Windows/Linux (`is_macos = false`)
/// or macOS (`is_macos = true`, avoiding the absent `PrintScreen` key and macOS screenshot shortcuts).
pub fn default_voltshot_hotkeys_for_os(is_macos: bool) -> Vec<HotkeyBinding> {
    if is_macos {
        let ctrl_shift = KeyModifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        return vec![
            HotkeyBinding {
                id: "capture-fullscreen".into(),
                action: HotkeyAction::CaptureFullscreen,
                combo: Some(HotkeyCombo::new(ctrl_shift, KeyCode::Digit1)),
                enabled: true,
            },
            HotkeyBinding {
                id: "capture-region".into(),
                action: HotkeyAction::CaptureRegion,
                combo: Some(HotkeyCombo::new(ctrl_shift, KeyCode::Digit2)),
                enabled: true,
            },
            HotkeyBinding {
                id: "capture-active-window".into(),
                action: HotkeyAction::CaptureActiveWindow,
                combo: Some(HotkeyCombo::new(ctrl_shift, KeyCode::Digit3)),
                enabled: true,
            },
            HotkeyBinding {
                id: "capture-last-region".into(),
                action: HotkeyAction::CaptureLastRegion,
                combo: Some(HotkeyCombo::new(ctrl_shift, KeyCode::Digit4)),
                enabled: true,
            },
            HotkeyBinding {
                id: "open-editor".into(),
                action: HotkeyAction::OpenEditor,
                combo: Some(HotkeyCombo::new(ctrl_shift, KeyCode::KeyE)),
                enabled: true,
            },
            HotkeyBinding {
                id: "record-video".into(),
                action: HotkeyAction::RecordVideo,
                combo: Some(HotkeyCombo::new(ctrl_shift, KeyCode::KeyR)),
                enabled: true,
            },
            HotkeyBinding {
                id: "record-gif".into(),
                action: HotkeyAction::RecordGif,
                combo: Some(HotkeyCombo::new(ctrl_shift, KeyCode::KeyG)),
                enabled: true,
            },
            HotkeyBinding {
                id: "capture-ocr".into(),
                action: HotkeyAction::CaptureOcr,
                combo: Some(HotkeyCombo::new(ctrl_shift, KeyCode::KeyO)),
                enabled: true,
            },
            HotkeyBinding {
                id: "pick-screen-color".into(),
                action: HotkeyAction::PickScreenColor,
                combo: Some(HotkeyCombo::new(ctrl_shift, KeyCode::KeyC)),
                enabled: true,
            },
        ];
    }

    vec![
        HotkeyBinding {
            id: "capture-fullscreen".into(),
            action: HotkeyAction::CaptureFullscreen,
            combo: Some(HotkeyCombo::new(KeyModifiers::NONE, KeyCode::PrintScreen)),
            enabled: true,
        },
        HotkeyBinding {
            id: "capture-region".into(),
            action: HotkeyAction::CaptureRegion,
            combo: Some(HotkeyCombo::new(KeyModifiers::ctrl(), KeyCode::PrintScreen)),
            enabled: true,
        },
        HotkeyBinding {
            id: "capture-active-window".into(),
            action: HotkeyAction::CaptureActiveWindow,
            combo: Some(HotkeyCombo::new(KeyModifiers::alt(), KeyCode::PrintScreen)),
            enabled: true,
        },
        HotkeyBinding {
            id: "capture-last-region".into(),
            action: HotkeyAction::CaptureLastRegion,
            combo: Some(HotkeyCombo::new(
                KeyModifiers::shift(),
                KeyCode::PrintScreen,
            )),
            enabled: true,
        },
        HotkeyBinding {
            id: "open-editor".into(),
            action: HotkeyAction::OpenEditor,
            combo: Some(HotkeyCombo::new(
                KeyModifiers {
                    ctrl: true,
                    shift: true,
                    alt: false,
                    meta: false,
                },
                KeyCode::KeyE,
            )),
            enabled: true,
        },
        HotkeyBinding {
            id: "record-video".into(),
            action: HotkeyAction::RecordVideo,
            combo: Some(HotkeyCombo::new(
                KeyModifiers {
                    ctrl: true,
                    shift: true,
                    ..KeyModifiers::NONE
                },
                KeyCode::PrintScreen,
            )),
            enabled: true,
        },
        HotkeyBinding {
            id: "record-gif".into(),
            action: HotkeyAction::RecordGif,
            combo: Some(HotkeyCombo::new(
                KeyModifiers {
                    alt: true,
                    shift: true,
                    ..KeyModifiers::NONE
                },
                KeyCode::PrintScreen,
            )),
            enabled: true,
        },
        HotkeyBinding {
            id: "capture-ocr".into(),
            action: HotkeyAction::CaptureOcr,
            combo: Some(HotkeyCombo::new(
                KeyModifiers {
                    ctrl: true,
                    shift: true,
                    ..KeyModifiers::NONE
                },
                KeyCode::KeyO,
            )),
            enabled: true,
        },
        HotkeyBinding {
            id: "pick-screen-color".into(),
            action: HotkeyAction::PickScreenColor,
            combo: Some(HotkeyCombo::new(
                KeyModifiers {
                    ctrl: true,
                    shift: true,
                    ..KeyModifiers::NONE
                },
                KeyCode::KeyC,
            )),
            enabled: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_voltshot_hotkey_combos() {
        let combo = HotkeyCombo::parse("Ctrl + PrintScreen").expect("parse");
        assert!(combo.modifiers.ctrl);
        assert!(!combo.modifiers.alt);
        assert_eq!(combo.key, KeyCode::PrintScreen);
        assert_eq!(combo.to_string(), "Ctrl + PrintScreen");

        let bare = HotkeyCombo::parse("PrtSc").expect("alias");
        assert_eq!(bare.to_string(), "PrintScreen");

        assert!(HotkeyCombo::parse("Ctrl + Shift").is_err());
        assert!(HotkeyCombo::parse("Ctrl + A + B").is_err());
    }

    #[test]
    fn default_hotkeys_include_standard_voltshot_bindings() {
        let defaults = default_voltshot_hotkeys_for_os(false);
        assert_eq!(defaults.len(), 9);
        assert!(
            defaults
                .iter()
                .map(|binding| binding.action)
                .eq(HotkeyAction::ALL)
        );
        assert_eq!(HotkeyAction::CaptureOcr.label(), "Capture text (OCR)");
        assert_eq!(HotkeyAction::PickScreenColor.label(), "Screen color picker");
        assert_eq!(
            defaults[0].combo.map(|c| c.to_string()).as_deref(),
            Some("PrintScreen")
        );
        assert_eq!(
            defaults[1].combo.map(|c| c.to_string()).as_deref(),
            Some("Ctrl + PrintScreen")
        );
        assert_eq!(
            defaults[2].combo.map(|c| c.to_string()).as_deref(),
            Some("Alt + PrintScreen")
        );
        assert_eq!(
            defaults[3].combo.map(|c| c.to_string()).as_deref(),
            Some("Shift + PrintScreen")
        );
        for (binding, id, action, combo) in [
            (
                &defaults[5],
                "record-video",
                HotkeyAction::RecordVideo,
                "Ctrl + Shift + PrintScreen",
            ),
            (
                &defaults[6],
                "record-gif",
                HotkeyAction::RecordGif,
                "Alt + Shift + PrintScreen",
            ),
            (
                &defaults[7],
                "capture-ocr",
                HotkeyAction::CaptureOcr,
                "Ctrl + Shift + O",
            ),
            (
                &defaults[8],
                "pick-screen-color",
                HotkeyAction::PickScreenColor,
                "Ctrl + Shift + C",
            ),
        ] {
            assert_eq!(binding.id, id);
            assert_eq!(binding.action, action);
            assert_eq!(binding.combo, Some(HotkeyCombo::parse(combo).unwrap()));
            assert!(binding.enabled);
        }

        let mac_defaults = default_voltshot_hotkeys_for_os(true);
        assert_eq!(mac_defaults.len(), 9);
        assert!(
            mac_defaults
                .iter()
                .map(|binding| binding.action)
                .eq(HotkeyAction::ALL)
        );
        assert!(
            mac_defaults
                .iter()
                .all(|b| b.combo.is_some_and(|c| c.key != KeyCode::PrintScreen))
        );
        assert_eq!(
            mac_defaults[1].combo.map(|c| c.to_string()).as_deref(),
            Some("Ctrl + Shift + 2")
        );
    }
}
