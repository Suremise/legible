//! Windows' shell for [`crate::actions`]: `.md`'s association (`IQueryAssociations`), launches
//! through it (`ShellExecuteExW` with its class key), Notepad, and Explorer. Made and used on
//! the actions thread, which it sets up for COM (single-threaded) and tears down after.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HANDLE, HWND};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Registry::{HKEY, RegCloseKey};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ASSOCF_INIT_IGNOREUNKNOWN, ASSOCF_NONE, ASSOCF_NOTRUNCATE, ASSOCKEY_SHELLEXECCLASS,
    ASSOCSTR_EXECUTABLE, AssocCreate, CLSID_QueryAssociations, ILCreateFromPathW, ILFree,
    IQueryAssociations, SEE_MASK_CLASSKEY, SEE_MASK_FLAG_NO_UI, SEE_MASK_IDLIST, SEE_MASK_NOASYNC,
    SHELLEXECUTEINFOW, SHOpenFolderAndSelectItems, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

use super::{io_error, wide};
use crate::actions::{Handler, Launch, Platform, Verb, notepad_command};
use crate::quiet;

/// Whether a launch is left undone: a quiet window's (a test's) never opens another app, which
/// would take the user's focus.
fn held_back(what: &str) -> bool {
    let held = quiet::on();
    if held {
        tracing::info!(what, "quiet: not launched");
    }
    held
}

/// An association's class key, closed when dropped (after the launch that uses it).
pub struct ClassKey(HKEY);

