//! Thin, safe wrappers round the Win32 calls the window needs: the user and their language,
//! message boxes, opening links with the user's own apps, the foreground, the work area and
//! process times here; the single-instance pipe in [`pipe`].
//!
//! The only module (with its submodules) where `unsafe` is allowed: the workspace denies it
//! elsewhere. Each wrapper owns the buffers it passes to Windows, checks the result, copies
//! what comes back into Rust types and frees what Windows allocated, so callers never see a
//! pointer. Errors are `io::Error`s carrying the Win32 code (`raw_os_error()`).
#![allow(unsafe_code)]

pub mod actions;
pub mod clipboard;
pub mod dialogs;
pub mod jumplist;
pub mod menu;
pub mod pdf;
pub mod pipe;
pub mod registry;
pub mod replace;
pub mod request_filter;
pub mod session;

use std::io;

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HLOCAL, HWND, LocalFree, RECT};
use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetProcessTimes, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_PATHMUSTEXIST,
    OPENFILENAMEW,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, IDYES, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND,
    MB_YESNO, MESSAGEBOX_STYLE, MessageBoxW, SPI_GETWORKAREA, SW_SHOWNORMAL,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{Error, HSTRING, PCWSTR, PWSTR, w};

/// An `io::Error` from a `windows` error, with the Win32 code where it carries one.
pub(crate) fn io_error(error: &Error) -> io::Error {
    let hr = error.code().0 as u32;
    // A Win32 error wrapped as an HRESULT (facility 7) gives its code back; anything else stays.
    let code = if hr & 0xFFFF_0000 == 0x8007_0000 {
        hr & 0xFFFF
    } else {
        hr
    };
    io::Error::from_raw_os_error(code as i32)
}

/// `path` as an HSTRING, losslessly (a path needn't be valid Unicode).
pub(crate) fn wide(path: &std::path::Path) -> HSTRING {
    use std::os::windows::ffi::OsStrExt;
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    HSTRING::from_wide(&units)
}

// --- The user ----------------------------------------------------------------------------------

/// The current user's SID, as text (`S-1-5-21-…`).
pub fn current_user_sid() -> io::Result<String> {
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needn't be closed.
    process_user_sid(unsafe { GetCurrentProcess() })
}

/// The SID of the user running process `pid`, as text. Fails for a process this user may not
/// query (another user's, usually).
pub fn user_sid_of_process(pid: u32) -> io::Result<String> {
    // SAFETY: opens a handle that is closed below on every path.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|e| io_error(&e))?;
    let result = process_user_sid(process);
    // SAFETY: closes the process handle once.
    let _ = unsafe { CloseHandle(process) };
    result
}

fn process_user_sid(process: HANDLE) -> io::Result<String> {
    let mut token = HANDLE::default();
    // SAFETY: `token` is a live out pointer, and the token it receives is closed below on every
    // path.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(|e| io_error(&e))?;
    let result = token_user_sid(token);
    // SAFETY: closes the token handle once.
    let _ = unsafe { CloseHandle(token) };
    result
}

fn token_user_sid(token: HANDLE) -> io::Result<String> {
    let mut needed = 0u32;
    // SAFETY: a size query: no buffer, and `needed` is a live out pointer. It fails with
    // "insufficient buffer" by design, so its result is ignored.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    // u64s so the buffer is aligned for TOKEN_USER.
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8).max(1)];
    let length = (buffer.len() * 8) as u32;
    // SAFETY: the buffer is live, writable and `length` bytes long.
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            length,
            &mut needed,
        )
    }
    .map_err(|e| io_error(&e))?;
    let mut text = PWSTR::null();
    // SAFETY: on success the buffer holds a TOKEN_USER whose SID points into the same buffer,
    // which outlives the call; Windows allocates `text`, which is copied and freed once with
    // LocalFree.
    unsafe {
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        ConvertSidToStringSidW(user.User.Sid, &mut text).map_err(|e| io_error(&e))?;
        let sid = text.to_string().unwrap_or_default();
        LocalFree(Some(HLOCAL(text.0.cast())));
        Ok(sid)
    }
}

/// The Remote Desktop session this process runs in (0 for services, 1 or more for users).
pub fn current_session_id() -> io::Result<u32> {
    let mut session = 0u32;
    // SAFETY: GetCurrentProcessId takes nothing; `session` is a live out pointer.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) }
        .map_err(|e| io_error(&e))?;
    Ok(session)
}

