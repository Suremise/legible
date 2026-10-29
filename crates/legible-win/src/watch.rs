//! Live reload's watcher: **one** service thread for the whole process polls every open
//! document, and the pictures it uses, every quarter of a second.
//!
//! - **What's on screen is the starting point.** A window registers with the document's
//!   snapshot taken *before* it read the file, and its pictures' state just after it rendered,
//!   so a change in between is reported on the first poll, never taken as the starting point.
//! - **A change** is acted on once two polls in a row agree (a debounce of about half a second,
//!   which also waits out editors that write in steps). A snapshot is a file's identity (its
//!   volume and index), creation time, last write and size, so a file replaced by a rename is
//!   seen as new even when the replacement kept its times and size. The changed document is read
//!   and prepared (rendered, by the caller's function) on this thread, so the UI thread never
//!   touches a watched file after opening it; the pictures the new render uses are watched from
//!   then on, from their state just after it.
//! - **A deletion** is reported once the document has been missing for a second: a file
//!   replaced by a rename is missing only for a moment, and comes back as a change.
//! - **Pictures** are watched by where they'd be ([`DocRoot::candidate`]), present or not; one
//!   that appears, changes or goes is a change of the document, never a deletion. One that
//!   resolves outside the document's folder counts as absent.
//! - **A limit of polling:** a write that keeps the file's size, made within the same tick of
//!   the filesystem's clock as the one before it, looks the same; the next write that differs
//!   is caught.
//! - **A read that fails** (half-written, locked) is reported, and the change is acted on again
//!   once two more polls agree, so nothing is lost and nothing on screen is wiped.
//!
//! **The bound:** there is never more than one watcher thread, however many windows open and
//! close. Registering and unregistering only take a lock, which is never held during file I/O,
//! so closing a window never waits on the disk. A call stuck on a network share stalls live
//! reload (for every window) until it returns, but can't pile up threads; a cycle that takes
//! more than 5 seconds is logged. The thread ends when the last handle to the service goes.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use legible_core::save::{self, Stamp};

use crate::confine::DocRoot;
use crate::win32;

/// How often the files are looked at.
pub const INTERVAL: Duration = Duration::from_millis(250);
/// How long the document must be missing before it counts as deleted.
pub const GRACE: Duration = Duration::from_secs(1);
/// A poll cycle slower than this is logged.
const SLOW_CYCLE: Duration = Duration::from_secs(5);

/// What a file looks like on disk, as far as noticing a change goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub volume: u32,
    pub index: u64,
    pub created: u64,
    pub written: u64,
    pub len: u64,
}

/// The filesystem, as the watcher uses it: the real one, or a test's.
pub trait Fs: Send + Sync + 'static {
    fn snapshot(&self, path: &Path) -> io::Result<Snapshot>;
    /// The document's bytes, with the stamp of exactly those bytes (one handle for both).
    fn read(&self, path: &Path) -> io::Result<(Vec<u8>, Stamp)>;
    /// Whether a picture is there and really inside the document's folder.
    fn holds(&self, root: &DocRoot, path: &Path) -> bool;
    /// A cycle has just copied what's registered, and is about to take the rescans asked for:
    /// nothing, for a real filesystem; a test holds a cycle here.
    fn entries_copied(&self) {}
}

/// The real filesystem.
pub struct RealFs;

impl Fs for RealFs {
    fn snapshot(&self, path: &Path) -> io::Result<Snapshot> {
        use std::os::windows::fs::MetadataExt;
        // Opened with every sharing mode (std's default), so nobody's writes are blocked.
        let file = File::open(path)?;
        let m = file.metadata()?;
        let (volume, index) = win32::file_identity(&file)?;
        Ok(Snapshot {
            volume,
            index,
            created: m.creation_time(),
            written: m.last_write_time(),
            len: m.file_size(),
        })
    }

    fn read(&self, path: &Path) -> io::Result<(Vec<u8>, Stamp)> {
        save::read(path, &win32::identity)
    }

    fn holds(&self, root: &DocRoot, path: &Path) -> bool {
        root.holds(path)
    }
}

