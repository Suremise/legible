//! Unsaved edits kept through Windows ending the session: a copy of an unsaved document's text
//! in Legible's own folder ([`FOLDER`]), never beside or over the document, written as Windows
//! asks whether the session may end and offered back at the next start. A copy is removed only
//! once its text is saved, the reader has chosen to drop it, or the session goes on after all.
//!
//! One copy per document and run of Legible ([`session`]), named from both, so a newer copy of
//! the same document in the same run replaces the older, and a copy from an earlier run, still
//! waiting for the reader's choice, is never written over: `<stem>-<hash>-<run>.recovered`,
//! holding a header line, the document's path, the hash of
//! the file the edits were made to (`-` if there was none), a blank line and the text, in UTF-8
//! with the editor's `\n` line endings. It's written in full beside itself and then renamed into
//! place, so a copy is never half-written.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::save;

/// The folder, inside Legible's, for the copies.
pub const FOLDER: &str = "Recovery";

/// What a copy's file starts with: its format, so another can't be taken for one.
const HEADER: &str = "legible-recovered 1";
const EXTENSION: &str = "recovered";

/// A copy found in the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    /// The copy's own file.
    pub copy: PathBuf,
    /// The document it's of.
    pub document: PathBuf,
    /// The hash ([`save::hash`]) of the document's bytes as the edits were made to them, so a
    /// file changed since can say so.
    pub base: Option<u64>,
    /// The unsaved text.
    pub text: String,
}

/// A name for this run of Legible's copies: its start, to the nanosecond, so a later run's
/// copies sort after an earlier's.
pub fn session() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}

/// The copy's file for `document` in run `session`, in `folder`: the same for the same document,
/// by any case, in the same run.
pub fn copy_for(folder: &Path, session: u64, document: &Path) -> PathBuf {
    let path = document.to_string_lossy().to_lowercase();
    let stem: String = document
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(40)
        .collect();
    folder.join(format!(
        "{stem}-{:016x}-{session:016x}.{EXTENSION}",
        save::hash(path.as_bytes())
    ))
}

/// Keeps `text`, unsaved in `document` (edits of the bytes whose hash is `base`), in `folder`
/// (made if it's missing), over that document's older copy from run `session`. Returns the
/// copy's file.
pub fn keep(
    folder: &Path,
    session: u64,
    document: &Path,
    base: Option<u64>,
    text: &str,
) -> io::Result<PathBuf> {
    fs::create_dir_all(folder)?;
    let copy = copy_for(folder, session, document);
    let staging = copy.with_extension(format!("{EXTENSION}-new"));
    let written = (|| {
        let mut file = fs::File::create(&staging)?;
        let base = base.map_or("-".to_owned(), |b| format!("{b:016x}"));
        write!(
            file,
            "{HEADER}\n{}\n{base}\n\n{text}",
            document.to_string_lossy()
        )?;
        file.sync_all()?;
        drop(file);
        fs::rename(&staging, &copy)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&staging);
    }
    written.map(|()| copy)
}

/// The copy at `copy`, if it is one.
pub fn read(copy: &Path) -> io::Result<Kept> {
    let bytes = fs::read(copy)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not UTF-8"))?;
    let not_one = || io::Error::new(io::ErrorKind::InvalidData, "not a recovery copy");
    let rest = text.strip_prefix(HEADER).ok_or_else(not_one)?;
    let rest = rest.strip_prefix('\n').ok_or_else(not_one)?;
    let (document, rest) = rest.split_once('\n').ok_or_else(not_one)?;
    let (base, text) = rest.split_once("\n\n").ok_or_else(not_one)?;
    let base = match base {
        "-" => None,
        hex => Some(u64::from_str_radix(hex, 16).map_err(|_| not_one())?),
    };
    if document.is_empty() {
        return Err(not_one());
    }
    Ok(Kept {
        copy: copy.to_owned(),
        document: PathBuf::from(document),
        base,
        text: text.to_owned(),
    })
}

/// Every copy in `folder`, oldest name first; files that aren't copies are left alone, unread.
pub fn list(folder: &Path) -> Vec<Kept> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut copies: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == EXTENSION))
        .collect();
    copies.sort();
    copies.iter().filter_map(|c| read(c).ok()).collect()
}

