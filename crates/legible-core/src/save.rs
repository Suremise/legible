//! Saving that can't lose or mangle work: what a file's text is made of, kept as it was, and
//! the stamp that says whether the file is still what was read.
//!
//! **Formats** ([`decode`], [`encode`]): a file opens as UTF-8 (with or without a BOM) or UTF-16
//! LE or BE (with its BOM); anything else is [`Unsupported`], and opens for reading only. Its
//! line endings are normalised to `\n` for the editor, and saving writes the file's own: so an
//! unedited text encodes back to exactly the bytes it came from. A file that mixes them is saved
//! with the most frequent (a tie goes to the one that comes first), and says so (`mixed`); one
//! with none is LF. A final newline, or none, is part of the text, so it's kept as the text has
//! it.
//!
//! **Stamps** ([`Stamp`]): what a file was when read, taken from the same handle as its bytes,
//! so a stamp is always of the bytes it goes with: their length and hash, and the file's last
//! write time and (where the platform has one) its identity.
//!
//! **Writing** ([`write`]): the new bytes go to a file of their own beside the document, flushed
//! to disk; then the document is checked against the stamp it was read with, and only if it's
//! still that is it replaced by the platform ([`Replace`]: on Windows, `ReplaceFileW`, which
//! keeps the document's attributes and security), the replaced file kept under a backup name
//! until the replace is known to have worked. Whatever goes wrong, the document is as it was, or
//! (if the platform left it half-done) both files are kept and said where; nothing that might be
//! the only copy is ever removed.

use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use crate::i18n::tr;

/// How a file's text is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    /// UTF-8 with its byte-order mark.
    Utf8Bom,
    /// UTF-16, little-endian, with its byte-order mark.
    Utf16Le,
    /// UTF-16, big-endian, with its byte-order mark.
    Utf16Be,
}

/// A file's line ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
    /// A carriage return alone (old Mac files).
    Cr,
}

impl LineEnding {
    fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }
}

/// What a file's text is made of: kept on save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFormat {
    pub encoding: Encoding,
    /// The line ending saving writes: the file's own, or its most frequent.
    pub line_ending: LineEnding,
    /// Whether the file had more than one kind: saving makes them all `line_ending`.
    pub mixed: bool,
}

/// A file's text, decoded: `\n` line endings, and the format to save it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    pub format: TextFormat,
}

/// A file that isn't UTF-8 or UTF-16 with a BOM: it opens for reading only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
    /// Its text as well as it can be read (invalid bytes as U+FFFD), for View alone: never
    /// saved, since it isn't what the file holds.
    pub lossy: String,
}

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16_LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16_BE_BOM: &[u8] = &[0xFE, 0xFF];

/// `bytes`, decoded: UTF-8 (a BOM noted), or UTF-16 with its BOM; anything that isn't exactly
/// one of those is [`Unsupported`].
pub fn decode(bytes: &[u8]) -> Result<Decoded, Unsupported> {
    let (encoding, text) = if let Some(rest) = bytes.strip_prefix(UTF8_BOM) {
        (Encoding::Utf8Bom, String::from_utf8(rest.to_vec()).ok())
    } else if let Some(rest) = bytes.strip_prefix(UTF16_LE_BOM) {
        (Encoding::Utf16Le, utf16(rest, u16::from_le_bytes))
    } else if let Some(rest) = bytes.strip_prefix(UTF16_BE_BOM) {
        (Encoding::Utf16Be, utf16(rest, u16::from_be_bytes))
    } else {
        (Encoding::Utf8, String::from_utf8(bytes.to_vec()).ok())
    };
    let Some(text) = text else {
        return Err(Unsupported {
            lossy: String::from_utf8_lossy(bytes).into_owned(),
        });
    };
    let (text, line_ending, mixed) = normalise(&text);
    Ok(Decoded {
        text,
        format: TextFormat {
            encoding,
            line_ending,
            mixed,
        },
    })
}

