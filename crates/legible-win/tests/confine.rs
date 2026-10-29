//! The `legible-doc` protocol serves only from inside the document's folder: traversal,
//! junctions and symlinks out, and UNC paths are all refused.
#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use legible_win::confine::{DocRoot, Refusal};

struct Folders {
    _temp: tempfile::TempDir,
    /// `<temp>\doc`: the document's folder.
    doc: PathBuf,
    /// `<temp>\outside`: a folder beside it.
    outside: PathBuf,
}

fn folders() -> Folders {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let (doc, outside) = (base.join("doc"), base.join("outside"));
    fs::create_dir_all(doc.join("images")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(doc.join("images").join("in.png"), b"in").unwrap();
    fs::write(doc.join("a b.jpg"), b"in").unwrap();
    fs::write(doc.join("notes.md"), b"# notes").unwrap();
    fs::write(outside.join("secret.png"), b"out").unwrap();
    fs::write(base.join("secret.png"), b"out").unwrap();
    Folders {
        _temp: temp,
        doc,
        outside,
    }
}

/// A junction at `link` to `target`, as `mklink /J` makes it (no rights needed).
fn junction(link: &Path, target: &Path) {
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(output.status.success(), "mklink /J: {output:?}");
}

#[test]
fn files_inside_the_folder_are_served() {
    let f = folders();
    let root = DocRoot::new(&f.doc).unwrap();
    let (file, kind) = root.resolve("/images/in.png").unwrap();
    assert_eq!(
        file,
        fs::canonicalize(f.doc.join("images").join("in.png")).unwrap()
    );
    assert_eq!(kind, "image/png");
    assert_eq!(root.resolve("/a%20b.jpg").unwrap().1, "image/jpeg");
}

#[test]
fn traversal_is_refused() {
    let f = folders();
    let root = DocRoot::new(&f.doc).unwrap();
    for path in [
        "/../secret.png",
        "/images/../../secret.png",
        "/%2e%2e/secret.png",
        "/%2E%2E%2Fsecret.png",
        "/..%5Csecret.png",
        r"/..\secret.png",
        "/../outside/secret.png",
    ] {
        assert_eq!(root.resolve(path), Err(Refusal::BadPath), "{path}");
    }
    let absolute = format!("/{}", f.outside.join("secret.png").display());
    assert_eq!(root.resolve(&absolute), Err(Refusal::BadPath));
}

#[test]
fn a_junction_out_of_the_folder_is_refused() {
    let f = folders();
    junction(&f.doc.join("escape"), &f.outside);
    // One that stays inside is fine.
    junction(&f.doc.join("pictures"), &f.doc.join("images"));
    let root = DocRoot::new(&f.doc).unwrap();
    assert_eq!(root.resolve("/escape/secret.png"), Err(Refusal::Outside));
    assert!(root.resolve("/pictures/in.png").is_ok());
}

#[test]
fn a_symlink_out_of_the_folder_is_refused() {
    let f = folders();
    let link = f.doc.join("linked.png");
    // Symlinks need Developer Mode or an elevated process; junctions (above) don't.
    if let Err(error) = std::os::windows::fs::symlink_file(f.outside.join("secret.png"), &link) {
        eprintln!("skipped: this account can't make symlinks ({error})");
        return;
    }
    let root = DocRoot::new(&f.doc).unwrap();
    assert_eq!(root.resolve("/linked.png"), Err(Refusal::Outside));
}

#[test]
fn unc_paths_in_a_request_are_refused() {
    let f = folders();
    let root = DocRoot::new(&f.doc).unwrap();
    for path in [
        "//localhost/C$/Windows/win.ini",
        "/%2F%2Flocalhost/C$/x.png",
        r"/\\localhost\C$\x.png",
        "/%5C%5Clocalhost%5CC$%5Cx.png",
        "/%5C%5C%3F%5CC:%5Cx.png",
    ] {
        assert_eq!(root.resolve(path), Err(Refusal::BadPath), "{path}");
    }
}

#[test]
fn a_document_on_a_share_is_confined_to_its_folder_there() {
    let f = folders();
    // The same folder by its administrative share, \\localhost\C$\…, where that's available.
    let local = f.doc.display().to_string();
    let local = local.trim_start_matches(r"\\?\");
    let Some((drive, rest)) = local.split_once(":\\") else {
        return;
    };
    let unc = PathBuf::from(format!(r"\\localhost\{drive}$\{rest}"));
    let Ok(root) = DocRoot::new(&unc) else {
        eprintln!("skipped: {} isn't reachable", unc.display());
        return;
    };
    assert!(root.path().display().to_string().starts_with(r"\\?\UNC\"));
    assert!(root.resolve("/images/in.png").is_ok());
    assert_eq!(root.resolve("/../secret.png"), Err(Refusal::BadPath));
    junction(&f.doc.join("escape"), &f.outside);
    assert_eq!(root.resolve("/escape/secret.png"), Err(Refusal::Outside));
}

#[test]
fn only_images_that_are_there_are_served() {
    let f = folders();
    fs::create_dir(f.doc.join("folder.png")).unwrap();
    let root = DocRoot::new(&f.doc).unwrap();
    assert_eq!(root.resolve("/notes.md"), Err(Refusal::NotAnImage));
    assert_eq!(root.resolve("/missing.png"), Err(Refusal::NotFound));
    assert_eq!(root.resolve("/folder.png"), Err(Refusal::NotFound));
    assert_eq!(root.resolve("/"), Err(Refusal::BadPath));
}

#[test]
fn live_reload_watches_pictures_by_the_same_rules_present_or_not() {
    let f = folders();
    junction(&f.doc.join("escape"), &f.outside);
    let root = DocRoot::new(&f.doc).unwrap();
    // Missing now, but a picture inside the folder: watched, and held once it's there.
    let later = root.candidate("/later.png").unwrap();
    assert!(!root.holds(&later));
    fs::write(&later, b"png").unwrap();
    assert!(root.holds(&later));
    // A junction out is a candidate by its path, but never held.
    let out = root.candidate("/escape/secret.png").unwrap();
    assert!(!root.holds(&out));
    // The path rules, and images only.
    for bad in [
        "/../secret.png",
        "/a/%2e%2e/secret.png",
        "/images/..%2fx.png",
        "//server/x.png",
        "/c:/x.png",
    ] {
        assert_eq!(root.candidate(bad), None, "{bad}");
    }
    assert_eq!(root.candidate("/notes.md"), None);
}

#[test]
fn a_generation_comes_off_once_and_the_rest_is_checked_as_usual() {
    use legible_win::confine::without_generation;
    let f = folders();
    let root = DocRoot::new(&f.doc).unwrap();
    assert_eq!(without_generation("/~3/images/in.png"), "/images/in.png");
    assert!(
        root.resolve(without_generation("/~3/images/in.png"))
            .is_ok()
    );
    assert_eq!(without_generation("/images/in.png"), "/images/in.png");
    assert_eq!(without_generation("/~x/in.png"), "/~x/in.png");
    // Only one comes off: a second is an ordinary part of the path, and isn't there.
    assert_eq!(without_generation("/~3/~4/a.png"), "/~4/a.png");
    assert_eq!(
        root.resolve(without_generation("/~3/~4/a.png")),
        Err(Refusal::NotFound)
    );
    // Encoded traversal after a generation is refused before the disk.
    for bad in [
        "/~3/..%2fsecret.png",
        "/~3/%2e%2e/secret.png",
        "/~3//server/x.png",
    ] {
        assert_eq!(
            root.resolve(without_generation(bad)),
            Err(Refusal::BadPath),
            "{bad}"
        );
    }
}
