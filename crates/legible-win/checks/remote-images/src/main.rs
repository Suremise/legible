//! The native check of pictures from the internet: Legible's page (`page::document_page`, its
//! generated CSP and the chrome's real script), its request filter (`request_filter`) and its
//! consent (`consent::Pictures`), in a real WebView2, against servers on 127.0.0.1 that log
//! every connection and request:
//!
//! - A: HTTPS, the pictures' origin, with redirects to the same origin, another host
//!   (`localhost`), another port (B) and plain HTTP (H), and a picture that waits at a gate;
//! - B: HTTPS on another port; H: plain HTTP.
//!
//! The certificate is made for this run, and this check's WebView2 accepts that one alone (by
//! its PEM): nothing of the kind is in Legible. The host mirrors the shell's glue (serving the
//! page again on a change of CSP, telling it otherwise, answers checked against the newest
//! generation). The steps run from a thread of their own, each waiting on conditions with a
//! bound, never on time; only script in this window's own WebView drives the page. Exits 1 if
//! any check fails, printing each server's log.
//!
//!   cargo run --manifest-path crates/legible-win/checks/remote-images/Cargo.toml

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use legible_core::assets;
use legible_core::render::render;
use legible_win::consent::{Next, Pictures, Unavailable};
use legible_win::page;
use legible_win::reload::FileBar;
use legible_win::settings::{RemoteImages, Settings, json_string};
use legible_win::win32::request_filter;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW,
    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL, ICoreWebView2_14,
};
use webview2_com::{ServerCertificateErrorDetectedEventHandler, take_pwstr};
use windows::core::{Interface, PWSTR};
use wry::http::{Response, header};
use wry::{WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 15, 4, 0, 9, 251, 3,
    253, 227, 85, 242, 156, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
/// How long any one condition may take.
const BOUND: Duration = Duration::from_secs(20);

// --- The servers -------------------------------------------------------------------------------

/// Everything the servers saw, in order: "<server> <host> <path>", or "<server> connection".
#[derive(Default)]
struct Log {
    lines: Mutex<Vec<String>>,
    /// The gate /slow.png waits at, and whether it's open.
    gate: Mutex<bool>,
    opened: Condvar,
}

impl Log {
    fn push(&self, line: String) {
        self.lines.lock().unwrap().push(line);
    }
    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
    fn open_gate(&self) {
        *self.gate.lock().unwrap() = true;
        self.opened.notify_all();
    }
    /// Waits at the gate, for [`BOUND`] at most.
    fn wait_at_gate(&self) {
        let deadline = Instant::now() + BOUND;
        let mut open = self.gate.lock().unwrap();
        while !*open && Instant::now() < deadline {
            open = self
                .opened
                .wait_timeout(open, Duration::from_millis(50))
                .unwrap()
                .0;
        }
    }
}

/// Opens the gate when dropped, so a failing step never leaves a server waiting.
struct OpenOnDrop(Arc<Log>);
impl Drop for OpenOnDrop {
    fn drop(&mut self) {
        self.0.open_gate();
    }
}

struct Ports {
    a: u16,
    b: u16,
    h: u16,
}

/// One request on `stream`: logged, then answered by `name`'s routes.
fn answer(mut stream: impl Read + Write, name: &str, log: &Log, ports: &Ports) {
    let mut reader = BufReader::new(&mut stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut host = String::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
            break;
        }
        if h.to_ascii_lowercase().starts_with("host:") {
            host = h[5..].trim().to_owned();
        }
    }
    let path = line.split(' ').nth(1).unwrap_or("").to_owned();
    log.push(format!("{name} {host} {path}"));
    let redirect = |to: String| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    };
    let reply = match (name, path.as_str()) {
        ("A", "/to-same.png") => redirect(format!("https://127.0.0.1:{}/same.png", ports.a)),
        ("A", "/to-other-host.png") => redirect(format!("https://localhost:{}/other.png", ports.a)),
        ("A", "/to-other-port.png") => redirect(format!("https://127.0.0.1:{}/port.png", ports.b)),
        ("A", "/to-http.png") => redirect(format!("http://127.0.0.1:{}/http.png", ports.h)),
        _ => {
            if path == "/slow.png" {
                log.wait_at_gate();
                log.push(format!("{name} answered /slow.png"));
            }
            let mut r = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                PNG.len()
            )
            .into_bytes();
            r.extend_from_slice(PNG);
            r
        }
    };
    let _ = stream.write_all(&reply);
    let _ = stream.flush();
}

fn serve(
    name: &'static str,
    listener: TcpListener,
    tls: Option<Arc<rustls::ServerConfig>>,
    log: Arc<Log>,
    ports: Arc<Ports>,
) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            log.push(format!("{name} connection"));
            let (log, ports, tls) = (log.clone(), ports.clone(), tls.clone());
            std::thread::spawn(move || match tls {
                Some(config) => {
                    let Ok(connection) = rustls::ServerConnection::new(config) else {
                        return;
                    };
                    answer(
                        rustls::StreamOwned::new(connection, stream),
                        name,
                        &log,
                        &ports,
                    );
                }
                None => answer(stream, name, &log, &ports),
            });
        }
    });
}