/// The user's Windows display language as a tag (`en-GB`), or `None` if Windows doesn't say.
pub fn ui_language() -> Option<String> {
    // SAFETY: it takes nothing.
    let lcid = unsafe { GetUserDefaultUILanguage() };
    let mut buffer = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    // SAFETY: the slice gives the buffer and its length.
    let len = unsafe { LCIDToLocaleName(u32::from(lcid), Some(&mut buffer), 0) };
    let len = usize::try_from(len).ok()?.checked_sub(1)?;
    String::from_utf16(&buffer[..len])
        .ok()
        .filter(|s| !s.is_empty())
}

// --- Time --------------------------------------------------------------------------------------

/// Milliseconds since this process was created, by Windows' own record of its start: what
/// the start-up budgets are measured against.
pub fn process_age_ms() -> f64 {
    let zero = FILETIME::default();
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: a pseudo-handle and four live out pointers.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    }
    .is_ok();
    // SAFETY: it takes nothing.
    let now = unsafe { GetSystemTimePreciseAsFileTime() };
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    if !ok {
        return 0.0;
    }
    ticks(now).saturating_sub(ticks(created)) as f64 / 10_000.0
}

// --- The desktop -------------------------------------------------------------------------------

/// The primary monitor's work area (the screen less the taskbar), in physical pixels: width
/// and height.
pub fn primary_work_area() -> Option<(u32, u32)> {
    let mut rect = RECT::default();
    // SAFETY: SPI_GETWORKAREA writes one RECT to the live pointer given.
    unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&mut rect as *mut RECT).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()?;
    let width = u32::try_from(rect.right - rect.left).ok()?;
    let height = u32::try_from(rect.bottom - rect.top).ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

/// The work area (the monitor less the taskbar) of monitor `hmonitor`, in physical pixels on the
/// virtual screen: x, y, width and height.
pub fn monitor_work_area(hmonitor: isize) -> Option<(i32, i32, i32, i32)> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: a monitor handle Windows gave, and a MONITORINFO sized as the call asks.
    unsafe { GetMonitorInfoW(HMONITOR(hmonitor as _), &mut info) }
        .ok()
        .ok()?;
    let work = info.rcWork;
    let (width, height) = (work.right - work.left, work.bottom - work.top);
    (width > 0 && height > 0).then_some((work.left, work.top, width, height))
}

/// Who has the focus, for a test's check that its windows never took it: the calling thread's
/// active window and the window with the keyboard focus (its input queue's, WebView2's own
/// windows included), as handles, zero for none; and whether the foreground window is one of
/// this process's.
#[cfg(feature = "harness")]
pub fn focus_here() -> (isize, isize, bool) {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId,
    };
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: this thread's ID, and a GUITHREADINFO sized as the call asks; failing leaves it
    // zeroed, as for a thread with no windows.
    let _ = unsafe { GetGUIThreadInfo(GetCurrentThreadId(), &mut info) };
    let mut pid = 0;
    // SAFETY: GetForegroundWindow takes nothing; GetWindowThreadProcessId writes the ID to our
    // own u32 (and leaves it zero for no window).
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    // SAFETY: it takes nothing.
    let me = unsafe { GetCurrentProcessId() };
    (
        info.hwndActive.0 as isize,
        info.hwndFocus.0 as isize,
        pid != 0 && pid == me,
    )
}

/// What a [`FocusWatch`] has heard and not yet been asked for.
#[cfg(feature = "harness")]
static TAKEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Hears every moment a window of this process comes to the foreground, takes the keyboard
/// focus (WebView2's own windows in it included), or is shown able to be activated: for a
/// test's check that its windows never take the focus, not even for a moment between its
/// checks. Windows tells it on the thread that starts it, as that thread takes its messages.
/// Stops when dropped.
#[cfg(feature = "harness")]
pub struct FocusWatch(Vec<windows::Win32::UI::Accessibility::HWINEVENTHOOK>);

