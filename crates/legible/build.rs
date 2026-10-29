//! Compiles Legible's Windows resources into the exe: the application manifest
//! (`res/legible.exe.manifest`), the app icon (`assets/icon/legible.ico`, made by
//! `assets/export.py`), the `.md` file icon (`assets/file/markdown.ico`, which the file
//! associations name), and the version info: Cargo's version, and LICENSE's copyright line and
//! holder, so Details shows them and the installer can compare the exe it replaces
//! (`scripts/package.ps1` checks it).

use std::env;
use std::fs;
use std::path::PathBuf;

/// The icon's resource ID: the lowest, so Explorer shows it for the exe. The window loads it by
/// this ID too (`legible_win::shell::APP_ICON`).
const APP_ICON: u16 = 1;
/// The `.md` file icon's resource ID (`legible_win::associations::FILE_ICON_ID`).
const FILE_ICON: u16 = 2;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by Cargo"));
    let manifest = crate_dir.join("res").join("legible.exe.manifest");
    let icon = crate_dir
        .join("..")
        .join("..")
        .join("assets")
        .join("icon")
        .join("legible.ico");
    let file_icon = crate_dir
        .join("..")
        .join("..")
        .join("assets")
        .join("file")
        .join("markdown.ico");
    let license = crate_dir.join("..").join("..").join("LICENSE");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed={}", file_icon.display());
    println!("cargo:rerun-if-changed={}", license.display());

    // LICENSE's own "Copyright (c) <year> <holder>" line, and its holder for CompanyName.
    let copyright = fs::read_to_string(&license)
        .expect("LICENSE")
        .lines()
        .find(|l| l.starts_with("Copyright"))
        .expect("LICENSE has a Copyright line")
        .trim()
        .to_owned();
    let holder = copyright
        .split_whitespace()
        .skip_while(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !holder.is_empty(),
        "LICENSE's Copyright line names its holder after the year"
    );

    let version = env::var("CARGO_PKG_VERSION").expect("set by Cargo");
    let numbers = ["MAJOR", "MINOR", "PATCH"]
        .map(|part| env::var(format!("CARGO_PKG_VERSION_{part}")).expect("set by Cargo"))
        .join(",");
    let rc = format!(
        r#"#pragma code_page(65001)
{APP_ICON} ICON "{icon}"
{FILE_ICON} ICON "{file_icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {numbers},0
PRODUCTVERSION {numbers},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "{holder}"
      VALUE "FileDescription", "Legible"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "legible"
      VALUE "LegalCopyright", "{copyright}"
      VALUE "OriginalFilename", "legible.exe"
      VALUE "ProductName", "Legible"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = rc_path(&icon),
        file_icon = rc_path(&file_icon),
        // rc.exe's strings write a quote as "".
        copyright = copyright.replace('"', "\"\""),
        holder = holder.replace('"', "\"\""),
        manifest = rc_path(&manifest),
    );
    let out = PathBuf::from(env::var("OUT_DIR").expect("set by Cargo")).join("legible.rc");
    fs::write(&out, rc).expect("write legible.rc");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_required()
        .expect("compile Legible's resources (needs the Windows SDK's rc.exe)");
    // And into the examples (icon-dpi checks the window icons legible.exe's windows get).
    embed_resource::compile_for_examples(&out, embed_resource::NONE)
        .manifest_required()
        .expect("compile Legible's resources for the examples");
}

/// A path as an .rc string: backslashes doubled.
fn rc_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}