// --- The host: Legible's page, filter and consent in a window ----------------------------------

enum Ev {
    /// Run script in the page, and send back what it gives (as JSON).
    Eval(String, Sender<String>),
    /// A live reload: the document's new render.
    Replace(String),
    Setting(RemoteImages),
    FailRefusals(bool),
    /// The document's file was moved or deleted (as live reload reports it).
    FileGone,
    /// Empties the filter's consented hosts and nothing else, leaving the page's CSP as it is:
    /// so a request the CSP lets through reaches a refusal (for failing small).
    RevokeQuietly,
    Ipc(String),
    RemoteFailed,
    Done,
}

struct Host {
    webview: WebView,
    served: Arc<Mutex<(Vec<u8>, String)>>,
    pictures: Pictures,
    generation: u64,
    document: String,
    setting: RemoteImages,
    /// "Always for this file" (the shell keeps it per file: `consent::AlwaysList`).
    always: bool,
    /// The filter's consented hosts, as `pictures` shares them with it.
    allowed: Rc<RefCell<HashSet<String>>>,
    /// The moved-or-deleted bar, as the shell keeps it.
    file_bar: FileBar,
}

impl Host {
    fn serve_again(&mut self, origins: Vec<String>) {
        self.generation += 1;
        let html = page::document_page(
            "en",
            "Remote images check",
            &page::with_generation(&self.document, self.generation),
            &Settings::default(),
            self.generation,
            &origins,
            true,
        );
        *self.served.lock().unwrap() = (
            html.into_bytes(),
            format!("{}{}", page::csp(&origins), page::CSP_HEADER_EXTRA),
        );
        self.pictures.served(origins);
        self.webview.load_url(page::START_URL).unwrap();
    }

    fn message(&self) -> String {
        self.pictures
            .message(self.generation, self.setting, self.always)
    }

    fn tell(&self) {
        let script = format!("window.LegibleChrome?.remoteImages({})", self.message());
        self.webview.evaluate_script(&script).unwrap();
    }

    fn apply(&mut self) {
        match self.pictures.grant(self.setting, self.always) {
            Next::ServeAgain(origins) => self.serve_again(origins),
            Next::Tell => self.tell(),
        }
    }

    fn replace(&mut self, html: String) {
        self.pictures.render(&html);
        self.document = html;
        self.file_bar.back();
        match self.pictures.grant(self.setting, self.always) {
            Next::ServeAgain(origins) => self.serve_again(origins),
            Next::Tell => {
                self.generation += 1;
                let script = format!(
                    "window.LegibleChrome?.replaceDocument({{ generation: {}, html: {}, remote: {} }})",
                    self.generation,
                    json_string(&page::with_generation(&self.document, self.generation)),
                    self.message()
                );
                self.webview.evaluate_script(&script).unwrap();
            }
        }
    }

    fn ipc(&mut self, message: &str) {
        let mut parts = message.split('\n');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("remote-images"), None, _) => self.tell(),
            (Some("file-status"), None, _) if self.file_bar.shows() => {
                self.webview
                    .evaluate_script("window.LegibleChrome?.fileGone(true)")
                    .unwrap();
            }
            (Some("file-keep"), None, _) => self.file_bar.keep_showing(),
            // As the shell: only for the newest render, and never while unavailable.
            (Some("remote-images"), Some("once"), Some(generation))
                if generation.parse() == Ok(self.generation)
                    && self.pictures.unavailable().is_none() =>
            {
                self.pictures.show_once();
                self.apply();
            }
            (Some("remote-images"), Some("always"), Some(generation))
                if generation.parse() == Ok(self.generation)
                    && self.pictures.unavailable().is_none() =>
            {
                self.always = true;
                self.apply();
            }
            _ => {}
        }
    }
}

fn serve_app(path: &str, served: &(Vec<u8>, String)) -> Response<std::borrow::Cow<'static, [u8]>> {
    if path == "/" {
        return Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header("Content-Security-Policy", served.1.as_str())
            .body(served.0.clone().into())
            .unwrap();
    }
    match assets::get(path) {
        Some(asset) => {
            let mut response = Response::builder().header(header::CONTENT_TYPE, asset.content_type);
            if path == page::MERMAID_FRAME {
                response = response.header("Content-Security-Policy", page::MERMAID_FRAME_CSP);
            }
            response.body(asset.bytes.into()).unwrap()
        }
        None => Response::builder()
            .status(404)
            .body(Vec::new().into())
            .unwrap(),
    }
}

/// A picture whose address has an encoded query: asked for exactly as the document has it.
const QUERY: &str = "/q.png?a=1&b=%20x&c=caf%C3%A9";

