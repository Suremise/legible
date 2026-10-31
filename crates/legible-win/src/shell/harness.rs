//! Drives the real shell in a test, in-process: the `harness` feature, which only test builds
//! turn on (legible-win's own dev-dependency on itself), never a release build.
//!
//! [`run`] runs the shell's own event loop on the calling thread, with Legible's folders where
//! the test says (a temp folder), and gives a [`Driver`] to a thread of its own. The driver opens
//! files, runs script in each window's own page, answers the window's dialogs, reads the copies
//! it made, holds the page's messages back (and a window's file, read, as it opens) and acts in
//! the moments of the window's writes
//! (another program writing the file between a save's check and its replace, say). Nothing is
//! typed or clicked: script runs in the shell's own WebViews only, so nothing reaches the
//! foreground, which is the user's.
//!
//! The shell stays out of the user's own state ([`super::StandIn`]): no app identity and no
//! Jump List (both are the user's, by the app's ID), no single-instance pipe, no real clipboard,
//! no dialogs or message boxes, and quiet windows ([`crate::quiet`]), which never take the focus
//! and open off every monitor. [`Driver::focus_kept`] checks they never took it.
//!
//! Every wait is for a condition, polled, and bounded by [`BOUND`] for a hung run.

use std::collections::VecDeque;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use legible_core::save::{Hooks, Stamp};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::{EventLoopBuilderExtWindows, WindowExtWindows};
use tao::window::WindowId;

use super::{From, Moments, Read, Shell, StandIn, UserEvent, handler};
use crate::clipboard::Written;
use crate::menu::Command;
use crate::paths::AppPaths;

pub use super::Write;
pub use crate::win32::dialogs::SaveAnswer;

/// How long any wait may take before the run is taken as hung.
pub const BOUND: Duration = Duration::from_secs(30);
/// How often a condition is looked at again.
const POLL: Duration = Duration::from_millis(50);

/// What a test's driver gets back: why it went wrong, said.
pub type Result<T> = std::result::Result<T, String>;

/// A moment of a write ([`Hooks`]), for [`Driver::at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moment {
    /// Its bytes are staged beside the file.
    Staged,
    /// The file has just been checked against its stamp, and is about to be replaced.
    Checked,
    /// The write is over; the window hasn't heard yet.
    Over,
}

/// One of the shell's windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window(WindowId);

type Act = Box<dyn FnOnce() + Send>;
type Work = Box<dyn FnOnce(&mut Shell, &EventLoopWindowTarget<UserEvent>, &mut ControlFlow) + Send>;

/// Something the driver asks of the shell, done on its event loop's thread.
pub(super) struct Call(Work);

impl Call {
    pub(super) fn run(
        self,
        shell: &mut Shell,
        target: &EventLoopWindowTarget<UserEvent>,
        flow: &mut ControlFlow,
    ) {
        (self.0)(shell, target, flow);
    }
}

impl std::fmt::Debug for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Call")
    }
}

/// What the stand-in and the driver share.
#[derive(Default)]
struct State {
    /// The window's questions' answers (the close question, or what to do with unsaved edits
    /// kept for a file that has gone), in turn (Cancel once they're used up), and how many have
    /// been asked.
    answers: VecDeque<SaveAnswer>,
    questions: usize,
    /// The files Save as's dialog chooses, in turn (cancelled once they're used up).
    save_as_to: VecDeque<PathBuf>,
    /// What message boxes said.
    messages: Vec<String>,
    /// The text of each copy written to the clipboard.
    clipboard: Vec<String>,
    /// What to do in a moment of the next write of a kind.
    acts: Vec<(Write, Moment, Act)>,
    /// The beginnings of page messages to hold back, each once, and those held, each with the
    /// page it came from; and those handed to the driver, till it releases them.
    holds: Vec<String>,
    held: VecDeque<(WindowId, From, String)>,
    handed: Vec<(WindowId, From, String)>,
    /// The next window's file, read as it opens, is held back; and those held, oldest first.
    hold_opened: bool,
    held_opened: VecDeque<(WindowId, Read)>,
    /// The driver is done: the shell exits once its windows are closed.
    done: bool,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The shell's stand-in, on its event loop's thread.
struct Host(Arc<Mutex<State>>);

impl StandIn for Host {
    fn answer(&self) -> SaveAnswer {
        let mut state = lock(&self.0);
        state.questions += 1;
        state.answers.pop_front().unwrap_or(SaveAnswer::Cancel)
    }

