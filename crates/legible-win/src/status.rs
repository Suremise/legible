//! The status bar's facts about a window's file: its path, where it was opened from
//! when that's why it can't be edited, and, for the editor, its encoding and line endings. The
//! page shows them (web/chrome/chrome.js, `LegibleChrome.status`); the word count is the page's
//! own, as the text changes.

use std::path::Path;

use legible_core::i18n::tr;
use legible_core::save::{Encoding, LineEnding, TextFormat};
use legible_core::tr;

use crate::editor;
use crate::jumplist;
use crate::settings::json_string;

/// Where a file was opened from, when it's somewhere Windows keeps things for a while: an
/// edit there would be lost, so it opens for reading only ([`editor::editable`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Outlook's attachments folder.
    Attachment,
    /// The browser's cache (a download opened from the browser).
    Download,
    /// The temp folder (an attachment or a download from most other apps).
    Temp,
}

impl Origin {
    /// Where `file` (canonical) was opened from, if it's one of those; `temp` is the temp
    /// folder, canonical.
    pub fn of(file: &Path, temp: Option<&Path>) -> Option<Origin> {
        let folder = file.parent()?;
        let has = |name: &str| {
            folder
                .components()
                .any(|part| part.as_os_str().eq_ignore_ascii_case(name))
        };
        if has("Content.Outlook") {
            Some(Origin::Attachment)
        } else if editor::CACHES.iter().any(|cache| has(cache)) {
            Some(Origin::Download)
        } else if temp.is_some_and(|temp| file.starts_with(temp)) {
            Some(Origin::Temp)
        } else {
            None
        }
    }

    /// As the status bar says it.
    pub fn note(self) -> String {
        tr(match self {
            Origin::Attachment => "status-from-attachment",
            Origin::Download => "status-from-download",
            Origin::Temp => "status-from-temp",
        })
    }
}

/// The encoding, as the status bar names it.
pub fn encoding_label(encoding: Encoding) -> String {
    tr(match encoding {
        Encoding::Utf8 => "status-utf8",
        Encoding::Utf8Bom => "status-utf8-bom",
        Encoding::Utf16Le => "status-utf16le",
        Encoding::Utf16Be => "status-utf16be",
    })
}

/// The line endings, as the status bar names them: the file's own, or, when it mixed them,
/// which saving keeps.
pub fn endings_label(format: &TextFormat) -> String {
    let ending = match format.line_ending {
        LineEnding::Crlf => tr("status-crlf"),
        LineEnding::Lf => tr("status-lf"),
        LineEnding::Cr => tr("status-cr"),
    };
    if format.mixed {
        tr!("status-endings-mixed", kept = ending)
    } else {
        ending
    }
}

/// What the page's status bar is told: `{path, note, encoding, endings}`, each a string or
/// null (no file; nothing to note; a file Legible can't save as it is).
pub fn json(file: Option<&Path>, origin: Option<Origin>, format: Option<&TextFormat>) -> String {
    let string = |s: Option<String>| s.map_or("null".to_owned(), |s| json_string(&s));
    let path = file.map(|f| jumplist::display_path(f).to_string_lossy().into_owned());
    format!(
        "{{\"path\":{},\"note\":{},\"encoding\":{},\"endings\":{}}}",
        string(path),
        string(origin.map(Origin::note)),
        string(format.map(|f| encoding_label(f.encoding))),
        string(format.map(endings_label)),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use legible_core::save::decode;

    use super::*;

    #[test]
    fn a_file_from_an_attachment_a_download_or_the_temp_folder_says_so() {
        let temp = PathBuf::from(r"C:\Users\Sam\AppData\Local\Temp");
        let of = |p: &str| Origin::of(Path::new(p), Some(&temp));
        assert_eq!(
            of(
                r"C:\Users\Sam\AppData\Local\Microsoft\Windows\INetCache\Content.Outlook\AB12\notes.md"
            ),
            Some(Origin::Attachment)
        );
        assert_eq!(
            of(r"C:\Users\Sam\AppData\Local\Microsoft\Windows\INetCache\IE\XY\notes.md"),
            Some(Origin::Download)
        );
        assert_eq!(
            of(r"C:\Users\Sam\AppData\Local\Temp\7zO1234\notes.md"),
            Some(Origin::Temp)
        );
        assert_eq!(of(r"C:\Users\Sam\Documents\notes.md"), None);
        assert_eq!(Origin::Attachment.note(), "Opened from an email attachment");
    }

    /// The format a file's bytes decode with, as opening the file reads them.
    fn format(bytes: &[u8]) -> TextFormat {
        decode(bytes).expect("a supported file").format
    }

    #[test]
    fn the_encoding_and_line_endings_are_named_as_saving_keeps_them() {
        let cases: [(&[u8], &str, &str); 7] = [
            (b"a\r\nb\r\n", "UTF-8", "CRLF"),
            (b"a\nb\n", "UTF-8", "LF"),
            (b"\xEF\xBB\xBFa\r\nb\r\n", "UTF-8 with BOM", "CRLF"),
            (b"\xFF\xFEa\x00\r\x00\n\x00", "UTF-16 LE", "CRLF"),
            (b"\xFE\xFF\x00a\x00\n", "UTF-16 BE", "LF"),
            (b"a\r\nb\r\nc\n", "UTF-8", "Mixed (CRLF kept)"),
            (b"a\nb\nc\r\n", "UTF-8", "Mixed (LF kept)"),
        ];
        for (bytes, encoding, endings) in cases {
            let format = format(bytes);
            assert_eq!(encoding_label(format.encoding), encoding, "{bytes:?}");
            assert_eq!(endings_label(&format), endings, "{bytes:?}");
        }
    }

    #[test]
    fn the_page_is_told_the_path_as_windows_shows_it() {
        let format = format(b"a\r\n");
        let json = json(Some(Path::new(r"\\?\C:\Notes\Q&A.md")), None, Some(&format));
        assert_eq!(
            json,
            r#"{"path":"C:\\Notes\\Q\u0026A.md","note":null,"encoding":"UTF-8","endings":"CRLF"}"#
        );
        assert_eq!(
            super::json(None, None, None),
            r#"{"path":null,"note":null,"encoding":null,"endings":null}"#
        );
    }
}
