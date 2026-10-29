//! Windows' side of writing a document (`legible_core::save::Replace`).
//!
//! **Replacing** is `ReplaceFileW`, always with a backup name, so the replaced file survives
//! until `save::write` knows the replace worked. It keeps the replaced file's attributes,
//! creation time and security (its DACL); a merge or security error fails the replace rather
//! than weakening it (no `REPLACEFILE_IGNORE_*` flags). Its documented results are mapped to
//! [`Replaced`]: the half-done one (`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`) as such, a sharing
//! violation as busy, and everything else (`ERROR_UNABLE_TO_REMOVE_REPLACED`,
//! `ERROR_UNABLE_TO_MOVE_REPLACEMENT`, access denied, a filesystem that can't replace) as
//! nothing moved. There's no fallback: where the document can't be replaced with its metadata,
//! it isn't saved, and stays as it was.
//!
//! **Creating** a file (a new copy, or one that went) moves the staged file into place without
//! ever replacing one that's there.

use std::fs::File;
use std::io;
use std::path::Path;

use legible_core::save::{Replace, Replaced};
use windows::Win32::Foundation::{
    ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION, ERROR_UNABLE_TO_MOVE_REPLACEMENT_2, WIN32_ERROR,
};
use windows::Win32::Storage::FileSystem::{
    MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACE_FILE_FLAGS, ReplaceFileW,
};

use super::wide;

/// The Windows filesystem, as `save::write` uses it.
pub struct Windows;

impl Replace for Windows {
    fn replace(&self, document: &Path, staged: &Path, backup: &Path) -> Replaced {
        // SAFETY: the three paths are live HSTRINGs for the call; the exclude and reserved
        // pointers are null, as the API asks.
        let replaced = unsafe {
            ReplaceFileW(
                &wide(document),
                &wide(staged),
                &wide(backup),
                REPLACE_FILE_FLAGS(0),
                None,
                None,
            )
        };
        match replaced {
            Ok(()) => Replaced::Done,
            Err(error) => {
                let code = WIN32_ERROR::from_error(&error).map_or(0, |c| c.0);
                outcome(code, io::Error::from_raw_os_error(code as i32))
            }
        }
    }

    fn create(&self, from: &Path, to: &Path) -> io::Result<()> {
        // Without MOVEFILE_REPLACE_EXISTING: a file at `to` stops it (AlreadyExists).
        // SAFETY: both paths are live HSTRINGs for the call.
        unsafe { MoveFileExW(&wide(from), &wide(to), MOVEFILE_WRITE_THROUGH) }
            .map_err(|e| super::io_error(&e))
    }

    fn identity(&self, file: &File) -> io::Result<Option<(u64, u64)>> {
        super::identity(file)
    }
}

/// What `ReplaceFileW`'s error `code` means for the files (with a backup name given, as
/// [`Windows::replace`] always does).
pub fn outcome(code: u32, error: io::Error) -> Replaced {
    match WIN32_ERROR(code) {
        // The document is at the backup name; the new file still at its own.
        ERROR_UNABLE_TO_MOVE_REPLACEMENT_2 => Replaced::OriginalAtBackup(error),
        ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION => Replaced::Busy(error),
        // ERROR_UNABLE_TO_REMOVE_REPLACED and ERROR_UNABLE_TO_MOVE_REPLACEMENT keep both files'
        // names (with a backup name given); anything else moved nothing.
        _ => Replaced::NotMoved(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use legible_core::save::{self, NoHooks};
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW,
        ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
    };
    use windows::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN, FILE_FLAGS_AND_ATTRIBUTES,
        GetFileAttributesW, SetFileAttributesW,
    };
    use windows::core::{HSTRING, PWSTR};

    #[test]
    fn replace_file_w_s_results_mean_what_windows_says_of_the_files() {
        let error = || io::Error::other("e");
        assert!(matches!(
            outcome(1177, error()),
            Replaced::OriginalAtBackup(_)
        ));
        assert!(matches!(outcome(32, error()), Replaced::Busy(_)));
        assert!(matches!(outcome(33, error()), Replaced::Busy(_)));
        // 1175, 1176 (with a backup name), access denied, not supported: nothing moved.
        for code in [1175, 1176, 5, 50, 1] {
            assert!(
                matches!(outcome(code, error()), Replaced::NotMoved(_)),
                "{code}"
            );
        }
    }

    /// The file's DACL, as SDDL.
    fn dacl(path: &Path) -> String {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: a live path for the call; the descriptor is LocalFree'd below.
        let got = unsafe {
            GetNamedSecurityInfoW(
                &wide(path),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                None,
                None,
                &mut descriptor,
            )
        };
        assert!(got.is_ok(), "{got:?}");
        let mut sddl = PWSTR::null();
        // SAFETY: the descriptor is the one just read; the string is LocalFree'd below.
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut sddl,
                None,
            )
        }
        .unwrap();
        // SAFETY: a string Windows allocated and null-terminated.
        let text = unsafe { sddl.to_string() }.unwrap();
        // SAFETY: both were allocated by the calls above, and are freed once.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sddl.0.cast())));
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
        text
    }

    /// Gives `path` a protected DACL of its own, from `sddl`.
    fn set_dacl(path: &Path, sddl: &str) {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: a live string for the call; the descriptor is LocalFree'd below.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &HSTRING::from(sddl),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .unwrap();
        let (mut present, mut defaulted) = (false.into(), false.into());
        let mut acl: *mut ACL = std::ptr::null_mut();
        // SAFETY: the descriptor is the one just made; acl points into it.
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) }
            .unwrap();
        // SAFETY: a live path; the ACL lives in the descriptor until it's freed below.
        let set = unsafe {
            SetNamedSecurityInfoW(
                &wide(path),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(acl),
                None,
            )
        };
        // SAFETY: allocated by the conversion above, freed once.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
        assert!(set.is_ok(), "{set:?}");
    }

    #[test]
    fn a_save_keeps_the_documents_attributes_and_security() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, b"old").unwrap();
        // Hidden and archived, and a DACL of its own: the owner, and everyone reading.
        // SAFETY: a live path for the call.
        unsafe { SetFileAttributesW(&wide(&file), FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_ARCHIVE) }
            .unwrap();
        set_dacl(&file, "D:P(A;;FA;;;OW)(A;;FR;;;WD)");
        // SAFETY: a live path for the call.
        let attributes = || FILE_FLAGS_AND_ATTRIBUTES(unsafe { GetFileAttributesW(&wide(&file)) });
        let (before_attributes, before_dacl) = (attributes(), dacl(&file));
        assert!(before_dacl.contains("(A;;FR;;;WD)"), "{before_dacl}");

        let stamp = save::read(&file, &super::super::identity).unwrap().1;
        save::write(&file, b"new", Some(&stamp), &Windows, &NoHooks).unwrap();

        assert_eq!(std::fs::read(&file).unwrap(), b"new");
        assert_eq!(attributes(), before_attributes);
        assert_eq!(dacl(&file), before_dacl);
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1, "the staged and backup files are gone");
    }

    #[test]
    fn creating_never_replaces_a_file_that_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::write(&from, b"mine").unwrap();
        std::fs::write(&to, b"theirs").unwrap();
        let error = Windows.create(&from, &to).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&to).unwrap(), b"theirs");
        std::fs::remove_file(&to).unwrap();
        Windows.create(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"mine");
    }
}
