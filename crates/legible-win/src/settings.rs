//! The settings (brief §4.4), kept in `settings.toml` in Legible's folder ([`AppPaths`]).
//!
//! The file is flat TOML, one `key = value` a line, read and written here without a TOML
//! library. A missing file or key is its default; a value that doesn't make sense is its
//! default too (logged). Lines this version doesn't know are kept as they are, so a newer
//! Legible's settings survive an older one saving. Saving writes a temporary file beside it
//! and renames it over the old one, so a failed save never leaves half a file.
//!
//! [`AppPaths`]: crate::paths::AppPaths

use std::fs;
use std::io;
use std::path::Path;

use crate::page::{Theme, View, Width};

/// The lowest and highest zoom, in percent.
pub const ZOOM_MIN: u16 = 50;
pub const ZOOM_MAX: u16 = 300;
/// The zoom steps the toolbar and Ctrl +/− go through, in percent.
pub const ZOOM_STEPS: [u16; 13] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];

/// Whether pictures from the internet are shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RemoteImages {
    #[default]
    Ask,
    Never,
    Always,
}

/// The editor's font.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EditorFont {
    /// The reading style's (Sans or Serif).
    #[default]
    Reading,
    Monospace,
}

/// The narrowest and widest the editor may be in Split, in percent of the window.
pub const SPLIT_MIN: u8 = 20;
pub const SPLIT_MAX: u8 = 80;

/// When the outline panel shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutlineMode {
    /// For long documents: more than 3 headings, and longer than two screens.
    #[default]
    Automatic,
    Always,
    Never,
}

/// Every setting, with its default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub theme: Theme,
    pub serif: bool,
    pub width: Width,
    /// A new window's zoom, in percent: one of [`ZOOM_STEPS`].
    pub default_zoom: u16,
    pub remote_images: RemoteImages,
    pub outline: OutlineMode,
    /// Whether a window offers to make Legible the default for Markdown files (while it isn't):
    /// off once the user dismisses the offer or takes it.
    pub offer_default_app: bool,
    pub editor_font: EditorFont,
    /// The editor's share of the window in Split, in percent.
    pub split: u8,
    /// Comforts, each on by default: Ctrl + mouse wheel zooms the document; a new window opens
    /// at the last one's size and place ([`crate::placement`]); a file opens where the reader
    /// was in it ([`crate::places`]).
    pub wheel_zoom: bool,
    pub reopen_windows: bool,
    pub remember_place: bool,
    /// Whether the status bar shows (the file's path; while editing, words, encoding and line
    /// endings). Off by default.
    pub status_bar: bool,
    /// Whether Legible checks GitHub for a newer version ([`crate::update`]): as it starts, and
    /// when the reader asks. Off by default: the one thing Legible itself sends online.
    pub check_updates: bool,
    /// Lines of the file this version doesn't know, kept for saving.
    unknown: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::System,
            serif: false,
            width: Width::Wide,
            default_zoom: 100,
            remote_images: RemoteImages::Ask,
            outline: OutlineMode::Automatic,
            offer_default_app: true,
            editor_font: EditorFont::Reading,
            split: 50,
            wheel_zoom: true,
            reopen_windows: true,
            remember_place: true,
            status_bar: false,
            check_updates: false,
            unknown: Vec::new(),
        }
    }
}

/// One setting with its new value, as the page sends it: `setting <key>\n<value>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Theme(Theme),
    Serif(bool),
    Width(Width),
    DefaultZoom(u16),
    RemoteImages(RemoteImages),
    Outline(OutlineMode),
    OfferDefaultApp(bool),
    EditorFont(EditorFont),
    Split(u8),
    WheelZoom(bool),
    ReopenWindows(bool),
    RememberPlace(bool),
    StatusBar(bool),
    CheckUpdates(bool),
}

