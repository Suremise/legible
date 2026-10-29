//! Pictures from the internet, only with the reader's say-so: what a document asks for, what may
//! load, and the page's policy for it.
//!
//! - **What's asked for** comes from Rust's own render of the document ([`remote_sources`]),
//!   never from the page: each `<img data-remote-src="…">`. Only `https:` addresses on a host a
//!   CSP can name are offered; anything else stays a placeholder.
//! - **What may load** ([`decide`]): Settings' Never, Always or Ask; under Ask, the file's
//!   "Always for this file" ([`AlwaysList`]) or the hosts this window was shown once. Consent is
//!   per host: a new picture on a host already let in shows; one on a new
//!   host asks again.
//! - **Two guards** keep it to that. The page's CSP lists the consented hosts' origins
//!   ([`img_sources`]), so WebView2 refuses any other origin before a request exists, a
//!   redirect's included. WebView2's request filter ([`admits`]) lets through only images, by
//!   GET, over HTTPS, to those origins; Legible's own origins pass to its protocols. A redirect
//!   within a consented host is within the consent.
//! - **The page only asks for what's listed:** it sets an `<img>`'s `src` only to an address
//!   the window lists from this render, so its first requests are exactly the document's.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use url::{Host, Url};

use crate::page;
use crate::recent;
use crate::settings::{RemoteImages, json_string};

/// The file in Legible's folder that keeps "Always for this file".
pub const ALWAYS_FILE: &str = "remote-images.txt";
/// How many files it keeps; the oldest go first.
pub const ALWAYS_MAX: usize = 500;

/// A picture from the internet in a document.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Remote {
    /// As the document has it: what the page sets as the `<img>`'s `src`.
    pub raw: String,
    /// As WebView2 asks for it: parsed, without its fragment.
    pub address: String,
    /// Its origin, as the CSP names it (`https://host[:port]`).
    pub origin: String,
}

/// `raw` as a picture Legible may offer: `https:`, a domain or IPv4 host (a CSP host-source
/// can't name an IPv6 one), no user name or password.
pub fn remote(raw: &str) -> Option<Remote> {
    let mut url = Url::parse(raw.trim()).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let origin = match url.host()? {
        Host::Domain(_) | Host::Ipv4(_) => url.origin().ascii_serialization(),
        Host::Ipv6(_) => return None,
    };
    url.set_fragment(None);
    Some(Remote {
        raw: raw.to_owned(),
        address: url.into(),
        origin,
    })
}

/// An address WebView2 asks for, in the form [`Remote::address`] has: parsed, no fragment.
pub fn address(uri: &str) -> Option<String> {
    let mut url = Url::parse(uri).ok()?;
    url.set_fragment(None);
    Some(url.into())
}

/// The origin of a picture WebView2 asks for, if it may be one Legible offers: `https:`, on a
/// domain or IPv4 host (`https://host[:port]`, as [`Remote::origin`]).
pub fn origin_of(uri: &str) -> Option<String> {
    let url = Url::parse(uri).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    match url.host()? {
        Host::Domain(_) | Host::Ipv4(_) => Some(url.origin().ascii_serialization()),
        Host::Ipv6(_) => None,
    }
}

/// The pictures from the internet in `html` (Legible's own render), each once, in order.
pub fn remote_sources(html: &str) -> Vec<Remote> {
    const ATTR: &str = "data-remote-src=\"";
    let mut found: Vec<Remote> = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(ATTR) {
        rest = &rest[at + ATTR.len()..];
        let Some(end) = rest.find('"') else { break };
        if let Some(remote) = remote(&unescape(&rest[..end]))
            && !found.contains(&remote)
        {
            found.push(remote);
        }
        rest = &rest[end..];
    }
    found
}

/// An attribute's value as it was before escaping (the sanitiser's entities, and numeric ones).
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// What a window's document may show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decision {
    /// The pictures that may load (and that the page is told to show).
    pub granted: Vec<Remote>,
    /// Whether the bar asks about the others.
    pub ask: bool,
}