#[cfg(feature = "harness")]
impl FocusWatch {
    pub fn start() -> FocusWatch {
        use windows::Win32::UI::Accessibility::SetWinEventHook;
        use windows::Win32::UI::WindowsAndMessaging::{
            EVENT_OBJECT_FOCUS, EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT,
        };
        lock_taken().clear();
        // SAFETY: it takes nothing.
        let me = unsafe { GetCurrentProcessId() };
        // The foreground and the focus from every process (WebView2's windows are its
        // browser's), each kept only if its top-level window is this process's; shown windows
        // from this process alone.
        let hooks = [
            (EVENT_SYSTEM_FOREGROUND, 0),
            (EVENT_OBJECT_FOCUS, 0),
            (EVENT_OBJECT_SHOW, me),
        ]
        .into_iter()
        .map(|(event, process)| {
            // SAFETY: an out-of-context hook (no module) whose callback is a plain function
            // that lives as long as the program; it's removed when the watch is dropped.
            unsafe {
                SetWinEventHook(
                    event,
                    event,
                    None,
                    Some(heard),
                    process,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                )
            }
        })
        .filter(|hook| !hook.is_invalid())
        .collect();
        FocusWatch(hooks)
    }

    /// What it has heard since last asked, oldest first.
    pub fn taken() -> Vec<String> {
        std::mem::take(&mut *lock_taken())
    }

    /// How much it has heard since last asked, leaving it to be asked for.
    pub fn heard() -> usize {
        lock_taken().len()
    }
}

#[cfg(feature = "harness")]
impl Drop for FocusWatch {
    fn drop(&mut self) {
        for hook in self.0.drain(..) {
            // SAFETY: a hook SetWinEventHook gave, removed once.
            let _ = unsafe { windows::Win32::UI::Accessibility::UnhookWinEvent(hook) };
        }
    }
}

#[cfg(feature = "harness")]
fn lock_taken() -> std::sync::MutexGuard<'static, Vec<String>> {
    TAKEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A [`FocusWatch`]'s callback: what Windows says happened to window `hwnd`.
#[cfg(feature = "harness")]
unsafe extern "system" fn heard(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _child: i32,
    thread: u32,
    _time: u32,
) {
    use windows::Win32::System::Threading::{
        GetProcessIdOfThread, OpenThread, THREAD_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EVENT_OBJECT_FOCUS, EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND, GA_ROOT, GetAncestor,
        GetWindowThreadProcessId, OBJID_WINDOW,
    };
    if hwnd.is_invalid() {
        return;
    }
    // SAFETY: it takes nothing.
    let me = unsafe { GetCurrentProcessId() };
    // SAFETY: plain reads of a window Windows named; one that has gone gives none.
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    let mut pid = 0;
    // SAFETY: as above, the ID written to our own u32.
    unsafe { GetWindowThreadProcessId(root, Some(&mut pid)) };
    // The window's own process (the top-level window it's in, WebView2's included), or, for a
    // window gone by now, the process of the thread that told Windows.
    let ours = pid == me || {
        // SAFETY: a thread ID Windows gave, opened only to ask its process, and closed.
        unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, thread) }.is_ok_and(|handle| {
            // SAFETY: the handle just opened, then closed once.
            let process = unsafe { GetProcessIdOfThread(handle) };
            let _ = unsafe { CloseHandle(handle) };
            process == me
        })
    };
    if !ours {
        return;
    }
    let what = match event {
        EVENT_SYSTEM_FOREGROUND => format!("window {:#x} came to the foreground", hwnd.0 as isize),
        EVENT_OBJECT_FOCUS => format!("window {:#x} took the keyboard focus", hwnd.0 as isize),
        // Only a window still here can be asked its style; one shown and gone before Windows
        // told the watch can't be, but its taking the focus meanwhile is heard as itself.
        EVENT_OBJECT_SHOW
            if object == OBJID_WINDOW.0 && root == hwnd && !never_activates(hwnd.0 as isize) =>
        {
            format!(
                "window {:#x} was shown able to be activated",
                hwnd.0 as isize
            )
        }
        _ => return,
    };
    lock_taken().push(what);
}

/// Whether window `hwnd` can never be activated (`WS_EX_NOACTIVATE`), as a quiet one can't.
#[cfg(feature = "harness")]
pub fn never_activates(hwnd: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongW, WS_EX_NOACTIVATE};
    // SAFETY: it only reads the window's style; a window that has gone gives zero.
    let style = unsafe { GetWindowLongW(HWND(hwnd as _), GWL_EXSTYLE) } as u32;
    style & WS_EX_NOACTIVATE.0 != 0
}

/// Lets process `pid` bring its window to the front: the process the user just started (and
/// so may take the foreground) passes that right on to the one that will show the file. A
/// test's process ([`crate::quiet`]) passes nothing on.
pub fn allow_foreground(pid: u32) {
    if crate::quiet::on() {
        return;
    }
    // SAFETY: it takes a process ID; failing only means the window flashes in the taskbar.
    let _ = unsafe { AllowSetForegroundWindow(pid) };
}