/// The document: pictures on A (straight, and redirecting each way), and one on plain HTTP.
fn document(ports: &Ports, extra: &[&str]) -> String {
    let a = |p: &str| format!("https://127.0.0.1:{}{p}", ports.a);
    let mut md = String::from("# Pictures from the internet\n\n## Straight\n\n");
    md += &format!(
        "![a]({}) ![a1]({}) ![q]({})\n\n",
        a("/a.png"),
        a("/a.png?v=1"),
        a(QUERY)
    );
    md += "## Redirecting\n\n";
    for p in [
        "/to-same.png",
        "/to-other-host.png",
        "/to-other-port.png",
        "/to-http.png",
    ] {
        md += &format!("![r]({}) ", a(p));
    }
    md += &format!(
        "\n\n## Plain HTTP\n\n![h](http://127.0.0.1:{}/plain.png)\n\n",
        ports.h
    );
    for e in extra {
        md += &format!("![x]({e})\n\n");
    }
    md += "## The end\n\nEnough text to scroll a little.\n";
    render(&md, std::path::Path::new(".")).html
}

// --- The steps ---------------------------------------------------------------------------------

struct Driver {
    proxy: EventLoopProxy<Ev>,
    log: Arc<Log>,
    ports: Arc<Ports>,
    failures: Vec<String>,
    next_id: u64,
}

