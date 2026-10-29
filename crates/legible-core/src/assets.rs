//! The web assets, compiled into the exe so the portable zip needs no extra files.
//!
//! Two folders are embedded: `web/` (Legible's own CSS, JavaScript and HTML: `doc/`, `chrome/`
//! and `editor/`) and `third_party/js/` (the pinned, audited bundles: highlight.js, Mermaid,
//! CodeMirror). The view asks for them by path:
//!
//! - `doc/…`, `chrome/…`, `editor/…`: a file under `web/`;
//! - `js/…`: a file under `third_party/js/`.
//!
//! Only types a page loads are served ([`content_type`]): a README, `VERSIONS` or
//! `LICENSES.md` beside them isn't.

use include_dir::{Dir, include_dir};

static WEB: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../web");
static JS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../third_party/js");

/// An embedded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    pub bytes: &'static [u8],
    /// Its `Content-Type`.
    pub content_type: &'static str,
}

/// The embedded file at `path` (`doc/document.css`, `js/highlight.min.js`), or `None` if
/// there's no such file or it isn't a type a page loads. `/` and `\` both separate folders;
/// `..` is never followed.
pub fn get(path: &str) -> Option<Asset> {
    let path = path.trim_start_matches(['/', '\\']).replace('\\', "/");
    if path.split('/').any(|part| part == ".." || part.is_empty()) {
        return None;
    }
    let file = match path.strip_prefix("js/") {
        Some(rest) => JS.get_file(rest)?,
        None => WEB.get_file(&path)?,
    };
    let content_type = content_type(&path)?;
    Some(Asset {
        bytes: file.contents(),
        content_type,
    })
}

/// The `Content-Type` for a file a page loads, by its extension, or `None` for any other file.
pub fn content_type(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendored_bundles_are_embedded() {
        for name in ["highlight.min.js", "codemirror.min.js", "mermaid.min.js"] {
            let asset = get(&format!("js/{name}")).unwrap_or_else(|| panic!("{name}"));
            assert!(asset.bytes.len() > 10_000, "{name}");
            assert_eq!(asset.content_type, "text/javascript; charset=utf-8");
        }
    }

    #[test]
    fn the_document_look_is_embedded() {
        let css = get("doc/document.css").expect("document.css");
        assert_eq!(css.content_type, "text/css; charset=utf-8");
        let js = get("doc/document.js").expect("document.js");
        assert_eq!(js.content_type, "text/javascript; charset=utf-8");
    }

    #[test]
    fn only_files_a_page_loads_are_served() {
        assert_eq!(get("js/VERSIONS"), None);
        assert_eq!(get("js/LICENSES.md"), None);
        assert_eq!(get("README.md"), None);
        assert_eq!(get("js/no-such.js"), None);
    }

    #[test]
    fn paths_never_leave_the_folders() {
        assert_eq!(get("doc/../../Cargo.toml"), None);
        assert_eq!(get("js/../web/README.md"), None);
        assert_eq!(get(r"js\..\highlight.min.js"), None);
        assert_eq!(get("js//highlight.min.js"), None);
        assert_eq!(get("/js/highlight.min.js"), get("js/highlight.min.js"));
        assert_eq!(get(r"js\highlight.min.js"), get("js/highlight.min.js"));
    }

    #[test]
    fn content_types_by_extension() {
        assert_eq!(content_type("a/b.CSS"), Some("text/css; charset=utf-8"));
        assert_eq!(content_type("x.svg"), Some("image/svg+xml"));
        assert_eq!(content_type("noext"), None);
        assert_eq!(content_type("x.exe"), None);
    }
}