/// Removes a copy (it's gone already: nothing to do).
pub fn forget(copy: &Path) -> io::Result<()> {
    match fs::remove_file(copy) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_keeps_the_text_and_the_document_and_reads_back_exactly() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join(FOLDER);
        let document = temp.path().join("Notes Q3.md");
        // Blank lines, a final newline and none, and text that looks like a header.
        for text in [
            "# Notes\n\nOne line",
            "\n\nlegible-recovered 1\n\n",
            "",
            "ünïcödé ✓\n",
        ] {
            for base in [Some(0x0123_4567_89ab_cdef), None] {
                let copy = keep(&folder, 7, &document, base, text).unwrap();
                assert!(copy.starts_with(&folder), "never beside the document");
                let kept = read(&copy).unwrap();
                assert_eq!(
                    (kept.document.as_path(), kept.base, kept.text.as_str()),
                    (document.as_path(), base, text)
                );
            }
        }
        // One copy per document in a run, the newest; no staging file left.
        let names = || -> Vec<String> {
            let mut names: Vec<_> = fs::read_dir(&folder)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        assert_eq!(names().len(), 1, "{:?}", names());
        assert!(names()[0].starts_with("Notes Q3-"));
    }

    #[test]
    fn a_later_runs_copy_never_writes_over_an_earlier_runs() {
        let temp = tempfile::tempdir().unwrap();
        let document = Path::new(r"C:\Docs\notes.md");
        let earlier = keep(temp.path(), 1, document, None, "earlier").unwrap();
        let later = keep(temp.path(), 2, document, None, "later").unwrap();
        assert_ne!(earlier, later);
        let texts: Vec<_> = list(temp.path()).into_iter().map(|k| k.text).collect();
        assert_eq!(texts, ["earlier", "later"], "oldest run first");
        // A run's own start names it, and a later one sorts after.
        let first = session();
        assert!(session() >= first);
        assert!(copy_for(temp.path(), 0x0f, document) < copy_for(temp.path(), 0x10, document));
    }

    #[test]
    fn each_document_has_its_own_copy_whatever_the_case_of_its_path() {
        let folder = Path::new(r"C:\Legible\Recovery");
        let a = copy_for(folder, 1, Path::new(r"C:\Docs\notes.md"));
        let path_hash = |p: &Path| p.to_string_lossy().split('-').nth(1).unwrap().to_owned();
        assert_eq!(
            path_hash(&a),
            path_hash(&copy_for(folder, 1, Path::new(r"c:\docs\NOTES.md")))
        );
        assert_eq!(a, copy_for(folder, 1, Path::new(r"c:\docs\notes.md")));
        assert_ne!(a, copy_for(folder, 1, Path::new(r"C:\Other\notes.md")));
        assert_ne!(a, copy_for(folder, 2, Path::new(r"C:\Docs\notes.md")));
        // A name that can't be a file name is kept out of the copy's.
        let odd = copy_for(folder, 1, Path::new(r"C:\Docs\a:b*?.md"));
        assert!(
            odd.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("ab-")
        );
    }

    #[test]
    fn only_copies_are_listed_and_forgetting_one_twice_is_fine() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().to_owned();
        let one = keep(&folder, 1, Path::new(r"C:\a.md"), None, "a").unwrap();
        keep(&folder, 1, Path::new(r"C:\b.md"), Some(1), "b").unwrap();
        fs::write(folder.join("x.recovered"), "something else").unwrap();
        fs::write(
            folder.join("y.recovered"),
            "legible-recovered 1\nC:\\y.md\nnot hex\n\ny",
        )
        .unwrap();
        fs::write(
            folder.join("notes.txt"),
            "legible-recovered 1\nC:\\c.md\n-\n\nc",
        )
        .unwrap();
        let listed = list(&folder);
        let texts: Vec<_> = listed.iter().map(|k| k.text.as_str()).collect();
        assert_eq!(texts, ["a", "b"]);
        forget(&one).unwrap();
        forget(&one).unwrap();
        assert_eq!(list(&folder).len(), 1);
        assert!(list(&temp.path().join("none")).is_empty());
    }
}
