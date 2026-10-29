//! Golden tests: each `tests/fixtures/<name>.md` renders to exactly `<name>.html`, which holds
//! the flags and the outline (as comments) and then the HTML.
//!
//! After a change to the rendering on purpose, regenerate them and review the diff:
//!
//! ```powershell
//! $env:LEGIBLE_REGENERATE_FIXTURES = "1"; cargo nextest run -p legible-core --test golden; Remove-Item Env:LEGIBLE_REGENERATE_FIXTURES
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use legible_core::render::{Rendered, render};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// What a fixture's expected file holds.
fn golden(rendered: &Rendered) -> String {
    let mut out = format!(
        "<!-- has_remote_images: {}, has_mermaid: {} -->\n",
        rendered.has_remote_images, rendered.has_mermaid
    );
    for h in &rendered.outline {
        out.push_str(&format!("<!-- h{} {} | {} -->\n", h.level, h.id, h.text));
    }
    out.push_str(&rendered.html);
    out
}

#[test]
fn every_fixture_renders_as_its_golden_file() {
    let regenerate = std::env::var_os("LEGIBLE_REGENERATE_FIXTURES").is_some();
    let mut checked = 0;
    let mut differ = Vec::new();
    for entry in fs::read_dir(fixtures()).unwrap() {
        let md = entry.unwrap().path();
        if md.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let actual = golden(&render(&fs::read_to_string(&md).unwrap(), &fixtures()));
        let expected_file = md.with_extension("html");
        if regenerate {
            fs::write(&expected_file, &actual).unwrap();
        } else {
            let expected = fs::read_to_string(&expected_file).unwrap_or_default();
            if expected != actual {
                differ.push(format!(
                    "{} differs from {}:\n{actual}",
                    md.display(),
                    expected_file.display()
                ));
            }
        }
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} fixtures");
    assert!(differ.is_empty(), "{}", differ.join("\n\n"));
}

/// Every `#fragment` link a fixture renders has its target: an element with that id. (The
/// headings fixture's link to `#no-such-heading` is the one that mustn't, and doesn't.)
#[test]
fn every_fragment_link_finds_its_target() {
    for entry in fs::read_dir(fixtures()).unwrap() {
        let md = entry.unwrap().path();
        if md.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let html = render(&fs::read_to_string(&md).unwrap(), &fixtures()).html;
        for (i, _) in html.match_indices("href=\"#") {
            let target = &html[i + 7..];
            let target = &target[..target.find('"').unwrap()];
            let found = html.contains(&format!(" id=\"{target}\""));
            let expected = target != "user-content-no-such-heading";
            assert_eq!(found, expected, "{}: #{target}", md.display());
        }
    }
}

/// A 2 MB document: every fixture but the nasty one, repeated. It must render whole; the time
/// is printed (`--no-capture`), and recorded in docs/ARCHITECTURE.md from a release run.
#[test]
fn a_2_mb_document_renders() {
    let mut sample = String::new();
    for name in ["gfm", "alerts", "headings", "mermaid", "images"] {
        sample.push_str(&fs::read_to_string(fixtures().join(format!("{name}.md"))).unwrap());
        sample.push_str("\n\n");
    }
    let copies = 2_000_000 / sample.len() + 1;
    let big = sample.repeat(copies);
    assert!(big.len() >= 2_000_000);

    let start = Instant::now();
    let rendered = render(&big, &fixtures());
    let elapsed = start.elapsed();
    println!(
        "rendered {} bytes of Markdown to {} bytes of HTML in {elapsed:?}",
        big.len(),
        rendered.html.len()
    );
    let small = render(&sample, &fixtures());
    assert_eq!(rendered.outline.len(), small.outline.len() * copies);
    assert!(rendered.has_mermaid && rendered.has_remote_images);
    // Ids stay unique however many times a heading repeats.
    let ids: std::collections::HashSet<_> = rendered.outline.iter().map(|h| &h.id).collect();
    assert_eq!(ids.len(), rendered.outline.len());
}
