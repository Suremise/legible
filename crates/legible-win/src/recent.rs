//! Recent files: the last ten documents opened, newest first, for the ⋯ menu and the Jump List
//! ([`crate::jumplist`]), kept in `recent.txt` in Legible's folder.
//!
//! - Every successful open adds to it (the canonical path the open already found, so no more
//!   I/O on the UI thread). Each change raises a token: the page names the list it showed by
//!   it, so a pick from a list that has changed since opens nothing ([`Recent::at`]).
//! - **Missing files** are found by [`Checker`], on a thread of its own, since a path on a share
//!   can take long to answer. It works on the newest list only (a request it hasn't started is
//!   replaced, never queued), and only a file that is certainly gone is dropped: one that can't
//!   be reached just now (a share that's down) stays.
//! - The file is one path a line. A path that isn't valid Unicode, which Windows allows, is
//!   kept losslessly as `~` and its UTF-16 code units in hex. It's written to a temporary file
//!   that's then renamed over the old one.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;

/// How many files the list keeps.
pub const MAX: usize = 10;
/// The file it's kept in, in Legible's folder.
pub const FILE: &str = "recent.txt";

/// The recent files, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recent {
    paths: Vec<PathBuf>,
    token: u64,
}

impl Recent {
    /// The list in `file`; empty if there isn't one or it can't be read.
    pub fn load(file: &Path) -> Recent {
        let text = match fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound {
                    tracing::warn!(%error, "couldn't read the recent files");
                }
                return Recent::default();
            }
        };
        let mut recent = Recent::default();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            if let Some(path) = decode(line)
                && !recent.paths.contains(&path)
                && recent.paths.len() < MAX
            {
                recent.paths.push(path);
            }
        }
        recent
    }

    /// Saves to `file`, through a temporary file renamed over it.
    pub fn save(&self, file: &Path) -> io::Result<()> {
        let mut text = String::new();
        for path in &self.paths {
            text.push_str(&encode(path));
            text.push('\n');
        }
        let temp = file.with_extension("txt.tmp");
        fs::write(&temp, text)?;
        fs::rename(&temp, file).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
    }

    /// `path` (canonical) opened: first, once, and the list kept to [`MAX`].
    pub fn add(&mut self, path: PathBuf) {
        if self.paths.first() == Some(&path) {
            return;
        }
        self.paths.retain(|p| p != &path);
        self.paths.insert(0, path);
        self.paths.truncate(MAX);
        self.token += 1;
    }

    /// Drops `gone`; says whether anything was dropped.
    pub fn remove(&mut self, gone: &[PathBuf]) -> bool {
        let before = self.paths.len();
        self.paths.retain(|p| !gone.contains(p));
        let changed = self.paths.len() != before;
        if changed {
            self.token += 1;
        }
        changed
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Raised with every change.
    pub fn token(&self) -> u64 {
        self.token
    }

    /// The file at `index` in the list the page showed as `token`; None if the list has changed
    /// since, or there's no such entry.
    pub fn at(&self, token: u64, index: usize) -> Option<&Path> {
        (token == self.token)
            .then(|| self.paths.get(index))
            .flatten()
            .map(PathBuf::as_path)
    }
}

/// A path as a line of the file.
pub(crate) fn encode(path: &Path) -> String {
    match path.to_str() {
        Some(text) if !text.starts_with('~') => text.to_owned(),
        _ => {
            let units: Vec<String> = path
                .as_os_str()
                .encode_wide()
                .map(|u| format!("{u:04x}"))
                .collect();
            format!("~{}", units.concat())
        }
    }
}

/// A line of the file as a path.
pub(crate) fn decode(line: &str) -> Option<PathBuf> {
    let Some(hex) = line.strip_prefix('~') else {
        return Some(PathBuf::from(line));
    };
    if hex.len() % 4 != 0 {
        return None;
    }
    let units: Option<Vec<u16>> = (0..hex.len())
        .step_by(4)
        .map(|i| u16::from_str_radix(hex.get(i..i + 4)?, 16).ok())
        .collect();
    Some(PathBuf::from(OsString::from_wide(&units?)))
}

