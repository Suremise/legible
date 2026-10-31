//! Copies of unsaved edits kept as Windows ended an earlier session (`crate::session`,
//! `legible_core::unsaved`), offered back at the start. Each copy's file opens in a window of
//! its own with a bar over the document, "Restore unsaved changes to notes.md?", saying too if
//! the file has changed since. Restore puts the text into the editor as unsaved changes, in
//! Split, and Discard drops it. A file that has gone, or can't be edited where it is, asks
//! instead whether to save the text as a new file. A copy is removed only once
//! the reader has chosen: a window closed with its bar still asking keeps it for next time.

use std::path::Path;

use legible_core::save::{self, NoHooks, TextFormat, WriteError};
use legible_core::unsaved::{self, Kept};
use legible_core::{i18n::tr, tr};
use tao::event_loop::EventLoopWindowTarget;
use tao::window::WindowId;

use super::{Shell, UserEvent};
use crate::disk::Disk;
use crate::editor;
use crate::reason;
use crate::recovery;
use crate::session::{self, Offer};
use crate::settings::json_string;
use crate::win32::{Message, dialogs::SaveAnswer};

/// A copy offered on a window's bar, until the reader chooses.
pub(super) struct Offered {
    kept: Kept,
    /// The file has changed since the copy's edits were made to it.
    changed: bool,
}

impl Shell {
    /// At the start: every copy kept in Legible's folder is offered back.
    pub(super) fn offer_kept(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        let Some(folder) = self.unsaved_folder.clone() else {
            return;
        };
        for kept in unsaved::list(&folder) {
            // The file as it is now, if it's there: a window opens only for the bar.
            let file = std::fs::canonicalize(&kept.document)
                .ok()
                .filter(|f| f.is_file());
            let disk = file.as_deref().and_then(|f| Disk::read(f).ok());
            let editable = file.as_deref().is_some_and(editor::file_is_editable);
            let format = disk.as_ref().and_then(|d| d.content.format()).copied();
            match session::offer(&kept, disk.as_ref(), editable) {
                Offer::Nothing => forget(&kept.copy),
                Offer::Bar { changed } => {
                    let Some(file) = file.filter(|f| self.open(f, target)) else {
                        continue;
                    };
                    let Some(&id) = self.by_file.get(&file) else {
                        continue;
                    };
                    if let Some(doc) = self.windows.get_mut(&id) {
                        doc.offered.push_back(Offered { kept, changed });
                    }
                    // Shown once its page is drawn, and again on a page served again.
                    self.offer_restore(id);
                }
                Offer::Elsewhere => self.kept_elsewhere(kept, format, target),
            }
        }
    }

    /// The window's page shows its bar for the first copy offered, if one is.
    pub(super) fn offer_restore(&self, id: WindowId) {
        let Some(offered) = self.windows.get(&id).and_then(|d| d.offered.front()) else {
            return;
        };
        let mut question = tr!("restore-question", name = name_of(&offered.kept.document));
        if offered.changed {
            let name = name_of(&offered.kept.document);
            question = format!("{question} {}", tr!("restore-changed", name = name));
        }
        self.run_script(
            id,
            &format!(
                "window.LegibleEditor?.offerRestore({})",
                json_string(&question)
            ),
        );
    }

    /// The bar's answer for the first copy offered: Restore puts the text into the editor as
    /// unsaved changes, and the copy goes once it's there; Discard drops it. The next copy (from
    /// another run) is offered then.
    pub(super) fn restore_answer(&mut self, id: WindowId, restore: bool) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let Some(Offered { kept, .. }) = doc.offered.pop_front() else {
            return;
        };
        if restore {
            doc.restoring.push_back(kept.copy);
            let script = format!("window.LegibleEditor?.restore({})", json_string(&kept.text));
            self.run_script(id, &script);
        } else {
            forget(&kept.copy);
        }
        self.offer_restore(id);
    }

    /// The page says a restore's text is in the editor: its copy has done its work.
    pub(super) fn restored(&mut self, id: WindowId) {
        if let Some(copy) = self
            .windows
            .get_mut(&id)
            .and_then(|d| d.restoring.pop_front())
        {
            forget(&copy);
        }
    }

    /// A copy whose file has gone, or can't be edited where it is, asked about before any window
    /// is open: saved as a new file (in the file's own format, if it's known), or dropped, as the
    /// reader chooses; Cancel keeps it for next time.
    fn kept_elsewhere(
        &mut self,
        kept: Kept,
        format: Option<TextFormat>,
        target: &EventLoopWindowTarget<UserEvent>,
    ) {
        let name = name_of(&kept.document);
        let answer = self.ask(
            0,
            &tr("app-name"),
            &tr!("restore-question", name = name.clone()),
            &tr!("restore-elsewhere", name = name),
            &tr("restore-save-as"),
            &tr("restore-discard"),
        );
        match answer {
            SaveAnswer::Save => {}
            SaveAnswer::DontSave => return forget(&kept.copy),
            SaveAnswer::Cancel => return,
        }
        let Some(chosen) = self.save_as_dialog(0, &kept.document) else {
            return;
        };
        let bytes = match &format {
            Some(format) => save::encode(&kept.text, format),
            None => kept.text.clone().into_bytes(),
        };
        // Never over the file itself, and only over what the dialog approved.
        let written = recovery::approve(&chosen)
            .map_err(|error| {
                tracing::warn!(%error, "couldn't check where to save unsaved edits kept");
                reason::of(&error)
            })
            .and_then(|approved| {
                recovery::write_copy(&chosen, &bytes, approved, &kept.document, &NoHooks)
                    .map_err(not_copied)
            });
        match written {
            Ok(_) => {
                forget(&kept.copy);
                self.open(&chosen, target);
            }
            Err(reason) => {
                tracing::warn!(reason, "couldn't save unsaved edits kept as a new file");
                self.message_box(
                    None,
                    &tr("app-name"),
                    &tr!("restore-failed", reason = reason),
                    Message::Error,
                );
            }
        }
    }
}

/// Why unsaved edits kept weren't written as a new file, for the reader.
fn not_copied(not: recovery::NotCopied) -> String {
    let error = match not {
        recovery::NotCopied::Same => return tr("chrome-save-as-same"),
        recovery::NotCopied::Write(error) => error,
    };
    tracing::warn!(?error, "couldn't write unsaved edits kept");
    match error {
        WriteError::Failed(error) | WriteError::Partial { error, .. } => reason::of(&error),
        // Something is there now that wasn't when it was chosen.
        WriteError::Conflict(_) | WriteError::Raced { .. } => tr("chrome-save-as-changed"),
    }
}

/// A document's file name, as the bar and the question name it.
fn name_of(document: &Path) -> String {
    document
        .file_name()
        .unwrap_or(document.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// The reader has chosen: the copy goes.
fn forget(copy: &Path) {
    if let Err(error) = unsaved::forget(copy) {
        tracing::warn!(%error, copy = %copy.display(), "couldn't remove a copy of unsaved edits");
    }
}
