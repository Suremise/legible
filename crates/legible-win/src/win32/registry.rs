//! The registry for [`associations`](crate::associations): HKEY_CURRENT_USER, or a hive of a
//! test's own (a file in a temp folder, loaded for this process only, so tests never touch the
//! user's registry). And telling Explorer associations changed, and what opens `.md` now.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, KEY_READ, KEY_SET_VALUE,
    REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteKeyValueW, RegDeleteKeyW, RegDeleteTreeW, RegGetValueW, RegLoadAppKeyW, RegOpenKeyExW,
    RegSetKeyValueW,
};
use windows::Win32::UI::Shell::{
    ASSOCF_NONE, ASSOCSTR_EXECUTABLE, AssocQueryStringW, SHCNE_ASSOCCHANGED, SHCNF_IDLIST,
    SHChangeNotify,
};
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

use crate::associations::{Registry, Value};

/// A Win32 status as an `io::Result`.
fn status(code: WIN32_ERROR) -> io::Result<()> {
    if code.is_ok() {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code.0 as i32))
    }
}

/// A registry root: HKEY_CURRENT_USER, or a hive loaded from a file (closed, and so unloaded,
/// when dropped).
pub struct Hive {
    root: HKEY,
    owned: bool,
}

impl Hive {
    pub fn current_user() -> Hive {
        Hive {
            root: HKEY_CURRENT_USER,
            owned: false,
        }
    }

    /// A private hive in `file` (made if it doesn't exist), seen by this process only.
    pub fn load(file: &Path) -> io::Result<Hive> {
        let mut root = HKEY::default();
        // SAFETY: the path is a live, null-terminated string; the key is ours to close.
        status(unsafe {
            RegLoadAppKeyW(
                &HSTRING::from(file.as_os_str()),
                &mut root,
                KEY_ALL_ACCESS.0,
                0,
                None,
            )
        })?;
        Ok(Hive { root, owned: true })
    }
}

/// Windows' own version, as it records it: its release ("24H2", if it says), build and
/// revision. Read only, from HKEY_LOCAL_MACHINE.
pub fn windows_version() -> Option<crate::about::WindowsVersion> {
    const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let machine = Hive {
        root: HKEY_LOCAL_MACHINE,
        owned: false,
    };
    let build = machine
        .read(KEY, Some("CurrentBuildNumber"))?
        .parse()
        .ok()?;
    let release = machine
        .read(KEY, Some("DisplayVersion"))
        .filter(|r| !r.is_empty());
    let (key, name) = (HSTRING::from(KEY), HSTRING::from("UBR"));
    let mut revision = 0u32;
    let mut bytes = size_of::<u32>() as u32;
    // SAFETY: the strings are live, and the value is a DWORD written into ours.
    let read = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &key,
            &name,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut revision).cast()),
            Some(&mut bytes),
        )
    };
    Some(crate::about::WindowsVersion {
        release,
        build,
        revision: read.is_ok().then_some(revision),
    })
}

impl Drop for Hive {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: the key came from RegLoadAppKeyW and is closed once.
            let _ = unsafe { RegCloseKey(self.root) };
        }
    }
}

/// `name` as a pointer: the default value's is null.
fn name_ptr(name: Option<&HSTRING>) -> PCWSTR {
    name.map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()))
}

