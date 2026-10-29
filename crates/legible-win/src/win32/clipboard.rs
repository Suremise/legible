//! Windows' clipboard for [`crate::clipboard::write`], opened by a window of Legible's own, and
//! reading it back (for the by-hand check only).

use std::io;

use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::core::w;

use super::io_error;
use crate::clipboard::ClipboardPort;

/// The clipboard, owned while it's open by the window `hwnd`.
pub struct WindowsClipboard {
    hwnd: HWND,
}

impl WindowsClipboard {
    /// For the window with handle `hwnd` (its own: the clipboard needs an owner to take data).
    pub fn new(hwnd: isize) -> WindowsClipboard {
        WindowsClipboard {
            hwnd: HWND(hwnd as *mut _),
        }
    }
}

/// A moveable block of global memory, holding a copy of some bytes.
pub struct Block(HGLOBAL);

impl ClipboardPort for WindowsClipboard {
    type Data = Block;

    fn html_format(&mut self) -> io::Result<u32> {
        // SAFETY: a static, NUL-terminated name.
        match unsafe { RegisterClipboardFormatW(w!("HTML Format")) } {
            0 => Err(io::Error::last_os_error()),
            format => Ok(format),
        }
    }

    fn alloc(&mut self, bytes: &[u8]) -> io::Result<Block> {
        // SAFETY: a new block of `bytes.len()` bytes, locked only while they're copied in.
        unsafe {
            let block = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).map_err(|e| io_error(&e))?;
            let at = GlobalLock(block);
            if at.is_null() {
                let error = io::Error::last_os_error();
                let _ = GlobalFree(Some(block));
                return Err(error);
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), at.cast::<u8>(), bytes.len());
            // Unlocking to a count of 0 "fails" with no error: nothing to report.
            let _ = GlobalUnlock(block);
            Ok(Block(block))
        }
    }

    fn free(&mut self, data: Block) {
        // SAFETY: a block this port allocated, which the clipboard didn't take.
        let _ = unsafe { GlobalFree(Some(data.0)) };
    }

    fn open(&mut self) -> io::Result<bool> {
        // SAFETY: the window is Legible's own and alive for the call.
        match unsafe { OpenClipboard(Some(self.hwnd)) } {
            Ok(()) => Ok(true),
            // Another app has it open just now.
            Err(error) if error.code() == ERROR_ACCESS_DENIED.to_hresult() => Ok(false),
            Err(error) => Err(io_error(&error)),
        }
    }

    fn empty(&mut self) -> io::Result<()> {
        // SAFETY: the clipboard is open by this window.
        unsafe { EmptyClipboard() }.map_err(|e| io_error(&e))
    }

    fn set(&mut self, format: u32, data: Block) -> Result<(), (Block, io::Error)> {
        // SAFETY: the clipboard is open and emptied by this window; on success it owns the block.
        match unsafe { SetClipboardData(format, Some(HANDLE(data.0.0))) } {
            Ok(_) => Ok(()),
            Err(error) => Err((data, io_error(&error))),
        }
    }

    fn close(&mut self) {
        // SAFETY: opened by this port.
        let _ = unsafe { CloseClipboard() };
    }
}

/// What the clipboard holds as `format`, read by window `hwnd`: for the by-hand check of a real
/// copy only (it never runs in the suite).
pub fn read(hwnd: isize, format: u32) -> io::Result<Vec<u8>> {
    // SAFETY: opened and closed here; the data is the clipboard's, only read while locked.
    unsafe {
        OpenClipboard(Some(HWND(hwnd as *mut _))).map_err(|e| io_error(&e))?;
        let read = (|| {
            let handle = GetClipboardData(format).map_err(|e| io_error(&e))?;
            let block = HGLOBAL(handle.0);
            let size = GlobalSize(block);
            let at = GlobalLock(block);
            if at.is_null() {
                return Err(io::Error::last_os_error());
            }
            let bytes = std::slice::from_raw_parts(at.cast::<u8>(), size).to_vec();
            let _ = GlobalUnlock(block);
            Ok(bytes)
        })();
        let _ = CloseClipboard();
        read
    }
}

/// "HTML Format"'s number on this PC.
pub fn html_format() -> io::Result<u32> {
    WindowsClipboard::new(0).html_format()
}

/// A message-only window to own the clipboard, destroyed when dropped: for the by-hand check,
/// which has no window of Legible's.
pub struct MessageWindow(HWND);

impl MessageWindow {
    pub fn new() -> io::Result<MessageWindow> {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
        };
        // SAFETY: a message-only STATIC window: no parent but HWND_MESSAGE, no data.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Legible clipboard check"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .map_err(|e| io_error(&e))?;
        Ok(MessageWindow(hwnd))
    }

    /// Its handle, as [`WindowsClipboard::new`] and [`read`] take it.
    pub fn handle(&self) -> isize {
        self.0.0 as isize
    }
}

impl Drop for MessageWindow {
    fn drop(&mut self) {
        // SAFETY: this window, made by `new`.
        let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.0) };
    }
}
