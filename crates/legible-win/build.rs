//! Compiles legible.exe's application manifest (`crates/legible/res/legible.exe.manifest`) into
//! this crate's test and example exes, so the window's tests (`tests/shell.rs`) and the examples
//! that run the real window (`examples/readme-shots.rs`) run as legible.exe does:
//! common controls 6 (the close question's task dialog is only there), per-monitor DPI
//! awareness, and UTF-8 as the ANSI code page. The examples also get the app icon, as resource 1
//! like the exe's, so their windows show it in the title bar; the tests don't (a test checks a
//! window without it). The library itself gets nothing: the exe that links it brings its own
//! resources.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by Cargo"));
    let manifest = crate_dir
        .join("..")
        .join("legible")
        .join("res")
        .join("legible.exe.manifest");
    let icon = crate_dir
        .join("..")
        .join("..")
        .join("assets")
        .join("icon")
        .join("legible.ico");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", icon.display());
    let out = PathBuf::from(env::var("OUT_DIR").expect("set by Cargo"));

    let tests = out.join("tests.rc");
    fs::write(
        &tests,
        format!(
            "#pragma code_page(65001)\n1 24 \"{}\"\n",
            rc_path(&manifest)
        ),
    )
    .expect("write tests.rc");
    embed_resource::compile_for_tests(&tests, embed_resource::NONE)
        .manifest_required()
        .expect("compile the manifest for the tests (needs the Windows SDK's rc.exe)");

    let examples = out.join("examples.rc");
    fs::write(
        &examples,
        format!(
            "#pragma code_page(65001)\n1 24 \"{}\"\n1 ICON \"{}\"\n",
            rc_path(&manifest),
            rc_path(&icon)
        ),
    )
    .expect("write examples.rc");
    embed_resource::compile_for_examples(&examples, embed_resource::NONE)
        .manifest_required()
        .expect("compile the manifest and icon for the examples");
}

/// A path as an .rc string: backslashes doubled.
fn rc_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}