/// Whether a file is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Existence {
    Present,
    /// Certainly not there: its name or its folder isn't found.
    Missing,
    /// Couldn't tell just now (a share that's down, access refused).
    Unknown,
}

/// The real check.
pub fn exists(path: &Path) -> Existence {
    match fs::metadata(path) {
        Ok(m) if m.is_file() => Existence::Present,
        Ok(_) => Existence::Missing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Existence::Missing,
        Err(_) => Existence::Unknown,
    }
}

type Check = dyn Fn(&Path) -> Existence + Send + Sync;
type Found = dyn Fn(u64, Vec<PathBuf>) + Send + Sync;

struct Inner {
    /// The newest request: a list's token and its paths. Replaced, never queued.
    slot: Mutex<Option<(u64, Vec<PathBuf>)>>,
    cv: Condvar,
    check: Box<Check>,
    found: Box<Found>,
}

/// Finds the missing files of the newest list on a thread of its own, and reports them with
/// that list's token. The thread ends when the last handle goes (once any check it's in
/// returns).
#[derive(Clone)]
pub struct Checker {
    inner: Arc<Inner>,
}

impl Checker {
    pub fn new(
        check: impl Fn(&Path) -> Existence + Send + Sync + 'static,
        found: impl Fn(u64, Vec<PathBuf>) + Send + Sync + 'static,
    ) -> Checker {
        let inner = Arc::new(Inner {
            slot: Mutex::new(None),
            cv: Condvar::new(),
            check: Box::new(check),
            found: Box::new(found),
        });
        let weak = Arc::downgrade(&inner);
        if let Err(error) = thread::Builder::new()
            .name("recent files".into())
            .spawn(move || run(weak))
        {
            tracing::error!(%error, "couldn't start checking recent files");
        }
        Checker { inner }
    }

