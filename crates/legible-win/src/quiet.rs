//! Quiet windows, for tests: windows that never take the user's focus or catch their eye while
//! they work in another app as the tests run.
//!
//! A quiet window is made without activating and can't be activated (`WS_EX_NOACTIVATE`), has
//! no taskbar button, and opens off every monitor at its normal size (WebView2 still lays it
//! out and paints it, so its page runs and can be captured), unless a test places it itself.
//! Its WebView2 never asks for the keyboard focus. Nothing brings it forward, restores it or
//! flashes it, nothing launches another app (a browser, Explorer, an editor) from it, and no
//! dialog or message box is shown (each is taken as cancelled). `watch_focus` and `focus_taken`
//! check none of that ever happened.
//!
//! Quiet only exists in a test build: the `harness` feature, which only tests and checks turn
//! on, never a build of the exe. There, it's on once [`enable`] is called (the harness, each
//! example and check that opens a window) or when the environment variable [`VAR`] is `1`
//! (for a process a test starts), read once. In any other build it's always off, whatever the
//! environment says.

use std::ffi::OsStr;
use std::sync::OnceLock;

use tao::dpi::PhysicalPosition;
use tao::event_loop::EventLoopWindowTarget;
use tao::platform::windows::WindowBuilderExtWindows;
use tao::window::{Window, WindowBuilder};

/// The environment variable that makes a test build's windows quiet: `1`.
pub const VAR: &str = "LEGIBLE_TEST_QUIET";

#[cfg(feature = "harness")]
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Makes every window this process opens from now on quiet: for a test's own process, as it
/// starts.
#[cfg(feature = "harness")]
pub fn enable() {
    ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Starts hearing every moment a window of this process takes the focus ([`focus_taken`]),
/// on this thread, which must be the one that takes the windows' messages: for a check of a
/// window of its own (an example's), as it starts. Stops when dropped.
#[cfg(feature = "harness")]
pub fn watch_focus() -> crate::win32::FocusWatch {
    crate::win32::FocusWatch::start()
}

/// Why the user's focus wasn't left alone, if it wasn't: since this was last asked (or the
/// watch started), a window of this process (or its WebView2's) came to the foreground, took
/// the keyboard focus or was shown able to be activated, even for a moment; or now one of this
/// thread's windows is active or has the keyboard focus, or the foreground is this process's.
/// Asked on the thread that watches.
#[cfg(feature = "harness")]
pub fn focus_taken() -> Option<String> {
    let mut why = crate::win32::FocusWatch::taken();
    let (active, focus, foreground) = crate::win32::focus_here();
    if active != 0 {
        why.push(format!("window {active:#x} is active"));
    }
    if focus != 0 {
        why.push(format!("window {focus:#x} has the keyboard focus"));
    }
    if foreground {
        why.push("a window of this process is in the foreground".to_owned());
    }
    (!why.is_empty()).then(|| format!("the focus was taken: {}", why.join("; ")))
}

/// Whether this process's windows are quiet.
pub fn on() -> bool {
    static FROM_ENV: OnceLock<bool> = OnceLock::new();
    enabled()
        || *FROM_ENV
            .get_or_init(|| decide(std::env::var_os(VAR).as_deref(), cfg!(feature = "harness")))
}

#[cfg(feature = "harness")]
fn enabled() -> bool {
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(not(feature = "harness"))]
fn enabled() -> bool {
    false
}

/// Whether [`VAR`]'s value (None: not set) makes windows quiet, in a build that is a test's or
/// not.
fn decide(value: Option<&OsStr>, test_build: bool) -> bool {
    test_build && value == Some(OsStr::new("1"))
}

/// `builder`'s window, for a check of a window of its own (an example's) on `target`: never
/// focused as it opens, and quiet if this process's windows are.
pub fn window<T: 'static>(
    builder: WindowBuilder,
    target: &EventLoopWindowTarget<T>,
) -> WindowBuilder {
    let builder = builder.with_focused(false);
    if !on() {
        return builder;
    }
    let builder = builder
        .with_focusable(false)
        .with_skip_taskbar(true)
        .with_maximized(false);
    match off_screen(target) {
        Some((x, y)) => builder.with_position(PhysicalPosition::new(x, y)),
        None => builder,
    }
}

/// Where a quiet window opens, its outer top-left in physical pixels: just past the right
/// edge of the monitor furthest right, at its top, so it's off every monitor. None with no
/// monitor.
pub fn off_screen<T: 'static>(target: &EventLoopWindowTarget<T>) -> Option<(i32, i32)> {
    target
        .available_monitors()
        .map(|m| {
            let (at, size) = (m.position(), m.size());
            (at.x.saturating_add(size.width as i32), at.y)
        })
        .max_by_key(|&(right, _)| right)
}

/// Whether any of `window` is on a monitor.
pub fn on_a_monitor(window: &Window) -> bool {
    let Ok(at) = window.outer_position() else {
        return false;
    };
    let size = window.outer_size();
    let (right, bottom) = (
        at.x.saturating_add(size.width as i32),
        at.y.saturating_add(size.height as i32),
    );
    window.available_monitors().any(|m| {
        let (m_at, m_size) = (m.position(), m.size());
        at.x < m_at.x.saturating_add(m_size.width as i32)
            && m_at.x < right
            && at.y < m_at.y.saturating_add(m_size.height as i32)
            && m_at.y < bottom
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_only_when_set_to_one_in_a_test_build() {
        assert!(decide(Some(OsStr::new("1")), true));
        assert!(!decide(None, true));
        assert!(!decide(Some(OsStr::new("")), true));
        assert!(!decide(Some(OsStr::new("0")), true));
        assert!(!decide(Some(OsStr::new("yes")), true));
    }

    #[test]
    fn a_build_that_isnt_a_tests_ignores_it() {
        assert!(!decide(Some(OsStr::new("1")), false));
        assert!(!decide(None, false));
    }
}
