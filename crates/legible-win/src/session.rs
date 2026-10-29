//! Windows ending the session (signing out, restarting or shutting down, or an installer's
//! Restart Manager) while a window has unsaved edits, as decisions: what each window answers,
//! and the round of close questions that follows. `shell` carries them out, `win32::session`
//! hears Windows, and `legible_core::unsaved` keeps the text; nothing here touches a window or a
//! file.
//!
//! **Asked whether the session may end** (`WM_QUERYENDSESSION`, sent before every end, a forced
//! one included), a window with anything unsaved keeps a copy of its text first, whatever comes
//! next, and then stops the end, saying why: Windows shows its screen of apps that are stopping
//! it. A critical end can't be stopped: the copy is all there is. A window with nothing unsaved
//! lets the session end at once.
//!
//! **The round:** once a window has stopped the end, the unsaved windows ask their close
//! question in turn, oldest first, each once the one before it has closed. Save saves and
//! closes; Don't save closes; with none left Legible exits, and Windows carries on. Cancel ends
//! the round: the reader is going back to their work. Asked again, the round starts again.
//!
//! **At the next start** ([`offer`]), each copy kept is offered back: its file's window opens
//! with a bar, Restore (the text goes into the editor as unsaved changes) or Discard; closed
//! without a choice, it's offered again next time. A file that has gone, or can't be edited
//! where it is, offers Save a copy instead. A file that already holds the text needs nothing.

use legible_core::unsaved::Kept;

use crate::disk::Disk;
use crate::saving::Session;

/// Whether a window has anything unsaved: edits (`dirty`, by either of the window's accounts of
/// them), or a save under way or waiting, or a close's own save.
pub fn unsaved(dirty: bool, saves: &Session) -> bool {
    dirty || saves.busy() || saves.frozen()
}

/// A window's answer to Windows' question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Nothing unsaved: the session may end.
    Allow,
    /// Keep a copy of the text, then stop the end and say why.
    KeepAndStop,
    /// A critical end, which can't be stopped: keep a copy of the text, and let it go.
    KeepAndAllow,
}

pub fn answer(unsaved: bool, critical: bool) -> Answer {
    match (unsaved, critical) {
        (false, _) => Answer::Allow,
        (true, false) => Answer::KeepAndStop,
        (true, true) => Answer::KeepAndAllow,
    }
}

/// The round of close questions after a window stopped the session's end.
#[derive(Debug, Default)]
pub struct Round {
    on: bool,
}

impl Round {
    /// A window stopped the end: whether the round starts now (it may be under way already).
    pub fn start(&mut self) -> bool {
        !std::mem::replace(&mut self.on, true)
    }

    /// The window to ask next, given the unsaved windows oldest first: the first, while the
    /// round is on. With none left, the round is over.
    pub fn next<W: Copy>(&mut self, unsaved: &[W]) -> Option<W> {
        if !self.on {
            return None;
        }
        let next = unsaved.first().copied();
        self.on = next.is_some();
        next
    }

    /// The reader cancelled a question, or the session goes on after all: the round is over.
    pub fn stop(&mut self) {
        self.on = false;
    }

    pub fn on(&self) -> bool {
        self.on
    }
}

/// What a copy kept from an earlier session comes to at the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// The file holds that text already: there's nothing to restore, and the copy goes.
    Nothing,
    /// The file's window opens with the bar; `changed`: the file has changed since the copy's
    /// edits were made to it.
    Bar { changed: bool },
    /// The file has gone, or can't be edited where it is: Save a copy, or Discard.
    Elsewhere,
}

