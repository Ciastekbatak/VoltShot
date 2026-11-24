use std::fmt;
use std::str::FromStr;

/// Supported desktop UI languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UiLanguage {
    #[default]
    En,
    Pl,
}

/// Error returned when parsing an unsupported language identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseUiLanguageError {
    input: String,
}

impl fmt::Display for ParseUiLanguageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported UI language: '{}'", self.input)
    }
}

impl std::error::Error for ParseUiLanguageError {}

impl UiLanguage {
    pub const ALL: [Self; 2] = [Self::En, Self::Pl];

    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Pl => "pl",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Pl => "Polski",
        }
    }

    pub const fn strings(self) -> &'static Strings {
        strings(self)
    }
}

impl fmt::Display for UiLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

impl FromStr for UiLanguage {
    type Err = ParseUiLanguageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "en" | "english" | "en-us" | "en_us" | "en-gb" | "en_gb" => Ok(Self::En),
            "pl" | "polish" | "polski" | "pl-pl" | "pl_pl" => Ok(Self::Pl),
            _ => Err(ParseUiLanguageError {
                input: s.to_owned(),
            }),
        }
    }
}

/// Complete localized UI string table for the ShareX desktop interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Strings {
    pub capture_section: &'static str,
    pub after_capture_section: &'static str,
    pub workspace_section: &'static str,

    pub capture: &'static str,
    pub fullscreen: &'static str,
    pub region: &'static str,
    pub active_window: &'static str,
    pub last_region: &'static str,
    pub capture_region_cta: &'static str,
    pub capture_monitor_go: &'static str,
    pub no_monitor: &'static str,

    pub after_capture_tasks: &'static str,
    pub save_image_to_file: &'static str,
    pub copy_image_to_clipboard: &'static str,
    pub open_in_image_editor: &'static str,
    pub copy_file_path: &'static str,
    pub pin_to_screen: &'static str,

    pub captures: &'static str,
    pub image_editor: &'static str,
    pub image_editor_dirty: &'static str,
    pub image_editor_empty: &'static str,
    pub hotkey_settings: &'static str,
    pub effects: &'static str,
    pub tools: &'static str,
    pub recording: &'static str,
    pub uploaders: &'static str,
    pub application_settings: &'static str,

    pub open_screenshots_folder: &'static str,
    pub refresh: &'static str,
    pub filter_captures_hint: &'static str,
    pub no_screenshots_yet: &'static str,
    pub no_preview: &'static str,
    pub edit: &'static str,
    pub copy_image: &'static str,
    pub copy_path: &'static str,
    pub delete_file: &'static str,

    pub no_image_in_editor: &'static str,
    pub save: &'static str,
    pub save_as_copy: &'static str,
    pub close: &'static str,

    pub apply_and_save: &'static str,
    pub reset_defaults: &'static str,
    pub clear: &'static str,
    pub status_active: &'static str,
    pub status_disabled: &'static str,
    pub status_unbound: &'static str,

    pub screenshots_folder: &'static str,
    pub open: &'static str,
    pub filename_pattern: &'static str,
    pub pattern_preview: &'static str,
    pub image_format: &'static str,
    pub theme: &'static str,
    pub theme_dark: &'static str,
    pub theme_light: &'static str,
    pub language: &'static str,
    pub save_settings: &'static str,

    pub pin_opacity: &'static str,
    pub pin_borderless: &'static str,
    pub pin_reset_zoom: &'static str,
    pub pin_close: &'static str,
    pub pin_close_all: &'static str,

    pub ready_status: &'static str,
}

const EN_STRINGS: Strings = Strings {
    capture_section: "CAPTURE",
    after_capture_section: "AFTER CAPTURE TASKS",
    workspace_section: "WORKSPACE",

    capture: "Capture",
    fullscreen: "Fullscreen",
    region: "Region",
    active_window: "Active window",
    last_region: "Last region",
    capture_region_cta: "+ Capture Region",
    capture_monitor_go: "Go",
    no_monitor: "No monitor",

    after_capture_tasks: "After capture tasks",
    save_image_to_file: "Save image to file",
    copy_image_to_clipboard: "Copy image to clipboard",
    open_in_image_editor: "Open in image editor",
    copy_file_path: "Copy file path",
    pin_to_screen: "Pin to screen",

    captures: "Captures",
    image_editor: "Image editor",
    image_editor_dirty: "Image editor *",
    image_editor_empty: "Image editor (empty)",
    hotkey_settings: "Hotkey settings",
    effects: "Effects",
    tools: "Tools",
    recording: "Recording",
    uploaders: "Uploaders",
    application_settings: "Application settings",

    open_screenshots_folder: "Open Screenshots Folder",
    refresh: "Refresh",
    filter_captures_hint: "Filter captures...",
    no_screenshots_yet: "No screenshots yet",
    no_preview: "No preview",
    edit: "Edit",
    copy_image: "Copy Image",
    copy_path: "Copy Path",
    delete_file: "Delete File",

    no_image_in_editor: "No image loaded in the editor",
    save: "Save",
    save_as_copy: "Save as Copy",
    close: "Close",

    apply_and_save: "Apply & Save",
    reset_defaults: "Reset ShareX Defaults",
    clear: "Clear",
    status_active: "Active",
    status_disabled: "Disabled",
    status_unbound: "Unbound",

    screenshots_folder: "Screenshots folder",
    open: "Open",
    filename_pattern: "Filename pattern",
    pattern_preview: "Pattern preview",
    image_format: "Image format",
    theme: "Theme",
    theme_dark: "ShareX Dark",
    theme_light: "Light",
    language: "Language",
    save_settings: "Save Settings",

    pin_opacity: "Opacity",
    pin_borderless: "Borderless",
    pin_reset_zoom: "Reset zoom (100%)",
    pin_close: "Close pinned window",
    pin_close_all: "Close all pinned",

    ready_status: "Ready. Press PrintScreen or Ctrl+PrintScreen to capture.",
};

