//! Legible's file associations, per user (brief §4.1): what the MSI registers
//! (`installer/legible.wxs`, which a test here keeps to the same values), and what a portable
//! copy registers when the user asks in Settings.
//!
//! Legible offers itself for `.md`, `.markdown` and `.mdown`: a ProgID (`Legible.Markdown`, with
//! the file icon and the open command) under each extension's `OpenWithProgids`,
//! `Applications\legible.exe` with its `SupportedTypes`, and its capabilities under
//! `RegisteredApplications`, which Default apps lists it by. It never sets itself as the
//! default: Windows keeps that the user's choice, and Legible only opens Default apps at its
//! own entry ([`DEFAULT_APPS`]). Unregistering removes only what Legible added: its own keys,
//! and its value under each extension's `OpenWithProgids` and `RegisteredApplications`.

use std::io;
use std::path::Path;

/// Legible's ProgID for Markdown files.
pub const PROG_ID: &str = "Legible.Markdown";
/// The extensions Legible opens.
pub const EXTENSIONS: [&str; 3] = [".md", ".markdown", ".mdown"];
/// Its name under `RegisteredApplications`, which Default apps' link names.
pub const REGISTERED_NAME: &str = "Legible";
/// Default apps, at Legible's own entry where Windows supports it (Windows 11; Windows 10 opens
/// Default apps).
pub const DEFAULT_APPS: &str = "ms-settings:defaultapps?registeredAppUser=Legible";
/// The resource ID of the file icon in `legible.exe` (`crates/legible/build.rs`).
pub const FILE_ICON_ID: u16 = 2;

const CLASSES: &str = r"Software\Classes";
const APPLICATION: &str = r"Software\Classes\Applications\legible.exe";
const CAPABILITIES: &str = r"Software\Legible\Capabilities";
const REGISTERED_APPLICATIONS: &str = r"Software\RegisteredApplications";

/// One registry value under HKEY_CURRENT_USER: its key, its name (None: the key's default
/// value) and its text (every value Legible writes is a string).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value {
    pub key: String,
    pub name: Option<String>,
    pub data: String,
}

fn value(key: impl Into<String>, name: Option<&str>, data: impl Into<String>) -> Value {
    Value {
        key: key.into(),
        name: name.map(str::to_owned),
        data: data.into(),
    }
}

/// The open command for `exe`.
pub fn open_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// Every value Legible registers for `exe`. `file_type` names Markdown files in Explorer ("Markdown
/// document"); `description` is Default apps' line about Legible.
pub fn values(exe: &Path, file_type: &str, description: &str) -> Vec<Value> {
    let prog_id = format!(r"{CLASSES}\{PROG_ID}");
    let command = open_command(exe);
    let icon = format!("\"{}\",-{FILE_ICON_ID}", exe.display());
    let mut values = vec![
        value(&prog_id, None, file_type),
        value(format!(r"{prog_id}\DefaultIcon"), None, &icon),
        value(format!(r"{prog_id}\shell\open\command"), None, &command),
        value(APPLICATION, Some("FriendlyAppName"), "Legible"),
        value(format!(r"{APPLICATION}\DefaultIcon"), None, &icon),
        value(format!(r"{APPLICATION}\shell\open\command"), None, &command),
        value(CAPABILITIES, Some("ApplicationName"), "Legible"),
        value(CAPABILITIES, Some("ApplicationDescription"), description),
        value(REGISTERED_APPLICATIONS, Some(REGISTERED_NAME), CAPABILITIES),
    ];
    for ext in EXTENSIONS {
        values.push(value(
            format!(r"{CLASSES}\{ext}\OpenWithProgids"),
            Some(PROG_ID),
            "",
        ));
        values.push(value(
            format!(r"{APPLICATION}\SupportedTypes"),
            Some(ext),
            "",
        ));
        values.push(value(
            format!(r"{CAPABILITIES}\FileAssociations"),
            Some(ext),
            PROG_ID,
        ));
    }
    values
}

/// The keys that are Legible's alone: unregistering removes them whole.
pub const OWNED_KEYS: [&str; 3] = [
    r"Software\Classes\Legible.Markdown",
    APPLICATION,
    CAPABILITIES,
];

/// The registry, as registering needs it (HKEY_CURRENT_USER, or a test's own hive).
pub trait Registry {
    /// A string value, or None if it isn't there.
    fn read(&self, key: &str, name: Option<&str>) -> Option<String>;
    /// Writes a string value, making its key.
    fn write(&mut self, value: &Value) -> io::Result<()>;
    /// Removes a value; one that isn't there is fine.
    fn delete_value(&mut self, key: &str, name: Option<&str>) -> io::Result<()>;
    /// Removes a key and everything under it; one that isn't there is fine.
    fn delete_tree(&mut self, key: &str) -> io::Result<()>;
    /// Removes a key if nothing is left in it; otherwise leaves it.
    fn delete_if_empty(&mut self, key: &str);
}

