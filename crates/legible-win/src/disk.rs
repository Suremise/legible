//! A window's document as it is on disk, as last read: its text and the format it's saved in
//! (`legible_core::save`), or, in an encoding Legible doesn't edit, its bytes as they are; and
//! the stamp of exactly those bytes, which saving checks before it replaces the file.

use std::io;
use std::path::Path;

use legible_core::save::{self, Stamp, TextFormat};

use crate::win32;

/// What the document holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// Text Legible can edit and save as it was.
    Text { text: String, format: TextFormat },
    /// Not UTF-8 or UTF-16 with a BOM: shown (`lossy`) but never edited, and copied only as
    /// the bytes it is.
    Unsupported { bytes: Vec<u8>, lossy: String },
}

impl Content {
    /// The content of a file's `bytes`.
    pub fn from_bytes(bytes: Vec<u8>) -> Content {
        match save::decode(&bytes) {
            Ok(decoded) => Content::Text {
                text: decoded.text,
                format: decoded.format,
            },
            Err(unsupported) => Content::Unsupported {
                bytes,
                lossy: unsupported.lossy,
            },
        }
    }

    /// The text to show: the file's, or as well as it can be read.
    pub fn text(&self) -> &str {
        match self {
            Content::Text { text, .. } => text,
            Content::Unsupported { lossy, .. } => lossy,
        }
    }

    /// The format to save in; None if it can't be edited.
    pub fn format(&self) -> Option<&TextFormat> {
        match self {
            Content::Text { format, .. } => Some(format),
            Content::Unsupported { .. } => None,
        }
    }
}

/// The document on disk, as read: what it holds, and the stamp of those bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    pub content: Content,
    pub stamp: Stamp,
}

impl Disk {
    /// The document at `path`, read: its bytes and their stamp from one handle.
    pub fn read(path: &Path) -> io::Result<Disk> {
        let (bytes, stamp) = save::read(path, &win32::identity)?;
        Ok(Disk::of(bytes, stamp))
    }

    /// The document of `bytes`, read with `stamp`.
    pub fn of(bytes: Vec<u8>, stamp: Stamp) -> Disk {
        Disk {
            content: Content::from_bytes(bytes),
            stamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use legible_core::save::{Encoding, LineEnding};

    #[test]
    fn utf16_opens_as_text_and_latin1_as_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let utf16 = dir.path().join("utf16.md");
        let bytes: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("# Title\r\n".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        std::fs::write(&utf16, &bytes).unwrap();
        let disk = Disk::read(&utf16).unwrap();
        assert_eq!(disk.content.text(), "# Title\n");
        let format = disk.content.format().unwrap();
        assert_eq!(
            (format.encoding, format.line_ending),
            (Encoding::Utf16Le, LineEnding::Crlf)
        );
        assert_eq!(disk.stamp.len, bytes.len() as u64);
        assert!(
            disk.stamp.identity.is_some(),
            "Windows gives a file's identity"
        );

        let latin1 = dir.path().join("latin1.md");
        std::fs::write(&latin1, b"caf\xe9").unwrap();
        let disk = Disk::read(&latin1).unwrap();
        assert!(disk.content.format().is_none());
        assert_eq!(
            disk.content,
            Content::Unsupported {
                bytes: b"caf\xe9".to_vec(),
                lossy: "caf\u{FFFD}".to_owned()
            }
        );
    }
}
