//! What live reload does with a change on disk, given unsaved edits: the hook safe saving uses.
//!
//! With no unsaved edits and nothing held, a change or a deletion is applied at once. While the
//! window has unsaved edits (the page said `dirty`), both are **held**: the edits stay on
//! screen, the newest change is kept as pending with a rising id, and the page is told
//! (`legible:external-change`, with its kind and id) so the conflict prompt can ask. Clearing
//! dirty doesn't apply what's pending, and later changes stay pending too, until the prompt
//! answers for the current id: [`State::reload`] ("take theirs", which discards the edits) or
//! [`State::keep`] ("keep mine"). An answer for an older id is ignored: a newer change has been
//! announced since.

/// What happened on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Changed,
    Deleted,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Changed => "changed",
            Kind::Deleted => "deleted",
        }
    }
}

/// What to do with a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Show it now.
    Apply(Kind),
    /// Hold it as pending `id` and tell the page.
    Hold { kind: Kind, id: u64 },
}

/// One window's live-reload state.
#[derive(Debug, Default)]
pub struct State {
    dirty: bool,
    pending: Option<(Kind, u64)>,
    last_id: u64,
}

impl State {
    /// A change or deletion arrived from disk.
    pub fn on_disk(&mut self, kind: Kind) -> Action {
        if !self.dirty && self.pending.is_none() {
            return Action::Apply(kind);
        }
        self.last_id += 1;
        self.pending = Some((kind, self.last_id));
        Action::Hold {
            kind,
            id: self.last_id,
        }
    }

    /// The page has, or no longer has, unsaved edits. Never applies what's pending.
    pub fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn pending(&self) -> Option<(Kind, u64)> {
        self.pending
    }

    /// "Take theirs" for pending `id`: what to show from disk, if `id` is the current one. The
    /// edits are discarded, so the window is no longer dirty.
    pub fn reload(&mut self, id: u64) -> Option<Kind> {
        match self.pending {
            Some((kind, current)) if current == id => {
                self.pending = None;
                self.dirty = false;
                Some(kind)
            }
            _ => None,
        }
    }

    /// "Keep mine" for pending `id`: drops it, if `id` is the current one.
    pub fn keep(&mut self, id: u64) -> bool {
        match self.pending {
            Some((_, current)) if current == id => {
                self.pending = None;
                true
            }
            _ => false,
        }
    }
}

/// The moved-or-deleted bar, as the window keeps it: so a page served again (when what may
/// load from the internet changes its CSP) shows the bar as the page before did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FileBar {
    gone: bool,
    /// "Keep showing it" was chosen while the file was gone.
    kept: bool,
}

impl FileBar {
    /// The file was moved or deleted.
    pub fn deleted(&mut self) {
        *self = FileBar {
            gone: true,
            kept: false,
        };
    }

    /// The file is back (live reload shows it again).
    pub fn back(&mut self) {
        *self = FileBar::default();
    }

    /// "Keep showing it": the bar goes, the file is still gone.
    pub fn keep_showing(&mut self) {
        self.kept = self.gone;
    }

    /// Whether a page should show the bar.
    pub fn shows(&self) -> bool {
        self.gone && !self.kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_bar_outlives_the_page_it_was_shown_on() {
        let mut bar = FileBar::default();
        assert!(!bar.shows());
        bar.keep_showing();
        assert!(!bar.shows(), "Keep showing it with nothing gone");
        // Deleted: every page served from now on shows it...
        bar.deleted();
        assert!(bar.shows());
        assert!(bar.shows(), "a page served again still shows it");
        // ...until the reader keeps showing the document, which sticks across pages too...
        bar.keep_showing();
        assert!(!bar.shows());
        // ...or the file comes back.
        bar.back();
        assert!(!bar.shows());
        // Gone again after it came back: shown again, whatever was chosen before.
        bar.deleted();
        assert!(bar.shows());
        bar.back();
        bar.keep_showing();
        bar.deleted();
        assert!(bar.shows());
    }

    #[test]
    fn with_no_edits_changes_apply_at_once() {
        let mut s = State::default();
        assert_eq!(s.on_disk(Kind::Changed), Action::Apply(Kind::Changed));
        assert_eq!(s.on_disk(Kind::Deleted), Action::Apply(Kind::Deleted));
        assert_eq!(s.pending(), None);
    }

    #[test]
    fn while_dirty_changes_and_deletions_are_held_newest_first() {
        let mut s = State::default();
        s.set_dirty(true);
        assert_eq!(
            s.on_disk(Kind::Deleted),
            Action::Hold {
                kind: Kind::Deleted,
                id: 1
            }
        );
        // Recreated while still dirty: the newest wins, with a new id.
        assert_eq!(
            s.on_disk(Kind::Changed),
            Action::Hold {
                kind: Kind::Changed,
                id: 2
            }
        );
        assert_eq!(s.pending(), Some((Kind::Changed, 2)));
    }

    #[test]
    fn clearing_dirty_keeps_what_is_pending_and_holds_later_changes() {
        let mut s = State::default();
        s.set_dirty(true);
        s.on_disk(Kind::Changed);
        s.set_dirty(false);
        assert_eq!(s.pending(), Some((Kind::Changed, 1)));
        assert_eq!(
            s.on_disk(Kind::Changed),
            Action::Hold {
                kind: Kind::Changed,
                id: 2
            }
        );
    }

    #[test]
    fn only_the_current_id_is_answered() {
        let mut s = State::default();
        s.set_dirty(true);
        s.on_disk(Kind::Changed);
        s.on_disk(Kind::Deleted);
        assert_eq!(s.reload(1), None);
        assert!(!s.keep(1));
        assert_eq!(s.reload(2), Some(Kind::Deleted));
        assert!(!s.is_dirty());
        assert_eq!(s.pending(), None);
        // Then changes apply again.
        assert_eq!(s.on_disk(Kind::Changed), Action::Apply(Kind::Changed));
    }

    #[test]
    fn keeping_mine_drops_the_change_and_stays_dirty() {
        let mut s = State::default();
        s.set_dirty(true);
        s.on_disk(Kind::Changed);
        assert!(s.keep(1));
        assert!(s.is_dirty());
        assert_eq!(s.pending(), None);
        assert_eq!(
            s.on_disk(Kind::Changed),
            Action::Hold {
                kind: Kind::Changed,
                id: 2
            }
        );
    }
}
