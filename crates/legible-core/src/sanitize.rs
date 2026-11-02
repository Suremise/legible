//! Sanitising: everything a document becomes, its own raw HTML included, passes through
//! `ammonia` with an allowlist of document tags and attributes before the view sees it.
//!
//! - **Removed with their content:** `<script>`, `<style>`, `<iframe>`, `<object>`, `<embed>`,
//!   `<template>`, `<noscript>`, form controls but a task list's tick, and **SVG and MathML**
//!   (stripped, not sanitised to a subset). Other tags not on the list (`<meta>`, `<form>`,
//!   `<link>`, `<base>`…) are removed and their text kept.
//! - **Attributes:** only those on the list; no `on*`, no `style`. Classes only Legible's own
//!   (`language-*` on code, alerts, Mermaid, footnotes). Every `id` gets [`ID_PREFIX`], and every
//!   `#fragment` link with it, the same way whatever the name already starts with, so the
//!   document can't clobber the page's own ids and its links still find their targets.
//! - **Links:** `http`, `https`, `mailto`, relative links and `#fragments` stay; a `file:` link
//!   stays only when it points inside the document's folder; anything else (`javascript:`,
//!   `data:`…) loses its `href`. Every link gets `rel="noopener noreferrer"`.
//! - **Images:** a relative `src` (or a `file:` one inside the document's folder) is rewritten
//!   onto the document base URL, which the view serves from that folder; one that leaves the
//!   folder loses its `src`. An `http`/`https` `src` becomes `data-remote-src`, with no `src`,
//!   so nothing loads until the user allows it. `data:` only for PNG, JPEG, GIF and WebP.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ammonia::{Builder, UrlRelative};

use crate::outline::ID_PREFIX;

const TAGS: &[&str] = &[
    "a",
    "abbr",
    "aside",
    "b",
    "bdi",
    "bdo",
    "blockquote",
    "br",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "input",
    "ins",
    "kbd",
    "li",
    "mark",
    "ol",
    "p",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "u",
    "ul",
    "var",
    "wbr",
];

/// Removed with everything inside them.
const CLEAN_CONTENT_TAGS: &[&str] = &[
    "script", "style", "iframe", "frame", "frameset", "object", "embed", "applet", "template",
    "noscript", "noembed", "svg", "math", "textarea", "select", "button", "canvas", "audio",
    "video",
];

// `data-sourcepos` only comes when the renderer was asked for it (the editor's preview).
const GENERIC_ATTRIBUTES: &[&str] = &["class", "data-sourcepos", "dir", "id", "lang", "title"];

const TAG_ATTRIBUTES: &[(&str, &[&str])] = &[
    ("a", &["href", "data-footnote-ref", "data-footnote-backref"]),
    ("abbr", &["title"]),
    ("col", &["span"]),
    ("colgroup", &["span"]),
    ("details", &["open"]),
    ("div", &["data-source"]),
    ("img", &["src", "alt", "width", "height"]),
    // Rewritten as a disabled checkbox by [`finish`].
    ("input", &["checked"]),
    ("li", &["value"]),
    ("ol", &["start", "reversed", "type"]),
    ("q", &["cite"]),
    ("blockquote", &["cite"]),
    ("section", &["data-footnotes"]),
    ("td", &["align", "colspan", "rowspan"]),
    ("th", &["align", "colspan", "rowspan", "scope"]),
    ("time", &["datetime"]),
];

/// Marks a remote image's address while ammonia runs; [`finish`] turns it into
/// `data-remote-src`.
const REMOTE_MARK: &str = "legible-remote:";

/// What sanitising found.
pub(crate) struct Sanitized {
    pub html: String,
    pub has_remote_images: bool,
}