const PL_STRINGS: Strings = Strings {
    capture_section: "PRZECHWYTYWANIE",
    after_capture_section: "ZADANIA PO PRZECHWYCENIU",
    workspace_section: "PRZESTRZEŃ ROBOCZA",

    capture: "Przechwyć",
    fullscreen: "Pełny ekran",
    region: "Obszar",
    active_window: "Aktywne okno",
    last_region: "Ostatni obszar",
    capture_region_cta: "+ Przechwyć obszar",
    capture_monitor_go: "Zrób",
    no_monitor: "Brak monitora",

    after_capture_tasks: "Zadania po przechwyceniu",
    save_image_to_file: "Zapisz obraz do pliku",
    copy_image_to_clipboard: "Kopiuj obraz do schowka",
    open_in_image_editor: "Otwórz w edytorze obrazów",
    copy_file_path: "Kopiuj ścieżkę pliku",
    pin_to_screen: "Przypnij do ekranu",

    captures: "Zrzuty",
    image_editor: "Edytor obrazów",
    image_editor_dirty: "Edytor obrazów *",
    image_editor_empty: "Edytor obrazów (pusty)",
    hotkey_settings: "Skróty klawiszowe",
    effects: "Efekty",
    tools: "Narzędzia",
    recording: "Nagrywanie",
    uploaders: "Wysyłanie",
    application_settings: "Ustawienia aplikacji",

    open_screenshots_folder: "Otwórz folder zrzutów",
    refresh: "Odśwież",
    filter_captures_hint: "Filtruj zrzuty...",
    no_screenshots_yet: "Brak zrzutów ekranu",
    no_preview: "Brak podglądu",
    edit: "Edytuj",
    copy_image: "Kopiuj obraz",
    copy_path: "Kopiuj ścieżkę",
    delete_file: "Usuń plik",

    no_image_in_editor: "Brak obrazu w edytorze",
    save: "Zapisz",
    save_as_copy: "Zapisz kopię",
    close: "Zamknij",

    apply_and_save: "Zastosuj i zapisz",
    reset_defaults: "Przywróć domyślne ShareX",
    clear: "Wyczyść",
    status_active: "Aktywny",
    status_disabled: "Wyłączony",
    status_unbound: "Nieprzypisany",

    screenshots_folder: "Folder zrzutów",
    open: "Otwórz",
    filename_pattern: "Wzorzec nazwy pliku",
    pattern_preview: "Podgląd nazwy",
    image_format: "Format obrazu",
    theme: "Motyw",
    theme_dark: "Ciemny ShareX",
    theme_light: "Jasny",
    language: "Język",
    save_settings: "Zapisz ustawienia",

    pin_opacity: "Przezroczystość",
    pin_borderless: "Bez ramki",
    pin_reset_zoom: "Resetuj powiększenie (100%)",
    pin_close: "Zamknij przypięte okno",
    pin_close_all: "Zamknij wszystkie przypięte",

    ready_status: "Gotowy. Naciśnij PrintScreen lub Ctrl+PrintScreen, aby przechwycić.",
};

