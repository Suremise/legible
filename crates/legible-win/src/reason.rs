//! Why something failed, as the messages that say so put it (`{ $reason }`): the common
//! failures in Legible's words, through Fluent (`reason-*`); another of Windows' in Windows' own
//! words, without the " (os error 5)" Rust adds; and one of Legible's own, which only the log
//! explains, in a few words that say so. The log keeps every error as it was.

use std::io::{self, ErrorKind};

use legible_core::i18n::tr;

/// Windows' errors for another program holding a file: ERROR_SHARING_VIOLATION and
/// ERROR_LOCK_VIOLATION, which Rust gives no kind of their own.
const IN_USE: [i32; 2] = [32, 33];
/// ERROR_WRITE_PROTECT: a disk that can't be written to.
const WRITE_PROTECT: i32 = 19;
/// ERROR_HANDLE_DISK_FULL and ERROR_DISK_FULL.
const DISK_FULL: [i32; 2] = [39, 112];

/// Why `error` happened, for a message to the reader.
pub fn of(error: &io::Error) -> String {
    let code = error.raw_os_error();
    let key = match error.kind() {
        _ if code.is_some_and(|c| IN_USE.contains(&c)) => "reason-in-use",
        _ if code == Some(WRITE_PROTECT) => "reason-read-only",
        _ if code.is_some_and(|c| DISK_FULL.contains(&c)) => "reason-disk-full",
        ErrorKind::ReadOnlyFilesystem => "reason-read-only",
        ErrorKind::StorageFull => "reason-disk-full",
        ErrorKind::NotFound => "reason-not-found",
        ErrorKind::PermissionDenied => "reason-denied",
        _ if code.is_some() => return windows_words(error),
        _ => "reason-unexpected",
    };
    tr(key)
}

/// Windows' own words for an error of its: its message, without Rust's " (os error N)" or the
/// full stop, as the messages round it have their own.
fn windows_words(error: &io::Error) -> String {
    let text = error.to_string();
    let text = match text.rfind(" (os error ") {
        Some(at) if text.ends_with(')') => &text[..at],
        _ => &text,
    };
    text.trim_end().trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_common_failures_are_said_in_legible_s_words() {
        let os = io::Error::from_raw_os_error;
        assert_eq!(of(&os(2)), tr("reason-not-found"));
        assert_eq!(of(&os(5)), tr("reason-denied"));
        assert_eq!(of(&os(32)), tr("reason-in-use"));
        assert_eq!(of(&os(33)), tr("reason-in-use"));
        assert_eq!(of(&os(19)), tr("reason-read-only"));
        assert_eq!(of(&os(112)), tr("reason-disk-full"));
        assert!(!of(&os(5)).contains("os error"));
    }

    #[test]
    fn another_of_windows_is_in_its_words_without_the_code() {
        // ERROR_NOT_READY: "The device is not ready." in English Windows.
        let said = of(&io::Error::from_raw_os_error(21));
        assert!(
            !said.is_empty() && !said.contains("os error") && !said.ends_with('.'),
            "{said}"
        );
    }

    #[test]
    fn one_of_legible_s_own_says_only_that_it_was_unexpected() {
        let error = io::Error::other("WebView2 couldn't print the PDF");
        assert_eq!(of(&error), tr("reason-unexpected"));
        // A kind it knows is said as that, whoever made it.
        let error = io::Error::new(ErrorKind::NotFound, "no folder");
        assert_eq!(of(&error), tr("reason-not-found"));
    }
}