/// UTF-16 code units from `bytes` (an even number of them), strictly: an unpaired surrogate
/// is None.
fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units = bytes.as_chunks::<2>().0.iter().map(|&pair| unit(pair));
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .ok()
}

/// `text` with every line ending as `\n`, the ending to save with, and whether it mixed them.
fn normalise(text: &str) -> (String, LineEnding, bool) {
    // By LineEnding: how many of each, and where each first comes (its line).
    let mut counts = [0usize; 3];
    let mut first_at = [usize::MAX; 3];
    let mut lines = 0usize;
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let ending = match c {
            '\r' if chars.peek() == Some(&'\n') => {
                chars.next();
                LineEnding::Crlf
            }
            '\r' => LineEnding::Cr,
            '\n' => LineEnding::Lf,
            c => {
                out.push(c);
                continue;
            }
        };
        counts[ending as usize] += 1;
        first_at[ending as usize] = first_at[ending as usize].min(lines);
        lines += 1;
        out.push('\n');
    }
    let kinds = counts.iter().filter(|&&n| n > 0).count();
    // The most frequent; of those tied, the one that comes first. None at all: LF, Markdown's
    // convention.
    let line_ending = [LineEnding::Lf, LineEnding::Crlf, LineEnding::Cr]
        .into_iter()
        .filter(|e| counts[*e as usize] > 0)
        .max_by_key(|e| {
            (
                counts[*e as usize],
                std::cmp::Reverse(first_at[*e as usize]),
            )
        })
        .unwrap_or(LineEnding::Lf);
    (out, line_ending, kinds > 1)
}

/// `text` (with `\n` line endings) as `format` writes it: its line ending, encoding and BOM.
pub fn encode(text: &str, format: &TextFormat) -> Vec<u8> {
    let text = match format.line_ending {
        LineEnding::Lf => std::borrow::Cow::Borrowed(text),
        ending => std::borrow::Cow::Owned(text.replace('\n', ending.as_str())),
    };
    match format.encoding {
        Encoding::Utf8 => text.into_owned().into_bytes(),
        Encoding::Utf8Bom => [UTF8_BOM, text.as_bytes()].concat(),
        Encoding::Utf16Le => {
            let mut out = UTF16_LE_BOM.to_vec();
            out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            out
        }
        Encoding::Utf16Be => {
            let mut out = UTF16_BE_BOM.to_vec();
            out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
            out
        }
    }
}

/// What a file was when read: its bytes' length and hash, its last write time, and its
/// identity where the platform has one (Windows: the volume and file index). Two stamps are the
/// same file unchanged only if all of them are equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub len: u64,
    pub hash: u64,
    pub written: Option<SystemTime>,
    pub identity: Option<(u64, u64)>,
}

impl Stamp {
    /// The stamp of `bytes`, read from a file with `metadata` and `identity`.
    pub fn of(bytes: &[u8], metadata: &Metadata, identity: Option<(u64, u64)>) -> Stamp {
        Stamp {
            len: bytes.len() as u64,
            hash: hash(bytes),
            written: metadata.modified().ok(),
            identity,
        }
    }
}

/// A file's identity, as the platform knows it (Windows: its volume and file index).
pub type Identity<'a> = dyn Fn(&File) -> io::Result<Option<(u64, u64)>> + 'a;

/// The file at `path`, read whole, with the stamp of exactly those bytes: one handle for both,
/// so nothing written between them can go unnoticed.
pub fn read(path: &Path, identity: &Identity<'_>) -> io::Result<(Vec<u8>, Stamp)> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.read_to_end(&mut bytes)?;
    let identity = identity(&file)?;
    // The metadata again, from the same handle, after the read: a write during it shows.
    let after = file.metadata()?;
    if after.len() != bytes.len() as u64 || after.modified().ok() != metadata.modified().ok() {
        return Err(io::Error::other(tr("error-file-changed-while-read")));
    }
    let stamp = Stamp::of(&bytes, &after, identity);
    Ok((bytes, stamp))
}