/// What may load of `sources` under `setting`, given the file's "Always for this file" and the
/// hosts (origins) this window was shown once.
pub fn decide(
    setting: RemoteImages,
    always_for_file: bool,
    shown_hosts: &HashSet<String>,
    sources: &[Remote],
) -> Decision {
    match setting {
        RemoteImages::Never => Decision::default(),
        RemoteImages::Always => Decision {
            granted: sources.to_vec(),
            ask: false,
        },
        RemoteImages::Ask if always_for_file => Decision {
            granted: sources.to_vec(),
            ask: false,
        },
        RemoteImages::Ask => {
            let granted: Vec<Remote> = sources
                .iter()
                .filter(|r| shown_hosts.contains(&r.origin))
                .cloned()
                .collect();
            let ask = granted.len() < sources.len();
            Decision { granted, ask }
        }
    }
}

/// Why a document's pictures from the internet can't show (until it's opened again).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// The request filter couldn't be installed, or this WebView2 can't filter every request.
    Runtime,
    /// A request couldn't be refused.
    Failed,
}

/// What a window's page needs after a change to what may load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Its CSP already fits: tell it ([`Pictures::message`]).
    Tell,
    /// Its CSP must change: serve the page again with these origins ([`Pictures::served`]).
    ServeAgain(Vec<String>),
}

/// A window's pictures from the internet, as its shell keeps them: the request filter's
/// consented origins (shared with the filter), the hosts shown once, the newest render's
/// pictures, and the origins the loaded page's CSP lets in.
#[derive(Debug)]
pub struct Pictures {
    allowed: Rc<RefCell<HashSet<String>>>,
    shown_hosts: HashSet<String>,
    sources: Vec<Remote>,
    page_origins: Vec<String>,
    unavailable: Option<Unavailable>,
}

impl Pictures {
    /// For a window whose filter shares `allowed` (or that has none: `unavailable`), showing
    /// the render `html`.
    pub fn new(
        allowed: Rc<RefCell<HashSet<String>>>,
        unavailable: Option<Unavailable>,
        html: &str,
    ) -> Pictures {
        Pictures {
            allowed,
            shown_hosts: HashSet::new(),
            sources: remote_sources(html),
            page_origins: Vec::new(),
            unavailable,
        }
    }

    /// The same window's, for the document that replaces this one, with nothing of this one's:
    /// the filter lets nothing through until the new document's page is served, and what was
    /// shown once is asked about again.
    pub fn for_another_document(&self) -> Pictures {
        self.allowed.borrow_mut().clear();
        Pictures::new(self.allowed.clone(), self.unavailable, "")
    }

    /// A new render of the document.
    pub fn render(&mut self, html: &str) {
        self.sources = remote_sources(html);
    }

    /// What may load now.
    pub fn decision(&self, setting: RemoteImages, always_for_file: bool) -> Decision {
        if self.unavailable.is_some() {
            return Decision::default();
        }
        decide(setting, always_for_file, &self.shown_hosts, &self.sources)
    }

    /// Lets the filter through exactly what may load now (so whatever is taken away is refused
    /// from this moment), and says what the page needs for it.
    pub fn grant(&mut self, setting: RemoteImages, always_for_file: bool) -> Next {
        let decision = self.decision(setting, always_for_file);
        let origins = img_sources(&decision.granted);
        self.allowed.replace(origins.iter().cloned().collect());
        if origins == self.page_origins {
            Next::Tell
        } else {
            Next::ServeAgain(origins)
        }
    }

    /// The page is served with a CSP letting pictures in from `origins`.
    pub fn served(&mut self, origins: Vec<String>) {
        self.page_origins = origins;
    }

    pub fn page_origins(&self) -> &[String] {
        &self.page_origins
    }

    pub fn unavailable(&self) -> Option<Unavailable> {
        self.unavailable
    }

    /// Show once: the hosts of this render's pictures, for this window.
    pub fn show_once(&mut self) {
        let hosts: Vec<String> = self.sources.iter().map(|r| r.origin.clone()).collect();
        self.shown_hosts.extend(hosts);
    }

    /// A request couldn't be refused: nothing more loads, and nothing shows from now on.
    /// Says whether that's news (it was showing until now).
    pub fn failed(&mut self) -> bool {
        self.allowed.borrow_mut().clear();
        let news = self.unavailable != Some(Unavailable::Failed);
        self.unavailable = Some(Unavailable::Failed);
        news
    }

