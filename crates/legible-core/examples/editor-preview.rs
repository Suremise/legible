//! Renders Markdown as the editor's preview gets it, for `scripts/chrome-tests.mjs`, whose
//! stand-in for the window renders each preview through core with it.
//!
//! `cargo run -p legible-core --example editor-preview -- <doc base>` reads Markdown on stdin
//! and writes its HTML, each block marked with its lines (`RenderOptions::sourcepos`), to
//! stdout.

use std::io::{Read, Write};
use std::path::Path;

use legible_core::render::{RenderOptions, render_with};

fn main() {
    let doc_base = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/docfiles/".to_owned());
    let mut markdown = String::new();
    std::io::stdin()
        .read_to_string(&mut markdown)
        .expect("Markdown on stdin");
    let options = RenderOptions {
        doc_base,
        sourcepos: true,
    };
    let rendered = render_with(&markdown, Path::new("."), &options);
    std::io::stdout()
        .write_all(rendered.html.as_bytes())
        .expect("write");
}