/// Sanitises `html` for a document in `base_dir`, whose files the view serves at `doc_base`.
pub(crate) fn sanitize(html: &str, base_dir: &Path, doc_base: &str) -> Sanitized {
    let remote = Arc::new(AtomicBool::new(false));
    let urls = Urls {
        base_dir: normalize(base_dir),
        doc_base: doc_base.to_owned(),
        remote: Arc::clone(&remote),
    };

    // The document base's own scheme too, should the view serve on one of its own.
    let doc_scheme = scheme_of(doc_base).unwrap_or("https").to_ascii_lowercase();
    let mut schemes: HashSet<&str> = ["http", "https", "mailto", "file", "data"].into();
    schemes.insert(&doc_scheme);
    let tag_attributes: HashMap<&str, HashSet<&str>> = TAG_ATTRIBUTES
        .iter()
        .map(|(tag, attrs)| (*tag, attrs.iter().copied().collect()))
        .collect();
    let mut builder = Builder::empty();
    builder
        .tags(TAGS.iter().copied().collect())
        .clean_content_tags(CLEAN_CONTENT_TAGS.iter().copied().collect())
        .generic_attributes(GENERIC_ATTRIBUTES.iter().copied().collect())
        .tag_attributes(tag_attributes)
        .url_schemes(schemes)
        .url_relative(UrlRelative::PassThrough)
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true)
        .attribute_filter(move |element, attribute, value| urls.filter(element, attribute, value));
    let clean = builder.clean(html).to_string();
    Sanitized {
        html: finish(&clean),
        has_remote_images: remote.load(Ordering::Relaxed),
    }
}

struct Urls {
    /// The document's folder, made absolute by text; `None` when it can't be (then nothing
    /// outside the document is reachable).
    base_dir: Option<PathBuf>,
    doc_base: String,
    remote: Arc<AtomicBool>,
}

impl Urls {
    fn filter<'v>(&self, element: &str, attribute: &str, value: &'v str) -> Option<Cow<'v, str>> {
        match (element, attribute) {
            (_, "class") => classes(element, value),
            ("a", "href") => self.href(value),
            ("img", "src") => self.src(value),
            ("q" | "blockquote", "cite") => None,
            (_, "id") => Some(Cow::Owned(format!("{ID_PREFIX}{value}"))),
            _ => Some(Cow::Borrowed(value)),
        }
    }

    fn href<'v>(&self, value: &'v str) -> Option<Cow<'v, str>> {
        let value = value.trim();
        if let Some(fragment) = value.strip_prefix('#') {
            // Every id gets the prefix (below), so every fragment does too, even one that
            // already starts with it: `#user-content-x` names the id `user-content-x`, which is
            // `user-content-user-content-x` once prefixed.
            return Some(if fragment.is_empty() {
                Cow::Borrowed(value)
            } else {
                Cow::Owned(format!("#{ID_PREFIX}{fragment}"))
            });
        }
        match scheme_of(value).map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("http" | "https" | "mailto") => Some(Cow::Borrowed(value)),
            Some("file") => self
                .inside_from_file_url(value)
                .map(|_| Cow::Borrowed(value)),
            Some(_) => None,
            None if value.starts_with("//") => Some(Cow::Owned(format!("https:{value}"))),
            None => Some(Cow::Borrowed(value)),
        }
    }

    fn src<'v>(&self, value: &'v str) -> Option<Cow<'v, str>> {
        let value = value.trim();
        let remote = |url: String| {
            self.remote.store(true, Ordering::Relaxed);
            Some(Cow::Owned(format!("{REMOTE_MARK}{url}")))
        };
        match scheme_of(value).map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("http" | "https") => remote(value.to_owned()),
            Some("data") => allowed_data_image(value).then_some(Cow::Borrowed(value)),
            Some("file") => self
                .inside_from_file_url(value)
                .map(|relative| Cow::Owned(self.served(&relative))),
            Some(_) => None,
            None if value.starts_with("//") => remote(format!("https:{value}")),
            None => relative_path(value).map(|relative| Cow::Owned(self.served(&relative))),
        }
    }

    /// The document base URL for a path relative to the document's folder.
    fn served(&self, relative: &[String]) -> String {
        let encoded: Vec<String> = relative.iter().map(|part| percent_encode(part)).collect();
        format!("{}{}", self.doc_base, encoded.join("/"))
    }

    /// For a `file:` URL inside the document's folder, its path relative to the folder.
    fn inside_from_file_url(&self, url: &str) -> Option<Vec<String>> {
        let base = self.base_dir.as_ref()?;
        let path = normalize(&file_url_to_path(url)?)?;
        let (base, path) = (components(base), components(&path));
        let same = |a: &String, b: &String| {
            if cfg!(windows) {
                a.eq_ignore_ascii_case(b)
            } else {
                a == b
            }
        };
        let inside = path.len() > base.len() && base.iter().zip(&path).all(|(a, b)| same(a, b));
        inside.then(|| path[base.len()..].to_vec())
    }
}

