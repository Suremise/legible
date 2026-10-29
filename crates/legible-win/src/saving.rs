//! A window's saving and closing, as decisions: what to write and when, and whether the window
//! may close. `shell` carries them out; nothing here touches a file or a window.
//!
//! **Saving:** one write at a time; a save asked for while one is written waits, the newest
//! winning, and goes once that one is done well. One that conflicts or fails takes the waiting
//! one with it: the edits stay unsaved until the reader saves again.
//!
//! **Closing** (every way a window closes asks [`Session::close`]): a window with nothing
//! unsaved and nothing being written closes. One with a save under way waits for it, then
//! decides again, the page frozen as it says whether it's clean. One with unsaved edits asks: **Save** stops the editor taking changes (the
//! page is frozen) and saves what it has; the window closes only if, once that's saved, the
//! frozen page says it's clean. **Don't save** closes: the only way edits are dropped.
//! **Cancel** does nothing. A save that conflicts or fails keeps the window open, the page
//! taking changes again.
//!
//! **Keep mine, again:** a conflict found by a save is remembered by its change's id. "Keep
//! mine" for that very id saves again; for any other (an older one, or a newer change since),
//! it doesn't: a newer change asks anew.

/// What a save is: the editor's text at `version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Save {
    pub version: u64,
    pub text: String,
}

/// How a write went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Saved,
    /// The file changed on disk (or went): nothing was written.
    Conflict,
    /// Anything else: nothing was written, or what was is reported on its own.
    Failed,
}

/// What a close comes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Close {
    /// Close the window now.
    Now,
    /// Ask: Save, Don't save, Cancel.
    Ask,
    /// A save is under way: decide again once it's done.
    Wait,
    /// Freeze the page, and save what it has (the close's own save).
    FreezeAndSave,
    /// Ask the frozen page whether it's clean, then [`Session::clean`].
    CheckClean,
    /// Let the page take changes again; the window stays open.
    Unfreeze,
    /// Nothing.
    Stay,
}

/// The reader's answer to the close question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Save,
    DontSave,
    Cancel,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Closing {
    #[default]
    No,
    /// Asked while a save was under way: decide once it's done.
    AfterSave,
    /// "Save" chosen: the page is frozen, its save under way.
    Saving,
    /// A save done: waiting for the frozen page's answer (it freezes itself as it answers),
    /// after "Save" was `chosen`, or after a save that was under way when the window was closed.
    Checking { chosen: bool },
}

/// What a finished write leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    /// The save that waited, to write now.
    pub next: Option<Save>,
    /// What the close (if one is under way) comes to.
    pub close: Close,
}

/// One window's saving and closing.
#[derive(Debug, Default)]
pub struct Session {
    in_flight: Option<u64>,
    waiting: Option<Save>,
    closing: Closing,
    /// The change a save's conflict was held as: "Keep mine" for it saves again.
    retry: Option<u64>,
}

impl Session {
    /// Whether a write is under way or waiting.
    pub fn busy(&self) -> bool {
        self.in_flight.is_some() || self.waiting.is_some()
    }

    /// Whether the page is frozen for the close's save.
    pub fn frozen(&self) -> bool {
        matches!(self.closing, Closing::Saving | Closing::Checking { .. })
    }

    /// Whether a close is under way: waiting for a save, saving, or checking the page.
    pub fn closing(&self) -> bool {
        self.closing != Closing::No
    }

    /// Whether the page's answer to "is it clean?" is awaited.
    pub fn checking(&self) -> bool {
        matches!(self.closing, Closing::Checking { .. })
    }

    /// A save is asked for: write it now (returned), or once the one under way is done.
    pub fn save(&mut self, save: Save) -> Option<Save> {
        if self.in_flight.is_some() {
            self.waiting = Some(save);
            return None;
        }
        self.in_flight = Some(save.version);
        Some(save)
    }

