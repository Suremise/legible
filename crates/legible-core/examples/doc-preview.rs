//! Renders Markdown files for `scripts/doc-shots.mjs`, which shows them in Edge to take the
//! document's screenshots.
//!
//! `cargo run -p legible-core --example doc-preview -- <out dir> <doc base> <file.md>...` writes
//! each file's rendered HTML to `<out dir>/<name>.html`, with its images served at `<doc base>`,
//! and the script's strings to `<out dir>/strings.json`.

use std::path::Path;

use legible_core::render::{RenderOptions, render_with, script_strings};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out, doc_base, files @ ..] = args.as_slice() else {
        eprintln!("usage: doc-preview <out dir> <doc base> <file.md>...");
        std::process::exit(2);
    };
    let out = Path::new(out);
    std::fs::create_dir_all(out).expect("the output folder");
    let options = RenderOptions {
        doc_base: doc_base.clone(),
        ..RenderOptions::default()
    };
    for file in files {
        let path = Path::new(file);
        let markdown = std::fs::read_to_string(path).expect(file);
        let folder = path.parent().unwrap_or(Path::new("."));
        let rendered = render_with(&markdown, folder, &options);
        let name = path.file_stem().expect(file).to_string_lossy();
        std::fs::write(out.join(format!("{name}.html")), rendered.html).expect("write");
    }
    std::fs::write(out.join("strings.json"), script_strings()).expect("write");
}