/// Opens `target` (a URL: `https:`, `mailto:`) with the user's own app for it; in a test's
/// process ([`crate::quiet`]), opens nothing, as it would take the user's focus.
pub fn shell_open(target: &str) -> io::Result<()> {
    if crate::quiet::on() {
        tracing::info!(target, "quiet: not opened");
        return Ok(());
    }
    let target = HSTRING::from(target);
    // SAFETY: both strings are live and null-terminated; no window, folder or parameters.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &target,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute returns a value above 32 on success, an error code otherwise.
    let code = result.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

/// A message box's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    /// An error, with OK.
    Error,
    /// A question, with Yes and No.
    Ask,
}

/// Whether a dialog is left unshown, as cancelled: a quiet window's (a test's) never shows one,
/// which would take the user's focus and wait for their answer.
pub(crate) fn dialog_held_back(what: &str) -> bool {
    let held = crate::quiet::on();
    if held {
        tracing::info!(what, "quiet: not shown, taken as cancelled");
        #[cfg(feature = "harness")]
        HELD_BACK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(what.to_owned());
    }
    held
}

/// Every dialog left unshown so far, oldest first.
#[cfg(feature = "harness")]
static HELD_BACK: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The dialogs left unshown so far in this process (as "the Open dialog"), oldest first: for a
/// test's check that quiet windows show none.
#[cfg(feature = "harness")]
pub fn dialogs_held_back() -> Vec<String> {
    HELD_BACK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Shows a message box, owned by `owner` (a window handle) if one is given, and says whether
/// the user answered Yes. In a test's process ([`crate::quiet`]), shows nothing: No.
pub fn message_box(owner: Option<isize>, title: &str, text: &str, kind: Message) -> bool {
    if dialog_held_back("a message box") {
        return false;
    }
    let style: MESSAGEBOX_STYLE = match kind {
        Message::Error => MB_OK | MB_ICONERROR,
        Message::Ask => MB_YESNO | MB_ICONWARNING,
    } | MB_SETFOREGROUND;
    let owner = owner.map(|h| HWND(h as *mut _));
    let (title, text) = (HSTRING::from(title), HSTRING::from(text));
    // SAFETY: the strings are live and null-terminated; the owner is a window of this process
    // or none.
    let answer = unsafe { MessageBoxW(owner, &text, &title, style) };
    answer == IDYES
}

// --- The Open dialog ---------------------------------------------------------------------------

/// Windows' Open dialog, owned by `owner`, for Markdown files: `filter` names them (as "Markdown
/// files"). The chosen file, or None if the user cancelled (or, in a test's process, at once).
pub fn open_file_dialog(owner: isize, title: &str, filter: &str) -> Option<std::path::PathBuf> {
    if dialog_held_back("the Open dialog") {
        return None;
    }
    use std::os::windows::ffi::OsStringExt;
    // Pairs of (name, patterns), each null-terminated, then one more null.
    let filter: Vec<u16> = format!("{filter}\0*.md;*.markdown;*.mdown\0\0")
        .encode_utf16()
        .collect();
    let title = HSTRING::from(title);
    let mut file = vec![0u16; 32_768];
    let mut options = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: HWND(owner as *mut _),
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY,
        ..Default::default()
    };
    // SAFETY: every pointer is to a buffer this function owns and keeps alive for the call;
    // the file buffer's length is given in nMaxFile.
    let chosen = unsafe { GetOpenFileNameW(&mut options) }.as_bool();
    if !chosen {
        return None;
    }
    let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    Some(std::ffi::OsString::from_wide(&file[..len]).into())
}

// --- A file's identity ---------------------------------------------------------------------------

/// The volume's serial number and the file's index on it: the same for as long as the file
/// exists, and different for a file that replaced it, whatever its times and size.
/// [`file_identity`] as `legible_core::save` takes it (`save::Identity`).
pub fn identity(file: &std::fs::File) -> io::Result<Option<(u64, u64)>> {
    file_identity(file).map(|(volume, index)| Some((u64::from(volume), index)))
}

pub fn file_identity(file: &std::fs::File) -> io::Result<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is the open file's own, alive for the call; info is ours to fill.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(|e| io_error(&e))?;
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok((info.dwVolumeSerialNumber, index))
}