    fn save_as_to(&self) -> Option<PathBuf> {
        lock(&self.0).save_as_to.pop_front()
    }

    fn message(&self, text: &str) -> bool {
        lock(&self.0).messages.push(text.to_owned());
        false
    }

    fn clipboard(&self, _html: &[u8], text: &str) -> Written {
        lock(&self.0).clipboard.push(text.to_owned());
        Written::Both
    }

    fn moments(&self, write: Write) -> Arc<dyn Moments> {
        let mut state = lock(&self.0);
        let (mine, others) = std::mem::take(&mut state.acts)
            .into_iter()
            .partition(|(w, ..)| *w == write);
        state.acts = others;
        let acts = WriteActs::default();
        for (_, moment, act) in mine {
            let slot = match moment {
                Moment::Staged => &acts.staged,
                Moment::Checked => &acts.checked,
                Moment::Over => &acts.over,
            };
            slot.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(act);
        }
        Arc::new(acts)
    }

    fn holds(&self, window: WindowId, from: From, message: &str) -> bool {
        let mut state = lock(&self.0);
        let Some(at) = state
            .holds
            .iter()
            .position(|p| message.starts_with(p.as_str()))
        else {
            return false;
        };
        state.holds.remove(at);
        state.held.push_back((window, from, message.to_owned()));
        true
    }

    fn holds_opened(&self, window: WindowId, read: Read) -> Option<Read> {
        let mut state = lock(&self.0);
        if !std::mem::take(&mut state.hold_opened) {
            return Some(read);
        }
        state.held_opened.push_back((window, read));
        None
    }

    fn stays(&self) -> bool {
        !lock(&self.0).done
    }
}

/// What one write does in its moments: each act once (a write checks again before each try).
#[derive(Default)]
struct WriteActs {
    staged: Mutex<Vec<Act>>,
    checked: Mutex<Vec<Act>>,
    over: Mutex<Vec<Act>>,
}

impl WriteActs {
    fn run(slot: &Mutex<Vec<Act>>) {
        let acts = std::mem::take(
            &mut *slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for act in acts {
            act();
        }
    }
}

impl Hooks for WriteActs {
    fn staged(&self, _staged: &Path) {
        Self::run(&self.staged);
    }

