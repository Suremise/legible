//! A window's saving, carried out: Save, Save as, closing with unsaved edits, and the
//! conflict bar's answers. [`crate::saving::Session`] decides what to write and whether the
//! window may close; `legible_core::save` writes, off the UI thread.
//!
//! **A save** writes the editor's text at a version, in the file's own format, checked against
//! the stamp of the file as the window last took it (`DocWindow::expected`). Only a write that
//! is still the file afterwards (its bytes read back) makes that version saved and the window's
//! file the one written; then live reload starts a new epoch from it, so its own write never
//! comes back as a change. Anything else leaves the text unsaved: a change found first is held
//! as a conflict (the bar asks), a failure is said.
//!
//! **A raced save** (another program wrote the file in the moment between the check and the
//! replace) is saved, and their bytes are kept beside the file, under a name that says what
//! they are: the bar says both versions are kept. The copy is never removed; Take theirs puts
//! it back only while the file is still the save's ([`crate::recovery`] does the file work),
//! with the page frozen from its confirmation until the take is over.
//!
//! **Save as** writes the editor's text, or the file's bytes, to a file chosen in the Save
//! dialog, over exactly what was there when it was chosen (never the file itself), and the
//! window then shows that file in place of its own, as Open does ([`Shell::replace`]): its
//! title, live reload, recent files and whether it can be edited are the new file's. The page
//! is frozen from the moment its text is asked for, so nothing typed meanwhile is left behind;
//! if the write fails it takes edits again, its text as it was.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use legible_core::render::render;
use legible_core::save::{self, Conflict, NoHooks, Stamp, TextFormat, WriteError};
use legible_core::{i18n::tr, tr};
use tao::platform::windows::WindowExtWindows;
use tao::window::WindowId;

use super::{Moments, OnDisk, Shell, UserEvent, Write, pictures};
use crate::consent::Next;
use crate::disk::{Content, Disk};
use crate::editor::{self, RaceAnswer};
use crate::page;
use crate::reason;
use crate::recovery;
use crate::reload::{Action, Kind};
use crate::saving::{Choice, Close, Outcome, Save};
use crate::settings::json_string;
use crate::watch::Watched;
use crate::win32::{self, dialogs};

/// What the editor's contents were asked for.
pub(super) enum Wanted {
    Save,
    SaveAs(NewFile),
}

/// Save as's file: where, and what was there when the Save dialog approved it.
pub(super) struct NewFile {
    path: PathBuf,
    approved: Option<Stamp>,
}

/// A write under way: what it writes, to take as saved once it's the file.
pub(super) struct Writing {
    version: u64,
    text: String,
    format: TextFormat,
    len: u64,
    hash: u64,
}

/// A save another program raced: their bytes, kept in `copy` beside the file.
pub(super) struct Race {
    id: u64,
    copy: PathBuf,
    /// The save's own stamp, read back once it was written: Keep mine and Take theirs act only
    /// while the file is still that. None: it wasn't the save's even then.
    ours: Option<Stamp>,
    /// Take theirs is being written.
    pub(super) taking: bool,
}

impl Shell {
    /// Ctrl+S. A file that can't be saved where it is offers Save as instead; a window whose
    /// editor hasn't opened has nothing to save.
    pub(super) fn save(&mut self, id: WindowId) {
        let Some(doc) = self.windows.get(&id) else {
            return;
        };
        if doc.file.is_none() {
            return;
        }
        if !doc.editable {
            return self.save_as(id);
        }
        if doc.editor_open {
            self.ask_contents(id, Wanted::Save);
        }
    }