impl Drop for ClassKey {
    fn drop(&mut self) {
        // SAFETY: the key is ours, from GetKey, and closed once.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

/// An item ID list Windows allocated, freed when dropped.
struct Pidl(*mut ITEMIDLIST);

impl Pidl {
    fn of(path: &Path) -> io::Result<Pidl> {
        // SAFETY: the string lives for the call; the list returned is ours to free.
        let pidl = unsafe { ILCreateFromPathW(&wide(path)) };
        if pidl.is_null() {
            Err(io::Error::from_raw_os_error(ERROR_FILE_NOT_FOUND.0 as i32))
        } else {
            Ok(Pidl(pidl))
        }
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        // SAFETY: allocated by ILCreateFromPathW, freed once.
        unsafe { ILFree(Some(self.0)) };
    }
}

/// The shell, on a thread set up for COM. Made with [`ShellPlatform::new`] on that thread.
pub struct ShellPlatform {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl ShellPlatform {
    /// Sets this thread up for COM (single-threaded, as the shell wants).
    pub fn new() -> io::Result<ShellPlatform> {
        // SAFETY: once on this thread; undone in Drop.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
            .ok()
            .map_err(|e| io_error(&e))?;
        Ok(ShellPlatform {
            _not_send: std::marker::PhantomData,
        })
    }
}

impl Drop for ShellPlatform {
    fn drop(&mut self) {
        // SAFETY: matches the CoInitializeEx in `new`, on the same thread (it isn't Send).
        unsafe { CoUninitialize() };
    }
}

/// `.md`'s association, from the user's choice down.
fn md_association() -> windows::core::Result<IQueryAssociations> {
    // SAFETY: COM is set up on this thread; the string is static.
    unsafe {
        let assoc: IQueryAssociations = AssocCreate(CLSID_QueryAssociations)?;
        assoc.Init(ASSOCF_INIT_IGNOREUNKNOWN, w!(".md"), None, None)?;
        Ok(assoc)
    }
}

impl Platform for ShellPlatform {
    type Class = ClassKey;

    fn md_handler(&self, verb: Verb) -> Option<Handler<ClassKey>> {
        let verb = HSTRING::from(verb.as_str());
        let found = (|| -> windows::core::Result<Handler<ClassKey>> {
            let assoc = md_association()?;
            // SAFETY: the buffer is ours and its length given; the key returned is ours.
            unsafe {
                let mut len = 0u32;
                assoc.GetString(
                    ASSOCF_NOTRUNCATE,
                    ASSOCSTR_EXECUTABLE,
                    &verb,
                    None,
                    &mut len,
                )?;
                let mut buffer = vec![0u16; len as usize];
                assoc.GetString(
                    ASSOCF_NOTRUNCATE,
                    ASSOCSTR_EXECUTABLE,
                    &verb,
                    Some(PWSTR(buffer.as_mut_ptr())),
                    &mut len,
                )?;
                let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
                let exe = PathBuf::from(OsString::from_wide(&buffer[..end]));
                let class = ClassKey(assoc.GetKey(ASSOCF_NONE, ASSOCKEY_SHELLEXECCLASS, &verb)?);
                Ok(Handler { exe, class })
            }
        })();
        match found {
            Ok(handler) if !handler.exe.as_os_str().is_empty() => Some(handler),
            Ok(_) => None,
            Err(error) => {
                tracing::debug!(%error, verb = %verb, "no handler for .md");
                None
            }
        }
    }

    fn launch(&self, launch: &Launch<'_, ClassKey>) -> io::Result<()> {
        if held_back("an editor") {
            return Ok(());
        }
        match launch {
            Launch::Verb { verb, class, file } => {
                let verb = HSTRING::from(verb.as_str());
                let file = wide(file);
                let mut info = SHELLEXECUTEINFOW {
                    cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                    fMask: SEE_MASK_CLASSKEY | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
                    hwnd: HWND::default(),
                    lpVerb: PCWSTR(verb.as_ptr()),
                    lpFile: PCWSTR(file.as_ptr()),
                    nShow: SW_SHOWNORMAL.0,
                    hkeyClass: class.0,
                    ..Default::default()
                };
                // SAFETY: every string and the class key outlive the call (NOASYNC: it's done
                // with them when it returns); no process handle is asked for.
                unsafe { ShellExecuteExW(&mut info) }.map_err(|e| io_error(&e))
            }
            Launch::Notepad { exe, file } => notepad_command(exe, file).spawn().map(|_| ()),
        }
    }

    fn notepad(&self) -> io::Result<PathBuf> {
        let mut buffer = vec![0u16; 260];
        // SAFETY: the buffer is ours, its length given.
        let len = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
        if len == 0 || len >= buffer.len() {
            return Err(io::Error::last_os_error());
        }
        Ok(PathBuf::from(OsString::from_wide(&buffer[..len])).join("notepad.exe"))
    }

    fn exists(&self, path: &Path) -> bool {
        path.try_exists().unwrap_or(false)
    }

    fn show_item(&self, file: &Path) -> io::Result<()> {
        if held_back("Explorer") {
            return Ok(());
        }
        let pidl = Pidl::of(file)?;
        // SAFETY: the full ID list of one item, no children: Explorer opens its folder and
        // selects it. The list is freed after the call.
        unsafe { SHOpenFolderAndSelectItems(pidl.0, None, 0) }.map_err(|e| io_error(&e))
    }

    fn open_folder(&self, folder: &Path) -> io::Result<()> {
        if held_back("Explorer") {
            return Ok(());
        }
        let pidl = Pidl::of(folder)?;
        let verb = w!("open");
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_IDLIST | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            hwnd: HWND::default(),
            lpVerb: verb,
            lpIDList: pidl.0.cast(),
            nShow: SW_SHOWNORMAL.0,
            hProcess: HANDLE::default(),
            ..Default::default()
        };
        // SAFETY: the ID list outlives the call (NOASYNC); no process handle is asked for.
        unsafe { ShellExecuteExW(&mut info) }.map_err(|e| io_error(&e))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::actions::shell_path;

    #[test]
    fn a_canonical_path_becomes_an_item_id_list_once_made_plain() {
        // The file as Shell::open has it: canonical, so verbatim. The shell parses it only
        // plain; nothing is opened or launched here, and each list is freed by its guard.
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Notes & résumé 日本語");
        fs::create_dir(&folder).unwrap();
        let file = folder.join("Plan (v2) 100%.md");
        fs::write(&file, "# Plan").unwrap();
        let canonical = fs::canonicalize(&file).unwrap();
        assert!(canonical.as_os_str().to_string_lossy().starts_with(r"\\?\"));
        assert!(
            Pidl::of(&canonical).is_err(),
            "the shell took a verbatim path after all"
        );
        let plain = shell_path(&canonical);
        assert!(Pidl::of(&plain).is_ok(), "{}", plain.display());
        assert!(Pidl::of(plain.parent().unwrap()).is_ok());
    }
}