    /// `{generation, show, ask, unavailable}` for the page (`LegibleChrome.remoteImages`).
    pub fn message(&self, generation: u64, setting: RemoteImages, always_for_file: bool) -> String {
        let decision = self.decision(setting, always_for_file);
        let show: Vec<String> = decision
            .granted
            .iter()
            .map(|r| json_string(&r.raw))
            .collect();
        // A document with no pictures from the internet has nothing to say about them.
        let unavailable = match self.unavailable {
            Some(_) if self.sources.is_empty() => "",
            Some(Unavailable::Runtime) => "runtime",
            Some(Unavailable::Failed) => "failed",
            None => "",
        };
        format!(
            "{{\"generation\":{generation},\"show\":[{}],\"ask\":{},\"unavailable\":\"{unavailable}\"}}",
            show.join(","),
            decision.ask
        )
    }
}

/// The page's `img-src` additions for `granted`: each origin once, in order.
pub fn img_sources(granted: &[Remote]) -> Vec<String> {
    granted
        .iter()
        .map(|r| r.origin.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// What WebView2 says of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Image,
    /// A document, frame, script, fetch, stylesheet, font, media or anything else.
    Other,
}

/// A request, as the filter reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub uri: String,
    pub context: Context,
    pub method: String,
}

/// The filter's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Legible's own origin: its protocols answer it.
    Own,
    /// A consented picture.
    Allow,
    Refuse,
}

/// Whether `request` may go ahead, given the window's consented hosts (origins).
pub fn admits(request: &Request, allowed: &HashSet<String>) -> Verdict {
    let Ok(url) = Url::parse(&request.uri) else {
        return Verdict::Refuse;
    };
    if is_own_origin(&url) {
        return Verdict::Own;
    }
    let consented = request.context == Context::Image
        && request.method == "GET"
        && origin_of(&request.uri).is_some_and(|o| allowed.contains(&o));
    if consented {
        Verdict::Allow
    } else {
        Verdict::Refuse
    }
}

/// One request at the filter: read, and refused if need be (WebView2's, or a test's).
pub trait Exchange {
    fn read(&self) -> io::Result<Request>;
    /// Answers the request with the window's denial response.
    fn refuse(&self) -> io::Result<()>;
}

/// What the filter did with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Own,
    Allowed,
    Refused,
    /// It couldn't be refused: the document's pictures must stop (the window fails small).
    RefusalFailed,
}

/// The filter, for one request: a request that can't be read is refused like any other that
/// isn't consented.
pub fn filter(exchange: &impl Exchange, allowed: &HashSet<String>) -> Outcome {
    let verdict = match exchange.read() {
        Ok(request) => admits(&request, allowed),
        Err(error) => {
            tracing::warn!(%error, "couldn't read a request: refused");
            Verdict::Refuse
        }
    };
    match verdict {
        Verdict::Own => Outcome::Own,
        Verdict::Allow => Outcome::Allowed,
        Verdict::Refuse => match exchange.refuse() {
            Ok(()) => Outcome::Refused,
            Err(error) => {
                tracing::error!(%error, "couldn't refuse a request");
                Outcome::RefusalFailed
            }
        },
    }
}

/// A window's WebView2, as the filter is installed on it (WebView2's, or a test's).
pub trait FilterHost {
    /// Whether the runtime can filter every request source (frames and workers too).
    fn filters_every_source(&self) -> bool;
    /// Makes the window's denial response, before anything loads.
    fn make_denial(&mut self) -> io::Result<()>;
    fn add_handler(&mut self) -> io::Result<()>;
    fn add_filter(&mut self) -> io::Result<()>;
}

/// Installs the filter, every step or none that counts: on an error the window shows no
/// pictures from the internet ([`page::CSP`] as it is, and the bar says why).
pub fn install(host: &mut impl FilterHost) -> io::Result<()> {
    if !host.filters_every_source() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this WebView2 can't filter frames' and workers' requests",
        ));
    }
    host.make_denial()?;
    host.add_handler()?;
    host.add_filter()
}