    /// Asks for `recent`'s missing files. A request not yet started is replaced.
    pub fn check(&self, recent: &Recent) {
        *lock(&self.inner.slot) = Some((recent.token(), recent.paths().to_vec()));
        self.inner.cv.notify_all();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn run(weak: Weak<Inner>) {
    loop {
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let request = {
            let mut slot = lock(&inner.slot);
            if slot.is_none() {
                // Wake now and then to see whether the service is still wanted.
                slot = inner
                    .cv
                    .wait_timeout(slot, std::time::Duration::from_secs(5))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            slot.take()
        };
        if let Some((token, paths)) = request {
            let missing: Vec<PathBuf> = paths
                .into_iter()
                .filter(|p| (inner.check)(p) == Existence::Missing)
                .collect();
            (inner.found)(token, missing);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    const BOUND: Duration = Duration::from_secs(10);

    fn p(name: &str) -> PathBuf {
        PathBuf::from(format!(r"C:\Notes\{name}.md"))
    }

    #[test]
    fn the_newest_comes_first_once_and_ten_are_kept() {
        let mut recent = Recent::default();
        for i in 0..12 {
            recent.add(p(&i.to_string()));
        }
        assert_eq!(recent.paths().len(), MAX);
        assert_eq!(recent.paths()[0], p("11"));
        assert_eq!(recent.paths()[9], p("2"));
        recent.add(p("5"));
        assert_eq!(recent.paths()[0], p("5"));
        assert_eq!(recent.paths().iter().filter(|x| **x == p("5")).count(), 1);
        assert_eq!(recent.paths().len(), MAX);
    }

    #[test]
    fn a_pick_from_a_list_that_has_changed_opens_nothing() {
        let mut recent = Recent::default();
        recent.add(p("a"));
        recent.add(p("b"));
        let shown = recent.token();
        assert_eq!(recent.at(shown, 1), Some(p("a").as_path()));
        assert_eq!(recent.at(shown, 2), None);
        recent.add(p("c"));
        assert_eq!(recent.at(shown, 1), None);
        assert!(recent.remove(&[p("a")]));
        assert!(!recent.remove(&[p("a")]));
    }

    #[test]
    fn it_survives_a_save_unicode_and_all() {
        use std::os::windows::ffi::OsStringExt;
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join(FILE);
        assert_eq!(Recent::load(&file), Recent::default());
        let mut recent = Recent::default();
        recent.add(PathBuf::from(r"\\?\C:\Привет\笔记 ü.md"));
        // An unpaired surrogate: a valid Windows name, but not Unicode.
        recent.add(PathBuf::from(OsString::from_wide(&[
            0x43, 0x3a, 0x5c, 0xd800, 0x2e, 0x6d, 0x64,
        ])));
        recent.add(PathBuf::from("~tilde.md"));
        recent.save(&file).unwrap();
        assert!(!temp.path().join("recent.txt.tmp").exists());
        let back = Recent::load(&file);
        assert_eq!(back.paths(), recent.paths());
        // A folder that can't be written fails the save.
        assert!(
            recent
                .save(&temp.path().join("missing").join(FILE))
                .is_err()
        );
    }

    #[test]
    fn only_files_certainly_gone_are_reported_missing() {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let checker = Checker::new(
            |path| match path.file_stem().and_then(|s| s.to_str()) {
                Some("gone") => Existence::Missing,
                Some("share") => Existence::Unknown,
                _ => Existence::Present,
            },
            move |token, missing| drop(lock(&tx).send((token, missing))),
        );
        let mut recent = Recent::default();
        for name in ["here", "gone", "share"] {
            recent.add(p(name));
        }
        checker.check(&recent);
        let (token, missing) = rx.recv_timeout(BOUND).unwrap();
        assert_eq!(token, recent.token());
        assert_eq!(missing, [p("gone")]);
    }

    #[test]
    fn a_stuck_check_never_queues_lists_and_reports_the_newest() {
        // A check that waits at a gate the test opens, opened on drop too.
        struct Gate(Arc<(Mutex<bool>, Condvar)>);
        impl Drop for Gate {
            fn drop(&mut self) {
                *lock(&self.0.0) = true;
                self.0.1.notify_all();
            }
        }
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let entered = Arc::new(Mutex::new(0usize));
        let (g, e) = (Arc::clone(&gate), Arc::clone(&entered));
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let checker = Checker::new(
            move |_| {
                *lock(&e) += 1;
                let end = Instant::now() + BOUND;
                let mut open = lock(&g.0);
                while !*open && Instant::now() < end {
                    open =
                        g.1.wait_timeout(open, BOUND)
                            .unwrap_or_else(|x| x.into_inner())
                            .0;
                }
                Existence::Present
            },
            move |token, missing| drop(lock(&tx).send((token, missing))),
        );
        let release = Gate(Arc::clone(&gate));
        let mut recent = Recent::default();
        recent.add(p("first"));
        checker.check(&recent);
        let end = Instant::now() + BOUND;
        while *lock(&entered) == 0 {
            assert!(Instant::now() < end, "the check didn't start");
            thread::yield_now();
        }
        // While it's stuck, many newer lists come; asking returns at once.
        let started = Instant::now();
        for i in 0..100 {
            recent.add(p(&format!("n{i}")));
            checker.check(&recent);
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(release);
        // The stuck one reports, then only the newest: none in between.
        let (first, _) = rx.recv_timeout(BOUND).unwrap();
        let (second, _) = rx.recv_timeout(BOUND).unwrap();
        assert_eq!(first, 1);
        assert_eq!(second, recent.token());
        assert!(rx.recv_timeout(Duration::from_millis(0)).is_err());
    }
}
