//! The file work behind a save another program raced, and behind Save a copy: done here, on
//! files alone, so it can be proved on temp folders; `shell` decides when, and tells the page.
//!
//! **A raced save** (`legible_core::save::WriteError::Raced`): the other program's bytes are
//! moved, never over a file, to a name beside the document that says what they are
//! ([`keep_as`]). Legible never removes that copy: Keep mine leaves it, and Take theirs writes
//! the copy's bytes **as they are when taken** back over the document, only while the document
//! is still the save's own ([`take_theirs`]), and leaves the copy too. So whatever either
//! program writes meanwhile, to the document or to the copy, is kept somewhere.
//!
//! **Save a copy:** the destination is approved as it is when the dialog closes
//! ([`approve`]): absent, or a file with a stamp. The copy is written only over exactly that
//! ([`write_copy`]): a file that appears, or changes, after the approval is left alone and the
//! write is a conflict; and it's never the document itself, by any name, checked again just
//! before the write.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use legible_core::save::{self, Hooks, Replace, Stamp, WriteError};
use legible_core::tr;

use crate::win32;

/// How many numbered names [`keep_as`] tries.
const NAME_TRIES: u32 = 20;

/// Moves a raced save's `backup` to a name beside `document` that says what it is ("notes
/// (other version).md", or numbered if that's taken), never over a file that's there. None if
/// it couldn't be: it stays where it is.
pub fn keep_as(document: &Path, backup: &Path) -> Option<PathBuf> {
    let stem = document.file_stem()?.to_string_lossy().into_owned();
    let extension = document
        .extension()
        .map_or(String::new(), |e| format!(".{}", e.to_string_lossy()));
    for n in 1..=NAME_TRIES {
        let name = match n {
            1 => tr!("chrome-raced-name", name = stem.clone()),
            n => tr!("chrome-raced-name-numbered", name = stem.clone(), n = n),
        };
        let to = document.with_file_name(format!("{name}{extension}"));
        match win32::replace::Windows.create(backup, &to) {
            Ok(()) => return Some(to),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                tracing::warn!(%error, "a raced save's backup couldn't be given its name");
                return None;
            }
        }
    }
    None
}

/// The document's stamp now, if it's still `ours` (by its bytes).
pub fn still_ours(document: &Path, ours: Option<Stamp>) -> Option<Stamp> {
    let ours = ours?;
    let (_, now) = save::read(document, &win32::identity).ok()?;
    (now.len == ours.len && now.hash == ours.hash).then_some(now)
}

/// Why Take theirs didn't write.
#[derive(Debug)]
pub enum NotTaken {
    /// The document isn't the save's any more: nothing was written.
    Changed,
    /// Their copy couldn't be read: nothing was written.
    Unreadable(io::Error),
    /// The write didn't happen, or not wholly ([`save::write`]'s reasons).
    Write(WriteError),
}

/// Their version, written back. What the document is when the window takes this up is for
/// [`installed`] to say: another program may have written it since.
#[derive(Debug)]
pub struct Taken {
    /// What was written: the copy's bytes as they were taken.
    pub bytes: Vec<u8>,
    /// Another program raced this write too: its bytes are kept here.
    pub raced: Option<PathBuf>,
}

/// Take theirs: the copy's bytes as they are now, written over `document` only while it's
/// still the save's own (`ours`). The copy stays where it is.
pub fn take_theirs(
    document: &Path,
    copy: &Path,
    ours: Option<Stamp>,
    hooks: &dyn Hooks,
) -> Result<Taken, NotTaken> {
    let current = still_ours(document, ours).ok_or(NotTaken::Changed)?;
    let bytes = std::fs::read(copy).map_err(NotTaken::Unreadable)?;
    // A raced write's stamp is the other writer's, at the backup: never the document's.
    let raced = match save::write(
        document,
        &bytes,
        Some(&current),
        &win32::replace::Windows,
        hooks,
    ) {
        Ok(_) => None,
        Err(WriteError::Raced { backup, .. }) => Some(backup),
        Err(error) => return Err(NotTaken::Write(error)),
    };
    Ok(Taken { bytes, raced })
}