/// What a window's watch covers.
#[derive(Debug, Clone)]
pub struct Watched {
    pub document: PathBuf,
    /// The document's folder, for checking pictures; None if it couldn't be found.
    pub root: Option<DocRoot>,
    /// Where its pictures would be ([`DocRoot::candidate`]), present or not.
    pub pictures: Vec<PathBuf>,
}

/// What the watcher found.
#[derive(Debug)]
pub enum Report<T> {
    /// The document or a picture changed: the document as it now is, prepared. `epoch` is the
    /// registration it was read under ([`Watch::register`]): one from before the window last
    /// registered is out of date.
    Changed { key: u64, epoch: u64, prepared: T },
    /// Something changed, but the document couldn't be read; it's tried again.
    ReadFailed { key: u64, error: String },
    /// The document has been missing for the grace period.
    Deleted { key: u64 },
}

/// Prepares a changed document for the window, on the service's thread, from its bytes and
/// their stamp: returns what to report and the pictures it uses.
type Prepare<T> = dyn Fn(&Watched, Vec<u8>, Stamp) -> (T, Vec<PathBuf>) + Send + Sync;

/// What a poll saw: the document (None: missing) and each picture (None: absent).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    document: Option<Snapshot>,
    pictures: Vec<Option<Snapshot>>,
}

struct Entry {
    watched: Arc<Watched>,
    /// What the window shows, until the service takes over.
    shown: Option<Seen>,
    /// How many times the window has registered it: what its reports are read under.
    epoch: u64,
}

struct Inner<T> {
    entries: Mutex<HashMap<u64, Entry>>,
    rescans: Mutex<HashSet<u64>>,
    fs: Box<dyn Fs>,
    prepare: Box<Prepare<T>>,
    report: Box<dyn Fn(Report<T>) + Send + Sync>,
    interval: Duration,
    grace: Duration,
    spawned: AtomicUsize,
    cycles: AtomicUsize,
    /// Set to poll now rather than at the next interval.
    wake: (Mutex<bool>, Condvar),
}

