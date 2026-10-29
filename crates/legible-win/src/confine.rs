//! What the `legible-doc` protocol may serve: the open document's images, and only from inside
//! its folder.
//!
//! A request's path is percent-decoded and checked before anything touches the disk: no `..`
//! or `.`, no empty parts (so no `//server`), no `\`, `:` (drives, alternate data streams) or
//! nulls. Then the file is found and canonicalised, which follows every symlink and junction
//! to where the file really is, and it is served only if that is still inside the document's
//! folder (canonicalised the same way) and is an image.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The open document's folder, the only tree the protocol serves from.
#[derive(Debug, Clone)]
pub struct DocRoot {
    root: PathBuf,
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The path breaks the rules above before it reaches the disk.
    BadPath,
    /// No such file (or it couldn't be read).
    NotFound,
    /// The file is really outside the document's folder (a symlink or junction leads out).
    Outside,
    /// Not a type the protocol serves.
    NotAnImage,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refusal::BadPath => "a path that isn't allowed",
            Refusal::NotFound => "no such file",
            Refusal::Outside => "outside the document's folder",
            Refusal::NotAnImage => "not an image",
        })
    }
}

impl DocRoot {
    /// The folder of `document`.
    pub fn for_document(document: &Path) -> io::Result<Self> {
        let folder = document
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no folder"))?;
        Self::new(folder)
    }

    pub fn new(folder: &Path) -> io::Result<Self> {
        Ok(Self {
            root: fs::canonicalize(folder)?,
        })
    }

    /// The canonical folder.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// The file a request's URL path (`/images/a%20b.png`) names, and its `Content-Type`.
    pub fn resolve(&self, url_path: &str) -> Result<(PathBuf, &'static str), Refusal> {
        let relative = checked_relative(url_path).ok_or(Refusal::BadPath)?;
        let content_type = image_type(&relative).ok_or(Refusal::NotAnImage)?;
        let file = fs::canonicalize(self.root.join(&relative)).map_err(|_| Refusal::NotFound)?;
        if !file.starts_with(&self.root) {
            return Err(Refusal::Outside);
        }
        if !file.is_file() {
            return Err(Refusal::NotFound);
        }
        Ok((file, content_type))
    }
}

impl DocRoot {
    /// Where a picture's URL path would be in the folder, checked by the same rules as
    /// [`resolve`](Self::resolve) before the disk is touched; it needn't exist. Live reload
    /// watches these, so a picture missing now is seen when it appears.
    pub fn candidate(&self, url_path: &str) -> Option<PathBuf> {
        let relative = checked_relative(url_path)?;
        image_type(&relative)?;
        Some(self.root.join(relative))
    }

    /// Whether `file` exists and really is inside the folder (its symlinks and junctions
    /// followed), as [`resolve`](Self::resolve) would serve it.
    pub fn holds(&self, file: &Path) -> bool {
        fs::canonicalize(file).is_ok_and(|real| real.starts_with(&self.root) && real.is_file())
    }
}

/// A picture's URL path without its live-reload generation: `/~12/img/a.png` is `/img/a.png`.
/// Only one leading `~<digits>` part is taken off; the rest is checked as any path is.
pub fn without_generation(url_path: &str) -> &str {
    let Some(rest) = url_path.strip_prefix("/~") else {
        return url_path;
    };
    match rest.find('/') {
        Some(end) if end > 0 && rest[..end].bytes().all(|b| b.is_ascii_digit()) => &rest[end..],
        _ => url_path,
    }
}

/// The decoded relative path, if it keeps the rules; parts are joined with `\`.
fn checked_relative(url_path: &str) -> Option<String> {
    let decoded = percent_decode(url_path.strip_prefix('/').unwrap_or(url_path))?;
    if decoded.is_empty() || decoded.contains(['\\', ':', '\0']) {
        return None;
    }
    let parts: Vec<&str> = decoded.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return None;
    }
    Some(parts.join("\\"))
}

/// `%XX` escapes decoded, as UTF-8; `None` for a broken escape or text that isn't UTF-8.
pub fn percent_decode(text: &str) -> Option<String> {
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

/// The `Content-Type` of an image the document may show, by its extension.
fn image_type(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_checked_before_the_disk() {
        assert_eq!(checked_relative("/a/b.png").as_deref(), Some(r"a\b.png"));
        assert_eq!(checked_relative("/a%20b.png").as_deref(), Some("a b.png"));
        for bad in [
            "/",
            "/../x.png",
            "/a/../../x.png",
            "/%2e%2e/x.png",
            "/a/./b.png",
            "//server/share/x.png",
            "/%2F%2Fserver/share/x.png",
            r"/a\..\x.png",
            "/a%5c..%5cx.png",
            "/C:/Windows/x.png",
            "/x.png:stream",
            "/x%00.png",
            "/%zz.png",
            "/%c3.png",
        ] {
            assert_eq!(checked_relative(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_images_are_served() {
        assert_eq!(image_type(r"a\b.PNG"), Some("image/png"));
        assert_eq!(image_type("x.svg"), Some("image/svg+xml"));
        assert_eq!(image_type("notes.md"), None);
        assert_eq!(image_type("script.js"), None);
        assert_eq!(image_type("noext"), None);
    }
}