    /// The write of `version` is done, as `outcome`.
    pub fn done(&mut self, version: u64, outcome: Outcome) -> Done {
        if self.in_flight != Some(version) {
            return Done {
                next: None,
                close: Close::Stay,
            };
        }
        self.in_flight = None;
        if outcome != Outcome::Saved {
            // The edits stay unsaved: no later save goes ahead of the reader's next one, and
            // the window stays open.
            self.waiting = None;
            let close = match self.closing {
                Closing::Saving | Closing::Checking { chosen: true } => Close::Unfreeze,
                Closing::AfterSave | Closing::Checking { chosen: false } => Close::Ask,
                Closing::No => Close::Stay,
            };
            self.closing = Closing::No;
            return Done { next: None, close };
        }
        if let Some(next) = self.waiting.take() {
            self.in_flight = Some(next.version);
            return Done {
                next: Some(next),
                close: Close::Stay,
            };
        }
        let close = match self.closing {
            Closing::Saving => {
                self.closing = Closing::Checking { chosen: true };
                Close::CheckClean
            }
            // Asked while a save was under way, and it's done: is anything left unsaved?
            Closing::AfterSave => {
                self.closing = Closing::Checking { chosen: false };
                Close::CheckClean
            }
            Closing::Checking { .. } | Closing::No => Close::Stay,
        };
        Done { next: None, close }
    }

    /// The window is to close. `dirty`: the editor has unsaved edits (by either of the
    /// window's two accounts of it).
    pub fn close(&mut self, dirty: bool) -> Close {
        if self.frozen() || self.checking() {
            return Close::Stay;
        }
        if self.busy() {
            self.closing = Closing::AfterSave;
            return Close::Wait;
        }
        if dirty { Close::Ask } else { Close::Now }
    }

    /// The reader's answer to the close question.
    pub fn choose(&mut self, choice: Choice) -> Close {
        match choice {
            Choice::Save => {
                self.closing = Closing::Saving;
                Close::FreezeAndSave
            }
            Choice::DontSave => {
                self.closing = Closing::No;
                Close::Now
            }
            Choice::Cancel => {
                self.closing = Closing::No;
                Close::Stay
            }
        }
    }

    /// The close's save couldn't start (no text came, say): the page takes changes again.
    pub fn close_save_failed(&mut self) -> Close {
        if self.frozen() {
            self.closing = Closing::No;
            Close::Unfreeze
        } else {
            Close::Stay
        }
    }

    /// The page's answer, frozen or after a save under way: `clean`, and both of the window's
    /// accounts of unsaved edits agreeing. An answer nobody asked for does nothing.
    pub fn clean(&mut self, clean: bool) -> Close {
        let Closing::Checking { chosen } = self.closing else {
            return Close::Stay;
        };
        self.closing = Closing::No;
        match (clean && !self.busy(), chosen) {
            (true, _) => Close::Now,
            // Something isn't saved after all: the page takes changes again, and is asked.
            (false, true) => Close::Unfreeze,
            (false, false) => Close::Ask,
        }
    }

    /// A save's conflict was held as change `id`.
    pub fn conflict(&mut self, id: u64) {
        self.retry = Some(id);
    }

    /// "Keep mine" for change `id`: whether to save again (only for the conflict a save found,
    /// and only that very change).
    pub fn kept(&mut self, id: u64) -> bool {
        if self.retry == Some(id) {
            self.retry = None;
            true
        } else {
            false
        }
    }

