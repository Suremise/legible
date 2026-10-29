//! Windows ending the session, as a document window hears it (`crate::session` decides what it
//! answers): `WM_QUERYENDSESSION`, asking whether the session may end, and `WM_ENDSESSION`,
//! saying it ends now or goes on after all. Both are sent to each top-level window, so they're
//! heard through a subclass of the window's own procedure, ahead of tao's (which ignores the
//! question). And the reason a window gives for stopping the end, which Windows shows on its
//! screen of apps that are (`ShutdownBlockReasonCreate`).
//!
//! The listener runs on the window's thread, whenever Windows sends: between the event loop's
//! turns, or while a modal dialog of the window's runs. A panic in it would cross into Windows,
//! so it mustn't panic.

use std::io;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    ENDSESSION_CRITICAL, ENDSESSION_LOGOFF, SendMessageW, WM_ENDSESSION, WM_NCDESTROY,
    WM_QUERYENDSESSION,
};
use windows::core::HSTRING;

/// What Windows said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heard {
    /// May the session end? `critical`: it ends whatever the answer.
    Query { critical: bool },
    /// It ends now (`true`), or goes on after all (`false`).
    Ended(bool),
}

type Listener = Box<dyn Fn(Heard) -> bool>;

/// The subclass's ID on a window ("LEGS").
const SUBCLASS: usize = 0x4C45_4753;

/// Has `listener` hear Windows ending the session, for `hwnd` (a top-level window of the
/// caller's own, on this thread), until the window is destroyed. For a query, its answer is
/// whether the session may end; for the end itself, it's ignored.
pub fn listen(hwnd: isize, listener: impl Fn(Heard) -> bool + 'static) -> io::Result<()> {
    let data = Box::into_raw(Box::new(Box::new(listener) as Listener)) as usize;
    // SAFETY: the procedure is a valid SUBCLASSPROC; `data` is a leaked Box<Listener> that
    // `procedure` frees as the window is destroyed (or freed here if the subclass fails).
    if unsafe { SetWindowSubclass(HWND(hwnd as _), Some(procedure), SUBCLASS, data) }.as_bool() {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        // SAFETY: `data` came from Box::into_raw above, and Windows didn't take it.
        drop(unsafe { Box::from_raw(data as *mut Listener) });
        Err(error)
    }
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    match message {
        WM_QUERYENDSESSION => {
            // SAFETY: `data` is the Box<Listener> `listen` leaked, alive until WM_NCDESTROY.
            let listener = unsafe { &*(data as *const Listener) };
            let critical = (lparam.0 as u32) & ENDSESSION_CRITICAL != 0;
            return LRESULT(isize::from(listener(Heard::Query { critical })));
        }
        WM_ENDSESSION => {
            // SAFETY: as above.
            let listener = unsafe { &*(data as *const Listener) };
            listener(Heard::Ended(wparam.0 != 0));
        }
        WM_NCDESTROY => {
            // SAFETY: the window is going: the subclass comes off, and nothing calls the
            // listener again, so the Box `listen` leaked is freed.
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(procedure), id);
                drop(Box::from_raw(data as *mut Listener));
            }
        }
        _ => {}
    }
    // SAFETY: the arguments are the ones Windows passed.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

/// The window stops the session's end, for `reason` (shown on Windows' screen of apps that are
/// stopping it), until [`go_on`].
pub fn stop(hwnd: isize, reason: &str) -> io::Result<()> {
    // SAFETY: a live HSTRING for the call, on the caller's own window.
    unsafe { ShutdownBlockReasonCreate(HWND(hwnd as _), &HSTRING::from(reason)) }
        .map_err(io::Error::from)
}

/// The window no longer stops the session's end.
pub fn go_on(hwnd: isize) {
    // SAFETY: the caller's own window; a window with no reason fails harmlessly.
    let _ = unsafe { ShutdownBlockReasonDestroy(HWND(hwnd as _)) };
}

/// Asks window `hwnd` (the caller's own) what Windows asks as the user signs out (`critical`:
/// a sign-out that can't be stopped): whether it lets the session end. For the test harness:
/// it never ends a session.
pub fn ask(hwnd: isize, critical: bool) -> bool {
    let flags = ENDSESSION_LOGOFF | if critical { ENDSESSION_CRITICAL } else { 0 };
    // SAFETY: a message to the caller's own window, which answers it on this thread.
    let answer = unsafe {
        SendMessageW(
            HWND(hwnd as _),
            WM_QUERYENDSESSION,
            Some(WPARAM(0)),
            Some(LPARAM(flags as isize)),
        )
    };
    answer.0 != 0
}

/// Tells window `hwnd` (the caller's own) the session goes on after all. For the test
/// harness: never that it ends.
pub fn going_on(hwnd: isize) {
    // SAFETY: a message to the caller's own window, which handles it on this thread.
    unsafe {
        SendMessageW(
            HWND(hwnd as _),
            WM_ENDSESSION,
            Some(WPARAM(0)),
            Some(LPARAM(ENDSESSION_LOGOFF as isize)),
        );
    }
}