impl Driver {
    fn eval(&self, script: &str) -> String {
        let (tx, rx) = mpsc::channel();
        let _ = self.proxy.send_event(Ev::Eval(script.to_owned(), tx));
        rx.recv_timeout(BOUND)
            .unwrap_or_else(|_| "\"timed out\"".into())
    }
    fn send(&self, event: Ev) {
        let _ = self.proxy.send_event(event);
    }
    /// Waits for `script` to give `true`.
    fn until(&mut self, what: &str, script: &str) -> bool {
        let deadline = Instant::now() + BOUND;
        while Instant::now() < deadline {
            if self.eval(script) == "true" {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        self.failures.push(format!("timed out waiting for {what}"));
        false
    }
    /// What the promise `expr` gives, as JSON (WebView2 doesn't wait for promises itself: the
    /// page keeps the answer until it's fetched).
    fn eval_async(&mut self, what: &str, expr: &str) -> String {
        self.next_id += 1;
        let id = self.next_id;
        self.eval(&format!(
            "(window.__checks ??= {{}}, delete window.__checks[{id}], Promise.resolve().then(() => {expr}).then((v) => (window.__checks[{id}] = JSON.stringify(v ?? null)), (e) => (window.__checks[{id}] = JSON.stringify('error: ' + e))), true)"
        ));
        if !self.until(what, &format!("window.__checks?.[{id}] !== undefined")) {
            return String::new();
        }
        serde_free_parse(&serde_free_parse(
            &self.eval(&format!("window.__checks[{id}]")),
        ))
    }
    fn check(&mut self, ok: bool, what: String) {
        println!("  {}  {what}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            self.failures.push(what);
        }
    }
    /// The page at `generation` (or later), prepared, with every picture it has set settled
    /// (loaded or failed: `complete`), so whatever they asked for has been asked.
    fn settled(&mut self, generation: u64) -> bool {
        self.generation_at_least(generation);
        self.until(
            &format!("generation {generation}'s pictures to settle"),
            "[...document.querySelectorAll('#document img[data-remote-src][src]')].every((i) => i.complete)",
        )
    }
    fn lines_since(&self, from: usize) -> Vec<String> {
        self.log.lines()[from..].to_vec()
    }
    /// A server request, as logged: "<server> <host> <path>".
    fn got(&self, from: usize, server: &str, path: &str) -> bool {
        self.lines_since(from)
            .iter()
            .any(|l| l.starts_with(&format!("{server} ")) && l.ends_with(&format!(" {path}")))
    }
    fn page_generation(&self) -> u64 {
        self.eval("Number(document.getElementById('document')?.dataset.generation ?? -1)")
            .parse()
            .unwrap_or(0)
    }
    /// Waits for the newest generation the page knows to be at least `at_least`, and the page
    /// to be ready; returns that generation.
    fn generation_at_least(&mut self, at_least: u64) -> u64 {
        self.until(
            &format!("a page at generation {at_least} or later"),
            &format!(
                "!!window.LegibleChrome && Number(document.getElementById('document')?.dataset.generation ?? -1) >= {at_least}"
            ),
        );
        self.eval_async(
            "the page to be ready",
            "window.LegibleChrome.ready.then(() => true)",
        );
        self.page_generation()
    }
    fn shown(&self) -> Vec<String> {
        let json = self.eval("JSON.stringify([...document.querySelectorAll('#document img[data-remote-src][src]')].map((i) => i.getAttribute('src')))");
        let json: String = serde_free_parse(&json);
        json.trim_matches(|c| c == '[' || c == ']')
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_matches('"').to_owned())
            .collect()
    }
    /// Waits for the document to have the picture `raw` (a replacement having landed).
    fn has_picture(&mut self, raw: &str) -> bool {
        self.until(
            &format!("the document to have {raw}"),
            &format!(
                "[...document.querySelectorAll('#document img[data-remote-src]')].some((i) => i.getAttribute('data-remote-src') === {})",
                json_string(raw)
            ),
        )
    }
    /// The proof that the page asks only for what Rust lists: every `src` the page has set is
    /// one of Rust's sources for this render (`consent::remote_sources`, the exact text), and
    /// every request the servers got since `mark` that a picture started (not a redirect's hop,
    /// not the check's own) is one of those addresses.
    fn page_asks_only_for_the_list(&mut self, mark: usize, html: &str, hops: &[&str]) {
        let sources = legible_win::consent::remote_sources(html);
        let shown = self.shown();
        let unlisted: Vec<&String> = shown
            .iter()
            .filter(|s| !sources.iter().any(|r| &r.raw == *s))
            .collect();
        self.check(
            unlisted.is_empty(),
            format!(
                "every src the page set is in Rust's list ({} set): unlisted {unlisted:?}",
                shown.len()
            ),
        );
        let first: Vec<String> = self
            .lines_since(mark)
            .into_iter()
            .filter(|l| !l.ends_with(" connection") && !l.contains(" answered "))
            .filter(|l| !hops.iter().any(|h| l.ends_with(&format!(" {h}"))))
            .collect();
        let strays: Vec<&String> = first
            .iter()
            .filter(|l| {
                let mut parts = l.splitn(3, ' ');
                let (_, host, path) = (
                    parts.next(),
                    parts.next().unwrap_or(""),
                    parts.next().unwrap_or(""),
                );
                let address = format!("https://{host}{path}");
                !sources.iter().any(|r| r.address == address)
            })
            .collect();
        self.check(
            strays.is_empty(),
            format!(
                "every first request ({}) is a listed address, exactly: strays {strays:?}",
                first.len()
            ),
        );
    }
    /// Whether the moved-or-deleted bar shows.
    fn file_bar(&self) -> bool {
        self.eval("!document.getElementById('file-bar').hidden") == "true"
    }
    fn bar(&self) -> String {
        serde_free_parse(&self.eval(
            "(() => { const b = document.getElementById('pictures-bar'); return b && !b.hidden ? b.textContent : ''; })()",
        ))
    }

    fn run(&mut self) {
        let ports = self.ports.clone();
        let a = |p: &str| format!("https://127.0.0.1:{}{p}", ports.a);

        println!("1. Before any consent");
        self.settled(0);
        if std::env::var_os("CHECK_DEBUG").is_some() {
            println!("   page: {}", self.eval("JSON.stringify({ href: location.href, ready: document.readyState, chrome: !!window.LegibleChrome, html: document.documentElement.outerHTML.slice(0, 400) })"));
        }
        let shown = self.shown();
        self.check(
            shown.is_empty(),
            format!("no picture has an address: {shown:?}"),
        );
        let lines = self.log.lines();
        self.check(
            lines.is_empty(),
            format!("no server heard anything: {lines:?}"),
        );
        self.check(self.bar().contains("Show them?"), "the bar asks".into());
        // The file goes before the reader answers: the page served for the answer must say so.
        self.send(Ev::FileGone);
        self.until(
            "the moved-or-deleted bar",
            "!document.getElementById('file-bar').hidden",
        );

        if std::env::var_os(FAIL_ON_PURPOSE).is_some() {
            self.check(false, "failing on purpose, as asked".into());
            return;
        }

        println!("2. Show once: the page is served again, keeping the place and zoom");
        self.eval("document.dispatchEvent(new KeyboardEvent('keydown', { key: '=', ctrlKey: true, bubbles: true })); true");
        let mark = self.log.lines().len();
        self.eval("[...document.querySelectorAll('#pictures-bar button')].find((b) => b.textContent === 'Show once').click(); true");
        let generation = self.generation_at_least(1);
        self.settled(generation);
        let zoom =
            serde_free_parse(&self.eval("document.querySelector('.zoom-level')?.textContent"));
        self.check(
            zoom == "110%",
            format!("the zoom kept across serving again: {zoom}"),
        );
        for path in [
            "/a.png",
            "/a.png?v=1",
            QUERY,
            "/to-same.png",
            "/to-other-host.png",
            "/to-other-port.png",
            "/to-http.png",
        ] {
            self.check(
                self.got(mark, "A", path),
                format!("A got the consented {path}"),
            );
        }
        self.check(
            self.got(mark, "A", "/same.png"),
            "the same-host redirect's hop reached A (within the host's consent)".into(),
        );
        self.check(
            !self.got(mark, "A", "/other.png"),
            "the other host's hop never reached A".into(),
        );
        let b_or_h: Vec<String> = self
            .lines_since(mark)
            .into_iter()
            .filter(|l| !l.starts_with("A "))
            .collect();
        self.check(
            b_or_h.is_empty(),
            format!(
                "B and H heard nothing (other port, plain HTTP, the plain picture): {b_or_h:?}"
            ),
        );
        self.check(self.bar().is_empty(), "the bar has gone".into());
        let shown = self.until(
            "the moved-or-deleted bar on the page served again",
            "!document.getElementById('file-bar').hidden",
        );
        self.check(
            shown,
            "the moved-or-deleted bar is still shown on the page served again".into(),
        );
        let html = document(&ports, &[]);
        self.page_asks_only_for_the_list(mark, &html, &["/same.png"]);

        println!("3. Requests made by this check's own script, not by the page");
        let mark = self.log.lines().len();
        let injected = [
            a("/a.png?v=2"),
            a("/a.png/"),
            a("/unlisted.png"),
            format!("https://127.0.0.1:{}/b-direct.png", ports.b),
        ];
        let script = format!(
            "(async () => {{ window.__blocked = []; document.addEventListener('securitypolicyviolation', (e) => window.__blocked.push(e.violatedDirective)); \
             const images = {list}.map((u) => new Promise((done) => {{ const i = new Image(); i.onload = () => done('loaded ' + u); i.onerror = () => done('refused ' + u); i.src = u; }})); \
             const fetched = fetch({first}).then(() => 'fetched', () => 'fetch refused'); \
             const frame = document.createElement('iframe'); frame.src = {first}; document.body.append(frame); \
             const style = document.createElement('link'); style.rel = 'stylesheet'; style.href = {first}; document.head.append(style); \
             return JSON.stringify([...(await Promise.all(images)), await fetched]); }})()",
            list = serde_free_list(&injected),
            first = json_string(&a("/a.png")),
        );
        let outcome = self.eval_async("the check's own requests", &script);
        let on_a = |what: &str| format!("loaded {}", a(what));
        self.check(
            outcome.contains(&on_a("/a.png?v=2"))
                && outcome.contains(&on_a("/a.png/"))
                && outcome.contains(&on_a("/unlisted.png")),
            format!("pictures anywhere on the consented host load (the host's consent): {outcome}"),
        );
        self.check(
            outcome.contains(&format!(
                "refused https://127.0.0.1:{}/b-direct.png",
                ports.b
            )) && outcome.contains("fetch refused"),
            format!("a picture on another port, and a fetch, are refused: {outcome}"),
        );
        self.until(
            "the frame's and stylesheet's refusals",
            "window.__blocked.includes('frame-src') && window.__blocked.includes('style-src-elem')",
        );
        let heard: Vec<String> = self
            .lines_since(mark)
            .into_iter()
            .filter(|l| !l.ends_with(" connection"))
            .filter(|l| {
                !(l.starts_with("A ")
                    && (l.ends_with(" /a.png?v=2")
                        || l.ends_with(" /a.png/")
                        || l.ends_with(" /unlisted.png")))
            })
            .collect();
        self.check(
            heard.is_empty(),
            format!("nothing else reached a server (not B, not the fetch, frame or stylesheet): {heard:?}"),
        );

        println!("4. A live reload adds a picture on A, and one on B");
        let generation = self.page_generation();
        let mark = self.log.lines().len();
        let b_png = format!("https://127.0.0.1:{}/b.png", ports.b);
        let a_new = a("/a-new.png");
        self.send(Ev::Replace(document(&ports, &[&a_new, &b_png])));
        self.has_picture(&b_png);
        self.check(
            !self.file_bar(),
            "the file back (a live reload): the moved-or-deleted bar has gone".into(),
        );
        self.until("the bar asking again", "(() => { const b = document.getElementById('pictures-bar'); return b && !b.hidden && b.textContent.includes('Show them?'); })()");
        let shown = self.shown();
        self.check(
            !shown.contains(&b_png) && shown.contains(&a("/a.png")) && shown.contains(&a_new),
            format!("B's waits; A's stay, the new one on A with them: {shown:?}"),
        );
        self.settled(generation + 1);
        self.check(
            self.got(mark, "A", "/a-new.png"),
            "the new picture on A arrived without asking".into(),
        );
        // A message about this document's pictures for an older render: ignored.
        self.eval(&format!(
            "LegibleChrome.remoteImages({{ generation: {generation}, show: [{}], ask: false }}); true",
            json_string(&b_png)
        ));
        // An answer for the render before: ignored.
        self.eval(&format!(
            "window.ipc.postMessage('remote-images\\nonce\\n{generation}'); true"
        ));
        self.eval(&format!(
            "window.ipc.postMessage('remote-images\\nonce\\n{}'); true",
            generation.saturating_sub(1)
        ));
        // A round trip, so both have been handled: then nothing for B.
        self.eval("true");
        self.check(
            !self.got(mark, "B", "/b.png") && !self.shown().contains(&b_png),
            "a stale answer or message let nothing in".into(),
        );
        let html = document(&ports, &[&a_new, &b_png]);
        self.page_asks_only_for_the_list(mark, &html, &["/same.png"]);
        self.eval("[...document.querySelectorAll('#pictures-bar button')].find((b) => b.textContent === 'Always for this file').click(); true");
        let generation = self.generation_at_least(generation + 2);
        self.settled(generation);
        self.check(
            self.got(mark, "B", "/b.png"),
            "Always for this file let B's in, with B in the CSP".into(),
        );

        println!("5. Never while a picture is on its way");
        self.send(Ev::Setting(RemoteImages::Always));
        let gate = OpenOnDrop(self.log.clone());
        *self.log.gate.lock().unwrap() = false;
        let mark = self.log.lines().len();
        self.send(Ev::Replace(document(&ports, &[&b_png, &a("/slow.png")])));
        let deadline = Instant::now() + BOUND;
        while !self.got(mark, "A", "/slow.png") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        self.check(
            self.got(mark, "A", "/slow.png"),
            "/slow.png reached A, and waits there".into(),
        );
        // Gone again, and Keep showing it chosen: the page served for Never mustn't bring the
        // bar back.
        self.send(Ev::FileGone);
        self.until(
            "the moved-or-deleted bar again",
            "!document.getElementById('file-bar').hidden",
        );
        self.eval("[...document.querySelectorAll('#file-bar button')].find((b) => b.textContent === 'Keep showing it').click(); true");
        let before = self.page_generation();
        self.send(Ev::Setting(RemoteImages::Never));
        let generation = self.generation_at_least(before + 1);
        self.settled(generation);
        self.eval("true");
        self.check(
            !self.file_bar(),
            "Keep showing it holds on the page served again".into(),
        );
        let after_never = self.log.lines().len();
        drop(gate);
        let deadline = Instant::now() + BOUND;
        while !self
            .lines_since(mark)
            .iter()
            .any(|l| l == "A answered /slow.png")
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(25));
        }
        self.eval("true");
        let shown = self.shown();
        self.check(
            shown.is_empty(),
            format!("no picture shows after Never, the late one included: {shown:?}"),
        );
        let late: Vec<String> = self
            .lines_since(after_never)
            .into_iter()
            .filter(|l| l != "A answered /slow.png" && !l.ends_with(" connection"))
            .collect();
        self.check(
            late.is_empty(),
            format!("nothing new reached a server after Never: {late:?}"),
        );
        self.check(
            self.page_generation() == generation,
            "the newest generation is the one shown".into(),
        );