    /// A newer change, or "take theirs": no save waits to be done again.
    pub fn forget_conflict(&mut self) {
        self.retry = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save(version: u64) -> Save {
        Save {
            version,
            text: format!("v{version}"),
        }
    }

    #[test]
    fn one_write_at_a_time_and_the_newest_waits() {
        let mut s = Session::default();
        assert_eq!(s.save(save(1)), Some(save(1)));
        assert_eq!(s.save(save(2)), None);
        assert_eq!(s.save(save(3)), None);
        assert!(s.busy());
        // An answer for a write that isn't under way changes nothing.
        assert_eq!(s.done(2, Outcome::Saved).next, None);
        assert_eq!(s.done(1, Outcome::Saved).next, Some(save(3)));
        assert_eq!(s.done(3, Outcome::Saved).next, None);
        assert!(!s.busy());
    }

    #[test]
    fn a_write_that_conflicts_or_fails_takes_the_waiting_one_with_it() {
        for outcome in [Outcome::Conflict, Outcome::Failed] {
            let mut s = Session::default();
            s.save(save(1));
            s.save(save(2));
            let done = s.done(1, outcome);
            assert_eq!((done.next, done.close), (None, Close::Stay), "{outcome:?}");
            assert!(!s.busy());
        }
    }

    #[test]
    fn closing_clean_closes_dirty_asks_and_dont_save_is_the_only_discard() {
        let mut s = Session::default();
        assert_eq!(s.close(false), Close::Now);
        assert_eq!(s.close(true), Close::Ask);
        assert_eq!(s.choose(Choice::Cancel), Close::Stay);
        assert_eq!(s.close(true), Close::Ask);
        assert_eq!(s.choose(Choice::DontSave), Close::Now);
    }

    #[test]
    fn save_and_close_closes_only_once_the_frozen_page_is_saved_and_clean() {
        let mut s = Session::default();
        assert_eq!(s.close(true), Close::Ask);
        assert_eq!(s.choose(Choice::Save), Close::FreezeAndSave);
        assert!(s.frozen());
        // A second close while frozen does nothing.
        assert_eq!(s.close(true), Close::Stay);
        assert_eq!(s.save(save(5)), Some(save(5)));
        assert_eq!(s.done(5, Outcome::Saved).close, Close::CheckClean);
        assert!(s.frozen(), "still frozen while the page is asked");
        assert_eq!(s.clean(true), Close::Now);
        // An answer nobody asked for closes nothing.
        assert_eq!(s.clean(true), Close::Stay);

        // Saved, but the page isn't clean after all: it takes changes again, and stays open.
        let mut s = Session::default();
        s.close(true);
        s.choose(Choice::Save);
        s.save(save(5));
        s.done(5, Outcome::Saved);
        assert_eq!(s.clean(false), Close::Unfreeze);
        assert!(!s.frozen());
    }

    #[test]
    fn a_close_save_that_conflicts_or_fails_keeps_the_window_open() {
        for outcome in [Outcome::Conflict, Outcome::Failed] {
            let mut s = Session::default();
            s.close(true);
            s.choose(Choice::Save);
            s.save(save(5));
            assert_eq!(s.done(5, outcome).close, Close::Unfreeze, "{outcome:?}");
            assert!(!s.frozen());
            // And the next close asks again.
            assert_eq!(s.close(true), Close::Ask);
        }
        let mut s = Session::default();
        s.close(true);
        s.choose(Choice::Save);
        assert_eq!(s.close_save_failed(), Close::Unfreeze);
    }

    #[test]
    fn a_close_during_a_save_waits_for_it_and_everything_after_it() {
        let mut s = Session::default();
        s.save(save(1));
        s.save(save(2));
        assert_eq!(s.close(false), Close::Wait);
        // The first done, the waiting one goes: still waiting to decide.
        let done = s.done(1, Outcome::Saved);
        assert_eq!((done.next, done.close), (Some(save(2)), Close::Stay));
        // All done: the page is asked, frozen as it answers (so nothing typed after its answer
        // is lost), and closes only if clean; a second close meanwhile does nothing.
        assert_eq!(s.done(2, Outcome::Saved).close, Close::CheckClean);
        assert!(s.frozen() && s.checking());
        assert_eq!(s.close(true), Close::Stay);
        assert_eq!(s.clean(true), Close::Now);

        // Newer edits after that save: asked, not closed.
        let mut s = Session::default();
        s.save(save(1));
        s.close(true);
        assert_eq!(s.done(1, Outcome::Saved).close, Close::CheckClean);
        assert_eq!(s.clean(false), Close::Ask);
        assert!(!s.frozen(), "asked with the page taking edits again");

        // The save under way fails: asked.
        let mut s = Session::default();
        s.save(save(1));
        s.close(true);
        assert_eq!(s.done(1, Outcome::Failed).close, Close::Ask);
    }

    #[test]
    fn keep_mine_saves_again_only_for_the_conflict_a_save_found() {
        let mut s = Session::default();
        s.conflict(4);
        assert!(!s.kept(3), "an older change");
        // A newer change since: that one's conflict is remembered instead.
        s.conflict(5);
        assert!(!s.kept(4));
        assert!(s.kept(5));
        assert!(!s.kept(5), "once");
        s.conflict(6);
        s.forget_conflict();
        assert!(!s.kept(6));
    }
}
