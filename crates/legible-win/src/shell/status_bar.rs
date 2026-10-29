//! A window's status bar ([`crate::status`]), carried out: what it shows of the window's file,
//! and Copy path.

use legible_core::i18n::tr;
use legible_core::tr;
use tao::platform::windows::WindowExtWindows;
use tao::window::WindowId;

use super::Shell;
use crate::clipboard::{self, Written};
use crate::jumplist;
use crate::status::{self, Origin};
use crate::win32;

impl Shell {
    /// Tells window `id`'s page what its status bar shows of its file (the page asks as it
    /// starts, and is told again when the file changes on disk).
    pub(super) fn send_status(&self, id: WindowId) {
        let Some(doc) = self.windows.get(&id) else {
            return;
        };
        let temp = std::fs::canonicalize(std::env::temp_dir()).ok();
        let origin = doc
            .file
            .as_deref()
            .and_then(|f| Origin::of(f, temp.as_deref()));
        let format = doc.disk.as_ref().and_then(|d| d.content.format());
        let json = status::json(doc.file.as_deref(), origin, format);
        self.run_script(id, &format!("window.LegibleChrome?.status({json})"));
    }

    /// The status bar's path was clicked: the window's own file's full path (never one from
    /// the page) goes on the clipboard as text, the newest copy of the process's.
    pub(super) fn copy_path(&mut self, id: WindowId) {
        let Some(doc) = self.windows.get(&id) else {
            return;
        };
        let Some(file) = &doc.file else {
            return;
        };
        let text = jumplist::display_path(file).to_string_lossy().into_owned();
        if self.copy_text(id, &text) {
            self.run_script(id, "window.LegibleChrome?.pathCopied()");
        }
    }

    /// Puts `text` on the clipboard for window `id`, as the newest copy of the process's (a
    /// copy still being built or waiting is older, and won't be written). Whether it's there:
    /// if not, the window says why.
    pub(super) fn copy_text(&mut self, id: WindowId, text: &str) -> bool {
        let Some(owner) = self.windows.get(&id).map(|d| d.window.hwnd()) else {
            return false;
        };
        self.copy_order.accept();
        let written = match &self.stand_in {
            Some(stand_in) => stand_in.clipboard(b"", text),
            None => {
                let mut port = win32::clipboard::WindowsClipboard::new(owner);
                clipboard::write(&mut port, b"", text)
            }
        };
        let reason = match written {
            Written::Both | Written::TextOnly => return true,
            Written::Busy => tr("chrome-copy-busy"),
            Written::Failed(reason) => reason,
        };
        self.notice(id, &tr!("chrome-copy-failed", reason = reason));
        false
    }
}