        println!("6. Always, with fresh addresses; then Never with fresh ones");
        let mark = self.log.lines().len();
        self.send(Ev::Setting(RemoteImages::Always));
        self.send(Ev::Replace(document(
            &ports,
            &[&a("/fresh-1.png"), &a("/fresh-2.png")],
        )));
        let deadline = Instant::now() + BOUND;
        while !(self.got(mark, "A", "/fresh-1.png") && self.got(mark, "A", "/fresh-2.png"))
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(25));
        }
        self.check(
            self.got(mark, "A", "/fresh-1.png") && self.got(mark, "A", "/fresh-2.png"),
            "Always: the fresh pictures arrived".into(),
        );
        let before = self.page_generation();
        self.send(Ev::Setting(RemoteImages::Never));
        self.generation_at_least(before + 1);
        let mark = self.log.lines().len();
        self.send(Ev::Replace(document(&ports, &[&a("/fresh-3.png")])));
        self.has_picture(&a("/fresh-3.png"));
        self.eval("true");
        self.check(
            !self.got(mark, "A", "/fresh-3.png") && self.shown().is_empty(),
            "Never: a fresh picture never asked for".into(),
        );

        println!("7. A refusal that fails: this document shows no more pictures, and says why");
        self.send(Ev::Setting(RemoteImages::Always));
        let deadline = Instant::now() + BOUND;
        while !self.got(mark, "A", "/fresh-3.png") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        let generation = self.page_generation();
        // The filter consents to nothing now, while the page's CSP still lets A in: a picture on A
        // gets past the CSP and reaches a refusal, which is made to fail.
        self.send(Ev::RevokeQuietly);
        self.send(Ev::FailRefusals(true));
        let mark = self.log.lines().len();
        self.eval(&format!(
            "(() => {{ const i = new Image(); i.src = {}; return true; }})()",
            json_string(&a("/injected.png"))
        ));
        let generation = self.generation_at_least(generation + 1);
        self.settled(generation);
        self.send(Ev::FailRefusals(false));
        self.check(
            self.bar().contains("stopped showing"),
            format!("the bar says why: {:?}", self.bar()),
        );
        self.check(self.shown().is_empty(), "no picture shows".into());
        let released = self.got(mark, "A", "/injected.png");
        println!(
            "        (the request whose refusal failed {} A: it went ahead before the page was strict)",
            if released { "reached" } else { "didn't reach" }
        );
        let mark = self.log.lines().len();
        // Nothing brings them back for this document.
        self.eval(&format!(
            "window.ipc.postMessage('remote-images\\nonce\\n{generation}'); window.ipc.postMessage('remote-images\\nalways\\n{generation}'); true"
        ));
        self.send(Ev::Setting(RemoteImages::Always));
        self.send(Ev::Replace(document(&ports, &[&a("/fresh-4.png")])));
        self.has_picture(&a("/fresh-4.png"));
        let outcome = self.eval_async(
            "a later picture",
            &format!(
                "new Promise((done) => {{ const i = new Image(); i.onload = () => done('loaded'); i.onerror = () => done('refused'); i.src = {}; }})",
                json_string(&a("/after.png"))
            ),
        );
        self.check(
            outcome == "refused",
            format!("a later picture is refused: {outcome}"),
        );
        self.eval("true");
        let heard: Vec<String> = self
            .lines_since(mark)
            .into_iter()
            .filter(|l| !l.ends_with(" connection"))
            .collect();
        self.check(
            heard.is_empty(),
            format!("nothing reached a server after failing small: {heard:?}"),
        );
        self.check(
            self.shown().is_empty() && self.bar().contains("stopped showing"),
            "still none, still saying why".into(),
        );
        self.check(
            self.eval("1 + 1") == "2",
            "the window is alive and answering".into(),
        );
    }
}

