//! Golden tests for the clipboard: each fixture renders, then copies, to exactly
//! `tests/fixtures/clipboard/<name>.html` and `<name>.txt`. Besides the Markdown fixtures,
//! `prepared.input.html` is a document as the view holds it once `web/doc/document.js` has run
//! (table frames, highlighted code, a Copy button, placeholders, a drawn diagram).
//!
//! After a change on purpose, regenerate them and review the diff:
//!
//! ```powershell
//! $env:LEGIBLE_REGENERATE_FIXTURES = "1"; cargo nextest run -p legible-core --test clipboard; Remove-Item Env:LEGIBLE_REGENERATE_FIXTURES
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use legible_core::clipboard_html::{Copied, Limits, build};
use legible_core::render::render;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// The document's one picture that's there: a 1200 × 600 PNG header, for the width cap.
fn images(src: &str) -> Option<Vec<u8>> {
    (src == "https://legible-doc.localhost/img/chart.png").then(|| {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&1200u32.to_be_bytes());
        png.extend_from_slice(&600u32.to_be_bytes());
        png
    })
}

fn copy(html: &str) -> Copied {
    build(html, &images, &Limits::default())
}

/// Each fixture's input HTML, by name.
fn inputs() -> Vec<(String, String)> {
    let mut inputs = Vec::new();
    for entry in fs::read_dir(fixtures()).unwrap() {
        let md = entry.unwrap().path();
        if md.extension().is_some_and(|e| e == "md") {
            let name = md.file_stem().unwrap().to_string_lossy().into_owned();
            let html = render(&fs::read_to_string(&md).unwrap(), &fixtures()).html;
            inputs.push((name, html));
        }
    }
    let prepared = fixtures().join("clipboard").join("prepared.input.html");
    inputs.push(("prepared".into(), fs::read_to_string(prepared).unwrap()));
    inputs
}

#[test]
fn every_fixture_copies_as_its_golden_files() {
    let regenerate = std::env::var_os("LEGIBLE_REGENERATE_FIXTURES").is_some();
    let dir = fixtures().join("clipboard");
    let mut differ = Vec::new();
    let inputs = inputs();
    assert!(inputs.len() >= 9, "only {} fixtures", inputs.len());
    for (name, html) in inputs {
        let copied = copy(&html);
        for (ext, actual) in [("html", &copied.html), ("txt", &copied.text)] {
            let file = dir.join(format!("{name}.{ext}"));
            if regenerate {
                fs::write(&file, actual).unwrap();
            } else if fs::read_to_string(&file).unwrap_or_default() != *actual {
                differ.push(format!("{} differs:\n{actual}", file.display()));
            }
        }
    }
    assert!(differ.is_empty(), "{}", differ.join("\n\n"));
}

/// Nothing in any copy depends on a stylesheet or can run.
#[test]
fn no_copy_keeps_styles_scripts_or_classes() {
    for (name, html) in inputs() {
        let copied = copy(&html);
        let lower = copied.html.to_ascii_lowercase();
        for banned in [
            "<style",
            "<script",
            "<svg",
            "<button",
            "<input",
            "<details",
            "class=",
            " id=",
            "javascript:",
            "var(--",
        ] {
            assert!(!lower.contains(banned), "{name}: {banned}");
        }
        for prefix in [" on", " data-"] {
            assert!(
                !has_attribute(&lower, prefix),
                "{name}: a{prefix}… attribute"
            );
        }
        for tag in ["<p", "<div", "<span", "<table", "<td", "<br", "<img"] {
            assert!(!copied.text.contains(tag), "{name}: {tag} in the text");
        }
    }
}

/// Whether `html` has an attribute whose name starts with `prefix` (" on", " data-"): the
/// prefix, then letters or hyphens, then `=`.
fn has_attribute(html: &str, prefix: &str) -> bool {
    html.match_indices(prefix).any(|(i, _)| {
        let rest = &html[i + prefix.len()..];
        let name_len = rest
            .find(|c: char| !(c.is_ascii_alphabetic() || c == '-'))
            .unwrap_or(rest.len());
        name_len > 0 && rest[name_len..].starts_with('=')
    })
}

/// Every picture embedded is a data: URI, and none comes from the network.
#[test]
fn pictures_are_embedded_or_named() {
    for (name, html) in inputs() {
        let copied = copy(&html);
        for (i, _) in copied.html.match_indices("<img ") {
            assert!(
                copied.html[i..].starts_with("<img src=\"data:image/"),
                "{name}: {}",
                &copied.html[i..i + 60.min(copied.html.len() - i)]
            );
        }
    }
}

/// A document nested deeper than the stack could follow copies whole. It runs in a child
/// process (this test binary again, for this test alone), so a stack overflow fails this test
/// rather than ending the whole run.
#[test]
fn deeply_nested_documents_copy_without_overflowing() {
    const CHILD: &str = "LEGIBLE_DEEP_NESTING_CHILD";
    if std::env::var_os(CHILD).is_some() {
        for depth in [1_000, 4_000] {
            let markdown = format!("{}text{}", "<div>".repeat(depth), "</div>".repeat(depth));
            let rendered = render(&markdown, Path::new("C:/docs"));
            let copied = copy(&rendered.html);
            assert!(copied.text.contains("text"), "depth {depth}");
        }
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "deeply_nested_documents_copy_without_overflowing",
            "--exact",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the child failed ({}):\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // It ran the test, rather than finding nothing to run.
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}