/// How the platform's replace went ([`Replace::replace`]).
#[derive(Debug)]
pub enum Replaced {
    /// The document is the new file; the one it replaced is at the backup name.
    Done,
    /// Nothing moved: the document, and the new file, are as they were.
    NotMoved(io::Error),
    /// Another program has the document open in a way that blocks it: nothing moved, and it
    /// may be tried again.
    Busy(io::Error),
    /// Half done: the document is at the backup name, the new file still at its own (Windows'
    /// `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`).
    OriginalAtBackup(io::Error),
}

/// The platform's side of writing.
pub trait Replace {
    /// Replaces `document` with `staged`, keeping `document`'s metadata, and keeping the file
    /// it replaces as `backup`.
    fn replace(&self, document: &Path, staged: &Path, backup: &Path) -> Replaced;
    /// Moves `from` to `to`, which mustn't exist: fails (`AlreadyExists`) if it does.
    fn create(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// An open file's identity, if the platform has one.
    fn identity(&self, file: &File) -> io::Result<Option<(u64, u64)>>;
}

/// Points in [`write`] a test can act at.
pub trait Hooks {
    /// The new bytes are staged, flushed to disk.
    fn staged(&self, _staged: &Path) {}
    /// The document has just been checked against its stamp, and is about to be replaced.
    fn checked(&self) {}
}

/// No hooks.
pub struct NoHooks;
impl Hooks for NoHooks {}

/// Why a write didn't happen.
#[derive(Debug)]
pub enum WriteError {
    /// The document isn't what it was when read: nothing was written.
    Conflict(Conflict),
    /// Nothing was written; the document is as it was.
    Failed(io::Error),
    /// The platform left the replace half-done, and putting the document back failed too:
    /// both files are kept, the document's bytes at `original_at` and the new ones at
    /// `saved_at`.
    Partial {
        original_at: PathBuf,
        saved_at: PathBuf,
        error: io::Error,
    },
    /// The document was replaced, but what it replaced wasn't what was read: someone wrote it
    /// in the moment between the check and the replace. Their bytes are kept at `backup`.
    Raced { backup: PathBuf, stamp: Stamp },
}

/// How the document differs from its stamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// It's another version (written, or replaced by another file).
    Changed,
    /// It's gone.
    Deleted,
    /// A file is there that wasn't (saving one that's new, or recreating one that went).
    Appeared,
}

/// How many times a document another program blocks is tried.
const BUSY_TRIES: u32 = 5;