/// A JSON string's text (the evaluated script's result), without a JSON parser.
fn serde_free_parse(json: &str) -> String {
    let t = json.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        t[1..t.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        t.to_owned()
    }
}

fn serde_free_list(items: &[String]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|i| json_string(i))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// Set on the check's own child run: the temp folder it keeps WebView2's data in.
const CHILD: &str = "LEGIBLE_REMOTE_IMAGES_CHECK_ROOT";
/// Set to make the check fail after its first step (to see a failing run clean up too).
const FAIL_ON_PURPOSE: &str = "LEGIBLE_CHECK_FAIL_ON_PURPOSE";
/// How long a whole run may take (in seconds, this variable shortens it: to see an overrun
/// clean up too), and how long WebView2's processes may hold the folder after.
const RUN_BOUND_VAR: &str = "LEGIBLE_CHECK_RUN_BOUND";
const RUN_BOUND: Duration = Duration::from_secs(300);
const RELEASE_BOUND: Duration = Duration::from_secs(30);

/// The check runs as a child of this process, with a temp folder of its own; this process
/// waits for it (ending it if it overruns), then removes that folder once WebView2's processes
/// have let it go, whether the check passed or not. Concurrent runs never share a folder.
fn main() {
    // A window of its own: quiet, so it never takes the user's focus, and checked
    // never to.
    legible_win::quiet::enable();
    let _focus = legible_win::quiet::watch_focus();
    if let Some(root) = std::env::var_os(CHILD) {
        check(&PathBuf::from(root));
        return;
    }
    std::process::exit(supervise());
}

