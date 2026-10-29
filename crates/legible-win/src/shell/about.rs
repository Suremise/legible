//! About Legible ([`crate::about`]), carried out: its facts, Copy, and the texts it opens.

use tao::window::WindowId;

use super::{Shell, webview2_version};
use crate::about::{Facts, Reading};
use crate::settings::json_string;
use crate::win32;

impl Shell {
    /// What About says of this copy of Legible, as it is now.
    fn facts(&self) -> Facts {
        Facts {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            portable: self.portable,
            webview2: webview2_version(),
            windows: win32::registry::windows_version(),
        }
    }

    /// About opened in window `id`: its table's rows.
    pub(super) fn send_about(&self, id: WindowId) {
        let script = format!("window.LegibleChrome?.about({})", self.facts().json());
        self.run_script(id, &script);
    }

    /// About's Copy: the version information (as the window has it, never the page's text)
    /// goes on the clipboard, and About says so.
    pub(super) fn copy_about(&mut self, id: WindowId) {
        let text = self.facts().copy_text();
        if self.copy_text(id, &text) {
            self.run_script(id, "window.LegibleChrome?.aboutCopied()");
        }
    }

    /// One of the texts About opens (the licence, the notices, the release notes), by name:
    /// the release notes rendered as a document too.
    pub(super) fn send_reading(&self, id: WindowId, name: &str) {
        let Some(reading) = Reading::parse(name) else {
            return tracing::warn!(name, "the page asked for a text Legible doesn't have");
        };
        let html = reading
            .html()
            .map_or_else(|| "null".to_owned(), |html| json_string(&html));
        let script = format!(
            "window.LegibleChrome?.reading({},{},{html})",
            json_string(name),
            json_string(reading.text())
        );
        self.run_script(id, &script);
    }
}
