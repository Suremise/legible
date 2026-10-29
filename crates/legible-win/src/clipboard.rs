//! Copy document and Copy selection: the page's HTML, made fit for Outlook, Word and OneNote by
//! `legible_core::clipboard_html`, put on the clipboard as "HTML Format" (CF_HTML) and plain
//! text.
//!
//! - **A request** is the page's HTML with the window's document folder ([`DocRoot`]) as it was
//!   when the shell accepted it, and a number in one order for the whole process ([`Order`]):
//!   the clipboard is everyone's, so a copy made later in any window wins over one made
//!   earlier, and an earlier one that finishes later is never written.
//! - **Building** happens on one thread ([`Copier`]) with a slot for one waiting request: a
//!   newer one replaces it, and the replaced one is answered as superseded. A document's
//!   pictures are read there, confined to the accepted folder as the window serves them
//!   ([`pictures`]); pictures from the internet are never fetched (they become their alt text).
//! - **Writing** ([`write`]) happens on the UI thread, owned by the window that asked, through
//!   [`ClipboardPort`] (Windows' clipboard in `win32::clipboard`, a recording fake in tests):
//!   both formats are prepared before the clipboard is emptied, and an empty copy never
//!   empties it.

use std::io;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use legible_core::clipboard_html::{self, Limits};
use legible_core::render::DOC_BASE;

use crate::confine::{self, DocRoot};
use crate::reason;

/// A copy as its page names it: its number, and the render (generation) it was made from.
/// A page served again starts at a newer generation, so an older page's copy never answers
/// for a newer page's, whatever their numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket {
    pub id: u64,
    pub generation: u64,
}

/// A copy the shell accepted.
#[derive(Debug, Clone)]
pub struct Request<W> {
    /// Its place in the process's order ([`Order`]).
    pub seq: u64,
    pub window: W,
    /// The page's name for it, for its answer.
    pub page: Ticket,
    /// The page's HTML: the document, or a selection of it.
    pub html: String,
    /// The window's document folder when the copy was accepted (None: no document).
    pub root: Option<DocRoot>,
}

/// What a request became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Built<W> {
    /// Ready to write: CF_HTML's bytes, and the text.
    Ready {
        seq: u64,
        window: W,
        page: Ticket,
        html: Vec<u8>,
        text: String,
    },
    /// Replaced by a newer copy before it was built.
    Superseded { window: W, page: Ticket },
}

/// The process's order of copies: each accepted copy gets the next number, and only the newest
/// may be written.
#[derive(Debug, Default)]
pub struct Order {
    newest: u64,
}

impl Order {
    /// A copy accepted now.
    pub fn accept(&mut self) -> u64 {
        self.newest += 1;
        self.newest
    }

    /// Whether copy `seq` is still the newest (none accepted after it, in any window).
    pub fn is_newest(&self, seq: u64) -> bool {
        seq == self.newest
    }
}

/// Reads a document picture for the builder: an `src` on the document origin, through `root`
/// as the window serves it (the generation taken off once, then the same checks and canonical
/// confinement), at most `limits.per_image` bytes. Anything else, a picture from the internet
/// included, is `None` (the builder writes its alt text). `data:` pictures are the builder's.
pub fn pictures(root: Option<DocRoot>, limits: Limits) -> impl Fn(&str) -> Option<Vec<u8>> {
    move |src| {
        let root = root.as_ref()?;
        let path = src.strip_prefix(DOC_BASE.trim_end_matches('/'))?;
        let path = path.split(['?', '#']).next().unwrap_or(path);
        let (file, _) = root.resolve(confine::without_generation(path)).ok()?;
        let size = std::fs::metadata(&file).ok()?.len();
        if size > limits.per_image as u64 {
            return None;
        }
        std::fs::read(&file).ok()
    }
}

/// Builds a request: the builder's HTML framed as CF_HTML, and its text with Windows' line ends.
pub fn build<W>(request: Request<W>) -> Built<W> {
    let limits = Limits::default();
    let read = pictures(request.root, limits);
    let copied = clipboard_html::build(&request.html, &read, &limits);
    Built::Ready {
        seq: request.seq,
        window: request.window,
        page: request.page,
        html: if copied.html.trim().is_empty() {
            Vec::new()
        } else {
            clipboard_html::cf_html(&copied.html)
        },
        text: copied.text.replace("\r\n", "\n").replace('\n', "\r\n"),
    }
}