/// Writes `bytes` as `document`, which must still be what `expected` says (None: there must be
/// no file there, as for a new copy or one recreated). Returns the stamp of what was written,
/// read back from the document.
pub fn write(
    document: &Path,
    bytes: &[u8],
    expected: Option<&Stamp>,
    platform: &dyn Replace,
    hooks: &dyn Hooks,
) -> Result<Stamp, WriteError> {
    let (staged, backup) = reserve(document, bytes).map_err(WriteError::Failed)?;
    hooks.staged(&staged);
    let identity = |file: &File| platform.identity(file);
    let Some(expected) = expected else {
        // New: never over a file that's appeared. Nothing is replaced, so there's no backup.
        let _ = std::fs::remove_file(&backup);
        hooks.checked();
        return match platform.create(&staged, document) {
            Ok(()) => read(document, &identity)
                .map(|(_, stamp)| stamp)
                .map_err(WriteError::Failed),
            Err(error) => {
                let _ = std::fs::remove_file(&staged);
                Err(match error.kind() {
                    io::ErrorKind::AlreadyExists => WriteError::Conflict(Conflict::Appeared),
                    _ => WriteError::Failed(error),
                })
            }
        };
    };
    let mut tries = 0;
    loop {
        // Checked again before every try, so a retry never skips it.
        let now = match read(document, &identity) {
            Ok((_, now)) => now,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                unreserve(&staged, &backup);
                return Err(WriteError::Conflict(Conflict::Deleted));
            }
            Err(error) => {
                unreserve(&staged, &backup);
                return Err(WriteError::Failed(error));
            }
        };
        if now != *expected {
            unreserve(&staged, &backup);
            return Err(WriteError::Conflict(Conflict::Changed));
        }
        hooks.checked();
        match platform.replace(document, &staged, &backup) {
            Replaced::Done => break,
            Replaced::Busy(error) => {
                tries += 1;
                if tries >= BUSY_TRIES {
                    unreserve(&staged, &backup);
                    return Err(WriteError::Failed(error));
                }
                std::thread::sleep(std::time::Duration::from_millis(50 * u64::from(tries)));
            }
            Replaced::NotMoved(error) => {
                unreserve(&staged, &backup);
                return Err(WriteError::Failed(error));
            }
            Replaced::OriginalAtBackup(error) => {
                // Put the document back where it was: never over a file that's appeared there.
                return match platform.create(&backup, document) {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&staged);
                        Err(WriteError::Failed(error))
                    }
                    Err(_) => Err(WriteError::Partial {
                        original_at: backup,
                        saved_at: staged,
                        error,
                    }),
                };
            }
        }
    }
    // Replaced: what it replaced was the version read, or someone's who wrote in between.
    match read(&backup, &identity) {
        Ok((_, replaced)) if replaced.len == expected.len && replaced.hash == expected.hash => {
            let _ = std::fs::remove_file(&backup);
        }
        Ok((_, replaced)) => {
            return Err(WriteError::Raced {
                backup,
                stamp: replaced,
            });
        }
        // Can't be read: kept, and said.
        Err(error) => {
            return Err(WriteError::Partial {
                original_at: backup,
                saved_at: document.to_owned(),
                error,
            });
        }
    }
    read(document, &identity)
        .map(|(_, stamp)| stamp)
        .map_err(WriteError::Failed)
}

/// How many pairs of names a write tries before it gives up (each taken by another file).
const NAME_TRIES: u32 = 32;

/// The staged file (holding `bytes`, flushed) and the backup name beside `document`, both this
/// write's own: each is created new, so a file that's already there under such a name (a
/// crashed run's, or any other) is never written, replaced or removed; another pair is tried.
/// The backup is empty until the replace puts the document there.
fn reserve(document: &Path, bytes: &[u8]) -> io::Result<(PathBuf, PathBuf)> {
    let new = |path: &Path| OpenOptions::new().write(true).create_new(true).open(path);
    for _ in 0..NAME_TRIES {
        let (staged, backup) = beside(document);
        match new(&backup) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
        let mut file = match new(&staged) {
            Ok(file) => file,
            Err(error) => {
                let _ = std::fs::remove_file(&backup);
                if error.kind() == io::ErrorKind::AlreadyExists {
                    continue;
                }
                return Err(error);
            }
        };
        if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            drop(file);
            unreserve(&staged, &backup);
            return Err(error);
        }
        return Ok((staged, backup));
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        tr("error-no-free-name"),
    ))
}

/// Removes what [`reserve`] made, before anything was replaced: the staged file, and the
/// backup, still empty.
fn unreserve(staged: &Path, backup: &Path) {
    let _ = std::fs::remove_file(staged);
    let _ = std::fs::remove_file(backup);
}

/// The staged and backup names beside `document`: hidden, and named for this process.
fn beside(document: &Path) -> (PathBuf, PathBuf) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let name = document
        .file_name()
        .map_or_else(|| "document".into(), |n| n.to_string_lossy());
    let pid = std::process::id();
    let at = |ext: &str| document.with_file_name(format!(".{name}.legible-{pid}-{n}.{ext}"));
    (at("tmp"), at("bak"))
}