/// The allowed classes of an element, the rest dropped; `None` when none is left.
fn classes<'v>(element: &str, value: &'v str) -> Option<Cow<'v, str>> {
    let allowed = |class: &str| match element {
        "code" => class.strip_prefix("language-").is_some_and(|lang| {
            !lang.is_empty()
                && lang
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+#._-".contains(c))
        }),
        "div" => class == "mermaid",
        "aside" => matches!(
            class,
            "alert" | "note" | "tip" | "important" | "warning" | "caution"
        ),
        "p" => class == "alert-title",
        "section" => class == "footnotes",
        "sup" => class == "footnote-ref",
        "a" => class == "footnote-backref",
        _ => false,
    };
    let kept: Vec<&str> = value
        .split_ascii_whitespace()
        .filter(|c| allowed(c))
        .collect();
    (!kept.is_empty()).then(|| Cow::Owned(kept.join(" ")))
}

fn allowed_data_image(url: &str) -> bool {
    let lower = url[5..].to_ascii_lowercase();
    ["image/png", "image/jpeg", "image/gif", "image/webp"]
        .iter()
        .any(|t| lower.starts_with(&format!("{t};")) || lower.starts_with(&format!("{t},")))
}

/// A URL's scheme, if it has one (`http` in `http://…`). A Windows drive (`C:\…`) counts as
/// one, so it's never taken for a relative path.
fn scheme_of(url: &str) -> Option<&str> {
    let colon = url.find(':')?;
    let scheme = &url[..colon];
    let mut chars = scheme.chars();
    (chars.next()?.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)))
    .then_some(scheme)
}

/// A relative URL's path within the document's folder, as decoded segments, or `None` when it
/// leaves the folder (`..` past it) or starts at a root.
fn relative_path(url: &str) -> Option<Vec<String>> {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let path = percent_decode(path)?.replace('\\', "/");
    if path.starts_with('/') {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part.to_owned()),
        }
    }
    (!parts.is_empty()).then_some(parts)
}

/// The path a `file:` URL names: `file:///C:/a/b`, `file://localhost/C:/a/b`, `file:///home/a`
/// or a share's `file://server/share/a`.
fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = &url[url.find(':')? + 1..];
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let rest = percent_decode(rest)?.replace('\\', "/");
    let path = if let Some(after) = rest.strip_prefix("//") {
        let (host, path) = after.split_at(after.find('/').unwrap_or(after.len()));
        if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
            path.to_owned()
        } else {
            format!("//{host}{path}")
        }
    } else {
        rest
    };
    // `/C:/…`: a Windows drive after the URL's leading slash.
    let bytes = path.as_bytes();
    let path = if bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
    {
        path[1..].to_owned()
    } else {
        path
    };
    Some(PathBuf::from(path))
}

