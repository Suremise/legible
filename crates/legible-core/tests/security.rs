//! The security fixtures: whatever a document holds, what reaches the view can't run script,
//! load anything from the network or reach files outside the document's folder.

use std::path::{Path, PathBuf};

use legible_core::render::render;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn nasty() -> String {
    std::fs::read_to_string(fixtures().join("security.md")).unwrap()
}

#[test]
fn nothing_can_run_script() {
    let html = render(&nasty(), &fixtures()).html.to_ascii_lowercase();
    for banned in [
        "<script",
        "javascript:",
        "onerror",
        "onload",
        "onclick",
        "<iframe",
        "<svg",
        "<style",
        "display: none",
        "<meta",
        "http-equiv",
        "<form",
        "password",
        "style=",
        "alert(",
    ] {
        assert!(!html.contains(banned), "{banned:?} survived:\n{html}");
    }
}

#[test]
fn nothing_loads_from_the_network() {
    let doc = render(&nasty(), &fixtures());
    assert!(doc.has_remote_images);
    assert!(!doc.html.contains("evil.example/frame"));
    assert!(!doc.html.contains("evil.example/\""), "the refresh target");
    // The remote image waits, with no src.
    assert!(
        doc.html
            .contains("<img data-remote-src=\"https://evil.example/track.gif\" alt=\"remote\">"),
        "{}",
        doc.html
    );
    for (i, _) in doc.html.match_indices(" src=\"") {
        let value = &doc.html[i + 6..];
        assert!(
            value.starts_with("https://legible-doc.localhost/"),
            "a src off the document base: {value:.60}"
        );
    }
}

#[test]
fn a_file_link_outside_the_folder_isn_t_clickable() {
    let html = render(&nasty(), &fixtures()).html;
    assert!(!html.contains("file:"), "{html}");
    assert!(
        html.contains("<a rel=\"noopener noreferrer\">a file outside the folder</a>"),
        "{html}"
    );
}

#[test]
fn every_link_gets_noopener() {
    let doc = render(
        "[a](https://example.com) <a href=\"https://example.com\" rel=\"opener\">b</a> <https://example.com/c>",
        &fixtures(),
    );
    assert_eq!(doc.html.matches("<a ").count(), 3, "{}", doc.html);
    assert_eq!(
        doc.html.matches("rel=\"noopener noreferrer\"").count(),
        3,
        "{}",
        doc.html
    );
    assert!(!doc.html.contains("rel=\"opener\""));
}