/// Returns the static localization table for the selected UI language.
pub const fn strings(lang: UiLanguage) -> &'static Strings {
    match lang {
        UiLanguage::En => &EN_STRINGS,
        UiLanguage::Pl => &PL_STRINGS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_all_fields(s: &Strings) -> Vec<(&'static str, &'static str)> {
        let Strings {
            capture_section,
            after_capture_section,
            workspace_section,
            capture,
            fullscreen,
            region,
            active_window,
            last_region,
            capture_region_cta,
            capture_monitor_go,
            no_monitor,
            after_capture_tasks,
            save_image_to_file,
            copy_image_to_clipboard,
            open_in_image_editor,
            copy_file_path,
            pin_to_screen,
            captures,
            image_editor,
            image_editor_dirty,
            image_editor_empty,
            hotkey_settings,
            effects,
            tools,
            recording,
            uploaders,
            application_settings,
            open_screenshots_folder,
            refresh,
            filter_captures_hint,
            no_screenshots_yet,
            no_preview,
            edit,
            copy_image,
            copy_path,
            delete_file,
            no_image_in_editor,
            save,
            save_as_copy,
            close,
            apply_and_save,
            reset_defaults,
            clear,
            status_active,
            status_disabled,
            status_unbound,
            screenshots_folder,
            open,
            filename_pattern,
            pattern_preview,
            image_format,
            theme,
            theme_dark,
            theme_light,
            language,
            save_settings,
            pin_opacity,
            pin_borderless,
            pin_reset_zoom,
            pin_close,
            pin_close_all,
            ready_status,
        } = *s;

        vec![
            ("capture_section", capture_section),
            ("after_capture_section", after_capture_section),
            ("workspace_section", workspace_section),
            ("capture", capture),
            ("fullscreen", fullscreen),
            ("region", region),
            ("active_window", active_window),
            ("last_region", last_region),
            ("capture_region_cta", capture_region_cta),
            ("capture_monitor_go", capture_monitor_go),
            ("no_monitor", no_monitor),
            ("after_capture_tasks", after_capture_tasks),
            ("save_image_to_file", save_image_to_file),
            ("copy_image_to_clipboard", copy_image_to_clipboard),
            ("open_in_image_editor", open_in_image_editor),
            ("copy_file_path", copy_file_path),
            ("pin_to_screen", pin_to_screen),
            ("captures", captures),
            ("image_editor", image_editor),
            ("image_editor_dirty", image_editor_dirty),
            ("image_editor_empty", image_editor_empty),
            ("hotkey_settings", hotkey_settings),
            ("effects", effects),
            ("tools", tools),
            ("recording", recording),
            ("uploaders", uploaders),
            ("application_settings", application_settings),
            ("open_screenshots_folder", open_screenshots_folder),
            ("refresh", refresh),
            ("filter_captures_hint", filter_captures_hint),
            ("no_screenshots_yet", no_screenshots_yet),
            ("no_preview", no_preview),
            ("edit", edit),
            ("copy_image", copy_image),
            ("copy_path", copy_path),
            ("delete_file", delete_file),
            ("no_image_in_editor", no_image_in_editor),
            ("save", save),
            ("save_as_copy", save_as_copy),
            ("close", close),
            ("apply_and_save", apply_and_save),
            ("reset_defaults", reset_defaults),
            ("clear", clear),
            ("status_active", status_active),
            ("status_disabled", status_disabled),
            ("status_unbound", status_unbound),
            ("screenshots_folder", screenshots_folder),
            ("open", open),
            ("filename_pattern", filename_pattern),
            ("pattern_preview", pattern_preview),
            ("image_format", image_format),
            ("theme", theme),
            ("theme_dark", theme_dark),
            ("theme_light", theme_light),
            ("language", language),
            ("save_settings", save_settings),
            ("pin_opacity", pin_opacity),
            ("pin_borderless", pin_borderless),
            ("pin_reset_zoom", pin_reset_zoom),
            ("pin_close", pin_close),
            ("pin_close_all", pin_close_all),
            ("ready_status", ready_status),
        ]
    }

    #[test]
    fn all_string_fields_are_non_empty_in_en_and_pl() {
        for lang in UiLanguage::ALL {
            let table = strings(lang);
            let fields = collect_all_fields(table);
            assert!(!fields.is_empty());
            for (name, value) in fields {
                assert!(
                    !value.trim().is_empty(),
                    "field '{name}' is empty for language {lang:?}"
                );
            }
        }
    }

    #[test]
    fn ui_language_parses_aliases_and_rejects_unknown_inputs() {
        assert_eq!(UiLanguage::default(), UiLanguage::En);
        assert_eq!(UiLanguage::En.code(), "en");
        assert_eq!(UiLanguage::Pl.code(), "pl");
        assert_eq!(UiLanguage::En.display_name(), "English");
        assert_eq!(UiLanguage::Pl.display_name(), "Polski");

        for input in ["en", "EN", "english", "English", " en-US "] {
            assert_eq!(input.parse::<UiLanguage>(), Ok(UiLanguage::En));
        }
        for input in ["pl", "PL", "polish", "Polish", "Polski", " pl-PL "] {
            assert_eq!(input.parse::<UiLanguage>(), Ok(UiLanguage::Pl));
        }

        assert!("".parse::<UiLanguage>().is_err());
        assert!("de".parse::<UiLanguage>().is_err());
        assert!("french".parse::<UiLanguage>().is_err());
    }

    #[test]
    fn polish_translations_differ_from_english() {
        let en = UiLanguage::En.strings();
        let pl = UiLanguage::Pl.strings();
        assert_ne!(en.capture, pl.capture);
        assert_ne!(en.fullscreen, pl.fullscreen);
        assert_ne!(en.pin_to_screen, pl.pin_to_screen);
        assert_ne!(en.application_settings, pl.application_settings);
    }
}