/// Whether every value in `values` is registered as it is there (for this exe; paths compare
/// as Windows compares them, whatever their case).
pub fn is_registered(registry: &impl Registry, values: &[Value]) -> bool {
    values.iter().all(|v| {
        registry
            .read(&v.key, v.name.as_deref())
            .is_some_and(|data| data.to_lowercase() == v.data.to_lowercase())
    })
}

/// Registers `values`.
pub fn register(registry: &mut impl Registry, values: &[Value]) -> io::Result<()> {
    values.iter().try_for_each(|v| registry.write(v))
}

/// Removes what Legible registered, and nothing else: its own keys, and its values in keys it
/// shares with other apps (each extension's `OpenWithProgids`, `RegisteredApplications`).
pub fn unregister(registry: &mut impl Registry, values: &[Value]) -> io::Result<()> {
    let owned = |key: &str| {
        OWNED_KEYS
            .iter()
            .any(|o| key == *o || key.starts_with(&format!("{o}\\")))
    };
    for v in values.iter().filter(|v| !owned(&v.key)) {
        registry.delete_value(&v.key, v.name.as_deref())?;
    }
    for key in OWNED_KEYS {
        registry.delete_tree(key)?;
    }
    // Legible's own key, if the capabilities were all that was in it.
    registry.delete_if_empty(r"Software\Legible");
    Ok(())
}

/// Whether `handler` (the exe that opens `.md` now, if any) is `exe`.
pub fn is_default(handler: Option<&Path>, exe: &Path) -> bool {
    let plain = |p: &Path| {
        let p = p.to_string_lossy();
        p.strip_prefix(r"\\?\")
            .unwrap_or(&p)
            .replace('/', "\\")
            .to_lowercase()
    };
    handler.is_some_and(|h| plain(h) == plain(exe))
}

/// What the page shows about the default app: Settings' part, and the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultApp {
    /// Legible is registered for `.md` at this exe.
    pub registered: bool,
    /// It opens `.md` files now.
    pub is_default: bool,
    /// A portable copy (it registers only when asked, and can unregister).
    pub portable: bool,
    /// The user closed the offer for now: not offered again until Legible starts again.
    pub closed: bool,
}

impl DefaultApp {
    /// Whether to offer making Legible the default: it's registered (so Default apps lists it)
    /// but isn't the default, and the offer wasn't closed this time.
    pub fn offer(&self) -> bool {
        self.registered && !self.is_default && !self.closed
    }