/// The document's stamp, read back now, if it's exactly `bytes`: what a window may take as the
/// file it shows. None if it's anything else (another program wrote it since) or can't be read.
pub fn installed(document: &Path, bytes: &[u8]) -> Option<Stamp> {
    let (now, stamp) = save::read(document, &win32::identity).ok()?;
    (now == bytes).then_some(stamp)
}

/// A copy's destination as the Save dialog approved it: None if there's no file there.
pub fn approve(destination: &Path) -> io::Result<Option<Stamp>> {
    match save::read(destination, &win32::identity) {
        Ok((_, stamp)) => Ok(Some(stamp)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Why a copy wasn't written.
#[derive(Debug)]
pub enum NotCopied {
    /// The destination is the document itself (by another name, say).
    Same,
    Write(WriteError),
}

/// Writes a copy to `destination` over exactly what [`approve`] found there (`approved`), never
/// over `source` (canonical), the document it's a copy of.
pub fn write_copy(
    destination: &Path,
    bytes: &[u8],
    approved: Option<Stamp>,
    source: &Path,
    hooks: &dyn Hooks,
) -> Result<Stamp, NotCopied> {
    if same_file(destination, source) {
        return Err(NotCopied::Same);
    }
    save::write(
        destination,
        bytes,
        approved.as_ref(),
        &win32::replace::Windows,
        hooks,
    )
    .map_err(NotCopied::Write)
}

/// Whether `chosen` is `file` (canonical): the same path, or the same file by another name.
pub fn same_file(chosen: &Path, file: &Path) -> bool {
    if std::fs::canonicalize(chosen).is_ok_and(|c| c == file) {
        return true;
    }
    let identity = |path: &Path| {
        File::open(path)
            .ok()
            .and_then(|f| win32::identity(&f).ok().flatten())
    };
    matches!((identity(chosen), identity(file)), (Some(a), Some(b)) if a == b)
}

/// The name a message gives a file: its own, without its folder.
pub fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// What Keep my changes says of a raced save's copy, for open file `file` (its name): it stays.
pub fn kept_note(file: &str, copy: &Path) -> String {
    tr!(
        "chrome-conflict-raced-kept",
        file = file.to_owned(),
        name = name_of(copy)
    )
}

/// What Use the other version says once it's done: the copy stays.
pub fn taken_note(file: &str, copy: &Path) -> String {
    tr!(
        "chrome-conflict-raced-taken",
        file = file.to_owned(),
        name = name_of(copy)
    )
}

/// Says the document changed again, so nothing was replaced.
pub fn changed_again_note(file: &str, copy: &Path) -> String {
    tr!(
        "chrome-conflict-raced-again",
        file = file.to_owned(),
        name = name_of(copy)
    )
}

/// Says why Use the other version didn't write.
pub fn not_taken_note(file: &str, copy: &Path, not: &NotTaken) -> String {
    let reason = match not {
        NotTaken::Changed => return changed_again_note(file, copy),
        NotTaken::Unreadable(error) => crate::reason::of(error),
        NotTaken::Write(WriteError::Conflict(_)) => {
            tr!("chrome-conflict-changed", file = file.to_owned())
        }
        NotTaken::Write(WriteError::Failed(error) | WriteError::Partial { error, .. }) => {
            crate::reason::of(error)
        }
        NotTaken::Write(WriteError::Raced { .. }) => String::new(),
    };
    tr!(
        "chrome-conflict-raced-take-failed",
        reason = reason,
        name = name_of(copy)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use legible_core::save::{Conflict, NoHooks};

    /// Runs a closure once the document has been checked, just before it's replaced.
    struct Checked<F: Fn()>(F);
    impl<F: Fn()> Hooks for Checked<F> {
        fn checked(&self) {
            (self.0)();
        }
    }

    fn stamp_of(path: &Path) -> Stamp {
        save::read(path, &win32::identity).unwrap().1
    }

    #[test]
    fn a_raced_saves_copy_takes_a_name_beside_the_file_and_never_replaces_one() {
        legible_core::i18n::init("en");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, b"mine").unwrap();
        let first = dir.path().join(".notes.md.legible-1-0.bak");
        std::fs::write(&first, b"theirs").unwrap();
        let kept = keep_as(&file, &first).unwrap();
        assert_eq!(kept.file_name().unwrap(), "notes (other version).md");
        assert_eq!(std::fs::read(&kept).unwrap(), b"theirs");
        assert!(!first.exists());
        let second = dir.path().join(".notes.md.legible-1-1.bak");
        std::fs::write(&second, b"theirs again").unwrap();
        let kept_again = keep_as(&file, &second).unwrap();
        assert_eq!(
            kept_again.file_name().unwrap(),
            "notes (other version 2).md"
        );
        assert_eq!(
            std::fs::read(&kept).unwrap(),
            b"theirs",
            "the first is kept"
        );
    }

    #[test]
    fn take_theirs_writes_the_copy_as_it_is_then_and_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        let copy = dir.path().join("notes (other version).md");
        std::fs::write(&file, b"mine").unwrap();
        std::fs::write(&copy, b"theirs").unwrap();
        let ours = stamp_of(&file);
        // Changed before Take: the newest of the copy goes in.
        std::fs::write(&copy, b"theirs, edited").unwrap();
        let taken = take_theirs(&file, &copy, Some(ours), &NoHooks).unwrap();
        assert_eq!(taken.bytes, b"theirs, edited");
        assert_eq!(std::fs::read(&file).unwrap(), b"theirs, edited");
        assert_eq!(installed(&file, &taken.bytes), Some(stamp_of(&file)));
        assert_eq!(
            std::fs::read(&copy).unwrap(),
            b"theirs, edited",
            "the copy stays"
        );
    }

    #[test]
    fn take_theirs_while_either_file_changes_keeps_every_version() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        let copy = dir.path().join("notes (other version).md");
        std::fs::write(&file, b"mine").unwrap();
        std::fs::write(&copy, b"theirs").unwrap();
        let ours = stamp_of(&file);
        // The copy edited while Take is written: the file has what was taken, the copy the
        // newer edit.
        let edit = || std::fs::write(&copy, b"theirs, newer").unwrap();
        take_theirs(&file, &copy, Some(ours), &Checked(edit)).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"theirs");
        assert_eq!(std::fs::read(&copy).unwrap(), b"theirs, newer");

        // The document changed since the save: nothing is written, and nothing removed.
        std::fs::write(&file, b"mine").unwrap();
        let ours = stamp_of(&file);
        std::fs::write(&file, b"someone else's").unwrap();
        let not = take_theirs(&file, &copy, Some(ours), &NoHooks).unwrap_err();
        assert!(matches!(not, NotTaken::Changed), "{not:?}");
        assert_eq!(std::fs::read(&file).unwrap(), b"someone else's");
        assert_eq!(std::fs::read(&copy).unwrap(), b"theirs, newer");
        // Not the save's even when it was written: never.
        let not = take_theirs(&file, &copy, None, &NoHooks).unwrap_err();
        assert!(matches!(not, NotTaken::Changed), "{not:?}");
    }

    #[test]
    fn take_theirs_raced_in_turn_keeps_all_three_and_gives_the_documents_own_stamp() {
        legible_core::i18n::init("en");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        let copy = dir.path().join("notes (other version).md");
        std::fs::write(&file, b"mine").unwrap();
        std::fs::write(&copy, b"theirs").unwrap();
        let ours = stamp_of(&file);
        // A third program writes the document between Take's check and its replace.
        let third = || std::fs::write(&file, b"third writer").unwrap();
        let taken = take_theirs(&file, &copy, Some(ours), &Checked(third)).unwrap();
        let backup = taken.raced.clone().expect("raced");
        assert_eq!(std::fs::read(&file).unwrap(), b"theirs");
        assert_eq!(std::fs::read(&copy).unwrap(), b"theirs");
        assert_eq!(std::fs::read(&backup).unwrap(), b"third writer");
        // The stamp the window takes is the document's, read back: a save with it goes.
        let stamp = installed(&file, &taken.bytes).expect("the document is theirs");
        assert_eq!(stamp, stamp_of(&file));
        save::write(
            &file,
            b"next",
            Some(&stamp),
            &win32::replace::Windows,
            &NoHooks,
        )
        .unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"next");
    }

    #[test]
    fn a_take_written_over_before_the_window_takes_it_up_isnt_installed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        let copy = dir.path().join("notes (other version).md");
        std::fs::write(&file, b"mine").unwrap();
        std::fs::write(&copy, b"theirs").unwrap();
        let taken = take_theirs(&file, &copy, Some(stamp_of(&file)), &NoHooks).unwrap();
        // Between the take and the window's read-back, another program writes it.
        std::fs::write(&file, b"later").unwrap();
        assert_eq!(installed(&file, &taken.bytes), None);
        assert_eq!(std::fs::read(&copy).unwrap(), b"theirs", "the copy stays");
    }

    #[test]
    fn a_copy_is_written_only_over_what_the_dialog_approved() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("notes.md");
        std::fs::write(&source, b"source").unwrap();
        let source = std::fs::canonicalize(&source).unwrap();

        // Approved absent; a file appears before the write: left alone.
        let new = dir.path().join("copy.md");
        let approved = approve(&new).unwrap();
        assert_eq!(approved, None);
        std::fs::write(&new, b"appeared").unwrap();
        let not = write_copy(&new, b"copied", approved, &source, &NoHooks).unwrap_err();
        assert!(
            matches!(
                not,
                NotCopied::Write(WriteError::Conflict(Conflict::Appeared))
            ),
            "{not:?}"
        );
        assert_eq!(std::fs::read(&new).unwrap(), b"appeared");

        // Approved as it was; changed before the write: left alone.
        let approved = approve(&new).unwrap();
        std::fs::write(&new, b"changed since").unwrap();
        let not = write_copy(&new, b"copied", approved, &source, &NoHooks).unwrap_err();
        assert!(
            matches!(
                not,
                NotCopied::Write(WriteError::Conflict(Conflict::Changed))
            ),
            "{not:?}"
        );
        assert_eq!(std::fs::read(&new).unwrap(), b"changed since");

        // Approved as it is, unchanged: replaced. Absent, still absent: made.
        let approved = approve(&new).unwrap();
        write_copy(&new, b"copied", approved, &source, &NoHooks).unwrap();
        assert_eq!(std::fs::read(&new).unwrap(), b"copied");
        let fresh = dir.path().join("fresh.md");
        write_copy(
            &fresh,
            b"copied",
            approve(&fresh).unwrap(),
            &source,
            &NoHooks,
        )
        .unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), b"copied");

        // Approved absent, but by the write it's the source by another name: refused.
        let link = dir.path().join("link.md");
        let approved = approve(&link).unwrap();
        std::fs::hard_link(&source, &link).unwrap();
        let not = write_copy(&link, b"copied", approved, &source, &NoHooks).unwrap_err();
        assert!(matches!(not, NotCopied::Same), "{not:?}");
        assert_eq!(std::fs::read(&source).unwrap(), b"source");
    }

    #[test]
    fn a_copy_is_never_the_file_itself_by_any_name() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, b"text").unwrap();
        let file = std::fs::canonicalize(&file).unwrap();
        assert!(same_file(&dir.path().join("notes.md"), &file));
        assert!(same_file(&dir.path().join("NOTES.MD"), &file));
        let link = dir.path().join("link.md");
        std::fs::hard_link(&file, &link).unwrap();
        assert!(same_file(&link, &file));
        let copy = dir.path().join("copy.md");
        std::fs::copy(&file, &copy).unwrap();
        assert!(!same_file(&copy, &file));
        assert!(!same_file(&dir.path().join("new.md"), &file));
    }

    #[test]
    fn a_raced_saves_answer_acts_only_while_the_file_is_still_the_saves() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, b"mine").unwrap();
        let ours = stamp_of(&file);
        assert!(still_ours(&file, Some(ours)).is_some());
        assert!(still_ours(&file, None).is_none());
        std::fs::write(&file, b"again").unwrap();
        assert!(still_ours(&file, Some(ours)).is_none());
        std::fs::remove_file(&file).unwrap();
        assert!(still_ours(&file, Some(ours)).is_none());
    }
}
