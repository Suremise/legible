//! Writes the window's real page for Markdown files, for `scripts/chrome-tests.mjs`, which
//! tests the chrome in Edge.
//!
//! `cargo run -p legible-win --example chrome-page -- <out dir> <file.md>...` writes each
//! file's page, exactly as a window gets it (`legible_win::page::page`, with the default
//! settings), to `<out dir>/<name>.html`, and the page of a window with no document to
//! `<out dir>/empty.html`, and the first file's page as served again (generation 1) to
//! `<out dir>/again.html`. A file whose name ends in `-read-only.md` gets the page of a file
//! that can't be edited where it is.

#[cfg(windows)]
fn main() {
    use std::path::Path;

    use legible_core::render::render;
    use legible_win::page;
    use legible_win::settings::Settings;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out, files @ ..] = args.as_slice() else {
        eprintln!("usage: chrome-page <out dir> <file.md>...");
        std::process::exit(2);
    };
    legible_core::i18n::init("en");
    let out = Path::new(out);
    std::fs::create_dir_all(out).expect("the output folder");
    for file in files {
        let path = Path::new(file);
        let markdown = std::fs::read_to_string(path).expect(file);
        let folder = path.parent().unwrap_or(Path::new("."));
        let name = path.file_stem().expect(file).to_string_lossy();
        let html = page::document_page(
            "en",
            &name,
            &render(&markdown, folder).html,
            &Settings::default(),
            0,
            &[],
            !name.ends_with("-read-only"),
        );
        std::fs::write(out.join(format!("{name}.html")), html).expect("write");
    }
    // The first file's page again, as a window serves it again (its next generation).
    if let Some(first) = files.first() {
        let path = Path::new(first);
        let markdown = std::fs::read_to_string(path).expect(first);
        let folder = path.parent().unwrap_or(Path::new("."));
        let html = page::document_page(
            "en",
            "again",
            &render(&markdown, folder).html,
            &Settings::default(),
            1,
            &[],
            true,
        );
        std::fs::write(out.join("again.html"), html).expect("write");
    }
    let empty = page::empty_page("en", "Legible", "No document", &Settings::default());
    std::fs::write(out.join("empty.html"), empty).expect("write");
}

#[cfg(not(windows))]
fn main() {}