fn supervise() -> i32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let root = std::env::temp_dir().join(format!(
        "legible-remote-images-check-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("the check's temp folder");
    let mut child = Command::new(std::env::current_exe().expect("this exe"))
        .env(CHILD, &root)
        .spawn()
        .expect("the check's run");
    let bound = std::env::var(RUN_BOUND_VAR)
        .ok()
        .and_then(|s| s.parse().ok())
        .map_or(RUN_BOUND, Duration::from_secs);
    let deadline = Instant::now() + bound;
    let mut code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(1),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                println!("the check overran: ended");
                let _ = child.kill();
                let _ = child.wait();
                break 1;
            }
        }
    };
    if !remove_when_released(&root) {
        println!("couldn't remove the check's temp folder {}", root.display());
        code = 1;
    }
    code
}

/// Removes `root`, retrying while WebView2's processes (which outlive the check by a moment)
/// still hold files in it, for [`RELEASE_BOUND`] at most.
fn remove_when_released(root: &Path) -> bool {
    let deadline = Instant::now() + RELEASE_BOUND;
    loop {
        match std::fs::remove_dir_all(root) {
            Ok(()) => return true,
            Err(_) if !root.exists() => return true,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return false,
        }
    }
}

/// The check itself, keeping WebView2's data under `root`.
fn check(root: &Path) {
    legible_core::i18n::init("en");
    let _ = rustls::crypto::ring::default_provider().install_default();
    // The servers.
    let certified =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".into(), "localhost".into()])
            .expect("a certificate");
    let pem = certified.cert.pem();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der()));
    let tls = Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certified.cert.der().to_vec())],
                key,
            )
            .expect("a TLS configuration"),
    );
    let bind = || TcpListener::bind("127.0.0.1:0").unwrap();
    let (la, lb, lh) = (bind(), bind(), bind());
    let ports = Arc::new(Ports {
        a: la.local_addr().unwrap().port(),
        b: lb.local_addr().unwrap().port(),
        h: lh.local_addr().unwrap().port(),
    });
    let log = Arc::new(Log::default());
    serve("A", la, Some(tls.clone()), log.clone(), ports.clone());
    serve("B", lb, Some(tls), log.clone(), ports.clone());
    serve("H", lh, None, log.clone(), ports.clone());

    // The window, its WebView2 with nothing loaded, the filter, then the page.
    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = legible_win::quiet::window(
        WindowBuilder::new().with_title("Legible: remote images check"),
        &event_loop,
    )
    .build(&event_loop)
        .unwrap();
    // WebView2's data, in this run's own folder (the parent removes it).
    let data = root.join("WebView2");
    let mut context = WebContext::new(Some(data.clone()));
    let served: Arc<Mutex<(Vec<u8>, String)>> = Arc::default();
    let serving = served.clone();
    let ipc = proxy.clone();
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_focused(false)
        .with_https_scheme(true)
        .with_custom_protocol(page::APP_PROTOCOL.into(), move |_, request| {
            serve_app(request.uri().path(), &serving.lock().unwrap())
        })
        .with_ipc_handler(move |request| {
            let _ = ipc.send_event(Ev::Ipc(request.body().clone()));
        })
        .build(&window)
        .unwrap();
    let core = webview.webview();
    // This check's certificate, and no other.
    {
        let pem = pem.split_whitespace().collect::<String>();
        let handler =
            ServerCertificateErrorDetectedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut text = PWSTR::null();
                // SAFETY: the event's own certificate; the string is copied and freed.
                let theirs = unsafe {
                    args.ServerCertificate()?.ToPemEncoding(&mut text)?;
                    take_pwstr(text)
                };
                let ours = theirs.split_whitespace().collect::<String>() == pem;
                let action = if ours {
                    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW
                } else {
                    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL
                };
                // SAFETY: as above.
                unsafe { args.SetAction(action) }
            }));
        let mut token = Default::default();
        // SAFETY: COM on this thread; the handler is WebView2's to keep.
        unsafe {
            core.cast::<ICoreWebView2_14>()
                .unwrap()
                .add_ServerCertificateErrorDetected(&handler, &mut token)
        }
        .unwrap();
    }
    let allowed = Rc::new(RefCell::new(HashSet::new()));
    let fail = Rc::new(Cell::new(false));
    let failed = proxy.clone();
    let installed = request_filter::install_for_checks(
        &core,
        allowed.clone(),
        Box::new(move || {
            let _ = failed.send_event(Ev::RemoteFailed);
        }),
        fail.clone(),
    );
    if let Err(error) = &installed {
        eprintln!("the request filter couldn't be installed: {error}");
        std::process::exit(1);
    }
    let html = document(&ports, &[]);
    let mut pictures = Pictures::new(allowed.clone(), None::<Unavailable>, &html);
    let origins = match pictures.grant(RemoteImages::Ask, false) {
        Next::Tell => Vec::new(),
        Next::ServeAgain(o) => o,
    };
    pictures.served(origins.clone());
    let first = page::document_page(
        "en",
        "Remote images check",
        &page::with_generation(&html, 0),
        &Settings::default(),
        0,
        &origins,
        true,
    );
    *served.lock().unwrap() = (
        first.into_bytes(),
        format!("{}{}", page::csp(&origins), page::CSP_HEADER_EXTRA),
    );
    webview.load_url(page::START_URL).unwrap();
    let mut host = Host {
        webview,
        served,
        pictures,
        generation: 0,
        document: html,
        setting: RemoteImages::Ask,
        always: false,
        allowed,
        file_bar: FileBar::default(),
    };

    // The steps, on a thread of their own.
    let driver_proxy = proxy.clone();
    let (log2, ports2) = (log.clone(), ports.clone());
    let result: Arc<Mutex<Option<Vec<String>>>> = Arc::default();
    let out = result.clone();
    std::thread::spawn(move || {
        let mut driver = Driver {
            proxy: driver_proxy.clone(),
            log: log2,
            ports: ports2,
            failures: Vec::new(),
            next_id: 0,
        };
        driver.run();
        *out.lock().unwrap() = Some(driver.failures);
        let _ = driver_proxy.send_event(Ev::Done);
    });

    let started = Instant::now();
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(200));
        match event {
            Event::UserEvent(Ev::Eval(script, reply)) => {
                let _ = host
                    .webview
                    .evaluate_script_with_callback(&script, move |value| {
                        let _ = reply.send(value);
                    });
            }
            Event::UserEvent(Ev::Replace(html)) => host.replace(html),
            Event::UserEvent(Ev::Setting(setting)) => {
                host.setting = setting;
                host.apply();
            }
            Event::UserEvent(Ev::FailRefusals(on)) => fail.set(on),
            Event::UserEvent(Ev::RevokeQuietly) => host.allowed.borrow_mut().clear(),
            Event::UserEvent(Ev::FileGone) => {
                host.file_bar.deleted();
                host.webview
                    .evaluate_script("window.LegibleChrome?.fileGone(true)")
                    .unwrap();
            }
            Event::UserEvent(Ev::Ipc(message)) => host.ipc(&message),
            Event::UserEvent(Ev::RemoteFailed) => {
                if host.pictures.failed() {
                    host.apply();
                }
            }
            Event::UserEvent(Ev::Done) => {
                let mut failures = result.lock().unwrap().take().unwrap_or_default();
                failures.extend(legible_win::quiet::focus_taken());
                println!("\nWhat the servers saw:");
                for line in log.lines().iter().filter(|l| !l.ends_with(" connection")) {
                    println!("  {line}");
                }
                if failures.is_empty() {
                    println!("\nAll passed.");
                    std::process::exit(0);
                }
                println!("\n{} failed:", failures.len());
                for f in &failures {
                    println!("  {f}");
                }
                std::process::exit(1);
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. })
                if started.elapsed() > Duration::from_secs(300) =>
            {
                println!("the check as a whole took too long");
                std::process::exit(1);
            }
            _ => {}
        }
    });
}