/// 64-bit FNV-1a: cheap, and enough to tell one version of a file from another.
pub fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(bytes: &[u8]) -> Decoded {
        let decoded = decode(bytes).unwrap_or_else(|e| panic!("{bytes:?}: unsupported {e:?}"));
        assert_eq!(encode(&decoded.text, &decoded.format), bytes, "{bytes:?}");
        decoded
    }

    fn utf16le(text: &str) -> Vec<u8> {
        [
            UTF16_LE_BOM.to_vec(),
            text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        ]
        .concat()
    }

    fn utf16be(text: &str) -> Vec<u8> {
        [
            UTF16_BE_BOM.to_vec(),
            text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        ]
        .concat()
    }

    #[test]
    fn every_encoding_and_line_ending_comes_back_byte_for_byte() {
        for text in [
            "# Title\n\nText é ✓\n",
            "# Title\r\n\r\nText é ✓\r\n",
            "no final newline",
            "a\r\nb",
        ] {
            let utf8 = round_trip(text.as_bytes());
            assert_eq!(utf8.format.encoding, Encoding::Utf8);
            assert!(!utf8.text.contains('\r'), "{text:?}");
            assert_eq!(
                round_trip(&[UTF8_BOM, text.as_bytes()].concat())
                    .format
                    .encoding,
                Encoding::Utf8Bom
            );
            assert_eq!(
                round_trip(&utf16le(text)).format.encoding,
                Encoding::Utf16Le
            );
            assert_eq!(
                round_trip(&utf16be(text)).format.encoding,
                Encoding::Utf16Be
            );
        }
        let crlf = round_trip(b"a\r\nb\r\n");
        assert_eq!(
            (
                crlf.text.as_str(),
                crlf.format.line_ending,
                crlf.format.mixed
            ),
            ("a\nb\n", LineEnding::Crlf, false)
        );
        // Lone CRs, kept as they were.
        let cr = round_trip(b"a\rb\r");
        assert_eq!(
            (cr.text.as_str(), cr.format.line_ending),
            ("a\nb\n", LineEnding::Cr)
        );
        // A final newline or none, and several, as the text has them.
        assert_eq!(round_trip(b"a\n\n\n").text, "a\n\n\n");
        assert_eq!(round_trip(b"a").text, "a");
        // A character outside the BMP in UTF-16 (a surrogate pair).
        round_trip(&utf16le("emoji 😀\r\n"));
    }

    #[test]
    fn empty_and_bom_only_files_keep_what_they_are() {
        let empty = round_trip(b"");
        assert_eq!(
            (
                empty.text.as_str(),
                empty.format.encoding,
                empty.format.line_ending
            ),
            ("", Encoding::Utf8, LineEnding::Lf)
        );
        assert_eq!(round_trip(UTF8_BOM).format.encoding, Encoding::Utf8Bom);
        assert_eq!(round_trip(UTF16_LE_BOM).format.encoding, Encoding::Utf16Le);
        assert_eq!(round_trip(UTF16_BE_BOM).format.encoding, Encoding::Utf16Be);
    }

    #[test]
    fn a_mixed_file_is_saved_with_its_most_frequent_ending_and_says_so() {
        // Two CRLF, one LF: CRLF.
        let mixed = decode(b"a\r\nb\nc\r\n").unwrap();
        assert_eq!(
            (
                mixed.text.as_str(),
                mixed.format.line_ending,
                mixed.format.mixed
            ),
            ("a\nb\nc\n", LineEnding::Crlf, true)
        );
        assert_eq!(encode(&mixed.text, &mixed.format), b"a\r\nb\r\nc\r\n");
        // A tie: the one that comes first.
        assert_eq!(
            decode(b"a\nb\r\n").unwrap().format.line_ending,
            LineEnding::Lf
        );
        assert_eq!(
            decode(b"a\r\nb\n").unwrap().format.line_ending,
            LineEnding::Crlf
        );
        assert_eq!(
            decode(b"a\rb\n").unwrap().format.line_ending,
            LineEnding::Cr
        );
        // A tie the very first ending isn't in: of the tied, the one that comes first (LF, at
        // line 2, before CRLF at line 4).
        assert_eq!(
            decode(b"a\rb\nc\nd\r\ne\r\n").unwrap().format.line_ending,
            LineEnding::Lf
        );
        assert_eq!(
            decode(b"a\rb\r\nc\r\nd\ne\n").unwrap().format.line_ending,
            LineEnding::Crlf
        );
        assert!(!decode(b"a\nb\n").unwrap().format.mixed);
    }

    #[test]
    fn anything_else_is_unsupported_and_kept_out_of_saving() {
        // Latin-1, a lone UTF-16 surrogate, an odd UTF-16 length, invalid UTF-8 after a BOM.
        for bytes in [
            &b"caf\xe9"[..],
            &[0xFF, 0xFE, 0x00, 0xD8][..],
            &[0xFF, 0xFE, 0x41][..],
            &[0xEF, 0xBB, 0xBF, 0xC3][..],
        ] {
            let lossy = decode(bytes).expect_err("supported").lossy;
            assert!(lossy.contains('\u{FFFD}') || !lossy.is_empty(), "{bytes:?}");
        }
    }

    /// A replace on the real filesystem that can act out each of the platform's outcomes.
    struct FakeReplace {
        outcome: std::cell::Cell<&'static str>,
        busy: std::cell::Cell<u32>,
        create_fails: bool,
    }

    impl FakeReplace {
        fn doing(outcome: &'static str) -> FakeReplace {
            FakeReplace {
                outcome: std::cell::Cell::new(outcome),
                busy: std::cell::Cell::new(0),
                create_fails: false,
            }
        }
    }

    impl Replace for FakeReplace {
        fn replace(&self, document: &Path, staged: &Path, backup: &Path) -> Replaced {
            let error = || io::Error::other(self.outcome.get());
            match self.outcome.get() {
                "done" => {
                    std::fs::rename(document, backup).unwrap();
                    std::fs::rename(staged, document).unwrap();
                    Replaced::Done
                }
                "busy" if self.busy.get() > 0 => {
                    self.busy.set(self.busy.get() - 1);
                    Replaced::Busy(error())
                }
                "busy" => {
                    self.outcome.set("done");
                    self.replace(document, staged, backup)
                }
                "not moved" => Replaced::NotMoved(error()),
                "half done" => {
                    // As Windows documents ERROR_UNABLE_TO_MOVE_REPLACEMENT_2.
                    std::fs::rename(document, backup).unwrap();
                    Replaced::OriginalAtBackup(error())
                }
                "panic" => panic!("the platform died mid-replace"),
                other => unreachable!("{other}"),
            }
        }
        fn create(&self, from: &Path, to: &Path) -> io::Result<()> {
            if self.create_fails || to.exists() {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "there"));
            }
            std::fs::rename(from, to)
        }
        fn identity(&self, _: &File) -> io::Result<Option<(u64, u64)>> {
            Ok(None)
        }
    }

    /// Hooks that run a closure at a point.
    struct At<F: Fn()> {
        checked: F,
    }
    impl<F: Fn()> Hooks for At<F> {
        fn checked(&self) {
            (self.checked)();
        }
    }

    fn document(text: &[u8]) -> (tempfile::TempDir, PathBuf, Stamp) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, text).unwrap();
        let stamp = read(&file, &|_| Ok(None)).unwrap().1;
        (dir, file, stamp)
    }

    /// The names in `dir`, sorted.
    fn left(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_write_replaces_the_document_and_leaves_nothing_else() {
        let (dir, file, stamp) = document(b"old");
        let written = write(
            &file,
            b"new",
            Some(&stamp),
            &FakeReplace::doing("done"),
            &NoHooks,
        )
        .unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"new");
        assert_eq!((written.len, written.hash), (3, hash(b"new")));
        assert_eq!(left(dir.path()), ["notes.md"]);
    }

    #[test]
    fn a_document_changed_or_gone_since_it_was_read_is_never_written() {
        let (dir, file, stamp) = document(b"old");
        std::fs::write(&file, b"theirs").unwrap();
        let err = write(
            &file,
            b"mine",
            Some(&stamp),
            &FakeReplace::doing("done"),
            &NoHooks,
        )
        .unwrap_err();
        assert!(
            matches!(err, WriteError::Conflict(Conflict::Changed)),
            "{err:?}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"theirs");
        assert_eq!(left(dir.path()), ["notes.md"]);

        std::fs::remove_file(&file).unwrap();
        let err = write(
            &file,
            b"mine",
            Some(&stamp),
            &FakeReplace::doing("done"),
            &NoHooks,
        )
        .unwrap_err();
        assert!(
            matches!(err, WriteError::Conflict(Conflict::Deleted)),
            "{err:?}"
        );
        assert!(left(dir.path()).is_empty());
    }

    #[test]
    fn a_write_after_the_check_is_kept_never_lost() {
        // Someone writes in the moment between the check and the replace.
        let (dir, file, stamp) = document(b"old");
        let hooks = At {
            checked: || std::fs::write(&file, b"theirs, just now").unwrap(),
        };
        let err = write(
            &file,
            b"mine",
            Some(&stamp),
            &FakeReplace::doing("done"),
            &hooks,
        )
        .unwrap_err();
        let WriteError::Raced {
            backup,
            stamp: theirs,
        } = err
        else {
            panic!("{err:?}");
        };
        assert_eq!(std::fs::read(&backup).unwrap(), b"theirs, just now");
        assert_eq!(theirs.hash, hash(b"theirs, just now"));
        assert_eq!(std::fs::read(&file).unwrap(), b"mine");
        assert_eq!(left(dir.path()).len(), 2, "the document and their backup");
    }

    #[test]
    fn a_replace_that_fails_leaves_the_document_and_cleans_up_its_own_file() {
        let (dir, file, stamp) = document(b"old");
        let err = write(
            &file,
            b"new",
            Some(&stamp),
            &FakeReplace::doing("not moved"),
            &NoHooks,
        )
        .unwrap_err();
        assert!(matches!(err, WriteError::Failed(_)), "{err:?}");
        assert_eq!(std::fs::read(&file).unwrap(), b"old");
        assert_eq!(left(dir.path()), ["notes.md"]);
    }

    #[test]
    fn a_document_blocked_for_a_moment_is_tried_again_checked_each_time() {
        let (_dir, file, stamp) = document(b"old");
        let replace = FakeReplace::doing("busy");
        replace.busy.set(2);
        write(&file, b"new", Some(&stamp), &replace, &NoHooks).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"new");

        // Blocked, then changed before the next try: the check stops it.
        let (_dir, file, stamp) = document(b"old");
        let replace = FakeReplace::doing("busy");
        replace.busy.set(1);
        let tries = std::cell::Cell::new(0);
        let hooks = At {
            checked: || {
                tries.set(tries.get() + 1);
                if tries.get() == 1 {
                    std::fs::write(&file, b"theirs").unwrap();
                }
            },
        };
        // The first try is blocked; the second's check finds their version.
        let err = write(&file, b"mine", Some(&stamp), &replace, &hooks).unwrap_err();
        assert!(
            matches!(err, WriteError::Conflict(Conflict::Changed)),
            "{err:?}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"theirs");
    }

    #[test]
    fn a_half_done_replace_puts_the_document_back_or_keeps_both() {
        let (dir, file, stamp) = document(b"old");
        let err = write(
            &file,
            b"new",
            Some(&stamp),
            &FakeReplace::doing("half done"),
            &NoHooks,
        )
        .unwrap_err();
        assert!(matches!(err, WriteError::Failed(_)), "{err:?}");
        assert_eq!(std::fs::read(&file).unwrap(), b"old");
        assert_eq!(left(dir.path()), ["notes.md"]);

        // Putting it back fails too: nothing is removed, and both are said.
        let (dir, file, stamp) = document(b"old");
        let mut replace = FakeReplace::doing("half done");
        replace.create_fails = true;
        let err = write(&file, b"new", Some(&stamp), &replace, &NoHooks).unwrap_err();
        let WriteError::Partial {
            original_at,
            saved_at,
            ..
        } = err
        else {
            panic!("{err:?}");
        };
        assert_eq!(std::fs::read(&original_at).unwrap(), b"old");
        assert_eq!(std::fs::read(&saved_at).unwrap(), b"new");
        assert_eq!(left(dir.path()).len(), 2);
    }

    #[test]
    fn a_crash_before_the_replace_leaves_the_document_as_it_was() {
        let (_dir, file, stamp) = document(b"old");
        let crashed = std::panic::catch_unwind(|| {
            let _ = write(
                &file,
                b"new",
                Some(&stamp),
                &FakeReplace::doing("panic"),
                &NoHooks,
            );
        });
        assert!(crashed.is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"old");
    }

    #[test]
    fn files_already_under_a_writes_names_are_never_touched() {
        // A crashed run's (or any other) files, under the very names this process would take
        // first: the write takes others, and leaves them as they are.
        let (dir, file, stamp) = document(b"old");
        let pid = std::process::id();
        let theirs: Vec<PathBuf> = (0..8)
            .flat_map(|n| {
                ["tmp", "bak"].map(|ext| {
                    dir.path()
                        .join(format!(".notes.md.legible-{pid}-{n}.{ext}"))
                })
            })
            .collect();
        for path in &theirs {
            std::fs::write(path, b"valuable").unwrap();
        }
        write(
            &file,
            b"new",
            Some(&stamp),
            &FakeReplace::doing("done"),
            &NoHooks,
        )
        .unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"new");
        for path in &theirs {
            assert_eq!(
                std::fs::read(path).unwrap(),
                b"valuable",
                "{}",
                path.display()
            );
        }
        // A refused write, too, removes only its own.
        let (_, stamp) = read(&file, &|_| Ok(None)).unwrap();
        std::fs::write(&file, b"changed").unwrap();
        let err = write(
            &file,
            b"mine",
            Some(&stamp),
            &FakeReplace::doing("done"),
            &NoHooks,
        )
        .unwrap_err();
        assert!(
            matches!(err, WriteError::Conflict(Conflict::Changed)),
            "{err:?}"
        );
        for path in &theirs {
            assert_eq!(std::fs::read(path).unwrap(), b"valuable");
        }
        assert_eq!(left(dir.path()).len(), 1 + theirs.len());
    }

    #[test]
    fn a_new_file_is_never_written_over_one_that_appeared() {
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("copy.md");
        write(
            &copy,
            b"copied",
            None,
            &FakeReplace::doing("done"),
            &NoHooks,
        )
        .unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"copied");
        // There now: a second "new" write is refused, the file untouched.
        let err = write(&copy, b"again", None, &FakeReplace::doing("done"), &NoHooks).unwrap_err();
        assert!(
            matches!(err, WriteError::Conflict(Conflict::Appeared)),
            "{err:?}"
        );
        assert_eq!(std::fs::read(&copy).unwrap(), b"copied");
        assert_eq!(left(dir.path()), ["copy.md"]);
    }

    #[test]
    fn a_stamp_is_of_the_bytes_read_and_tells_versions_apart() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.md");
        std::fs::write(&file, b"first").unwrap();
        let none = |_: &File| Ok(None);
        let (bytes, stamp) = read(&file, &none).unwrap();
        assert_eq!(
            (bytes.as_slice(), stamp.len, stamp.hash),
            (&b"first"[..], 5, hash(b"first"))
        );
        assert_eq!(
            read(&file, &none).unwrap().1,
            stamp,
            "the same file, the same stamp"
        );
        // The same length, another text: the hash tells them apart whatever the clock says.
        std::fs::write(&file, b"other").unwrap();
        let changed = read(&file, &none).unwrap().1;
        assert_eq!(changed.len, stamp.len);
        assert_ne!(changed, stamp);
        assert_ne!(hash(b"first"), hash(b"other"));
    }
}