/// Whether `url` is one of Legible's own origins, exactly: `https`, the host, no port.
fn is_own_origin(url: &Url) -> bool {
    let own = [page::APP_ORIGIN, page::DOC_ORIGIN];
    let origin = url.origin().ascii_serialization();
    url.scheme() == "https" && url.port().is_none() && own.contains(&origin.as_str())
}

/// The files whose pictures always show ("Always for this file"), newest last, kept in
/// [`ALWAYS_FILE`] one canonical path a line (as recent files are, losslessly).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlwaysList {
    files: Vec<PathBuf>,
}

impl AlwaysList {
    /// The list in `file`; empty if there isn't one or it can't be read.
    pub fn load(file: &Path) -> AlwaysList {
        let text = match fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound {
                    tracing::warn!(%error, "couldn't read the files that always show pictures");
                }
                return AlwaysList::default();
            }
        };
        let mut list = AlwaysList::default();
        for path in text.lines().filter_map(recent::decode) {
            list.add(path);
        }
        list
    }

    /// Saves to `file`, through a temporary file renamed over it.
    pub fn save(&self, file: &Path) -> io::Result<()> {
        let mut text = String::new();
        for path in &self.files {
            text.push_str(&recent::encode(path));
            text.push('\n');
        }
        let temp = file.with_extension("txt.tmp");
        fs::write(&temp, text)?;
        fs::rename(&temp, file).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
    }

    pub fn contains(&self, file: &Path) -> bool {
        self.files.iter().any(|f| f == file)
    }

    /// Adds `file` (canonical) as the newest, keeping [`ALWAYS_MAX`].
    pub fn add(&mut self, file: PathBuf) {
        self.files.retain(|f| f != &file);
        self.files.push(file);
        if self.files.len() > ALWAYS_MAX {
            let extra = self.files.len() - ALWAYS_MAX;
            self.files.drain(..extra);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(raws: &[&str]) -> Vec<Remote> {
        raws.iter().map(|r| remote(r).unwrap()).collect()
    }

    #[test]
    fn only_https_on_a_nameable_host_is_offered() {
        let r = remote("https://Example.COM:443/a%20b.png?x=1&y=2#top").unwrap();
        assert_eq!(r.address, "https://example.com/a%20b.png?x=1&y=2");
        assert_eq!(r.origin, "https://example.com");
        assert_eq!(
            remote("https://cdn.example:8443/a.png").unwrap().origin,
            "https://cdn.example:8443"
        );
        assert_eq!(
            remote("https://bücher.example/a.png").unwrap().origin,
            "https://xn--bcher-kva.example"
        );
        assert_eq!(
            remote("https://192.0.2.1/a.png").unwrap().origin,
            "https://192.0.2.1"
        );
        for refused in [
            "http://example.com/a.png",
            "https://[2001:db8::1]/a.png",
            "https://user:pw@example.com/a.png",
            "ftp://example.com/a.png",
            "data:image/png;base64,AAAA",
            "not a url",
        ] {
            assert_eq!(remote(refused), None, "{refused}");
        }
    }

    #[test]
    fn the_sources_come_from_the_render_once_each_unescaped() {
        let html = r#"<p><img data-remote-src="https://x.example/a.png?u=1&amp;v=2" alt="a">
            <img data-remote-src="https://x.example/a.png?u=1&amp;v=2">
            <img data-remote-src="http://plain.example/b.png">
            <img data-remote-src="https://y.example/&#x63;.png"><img src="local.png"></p>"#;
        let found: Vec<String> = remote_sources(html).into_iter().map(|r| r.raw).collect();
        assert_eq!(
            found,
            ["https://x.example/a.png?u=1&v=2", "https://y.example/c.png"]
        );
        assert_eq!(unescape("a &amp b &bogus; &#;"), "a &amp b &bogus; &#;");
    }

    #[test]
    fn never_always_and_ask_decide_what_loads() {
        let all = sources(&["https://a.example/1.png", "https://b.example/2.png"]);
        let none = HashSet::new();
        assert_eq!(
            decide(RemoteImages::Never, true, &none, &all),
            Decision::default()
        );
        let always = Decision {
            granted: all.clone(),
            ask: false,
        };
        assert_eq!(decide(RemoteImages::Always, false, &none, &all), always);
        assert_eq!(decide(RemoteImages::Ask, true, &none, &all), always);
        let ask = decide(RemoteImages::Ask, false, &none, &all);
        assert_eq!(
            ask,
            Decision {
                granted: vec![],
                ask: true
            }
        );
        // Shown once, then a live reload adds pictures: one on a host let in shows, one on a new
        // host waits, and the bar asks about it.
        let once: HashSet<String> = all.iter().map(|r| r.origin.clone()).collect();
        assert_eq!(decide(RemoteImages::Ask, false, &once, &all), always);
        let more = sources(&[
            "https://a.example/1.png",
            "https://b.example/2.png",
            "https://a.example/new.png?x=1",
            "https://c.example/3.png",
        ]);
        assert_eq!(
            decide(RemoteImages::Ask, false, &once, &more),
            Decision {
                granted: more[..3].to_vec(),
                ask: true
            }
        );
        // No pictures: nothing to ask.
        assert_eq!(
            decide(RemoteImages::Ask, false, &none, &[]),
            Decision::default()
        );
    }

    #[test]
    fn a_windows_pictures_follow_consent_render_by_render() {
        let html = |srcs: &[&str]| -> String {
            srcs.iter()
                .map(|s| format!(r#"<img data-remote-src="{s}">"#))
                .collect()
        };
        let allowed = Rc::new(RefCell::new(HashSet::new()));
        let first = html(&["https://a.example/1.png", "https://a.example/2.png?x=1"]);
        let mut p = Pictures::new(allowed.clone(), None, &first);
        let ask = RemoteImages::Ask;
        let set =
            |items: &[&str]| -> HashSet<String> { items.iter().map(|s| s.to_string()).collect() };

        // Ask: nothing loads, the page's CSP is as strict as ever, and the bar asks.
        assert_eq!(p.grant(ask, false), Next::Tell);
        assert!(allowed.borrow().is_empty());
        assert_eq!(
            p.message(0, ask, false),
            r#"{"generation":0,"show":[],"ask":true,"unavailable":""}"#
        );

        // Show once: this render's host, and the page served again for its origin.
        p.show_once();
        assert_eq!(
            p.grant(ask, false),
            Next::ServeAgain(vec!["https://a.example".into()])
        );
        assert_eq!(*allowed.borrow(), set(&["https://a.example"]));
        p.served(vec!["https://a.example".into()]);
        assert_eq!(p.grant(ask, false), Next::Tell);

        // A live reload adds a picture on a, which shows, and one on b, which waits while the
        // bar asks about it.
        p.render(&html(&[
            "https://a.example/1.png",
            "https://a.example/new.png",
            "https://b.example/3.png",
        ]));
        assert_eq!(p.grant(ask, false), Next::Tell);
        assert_eq!(*allowed.borrow(), set(&["https://a.example"]));
        assert_eq!(
            p.message(4, ask, false),
            r#"{"generation":4,"show":["https://a.example/1.png","https://a.example/new.png"],"ask":true,"unavailable":""}"#
        );

        // Always for this file: both, and b's origin in the CSP.
        assert_eq!(
            p.grant(ask, true),
            Next::ServeAgain(vec!["https://a.example".into(), "https://b.example".into()])
        );
        p.served(vec!["https://a.example".into(), "https://b.example".into()]);

        // Never: nothing allowed at once, and the page served again with the strict CSP.
        assert_eq!(p.grant(RemoteImages::Never, true), Next::ServeAgain(vec![]));
        assert!(allowed.borrow().is_empty());
        p.served(vec![]);

        // Always, then a request that couldn't be refused: nothing from then on, whatever the
        // settings, and the page says why.
        assert!(matches!(
            p.grant(RemoteImages::Always, false),
            Next::ServeAgain(_)
        ));
        p.served(vec!["https://a.example".into(), "https://b.example".into()]);
        assert!(p.failed());
        assert!(!p.failed(), "only news once");
        assert!(allowed.borrow().is_empty());
        assert_eq!(
            p.grant(RemoteImages::Always, true),
            Next::ServeAgain(vec![])
        );
        p.show_once();
        assert_eq!(p.grant(ask, true), Next::ServeAgain(vec![]));
        assert!(allowed.borrow().is_empty());
        assert_eq!(
            p.message(9, RemoteImages::Always, true),
            r#"{"generation":9,"show":[],"ask":false,"unavailable":"failed"}"#
        );

        // No filter on this computer: nothing, and said only where there are pictures.
        let none = Pictures::new(Rc::default(), Some(Unavailable::Runtime), &first);
        assert_eq!(
            none.message(0, RemoteImages::Always, true),
            r#"{"generation":0,"show":[],"ask":false,"unavailable":"runtime"}"#
        );
        let plain = Pictures::new(
            Rc::default(),
            Some(Unavailable::Runtime),
            "<p>no pictures</p>",
        );
        assert!(plain.message(0, ask, false).contains(r#""unavailable":"""#));
    }

    #[test]
    fn the_policy_names_each_origin_once() {
        let granted = sources(&[
            "https://b.example/1.png",
            "https://a.example/2.png",
            "https://b.example/3.png?x",
            "https://b.example:8443/4.png",
        ]);
        assert_eq!(
            img_sources(&granted),
            [
                "https://a.example",
                "https://b.example",
                "https://b.example:8443"
            ]
        );
    }

    fn request(uri: &str, context: Context, method: &str) -> Request {
        Request {
            uri: uri.to_owned(),
            context,
            method: method.to_owned(),
        }
    }

    #[test]
    fn only_pictures_from_the_consented_hosts_load() {
        let allowed: HashSet<String> = img_sources(&sources(&["https://a.example/p.png?v=1"]))
            .into_iter()
            .collect();
        let image = |uri| admits(&request(uri, Context::Image, "GET"), &allowed);
        // Anywhere on a consented host (a redirect within it, or another picture of it).
        for allowed in [
            "https://a.example/p.png?v=1",
            "https://A.example:443/p.png?v=1#x",
            "https://a.example/p.png?v=2",
            "https://a.example/p.png/",
            "https://a.example/elsewhere/other.png",
        ] {
            assert_eq!(image(allowed), Verdict::Allow, "{allowed}");
        }
        for refused in [
            "https://a.example:8443/p.png?v=1",
            "http://a.example/p.png?v=1",
            "https://b.example/p.png?v=1",
            "https://sub.a.example/p.png",
            "https://a.example.evil.example/p.png",
            "nonsense",
        ] {
            assert_eq!(image(refused), Verdict::Refuse, "{refused}");
        }
        // A consented host gives pictures, by GET: anything else to it is refused.
        assert_eq!(
            admits(
                &request("https://a.example/p.png?v=1", Context::Other, "GET"),
                &allowed
            ),
            Verdict::Refuse
        );
        assert_eq!(
            admits(
                &request("https://a.example/p.png?v=1", Context::Image, "POST"),
                &allowed
            ),
            Verdict::Refuse
        );
    }

    #[test]
    fn legibles_own_origins_are_matched_exactly() {
        let none = HashSet::new();
        let any = |uri| admits(&request(uri, Context::Other, "GET"), &none);
        assert_eq!(any("https://legible.localhost/"), Verdict::Own);
        assert_eq!(
            any("https://legible.localhost/chrome/chrome.js"),
            Verdict::Own
        );
        assert_eq!(any("https://legible-doc.localhost/~3/a.png"), Verdict::Own);
        for lookalike in [
            "http://legible.localhost/",
            "https://legible.localhost:8443/",
            "https://legible.localhost.evil.example/",
            "https://evil.example/https://legible.localhost/",
            "https://xlegible.localhost/",
        ] {
            assert_eq!(any(lookalike), Verdict::Refuse, "{lookalike}");
        }
    }

    /// A request whose reading and refusing can each fail.
    struct Fake {
        request: Option<Request>,
        refusable: bool,
        refused: std::cell::Cell<u32>,
    }

    impl Exchange for Fake {
        fn read(&self) -> io::Result<Request> {
            self.request
                .clone()
                .ok_or_else(|| io::Error::other("unreadable"))
        }
        fn refuse(&self) -> io::Result<()> {
            self.refused.set(self.refused.get() + 1);
            if self.refusable {
                Ok(())
            } else {
                Err(io::Error::other("SetResponse failed"))
            }
        }
    }

    #[test]
    fn the_filter_refuses_what_it_cant_read_and_says_when_it_cant_refuse() {
        let allowed: HashSet<String> = ["https://a.example".to_owned()].into();
        let fake = |uri: Option<&str>, refusable| Fake {
            request: uri.map(|u| request(u, Context::Image, "GET")),
            refusable,
            refused: std::cell::Cell::new(0),
        };
        let ok = fake(Some("https://a.example/p.png"), true);
        assert_eq!(filter(&ok, &allowed), Outcome::Allowed);
        assert_eq!(ok.refused.get(), 0);
        let own = fake(Some("https://legible.localhost/"), false);
        assert_eq!(filter(&own, &allowed), Outcome::Own);
        assert_eq!(own.refused.get(), 0);
        let other = fake(Some("https://b.example/q.png"), true);
        assert_eq!(filter(&other, &allowed), Outcome::Refused);
        assert_eq!(other.refused.get(), 1);
        let unreadable = fake(None, true);
        assert_eq!(filter(&unreadable, &allowed), Outcome::Refused);
        assert_eq!(unreadable.refused.get(), 1);
        let stuck = fake(Some("https://b.example/q.png"), false);
        assert_eq!(filter(&stuck, &allowed), Outcome::RefusalFailed);
        assert_eq!(filter(&fake(None, false), &allowed), Outcome::RefusalFailed);
    }

    /// A WebView2 whose steps can each fail, recording those taken.
    #[derive(Default)]
    struct Host {
        old_runtime: bool,
        failing: Option<&'static str>,
        taken: Vec<&'static str>,
    }

    impl Host {
        fn step(&mut self, name: &'static str) -> io::Result<()> {
            self.taken.push(name);
            if self.failing == Some(name) {
                Err(io::Error::other(name))
            } else {
                Ok(())
            }
        }
    }

    impl FilterHost for Host {
        fn filters_every_source(&self) -> bool {
            !self.old_runtime
        }
        fn make_denial(&mut self) -> io::Result<()> {
            self.step("denial")
        }
        fn add_handler(&mut self) -> io::Result<()> {
            self.step("handler")
        }
        fn add_filter(&mut self) -> io::Result<()> {
            self.step("filter")
        }
    }

    #[test]
    fn the_filter_is_installed_whole_or_the_window_shows_no_pictures() {
        let mut host = Host::default();
        assert!(install(&mut host).is_ok());
        assert_eq!(host.taken, ["denial", "handler", "filter"]);
        let mut old = Host {
            old_runtime: true,
            ..Host::default()
        };
        assert_eq!(
            install(&mut old).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert!(old.taken.is_empty());
        for step in ["denial", "handler", "filter"] {
            let mut host = Host {
                failing: Some(step),
                ..Host::default()
            };
            assert!(install(&mut host).is_err(), "{step}");
            assert_eq!(host.taken.last(), Some(&step));
        }
    }

    #[test]
    fn always_for_this_file_is_kept_losslessly_and_bounded() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join(ALWAYS_FILE);
        let mut list = AlwaysList::default();
        let odd = PathBuf::from(OsString::from_wide(&[
            0x43, 0x3A, 0x5C, 0xD800, 0x2E, 0x6D, 0x64,
        ]));
        list.add(PathBuf::from(r"C:\Notes\Plan.md"));
        list.add(odd.clone());
        list.save(&file).unwrap();
        let loaded = AlwaysList::load(&file);
        assert!(loaded.contains(Path::new(r"C:\Notes\Plan.md")) && loaded.contains(&odd));
        assert_eq!(loaded, list);
        for n in 0..ALWAYS_MAX + 5 {
            list.add(PathBuf::from(format!(r"C:\Many\{n}.md")));
        }
        assert!(!list.contains(Path::new(r"C:\Notes\Plan.md")));
        assert!(list.contains(Path::new(&format!(r"C:\Many\{}.md", ALWAYS_MAX + 4))));
        assert_eq!(
            AlwaysList::load(&temp.path().join("none.txt")),
            AlwaysList::default()
        );
    }
}
