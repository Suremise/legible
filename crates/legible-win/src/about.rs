//! About Legible's facts: the version, how this copy runs, the WebView2 runtime and
//! Windows, each as About's table shows it and as Copy puts it on the clipboard. The window
//! gathers them ([`Facts`]); the page shows them (web/chrome/chrome.js, `LegibleChrome.about`).

use std::path::Path;

use legible_core::i18n::tr;
use legible_core::render::render;
use legible_core::tr;

use crate::settings::json_string;

/// Windows' version, as it records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsVersion {
    /// Its release ("24H2"), if it says.
    pub release: Option<String>,
    pub build: u32,
    /// The build's update revision, if it says.
    pub revision: Option<u32>,
}

impl WindowsVersion {
    /// "Windows 11 24H2 (build 26200.6899)": Windows 11 from build 22000 on, 10 before it.
    pub fn text(&self) -> String {
        let major = if self.build >= 22000 { "11" } else { "10" };
        let build = match self.revision {
            Some(revision) => format!("{}.{revision}", self.build),
            None => self.build.to_string(),
        };
        match &self.release {
            Some(release) => tr!(
                "about-windows-release",
                major = major,
                release = release.as_str(),
                build = build
            ),
            None => tr!("about-windows", major = major, build = build),
        }
    }
}

/// What About says of this copy of Legible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub version: String,
    /// A portable copy (its settings beside it), or installed.
    pub portable: bool,
    /// The WebView2 runtime's version, if it could be read.
    pub webview2: Option<String>,
    pub windows: Option<WindowsVersion>,
}

impl Facts {
    /// About's table: each row's label and value.
    pub fn rows(&self) -> Vec<(String, String)> {
        let unknown = || tr("about-unknown");
        vec![
            (tr("about-version"), self.version.clone()),
            (
                tr("about-build"),
                tr(if self.portable {
                    "about-build-portable"
                } else {
                    "about-build-installed"
                }),
            ),
            (
                tr("about-webview2"),
                self.webview2.clone().unwrap_or_else(unknown),
            ),
            (
                tr("about-windows-label"),
                self.windows
                    .as_ref()
                    .map_or_else(unknown, WindowsVersion::text),
            ),
        ]
    }

    /// What Copy puts on the clipboard: "Legible <version>", then the table's other rows.
    pub fn copy_text(&self) -> String {
        let mut lines = vec![tr!(
            "about-copy-title",
            app = tr("app-name"),
            version = self.version.clone()
        )];
        lines.extend(
            self.rows()
                .into_iter()
                .skip(1)
                .map(|(label, value)| tr!("about-copy-row", label = label, value = value)),
        );
        lines.join("\r\n") + "\r\n"
    }

    /// For the page: `{"rows": [[label, value], ...]}`.
    pub fn json(&self) -> String {
        let rows: Vec<String> = self
            .rows()
            .iter()
            .map(|(label, value)| format!("[{},{}]", json_string(label), json_string(value)))
            .collect();
        format!("{{\"rows\":[{}]}}", rows.join(","))
    }
}

/// The texts About opens in its reading view, bundled in the exe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// Legible's own licence (MIT).
    Licence,
    /// The third-party software's notices.
    Notices,
    /// The release notes (CHANGELOG.md).
    Changelog,
}

impl Reading {
    /// The page's name for it (`reading\n<name>`).
    pub fn parse(name: &str) -> Option<Reading> {
        Some(match name {
            "licence" => Reading::Licence,
            "notices" => Reading::Notices,
            "changelog" => Reading::Changelog,
            _ => return None,
        })
    }

    /// The release notes as a document (Legible renders Markdown, its own too); None for the
    /// others, which read as plain text.
    pub fn html(self) -> Option<String> {
        // It has no pictures: the folder they'd be found in is never used.
        (self == Reading::Changelog).then(|| render(self.text(), Path::new(".")).html)
    }

    pub fn text(self) -> &'static str {
        match self {
            Reading::Licence => include_str!("../../../LICENSE"),
            Reading::Notices => include_str!("../../../THIRD-PARTY-NOTICES.txt"),
            Reading::Changelog => include_str!("../../../CHANGELOG.md"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            version: "0.1.0".into(),
            portable: true,
            webview2: Some("156.0.4314.50".into()),
            windows: Some(WindowsVersion {
                release: Some("24H2".into()),
                build: 26200,
                revision: Some(6899),
            }),
        }
    }

    #[test]
    fn the_table_says_the_version_the_build_webview2_and_windows() {
        let rows = facts().rows();
        let rows: Vec<(&str, &str)> = rows.iter().map(|(l, v)| (l.as_str(), v.as_str())).collect();
        assert_eq!(
            rows,
            [
                ("Version", "0.1.0"),
                ("Build", "Portable"),
                ("WebView2", "156.0.4314.50"),
                ("Windows", "Windows 11 24H2 (build 26200.6899)"),
            ]
        );
        let installed = Facts {
            portable: false,
            webview2: None,
            windows: None,
            ..facts()
        };
        let rows = installed.rows();
        assert_eq!(rows[1].1, "Installed");
        assert_eq!(rows[2].1, "Unknown");
        assert_eq!(rows[3].1, "Unknown");
    }

    #[test]
    fn windows_10_and_a_release_that_isnt_said() {
        let ten = WindowsVersion {
            release: None,
            build: 19045,
            revision: None,
        };
        assert_eq!(ten.text(), "Windows 10 (build 19045)");
    }

    #[test]
    fn copy_puts_the_name_and_version_first_then_the_rows() {
        assert_eq!(
            facts().copy_text(),
            "Legible 0.1.0\r\nBuild: Portable\r\nWebView2: 156.0.4314.50\r\nWindows: Windows 11 24H2 (build 26200.6899)\r\n"
        );
    }

    #[test]
    fn the_page_gets_the_rows() {
        assert!(
            facts()
                .json()
                .starts_with(r#"{"rows":[["Version","0.1.0"],["Build","Portable"]"#)
        );
    }

    #[test]
    fn the_readings_are_the_bundled_files() {
        assert_eq!(Reading::parse("licence"), Some(Reading::Licence));
        assert_eq!(Reading::parse("other"), None);
        assert!(Reading::Licence.text().starts_with("MIT License"));
        assert!(Reading::Notices.text().contains("muda"));
        assert!(Reading::Changelog.text().contains("## [Unreleased]"));
        let html = Reading::Changelog
            .html()
            .expect("the release notes rendered");
        assert!(
            html.contains(">Changelog</h1>")
                && html.contains("href=\"https://keepachangelog.com/en/1.1.0/\"")
        );
        assert!(!html.contains("# Changelog"));
        assert_eq!(Reading::Licence.html(), None);
        assert_eq!(Reading::Notices.html(), None);
    }
}
