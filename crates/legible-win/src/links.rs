//! What clicking a link in a document does. The page never navigates itself: its script sends
//! every click's `href`, as the document wrote it, and [`classify`] decides:
//!
//! | The link | What happens |
//! |---|---|
//! | `#heading` | scrolls the page (the page does that itself) |
//! | `https://…` | opens in the default browser |
//! | `http://…` | asks first (it isn't secure), then opens in the default browser |
//! | `mailto:…` | opens the mail app |
//! | a `.md`, `.markdown` or `.mdown` file, relative to the document, by its drive path or as `file:` | opens it in a Legible window |
//! | anything else (`javascript:`, `data:`, `file:` to anything else, other apps' schemes, a link to a file that isn't there) | nothing |

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::confine::percent_decode;

/// The extensions Legible opens.
pub const MARKDOWN: [&str; 3] = ["md", "markdown", "mdown"];

/// What a link does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAction {
    /// A link within the page: it scrolls there itself.
    InPage,
    /// An `https` URL, for the default browser.
    Browser(String),
    /// An `http` URL: the user is asked first.
    BrowserAfterAsking(String),
    /// A `mailto:` URL, for the mail app.
    Mail(String),
    /// Another Markdown file, canonicalised.
    OpenDocument(PathBuf),
    /// Nothing happens.
    Refuse(Refused),
}

/// Why a link does nothing (logged, not shown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// A scheme Legible doesn't follow (`javascript:`, `data:`, `ftp:`, other apps').
    Scheme,
    /// An `http(s)` URL with no host, or with characters a URL can't have.
    BadUrl,
    /// A file that isn't Markdown.
    NotMarkdown,
    /// A file that isn't there.
    NotFound,
    /// A path that can't be placed: rooted (`/docs/a.md`) or scheme-relative (`//host/a.md`).
    Unplaceable,
}

/// What clicking `href` in `document` (its canonical path) does.
pub fn classify(href: &str, document: &Path) -> LinkAction {
    classify_in(href, Some(document))
}

/// What clicking `href` does in a window with no document (About's release notes, in the empty
/// window): a web or mail link as anywhere, but a path or a `file:` link has no document to be
/// placed by, so it does nothing.
pub fn classify_without_document(href: &str) -> LinkAction {
    classify_in(href, None)
}

fn classify_in(href: &str, document: Option<&Path>) -> LinkAction {
    let href = clean(href);
    if href.is_empty() || href.starts_with('#') {
        return LinkAction::InPage;
    }
    match scheme(&href) {
        Some(scheme) if scheme.len() > 1 => {
            let scheme = scheme.to_ascii_lowercase();
            match scheme.as_str() {
                "https" => web_url(&href).map_or(LinkAction::Refuse(Refused::BadUrl), |url| {
                    LinkAction::Browser(url)
                }),
                "http" => web_url(&href).map_or(LinkAction::Refuse(Refused::BadUrl), |url| {
                    LinkAction::BrowserAfterAsking(url)
                }),
                "mailto" => LinkAction::Mail(href),
                "file" if document.is_some() => file_url_path(&href)
                    .map_or(LinkAction::Refuse(Refused::Unplaceable), |p| markdown(&p)),
                "file" => LinkAction::Refuse(Refused::Unplaceable),
                _ => LinkAction::Refuse(Refused::Scheme),
            }
        }
        // A drive letter (`C:\notes\a.md`), or no scheme: a path.
        _ => match document {
            Some(document) => path_link(&href, document),
            None => LinkAction::Refuse(Refused::Unplaceable),
        },
    }
}

/// The href as a browser reads it: tabs and line breaks dropped, and the controls and spaces
/// at each end trimmed (so `java\nscript:` is still `javascript:`).
fn clean(href: &str) -> String {
    href.chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect::<String>()
        .trim_matches(|c: char| c <= ' ')
        .to_owned()
}

/// The scheme, if the href starts with one (`https`, `C` for a drive letter).
fn scheme(href: &str) -> Option<&str> {
    let end = href.find(':')?;
    let scheme = &href[..end];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic() && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)))
        .then_some(scheme)
}

/// An `http(s)` URL fit to hand to the browser: it has a host, and no spaces, quotes or other
/// characters a URL can't have.
fn web_url(href: &str) -> Option<String> {
    let rest = href.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    if host.is_empty() || host.starts_with(':') {
        return None;
    }
    let fine = |c: char| {
        (!c.is_ascii() && !c.is_control())
            || (c.is_ascii_graphic()
                && !matches!(c, '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}'))
    };
    href.chars().all(fine).then(|| href.to_owned())
}

/// The path a `file:` URL names: `file:///C:/a.md` is `C:\a.md`, `file://server/share/a.md`
/// is `\\server\share\a.md`.
fn file_url_path(href: &str) -> Option<PathBuf> {
    let rest = &href[href.find(':')? + 1..];
    let rest = rest.split(['?', '#']).next()?;
    let rest = percent_decode(rest)?.replace('/', "\\");
    let path = if let Some(after) = rest.strip_prefix(r"\\\") {
        // file:///C:/… : a local path.
        after.to_owned()
    } else {
        // file://host/share/… : a UNC path, or file://localhost/C:/….
        let after = rest.strip_prefix(r"\\")?;
        match after.strip_prefix(r"localhost\") {
            Some(local) => local.to_owned(),
            None => format!(r"\\{after}"),
        }
    };
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

/// A link to a path, relative to the document unless it has a drive.
fn path_link(href: &str, document: &Path) -> LinkAction {
    let Some(path) = href.split(['?', '#']).next().and_then(percent_decode) else {
        return LinkAction::Refuse(Refused::Unplaceable);
    };
    if path.starts_with(['/', '\\']) {
        return LinkAction::Refuse(Refused::Unplaceable);
    }
    let path = Path::new(&path);
    if path.is_absolute() {
        return markdown(path);
    }
    // A drive-relative path (`C:a.md`) has no one place.
    if matches!(path.components().next(), Some(Component::Prefix(_))) {
        return LinkAction::Refuse(Refused::Unplaceable);
    }
    match document.parent() {
        Some(folder) => markdown(&folder.join(path)),
        None => LinkAction::Refuse(Refused::Unplaceable),
    }
}

/// Whether `path` names a Markdown file by its extension (`.md`, `.markdown` or `.mdown`, any
/// case): what links and drops open.
pub fn has_markdown_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MARKDOWN.iter().any(|m| e.eq_ignore_ascii_case(m)))
}

/// What of a drop Legible opens: the Markdown files, in the order dropped, and never a folder
/// (even one named like a file).
pub fn dropped_markdown(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|p| has_markdown_extension(p) && !p.is_dir())
        .collect()
}

/// Opens `path` if it's a Markdown file that's there.
fn markdown(path: &Path) -> LinkAction {
    if !has_markdown_extension(path) {
        return LinkAction::Refuse(Refused::NotMarkdown);
    }
    match fs::canonicalize(path) {
        Ok(file) if file.is_file() => LinkAction::OpenDocument(file),
        _ => LinkAction::Refuse(Refused::NotFound),
    }
}