    fn checked(&self) {
        Self::run(&self.checked);
    }
}

impl Moments for WriteActs {
    fn over(&self) {
        Self::run(&self.over);
    }
}

/// Runs the shell, with Legible's folders at `paths` and its page in `lang`, while `drive`
/// drives it from a thread of its own; returns what `drive` did once the shell has exited (its
/// windows are closed as `drive` returns, whatever they hold). A panic in `drive` is resumed
/// here, after the shell has exited.
///
/// The event loop runs on the calling thread, which needn't be the process's main thread (a
/// test's isn't). One shell at a time in a process.
pub fn run<R: Send + 'static>(
    paths: AppPaths,
    lang: &str,
    drive: impl FnOnce(&Driver) -> R + Send + 'static,
) -> R {
    crate::quiet::enable();
    let mut builder = EventLoopBuilder::<UserEvent>::with_user_event();
    builder.with_any_thread(true);
    let mut event_loop = builder.build();
    let state = Arc::new(Mutex::new(State::default()));
    let host: Rc<dyn StandIn> = Rc::new(Host(state.clone()));
    let shell = Shell::new(
        Some(paths),
        lang.to_owned(),
        event_loop.create_proxy(),
        Some(host),
    );
    let driver = Driver {
        proxy: event_loop.create_proxy(),
        state,
    };
    let driving = thread::spawn(move || {
        let result = panic::catch_unwind(AssertUnwindSafe(|| drive(&driver)));
        driver.finish();
        result
    });
    // Hears every moment a window takes the focus, on this thread, for [`Driver::focus_kept`].
    let watch = crate::quiet::watch_focus();
    event_loop.run_return(handler(shell, None));
    drop(watch);
    match driving.join() {
        Ok(Ok(result)) => result,
        Ok(Err(panicked)) | Err(panicked) => panic::resume_unwind(panicked),
    }
}

thread_local! {
    /// [`Driver::show_a_window_that_could_be_activated`]'s window, on the shell's thread.
    static ACTIVATABLE: std::cell::RefCell<Option<tao::window::Window>> =
        const { std::cell::RefCell::new(None) };
}

/// A test's hold on the shell, from the thread [`run`] gives it.
pub struct Driver {
    proxy: EventLoopProxy<UserEvent>,
    state: Arc<Mutex<State>>,
}

impl Driver {
    /// Does `work` on the shell's thread; its answer.
    fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Shell, &EventLoopWindowTarget<UserEvent>, &mut ControlFlow) -> T
        + Send
        + 'static,
    ) -> Result<T> {
        let (answer, answered) = mpsc::channel();
        let call = Call(Box::new(move |shell, target, flow| {
            let _ = answer.send(work(shell, target, flow));
        }));
        self.proxy
            .send_event(UserEvent::Harness(call))
            .map_err(|_| "the shell has stopped".to_owned())?;
        answered
            .recv_timeout(BOUND)
            .map_err(|_| "the shell didn't answer".to_owned())
    }

    /// Opens `file` as any way of opening one does, and gives its window.
    pub fn open(&self, file: &Path) -> Result<Window> {
        let file = file.to_owned();
        let shown = file.display().to_string();
        self.call(move |shell, target, _| {
            shell.open(&file, target);
            std::fs::canonicalize(&file)
                .ok()
                .and_then(|canonical| shell.by_file.get(&canonical).copied())
        })?
        .map(Window)
        .ok_or_else(|| format!("{shown} didn't open"))
    }

    /// Whether the user still has the focus wherever they had it: since this was last asked, no
    /// window of the run (nor its WebView2's) came to the foreground, took the keyboard focus
    /// or was shown able to be activated, even for a moment; and now none is active or has the
    /// keyboard focus, the foreground isn't this process's, and every window is one that can't
    /// be activated. Why not, if not.
    pub fn focus_kept(&self) -> Result<()> {
        let (taken, activatable) = self.call(|shell, _, _| {
            let activatable = shell
                .windows
                .values()
                .filter(|d| !crate::win32::never_activates(d.window.hwnd()))
                .count();
            (crate::quiet::focus_taken(), activatable)
        })?;
        match (taken, activatable) {
            (None, 0) => Ok(()),
            (taken, 0) => Err(taken.unwrap_or_default()),
            (taken, n) => Err(format!(
                "{}{n} window(s) could be activated",
                taken.map_or_else(String::new, |t| format!("{t}; "))
            )),
        }
    }

    /// Whether the window has a Save as PDF under way: choosing where, printing or committing.
    pub fn saving_pdf(&self, window: Window) -> Result<bool> {
        self.call(move |shell, _, _| shell.pdf_jobs.running(window.0))
    }

    /// Shows a window of the run's own, off every monitor and not activated, but one that could
    /// be, till [`Driver::close_the_window_that_could_be_activated`]: for a check that
    /// [`Driver::focus_kept`] hears what happens between its checks.
    pub fn show_a_window_that_could_be_activated(&self) -> Result<()> {
        self.call(|_, target, _| {
            use tao::platform::windows::WindowBuilderExtWindows;
            let mut builder = tao::window::WindowBuilder::new()
                .with_title("Legible: focus check")
                .with_focused(false)
                .with_skip_taskbar(true);
            if let Some((x, y)) = crate::quiet::off_screen(target) {
                builder = builder.with_position(tao::dpi::PhysicalPosition::new(x, y));
            }
            let window = builder.build(target).map_err(|e| e.to_string())?;
            ACTIVATABLE.with(|w| *w.borrow_mut() = Some(window));
            Ok(())
        })?
    }

    pub fn close_the_window_that_could_be_activated(&self) -> Result<()> {
        self.call(|_, _, _| ACTIVATABLE.with(|w| w.borrow_mut().take().map(drop)))
            .map(drop)
    }

    /// How much the focus check has heard since it was last asked, leaving it to be asked.
    pub fn focus_heard(&self) -> Result<usize> {
        self.call(|_, _, _| crate::win32::FocusWatch::heard())
    }

    /// The window `file` is open in, if it is.
    pub fn window_of(&self, file: &Path) -> Result<Option<Window>> {
        let file = file.to_owned();
        self.call(move |shell, _, _| {
            std::fs::canonicalize(&file)
                .ok()
                .and_then(|canonical| shell.by_file.get(&canonical).copied())
                .map(Window)
        })
    }

    pub fn is_open(&self, window: Window) -> Result<bool> {
        self.call(move |shell, _, _| shell.windows.contains_key(&window.0))
    }

    /// How many windows are open.
    pub fn windows(&self) -> Result<usize> {
        self.call(|shell, _, _| shell.windows.len())
    }

    /// The file the window shows (None: it has none, or it's closed).
    pub fn file_of(&self, window: Window) -> Result<Option<PathBuf>> {
        self.call(move |shell, _, _| shell.windows.get(&window.0)?.file.clone())
    }

    /// The window's title, as its title bar shows it.
    pub fn title(&self, window: Window) -> Result<Option<String>> {
        self.call(move |shell, _, _| shell.windows.get(&window.0).map(|d| d.window.title()))
    }

    /// Opens `file` in the window, in place of its document, as Open and Open recent do.
    pub fn open_here(&self, window: Window, file: &Path) -> Result<()> {
        let file = file.to_owned();
        self.call(move |shell, _, _| shell.open_here(window.0, &file))
    }

    /// Drops `files` on the window, as they would be dragged there from Explorer.
    pub fn drop_on(&self, window: Window, files: Vec<PathBuf>) -> Result<()> {
        self.proxy
            .send_event(UserEvent::Dropped(window.0, files))
            .map_err(|_| "the shell has stopped".to_owned())
    }

    /// `files` handed on from a later launch, as the single-instance pipe hands them on.
    pub fn launch(&self, files: Vec<PathBuf>) -> Result<()> {
        self.proxy
            .send_event(UserEvent::Open(files))
            .map_err(|_| "the shell has stopped".to_owned())
    }

    /// Closes the window as its title bar's Close does: with unsaved edits, it asks.
    pub fn close(&self, window: Window) -> Result<()> {
        self.call(move |shell, _, _| shell.ask_close(window.0))
    }

    /// The newest render the window has sent its page.
    pub fn generation(&self, window: Window) -> Result<Option<u64>> {
        self.call(move |shell, _, _| shell.windows.get(&window.0).map(|d| d.generation))
    }

    /// The file as the window's next save expects to find it.
    pub fn stamp(&self, window: Window) -> Result<Option<Stamp>> {
        self.call(move |shell, _, _| shell.windows.get(&window.0).and_then(|d| d.expected))
    }

    /// The page the window serves now.
    pub fn served(&self, window: Window) -> Result<Option<String>> {
        self.call(move |shell, _, _| {
            shell.windows.get(&window.0).map(|d| {
                let served = d.served.lock().unwrap_or_else(|e| e.into_inner());
                String::from_utf8_lossy(&served.html).into_owned()
            })
        })
    }

    /// Whether a reload of the window's page waits for its edits to be saved.
    pub fn reload_waits(&self, window: Window) -> Result<bool> {
        self.call(move |shell, _, _| {
            shell
                .windows
                .get(&window.0)
                .is_some_and(|d| d.reload_gate.waiting())
        })
    }

    /// Asks the window what Windows asks as the user signs out (`critical`: a sign-out that
    /// can't be stopped), by a message to its own handle: whether it lets the session end.
    /// Never a real end of the session.
    pub fn ask_to_end_session(&self, window: Window, critical: bool) -> Result<bool> {
        self.call(move |shell, _, _| {
            shell
                .windows
                .get(&window.0)
                .map(|d| crate::win32::session::ask(d.window.hwnd(), critical))
        })?
        .ok_or_else(|| "the window has gone".to_owned())
    }

    /// Offers back the copies of unsaved edits kept in Legible's folder, as the shell does as it
    /// starts.
    pub fn offer_kept(&self) -> Result<()> {
        self.call(|shell, target, _| shell.offer_kept(target))
    }

    /// Tells the window, by a message to its own handle, that the session goes on after all.
    pub fn session_goes_on(&self, window: Window) -> Result<()> {
        self.call(move |shell, _, _| {
            if let Some(d) = shell.windows.get(&window.0) {
                crate::win32::session::going_on(d.window.hwnd());
            }
        })
    }

    /// Where the reader is, as the window has it from its page ([`crate::places`]), as JSON.
    pub fn place(&self, window: Window) -> Result<Option<String>> {
        self.call(move |shell, _, _| {
            shell
                .windows
                .get(&window.0)
                .and_then(|d| d.place.as_ref())
                .map(|p| p.to_json())
        })
    }

    /// The recent files, newest first.
    pub fn recent(&self) -> Result<Vec<PathBuf>> {
        self.call(|shell, _, _| shell.recent.paths().to_vec())
    }

    /// How many files' reading places the shell keeps.
    pub fn places_kept(&self) -> Result<usize> {
        self.call(|shell, _, _| shell.places.paths().len())
    }

    /// The window's outer top-left, in physical pixels.
    pub fn position(&self, window: Window) -> Result<Option<(i32, i32)>> {
        self.call(move |shell, _, _| {
            shell
                .windows
                .get(&window.0)
                .and_then(|d| d.window.outer_position().ok())
                .map(|p| (p.x, p.y))
        })
    }

    /// Whether any of the window is on a monitor: None if it has gone.
    pub fn on_a_monitor(&self, window: Window) -> Result<Option<bool>> {
        self.call(move |shell, _, _| {
            shell
                .windows
                .get(&window.0)
                .map(|d| crate::quiet::on_a_monitor(&d.window))
        })
    }

    /// The window's whole outer rectangle (its frame, title and menu bar included), in physical
    /// pixels: x, y, width and height.
    pub fn outer_rect(&self, window: Window) -> Result<Option<(i32, i32, i32, i32)>> {
        self.call(move |shell, _, _| {
            let d = shell.windows.get(&window.0)?;
            let at = d.window.outer_position().ok()?;
            let size = d.window.outer_size();
            Some((at.x, at.y, size.width as i32, size.height as i32))
        })
    }

    /// The window's inner size, in logical pixels.
    pub fn size(&self, window: Window) -> Result<Option<(f64, f64)>> {
        self.call(move |shell, _, _| {
            shell.windows.get(&window.0).map(|d| {
                let size = d
                    .window
                    .inner_size()
                    .to_logical::<f64>(d.window.scale_factor());
                (size.width, size.height)
            })
        })
    }

    /// Moves the window (its outer top-left, physical pixels) and sizes it (logical), as the
    /// reader might: the window's own, never by input.
    pub fn place_window(&self, window: Window, at: (i32, i32), size: (f64, f64)) -> Result<()> {
        self.call(move |shell, _, _| {
            if let Some(d) = shell.windows.get(&window.0) {
                d.window
                    .set_outer_position(tao::dpi::PhysicalPosition::new(at.0, at.1));
                d.window
                    .set_inner_size(tao::dpi::LogicalSize::new(size.0, size.1));
            }
        })
    }

    /// The primary monitor's work area: x, y, width and height, in physical pixels.
    pub fn primary_work_area(&self) -> Result<Option<(i32, i32, i32, i32)>> {
        self.call(|_, target, _| {
            use tao::platform::windows::MonitorHandleExtWindows;
            target
                .primary_monitor()
                .and_then(|m| crate::win32::monitor_work_area(m.hmonitor()))
        })
    }

    /// Runs `script` in the window's page: its result as JSON, or None if the window has gone.
    pub fn eval(&self, window: Window, script: &str) -> Result<Option<String>> {
        let (answer, answered) = mpsc::channel();
        let script = script.to_owned();
        let sent = self.call(move |shell, _, _| match shell.windows.get(&window.0) {
            Some(doc) => doc
                .webview
                .evaluate_script_with_callback(&script, move |result| {
                    let _ = answer.send(result);
                })
                .is_ok(),
            None => false,
        })?;
        if !sent {
            return Ok(None);
        }
        match answered.recv_timeout(BOUND) {
            Ok(result) => Ok(Some(result)),
            Err(RecvTimeoutError::Disconnected) => Ok(None),
            Err(RecvTimeoutError::Timeout) => Err("the page didn't answer".to_owned()),
        }
    }

    /// Waits until `condition`, a JavaScript expression, is true in the window's page (one that
    /// throws counts as false). `what` names it if it never is.
    pub fn until(&self, window: Window, what: &str, condition: &str) -> Result<()> {
        let script =
            format!("(() => {{ try {{ return !!({condition}); }} catch {{ return false; }} }})()");
        self.until_true(what, || {
            Ok(self.eval(window, &script)?.as_deref() == Some("true"))
        })
    }

    /// Waits until `check` says true; an error from it ends the wait.
    pub fn until_true(&self, what: &str, mut check: impl FnMut() -> Result<bool>) -> Result<()> {
        let end = Instant::now() + BOUND;
        loop {
            if check()? {
                return Ok(());
            }
            if Instant::now() > end {
                return Err(format!("timed out waiting for {what}"));
            }
            thread::sleep(POLL);
        }
    }

    /// Does `act` in `moment` of the next write of kind `write` to start, on its thread: a
    /// write of another program's, say, or a wait that keeps the write under way.
    pub fn at(&self, write: Write, moment: Moment, act: impl FnOnce() + Send + 'static) {
        lock(&self.state).acts.push((write, moment, Box::new(act)));
    }

    /// The next question's answer: the close question's, or the one for unsaved edits kept for
    /// a file that has gone.
    pub fn answer(&self, answer: SaveAnswer) {
        lock(&self.state).answers.push_back(answer);
    }

    /// How many questions the windows have asked.
    pub fn questions(&self) -> usize {
        lock(&self.state).questions
    }

    /// The file Save as's dialog chooses next.
    pub fn save_as_to(&self, file: &Path) {
        lock(&self.state).save_as_to.push_back(file.to_owned());
    }

    /// The text of each copy written to the clipboard (the harness's own), oldest first.
    pub fn clipboard(&self) -> Vec<String> {
        lock(&self.state).clipboard.clone()
    }

    /// What each message box said, oldest first.
    pub fn messages(&self) -> Vec<String> {
        lock(&self.state).messages.clone()
    }

    /// Holds back the next message from a page that starts with `prefix`, until
    /// [`Driver::release`]: the moment between the page's message and the window's hearing it.
    pub fn hold(&self, prefix: &str) {
        lock(&self.state).holds.push(prefix.to_owned());
    }

    /// Waits for a held message: its window, and the message.
    pub fn held(&self, what: &str) -> Result<(Window, String)> {
        let mut found = None;
        self.until_true(what, || {
            let mut state = lock(&self.state);
            found = state.held.pop_front();
            if let Some(held) = &found {
                state.handed.push(held.clone());
            }
            Ok(found.is_some())
        })?;
        let (window, _, message) = found.ok_or_else(|| format!("no {what}"))?;
        Ok((Window(window), message))
    }

    /// The window hears a held message, as from the page that sent it; or any the page might
    /// send, as from the page it has now.
    pub fn release(&self, window: Window, message: String) -> Result<()> {
        let from = {
            let mut state = lock(&self.state);
            let at = state
                .handed
                .iter()
                .position(|(w, _, m)| *w == window.0 && *m == message);
            at.and_then(|at| state.handed.remove(at).1)
        };
        self.proxy
            .send_event(UserEvent::Chrome(window.0, from, message))
            .map_err(|_| "the shell has stopped".to_owned())
    }

    /// Whether the window is still reading its file.
    pub fn is_opening(&self, window: Window) -> Result<bool> {
        self.call(move |shell, _, _| {
            shell
                .windows
                .get(&window.0)
                .is_some_and(|d| d.opening.is_some())
        })
    }

    /// A window with no document, if one is open.
    pub fn empty_window(&self) -> Result<Option<Window>> {
        self.call(|shell, _, _| {
            shell
                .windows
                .iter()
                .find(|(_, doc)| doc.file.is_none())
                .map(|(id, _)| Window(*id))
        })
    }

    /// Holds back the next window's file, read as it opens, until [`Driver::finish_opening`]:
    /// the window stays as it is while its file is read, with no page.
    pub fn hold_opening(&self) {
        lock(&self.state).hold_opened = true;
    }

    /// Waits for the file held as its window opens: its window (the newest's, with more held).
    pub fn opening_held(&self) -> Result<Window> {
        self.openings_held(1)
    }

    /// Waits till `count` files are held as their windows open: the newest's window.
    pub fn openings_held(&self, count: usize) -> Result<Window> {
        let mut found = None;
        self.until_true("a file read as its window opens", || {
            let state = lock(&self.state);
            found = state
                .held_opened
                .back()
                .filter(|_| state.held_opened.len() >= count)
                .map(|(w, _)| *w);
            Ok(found.is_some())
        })?;
        found
            .map(Window)
            .ok_or_else(|| "no file held as its window opens".to_owned())
    }

    /// The window hears the oldest file held: its page is served, or it closes.
    pub fn finish_opening(&self) -> Result<()> {
        let (window, read) = lock(&self.state)
            .held_opened
            .pop_front()
            .ok_or_else(|| "no file held as its window opens".to_owned())?;
        self.proxy
            .send_event(UserEvent::Opened(window, read))
            .map_err(|_| "the shell has stopped".to_owned())
    }

    /// The commands the window's menu bar offers now (its items enabled), in the bar's order.
    pub fn offered(&self, window: Window) -> Result<Vec<Command>> {
        self.call(move |shell, _, _| {
            shell
                .windows
                .get(&window.0)
                .and_then(|d| d.bar.as_ref())
                .map(|bar| bar.offered())
        })?
        .ok_or_else(|| "the window has no menu bar".to_owned())
    }

    /// The driver is done: every window closes, whatever it holds, and the shell exits.
    fn finish(&self) {
        lock(&self.state).done = true;
        let _ = self.call(|shell, _, _| {
            let ids: Vec<WindowId> = shell.windows.keys().copied().collect();
            for id in ids {
                shell.close_now(id);
            }
        });
    }
}
