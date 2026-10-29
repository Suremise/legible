//! The resources build.rs compiles into legible.exe: the manifest, every image of the app icon
//! and of the `.md` file icon, and the version info. rc.exe copies each of them into the exe unchanged, so each is found
//! by its bytes; nothing is run.
#![cfg(windows)]

use std::fs;
use std::path::Path;

fn exe() -> Vec<u8> {
    fs::read(env!("CARGO_BIN_EXE_legible")).unwrap()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn source(path: &str) -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap()
}

#[test]
fn the_manifest_is_embedded() {
    let manifest = source("res/legible.exe.manifest");
    let text = String::from_utf8_lossy(&manifest);
    assert!(text.contains(r#"name="Microsoft.Windows.Common-Controls" version="6.0.0.0""#));
    assert!(text.contains(r#"level="asInvoker""#));
    assert!(text.contains("PerMonitorV2"));
    assert!(contains(&exe(), &manifest), "the manifest isn't in the exe");
}

#[test]
fn every_size_of_the_app_icon_is_embedded() {
    every_size_is_embedded("../../assets/icon/legible.ico");
}

#[test]
fn every_size_of_the_file_icon_is_embedded() {
    every_size_is_embedded("../../assets/file/markdown.ico");
}

fn every_size_is_embedded(icon: &str) {
    let ico = source(icon);
    let count = usize::from(u16::from_le_bytes([ico[4], ico[5]]));
    let exe = exe();
    let mut sizes = Vec::new();
    for entry in ico[6..6 + count * 16].chunks(16) {
        // A width of 0 means 256.
        sizes.push(if entry[0] == 0 {
            256
        } else {
            u32::from(entry[0])
        });
        let len = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let at = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        assert!(
            contains(&exe, &ico[at..at + len]),
            "{icon}'s {} px image isn't in the exe",
            sizes.last().unwrap()
        );
    }
    sizes.sort_unstable();
    assert_eq!(sizes, [16, 20, 24, 32, 40, 48, 64, 256]);
}

#[test]
fn the_version_info_has_cargos_version_and_licences_copyright() {
    let exe = exe();
    let license = String::from_utf8(source("../../LICENSE")).unwrap();
    let copyright = license
        .lines()
        .find(|l| l.starts_with("Copyright"))
        .unwrap()
        .trim();
    for (name, value) in [
        ("ProductName", "Legible"),
        ("ProductVersion", env!("CARGO_PKG_VERSION")),
        ("LegalCopyright", copyright),
    ] {
        let mut entry = utf16(name);
        entry.extend([0, 0]);
        assert!(contains(&exe, &entry), "no {name} in the version info");
        assert!(
            contains(&exe, &utf16(value)),
            "no {value} in the version info"
        );
    }
}