/// The watch service. Cloning it shares the one thread.
pub struct Watch<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Clone for Watch<T> {
    fn clone(&self) -> Self {
        Watch {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: Send + 'static> Watch<T> {
    /// The service over the real filesystem: `prepare` and `report` are called on its thread.
    pub fn new(
        prepare: impl Fn(&Watched, Vec<u8>, Stamp) -> (T, Vec<PathBuf>) + Send + Sync + 'static,
        report: impl Fn(Report<T>) + Send + Sync + 'static,
    ) -> Watch<T> {
        Watch::with(RealFs, prepare, report, INTERVAL, GRACE)
    }

    /// The service over `fs`, polling every `interval`, with `grace` before a deletion.
    pub fn with(
        fs: impl Fs,
        prepare: impl Fn(&Watched, Vec<u8>, Stamp) -> (T, Vec<PathBuf>) + Send + Sync + 'static,
        report: impl Fn(Report<T>) + Send + Sync + 'static,
        interval: Duration,
        grace: Duration,
    ) -> Watch<T> {
        let inner = Arc::new(Inner {
            entries: Mutex::default(),
            rescans: Mutex::default(),
            fs: Box::new(fs),
            prepare: Box::new(prepare),
            report: Box::new(report),
            interval,
            grace,
            spawned: AtomicUsize::new(0),
            cycles: AtomicUsize::new(0),
            wake: (Mutex::new(false), Condvar::new()),
        });
        let weak = Arc::downgrade(&inner);
        inner.spawned.fetch_add(1, Ordering::SeqCst);
        let spawned = thread::Builder::new()
            .name("live reload".into())
            .spawn(move || run(weak));
        if let Err(error) = spawned {
            tracing::error!(%error, "couldn't start live reload");
        }
        Watch { inner }
    }

    /// The document's snapshot now, taken before a window reads it: what
    /// [`register`](Self::register) starts from.
    pub fn document_snapshot(&self, document: &Path) -> Option<Snapshot> {
        self.inner.fs.snapshot(document).ok()
    }

    /// Starts watching under `key`, from what the window shows: `document`, the document's
    /// snapshot from before it was read (None: it wasn't there), and its pictures as they are
    /// now, just after the render. Called once, when a window opens.
    ///
    /// Registering a key again (a save of the window's own, say) makes `document` what the
    /// window shows from now on, and starts a new epoch, which it returns: a report read under
    /// an older one is out of date.
    pub fn register(&self, key: u64, watched: Watched, document: Option<Snapshot>) -> u64 {
        let shown = Seen {
            document,
            pictures: pictures_now(&*self.inner.fs, &watched),
        };
        let mut entries = lock(&self.inner.entries);
        let epoch = entries.get(&key).map_or(1, |e| e.epoch + 1);
        entries.insert(
            key,
            Entry {
                watched: Arc::new(watched),
                shown: Some(shown),
                epoch,
            },
        );
        epoch
    }

    /// Stops watching `key`. Returns at once, whatever the service thread is doing.
    pub fn unregister(&self, key: u64) {
        lock(&self.inner.entries).remove(&key);
    }

    /// Acts on `key`'s document again on the next agreeing polls, changed or not: for a report
    /// that couldn't be shown.
    pub fn rescan(&self, key: u64) {
        lock(&self.inner.rescans).insert(key);
    }

    /// How many service threads have been started: one.
    pub fn threads_started(&self) -> usize {
        self.inner.spawned.load(Ordering::SeqCst)
    }

    /// How many poll cycles have finished.
    pub fn cycles(&self) -> usize {
        self.inner.cycles.load(Ordering::SeqCst)
    }

    /// Asks for a poll now, rather than at the next interval.
    pub fn wake(&self) {
        let (flag, cv) = &self.inner.wake;
        *lock(flag) = true;
        cv.notify_all();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Each picture's state now: present inside the folder, or absent.
fn pictures_now(fs: &dyn Fs, watched: &Watched) -> Vec<Option<Snapshot>> {
    watched
        .pictures
        .iter()
        .map(|picture| {
            let inside = watched
                .root
                .as_ref()
                .is_some_and(|root| fs.holds(root, picture));
            inside.then(|| fs.snapshot(picture).ok()).flatten()
        })
        .collect()
}

#[derive(Default)]
struct Track {
    baseline: Option<Seen>,
    candidate: Option<Seen>,
    missing_since: Option<Instant>,
    deleted_reported: bool,
    force: bool,
    last_error: Option<io::ErrorKind>,
}

fn run<T>(weak: Weak<Inner<T>>) {
    let mut tracks: HashMap<u64, Track> = HashMap::new();
    let mut slow_logged = false;
    loop {
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let started = Instant::now();
        // Copied under the locks; all I/O happens after they're released. The rescans are taken
        // while the entries are still locked (always in that order: entries, then rescans), so
        // a rescan of a window registered after this copy waits for the next cycle, which sees
        // that window, rather than being taken by this one, which doesn't.
        let (entries, rescans) = {
            let mut registered = lock(&inner.entries);
            let entries: Vec<(u64, Arc<Watched>, Option<Seen>, u64)> = registered
                .iter_mut()
                .map(|(key, e)| (*key, Arc::clone(&e.watched), e.shown.take(), e.epoch))
                .collect();
            inner.fs.entries_copied();
            let rescans: HashSet<u64> = std::mem::take(&mut *lock(&inner.rescans));
            (entries, rescans)
        };
        tracks.retain(|key, _| entries.iter().any(|(k, ..)| k == key));
        for (key, watched, shown, epoch) in entries {
            let track = tracks.entry(key).or_default();
            if let Some(shown) = shown {
                track.baseline = Some(shown);
            }
            if rescans.contains(&key) {
                track.force = true;
            }
            poll(&inner, key, epoch, &watched, track);
        }
        let took = started.elapsed();
        if took > SLOW_CYCLE && !slow_logged {
            tracing::warn!(
                seconds = took.as_secs(),
                "live reload's files were slow to answer"
            );
            slow_logged = true;
        } else if took <= SLOW_CYCLE {
            slow_logged = false;
        }
        inner.cycles.fetch_add(1, Ordering::SeqCst);
        // Wait for the interval, or a wake.
        let (flag, cv) = &inner.wake;
        let mut woken = lock(flag);
        if !*woken {
            woken = cv
                .wait_timeout(woken, inner.interval)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        *woken = false;
    }
}

fn poll<T>(inner: &Inner<T>, key: u64, epoch: u64, watched: &Arc<Watched>, track: &mut Track) {
    let document = match inner.fs.snapshot(&watched.document) {
        Ok(snapshot) => {
            track.last_error = None;
            Some(snapshot)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            // Not knowing isn't a change: wait for the next poll.
            if track.last_error != Some(error.kind()) {
                tracing::warn!(%error, file = %watched.document.display(), "couldn't look at a watched file");
                track.last_error = Some(error.kind());
            }
            return;
        }
    };
    let now = Seen {
        document,
        pictures: pictures_now(&*inner.fs, watched),
    };
    let Some(baseline) = &track.baseline else {
        track.baseline = Some(now);
        return;
    };
    if !track.force && *baseline == now {
        track.candidate = None;
        track.missing_since = None;
        return;
    }

    if now.document.is_none() {
        track.candidate = None;
        let since = *track.missing_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= inner.grace && !track.deleted_reported {
            if is_watched(inner, key) {
                (inner.report)(Report::Deleted { key });
            }
            track.deleted_reported = true;
            track.force = false;
            track.baseline = Some(now);
        }
        return;
    }
    track.missing_since = None;
    track.deleted_reported = false;

    // Debounce: act once two polls in a row agree.
    if track.candidate.as_ref() != Some(&now) {
        track.candidate = Some(now);
        return;
    }
    track.candidate = None;
    let (bytes, stamp) = match inner.fs.read(&watched.document) {
        Ok(read) => read,
        Err(error) => {
            // The baseline stays, so it's acted on again once two polls agree.
            if is_watched(inner, key) {
                (inner.report)(Report::ReadFailed {
                    key,
                    error: error.to_string(),
                });
            }
            return;
        }
    };
    let (prepared, pictures) = (inner.prepare)(watched, bytes, stamp);
    // The new render's pictures, watched from their state now (after it, before the window
    // loads them).
    let rendered = Watched {
        pictures,
        ..(**watched).clone()
    };
    let pictures_seen = pictures_now(&*inner.fs, &rendered);
    {
        let mut entries = lock(&inner.entries);
        let Some(entry) = entries.get_mut(&key) else {
            return;
        };
        // Registered again meanwhile (the window's own save): what it registered stands.
        if entry.epoch != epoch {
            return;
        }
        entry.watched = Arc::new(rendered);
    }
    track.force = false;
    track.baseline = Some(Seen {
        document: now.document,
        pictures: pictures_seen,
    });
    (inner.report)(Report::Changed {
        key,
        epoch,
        prepared,
    });
}

fn is_watched<T>(inner: &Inner<T>, key: u64) -> bool {
    lock(&inner.entries).contains_key(&key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc::{self, Receiver};

    /// Every wait in these tests ends by this deadline at the latest.
    const BOUND: Duration = Duration::from_secs(10);
    /// Polls only when a test steps the service.
    const NEVER: Duration = Duration::from_secs(3600);

    /// What a test's prepare returns: the text, and the pictures it names on lines `pic:<name>`.
    fn prepare(watched: &Watched, bytes: Vec<u8>, _: Stamp) -> (String, Vec<PathBuf>) {
        let source = String::from_utf8(bytes).unwrap();
        let pictures = source
            .lines()
            .filter_map(|l| l.strip_prefix("pic:"))
            .filter_map(|name| watched.root.as_ref()?.candidate(&format!("/{name}")))
            .collect();
        (source, pictures)
    }

    fn service(fs: impl Fs, grace: Duration) -> (Watch<String>, Receiver<Report<String>>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let watch = Watch::with(fs, prepare, move |r| drop(lock(&tx).send(r)), NEVER, grace);
        (watch, rx)
    }

    /// Waits for `check`, to the deadline, failing with `what`.
    fn until(what: &str, mut check: impl FnMut() -> bool) {
        let end = Instant::now() + BOUND;
        while !check() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            thread::yield_now();
        }
    }

    /// Polls twice from now: a whole cycle that started after this call, and the debounce's
    /// second look. Every report from them has been sent when it returns.
    fn step<T: Send + 'static>(watch: &Watch<T>) {
        for _ in 0..2 {
            let start = watch.cycles();
            // A cycle already under way may have looked before the change: wait for the next.
            until("a poll", || {
                if watch.cycles() < start + 2 {
                    watch.wake();
                    false
                } else {
                    true
                }
            });
        }
    }

    /// Opens `doc` as a window does: snapshot, read, then register.
    fn open(watch: &Watch<String>, key: u64, doc: &Path) {
        let before = watch.document_snapshot(doc);
        let (bytes, stamp) = save::read(doc, &|_| Ok(None)).unwrap();
        let root = DocRoot::for_document(doc).ok();
        let bare = Watched {
            document: doc.to_owned(),
            root,
            pictures: Vec::new(),
        };
        let pictures = prepare(&bare, bytes, stamp).1;
        watch.register(key, Watched { pictures, ..bare }, before);
    }

    fn changed(rx: &Receiver<Report<String>>) -> String {
        match rx.recv_timeout(BOUND).expect("a report in time") {
            Report::Changed { prepared, .. } => prepared,
            other => panic!("not a change: {other:?}"),
        }
    }

    fn nothing(rx: &Receiver<Report<String>>) {
        if let Ok(report) = rx.try_recv() {
            panic!("an unexpected report: {report:?}");
        }
    }

    #[test]
    fn a_write_is_reported_with_the_new_text() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "one").unwrap();
        let (watch, rx) = service(RealFs, GRACE);
        open(&watch, 1, &doc);
        step(&watch);
        nothing(&rx);
        fs::write(&doc, "two, a longer text").unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "two, a longer text");
    }

    #[test]
    fn a_change_before_the_first_poll_is_not_lost() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "shown").unwrap();
        let (watch, rx) = service(RealFs, Duration::ZERO);
        open(&watch, 1, &doc);
        fs::write(&doc, "written before the first poll").unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "written before the first poll");
        // And a deletion before the first poll.
        let gone = temp.path().join("gone.md");
        fs::write(&gone, "shown").unwrap();
        open(&watch, 2, &gone);
        fs::remove_file(&gone).unwrap();
        step(&watch);
        assert!(matches!(
            rx.recv_timeout(BOUND),
            Ok(Report::Deleted { key: 2 })
        ));
    }

    #[test]
    fn a_replacement_keeping_the_times_and_size_is_seen() {
        use std::os::windows::fs::FileTimesExt;
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "old").unwrap();
        let (watch, rx) = service(RealFs, GRACE);
        open(&watch, 1, &doc);
        step(&watch);
        let m = fs::metadata(&doc).unwrap();
        let new = temp.path().join("doc.md.tmp");
        fs::write(&new, "NEW").unwrap();
        let times = fs::FileTimes::new()
            .set_modified(m.modified().unwrap())
            .set_created(m.created().unwrap());
        fs::File::options()
            .write(true)
            .open(&new)
            .unwrap()
            .set_times(times)
            .unwrap();
        fs::rename(&new, &doc).unwrap();
        let after = fs::metadata(&doc).unwrap();
        assert_eq!(after.modified().unwrap(), m.modified().unwrap());
        assert_eq!(after.created().unwrap(), m.created().unwrap());
        assert_eq!(after.len(), m.len());
        step(&watch);
        assert_eq!(changed(&rx), "NEW");
    }

    #[test]
    fn a_replace_by_rename_is_a_change_not_a_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "one").unwrap();
        let (watch, rx) = service(RealFs, NEVER);
        open(&watch, 1, &doc);
        let new = temp.path().join("doc.md.tmp");
        fs::write(&new, "replaced").unwrap();
        fs::rename(&new, &doc).unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "replaced");
    }

    #[test]
    fn a_deletion_is_reported_and_a_recreation_after_it_is_a_change() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "one").unwrap();
        let (watch, rx) = service(RealFs, Duration::ZERO);
        open(&watch, 1, &doc);
        fs::remove_file(&doc).unwrap();
        step(&watch);
        assert!(matches!(
            rx.recv_timeout(BOUND),
            Ok(Report::Deleted { key: 1 })
        ));
        fs::write(&doc, "back").unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "back");
    }

    #[test]
    fn missing_for_less_than_the_grace_is_one_change() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "one").unwrap();
        let (watch, rx) = service(RealFs, NEVER);
        open(&watch, 1, &doc);
        fs::remove_file(&doc).unwrap();
        step(&watch);
        nothing(&rx);
        fs::write(&doc, "again").unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "again");
        nothing(&rx);
    }

    #[test]
    fn a_burst_of_writes_is_one_change() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "one").unwrap();
        let (watch, rx) = service(RealFs, GRACE);
        open(&watch, 1, &doc);
        for i in 0..5 {
            fs::write(&doc, format!("burst {i}")).unwrap();
        }
        step(&watch);
        assert_eq!(changed(&rx), "burst 4");
        step(&watch);
        nothing(&rx);
    }

    #[test]
    fn pictures_present_or_not_are_watched_and_never_delete_the_document() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "pic:later.png").unwrap();
        let (watch, rx) = service(RealFs, Duration::ZERO);
        open(&watch, 1, &doc);
        let picture = temp.path().join("later.png");
        fs::write(&picture, b"png").unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "pic:later.png");
        fs::remove_file(&picture).unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "pic:later.png");
        fs::write(&picture, b"png again").unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "pic:later.png");
    }

    #[test]
    fn a_picture_dropped_by_a_new_render_is_no_longer_watched() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        let picture = temp.path().join("a.png");
        fs::write(&picture, b"png").unwrap();
        fs::write(&doc, "pic:a.png").unwrap();
        let (watch, rx) = service(RealFs, GRACE);
        open(&watch, 1, &doc);
        fs::write(&doc, "no pictures now").unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "no pictures now");
        fs::write(&picture, b"changed").unwrap();
        step(&watch);
        nothing(&rx);
        // And one a new render adds is watched from its state then.
        fs::write(&doc, "pic:b.png").unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "pic:b.png");
        fs::write(temp.path().join("b.png"), b"png").unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "pic:b.png");
    }

    #[test]
    fn a_rescan_acts_on_the_document_again() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "same").unwrap();
        let (watch, rx) = service(RealFs, GRACE);
        open(&watch, 1, &doc);
        watch.rescan(1);
        step(&watch);
        assert_eq!(changed(&rx), "same");
    }

    /// The real filesystem, but a read waits at the gate while it's armed (and closed).
    struct HeldReads {
        armed: Arc<Mutex<bool>>,
        gate: Arc<Gate>,
    }

    impl Fs for HeldReads {
        fn snapshot(&self, path: &Path) -> io::Result<Snapshot> {
            RealFs.snapshot(path)
        }
        fn read(&self, path: &Path) -> io::Result<(Vec<u8>, Stamp)> {
            if *lock(&self.armed) {
                *lock(&self.gate.entered) = true;
                self.gate.cv.notify_all();
                let end = Instant::now() + BOUND;
                let mut open = lock(&self.gate.open);
                while !*open && Instant::now() < end {
                    open = self
                        .gate
                        .cv
                        .wait_timeout(open, BOUND)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            }
            RealFs.read(path)
        }
        fn holds(&self, root: &DocRoot, path: &Path) -> bool {
            RealFs.holds(root, path)
        }
    }

    #[test]
    fn a_read_from_before_the_window_registered_again_is_never_reported() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "before").unwrap();
        let armed = Arc::new(Mutex::new(false));
        let gate = Arc::new(Gate {
            open: Mutex::new(false),
            entered: Mutex::new(false),
            cv: Condvar::new(),
        });
        let fs_ = HeldReads {
            armed: Arc::clone(&armed),
            gate: Arc::clone(&gate),
        };
        let (watch, rx) = service(fs_, GRACE);
        open(&watch, 1, &doc);
        // A change, read by a cycle that's held in its read, under the window's first epoch.
        fs::write(&doc, "changed outside").unwrap();
        *lock(&armed) = true;
        until("the read held", || {
            watch.wake();
            *lock(&gate.entered)
        });
        // Meanwhile the window registers again (its own save): a new epoch.
        let before = watch.document_snapshot(&doc);
        let epoch = watch.register(
            1,
            Watched {
                document: doc.clone(),
                root: DocRoot::for_document(&doc).ok(),
                pictures: Vec::new(),
            },
            before,
        );
        assert_eq!(epoch, 2);
        *lock(&armed) = false;
        *lock(&gate.open) = true;
        gate.cv.notify_all();
        // What that cycle read is out of date: it's never reported, and what the window
        // registered stands.
        step(&watch);
        nothing(&rx);
        // A later outside write is reported, under the new epoch.
        fs::write(&doc, "changed again").unwrap();
        step(&watch);
        match rx.recv_timeout(BOUND).expect("a report in time") {
            Report::Changed {
                epoch, prepared, ..
            } => assert_eq!((epoch, prepared.as_str()), (2, "changed again")),
            other => panic!("not a change: {other:?}"),
        }
    }

    /// A filesystem whose reads fail until `fail` is cleared.
    struct FailingReads {
        fail: Arc<Mutex<bool>>,
    }

    impl Fs for FailingReads {
        fn snapshot(&self, path: &Path) -> io::Result<Snapshot> {
            RealFs.snapshot(path)
        }
        fn read(&self, path: &Path) -> io::Result<(Vec<u8>, Stamp)> {
            if *lock(&self.fail) {
                return Err(io::Error::other("half written"));
            }
            RealFs.read(path)
        }
        fn holds(&self, root: &DocRoot, path: &Path) -> bool {
            RealFs.holds(root, path)
        }
    }

    /// The real filesystem, but the service's first cycle waits at `entries_copied` until the
    /// gate opens.
    struct FirstCycleHeld {
        gate: Arc<Gate>,
        held: Mutex<bool>,
    }

    impl Fs for FirstCycleHeld {
        fn snapshot(&self, path: &Path) -> io::Result<Snapshot> {
            RealFs.snapshot(path)
        }
        fn read(&self, path: &Path) -> io::Result<(Vec<u8>, Stamp)> {
            RealFs.read(path)
        }
        fn holds(&self, root: &DocRoot, path: &Path) -> bool {
            RealFs.holds(root, path)
        }
        fn entries_copied(&self) {
            if std::mem::replace(&mut *lock(&self.held), true) {
                return;
            }
            *lock(&self.gate.entered) = true;
            self.gate.cv.notify_all();
            let end = Instant::now() + BOUND;
            let mut open = lock(&self.gate.open);
            while !*open && Instant::now() < end {
                open = self
                    .gate
                    .cv
                    .wait_timeout(open, BOUND)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        }
    }

    /// A window registered and rescanned while a cycle that started before it is under way:
    /// the rescan isn't lost to that cycle, which never saw the window, but acts on the next.
    #[test]
    fn a_rescan_made_during_a_cycle_that_missed_its_window_is_kept() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "same").unwrap();
        let gate = Arc::new(Gate {
            open: Mutex::new(false),
            entered: Mutex::new(false),
            cv: Condvar::new(),
        });
        let release = OpenOnDrop(Arc::clone(&gate));
        let (watch, rx) = service(
            FirstCycleHeld {
                gate: Arc::clone(&gate),
                held: Mutex::new(false),
            },
            GRACE,
        );
        // The service's first cycle has copied what's registered (nothing yet), and waits there,
        // before taking the rescans.
        until("the first cycle to be held", || *lock(&gate.entered));
        // Whether the held cycle still holds what's registered (nothing else does yet).
        let cycle_holds_entries = watch.inner.entries.try_lock().is_err();
        // A window registered and rescanned now, from another thread, as a window opens.
        let registered = Arc::new(Mutex::new(false));
        let opener = {
            let (watch, doc, registered) = (watch.clone(), doc.clone(), Arc::clone(&registered));
            thread::spawn(move || {
                open(&watch, 1, &doc);
                watch.rescan(1);
                *lock(&registered) = true;
            })
        };
        // If the cycle holds what's registered, registering waits for it, and the rescan can
        // only come after its take: let it go on now. Otherwise the window is registered and
        // rescanned while the cycle waits, before its take: let it go on once that's done.
        until("the window to be registered, or held off", || {
            cycle_holds_entries || *lock(&registered)
        });
        drop(release);
        opener.join().unwrap();
        step(&watch);
        assert_eq!(changed(&rx), "same");
    }

    #[test]
    fn a_failed_read_is_tried_again() {
        let temp = tempfile::tempdir().unwrap();
        let doc = temp.path().join("doc.md");
        fs::write(&doc, "one").unwrap();
        let fail = Arc::new(Mutex::new(true));
        let (watch, rx) = service(
            FailingReads {
                fail: Arc::clone(&fail),
            },
            GRACE,
        );
        open(&watch, 1, &doc);
        fs::write(&doc, "two, a longer text").unwrap();
        step(&watch);
        assert!(matches!(
            rx.recv_timeout(BOUND),
            Ok(Report::ReadFailed { key: 1, .. })
        ));
        *lock(&fail) = false;
        step(&watch);
        // Each retry that failed meanwhile said so again; then the change comes through.
        loop {
            match rx.recv_timeout(BOUND).expect("a report in time") {
                Report::ReadFailed { key: 1, .. } => {}
                Report::Changed { prepared, .. } => {
                    break assert_eq!(prepared, "two, a longer text");
                }
                other => panic!("{other:?}"),
            }
        }
    }

    /// A gate the service's snapshots of one path wait at.
    struct Gate {
        open: Mutex<bool>,
        entered: Mutex<bool>,
        cv: Condvar,
    }

    struct GatedFs {
        blocked: PathBuf,
        gate: Arc<Gate>,
    }

    impl Fs for GatedFs {
        fn snapshot(&self, path: &Path) -> io::Result<Snapshot> {
            if path == self.blocked {
                *lock(&self.gate.entered) = true;
                self.gate.cv.notify_all();
                let end = Instant::now() + BOUND;
                let mut open = lock(&self.gate.open);
                while !*open && Instant::now() < end {
                    open = self
                        .gate
                        .cv
                        .wait_timeout(open, BOUND)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            }
            RealFs.snapshot(path)
        }
        fn read(&self, path: &Path) -> io::Result<(Vec<u8>, Stamp)> {
            RealFs.read(path)
        }
        fn holds(&self, root: &DocRoot, path: &Path) -> bool {
            RealFs.holds(root, path)
        }
    }

    /// Opens the gate when dropped, so a failing test never leaves the service stuck.
    struct OpenOnDrop(Arc<Gate>);

    impl Drop for OpenOnDrop {
        fn drop(&mut self) {
            *lock(&self.0.open) = true;
            self.0.cv.notify_all();
        }
    }

    #[test]
    fn a_stuck_share_never_blocks_closing_or_adds_threads() {
        let temp = tempfile::tempdir().unwrap();
        let stuck = temp.path().join("stuck.md");
        let live = temp.path().join("live.md");
        fs::write(&stuck, "stuck").unwrap();
        fs::write(&live, "live").unwrap();
        let gate = Arc::new(Gate {
            open: Mutex::new(false),
            entered: Mutex::new(false),
            cv: Condvar::new(),
        });
        let release = OpenOnDrop(Arc::clone(&gate));
        let (watch, rx) = service(
            GatedFs {
                blocked: stuck.clone(),
                gate: Arc::clone(&gate),
            },
            GRACE,
        );
        // Registered as if opened before the share stopped answering, then stuck.
        watch.register(
            1,
            Watched {
                document: stuck.clone(),
                root: None,
                pictures: Vec::new(),
            },
            None,
        );
        watch.wake();
        until("the service to be stuck in the call", || {
            *lock(&gate.entered)
        });
        // Closing it, and opening and closing many more, returns at once on one thread.
        let started = Instant::now();
        watch.unregister(1);
        for key in 2..52 {
            open(&watch, key, &live);
            watch.unregister(key);
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "closing waited on the stuck call"
        );
        assert_eq!(watch.threads_started(), 1);
        // A window still open; then the share answers: it carries on, and nothing comes for
        // the closed ones.
        open(&watch, 60, &live);
        drop(release);
        fs::write(&live, "live, changed").unwrap();
        step(&watch);
        match rx.recv_timeout(BOUND).expect("a report in time") {
            Report::Changed {
                key: 60, prepared, ..
            } => assert_eq!(prepared, "live, changed"),
            other => panic!("{other:?}"),
        }
        nothing(&rx);
    }
}
