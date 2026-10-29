//! A window's own dialogs: Windows' Save dialog, and the question asked when a window with
//! unsaved changes closes.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_EXPLORER, OFN_HIDEREADONLY, OFN_NOREADONLYRETURN, OFN_OVERWRITEPROMPT,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TD_WARNING_ICON, TDCBF_CANCEL_BUTTON,
    TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW, TaskDialogIndirect,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

use super::wide;

/// Windows' Save dialog, owned by `owner`, titled `title`, offering files of type `filter`
/// (its name) matching `pattern` (`*.pdf`), with `extension` added to a name typed without
/// one; it starts at `name`, in `folder` if given. It asks before a file that's there is
/// replaced. None if the reader cancels (or, in a test's process, at once).
pub fn save_dialog(
    owner: isize,
    title: &str,
    filter: &str,
    pattern: &str,
    extension: &str,
    name: &str,
    folder: Option<&Path>,
) -> Option<PathBuf> {
    if super::dialog_held_back("the Save dialog") {
        return None;
    }
    // Pairs of (name, patterns), each null-terminated, then one more null.
    let filter: Vec<u16> = format!("{filter}\0{pattern}\0\0").encode_utf16().collect();
    let title = HSTRING::from(title);
    let extension = HSTRING::from(extension);
    let folder = folder.map(wide);
    let mut file = vec![0u16; 32_768];
    for (i, unit) in name.encode_utf16().take(file.len() - 1).enumerate() {
        file[i] = unit;
    }
    let mut options = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: HWND(owner as *mut _),
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        lpstrInitialDir: folder
            .as_ref()
            .map_or(PCWSTR::null(), |f| PCWSTR(f.as_ptr())),
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrDefExt: PCWSTR(extension.as_ptr()),
        Flags: OFN_EXPLORER
            | OFN_OVERWRITEPROMPT
            | OFN_PATHMUSTEXIST
            | OFN_HIDEREADONLY
            | OFN_NOREADONLYRETURN,
        ..Default::default()
    };
    // SAFETY: every pointer is to a buffer this function owns and keeps alive for the call;
    // the file buffer's length is given in nMaxFile.
    if !unsafe { GetSaveFileNameW(&mut options) }.as_bool() {
        return None;
    }
    let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    Some(OsString::from_wide(&file[..len]).into())
}

/// The answer to [`ask_save`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveAnswer {
    Save,
    DontSave,
    Cancel,
}

/// The question a window with unsaved changes asks as it closes: Save, Don't save, or Cancel
/// (also Esc and the dialog's own close), owned by `owner`. In a test's process, Cancel at once.
pub fn ask_save(
    owner: isize,
    title: &str,
    question: &str,
    detail: &str,
    save: &str,
    dont_save: &str,
) -> SaveAnswer {
    if super::dialog_held_back("the close question") {
        return SaveAnswer::Cancel;
    }
    const SAVE: i32 = 100;
    const DONT_SAVE: i32 = 101;
    let (title, question, detail) = (
        HSTRING::from(title),
        HSTRING::from(question),
        HSTRING::from(detail),
    );
    let (save, dont_save) = (HSTRING::from(save), HSTRING::from(dont_save));
    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: SAVE,
            pszButtonText: PCWSTR(save.as_ptr()),
        },
        TASKDIALOG_BUTTON {
            nButtonID: DONT_SAVE,
            pszButtonText: PCWSTR(dont_save.as_ptr()),
        },
    ];
    let mut config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: HWND(owner as *mut _),
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        pszMainInstruction: PCWSTR(question.as_ptr()),
        pszContent: PCWSTR(detail.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: SAVE,
        ..Default::default()
    };
    config.Anonymous1.pszMainIcon = TD_WARNING_ICON;
    let mut pressed = 0;
    // SAFETY: the config and everything it points to live for the call, which returns once
    // the dialog has closed.
    match unsafe { TaskDialogIndirect(&config, Some(&mut pressed), None, None) } {
        Ok(()) if pressed == SAVE => SaveAnswer::Save,
        Ok(()) if pressed == DONT_SAVE => SaveAnswer::DontSave,
        // Cancel, Esc, the dialog closed, or it couldn't show: nothing's dropped.
        _ => SaveAnswer::Cancel,
    }
}