/// The thread copies are built on, with a slot for one waiting request. It ends when this is
/// dropped.
pub struct Copier<W> {
    inner: Arc<(Mutex<Slot<W>>, Condvar)>,
}

struct Slot<W> {
    waiting: Option<Request<W>>,
    closed: bool,
}

impl<W: Send + 'static> Copier<W> {
    /// Starts the thread; `done` hears of each request, built or superseded.
    pub fn new(done: impl Fn(Built<W>) + Send + 'static) -> Copier<W> {
        let inner = Arc::new((
            Mutex::new(Slot {
                waiting: None,
                closed: false,
            }),
            Condvar::new(),
        ));
        let worker = inner.clone();
        let started = thread::Builder::new().name("copy".into()).spawn(move || {
            loop {
                let request = {
                    let (slot, ready) = &*worker;
                    let mut slot = slot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    loop {
                        if let Some(request) = slot.waiting.take() {
                            break request;
                        }
                        if slot.closed {
                            return;
                        }
                        slot = ready
                            .wait(slot)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                };
                done(build(request));
            }
        });
        if let Err(error) = started {
            tracing::warn!(%error, "couldn't start the copying thread");
        }
        Copier { inner }
    }

    /// Hands a request to the thread. One still waiting is replaced, and returned: answer it as
    /// superseded.
    pub fn request(&self, request: Request<W>) -> Option<Request<W>> {
        let (slot, ready) = &*self.inner;
        let replaced = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .waiting
            .replace(request);
        ready.notify_one();
        replaced
    }
}

impl<W> Drop for Copier<W> {
    fn drop(&mut self) {
        let (slot, ready) = &*self.inner;
        slot.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed = true;
        ready.notify_all();
    }
}

/// The clipboard, as [`write`] uses it: Windows' (`win32::clipboard`), or a test's.
pub trait ClipboardPort {
    /// A block of memory for one format, until the clipboard takes it.
    type Data;
    /// "HTML Format"'s number.
    fn html_format(&mut self) -> io::Result<u32>;
    fn alloc(&mut self, bytes: &[u8]) -> io::Result<Self::Data>;
    /// Frees a block the clipboard didn't take.
    fn free(&mut self, data: Self::Data);
    /// Opens the clipboard for the window; `Ok(false)` while another app holds it.
    fn open(&mut self) -> io::Result<bool>;
    fn empty(&mut self) -> io::Result<()>;
    /// Gives `data` to the clipboard as `format`; on an error it's handed back.
    fn set(&mut self, format: u32, data: Self::Data) -> Result<(), (Self::Data, io::Error)>;
    fn close(&mut self);
}

/// CF_UNICODETEXT.
pub const UNICODE_TEXT: u32 = 13;

/// What writing a copy came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// HTML and text.
    Both,
    /// Only the text: the HTML couldn't be set.
    TextOnly,
    /// Another app holds the clipboard: try again shortly. Nothing was changed.
    Busy,
    /// Nothing was written, or the clipboard was emptied and nothing could be set.
    Failed(String),
}

/// A copy that failed, said for the reader ([`reason::of`]); the log keeps the error.
fn failed(error: &io::Error) -> Written {
    tracing::warn!(%error, "couldn't write to the clipboard");
    Written::Failed(reason::of(error))
}

/// Writes CF_HTML's `html` (may be empty: text only) and `text` (CRLF) to the clipboard.
/// Everything is prepared before the clipboard is emptied; a copy with nothing in it never
/// empties it; blocks the clipboard doesn't take are freed; it's closed whatever happens.
pub fn write<P: ClipboardPort>(port: &mut P, html: &[u8], text: &str) -> Written {
    if html.is_empty() && text.is_empty() {
        return failed(&io::Error::other("there's nothing to copy"));
    }
    let format = if html.is_empty() {
        None
    } else {
        match port.html_format() {
            Ok(format) => Some(format),
            Err(error) => return failed(&error),
        }
    };
    let mut html_data = None;
    if let Some(format) = format {
        match port.alloc(&nul_terminated(html)) {
            Ok(data) => html_data = Some((format, data)),
            Err(error) => return failed(&error),
        }
    }
    let text_data = match port.alloc(&utf16_nul(text)) {
        Ok(data) => data,
        Err(error) => {
            if let Some((_, data)) = html_data {
                port.free(data);
            }
            return failed(&error);
        }
    };
    let opened = port.open();
    if !matches!(opened, Ok(true)) {
        if let Some((_, data)) = html_data {
            port.free(data);
        }
        port.free(text_data);
        return match opened {
            Ok(_) => Written::Busy,
            Err(error) => failed(&error),
        };
    }
    let written = (|| {
        if let Err(error) = port.empty() {
            if let Some((_, data)) = html_data {
                port.free(data);
            }
            port.free(text_data);
            return failed(&error);
        }
        let html_set = match html_data {
            Some((format, data)) => match port.set(format, data) {
                Ok(()) => true,
                Err((data, error)) => {
                    tracing::warn!(%error, "couldn't put the HTML on the clipboard");
                    port.free(data);
                    false
                }
            },
            None => false,
        };
        match port.set(UNICODE_TEXT, text_data) {
            Ok(()) if html_set => Written::Both,
            Ok(()) => Written::TextOnly,
            Err((data, error)) => {
                port.free(data);
                failed(&error)
            }
        }
    })();
    port.close();
    written
}

