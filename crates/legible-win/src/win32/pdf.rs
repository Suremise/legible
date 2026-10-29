//! Save as PDF's Windows parts: the user's paper and Documents folder, the Save dialog, and
//! WebView2's `PrintToPdf` (`crate::export` has the rest).

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use legible_core::i18n::tr;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2_2, ICoreWebView2_7, ICoreWebView2Environment6,
};
use webview2_com::PrintToPdfCompletedHandler;
use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_IMEASURE};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::core::{Interface, PCWSTR, PWSTR};

use super::{io_error, wide};
use crate::export::Paper;

/// The paper the user's locale measures in.
pub fn locale_paper() -> Paper {
    let mut buffer = [0u16; 4];
    // SAFETY: the user's default locale (a null name), into a buffer of our own.
    let len = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_IMEASURE, Some(&mut buffer)) };
    let len = usize::try_from(len).unwrap_or(0).saturating_sub(1);
    Paper::for_measure(&String::from_utf16_lossy(&buffer[..len.min(buffer.len())]))
}

/// The user's Documents folder.
pub fn documents_folder() -> Option<PathBuf> {
    // SAFETY: the path Windows returns is ours to free, copied out first.
    unsafe {
        let path: PWSTR = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None).ok()?;
        let folder = PathBuf::from(OsString::from_wide(path.as_wide()));
        CoTaskMemFree(Some(path.0 as _));
        Some(folder)
    }
}

/// Asks where to save a PDF, owned by window `owner`: `name` in `folder` to begin with.
/// None when cancelled.
pub fn save_pdf_dialog(
    owner: isize,
    title: &str,
    filter: &str,
    name: &str,
    folder: Option<&Path>,
) -> Option<PathBuf> {
    super::dialogs::save_dialog(owner, title, filter, "*.pdf", "pdf", name, folder)
}

/// Prints `core`'s page to the PDF `path` on `paper`, with backgrounds and without WebView2's
/// own header and footer (the page's `@page` boxes print those); `done` hears how it went, on
/// this (the window's) thread. Only one print may run on a WebView2 at a time.
pub fn print_to_pdf(
    core: &ICoreWebView2,
    path: &Path,
    paper: Paper,
    done: Box<dyn FnOnce(io::Result<()>)>,
) -> io::Result<()> {
    let (width, height) = paper.inches();
    // SAFETY: COM calls on the window's thread, on WebView2's own objects; the path lives for
    // the call (WebView2 copies it).
    unsafe {
        let core7: ICoreWebView2_7 = core.cast().map_err(|e| io_error(&e))?;
        let environment: ICoreWebView2Environment6 = core
            .cast::<ICoreWebView2_2>()
            .and_then(|c| c.Environment())
            .and_then(|e| e.cast())
            .map_err(|e| io_error(&e))?;
        let settings = environment
            .CreatePrintSettings()
            .map_err(|e| io_error(&e))?;
        settings.SetPageWidth(width).map_err(|e| io_error(&e))?;
        settings.SetPageHeight(height).map_err(|e| io_error(&e))?;
        settings
            .SetShouldPrintBackgrounds(true)
            .map_err(|e| io_error(&e))?;
        settings
            .SetShouldPrintHeaderAndFooter(false)
            .map_err(|e| io_error(&e))?;
        let handler = PrintToPdfCompletedHandler::create(Box::new(move |result, succeeded| {
            done(match result {
                Ok(()) if succeeded => Ok(()),
                Ok(()) => Err(io::Error::other(tr("chrome-pdf-print-failed"))),
                Err(error) => Err(io_error(&error)),
            });
            Ok(())
        }));
        core7
            .PrintToPdf(&wide(path), &settings, &handler)
            .map_err(|e| io_error(&e))
    }
}