impl Setting {
    /// The setting `key` set to `value`, or None if either isn't one Legible knows.
    pub fn parse(key: &str, value: &str) -> Option<Setting> {
        Some(match key {
            "theme" => Setting::Theme(theme(value)?),
            "style" => Setting::Serif(style(value)?),
            "width" => Setting::Width(width(value)?),
            "zoom" => Setting::DefaultZoom(zoom(value)?),
            "remote-images" => Setting::RemoteImages(remote_images(value)?),
            "outline" => Setting::Outline(outline(value)?),
            "offer-default-app" => Setting::OfferDefaultApp(boolean(value)?),
            "editor-font" => Setting::EditorFont(editor_font(value)?),
            "split" => Setting::Split(split(value)?),
            "wheel-zoom" => Setting::WheelZoom(boolean(value)?),
            "reopen-windows" => Setting::ReopenWindows(boolean(value)?),
            "remember-place" => Setting::RememberPlace(boolean(value)?),
            "status-bar" => Setting::StatusBar(boolean(value)?),
            "check-updates" => Setting::CheckUpdates(boolean(value)?),
            _ => return None,
        })
    }
}

impl Settings {
    /// The settings in `file`: the defaults for what's missing or doesn't make sense.
    pub fn load(file: &Path) -> Settings {
        match fs::read_to_string(file) {
            Ok(text) => Settings::parse(&text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Settings::default(),
            Err(error) => {
                tracing::warn!(%error, "couldn't read the settings; using the defaults");
                Settings::default()
            }
        }
    }

    fn parse(text: &str) -> Settings {
        let mut settings = Settings::default();
        for line in text.lines() {
            let known = line.split_once('=').and_then(|(key, value)| {
                let (key, value) = (key.trim(), value.trim());
                let file_key = FILE_KEYS.iter().find(|(file, _)| *file == key)?;
                let value = value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(value);
                match Setting::parse(file_key.1, value) {
                    Some(setting) => settings.set(setting),
                    None => tracing::warn!(
                        key,
                        value,
                        "a setting that doesn't make sense; using its default"
                    ),
                }
                Some(())
            });
            let blank_or_ours = line.trim().is_empty() || line.trim_start().starts_with('#');
            if known.is_none() && !blank_or_ours {
                settings.unknown.push(line.to_owned());
            }
        }
        settings
    }

    /// How the document is shown.
    pub fn view(&self) -> View {
        View {
            theme: self.theme,
            width: self.width,
            serif: self.serif,
        }
    }

    /// Changes one setting.
    pub fn set(&mut self, setting: Setting) {
        match setting {
            Setting::Theme(v) => self.theme = v,
            Setting::Serif(v) => self.serif = v,
            Setting::Width(v) => self.width = v,
            Setting::DefaultZoom(v) => self.default_zoom = v,
            Setting::RemoteImages(v) => self.remote_images = v,
            Setting::Outline(v) => self.outline = v,
            Setting::OfferDefaultApp(v) => self.offer_default_app = v,
            Setting::EditorFont(v) => self.editor_font = v,
            Setting::Split(v) => self.split = v,
            Setting::WheelZoom(v) => self.wheel_zoom = v,
            Setting::ReopenWindows(v) => self.reopen_windows = v,
            Setting::RememberPlace(v) => self.remember_place = v,
            Setting::StatusBar(v) => self.status_bar = v,
            Setting::CheckUpdates(v) => self.check_updates = v,
        }
    }

    /// The file's text.
    fn to_toml(&self) -> String {
        let mut text = String::from("# Legible's settings. Its Settings page changes them.\n");
        for (file_key, key) in FILE_KEYS {
            let value = self.value(key);
            if is_unquoted(key) {
                text.push_str(&format!("{file_key} = {value}\n"));
            } else {
                text.push_str(&format!("{file_key} = \"{value}\"\n"));
            }
        }
        for line in &self.unknown {
            text.push_str(line);
            text.push('\n');
        }
        text
    }

    /// Saves to `file`: a temporary file beside it, renamed over it.
    pub fn save(&self, file: &Path) -> io::Result<()> {
        let temp = file.with_extension("toml.tmp");
        fs::write(&temp, self.to_toml())?;
        fs::rename(&temp, file).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
    }

    /// A setting's value as the page and the file name it.
    fn value(&self, key: &str) -> String {
        match key {
            "theme" => match self.theme {
                Theme::System => "system",
                Theme::Light => "light",
                Theme::Dark => "dark",
            }
            .into(),
            "style" => if self.serif { "serif" } else { "sans" }.into(),
            "width" => match self.width {
                Width::Narrow => "narrow",
                Width::Comfortable => "comfortable",
                Width::Wide => "wide",
            }
            .into(),
            "zoom" => self.default_zoom.to_string(),
            "remote-images" => match self.remote_images {
                RemoteImages::Ask => "ask",
                RemoteImages::Never => "never",
                RemoteImages::Always => "always",
            }
            .into(),
            "outline" => match self.outline {
                OutlineMode::Automatic => "automatic",
                OutlineMode::Always => "always",
                OutlineMode::Never => "never",
            }
            .into(),
            "offer-default-app" => self.offer_default_app.to_string(),
            "editor-font" => match self.editor_font {
                EditorFont::Reading => "reading",
                EditorFont::Monospace => "monospace",
            }
            .into(),
            "split" => self.split.to_string(),
            "wheel-zoom" => self.wheel_zoom.to_string(),
            "reopen-windows" => self.reopen_windows.to_string(),
            "remember-place" => self.remember_place.to_string(),
            "status-bar" => self.status_bar.to_string(),
            "check-updates" => self.check_updates.to_string(),
            _ => unreachable!("not a setting: {key}"),
        }
    }

    /// The settings as a JSON object for the page, keyed as the page sends them.
    pub fn to_json(&self) -> String {
        let fields: Vec<String> = FILE_KEYS
            .iter()
            .map(|(_, key)| {
                let value = self.value(key);
                let value = if is_unquoted(key) {
                    value
                } else {
                    json_string(&value)
                };
                format!("{}:{value}", json_string(key))
            })
            .collect();
        format!("{{{}}}", fields.join(","))
    }
}

/// Each setting's key in the file, and as the page names it.
const FILE_KEYS: [(&str, &str); 14] = [
    ("theme", "theme"),
    ("reading_style", "style"),
    ("reading_width", "width"),
    ("default_zoom", "zoom"),
    ("remote_images", "remote-images"),
    ("outline", "outline"),
    ("offer_default_app", "offer-default-app"),
    ("editor_font", "editor-font"),
    ("split", "split"),
    ("wheel_zoom", "wheel-zoom"),
    ("reopen_windows", "reopen-windows"),
    ("remember_place", "remember-place"),
    ("status_bar", "status-bar"),
    // Not `check_for_updates`: an early copy had a setting of that name with no check behind
    // it, so a file that says it was never the reader choosing this one.
    ("check_updates", "check-updates"),
];

/// Whether a setting's value is a number or a boolean, unquoted in the file and the JSON.
fn is_unquoted(key: &str) -> bool {
    matches!(
        key,
        "zoom"
            | "offer-default-app"
            | "split"
            | "wheel-zoom"
            | "reopen-windows"
            | "remember-place"
            | "status-bar"
            | "check-updates"
    )
}

fn theme(v: &str) -> Option<Theme> {
    Some(match v {
        "system" => Theme::System,
        "light" => Theme::Light,
        "dark" => Theme::Dark,
        _ => return None,
    })
}

fn style(v: &str) -> Option<bool> {
    Some(match v {
        "sans" => false,
        "serif" => true,
        _ => return None,
    })
}

fn width(v: &str) -> Option<Width> {
    Some(match v {
        "narrow" => Width::Narrow,
        "comfortable" => Width::Comfortable,
        "wide" => Width::Wide,
        _ => return None,
    })
}

/// A zoom in percent from 50 to 300, to the nearest step.
fn zoom(v: &str) -> Option<u16> {
    let n: u16 = v.parse().ok()?;
    if !(ZOOM_MIN..=ZOOM_MAX).contains(&n) {
        return None;
    }
    ZOOM_STEPS
        .iter()
        .copied()
        .min_by_key(|step| step.abs_diff(n))
}

fn remote_images(v: &str) -> Option<RemoteImages> {
    Some(match v {
        "ask" => RemoteImages::Ask,
        "never" => RemoteImages::Never,
        "always" => RemoteImages::Always,
        _ => return None,
    })
}

fn outline(v: &str) -> Option<OutlineMode> {
    Some(match v {
        "automatic" => OutlineMode::Automatic,
        "always" => OutlineMode::Always,
        "never" => OutlineMode::Never,
        _ => return None,
    })
}

fn editor_font(v: &str) -> Option<EditorFont> {
    Some(match v {
        "reading" => EditorFont::Reading,
        "monospace" => EditorFont::Monospace,
        _ => return None,
    })
}

/// The editor's share in Split, in whole percent from [`SPLIT_MIN`] to [`SPLIT_MAX`].
fn split(v: &str) -> Option<u8> {
    let n: u8 = v.parse().ok()?;
    (SPLIT_MIN..=SPLIT_MAX).contains(&n).then_some(n)
}

fn boolean(v: &str) -> Option<bool> {
    v.parse().ok()
}

/// `text` as a JSON string, safe inside a `<script type="application/json">` block and in a
/// script Legible runs in the page: `<`, `>` and `&` are escaped too, so `</script>` can't end
/// the block, and so are U+2028 and U+2029.
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_follow_windows() {
        let s = Settings::default();
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.width, Width::Wide);
        assert_eq!(s.default_zoom, 100);
        assert_eq!(s.outline, OutlineMode::Automatic);
        assert_eq!(s.remote_images, RemoteImages::Ask);
    }

    #[test]
    fn settings_survive_a_save_and_keep_lines_they_dont_know() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("settings.toml");
        assert_eq!(Settings::load(&file), Settings::default());
        fs::write(&file, "future_thing = \"kept\"\ntheme = \"dark\"\n").unwrap();
        let mut s = Settings::load(&file);
        assert_eq!(s.theme, Theme::Dark);
        s.set(Setting::Width(Width::Wide));
        s.set(Setting::DefaultZoom(150));
        s.save(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("future_thing = \"kept\""));
        assert!(!temp.path().join("settings.toml.tmp").exists());
        let back = Settings::load(&file);
        assert_eq!(back, s);
        assert_eq!(back.default_zoom, 150);
    }

    /// Wide became the default after 0.1.0: a file with no width takes it, and a saved width,
    /// Comfortable too, stays.
    #[test]
    fn a_missing_width_is_wide_and_a_saved_one_stays() {
        assert_eq!(Settings::parse("theme = \"dark\"\n").width, Width::Wide);
        for (value, width) in [
            ("narrow", Width::Narrow),
            ("comfortable", Width::Comfortable),
            ("wide", Width::Wide),
        ] {
            let s = Settings::parse(&format!("reading_width = \"{value}\"\n"));
            assert_eq!(s.width, width);
            let saved = format!("reading_width = \"{value}\"");
            assert!(s.to_toml().contains(&saved));
        }
    }

    #[test]
    fn nonsense_values_fall_back_to_their_defaults() {
        let s = Settings::parse(
            "theme = \"purple\"\ndefault_zoom = 900\nreading_width = \"wide\"\nwheel_zoom = maybe\n",
        );
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.default_zoom, 100);
        assert_eq!(s.width, Width::Wide);
        assert!(s.wheel_zoom);
    }

    /// Check for updates was a setting once, with no update check behind it: a file that has
    /// it keeps the line as it is, and it doesn't turn on today's check (`check_updates`), which
    /// the reader never chose.
    #[test]
    fn an_old_check_for_updates_line_is_kept_and_turns_nothing_on() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("settings.toml");
        fs::write(&file, "check_for_updates = true\ntheme = \"dark\"\n").unwrap();
        let s = Settings::load(&file);
        assert_eq!(s.theme, Theme::Dark);
        assert!(!s.check_updates);
        assert!(s.to_json().contains(r#""check-updates":false"#));
        assert_eq!(Setting::parse("check-for-updates", "true"), None);
        s.save(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("check_for_updates = true\n"), "{text}");
        assert_eq!(Settings::load(&file), s);
    }

    #[test]
    fn a_folder_that_cant_be_written_fails_the_save() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("missing").join("settings.toml");
        assert!(Settings::default().save(&file).is_err());
    }

    #[test]
    fn the_page_can_only_send_known_values() {
        assert_eq!(
            Setting::parse("theme", "dark"),
            Some(Setting::Theme(Theme::Dark))
        );
        assert_eq!(Setting::parse("style", "serif"), Some(Setting::Serif(true)));
        assert_eq!(
            Setting::parse("zoom", "120"),
            Some(Setting::DefaultZoom(125))
        );
        assert_eq!(Setting::parse("zoom", "49"), None);
        assert_eq!(Setting::parse("zoom", "301"), None);
        assert_eq!(Setting::parse("zoom", "1e2"), None);
        assert_eq!(Setting::parse("theme", "Dark"), None);
        assert_eq!(Setting::parse("unknown", "x"), None);
        assert_eq!(
            Setting::parse("editor-font", "monospace"),
            Some(Setting::EditorFont(EditorFont::Monospace))
        );
        assert_eq!(Setting::parse("editor-font", "mono"), None);
        assert_eq!(Setting::parse("split", "35"), Some(Setting::Split(35)));
        for outside in ["19", "81", "50.5", "-1", ""] {
            assert_eq!(Setting::parse("split", outside), None, "{outside}");
        }
        assert_eq!(
            Setting::parse("status-bar", "true"),
            Some(Setting::StatusBar(true))
        );
        assert_eq!(Setting::parse("status-bar", "on"), None);
    }

    #[test]
    fn the_status_bar_is_off_by_default_and_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.toml");
        let mut s = Settings::default();
        assert!(!s.status_bar);
        s.set(Setting::StatusBar(true));
        s.save(&file).unwrap();
        assert!(
            fs::read_to_string(&file)
                .unwrap()
                .contains("status_bar = true\n")
        );
        assert!(Settings::load(&file).status_bar);
        assert!(s.to_json().contains(r#""status-bar":true,"#));
    }

    #[test]
    fn the_update_check_is_off_by_default_and_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.toml");
        let mut s = Settings::default();
        assert!(!s.check_updates);
        assert_eq!(
            Setting::parse("check-updates", "true"),
            Some(Setting::CheckUpdates(true))
        );
        assert_eq!(Setting::parse("check-updates", "yes"), None);
        s.set(Setting::CheckUpdates(true));
        s.save(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("check_updates = true\n"), "{text}");
        assert!(Settings::load(&file).check_updates);
        assert!(s.to_json().ends_with(r#""check-updates":true}"#));
    }

    #[test]
    fn the_editors_settings_survive_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.toml");
        let mut s = Settings::default();
        assert_eq!((s.editor_font, s.split), (EditorFont::Reading, 50));
        s.set(Setting::EditorFont(EditorFont::Monospace));
        s.set(Setting::Split(62));
        s.save(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("editor_font = \"monospace\"\nsplit = 62\n"),
            "{text}"
        );
        let back = Settings::load(&file);
        assert_eq!((back.editor_font, back.split), (EditorFont::Monospace, 62));
    }

    #[test]
    fn the_comforts_are_on_by_default_and_each_turns_off() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.toml");
        let mut s = Settings::default();
        assert!(s.wheel_zoom && s.reopen_windows && s.remember_place);
        for (key, setting) in [
            ("wheel-zoom", Setting::WheelZoom(false)),
            ("reopen-windows", Setting::ReopenWindows(false)),
            ("remember-place", Setting::RememberPlace(false)),
        ] {
            assert_eq!(Setting::parse(key, "false"), Some(setting));
            assert_eq!(Setting::parse(key, "off"), None);
            s.set(setting);
        }
        s.save(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("wheel_zoom = false\nreopen_windows = false\nremember_place = false\n"),
            "{text}"
        );
        let back = Settings::load(&file);
        assert!(!back.wheel_zoom && !back.reopen_windows && !back.remember_place);
        let json = back.to_json();
        assert!(
            json.contains(r#""wheel-zoom":false,"reopen-windows":false,"remember-place":false"#),
            "{json}"
        );
    }

    #[test]
    fn json_is_safe_in_a_script_block() {
        assert_eq!(json_string("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(json_string("</script>&"), r#""\u003c/script\u003e\u0026""#);
        assert_eq!(json_string("\u{2028}\n"), r#""\u2028\n""#);
        let json = Settings::default().to_json();
        assert_eq!(
            json,
            r#"{"theme":"system","style":"sans","width":"wide","zoom":100,"remote-images":"ask","outline":"automatic","offer-default-app":true,"editor-font":"reading","split":50,"wheel-zoom":true,"reopen-windows":true,"remember-place":true,"status-bar":false,"check-updates":false}"#
        );
    }
}
