//! The real clipboard: a copy put on it as Legible does, then read back. It overwrites what's on
//! the clipboard, so it never runs in the suite: run it by hand, knowingly,
//!
//!   cargo nextest run -p legible-win --run-ignored only -E 'binary(clipboard)'
//!
//! Its automated cover is `clipboard`'s tests, against a recording clipboard.
#![cfg(windows)]

use legible_win::clipboard::{self, Request, Written};
use legible_win::win32::clipboard::{MessageWindow, WindowsClipboard, html_format, read};

/// A CF_HTML header's offset, by name.
fn offset(header: &str, name: &str) -> usize {
    header
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{name}:")))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("no {name} in {header}"))
}

#[test]
#[ignore = "overwrites the clipboard: run by hand"]
fn a_copy_is_on_the_real_clipboard_in_both_formats_with_its_offsets() {
    let owner = MessageWindow::new().expect("a message-only window");
    let fragment_text = "Ünïcödé — 日本語";
    let built = clipboard::build(Request {
        seq: 1,
        window: (),
        page: clipboard::Ticket {
            id: 1,
            generation: 0,
        },
        html: format!(
            "<h1>{fragment_text}</h1><ul><li>one</li></ul><pre><code>let x = 1;</code></pre>"
        ),
        root: None,
    });
    let clipboard::Built::Ready { html, text, .. } = built else {
        panic!("not built");
    };
    let mut port = WindowsClipboard::new(owner.handle());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let written = loop {
        match clipboard::write(&mut port, &html, &text) {
            Written::Busy if std::time::Instant::now() < deadline => std::thread::yield_now(),
            other => break other,
        }
    };
    assert_eq!(written, Written::Both);

    let format = html_format().expect("HTML Format's number");
    let stored = read(owner.handle(), format).expect("HTML Format");
    let stored = &stored[..stored.iter().position(|&b| b == 0).unwrap_or(stored.len())];
    assert_eq!(stored, &html[..], "stored as built");
    let header =
        String::from_utf8_lossy(&stored[..offset(&String::from_utf8_lossy(stored), "StartHTML")])
            .into_owned();
    let (sh, eh) = (offset(&header, "StartHTML"), offset(&header, "EndHTML"));
    let (sf, ef) = (
        offset(&header, "StartFragment"),
        offset(&header, "EndFragment"),
    );
    assert_eq!(eh, stored.len());
    let fragment = std::str::from_utf8(&stored[sf..ef]).expect("the fragment is whole UTF-8");
    assert!(
        fragment.contains(fragment_text) && fragment.starts_with("<h1"),
        "{fragment}"
    );
    assert!(
        std::str::from_utf8(&stored[sh..eh])
            .unwrap()
            .contains("<!--StartFragment-->")
    );

    let unicode = read(owner.handle(), 13).expect("CF_UNICODETEXT");
    let units: Vec<u16> = unicode
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let back =
        String::from_utf16(&units[..units.iter().position(|&u| u == 0).unwrap_or(units.len())])
            .unwrap();
    assert_eq!(back, text);
    assert!(back.starts_with(fragment_text) && back.contains("\r\n"));
}