/// An absolute path with `.` and `..` resolved by text, or `None` if it isn't absolute or `..`
/// goes past its root.
fn normalize(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                // At the root (`C:\`, `/`), `..` would leave it.
                out.parent()?;
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

fn components(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect()
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Decodes `%XX`; `None` for a malformed escape or bytes that aren't UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The last changes ammonia can't make, on its serialised output:
/// - each `<img … src="legible-remote:URL">` becomes `<img … data-remote-src="URL">`;
/// - each `<input>` becomes a disabled checkbox, ticked if it was: a task list's tick, the
///   only form control kept, which can't be changed.
///
/// Only inside those tags, reading quoted values as values, so text that happens to contain
/// the mark is left alone. (Serialised text has `<` escaped, so a `<` starts a real tag.)
fn finish(html: &str) -> String {
    let needle = format!(" src=\"{REMOTE_MARK}");
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let end = tag_end(rest);
        let tag = &rest[..end];
        if tag.starts_with("<img ") {
            out.push_str(&tag.replacen(&needle, " data-remote-src=\"", 1));
        } else if tag.starts_with("<input") && tag[6..].starts_with([' ', '>']) {
            out.push_str(if tag.contains(" checked=") {
                "<input type=\"checkbox\" checked=\"\" disabled=\"\">"
            } else {
                "<input type=\"checkbox\" disabled=\"\">"
            });
        } else {
            out.push_str(tag);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The length of the tag `html` starts with, to its `>` outside quotes.
fn tag_end(html: &str) -> usize {
    let mut quoted = false;
    for (i, c) in html.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '>' if !quoted => return i + 1,
            _ => {}
        }
    }
    html.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC_BASE: &str = "https://legible-doc.localhost/";

    /// An absolute folder on either system, named as [`file_url`] names it: off Windows a path's
    /// case counts, so the two must agree letter for letter.
    fn base() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Docs\Report")
        } else {
            PathBuf::from("/docs/Report")
        }
    }

    /// A `file:` URL for a path under (or beside) [`base`].
    fn file_url(rest: &str) -> String {
        if cfg!(windows) {
            format!("file:///C:/Docs/{rest}")
        } else {
            format!("file:///docs/{rest}")
        }
    }

    fn clean(html: &str) -> Sanitized {
        sanitize(html, &base(), DOC_BASE)
    }

    #[test]
    fn scripts_handlers_and_active_content_are_removed() {
        let html = clean(concat!(
            "<p>a<script>alert(1)</script>b</p>",
            "<img src=\"x.png\" onerror=\"alert(2)\">",
            "<a href=\"javascript:alert(3)\">j</a>",
            "<a href=\"JaVaScRiPt:alert(3)\">J</a>",
            "<iframe src=\"https://evil.example\">frame</iframe>",
            "<svg onload=\"alert(4)\"><circle r=\"1\"/><text>svg text</text></svg>",
            "<style>body{display:none}</style>",
            "<meta http-equiv=\"refresh\" content=\"0;url=https://evil.example\">",
            "<object data=\"x.swf\">obj</object><embed src=\"x.swf\">",
            "<form action=\"https://evil.example\"><input type=\"text\" name=\"q\"><button>Go</button></form>",
            "<p style=\"color:red\" onclick=\"alert(5)\">styled</p>",
            "<a href=\"data:text/html,<script>alert(6)</script>\">d</a>",
            "<img src=\"data:image/svg+xml,<svg onload=alert(7)>\">",
            "<base href=\"https://evil.example/\"><link rel=\"stylesheet\" href=\"https://evil.example/x.css\">",
        ))
        .html;
        for banned in [
            "<script",
            "alert(",
            "onerror",
            "onload",
            "onclick",
            "javascript:",
            "<iframe",
            "<svg",
            "svg text",
            "<style",
            "display:none",
            "<meta",
            "refresh",
            "<object",
            "<embed",
            "<form",
            "<button",
            "style=",
            "data:text",
            "data:image/svg",
            "<base",
            "<link",
            "evil.example",
        ] {
            assert!(
                !html
                    .to_ascii_lowercase()
                    .contains(&banned.to_ascii_lowercase()),
                "{banned} in {html}"
            );
        }
        // The text around them stays.
        assert!(html.contains("<p>ab</p>"), "{html}");
        // The only form control is a disabled checkbox.
        assert!(
            html.contains("<input type=\"checkbox\" disabled=\"\">"),
            "{html}"
        );
    }

    #[test]
    fn links_keep_safe_schemes_and_get_rel() {
        let html = clean(concat!(
            "<a href=\"https://example.com/a?b=1\">w</a>",
            "<a href=\"mailto:a@example.com\">m</a>",
            "<a href=\"other.md\">r</a>",
            "<a href=\"#section\">f</a>",
            "<a href=\"//example.com/x\">p</a>",
            "<a href=\"ftp://example.com/x\">ftp</a>",
        ))
        .html;
        assert!(
            html.contains(
                "<a href=\"https://example.com/a?b=1\" rel=\"noopener noreferrer\">w</a>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<a href=\"mailto:a@example.com\" rel=\"noopener noreferrer\">m</a>")
        );
        assert!(html.contains("<a href=\"other.md\" rel=\"noopener noreferrer\">r</a>"));
        assert!(
            html.contains("<a href=\"#user-content-section\" rel=\"noopener noreferrer\">f</a>")
        );
        assert!(html.contains("<a href=\"https://example.com/x\""));
        assert!(
            html.contains("<a rel=\"noopener noreferrer\">ftp</a>"),
            "{html}"
        );
    }

    /// A path's case counts where the system's does: on Windows a link to the folder in another
    /// case is inside it; elsewhere it's another folder.
    #[test]
    fn a_file_link_in_another_case_is_inside_only_on_windows() {
        let other_case = file_url("REPORT/notes/other.md");
        let html = clean(&format!("<a href=\"{other_case}\">in</a>")).html;
        assert_eq!(
            html.contains(&format!("<a href=\"{other_case}\"")),
            cfg!(windows),
            "{html}"
        );
    }

    #[test]
    fn file_links_only_inside_the_document_s_folder() {
        let inside = file_url("Report/notes/other.md");
        let outside = file_url("Secret/passwords.md");
        let escaping = file_url("Report/../Secret/passwords.md");
        let html = clean(&format!(
            "<a href=\"{inside}\">in</a><a href=\"{outside}\">out</a><a href=\"{escaping}\">up</a><a href=\"file:///etc/passwd\">etc</a>"
        ))
        .html;
        assert!(html.contains(&format!("<a href=\"{inside}\"")), "{html}");
        assert!(
            html.contains("<a rel=\"noopener noreferrer\">out</a>"),
            "{html}"
        );
        assert!(
            html.contains("<a rel=\"noopener noreferrer\">up</a>"),
            "{html}"
        );
        assert!(
            html.contains("<a rel=\"noopener noreferrer\">etc</a>"),
            "{html}"
        );
    }

    #[test]
    fn local_images_are_served_from_the_document_s_folder() {
        let inside = file_url("Report/img/chart.png");
        let html = clean(&format!(
            concat!(
                "<img src=\"img/a b.png\" alt=\"A\">",
                "<img src=\"./img/%C3%A9t%C3%A9.png\">",
                "<img src=\"img\\back.png\">",
                "<img src=\"../outside.png\">",
                "<img src=\"img/../../outside.png\">",
                "<img src=\"/rooted.png\">",
                "<img src=\"{inside}\">",
            ),
            inside = inside
        ));
        let html = html.html;
        assert!(
            html.contains("<img src=\"https://legible-doc.localhost/img/a%20b.png\" alt=\"A\">"),
            "{html}"
        );
        assert!(html.contains("<img src=\"https://legible-doc.localhost/img/%C3%A9t%C3%A9.png\">"));
        assert!(html.contains("<img src=\"https://legible-doc.localhost/img/back.png\">"));
        assert!(
            html.contains("<img src=\"https://legible-doc.localhost/img/chart.png\">"),
            "{html}"
        );
        assert_eq!(
            html.matches("<img>").count(),
            3,
            "three leave the folder: {html}"
        );
    }

    #[test]
    fn remote_images_keep_no_src() {
        let s = clean(concat!(
            "<img src=\"https://tracker.example/pixel.gif?u=1&amp;v=2\" alt=\"t\">",
            "<img src=\"//cdn.example/x.png\">",
            "<p>text with src=\"legible-remote:x\" in it</p>",
        ));
        assert!(s.has_remote_images);
        assert!(
            !s.html.contains("<img src=") && !s.html.contains(" src=\"http"),
            "{}",
            s.html
        );
        assert!(
            s.html.contains(
                "<img data-remote-src=\"https://tracker.example/pixel.gif?u=1&amp;v=2\" alt=\"t\">"
            ),
            "{}",
            s.html
        );
        assert!(
            s.html
                .contains("<img data-remote-src=\"https://cdn.example/x.png\">")
        );
        assert!(
            s.html.contains("text with src=\"legible-remote:x\" in it"),
            "text is left alone"
        );
        assert!(!clean("<img src=\"a.png\">").has_remote_images);
    }

    #[test]
    fn data_images_only_in_raster_formats() {
        let html = clean(concat!(
            "<img src=\"data:image/png;base64,iVBORw0KGgo=\">",
            "<img src=\"data:image/webp;base64,UklGRg==\">",
            "<img src=\"data:image/svg+xml;base64,PHN2Zz4=\">",
            "<img src=\"data:text/html;base64,PHNjcmlwdD4=\">",
        ))
        .html;
        assert!(html.contains("data:image/png;base64"));
        assert!(html.contains("data:image/webp;base64"));
        assert!(
            !html.contains("svg+xml") && !html.contains("text/html"),
            "{html}"
        );
    }

    #[test]
    fn ids_and_classes() {
        let html = clean(concat!(
            "<h2 id=\"intro\">I</h2><p id=\"user-content-kept\">K</p>",
            "<pre><code class=\"language-rust evil\">x</code></pre>",
            "<div class=\"mermaid other\" data-source=\"graph TD\"></div>",
            "<p class=\"anything\">c</p>",
            "<code class=\"language-\\&quot;x\">y</code>",
        ))
        .html;
        assert!(html.contains("<h2 id=\"user-content-intro\">"), "{html}");
        // Prefixed like any other: a link to `#user-content-kept` is prefixed the same way.
        assert!(
            html.contains("<p id=\"user-content-user-content-kept\">"),
            "{html}"
        );
        assert!(html.contains("<code class=\"language-rust\">"));
        assert!(html.contains("<div class=\"mermaid\" data-source=\"graph TD\"></div>"));
        assert!(html.contains("<p>c</p>"), "{html}");
        assert!(html.contains("<code>y</code>"), "{html}");
    }

    #[test]
    fn helpers() {
        assert_eq!(scheme_of("http://x"), Some("http"));
        assert_eq!(scheme_of("C:\\x"), Some("C"));
        assert_eq!(scheme_of("a/b:c"), None);
        assert_eq!(scheme_of("./a:b"), None);
        assert_eq!(
            relative_path("a/./b/../c.png?x#y"),
            Some(vec!["a".into(), "c.png".into()])
        );
        assert_eq!(relative_path("../a"), None);
        assert_eq!(relative_path("%ZZ"), None);
        assert_eq!(percent_encode("a b/é"), "a%20b%2F%C3%A9");
        assert_eq!(
            file_url_to_path("file://localhost/C:/a%20b"),
            Some(PathBuf::from("C:/a b"))
        );
        assert_eq!(
            file_url_to_path("file://server/share/a"),
            Some(PathBuf::from("//server/share/a"))
        );
        assert_eq!(
            file_url_to_path("file:///home/a"),
            Some(PathBuf::from("/home/a"))
        );
    }
}