fn nul_terminated(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out.push(0);
    out
}

fn utf16_nul(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use super::*;

    /// A clipboard that records what's done to it, and fails where told to.
    #[derive(Default)]
    struct Fake {
        fail: Vec<&'static str>,
        busy: bool,
        next: u32,
        log: Vec<String>,
        freed: Vec<u32>,
        taken: Vec<(u32, Vec<u8>)>,
        blocks: Vec<(u32, Vec<u8>)>,
        /// The allocation (counting from 1) that fails.
        alloc_fails_at: Option<u32>,
    }

    impl Fake {
        fn failing(steps: &[&'static str]) -> Fake {
            Fake {
                fail: steps.to_vec(),
                ..Fake::default()
            }
        }
        fn step(&mut self, name: &str) -> io::Result<()> {
            self.log.push(name.to_owned());
            if self.fail.contains(&name) {
                Err(io::Error::other(name.to_owned()))
            } else {
                Ok(())
            }
        }
    }

    impl ClipboardPort for Fake {
        type Data = u32;
        fn html_format(&mut self) -> io::Result<u32> {
            self.step("register").map(|()| 49_000)
        }
        fn alloc(&mut self, bytes: &[u8]) -> io::Result<u32> {
            self.step("alloc")?;
            if self.alloc_fails_at == Some(self.next + 1) {
                return Err(io::Error::other("out of memory"));
            }
            self.next += 1;
            self.blocks.push((self.next, bytes.to_vec()));
            Ok(self.next)
        }
        fn free(&mut self, data: u32) {
            self.freed.push(data);
        }
        fn open(&mut self) -> io::Result<bool> {
            self.step("open")?;
            Ok(!self.busy)
        }
        fn empty(&mut self) -> io::Result<()> {
            self.step("empty")
        }
        fn set(&mut self, format: u32, data: u32) -> Result<(), (u32, io::Error)> {
            let name = if format == UNICODE_TEXT {
                "set text"
            } else {
                "set html"
            };
            match self.step(name) {
                Ok(()) => {
                    let bytes = self
                        .blocks
                        .iter()
                        .find(|(id, _)| *id == data)
                        .unwrap()
                        .1
                        .clone();
                    self.taken.push((format, bytes));
                    Ok(())
                }
                Err(error) => Err((data, error)),
            }
        }
        fn close(&mut self) {
            self.log.push("close".into());
        }
    }

    #[test]
    fn both_formats_are_prepared_before_the_clipboard_is_emptied() {
        let mut fake = Fake::default();
        let html = cf_html_for("<p>Ünïcödé — 日本語</p>");
        assert_eq!(
            write(&mut fake, &html, "Ünïcödé — 日本語\r\n"),
            Written::Both
        );
        assert_eq!(
            fake.log,
            [
                "register", "alloc", "alloc", "open", "empty", "set html", "set text", "close"
            ]
        );
        assert!(fake.freed.is_empty(), "the clipboard took both");
        let (format, stored) = &fake.taken[0];
        assert_eq!(*format, 49_000);
        assert_eq!(stored.last(), Some(&0), "CF_HTML is NUL-terminated");
        assert_eq!(&stored[..stored.len() - 1], &html[..]);
        let (format, stored) = &fake.taken[1];
        assert_eq!(*format, UNICODE_TEXT);
        assert_eq!(stored, &utf16_nul("Ünïcödé — 日本語\r\n"));
    }

    #[test]
    fn a_copy_with_nothing_in_it_never_empties_the_clipboard() {
        let mut fake = Fake::default();
        assert!(matches!(write(&mut fake, b"", ""), Written::Failed(_)));
        assert!(fake.log.is_empty());
    }

    #[test]
    fn whatever_fails_frees_what_the_clipboard_didnt_take_and_closes_it() {
        let html = cf_html_for("<p>x</p>");
        // Before the clipboard is opened: nothing touched, nothing emptied.
        for (step, freed) in [("register", 0), ("alloc", 0)] {
            let mut fake = Fake::failing(&[step]);
            assert!(
                matches!(write(&mut fake, &html, "x"), Written::Failed(_)),
                "{step}"
            );
            assert!(!fake.log.contains(&"empty".to_owned()), "{step}");
            assert_eq!(fake.freed.len(), freed, "{step}");
        }
        // The text's block failing frees the HTML's.
        let mut fake = Fake {
            alloc_fails_at: Some(2),
            ..Fake::default()
        };
        assert!(matches!(write(&mut fake, &html, "x"), Written::Failed(_)));
        assert_eq!(fake.freed, [1]);
        assert!(!fake.log.contains(&"open".to_owned()));
        // Busy, or couldn't open: both blocks freed, nothing emptied, Busy is said as such.
        let mut fake = Fake {
            busy: true,
            ..Fake::default()
        };
        assert_eq!(write(&mut fake, &html, "x"), Written::Busy);
        assert_eq!(fake.freed, [1, 2]);
        assert!(!fake.log.contains(&"empty".to_owned()) && !fake.log.contains(&"close".to_owned()));
        let mut fake = Fake::failing(&["open"]);
        assert!(matches!(write(&mut fake, &html, "x"), Written::Failed(_)));
        assert_eq!(fake.freed, [1, 2]);
        // Emptying fails: both freed, and closed.
        let mut fake = Fake::failing(&["empty"]);
        assert!(matches!(write(&mut fake, &html, "x"), Written::Failed(_)));
        assert_eq!(fake.freed, [1, 2]);
        assert_eq!(fake.log.last().map(String::as_str), Some("close"));
        // The HTML refused: text only, the HTML's block freed.
        let mut fake = Fake::failing(&["set html"]);
        assert_eq!(write(&mut fake, &html, "x"), Written::TextOnly);
        assert_eq!(fake.freed, [1]);
        assert_eq!(fake.taken.len(), 1);
        assert_eq!(fake.log.last().map(String::as_str), Some("close"));
        // The text refused (after the HTML went in): a failure, the text's block freed.
        let mut fake = Fake::failing(&["set text"]);
        assert!(matches!(write(&mut fake, &html, "x"), Written::Failed(_)));
        assert_eq!(fake.freed, [2]);
        assert_eq!(fake.log.last().map(String::as_str), Some("close"));
    }

    #[test]
    fn text_alone_needs_no_html_format() {
        let mut fake = Fake::default();
        assert_eq!(write(&mut fake, b"", "just text"), Written::TextOnly);
        assert_eq!(fake.log, ["alloc", "open", "empty", "set text", "close"]);
    }

    fn cf_html_for(fragment: &str) -> Vec<u8> {
        clipboard_html::cf_html(fragment)
    }

    #[test]
    fn the_newest_copy_in_the_process_wins() {
        let mut order = Order::default();
        let a = order.accept();
        let b = order.accept();
        assert!(!order.is_newest(a) && order.is_newest(b));
    }

    /// A document's folder: a picture in it, one beside it outside, and a big one.
    fn folder() -> (tempfile::TempDir, DocRoot) {
        let temp = tempfile::tempdir().unwrap();
        let docs = temp.path().join("docs");
        fs::create_dir_all(docs.join("img")).unwrap();
        fs::write(docs.join("img").join("a.png"), b"\x89PNG in the folder").unwrap();
        fs::write(temp.path().join("secret.png"), b"\x89PNG outside").unwrap();
        fs::write(docs.join("big.png"), vec![0u8; 64]).unwrap();
        let root = DocRoot::new(&docs).unwrap();
        (temp, root)
    }

    #[test]
    fn pictures_are_read_only_from_the_accepted_folder_as_the_window_serves_them() {
        let (_temp, root) = folder();
        let read = pictures(Some(root.clone()), Limits::default());
        assert_eq!(
            read("https://legible-doc.localhost/img/a.png").as_deref(),
            Some(&b"\x89PNG in the folder"[..])
        );
        assert_eq!(
            read("https://legible-doc.localhost/~3/img/a.png").as_deref(),
            Some(&b"\x89PNG in the folder"[..])
        );
        assert_eq!(
            read("https://legible-doc.localhost/img/a.png?x#y").as_deref(),
            Some(&b"\x89PNG in the folder"[..])
        );
        for refused in [
            "https://legible-doc.localhost/../secret.png",
            "https://legible-doc.localhost/%2e%2e/secret.png",
            "https://legible-doc.localhost/img/missing.png",
            "https://legible-doc.localhost/~3/~4/img/a.png",
            "https://example.com/img/a.png",
            "https://legible-doc.localhost.example.com/img/a.png",
            "file:///C:/Windows/win.ini",
            "img/a.png",
        ] {
            assert_eq!(read(refused), None, "{refused}");
        }
        let small = pictures(
            Some(root),
            Limits {
                per_image: 10,
                total: 100,
            },
        );
        assert_eq!(
            small("https://legible-doc.localhost/big.png"),
            None,
            "over the limit"
        );
        assert_eq!(
            pictures(None, Limits::default())("https://legible-doc.localhost/img/a.png"),
            None
        );
    }

    #[test]
    fn a_copy_is_built_with_its_pictures_embedded_and_internet_ones_as_alt_text() {
        let (_temp, root) = folder();
        let png = include_bytes!("../../../assets/icon/legible-64.png");
        fs::write(root.path().join("img").join("real.png"), png).unwrap();
        let html = concat!(
            "<h1>Plan — Ünïcödé</h1>",
            "<p><img src=\"https://legible-doc.localhost/~2/img/real.png\" alt=\"Local\"></p>",
            "<p><img src=\"https://legible-doc.localhost/../secret.png\" alt=\"Outside\"></p>",
            "<p><img src=\"https://example.com/remote.png\" alt=\"Remote\"></p>",
            "<table><tr><th>A</th></tr><tr><td>1</td></tr></table>",
        );
        let built = build(Request {
            seq: 1,
            window: (),
            page: Ticket {
                id: 7,
                generation: 3,
            },
            html: html.into(),
            root: Some(root),
        });
        let Built::Ready {
            html, text, page, ..
        } = built
        else {
            panic!("not built");
        };
        assert_eq!(
            page,
            Ticket {
                id: 7,
                generation: 3
            }
        );
        let html = String::from_utf8(html).unwrap();
        assert!(html.starts_with("Version:0.9\r\n"));
        assert!(
            html.contains("data:image/png;base64,"),
            "the local picture is embedded"
        );
        assert!(
            html.contains("Outside") && html.contains("Remote"),
            "the others are their alt text"
        );
        assert!(
            !html.contains("example.com") && !html.contains("secret"),
            "nothing else of them"
        );
        assert!(text.contains("Plan — Ünïcödé\r\n"), "{text:?}");
        assert!(
            !text.replace("\r\n", "").contains('\n'),
            "Windows line ends only"
        );
    }

    #[test]
    fn a_waiting_copy_is_replaced_by_a_newer_one_and_every_request_is_answered() {
        let (tx, rx) = std::sync::mpsc::channel();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let copier = {
            let gate = gate.clone();
            Copier::new(move |built: Built<u32>| {
                // The first build waits at the gate, so the next requests queue up behind it.
                let (open, opened) = &*gate;
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut is_open = open.lock().unwrap();
                while !*is_open && Instant::now() < deadline {
                    is_open = opened
                        .wait_timeout(is_open, Duration::from_millis(20))
                        .unwrap()
                        .0;
                }
                drop(is_open);
                tx.send(built).unwrap();
            })
        };
        let request = |seq: u64| Request {
            seq,
            window: seq as u32,
            page: Ticket {
                id: seq,
                generation: 0,
            },
            html: format!("<p>{seq}</p>"),
            root: None,
        };
        assert!(copier.request(request(1)).is_none());
        // Wait until the first is taken (the slot is empty again), so the next two wait.
        let deadline = Instant::now() + Duration::from_secs(10);
        while copier.inner.0.lock().unwrap().waiting.is_some() {
            assert!(Instant::now() < deadline, "the first copy wasn't taken");
            std::thread::yield_now();
        }
        assert!(copier.request(request(2)).is_none());
        let replaced = copier.request(request(3)).expect("2 was waiting");
        assert_eq!(replaced.seq, 2);
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        let mut seen: Vec<u64> = Vec::new();
        for _ in 0..2 {
            match rx.recv_timeout(Duration::from_secs(10)).expect("an answer") {
                Built::Ready { seq, .. } => seen.push(seq),
                Built::Superseded { .. } => panic!("superseded ones are answered by the caller"),
            }
        }
        assert_eq!(seen, [1, 3]);
    }
}