impl Registry for Hive {
    fn read(&self, key: &str, name: Option<&str>) -> Option<String> {
        let (key, name) = (HSTRING::from(key), name.map(HSTRING::from));
        let mut bytes = 0u32;
        // SAFETY: the strings are live; the first call only sizes the value.
        unsafe {
            RegGetValueW(
                self.root,
                &key,
                name_ptr(name.as_ref()),
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut bytes),
            )
        }
        .ok()
        .ok()?;
        let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
        // SAFETY: the buffer is ours and its size in bytes given.
        unsafe {
            RegGetValueW(
                self.root,
                &key,
                name_ptr(name.as_ref()),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        }
        .ok()
        .ok()?;
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..end]))
    }

    fn write(&mut self, value: &Value) -> io::Result<()> {
        let key = HSTRING::from(value.key.as_str());
        let name = value.name.as_deref().map(HSTRING::from);
        let mut created = HKEY::default();
        // SAFETY: the strings are live; the key made is closed below.
        status(unsafe {
            RegCreateKeyExW(
                self.root,
                &key,
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut created,
                None,
            )
        })?;
        // SAFETY: as above.
        let _ = unsafe { RegCloseKey(created) };
        let data: Vec<u16> = value.data.encode_utf16().chain([0]).collect();
        // SAFETY: the data is ours, null-terminated, and its size in bytes given.
        status(unsafe {
            RegSetKeyValueW(
                self.root,
                &key,
                name_ptr(name.as_ref()),
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * 2) as u32,
            )
        })
    }

    fn delete_value(&mut self, key: &str, name: Option<&str>) -> io::Result<()> {
        let name = name.map(HSTRING::from);
        // SAFETY: the strings are live.
        let code =
            unsafe { RegDeleteKeyValueW(self.root, &HSTRING::from(key), name_ptr(name.as_ref())) };
        if code == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            status(code)
        }
    }

    fn delete_tree(&mut self, key: &str) -> io::Result<()> {
        // SAFETY: the string is live.
        let code = unsafe { RegDeleteTreeW(self.root, &HSTRING::from(key)) };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        status(code)?;
        // RegDeleteTree empties the key; this removes the key itself.
        // SAFETY: as above.
        let code = unsafe { RegDeleteKeyW(self.root, &HSTRING::from(key)) };
        if code == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            status(code)
        }
    }

    fn delete_if_empty(&mut self, key: &str) {
        let key = HSTRING::from(key);
        let mut open = HKEY::default();
        // SAFETY: the string is live; the key is closed below.
        if unsafe { RegOpenKeyExW(self.root, &key, None, KEY_READ, &mut open) }.is_err() {
            return;
        }
        let mut subkeys = 0u32;
        let mut values = 0u32;
        // SAFETY: the key is open; only the counts are asked for.
        let counted = unsafe {
            windows::Win32::System::Registry::RegQueryInfoKeyW(
                open,
                None,
                None,
                None,
                Some(&mut subkeys),
                None,
                None,
                Some(&mut values),
                None,
                None,
                None,
                None,
            )
        };
        // SAFETY: opened above.
        let _ = unsafe { RegCloseKey(open) };
        if counted.is_ok() && subkeys == 0 && values == 0 {
            // SAFETY: the string is live.
            let _ = unsafe { RegDeleteKeyW(self.root, &key) };
        }
    }
}

/// Tells Explorer the associations changed, so icons and Open with catch up.
pub fn associations_changed() {
    // SAFETY: no items are passed with SHCNE_ASSOCCHANGED.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// The exe that opens `.md` files now, if any.
pub fn md_handler() -> Option<PathBuf> {
    let mut len = 0u32;
    // SAFETY: the first call only sizes the string.
    let _ = unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_EXECUTABLE,
            w!(".md"),
            w!("open"),
            None,
            &mut len,
        )
    };
    if len == 0 {
        return None;
    }
    let mut buffer = vec![0u16; len as usize];
    // SAFETY: the buffer is ours and its length given.
    unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_EXECUTABLE,
            w!(".md"),
            w!("open"),
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut len,
        )
    }
    .ok()
    .ok()?;
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let exe = PathBuf::from(OsString::from_wide(&buffer[..end]));
    (!exe.as_os_str().is_empty()).then_some(exe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::associations::{self, is_registered, register, unregister};

    #[test]
    fn registering_and_unregistering_in_a_hive_of_its_own() {
        let temp = tempfile::tempdir().unwrap();
        let mut hive = Hive::load(&temp.path().join("user.dat")).unwrap();
        let other = Value {
            key: r"Software\Classes\.md\OpenWithProgids".into(),
            name: Some("Other.md".into()),
            data: String::new(),
        };
        hive.write(&other).unwrap();
        let exe = temp.path().join("Légible folder").join("legible.exe");
        let values = associations::values(&exe, "Markdown document", "Reads Markdown — and more");
        assert!(!is_registered(&hive, &values));
        register(&mut hive, &values).unwrap();
        assert!(is_registered(&hive, &values));
        assert_eq!(
            hive.read(
                r"Software\Classes\Legible.Markdown\shell\open\command",
                None
            ),
            Some(associations::open_command(&exe))
        );
        assert_eq!(
            hive.read(
                r"Software\Legible\Capabilities",
                Some("ApplicationDescription")
            )
            .as_deref(),
            Some("Reads Markdown — and more")
        );

        unregister(&mut hive, &values).unwrap();
        assert!(
            values
                .iter()
                .all(|v| hive.read(&v.key, v.name.as_deref()).is_none())
        );
        assert_eq!(
            hive.read(&other.key, other.name.as_deref()).as_deref(),
            Some("")
        );
        for key in associations::OWNED_KEYS
            .iter()
            .chain([&r"Software\Legible"])
        {
            let mut open = HKEY::default();
            // SAFETY: the string is live; a key opened is closed.
            let found = unsafe {
                RegOpenKeyExW(hive.root, &HSTRING::from(*key), None, KEY_READ, &mut open)
            };
            if found.is_ok() {
                let _ = unsafe { RegCloseKey(open) };
            }
            assert!(found.is_err(), "{key} is left");
        }
        // Again, with nothing left to remove.
        unregister(&mut hive, &values).unwrap();
    }
}