    /// For the page: `{offer, registered, isDefault, portable}`.
    pub fn to_json(&self) -> String {
        format!(
            r#"{{"offer":{},"registered":{},"isDefault":{},"portable":{}}}"#,
            self.offer(),
            self.registered,
            self.is_default,
            self.portable
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A registry in memory: (key, name) to text.
    #[derive(Default)]
    struct Memory(BTreeMap<(String, String), String>);
    impl Registry for Memory {
        fn read(&self, key: &str, name: Option<&str>) -> Option<String> {
            self.0
                .get(&(key.to_lowercase(), name.unwrap_or("").to_lowercase()))
                .cloned()
        }
        fn write(&mut self, v: &Value) -> io::Result<()> {
            self.0.insert(
                (
                    v.key.to_lowercase(),
                    v.name.clone().unwrap_or_default().to_lowercase(),
                ),
                v.data.clone(),
            );
            Ok(())
        }
        fn delete_value(&mut self, key: &str, name: Option<&str>) -> io::Result<()> {
            self.0
                .remove(&(key.to_lowercase(), name.unwrap_or("").to_lowercase()));
            Ok(())
        }
        fn delete_tree(&mut self, key: &str) -> io::Result<()> {
            let key = key.to_lowercase();
            self.0
                .retain(|(k, _), _| *k != key && !k.starts_with(&format!("{key}\\")));
            Ok(())
        }
        fn delete_if_empty(&mut self, _: &str) {}
    }

    const EXE: &str = r"C:\Users\Ann\AppData\Local\Programs\Legible\legible.exe";

    #[test]
    fn it_offers_itself_for_each_extension_without_taking_the_default() {
        let values = values(Path::new(EXE), "Markdown document", "Reads Markdown");
        let has = |key: &str, name: Option<&str>, data: &str| {
            values
                .iter()
                .any(|v| v.key == key && v.name.as_deref() == name && v.data == data)
        };
        let command = format!("\"{EXE}\" \"%1\"");
        assert!(has(
            r"Software\Classes\Legible.Markdown\shell\open\command",
            None,
            &command
        ));
        assert!(has(
            r"Software\Classes\Legible.Markdown",
            None,
            "Markdown document"
        ));
        assert!(has(
            r"Software\Classes\Legible.Markdown\DefaultIcon",
            None,
            &format!("\"{EXE}\",-2")
        ));
        for ext in [".md", ".markdown", ".mdown"] {
            assert!(has(
                &format!(r"Software\Classes\{ext}\OpenWithProgids"),
                Some("Legible.Markdown"),
                ""
            ));
            assert!(has(
                r"Software\Classes\Applications\legible.exe\SupportedTypes",
                Some(ext),
                ""
            ));
        }
        // Never the extension's own default, nor the user's choice.
        assert!(!values.iter().any(|v| {
            EXTENSIONS
                .iter()
                .any(|ext| v.key == format!(r"Software\Classes\{ext}") && v.name.is_none())
                || v.key.contains("UserChoice")
        }));
    }

    #[test]
    fn unregistering_removes_only_what_legible_added() {
        let mut registry = Memory::default();
        // Another app's, in the keys Legible shares.
        let others = [
            value(
                r"Software\Classes\.md\OpenWithProgids",
                Some("Other.md"),
                "",
            ),
            value(r"Software\Classes\.md", None, "Other.md"),
            value(
                r"Software\RegisteredApplications",
                Some("Other"),
                r"Software\Other\Caps",
            ),
        ];
        for v in &others {
            registry.write(v).unwrap();
        }
        let ours = values(Path::new(EXE), "Markdown document", "Reads Markdown");
        assert!(!is_registered(&registry, &ours));
        register(&mut registry, &ours).unwrap();
        assert!(is_registered(&registry, &ours));
        // Another copy's exe isn't this one's registration.
        let elsewhere = values(
            Path::new(r"D:\Legible\legible.exe"),
            "Markdown document",
            "Reads Markdown",
        );
        assert!(!is_registered(&registry, &elsewhere));

        unregister(&mut registry, &ours).unwrap();
        let left: Vec<_> = registry.0.values().cloned().collect();
        assert_eq!(left.len(), others.len(), "{left:?}");
        for v in &others {
            assert_eq!(
                registry.read(&v.key, v.name.as_deref()).as_deref(),
                Some(v.data.as_str())
            );
        }
    }

    #[test]
    fn the_default_is_this_exe_however_its_path_is_written() {
        let exe = Path::new(EXE);
        assert!(is_default(Some(Path::new(&EXE.to_uppercase())), exe));
        assert!(is_default(Some(Path::new(&format!(r"\\?\{EXE}"))), exe));
        assert!(!is_default(Some(Path::new(r"C:\Windows\notepad.exe")), exe));
        assert!(!is_default(None, exe));
    }

    #[test]
    fn it_offers_the_default_only_once_registered_and_not_already() {
        let state = |registered, is_default| DefaultApp {
            registered,
            is_default,
            portable: false,
            closed: false,
        };
        assert!(state(true, false).offer());
        assert!(!state(true, true).offer());
        assert!(
            !state(false, false).offer(),
            "Default apps wouldn't list it"
        );
        let closed = DefaultApp {
            closed: true,
            ..state(true, false)
        };
        assert!(!closed.offer(), "offered again after Close");
        assert_eq!(
            state(true, false).to_json(),
            r#"{"offer":true,"registered":true,"isDefault":false,"portable":false}"#
        );
    }

    /// The MSI registers what a portable copy does: every `RegistryValue` in the .wxs outside
    /// Legible's own `Software\Legible\Installer` key is one of [`values`], with the install
    /// folder's exe, and none is missing.
    #[test]
    fn the_installer_registers_the_same_values() {
        let wxs = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installer/legible.wxs"),
        )
        .unwrap();
        let attr = |tag: &str, name: &str| -> Option<String> {
            let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
            let end = start + tag[start..].find('"')?;
            Some(tag[start..end].replace("&quot;", "\""))
        };
        let exe = Path::new(r"[INSTALLFOLDER]legible.exe");
        let expected = values(exe, "$(var.FileType)", "$(var.AppDescription)");
        let mut found = Vec::new();
        for tag in wxs.split("<RegistryValue").skip(1) {
            let tag = &tag[..tag.find("/>").unwrap()];
            let key = attr(tag, "Key").unwrap();
            if key.starts_with(r"Software\Legible\Installer") {
                continue;
            }
            assert_eq!(attr(tag, "Root").as_deref(), Some("HKCU"), "{tag}");
            assert_eq!(attr(tag, "Type").as_deref(), Some("string"), "{tag}");
            found.push(Value {
                key,
                name: attr(tag, "Name"),
                data: attr(tag, "Value").unwrap_or_default(),
            });
        }
        // The installer gives the exe's path as `[INSTALLFOLDER]legible.exe`, which `values`
        // wrote as it is.
        for v in &expected {
            assert!(found.contains(v), "the installer doesn't register {v:?}");
        }
        for v in &found {
            assert!(expected.contains(v), "the installer registers more: {v:?}");
        }
    }
}