/// What `kept` comes to, its file being `file` as it is now (None: it has gone), which can be
/// edited where it is or not.
pub fn offer(kept: &Kept, file: Option<&Disk>, editable: bool) -> Offer {
    let Some(disk) = file.filter(|d| editable && d.content.format().is_some()) else {
        return Offer::Elsewhere;
    };
    if disk.content.text() == kept.text {
        return Offer::Nothing;
    }
    Offer::Bar {
        changed: kept.base.is_some_and(|base| base != disk.stamp.hash),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saving::{Choice, Close, Outcome, Save};
    use legible_core::save::{self as core_save, Stamp};
    use std::path::PathBuf;

    fn disk(bytes: &[u8]) -> Disk {
        Disk::of(
            bytes.to_vec(),
            Stamp {
                len: bytes.len() as u64,
                hash: core_save::hash(bytes),
                written: None,
                identity: None,
            },
        )
    }

    fn kept(base: &[u8], text: &str) -> Kept {
        Kept {
            copy: PathBuf::from(r"C:\Legible\Recovery\notes-0.recovered"),
            document: PathBuf::from(r"C:\Docs\notes.md"),
            base: Some(core_save::hash(base)),
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_copy_is_offered_on_its_files_bar_saying_whether_the_file_changed_since() {
        let copy = kept(b"# Notes\n", "Mine. # Notes\n");
        assert_eq!(
            offer(&copy, Some(&disk(b"# Notes\n")), true),
            Offer::Bar { changed: false }
        );
        assert_eq!(
            offer(&copy, Some(&disk(b"# Notes, theirs\n")), true),
            Offer::Bar { changed: true }
        );
        // A file in its own format compares by its text: CRLF on disk is the same text.
        assert_eq!(
            offer(&copy, Some(&disk(b"Mine. # Notes\r\n")), true),
            Offer::Nothing
        );
        assert_eq!(
            offer(&copy, Some(&disk(b"Mine. # Notes\n")), true),
            Offer::Nothing
        );
    }

    #[test]
    fn a_file_gone_or_not_editable_where_it_is_offers_a_copy_elsewhere() {
        let copy = kept(b"# Notes\n", "Mine. # Notes\n");
        assert_eq!(offer(&copy, None, true), Offer::Elsewhere);
        assert_eq!(
            offer(&copy, Some(&disk(b"# Notes\n")), false),
            Offer::Elsewhere
        );
        // Not text Legible can save as it was (Windows-1252, say).
        assert_eq!(
            offer(&copy, Some(&disk(b"caf\xe9\n")), true),
            Offer::Elsewhere
        );
    }

    fn save(version: u64) -> Save {
        Save {
            version,
            text: format!("v{version}"),
        }
    }

    #[test]
    fn unsaved_is_edits_a_save_under_way_or_a_closes_own_save() {
        let mut s = Session::default();
        assert!(!unsaved(false, &s), "nothing at all");
        assert!(unsaved(true, &s), "edits");
        // A save under way: the page may already say clean, but nothing is on disk yet.
        s.save(save(1));
        assert!(unsaved(false, &s));
        s.done(1, Outcome::Saved);
        assert!(!unsaved(false, &s));
        // A close's save, frozen: unsaved until the page says it's clean.
        assert_eq!(s.close(true), Close::Ask);
        assert_eq!(s.choose(Choice::Save), Close::FreezeAndSave);
        assert!(unsaved(false, &s));
        s.save(save(2));
        assert_eq!(s.done(2, Outcome::Saved).close, Close::CheckClean);
        assert!(unsaved(false, &s), "still frozen, its answer awaited");
        assert_eq!(s.clean(true), Close::Now);
        assert!(!unsaved(false, &s));
    }

    #[test]
    fn only_a_window_with_something_unsaved_answers_otherwise_and_it_always_keeps_a_copy() {
        assert_eq!(answer(false, false), Answer::Allow);
        assert_eq!(answer(false, true), Answer::Allow);
        assert_eq!(answer(true, false), Answer::KeepAndStop);
        assert_eq!(answer(true, true), Answer::KeepAndAllow);
    }

    /// Windows `a` (edited), `b` (a save under way, its page clean) and `c` (nothing unsaved),
    /// oldest first, as the shell would list them.
    struct Windows {
        sessions: [Session; 3],
        dirty: [bool; 3],
        open: [bool; 3],
    }

    impl Windows {
        fn new() -> Windows {
            let mut b = Session::default();
            b.save(save(1));
            Windows {
                sessions: [Session::default(), b, Session::default()],
                dirty: [true, false, false],
                open: [true; 3],
            }
        }

        fn unsaved(&self) -> Vec<usize> {
            (0..3)
                .filter(|&i| self.open[i] && unsaved(self.dirty[i], &self.sessions[i]))
                .collect()
        }

        fn answers(&self, critical: bool) -> Vec<Answer> {
            (0..3)
                .map(|i| answer(unsaved(self.dirty[i], &self.sessions[i]), critical))
                .collect()
        }
    }

    #[test]
    fn a_stopped_end_asks_each_unsaved_window_in_turn_until_none_is_left() {
        let mut w = Windows::new();
        assert_eq!(
            w.answers(false),
            [Answer::KeepAndStop, Answer::KeepAndStop, Answer::Allow]
        );
        let mut round = Round::default();
        // Each window that stopped it starts the round; only the first one does.
        assert!(round.start());
        assert!(!round.start());
        // `a` asks first: Save saves, and it closes once the frozen page says it's clean.
        assert_eq!(round.next(&w.unsaved()), Some(0));
        let a = &mut w.sessions[0];
        assert_eq!(a.close(true), Close::Ask);
        assert_eq!(a.choose(Choice::Save), Close::FreezeAndSave);
        a.save(save(1));
        assert_eq!(a.done(1, Outcome::Saved).close, Close::CheckClean);
        w.dirty[0] = false;
        assert_eq!(w.sessions[0].clean(true), Close::Now);
        w.open[0] = false;
        // Then `b`: its close waits for the save under way, and needs no question.
        assert_eq!(round.next(&w.unsaved()), Some(1));
        let b = &mut w.sessions[1];
        assert_eq!(b.close(false), Close::Wait);
        assert_eq!(b.done(1, Outcome::Saved).close, Close::CheckClean);
        assert_eq!(b.clean(true), Close::Now);
        w.open[1] = false;
        // Nothing unsaved is left (`c` never had anything): the round is over, and the session
        // may end.
        assert_eq!(round.next(&w.unsaved()), None);
        assert!(!round.on());
        assert_eq!(w.answers(false)[2], Answer::Allow);
    }

    #[test]
    fn cancel_ends_the_round_and_a_new_question_starts_it_again() {
        let mut w = Windows::new();
        let mut round = Round::default();
        round.start();
        assert_eq!(round.next(&w.unsaved()), Some(0));
        let a = &mut w.sessions[0];
        assert_eq!(a.close(true), Close::Ask);
        assert_eq!(a.choose(Choice::Cancel), Close::Stay);
        round.stop();
        // Nothing more is asked, though `a` and `b` are unsaved still.
        assert_eq!(round.next(&w.unsaved()), None);
        assert_eq!(w.unsaved(), [0, 1]);
        // Windows asks again: it's stopped again, and the round starts over.
        assert_eq!(w.answers(false)[0], Answer::KeepAndStop);
        assert!(round.start());
        assert_eq!(round.next(&w.unsaved()), Some(0));
    }

    #[test]
    fn a_save_that_fails_in_the_round_leaves_the_window_open_and_the_round_waiting_for_it() {
        let mut w = Windows::new();
        let mut round = Round::default();
        round.start();
        assert_eq!(round.next(&w.unsaved()), Some(0));
        let a = &mut w.sessions[0];
        a.close(true);
        a.choose(Choice::Save);
        a.save(save(1));
        // A conflict: the page takes edits again, and the window stays, still unsaved.
        assert_eq!(a.done(1, Outcome::Conflict).close, Close::Unfreeze);
        assert!(unsaved(w.dirty[0], &w.sessions[0]));
        // The round goes on only from a window's close, so nothing else is asked meanwhile;
        // closed later, the next is asked.
        assert!(round.on());
        w.open[0] = false;
        assert_eq!(round.next(&w.unsaved()), Some(1));
    }

    #[test]
    fn a_critical_end_keeps_every_unsaved_text_and_stops_nothing() {
        let w = Windows::new();
        assert_eq!(
            w.answers(true),
            [Answer::KeepAndAllow, Answer::KeepAndAllow, Answer::Allow]
        );
    }
}