    fn ask_contents(&mut self, id: WindowId, wanted: Wanted) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        doc.next_ask += 1;
        let request = doc.next_ask;
        doc.asked.insert(request, wanted);
        self.run_script(id, &format!("window.LegibleEditor?.contents({request})"));
    }

    /// The editor's answer to request `request`: its text at `version`.
    pub(super) fn contents(&mut self, id: WindowId, request: u64, version: u64, text: &str) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        match doc.asked.remove(&request) {
            Some(Wanted::Save) => {
                let save = Save {
                    version,
                    text: text.to_owned(),
                };
                if let Some(save) = doc.saves.save(save) {
                    self.write(id, save);
                }
            }
            Some(Wanted::SaveAs(new_file)) => {
                let format = doc.disk.as_ref().and_then(|d| d.content.format()).copied();
                // This file's text stays as saved or unsaved as it was: the new file is another.
                self.abandoned(id, version);
                match format {
                    Some(format) => self.write_new(id, new_file, save::encode(text, &format)),
                    None => self.not_saved_as(id, None),
                }
            }
            None => tracing::debug!(request, "contents nobody asked for"),
        }
    }

    /// Writes `save` on a thread of its own: its answer is [`UserEvent::Written`].
    fn write(&mut self, id: WindowId, save: Save) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let (Some(file), Some(format)) = (
            doc.file.clone(),
            doc.disk.as_ref().and_then(|d| d.content.format()).copied(),
        ) else {
            // Nothing Legible can save as it was (the file on disk is in another encoding
            // now): never written.
            let done = doc.saves.done(save.version, Outcome::Failed);
            self.abandoned(id, save.version);
            self.notice(id, &tr("chrome-encoding-unsupported"));
            return self.carry_out(id, done.close);
        };
        let bytes = save::encode(&save.text, &format);
        let expected = doc.expected;
        doc.writing = Some(Writing {
            version: save.version,
            text: save.text,
            format,
            len: bytes.len() as u64,
            hash: save::hash(&bytes),
        });
        let proxy = self.proxy.clone();
        let version = save.version;
        let moments = self.moments(Write::Save);
        thread::spawn(move || {
            let written = save::write(
                &file,
                &bytes,
                expected.as_ref(),
                &win32::replace::Windows,
                &*moments,
            );
            moments.over();
            let _ = proxy.send_event(UserEvent::Written(id, version, written));
        });
    }

    /// The write of `version` is over.
    pub(super) fn written(
        &mut self,
        id: WindowId,
        version: u64,
        written: Result<Stamp, WriteError>,
    ) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let Some(writing) = doc.writing.take_if(|w| w.version == version) else {
            return;
        };
        let name = doc.title_name();
        let outcome = match written {
            Ok(_) => self.saved(id, writing),
            Err(WriteError::Raced { backup, .. }) => {
                let outcome = self.saved(id, writing);
                // Its stamp is the other writer's, at the backup; the save's own is what
                // `saved` read back, if it was the save's.
                let ours = self
                    .windows
                    .get(&id)
                    .and_then(|d| d.expected)
                    .filter(|_| outcome == Outcome::Saved);
                self.raced(id, backup, ours);
                outcome
            }
            Err(WriteError::Conflict(conflict)) => {
                self.abandoned(id, version);
                self.save_conflict(id, conflict);
                Outcome::Conflict
            }
            Err(WriteError::Failed(error)) => {
                tracing::warn!(%error, "a save failed");
                self.abandoned(id, version);
                let reason = reason::of(&error);
                self.notice(
                    id,
                    &tr!("chrome-save-file-failed", name = name, reason = reason),
                );
                Outcome::Failed
            }
            Err(WriteError::Partial {
                original_at,
                saved_at,
                error,
            }) => {
                tracing::error!(%error, ?original_at, ?saved_at, "a save was left half-done");
                self.abandoned(id, version);
                let text = tr!(
                    "chrome-save-partial",
                    name = name,
                    original = original_at.display().to_string(),
                    saved = saved_at.display().to_string()
                );
                self.notice(id, &text);
                Outcome::Failed
            }
        };
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let done = doc.saves.done(version, outcome);
        if let Some(next) = done.next {
            self.write(id, next);
        }
        self.carry_out(id, done.close);
    }

    /// A write is over and replaced the file: if the file is still exactly what was written,
    /// that's what the window has from now on, and the version is saved. If something else is
    /// there already, the text stays unsaved, and that change is held as a conflict.
    fn saved(&mut self, id: WindowId, writing: Writing) -> Outcome {
        let Some(doc) = self.windows.get_mut(&id) else {
            return Outcome::Failed;
        };
        let Some(file) = doc.file.clone() else {
            return Outcome::Failed;
        };
        // Taken before the file is read back, so a change after it is live reload's to report.
        let snapshot = self.watch.document_snapshot(&file);
        let read = save::read(&file, &win32::identity);
        let Some(doc) = self.windows.get_mut(&id) else {
            return Outcome::Failed;
        };
        let stamp = match read {
            Ok((_, stamp)) if stamp.len == writing.len && stamp.hash == writing.hash => stamp,
            _ => {
                self.abandoned(id, writing.version);
                self.save_conflict(id, Conflict::Changed);
                return Outcome::Conflict;
            }
        };
        // The window's own render is the saved text's from now on: a page served again (a
        // reload that waited for this save) shows it, and its pictures are the ones watched.
        let folder = file.parent().unwrap_or(&file);
        let html = render(&writing.text, folder).html;
        doc.disk = Some(Disk {
            content: Content::Text {
                text: writing.text,
                format: TextFormat {
                    mixed: false,
                    ..writing.format
                },
            },
            stamp,
        });
        doc.expected = Some(stamp);
        doc.pictures.render(&html);
        doc.document = html;
        if let Some(key) = doc.watch_key {
            let watched = Watched {
                document: file,
                root: doc.root.clone(),
                pictures: pictures(doc.root.as_ref(), &doc.document),
            };
            doc.watch_epoch = self.watch.register(key, watched, snapshot);
        }
        // What its pictures from the internet may do now: a CSP that has to change waits, as
        // any reload does, for the text to be clean, which markSaved makes it. Otherwise the
        // generation stays (the page's preview is already of this text, and its answers, a
        // copy or the pictures bar's, are for that generation), and the page is told what the
        // saved text's pictures may do: a new picture asks, one already let in shows. Told
        // after markSaved, so the bar asks rather than saying to save first.
        let tell = match self.grant_pictures(id) {
            Some(Next::ServeAgain(origins)) => {
                self.serve_again(id, origins);
                false
            }
            Some(Next::Tell) => true,
            None => false,
        };
        self.run_script(
            id,
            &format!("window.LegibleEditor?.markSaved({})", writing.version),
        );
        // Saving wrote one kind of line ending: the status bar no longer says they're mixed.
        self.send_status(id);
        if tell {
            self.send_pictures(id);
        }
        Outcome::Saved
    }

    fn abandoned(&self, id: WindowId, version: u64) {
        self.run_script(
            id,
            &format!("window.LegibleEditor?.saveAbandoned({version})"),
        );
    }

    /// A save found the file changed (or gone, or there again): nothing was written, and that
    /// change is held, as live reload holds one, for the bar to ask about. "Keep mine" for it
    /// saves again.
    fn save_conflict(&mut self, id: WindowId, conflict: Conflict) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let kind = match conflict {
            Conflict::Deleted => Kind::Deleted,
            Conflict::Changed | Conflict::Appeared => Kind::Changed,
        };
        let on_disk = match (kind, &doc.file) {
            (Kind::Changed, Some(file)) => match Disk::read(file) {
                Ok(disk) => {
                    let folder = file.parent().unwrap_or(file);
                    let html = render(disk.content.text(), folder).html;
                    Some(OnDisk { disk, html })
                }
                // Read again when it's taken (live reload rescans).
                Err(error) => {
                    tracing::info!(%error, "a file that changed couldn't be read yet");
                    None
                }
            },
            _ => None,
        };
        match doc.reload.on_disk(kind) {
            Action::Hold { kind, id: n } => {
                doc.held = on_disk;
                doc.saves.conflict(n);
                self.announce(id, kind.as_str(), n, None);
            }
            // The edits were undone meanwhile: the file is shown as it now is.
            Action::Apply(Kind::Changed) => match on_disk {
                Some(on_disk) => self.show_from_disk(id, on_disk),
                None => {
                    if let Some(key) = doc.watch_key {
                        self.watch.rescan(key);
                    }
                }
            },
            Action::Apply(Kind::Deleted) => self.file_gone(id),
        }
    }

    /// Tells the page's conflict bar of change `n` (`kind`: changed, deleted or raced).
    /// `text` is what the bar says; without it, the bar's own words for `kind`, naming the file.
    pub(super) fn announce(&self, id: WindowId, kind: &str, n: u64, text: Option<&str>) {
        let file = self
            .windows
            .get(&id)
            .map(|d| d.title_name())
            .unwrap_or_default();
        let text = match text {
            Some(text) => text.to_owned(),
            None if kind == "deleted" => tr!("chrome-conflict-deleted", file = file),
            None => tr!("chrome-conflict-changed", file = file),
        };
        let text = json_string(&text);
        let script = format!(
            "window.dispatchEvent(new CustomEvent('legible:external-change', {{ detail: {{ kind: {}, id: {n}, text: {text} }} }}))",
            json_string(kind)
        );
        self.run_script(id, &script);
    }

    /// "Keep mine" for held change `n`, which the window has dropped: the next save writes over
    /// what's on disk now (or puts back a file that went), and a save that found it goes again.
    pub(super) fn kept(&mut self, id: WindowId, n: u64, held: Option<(Kind, Option<OnDisk>)>) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        match held {
            Some((Kind::Changed, Some(on_disk))) => doc.expected = Some(on_disk.disk.stamp),
            Some((Kind::Deleted, _)) => doc.expected = None,
            // Changed, but not read: what's there is read again at the next save's check.
            Some((Kind::Changed, None)) => {
                if let Some(file) = &doc.file
                    && let Ok((_, stamp)) = save::read(file, &win32::identity)
                {
                    doc.expected = Some(stamp);
                }
            }
            None => return,
        }
        if doc.saves.kept(n) {
            self.save(id);
        }
    }

    /// Compare, for held change `n`: the version on disk, in the preview.
    pub(super) fn compare(&mut self, id: WindowId, n: u64) {
        let Some(doc) = self.windows.get(&id) else {
            return;
        };
        if doc.reload.pending() != Some((Kind::Changed, n)) {
            return;
        }
        let Some(text) = doc.held.as_ref().map(|h| h.disk.content.text().to_owned()) else {
            return;
        };
        self.send_compare(id, "changed", n, &text);
    }

    fn send_compare(&self, id: WindowId, kind: &str, n: u64, text: &str) {
        let Some(doc) = self.windows.get(&id) else {
            return;
        };
        let Some(file) = doc.file.as_deref() else {
            return;
        };
        let folder = file.parent().unwrap_or(file);
        let html = page::with_generation(&render(text, folder).html, doc.generation);
        let script = format!(
            "window.LegibleEditor?.compare({},{n},{},{{\"generation\":{},\"remote\":{}}})",
            json_string(kind),
            json_string(&html),
            doc.generation,
            self.pictures_message(id)
        );
        self.run_script(id, &script);
    }

    // --- A raced save --------------------------------------------------------------------------

    /// A save was raced: theirs is kept, under a name the reader can find beside the file,
    /// and the bar says so.
    /// `ours`: the document's stamp as the window took it after the write, if it was the
    /// write's own; Take theirs acts only while the document is still that.
    fn raced(&mut self, id: WindowId, backup: PathBuf, ours: Option<Stamp>) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let copy = doc
            .file
            .as_deref()
            .and_then(|file| recovery::keep_as(file, &backup))
            .unwrap_or(backup);
        tracing::warn!(?copy, "another program saved the file at the same moment");
        doc.races += 1;
        let n = doc.races;
        // An older race's copy stays where it is: Legible never removes one.
        doc.race = Some(Race {
            id: n,
            copy,
            ours,
            taking: false,
        });
        self.announce_race(id);
    }

    /// The bar's answer for raced save `n`. Take theirs acts only while the file is still the
    /// save's own; their copy is never removed.
    ///
    /// The page froze its text when Take theirs was confirmed, so nothing typed after the
    /// confirmation can be replaced: it's unfrozen once the take is over, however it went.
    pub(super) fn race_answer(&mut self, id: WindowId, answer: RaceAnswer, n: u64) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let current = doc.race.as_ref().filter(|r| r.id == n && !r.taking);
        let (Some(race), Some(file)) = (current, doc.file.clone()) else {
            tracing::info!(n, "an answer for a raced save that isn't the newest");
            if answer == RaceAnswer::Take {
                self.unfreeze_after_take(id);
            }
            return;
        };
        let copy = race.copy.clone();
        let name = doc.title_name();
        match answer {
            RaceAnswer::Compare => match std::fs::read(&copy) {
                Ok(bytes) => {
                    let text = Content::from_bytes(bytes).text().to_owned();
                    self.send_compare(id, "raced", n, &text);
                }
                Err(error) => {
                    tracing::warn!(%error, "couldn't read the other version to compare");
                    let not = recovery::NotTaken::Unreadable(error);
                    self.notice(id, &recovery::not_taken_note(&name, &copy, &not));
                }
            },
            RaceAnswer::Keep => {
                doc.race = None;
                self.notice(id, &recovery::kept_note(&name, &copy));
            }
            RaceAnswer::Take => {
                let ours = race.ours;
                if let Some(race) = doc.race.as_mut() {
                    race.taking = true;
                }
                let proxy = self.proxy.clone();
                let moments = self.moments(Write::Take);
                thread::spawn(move || {
                    let taken = recovery::take_theirs(&file, &copy, ours, &*moments);
                    moments.over();
                    let _ = proxy.send_event(UserEvent::RaceTaken(id, n, taken));
                });
            }
        }
    }

    /// Take theirs is over: their version is the file now (and the editor's text), or nothing
    /// changed and the reader is told why.
    pub(super) fn race_taken(
        &mut self,
        id: WindowId,
        n: u64,
        taken: Result<recovery::Taken, recovery::NotTaken>,
    ) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let Some(race) = doc.race.take_if(|r| r.id == n) else {
            return;
        };
        let name = doc.title_name();
        let taken = match taken {
            Ok(taken) => taken,
            Err(not) => {
                tracing::warn!(?not, "the other version wasn't put back");
                let note = recovery::not_taken_note(&name, &race.copy, &not);
                // Asked again: both versions are still where they were.
                if !matches!(not, recovery::NotTaken::Changed) {
                    doc.race = Some(Race {
                        taking: false,
                        ..race
                    });
                }
                self.notice(id, &note);
                return self.after_take(id);
            }
        };
        let Some(file) = doc.file.clone() else {
            return self.after_take(id);
        };
        // As after a save: the snapshot first, then the file read back. Only if it's exactly
        // what was taken is it the window's file (its stamp the document's own); if another
        // program has written it since, that's a change like any other, and nothing is marked.
        let snapshot = self.watch.document_snapshot(&file);
        let Some(stamp) = recovery::installed(&file, &taken.bytes) else {
            self.notice(id, &recovery::taken_note(&name, &race.copy));
            if let Some(backup) = taken.raced {
                self.raced(id, backup, None);
            }
            self.save_conflict(id, Conflict::Changed);
            return self.after_take(id);
        };
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        // Theirs is the file now, as live reload would have shown it: in the editor too.
        let disk = Disk::of(taken.bytes, stamp);
        doc.expected = Some(stamp);
        let folder = file.parent().unwrap_or(&file);
        let html = render(disk.content.text(), folder).html;
        if let Some(key) = doc.watch_key {
            let watched = Watched {
                document: file,
                root: doc.root.clone(),
                pictures: pictures(doc.root.as_ref(), &html),
            };
            doc.watch_epoch = self.watch.register(key, watched, snapshot);
        }
        self.show_from_disk(id, OnDisk { disk, html });
        self.notice(id, &recovery::taken_note(&name, &race.copy));
        if let Some(backup) = taken.raced {
            self.raced(id, backup, Some(stamp));
        }
        self.after_take(id);
    }

    /// A take is over: the page takes edits again, and a close asked for meanwhile goes on.
    fn after_take(&mut self, id: WindowId) {
        self.unfreeze_after_take(id);
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        if std::mem::take(&mut doc.close_after_take) {
            self.ask_close(id);
        }
    }

    /// The page froze itself for a take: it takes edits again (unless a close's save has it
    /// frozen, which unfreezes it itself).
    pub(super) fn unfreeze_after_take(&self, id: WindowId) {
        if self.windows.get(&id).is_some_and(|d| !d.saves.frozen()) {
            self.run_script(id, "window.LegibleEditor?.unfreeze()");
        }
    }

    /// Shows the raced save's bar (again, on a page served again).
    pub(super) fn announce_race(&self, id: WindowId) {
        let Some(race) = self.windows.get(&id).and_then(|d| d.race.as_ref()) else {
            return;
        };
        let file = self
            .windows
            .get(&id)
            .map(|d| d.title_name())
            .unwrap_or_default();
        let text = tr!(
            "chrome-conflict-raced",
            file = file,
            name = recovery::name_of(&race.copy)
        );
        self.announce(id, "raced", race.id, Some(&text));
    }

    // --- Save as -------------------------------------------------------------------------------

    /// Save as… (Ctrl+Shift+S, the toolbar's Save as, or Ctrl+S on a file that can't be saved
    /// where it is): the Save dialog, then the editor's text, or the file as it is, to the file
    /// chosen, over exactly what was there when it was chosen; then the window shows that file
    /// ([`Shell::saved_as`]). Choosing the file itself is Save, where it can be saved; never a
    /// file open in another window.
    pub(super) fn save_as(&mut self, id: WindowId) {
        let busy = self.closing.contains_key(&id);
        // A window still reading its file has nothing yet to save: no dialog. Nor does one
        // that's closing, replacing its document, taking a raced save's other version, or
        // saving as already.
        let Some(doc) = self.windows.get(&id).filter(|d| {
            d.opening.is_none()
                && !busy
                && d.replacing.is_none()
                && !d.saves.closing()
                && !d.saving_as
                && !d.race.as_ref().is_some_and(|r| r.taking)
        }) else {
            return;
        };
        let Some(file) = doc.file.clone() else {
            return;
        };
        let Some(chosen) = self.save_as_dialog(doc.window.hwnd(), &file) else {
            return;
        };
        if recovery::same_file(&chosen, &file) {
            if self.windows.get(&id).is_some_and(|d| d.editable) {
                return self.save(id);
            }
            return self.notice(id, &tr("chrome-save-as-same"));
        }
        if let Some(open) = std::fs::canonicalize(&chosen)
            .ok()
            .and_then(|c| self.by_file.get(&c).copied())
        {
            let name = recovery::name_of(&chosen);
            self.focus(open);
            return self.notice(id, &tr!("chrome-save-as-open", name = name));
        }
        // As the dialog approved it: whatever is there later, or appears, isn't written over.
        let approved = match recovery::approve(&chosen) {
            Ok(approved) => approved,
            Err(error) => {
                tracing::warn!(%error, "couldn't check Save as's destination");
                let reason = reason::of(&error);
                return self.notice(id, &tr!("chrome-save-as-failed", reason = reason));
            }
        };
        let new_file = NewFile {
            path: chosen,
            approved,
        };
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        doc.saving_as = true;
        if doc.editor_open && doc.editable {
            // Frozen first: the text asked for next is the last the editor takes, so nothing
            // typed meanwhile is lost when the window moves to the new file.
            self.run_script(id, "window.LegibleEditor?.freeze()");
            return self.ask_contents(id, Wanted::SaveAs(new_file));
        }
        let Some(disk) = &doc.disk else {
            doc.saving_as = false;
            return;
        };
        let bytes = match &disk.content {
            Content::Text { text, format } => save::encode(text, format),
            Content::Unsupported { bytes, .. } => bytes.clone(),
        };
        self.write_new(id, new_file, bytes);
    }

    /// Writes Save as's file on a thread of its own. Its answer is [`UserEvent::SavedAs`].
    fn write_new(&self, id: WindowId, new_file: NewFile, bytes: Vec<u8>) {
        let Some(source) = self.windows.get(&id).and_then(|d| d.file.clone()) else {
            return;
        };
        let proxy = self.proxy.clone();
        let moments = self.moments(Write::SaveAs);
        thread::spawn(move || {
            let written = recovery::write_copy(
                &new_file.path,
                &bytes,
                new_file.approved,
                &source,
                &*moments,
            );
            moments.over();
            let _ = proxy.send_event(UserEvent::SavedAs(id, new_file.path, written));
        });
    }

    /// Save as's file is written, and the window shows it in place of its own document: what
    /// the reader had there is in the new file, so nothing is asked, and the old file stays as
    /// it was on disk. Or it couldn't be written, and the window stays as it was, taking edits
    /// again. A close asked for meanwhile goes on now.
    pub(super) fn saved_as(
        &mut self,
        id: WindowId,
        path: PathBuf,
        written: Result<Stamp, recovery::NotCopied>,
    ) {
        let Some(doc) = self.windows.get_mut(&id).filter(|d| d.saving_as) else {
            return;
        };
        let close = std::mem::take(&mut doc.close_after_save_as);
        self.after_save_as(id, path, written);
        if close {
            self.ask_close(id);
        }
    }

    /// Save as's write is over: the window moves to the new file, or stays as it was.
    fn after_save_as(
        &mut self,
        id: WindowId,
        path: PathBuf,
        written: Result<Stamp, recovery::NotCopied>,
    ) {
        let name = recovery::name_of(&path);
        let error = match written {
            Ok(_) => return self.switch_to(id, &path),
            Err(recovery::NotCopied::Same) => {
                return self.not_saved_as(id, Some(tr("chrome-save-as-same")));
            }
            Err(recovery::NotCopied::Write(error)) => error,
        };
        let text = match error {
            WriteError::Raced { backup, .. } => {
                // Written, and theirs kept beside it: the window moves to it all the same.
                let backup = recovery::keep_as(&path, &backup).unwrap_or(backup);
                let text = tr!(
                    "chrome-conflict-raced",
                    file = name,
                    name = recovery::name_of(&backup)
                );
                self.switch_to(id, &path);
                return self.notice(id, &text);
            }
            WriteError::Conflict(_) => tr("chrome-save-as-changed"),
            WriteError::Failed(error) => {
                tracing::warn!(%error, "Save as failed");
                let reason = reason::of(&error);
                tr!("chrome-save-as-failed", reason = reason)
            }
            WriteError::Partial {
                original_at,
                saved_at,
                ..
            } => tr!(
                "chrome-save-partial",
                name = name,
                original = original_at.display().to_string(),
                saved = saved_at.display().to_string()
            ),
        };
        self.not_saved_as(id, Some(text));
    }

    /// Save as is over without a new file: the window stays as it was, its page taking edits
    /// again, and says why (`text`).
    fn not_saved_as(&mut self, id: WindowId, text: Option<String>) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        doc.saving_as = false;
        if doc.editor_open {
            self.run_script(id, "window.LegibleEditor?.unfreeze()");
        }
        if let Some(text) = text {
            self.notice(id, &text);
        }
    }

    /// The window shows `path`, just written by Save as, in place of its document: everything
    /// that followed the old file goes with it, as when Open replaces a document. Its unsaved
    /// edits are the new file's text, so none is asked about; where the reader was in View is
    /// carried over, and Split or Edit stays.
    fn switch_to(&mut self, id: WindowId, path: &Path) {
        let Some(file) = self.found(path) else {
            return self.not_saved_as(id, None);
        };
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let place = doc.place.clone();
        doc.nav_guard.clear();
        self.replace(id, file);
        if let Some(doc) = self.windows.get_mut(&id)
            && doc.go_to.is_none()
        {
            doc.go_to = place;
        }
    }

    // --- Closing -------------------------------------------------------------------------------

    /// Every way a window closes: with unsaved edits, it asks first. While Take theirs or Save
    /// as is being written, the close waits for it. A close wins over a document waiting to replace
    /// the window's ([`Shell::open_here`]): the window closes, as the reader asked.
    pub(super) fn ask_close(&mut self, id: WindowId) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        doc.replacing = None;
        if doc.race.as_ref().is_some_and(|r| r.taking) {
            doc.close_after_take = true;
            return;
        }
        // The page is frozen for Save as's text, which is about to be the window's new file:
        // asked now, Cancel would unfreeze it, and an edit then would be lost with the old page.
        if doc.saving_as {
            doc.close_after_save_as = true;
            return;
        }
        let dirty = doc.reload.is_dirty() || doc.nav_guard.dirty();
        let close = doc.saves.close(dirty);
        self.carry_out(id, close);
    }

    /// The frozen page's answer: whether anything is unsaved.
    pub(super) fn clean(&mut self, id: WindowId, clean: bool) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let clean = clean && !doc.reload.is_dirty() && !doc.nav_guard.dirty();
        let close = doc.saves.clean(clean);
        self.carry_out(id, close);
    }

    /// Carries out what a close comes to; a document waiting to replace the window's
    /// ([`Shell::open_here`]) goes the same way, and doesn't once the window stays as it was.
    pub(super) fn carry_out(&mut self, id: WindowId, close: Close) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        match close {
            Close::Now => self.close(id),
            Close::Ask => {
                // Asked with the text taking edits: a check that froze it is over, and Cancel
                // leaves it as it was.
                if doc.editor_open {
                    self.run_script(id, "window.LegibleEditor?.unfreeze()");
                }
                let Some(doc) = self.windows.get(&id) else {
                    return;
                };
                let name = doc.title_name();
                let answer = self.ask(
                    doc.window.hwnd(),
                    &doc.title,
                    &tr!("save-close-question", name = name),
                    &tr("save-close-detail"),
                    &tr("save-close-save"),
                    &tr("save-close-dont-save"),
                );
                let choice = match answer {
                    dialogs::SaveAnswer::Save => Choice::Save,
                    dialogs::SaveAnswer::DontSave => Choice::DontSave,
                    dialogs::SaveAnswer::Cancel => Choice::Cancel,
                };
                // Cancel, as Windows ends the session: the reader is going back to their work.
                // Cancel, as a document waits to replace this one: it stays as it was.
                if choice == Choice::Cancel {
                    self.round.stop();
                    if let Some(doc) = self.windows.get_mut(&id) {
                        doc.replacing = None;
                    }
                }
                let Some(doc) = self.windows.get_mut(&id) else {
                    return;
                };
                let next = doc.saves.choose(choice);
                self.carry_out(id, next);
            }
            Close::FreezeAndSave => {
                if !doc.editor_open {
                    let next = doc.saves.close_save_failed();
                    return self.carry_out(id, next);
                }
                // Frozen first: the text asked for next is the last the editor takes.
                self.run_script(id, "window.LegibleEditor?.freeze()");
                self.ask_contents(id, Wanted::Save);
            }
            // The page freezes itself before it answers (closeCheck), so nothing can be typed
            // between its answer and the close.
            Close::CheckClean => {
                if doc.editor_open {
                    self.run_script(id, "window.LegibleEditor?.closeCheck()");
                } else {
                    self.clean(id, true);
                }
            }
            // The window stays as it was, its document too.
            Close::Unfreeze => {
                doc.replacing = None;
                self.run_script(id, "window.LegibleEditor?.unfreeze()");
            }
            Close::Wait | Close::Stay => {}
        }
    }

    /// Save as's dialog for `file`, owned by `owner` (0: none): named as the file is, in its own
    /// folder, or in Documents when that's the temp folder or a cache (an attachment's or a
    /// download's, [`editor::saves_beside`]) or has gone. None: cancelled.
    pub(super) fn save_as_dialog(&self, owner: isize, file: &Path) -> Option<PathBuf> {
        if let Some(stand_in) = &self.stand_in {
            return stand_in.save_as_to();
        }
        let stem = file
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let extension = file
            .extension()
            .map_or("md".to_owned(), |e| e.to_string_lossy().into_owned());
        let folder = file
            .parent()
            .filter(|folder| editor::saves_beside(file) && folder.is_dir())
            .map(Path::to_path_buf)
            .or_else(win32::pdf::documents_folder);
        dialogs::save_dialog(
            owner,
            &tr("chrome-save-as-title"),
            &tr("chrome-save-as-filter"),
            &format!("*.{extension}"),
            &extension,
            &stem,
            folder.as_deref(),
        )
    }

    /// A question with two answers and Cancel (a task dialog owned by `owner`, 0: none), or a
    /// test harness's answer.
    pub(super) fn ask(
        &self,
        owner: isize,
        title: &str,
        question: &str,
        detail: &str,
        yes: &str,
        no: &str,
    ) -> dialogs::SaveAnswer {
        match &self.stand_in {
            Some(stand_in) => stand_in.answer(),
            None => dialogs::ask_save(owner, title, question, detail, yes, no),
        }
    }

    /// The moments of a write starting now: a test harness's, or none.
    fn moments(&self, write: Write) -> Arc<dyn Moments> {
        match &self.stand_in {
            Some(stand_in) => stand_in.moments(write),
            None => Arc::new(NoHooks),
        }
    }

    /// A notice in the window's page.
    pub(super) fn notice(&self, id: WindowId, text: &str) {
        self.run_script(
            id,
            &format!("window.LegibleChrome?.notice({})", json_string(text)),
        );
    }
}
