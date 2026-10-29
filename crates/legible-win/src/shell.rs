//! The document windows: one per file, all on one event loop in one process.
//!
//! Each window is a tao window holding a WebView2 (through wry) that shows [`page::page`]. The
//! page can't navigate: its links come to [`Shell::follow`] and are decided by
//! [`links::classify`]; every navigation but Legible's own page is refused, and so is every
//! new window. Release builds turn off the developer tools (and so the context menu's
//! "Inspect") and the browser's own shortcut keys (reload, devtools, find, print).

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use legible_core::render::{RenderOptions, render, render_with};
use legible_core::save::{Hooks, NoHooks, Stamp, WriteError};
use legible_core::{assets, i18n::tr, tr, unsaved};
use tao::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::platform::windows::{
    IconExtWindows, MonitorHandleExtWindows, WindowBuilderExtWindows, WindowExtWindows,
};
use tao::window::{Icon, Window, WindowBuilder, WindowId};
use wry::http::{Request, Response, StatusCode, header};
use wry::{
    DragDropEvent, NewWindowResponse, WebContext, WebView, WebViewBuilder,
    WebViewBuilderExtWindows, WebViewExtWindows,
};

use crate::actions::{self, Actions};
use crate::associations;
use crate::clipboard::{self, Built, Copier, Order, Ticket, Written};
use crate::confine::DocRoot;
use crate::consent::{self, AlwaysList, Next, Pictures, Unavailable};
use crate::disk::{Content, Disk};
use crate::editor;
use crate::export;
use crate::jumplist;
use crate::links::{self, LinkAction};
use crate::menu::{self, Bar, Exit};
use crate::paths::AppPaths;
use crate::placement::{self, Placement, Size};
use crate::places::{self, Place, Places};
use crate::reason;
use crate::recent::{self, Checker, Recent};
use crate::recovery;
use crate::reload::{self, Action, FileBar, Kind};
use crate::saving;
use crate::session::{self, Answer, Round};
use crate::settings::{Setting, Settings, json_string};
use crate::watch::{Report, Snapshot, Watch, Watched};
use crate::win32::session::Heard;
use crate::win32::{self, Message, dialogs};
use crate::{instance, page, quiet};

mod about;
#[cfg(feature = "harness")]
pub mod harness;
mod kept;
mod menus;
mod save;
mod status_bar;

/// Microsoft's page for the WebView2 runtime, shown (never fetched) when it's missing.
pub const WEBVIEW2_URL: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

/// The app icon's resource ID in legible.exe (its build script embeds `assets/icon/legible.ico`).
pub const APP_ICON: u16 = 1;

/// The folder, inside Legible's, for WebView2's own data.
const WEBVIEW2_DIR: &str = "WebView2";

/// What the shell starts with.
pub struct Options {
    /// Legible's folders, if they could be worked out.
    pub paths: Option<AppPaths>,
    /// The files to open, from the command line.
    pub files: Vec<PathBuf>,
    /// The single-instance pipe, if this process holds it.
    pub primary: Option<instance::Primary>,
    /// The user's language, for the page.
    pub lang: String,
}

/// What the event loop is told from elsewhere.
#[derive(Debug)]
enum UserEvent {
    /// Files from a later launch (none: it was started without one).
    Open(Vec<PathBuf>),
    /// A link the page's script sent.
    Link(WindowId, From, String),
    /// The window's file, read and rendered on a thread of its own as it opens, or why it
    /// couldn't be read.
    Opened(WindowId, Read),
    /// The page has been drawn.
    Drawn(WindowId, From),
    /// The window's WebView is loading its page (a page served again, or reloaded).
    PageLoading(WindowId),
    /// The window's WebView was refused a navigation of its page: the editor has unsaved edits.
    NavigationRefused(WindowId),
    /// A message from the page's chrome (web/chrome/chrome.js): `open`, `mode`, `setting`,
    /// `about`, `about-copy`, `reading`, `reload`, `keep` or `close`; or from its editor (web/editor/editor.js,
    /// [`editor::Message`]).
    Chrome(WindowId, From, String),
    /// From live reload (rendered on its own thread), for the window watched under the key.
    Live(u64, Live),
    /// Close this window, as its own Close button asked.
    Close(WindowId),
    /// Files dropped on a window.
    Dropped(WindowId, Vec<PathBuf>),
    /// The recent files that are certainly gone, from the list with this token.
    Missing(u64, Vec<PathBuf>),
    /// The files whose reading places are kept that are certainly gone.
    PlacesGone(Vec<PathBuf>),
    /// A closing window's page said where the reader is (its script's result, as JSON), or
    /// didn't in time (empty): the window closes now.
    /// The `u64` is the close's number: a window whose document was replaced closes again later.
    PlaceTaken(WindowId, u64, String),
    /// What a window asked of the shell (Open in editor, Show in folder) didn't work.
    ActionFailed(WindowId, actions::Action),
    /// A window's request filter couldn't refuse a request: its document shows no pictures
    /// from the internet from now on.
    RemoteFailed(WindowId),
    /// A copy was built (or superseded), ready to write to the clipboard.
    CopyBuilt(Built<WindowId>),
    /// Try again to write the copy waiting for the clipboard (another app held it).
    CopyRetry(u64),
    /// Where Save as PDF's dialog opens, for job `u64` of the window.
    PdfFolder(WindowId, u64, Option<PathBuf>),
    /// WebView2 printed job `u64`'s PDF, or couldn't.
    PdfPrinted(u64, Result<(), String>),
    /// Job `u64`'s staging file replaced the chosen one, or couldn't.
    PdfCommitted(u64, Result<(), String>),
    /// Job `u64` has run out of time.
    PdfTimeout(u64),
    /// A staging file handed out for removal is gone, or given up on.
    PdfRemoved,
    /// Waiting `u64`, with the last window closed, for Save as PDF's work has run out of time.
    ShutdownOverdue(u64),
    /// The window's save of its editor's version `u64` is over.
    Written(WindowId, u64, Result<Stamp, WriteError>),
    /// A window's copy is written to the file, or couldn't be.
    CopyWritten(WindowId, PathBuf, Result<Stamp, recovery::NotCopied>),
    /// Their version of raced save `u64` is back in the window's file, or wasn't put back.
    RaceTaken(WindowId, u64, Result<recovery::Taken, recovery::NotTaken>),
    /// A window stopped Windows ending the session: its unsaved windows ask, in turn.
    SessionStopped,
    /// A window closed while they ask: the next one asks.
    SessionNext,
    /// Windows' session goes on after all.
    SessionGoesOn,
    /// A menu bar item was picked: its ID ([`menu::parse_id`]).
    Menu(String),
    /// The test harness's driver asks something of the shell ([`harness`]).
    #[cfg(feature = "harness")]
    Harness(harness::Call),
}

/// Which of a window's writes: its saves, Take theirs, or Save a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    Save,
    Take,
    Copy,
}

/// A write's moments, as a test acts in them: [`Hooks`]'s, and the moment it's over, before the
/// window hears of it. Legible's own writes have none ([`NoHooks`]).
trait Moments: Hooks + Send + Sync {
    fn over(&self) {}
}

impl Moments for NoHooks {}

/// What a test harness stands in for: the reader's answers to the window's dialogs, the
/// clipboard, the moments of its writes, and when the page's messages arrive. Legible itself
/// never has one: only [`harness::run`] makes one, in a test build (the `harness` feature, never
/// in a release build), and [`run`], which the exe runs, never takes one. With one, the shell
/// also leaves the user's own state alone: no app identity and no Jump List; and its windows
/// are quiet ([`quiet`]), as the harness turns that on.
trait StandIn {
    /// A question's answer: the close question's, or the one for unsaved edits kept for a file
    /// that has gone.
    fn answer(&self) -> dialogs::SaveAnswer;
    /// The file Save a copy's dialog chooses; None: cancelled.
    fn copy_to(&self) -> Option<PathBuf>;
    /// A message box, never shown: its answer to a question.
    fn message(&self, text: &str) -> bool;
    /// A copy, written to a clipboard of the harness's own, never the real one.
    fn clipboard(&self, html: &[u8], text: &str) -> clipboard::Written;
    /// The moments of a `write` starting now.
    fn moments(&self, write: Write) -> Arc<dyn Moments>;
    /// Whether the page's `message` is held back (the harness hands it on later).
    fn holds(&self, window: WindowId, from: From, message: &str) -> bool;
    /// Window `window`'s file, read as it opens: None if it's held back (the harness hands it on
    /// later, and the window stays opening till then), or `opened` back.
    fn holds_opened(&self, window: WindowId, read: Read) -> Option<Read>;
    /// Whether the shell runs on with no window open: the harness isn't done.
    fn stays(&self) -> bool;
}

/// What live reload found for a window.
#[derive(Debug)]
enum Live {
    /// The document as it now is, read under watch epoch `u64` ([`Watch::register`]).
    Changed(u64, OnDisk),
    /// The document has gone.
    Deleted,
    /// It changed but couldn't be read; it's tried again.
    ReadFailed { error: String },
}

/// A document as it is on disk: its text, for the editor, and its render, for View.
#[derive(Debug)]
struct OnDisk {
    disk: Disk,
    html: String,
}

/// A window's file as it opens, read and rendered off the UI thread ([`Shell::opened`]).
#[derive(Debug)]
struct Opened {
    on_disk: OnDisk,
    /// Where the render's pictures would be, for live reload to watch.
    pictures: Vec<PathBuf>,
    /// Whether the file may be edited where it is ([`editor::file_is_editable`]).
    editable_file: bool,
}

/// A file read for a window as it opens on it: the number of the read ([`Opening::read`]), and
/// the file's render or why it couldn't be read.
type Read = (u64, Result<Opened, String>);

/// The page a message came from, by its number ([`page_url`]); None: from a test harness's
/// driver, as the window's page is now.
type From = Option<u64>;

/// A window whose file is still being read and rendered: live reload's snapshot of the file,
/// taken before the read, and whether the file replaced the window's document (Open, Open
/// recent or a drop), so one that can't be read leaves the window open, with no document.
struct Opening {
    before: Option<Snapshot>,
    in_place: bool,
    /// The read's number, one of the shell's own: a read for an earlier opening, of the same
    /// file or another, is out of date.
    read: u64,
}

/// The document's folder as a window's document protocol serves it: shared with the protocol's
/// handler, as the window's document can be replaced.
type SharedRoot = Arc<Mutex<Option<DocRoot>>>;

/// The installed WebView2 runtime's version, or `None` if there isn't one.
pub fn webview2_version() -> Option<String> {
    wry::webview_version().ok().filter(|v| !v.is_empty())
}

/// Says WebView2 is missing, and opens Microsoft's page for it if the user asks.
pub fn report_missing_webview2() {
    let text = tr!("webview2-missing", url = WEBVIEW2_URL);
    if win32::message_box(None, &tr("webview2-missing-title"), &text, Message::Ask) {
        let _ = win32::shell_open(WEBVIEW2_URL);
    }
}

/// Runs the windows until the last one closes, then exits the process.
pub fn run(options: Options) -> ! {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    if let Some(primary) = options.primary {
        let proxy = event_loop.create_proxy();
        primary.serve(move |files| {
            let _ = proxy.send_event(UserEvent::Open(files));
        });
    }
    // Before any window, so its taskbar button and Jump List are Legible's.
    if let Err(error) = win32::jumplist::set_app_id(jumplist::APP_ID) {
        tracing::warn!(%error, "couldn't set Legible's app identity");
    }
    let shell = Shell::new(options.paths, options.lang, event_loop.create_proxy(), None);
    shell.hear_menus();
    event_loop.run(handler(shell, Some(options.files)))
}

/// The event loop's work, for [`run`] and the test harness: `files` open as it starts (None:
/// nothing opens until asked).
fn handler(
    mut shell: Shell,
    mut files: Option<Vec<PathBuf>>,
) -> impl FnMut(Event<'_, UserEvent>, &EventLoopWindowTarget<UserEvent>, &mut ControlFlow) + 'static
{
    move |event, target, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                if let Some(files) = files.take() {
                    // Unsaved edits kept as Windows last ended the session, offered back first.
                    shell.offer_kept(target);
                    shell.open_all(files, target);
                }
            }
            Event::UserEvent(UserEvent::Open(files)) => shell.launched(files, target),
            // What a page of a document the window has since replaced said, taken before it
            // was, is that document's: it does nothing now.
            Event::UserEvent(UserEvent::Link(id, from, _))
            | Event::UserEvent(UserEvent::Drawn(id, from))
            | Event::UserEvent(UserEvent::Chrome(id, from, _))
                if !shell.of_the_document(id, from) => {}
            Event::UserEvent(UserEvent::Link(id, _, href)) => shell.follow(id, &href, target),
            Event::UserEvent(UserEvent::Opened(id, read)) => {
                shell.opened(id, read);
                // A file that couldn't be read closes its window, which may have been the last.
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::Drawn(id, _)) => shell.drawn(id),
            Event::UserEvent(UserEvent::PageLoading(id)) => shell.page_loading(id),
            Event::UserEvent(UserEvent::NavigationRefused(id)) => shell.navigation_refused(id),
            Event::UserEvent(UserEvent::Chrome(id, _, message)) => {
                shell.chrome(id, &message);
                // The page's answer may close the window (a close's save, done).
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::Live(key, live)) => shell.live(key, live),
            Event::UserEvent(UserEvent::Dropped(id, paths)) => shell.dropped(id, paths, target),
            Event::UserEvent(UserEvent::Missing(token, missing)) => shell.missing(token, &missing),
            Event::UserEvent(UserEvent::PlacesGone(gone)) => shell.places_gone(&gone),
            Event::UserEvent(UserEvent::PlaceTaken(id, n, answer)) => {
                shell.place_taken(id, n, &answer);
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::ActionFailed(id, action)) => {
                shell.action_failed(id, action)
            }
            Event::UserEvent(UserEvent::RemoteFailed(id)) => shell.remote_failed(id),
            Event::UserEvent(UserEvent::CopyBuilt(built)) => shell.copy_built(built),
            Event::UserEvent(UserEvent::CopyRetry(seq)) => shell.copy_retry(seq),
            Event::UserEvent(UserEvent::PdfFolder(id, job, folder)) => {
                shell.pdf_folder(id, job, folder)
            }
            Event::UserEvent(UserEvent::PdfPrinted(job, result)) => {
                shell.pdf_printed(job, result);
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::PdfCommitted(job, result)) => {
                shell.pdf_committed(job, result);
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::PdfTimeout(job)) => {
                shell.pdf_timeout(job);
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::PdfRemoved) => {
                shell.pdf_jobs.removed();
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::ShutdownOverdue(wait)) => {
                shell.shutdown_overdue(wait);
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::Close(id)) => {
                shell.ask_close(id);
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::Written(id, version, written)) => {
                shell.written(id, version, written);
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::CopyWritten(id, path, written)) => {
                shell.copy_written(id, path, written, target)
            }
            Event::UserEvent(UserEvent::RaceTaken(id, n, taken)) => {
                shell.race_taken(id, n, taken);
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::SessionStopped) => shell.session_stopped(),
            Event::UserEvent(UserEvent::SessionNext) => {
                shell.ask_next_unsaved();
                shell.exit_when_settled(flow);
            }
            Event::UserEvent(UserEvent::SessionGoesOn) => shell.session_goes_on(),
            Event::UserEvent(UserEvent::Menu(item)) => {
                shell.menu(&item);
                shell.exit_when_settled(flow);
            }
            #[cfg(feature = "harness")]
            Event::UserEvent(UserEvent::Harness(call)) => {
                call.run(&mut shell, target, flow);
                shell.exit_when_settled(flow);
            }
            Event::WindowEvent {
                window_id, event, ..
            } => match event {
                WindowEvent::CloseRequested => {
                    shell.ask_close(window_id);
                    shell.exit_when_settled(flow);
                }
                WindowEvent::Focused(true) => shell.last_focused = Some(window_id),
                // Moved to a monitor with another scaling (or its scaling changed): the icons
                // are loaded again at its size, so Windows doesn't resample the wrong one.
                WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                    if let Some(doc) = shell.windows.get(&window_id) {
                        set_app_icons(&doc.window, scale_factor);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        // What each window answers Windows ending the session, as of now.
        shell.share_ending();
        // The menu bar follows the windows, and Exit goes on to its next.
        shell.menus_follow(flow);
    }
}

/// What a window's listener for the session's end knows. It runs whenever Windows asks, outside
/// the shell's own work, so the shell brings it up to date after every event
/// ([`Shell::share_ending`]). Its copy of the text is the newest the page has sent, kept by the
/// window's message handler as it comes ([`editor::unsaved_text`]).
#[derive(Default)]
struct Ending {
    /// The file's name, for the reason the window gives.
    name: String,
    document: Option<PathBuf>,
    /// Whether anything is unsaved ([`session::unsaved`]).
    unsaved: bool,
    /// The editor's newest text, and the hash of the file's bytes it's an edit of.
    text: Option<Rc<str>>,
    base: Option<u64>,
    /// The copy kept ([`unsaved::keep`], under this run's name for it), and the text it holds.
    session: u64,
    kept: Option<(PathBuf, Rc<str>)>,
    /// Whether the window is stopping the session's end.
    stopping: bool,
}

impl Ending {
    /// What the window answers Windows asking whether the session may end: a copy kept of
    /// anything unsaved, whatever comes next, then its reason for stopping the end.
    fn query(&mut self, hwnd: isize, critical: bool, folder: Option<&Path>) -> Answer {
        let answer = session::answer(self.unsaved, critical);
        if answer != Answer::Allow {
            self.keep(folder);
        }
        if answer == Answer::KeepAndStop && !self.stopping {
            let reason = tr!("session-unsaved", name = self.name.clone());
            match win32::session::stop(hwnd, &reason) {
                Ok(()) => self.stopping = true,
                Err(error) => tracing::warn!(%error, "couldn't give a reason for stopping the end"),
            }
        }
        answer
    }

    /// Keeps a copy of the newest text, unless the copy kept already holds it.
    fn keep(&mut self, folder: Option<&Path>) {
        let (Some(folder), Some(document), Some(text)) = (folder, &self.document, &self.text)
        else {
            return;
        };
        if self
            .kept
            .as_ref()
            .is_some_and(|(_, kept)| Rc::ptr_eq(kept, text))
        {
            return;
        }
        match unsaved::keep(folder, self.session, document, self.base, text) {
            Ok(copy) => self.kept = Some((copy, text.clone())),
            Err(error) => tracing::warn!(%error, "couldn't keep a copy of unsaved edits"),
        }
    }

    /// Nothing is unsaved after all (saved, or dropped), or the session goes on: the copy goes,
    /// and the window stops the end no more.
    fn forget(&mut self, hwnd: isize) {
        if let Some((copy, _)) = self.kept.take()
            && let Err(error) = unsaved::forget(&copy)
        {
            tracing::warn!(%error, copy = %copy.display(), "couldn't remove a copy of unsaved edits");
        }
        if std::mem::take(&mut self.stopping) {
            win32::session::go_on(hwnd);
        }
    }
}

struct DocWindow {
    // Dropped before the window it's in.
    webview: WebView,
    window: Window,
    /// Its menu bar (none if it couldn't be made): dropped after the window it's on.
    bar: Option<Bar>,
    /// The editor's mode and whether the outline is shown, as the page last said.
    mode: menu::Mode,
    outline: bool,
    file: Option<PathBuf>,
    /// Its file is still being read and rendered: no page is loaded yet, and it isn't watched.
    opening: Option<Opening>,
    /// Its key with live reload, if it has a file.
    watch_key: Option<u64>,
    /// The newest render sent to the page.
    generation: u64,
    /// Unsaved edits and the change held for them.
    reload: reload::State,
    /// The held change, for "take theirs".
    held: Option<OnDisk>,
    /// The moved-or-deleted bar, so a page served again shows it as the one before did.
    file_bar: FileBar,
    /// The document's folder as the window serves it, for a copy's pictures, and as its
    /// document protocol has it.
    root: Option<DocRoot>,
    shared_root: SharedRoot,
    /// The PDF Save as PDF saved last (Open and Show in folder use that).
    last_pdf: Option<PathBuf>,
    /// The page as it's served (its CSP with it), shared with the window's protocol handler.
    served: Arc<Mutex<Served>>,
    /// The number of the page the window loaded last ([`page_url`]), shared with its message
    /// handler: a page left behind, its document replaced or served again, says nothing more of
    /// where the reader is, or that it's drawn.
    page: Arc<AtomicU64>,
    /// The number of the document's first page, shared with the message handler: a page before
    /// it is of a document the window has since replaced, and says nothing more at all.
    first_page: Arc<AtomicU64>,
    /// The newest render of the document, for serving the page again.
    document: String,
    pictures: Pictures,
    /// Its title, without the unsaved dot.
    title: String,
    /// The file as last applied (shown, or given to the editor): what the editor opens on, the
    /// format saving keeps and the stamp it checks. A change held while there are unsaved
    /// edits isn't, until "take theirs". None: a window with no document.
    disk: Option<Disk>,
    /// Live reload's epoch for the window's file: a report read under an older one is out of
    /// date (the window has registered since, as after a save).
    watch_epoch: u64,
    /// A note for the page once it's drawn (the file opens for reading only, say).
    note: Option<String>,
    /// Whether the page's editor holds the text: from then on a change from disk goes to the
    /// editor itself.
    editor_open: bool,
    /// A reload waiting for the unsaved edits to be saved ([`editor::ReloadGate`]).
    reload_gate: editor::ReloadGate,
    /// Whether the editor has unsaved edits, as the WebView's navigation handler sees it.
    nav_guard: editor::NavigationGuard,
    /// A page served again whose navigation hasn't started yet: what to put back if it's
    /// refused (the editor became dirty meanwhile).
    pending_serve: Option<PendingServe>,
    /// Whether the file may be edited where it is ([`editor::file_is_editable`]).
    editable: bool,
    /// Its saving and closing ([`saving::Session`]).
    saves: saving::Session,
    /// The file as a save expects to find it: as the window last took it, or, after "Keep
    /// mine", as it was then (None: there's to be no file, as after one that went).
    expected: Option<Stamp>,
    /// What each request for the editor's contents was for, by its number.
    asked: HashMap<u64, save::Wanted>,
    next_ask: u64,
    /// The write under way.
    writing: Option<save::Writing>,
    /// The newest save another program raced, kept until the reader chooses, and how many
    /// there have been.
    race: Option<save::Race>,
    races: u64,
    /// A close asked for while Take theirs was written: it goes on once that's over.
    close_after_take: bool,
    /// Whether the window has said the file mixes line endings.
    told_mixed: bool,
    /// What its listener for the session's end knows, and when it opened (the oldest asks
    /// first).
    ending: Rc<RefCell<Ending>>,
    opened: u64,
    /// The copies of unsaved edits from earlier runs, offered on the bar in turn until the
    /// reader chooses ([`kept`]), and those whose Restore was chosen, removed once their text is
    /// in.
    offered: VecDeque<kept::Offered>,
    restoring: VecDeque<PathBuf>,
    /// Where the reader is, as the page last said, kept for the file as the window closes; and
    /// where they were last time, for the page once it's first drawn ([`places`]).
    place: Option<Place>,
    go_to: Option<Place>,
    /// The file to show in place of the document once nothing is unsaved (Open, Open recent or
    /// a drop): the close question's path decides, and the close it comes to replaces the
    /// document instead of closing the window ([`Shell::replace`]).
    replacing: Option<PathBuf>,
    /// The mode the reader had before the document was replaced, for the new one's page once
    /// it's drawn, if the new file can be edited.
    mode_after: Option<menu::Mode>,
}

/// What a window's WebView shares with the window: the page it serves, the document's folder,
/// its navigation guard, and what its listener for the session's end knows.
struct WindowParts {
    served: Arc<Mutex<Served>>,
    page: Arc<AtomicU64>,
    first_page: Arc<AtomicU64>,
    root: SharedRoot,
    nav_guard: editor::NavigationGuard,
    ending: Rc<RefCell<Ending>>,
}

/// A page served again, waiting for its navigation: the page and generation it replaced, and
/// the pictures' origins its CSP lets in.
struct PendingServe {
    before: Served,
    page: u64,
    generation: u64,
    origins: Vec<String>,
}

/// The page a window's protocol serves at `/`, and its CSP header.
#[derive(Default)]
struct Served {
    html: Vec<u8>,
    csp: String,
}

/// A copy ready for the clipboard, waiting because another app held it.
struct CopyWaiting {
    seq: u64,
    window: WindowId,
    page: Ticket,
    html: Vec<u8>,
    text: String,
    tries: u32,
}

/// How long a closing window waits for its page to say where the reader is.
const PLACE_BOUND: std::time::Duration = std::time::Duration::from_secs(1);
/// What a closing window asks its page, in one script: no more edits, then whether anything is
/// unsaved, and where the reader is (`LegibleChrome.place()`, empty when there's none to give).
const CLOSING: &str = "(() => { window.LegibleEditor?.freeze?.(); \
    const unsaved = !!window.LegibleEditor?.isDirty?.(); \
    return `${unsaved ? 'unsaved' : 'clean'}\\n${window.LegibleChrome?.place?.() ?? ''}`; })()";
/// How often, and how long apart, a copy tries for a clipboard another app holds.
const COPY_TRIES: u32 = 20;
const COPY_RETRY_AFTER: std::time::Duration = std::time::Duration::from_millis(50);
/// How long a PDF may take to print.
const PDF_BOUND: std::time::Duration = std::time::Duration::from_secs(120);
/// How long a staging file is tried for removal while WebView2 still holds it.
const PDF_CLEANUP_BOUND: std::time::Duration = std::time::Duration::from_secs(30);
/// How long Legible, its last window closed, waits for Save as PDF's work (a commit to a slow
/// share, a staging file's removal) before it exits anyway.
const SHUTDOWN_BOUND: std::time::Duration = std::time::Duration::from_secs(60);

/// What Legible does about exiting, once something that may settle it has happened.
#[derive(Debug, PartialEq, Eq)]
enum Shutdown {
    /// A window is open: Legible runs on.
    Run,
    /// The last window is closed, but work Legible owns isn't settled: it waits (for
    /// [`SHUTDOWN_BOUND`] at most).
    Wait,
    Exit,
}

/// With no window open, Legible exits once its own work is settled, or the wait is overdue.
fn shutdown(windows_open: bool, busy: bool, overdue: bool) -> Shutdown {
    match (windows_open, busy, overdue) {
        (true, _, _) => Shutdown::Run,
        (false, true, false) => Shutdown::Wait,
        (false, _, _) => Shutdown::Exit,
    }
}

impl DocWindow {
    /// The file's name, as a message names it.
    fn title_name(&self) -> String {
        self.file
            .as_deref()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    /// Whether anything is unsaved: edits, by either account, or a save.
    fn unsaved(&self) -> bool {
        session::unsaved(
            self.reload.is_dirty() || self.nav_guard.dirty(),
            &self.saves,
        )
    }

    /// The title as the window's state has it: with the unsaved dot while there are unsaved
    /// edits. Called after every change to them, from whichever side it came.
    fn show_title(&self) {
        self.window
            .set_title(&editor::window_title(&self.title, self.reload.is_dirty()));
    }
}

struct Shell {
    windows: HashMap<WindowId, DocWindow>,
    /// Each open file's window, by its canonical path.
    by_file: HashMap<PathBuf, WindowId>,
    last_focused: Option<WindowId>,
    context: WebContext,
    placement_file: Option<PathBuf>,
    remembered: Option<Placement>,
    lang: String,
    proxy: EventLoopProxy<UserEvent>,
    /// Live reload: one service for every window.
    watch: Watch<OnDisk>,
    watch_keys: HashMap<u64, WindowId>,
    next_watch_key: u64,
    settings: Settings,
    /// The recent files, where they're saved (None: Legible's folder can't be written), and
    /// the thread that finds the missing ones.
    recent: Recent,
    recent_file: Option<PathBuf>,
    /// Where the reader was in each file, and where that's saved (None: Legible's folder can't
    /// be written).
    places: Places,
    places_file: Option<PathBuf>,
    /// The windows closing once their page has said where the reader is, and whether each
    /// close drops unsaved edits as the reader chose (Don't save).
    /// Each is numbered, from `next_close`, so an answer or a bound is only its own close's.
    closing: HashMap<WindowId, (u64, bool)>,
    next_close: u64,
    /// The last file read's number ([`Opening::read`]).
    next_read: u64,
    checker: Checker,
    /// Open in editor and Show in folder, on a thread of their own.
    actions: Actions<WindowId>,
    /// This exe, and whether it's a portable copy: the file associations name it, and a
    /// portable copy registers them only when asked.
    exe: PathBuf,
    portable: bool,
    /// The default-app bar's Close: not offered again in any window until Legible starts again.
    default_offer_closed: bool,
    /// Copies: built on a thread of their own, in one order for the process, and the one
    /// waiting for the clipboard.
    copier: Copier<WindowId>,
    copy_order: Order,
    copy_waiting: Option<CopyWaiting>,
    /// Save as PDF's jobs, for the process (they outlive their windows, to finish or clean up).
    pdf_jobs: export::Jobs<WindowId>,
    /// The files whose pictures from the internet always show, and where they're saved.
    always: AlwaysList,
    always_file: Option<PathBuf>,
    /// Whether the Jump List has failed, so it's logged once.
    jumplist_failed: bool,
    /// Where the settings are saved: None if Legible's folder isn't known.
    settings_file: Option<PathBuf>,
    /// Whether that folder can be written to.
    settings_writable: bool,
    /// Whether the first window's start-up times have been logged: shown, then drawn.
    shown_timed: bool,
    drawn_timed: bool,
    /// The wait for Save as PDF's work with the last window closed (None: not waiting), its
    /// number (so a wait a new window ended can't time out a later one), and whether it's
    /// overdue.
    shutdown_wait: Option<u64>,
    shutdown_waits: u64,
    shutdown_overdue: bool,
    /// The last menu bar's number, and File's Exit, while it closes the windows.
    next_bar: u64,
    exit: Option<Exit<WindowId>>,
    /// A test harness's stand-in ([`StandIn`]); None in Legible itself.
    stand_in: Option<Rc<dyn StandIn>>,
    /// Whether the windows are quiet, as a test's are ([`quiet`]): never Legible's own.
    quiet: bool,
    /// Where copies of unsaved edits are kept as Windows ends the session (None: Legible's
    /// folder can't be written), the close questions asked as it does, and the windows opened.
    unsaved_folder: Option<PathBuf>,
    round: Round,
    opened: u64,
    /// Every window's [`Ending`], by its handle, for whichever window Windows asks: once one
    /// stops the end, Windows may ask no other.
    endings: Endings,
    /// This run's own name for its copies ([`unsaved::copy_for`]), so a copy from an earlier
    /// run, still waiting for the reader's choice, is never written over.
    session: u64,
}

/// Every window's [`Ending`], by its window's handle.
type Endings = Rc<RefCell<Vec<(isize, Rc<RefCell<Ending>>)>>>;

impl Shell {
    fn new(
        paths: Option<AppPaths>,
        lang: String,
        proxy: EventLoopProxy<UserEvent>,
        stand_in: Option<Rc<dyn StandIn>>,
    ) -> Self {
        let writable = paths.as_ref().filter(|p| p.is_writable());
        let data = match writable {
            Some(paths) => paths.file(WEBVIEW2_DIR),
            None => {
                // WebView2 can't run without a folder it can write to.
                let temp = std::env::temp_dir().join(format!("Legible-{WEBVIEW2_DIR}"));
                tracing::warn!(folder = %temp.display(), "Legible's folder can't be written to: WebView2 keeps its data in the temp folder");
                temp
            }
        };
        let placement_file = writable.map(|p| p.file(placement::FILE));
        let settings_file = paths.as_ref().map(AppPaths::settings_file);
        let settings = settings_file
            .as_deref()
            .map(Settings::load)
            .unwrap_or_default();
        let remembered = placement_file.as_deref().and_then(Placement::load);
        let recent_file = writable.map(|p| p.file(recent::FILE));
        let recent = recent_file.as_deref().map(Recent::load).unwrap_or_default();
        let found = proxy.clone();
        let checker = Checker::new(recent::exists, move |token, missing| {
            let _ = found.send_event(UserEvent::Missing(token, missing));
        });
        checker.check(&recent);
        let places_file = writable.map(|p| p.file(places::FILE));
        let places = places_file.as_deref().map(Places::load).unwrap_or_default();
        // The files that have gone leave the list, found off the UI thread (a share may be slow).
        let kept = places.paths();
        let gone = proxy.clone();
        thread::spawn(move || {
            let missing: Vec<PathBuf> = kept
                .into_iter()
                .filter(|p| recent::exists(p) == recent::Existence::Missing)
                .collect();
            if !missing.is_empty() {
                let _ = gone.send_event(UserEvent::PlacesGone(missing));
            }
        });
        let failed = proxy.clone();
        let exe = std::env::current_exe().unwrap_or_default();
        let portable = paths.as_ref().is_some_and(AppPaths::is_portable);
        let actions = Actions::new(
            win32::actions::ShellPlatform::new,
            exe.clone(),
            move |id, action| {
                let _ = failed.send_event(UserEvent::ActionFailed(id, action));
            },
        );
        let built = proxy.clone();
        let copier = Copier::new(move |b| {
            let _ = built.send_event(UserEvent::CopyBuilt(b));
        });
        let always_file = writable.map(|p| p.file(consent::ALWAYS_FILE));
        let always = always_file
            .as_deref()
            .map(AlwaysList::load)
            .unwrap_or_default();
        let reports = proxy.clone();
        // Both run on live reload's thread: the document is read and rendered there, off the UI
        // thread, and the pictures of the new render are watched from then on.
        let watch = Watch::new(
            |watched: &Watched, bytes: Vec<u8>, stamp: Stamp| {
                let folder = watched.document.parent().unwrap_or(&watched.document);
                let disk = Disk::of(bytes, stamp);
                let html = render(disk.content.text(), folder).html;
                let pictures = pictures(watched.root.as_ref(), &html);
                (OnDisk { disk, html }, pictures)
            },
            move |report| {
                let (key, live) = match report {
                    Report::Changed {
                        key,
                        epoch,
                        prepared,
                    } => (key, Live::Changed(epoch, prepared)),
                    Report::Deleted { key } => (key, Live::Deleted),
                    Report::ReadFailed { key, error } => (key, Live::ReadFailed { error }),
                };
                let _ = reports.send_event(UserEvent::Live(key, live));
            },
        );
        Self {
            windows: HashMap::new(),
            by_file: HashMap::new(),
            last_focused: None,
            context: WebContext::new(Some(data)),
            placement_file,
            remembered,
            lang,
            proxy,
            watch,
            watch_keys: HashMap::new(),
            next_watch_key: 0,
            settings,
            recent,
            recent_file,
            places,
            places_file,
            closing: HashMap::new(),
            next_close: 0,
            next_read: 0,
            checker,
            actions,
            copier,
            copy_order: Order::default(),
            copy_waiting: None,
            pdf_jobs: export::Jobs::default(),
            always,
            always_file,
            jumplist_failed: false,
            settings_file,
            settings_writable: writable.is_some(),
            exe,
            portable,
            default_offer_closed: false,
            shown_timed: false,
            drawn_timed: false,
            shutdown_wait: None,
            shutdown_waits: 0,
            shutdown_overdue: false,
            next_bar: 0,
            exit: None,
            stand_in,
            quiet: quiet::on(),
            unsaved_folder: writable.map(|p| p.file(unsaved::FOLDER)),
            round: Round::default(),
            opened: 0,
            endings: Endings::default(),
            session: unsaved::session(),
        }
    }

    /// Both ways a window closes come here after: with the last one closed, Legible exits once
    /// the work it owns is settled, waiting off the UI thread for [`SHUTDOWN_BOUND`] at most.
    fn exit_when_settled(&mut self, flow: &mut ControlFlow) {
        if !self.windows.is_empty() || self.stand_in.as_ref().is_some_and(|s| s.stays()) {
            return;
        }
        let busy = self.pdf_jobs.busy();
        match shutdown(false, busy, self.shutdown_overdue) {
            Shutdown::Run => {}
            Shutdown::Exit => {
                if busy {
                    tracing::warn!("exiting with Save as PDF's work unsettled: it took too long");
                }
                *flow = ControlFlow::Exit;
            }
            Shutdown::Wait if self.shutdown_wait.is_none() => {
                tracing::info!("waiting for Save as PDF's work to settle before exiting");
                self.shutdown_waits += 1;
                let wait = self.shutdown_waits;
                self.shutdown_wait = Some(wait);
                let proxy = self.proxy.clone();
                thread::spawn(move || {
                    thread::sleep(SHUTDOWN_BOUND);
                    let _ = proxy.send_event(UserEvent::ShutdownOverdue(wait));
                });
            }
            Shutdown::Wait => {}
        }
    }

    /// The wait `wait` has run out: if it's still the one under way, Legible stops waiting.
    fn shutdown_overdue(&mut self, wait: u64) {
        if self.shutdown_wait == Some(wait) {
            self.shutdown_overdue = true;
        }
    }

    /// A later launch's files (the single-instance hand-off: a double-click, the Jump List, the
    /// shortcut): each opens in a window of its own, or brings its window forward if it's open.
    /// With none (the shortcut), an empty window opens, as a launch opens a new window.
    fn launched(&mut self, files: Vec<PathBuf>, target: &EventLoopWindowTarget<UserEvent>) {
        if files.is_empty() {
            self.create(None, target);
        }
        self.open_all(files, target);
    }

    /// Opens each file in its own window, or brings its window forward if it's open. With
    /// none, brings the last window forward, or opens an empty one.
    fn open_all(&mut self, files: Vec<PathBuf>, target: &EventLoopWindowTarget<UserEvent>) {
        if files.is_empty() {
            match self
                .last_focused
                .or_else(|| self.windows.keys().next().copied())
            {
                Some(id) => self.focus(id),
                None => self.create(None, target),
            }
        }
        for file in files {
            self.open(&file, target);
        }
        // Nothing could be opened, and nothing was open: there's nothing to keep running for.
        if self.windows.is_empty() {
            std::process::exit(1);
        }
    }

    /// Opens `file` in a window of its own, or brings its window forward; says whether it's
    /// open now. Every launch comes here (a double-click, the Jump List, a later launch's
    /// hand-off), and so do a link and a copy just saved; each that works adds to recent files.
    fn open(&mut self, file: &Path, target: &EventLoopWindowTarget<UserEvent>) -> bool {
        let Some(canonical) = self.found(file) else {
            return false;
        };
        match self.by_file.get(&canonical) {
            Some(&id) => self.focus(id),
            None => self.create(Some(canonical.clone()), target),
        }
        let Some(&id) = self.by_file.get(&canonical) else {
            return false;
        };
        self.add_recent(id, canonical);
        true
    }

    /// Opens `file` in window `id`, in place of its document: Open, Open recent and a drop come
    /// here. A file open in another window brings that one forward instead, and this one stays
    /// as it is. Anything unsaved is asked about first, as a close asks, through the window's
    /// own closing ([`saving::Session`]): the document is replaced only once nothing is
    /// unsaved, and Cancel, or a save that fails, keeps it as it was.
    fn open_here(&mut self, id: WindowId, file: &Path) {
        let Some(canonical) = self.found(file) else {
            return;
        };
        if let Some(&open) = self.by_file.get(&canonical) {
            if open != id {
                self.focus(open);
            }
            return self.add_recent(open, canonical);
        }
        let busy = self.closing.contains_key(&id);
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        // Closing, or already replacing its document: it finishes that first.
        if busy
            || doc.replacing.is_some()
            || doc.saves.closing()
            || doc.close_after_take
            || doc.race.as_ref().is_some_and(|r| r.taking)
        {
            return tracing::info!("an open asked of a window that's closing or replacing");
        }
        doc.replacing = Some(canonical);
        let dirty = doc.reload.is_dirty() || doc.nav_guard.dirty();
        let close = doc.saves.close(dirty);
        self.carry_out(id, close);
    }

    /// `file`, canonical, if it's a file that's there; if not, the reader is told why.
    fn found(&self, file: &Path) -> Option<PathBuf> {
        match fs::canonicalize(file) {
            Ok(canonical) if canonical.is_file() => Some(canonical),
            Ok(_) => {
                self.report_unopened(
                    file,
                    &tr!("error-file-not-found", file = file.display().to_string()),
                );
                None
            }
            Err(error) => {
                tracing::warn!(%error, file = %file.display(), "couldn't find a document");
                self.report_unopened(file, &reason::of(&error));
                None
            }
        }
    }

    /// `file`, open in window `id`, goes to the top of recent files: a window still reading
    /// its file adds it once it has ([`Shell::opened`]).
    fn add_recent(&mut self, id: WindowId, file: PathBuf) {
        if self.windows.get(&id).is_some_and(|d| d.opening.is_none()) {
            self.recent.add(file);
            self.recent_changed();
        }
    }

    /// Files dropped on window `id`: the first Markdown one replaces its document, as Open does,
    /// and the others open in windows of their own; with none, the window says what it opens.
    fn dropped(
        &mut self,
        id: WindowId,
        paths: Vec<PathBuf>,
        target: &EventLoopWindowTarget<UserEvent>,
    ) {
        let mut markdown = links::dropped_markdown(paths).into_iter();
        let Some(first) = markdown.next() else {
            let text = json_string(&tr("chrome-drop-not-markdown"));
            self.run_script(id, &format!("window.LegibleChrome?.notice({text})"));
            return;
        };
        self.open_here(id, &first);
        for file in markdown {
            self.open(&file, target);
        }
    }

    /// An action window `id` asked for didn't work: its window says so.
    fn action_failed(&self, id: WindowId, action: actions::Action) {
        let key = match action {
            actions::Action::OpenInEditor => "chrome-editor-failed",
            actions::Action::ShowInFolder => "chrome-folder-failed",
        };
        let text = json_string(&tr(key));
        self.run_script(id, &format!("window.LegibleChrome?.notice({text})"));
    }

    /// The recent files changed: shown in the Jump List (first, as the files the user took off
    /// it leave the list), saved, and shown in every window's menu.
    fn recent_changed(&mut self) {
        self.publish_jump_list();
        if let Some(file) = &self.recent_file
            && let Err(error) = self.recent.save(file)
        {
            tracing::warn!(%error, "couldn't save the recent files");
        }
        let ids: Vec<WindowId> = self.windows.keys().copied().collect();
        for id in ids {
            self.send_recent(id);
        }
        self.bars_recent();
        self.checker.check(&self.recent);
    }

    /// The files whose reading places are kept that are certainly gone: they leave the list.
    fn places_gone(&mut self, gone: &[PathBuf]) {
        if self.places.remove(gone) {
            self.save_places();
        }
    }

    fn save_places(&self) {
        if let Some(file) = &self.places_file
            && let Err(error) = self.places.save(file)
        {
            tracing::warn!(%error, "couldn't save where the reader was in each file");
        }
    }

    /// The recent files that are certainly gone, found for the list with `token`.
    fn missing(&mut self, token: u64, missing: &[PathBuf]) {
        if token == self.recent.token() && self.recent.remove(missing) {
            self.recent_changed();
        }
    }

    /// Legible's Jump List, from the recent files. Files the user took off it leave the list
    /// too. A failure is logged (once) and changes nothing else. A test harness's shell has none:
    /// the Jump List is the user's.
    fn publish_jump_list(&mut self) {
        if self.stand_in.is_some() {
            return;
        }
        let published = std::env::current_exe().and_then(|exe| {
            let mut list = win32::jumplist::ShellDestinations::new(jumplist::APP_ID)?;
            jumplist::publish(&mut list, &exe, &tr("chrome-recent"), self.recent.paths())
        });
        match published {
            Ok(gone) => {
                self.jumplist_failed = false;
                self.recent.remove(&gone);
            }
            Err(error) if !self.jumplist_failed => {
                tracing::warn!(%error, "couldn't update the Jump List");
                self.jumplist_failed = true;
            }
            Err(_) => {}
        }
    }

    /// Window `id`'s menu gets the recent files, named by the list's token.
    fn send_recent(&self, id: WindowId) {
        let items: Vec<String> = self
            .recent
            .paths()
            .iter()
            .map(|path| {
                let shown = jumplist::display_path(path);
                let name = shown
                    .file_name()
                    .unwrap_or(shown.as_os_str())
                    .to_string_lossy();
                format!(
                    "{{\"name\":{},\"path\":{}}}",
                    json_string(&name),
                    json_string(&shown.to_string_lossy())
                )
            })
            .collect();
        let script = format!(
            "window.LegibleChrome?.setRecent({{\"token\":{},\"items\":[{}]}})",
            self.recent.token(),
            items.join(",")
        );
        self.run_script(id, &script);
    }

    fn report_unopened(&self, file: &Path, reason: &str) {
        tracing::warn!(file = %file.display(), reason, "couldn't open a document");
        let text = tr!(
            "error-open",
            file = file.display().to_string(),
            reason = reason.to_owned()
        );
        let owner = self.last_focused.and_then(|id| self.windows.get(&id));
        self.message_box(
            owner.map(|w| w.window.hwnd()),
            &tr("error-open-title"),
            &text,
            Message::Error,
        );
    }

    /// A message box (or a test harness's stand-in for one): whether a question was answered Yes.
    fn message_box(&self, owner: Option<isize>, title: &str, text: &str, kind: Message) -> bool {
        match &self.stand_in {
            Some(stand_in) => stand_in.message(text),
            None => win32::message_box(owner, title, text, kind),
        }
    }

    /// Brings window `id` forward; a quiet one is left where it is, so it never takes the focus
    /// from the user's work.
    fn focus(&self, id: WindowId) {
        if self.quiet {
            return;
        }
        if let Some(doc) = self.windows.get(&id) {
            doc.window.set_minimized(false);
            doc.window.set_focus();
        }
    }

    /// A new window for `file` (canonical), or an empty one. The window is made, and WebView2
    /// started in it, while the file is read and rendered on a thread of its own: its page is
    /// served once that's done ([`Shell::opened`]), and a file that can't be read closes it.
    fn create(&mut self, file: Option<PathBuf>, target: &EventLoopWindowTarget<UserEvent>) {
        // Taken before the file is read, so a change in between is live reload's first report.
        let before = file
            .as_deref()
            .and_then(|f| self.watch.document_snapshot(f));
        let title = title_of(file.as_deref());
        let root = file.as_deref().and_then(|f| DocRoot::for_document(f).ok());

        let scale = target.primary_monitor().map_or(1.0, |m| m.scale_factor());
        let (monitors, primary) = monitors(target);
        // The last window's size and place, unless Settings says to let Windows place each.
        let remembered = self.remembered.filter(|_| self.settings.reopen_windows);
        let placed = Placement::for_new_window(remembered, &monitors, primary);
        let bar = self.new_bar();
        // Its title bar in Legible's theme from the first frame, as its menus are.
        let mut builder = WindowBuilder::new().with_theme(menus::window_theme(self.settings.theme));
        if let Some((_, placeholder)) = &bar {
            builder = builder.with_menu(placeholder.0 as isize);
        }
        // A quiet window opens off every monitor, unless a test placed the last one.
        let position = placed
            .position
            .or_else(|| self.quiet.then(|| quiet::off_screen(target)).flatten());
        if let Some((x, y)) = position {
            builder = builder.with_position(PhysicalPosition::new(x, y));
        }
        let window = match builder
            .with_title(&title)
            .with_inner_size(LogicalSize::new(placed.size.width, placed.size.height))
            .with_min_inner_size(LogicalSize::new(
                placement::MIN.width,
                placement::MIN.height,
            ))
            // Maximising shows it activated: a quiet window is never maximised.
            .with_maximized(placed.maximized && !self.quiet)
            .with_window_icon(app_icon(16, scale))
            .with_taskbar_icon(app_icon(32, scale))
            .with_focused(!self.quiet)
            .with_focusable(!self.quiet)
            .with_skip_taskbar(self.quiet)
            .build(target)
        {
            Ok(window) => window,
            Err(error) => {
                tracing::error!(%error, "couldn't make a window");
                return;
            }
        };
        let bar = bar.and_then(|(bar, placeholder)| self.attach_bar(bar, placeholder, &window));
        // Its place and size were worked out from its inner size: with its frame, title and menu
        // bar, it's made smaller by what would be too big for its monitor's work area, and moved
        // fully onto it.
        if placed.position.is_some()
            && !placed.maximized
            && let Ok(at) = window.outer_position()
            && let outer = window.outer_size()
            && let Some(fitted) = placement::onto_monitor(
                placement::Rect {
                    x: at.x,
                    y: at.y,
                    width: i32::try_from(outer.width).unwrap_or(i32::MAX),
                    height: i32::try_from(outer.height).unwrap_or(i32::MAX),
                },
                &monitors,
            )
        {
            let over_width = outer.width.saturating_sub(fitted.width.max(0) as u32);
            let over_height = outer.height.saturating_sub(fitted.height.max(0) as u32);
            if over_width > 0 || over_height > 0 {
                let inner = window.inner_size();
                window.set_inner_size(PhysicalSize::new(
                    inner.width.saturating_sub(over_width),
                    inner.height.saturating_sub(over_height),
                ));
            }
            window.set_outer_position(PhysicalPosition::new(fitted.x, fitted.y));
        }
        // The icons above are for the primary monitor; Windows may have placed the window on
        // another, scaled otherwise.
        let placed_scale = window.scale_factor();
        if (placed_scale - scale).abs() > f64::EPSILON {
            set_app_icons(&window, placed_scale);
        }
        if !self.shown_timed {
            self.shown_timed = true;
            tracing::info!(ms = win32::process_age_ms(), "the first window is shown");
        }

        let id = window.id();
        let read = file
            .as_ref()
            .map(|file| self.read(id, file.clone(), root.clone()));
        let served = Arc::new(Mutex::new(Served::default()));
        let page = Arc::new(AtomicU64::new(0));
        let first_page = Arc::new(AtomicU64::new(0));
        let shared_root = Arc::new(Mutex::new(root.clone()));
        let nav_guard = editor::NavigationGuard::default();
        let go_to = self.place_in(file.as_deref());
        let ending = Rc::new(RefCell::new(self.ending_for(file.as_deref())));
        let parts = WindowParts {
            served: served.clone(),
            page: page.clone(),
            first_page: first_page.clone(),
            root: shared_root.clone(),
            nav_guard: nav_guard.clone(),
            ending: ending.clone(),
        };
        let webview = match self.webview(id, parts, &window) {
            Ok(webview) => webview,
            Err(error) => {
                tracing::error!(%error, "couldn't start WebView2 in a window");
                self.message_box(
                    Some(window.hwnd()),
                    &tr("error-open-title"),
                    &tr!(
                        "error-open",
                        file = title.clone(),
                        reason = error.to_string()
                    ),
                    Message::Error,
                );
                return;
            }
        };
        // The request filter goes on before the page loads; without it, the page's CSP stays
        // as strict as ever and its pictures from the internet stay placeholders.
        let allowed = Rc::new(RefCell::new(HashSet::new()));
        let failed = self.proxy.clone();
        let unavailable = match win32::request_filter::install(
            &webview.webview(),
            allowed.clone(),
            Box::new(move || {
                let _ = failed.send_event(UserEvent::RemoteFailed(id));
            }),
        ) {
            Ok(()) => None,
            Err(error) => {
                tracing::warn!(%error, "no request filter: pictures from the internet won't show");
                Some(Unavailable::Runtime)
            }
        };
        if let Some(file) = &file {
            self.by_file.insert(file.clone(), id);
        }
        // A window opened while Legible waited to exit: it runs on, and waits afresh later.
        self.shutdown_wait = None;
        self.shutdown_overdue = false;
        self.endings
            .borrow_mut()
            .push((window.hwnd(), ending.clone()));
        self.listen_for_the_end(window.hwnd());
        self.opened += 1;
        self.windows.insert(
            id,
            DocWindow {
                webview,
                window,
                bar,
                mode: menu::Mode::View,
                outline: false,
                // Known once the file is read.
                editable: false,
                opening: read.map(|read| Opening {
                    before,
                    in_place: false,
                    read,
                }),
                file,
                watch_key: None,
                generation: 0,
                reload: reload::State::default(),
                held: None,
                file_bar: FileBar::default(),
                root: root.clone(),
                shared_root,
                last_pdf: None,
                served,
                page,
                first_page,
                document: String::new(),
                pictures: Pictures::new(allowed, unavailable, ""),
                title,
                disk: None,
                watch_epoch: 0,
                note: None,
                editor_open: false,
                reload_gate: editor::ReloadGate::default(),
                nav_guard,
                pending_serve: None,
                saves: saving::Session::default(),
                expected: None,
                asked: HashMap::new(),
                next_ask: 0,
                writing: None,
                race: None,
                races: 0,
                close_after_take: false,
                told_mixed: false,
                ending,
                opened: self.opened,
                place: None,
                go_to,
                offered: VecDeque::new(),
                restoring: VecDeque::new(),
                replacing: None,
                mode_after: None,
            },
        );
        self.last_focused = Some(id);
        // The empty window's page now; a document's once it's rendered.
        if self.windows.get(&id).is_some_and(|d| d.file.is_none()) {
            self.serve_first(id);
        }
    }

    /// Window `id`'s file is read and rendered, or couldn't be read. Its page is served, and live
    /// reload watches the file from the snapshot taken before the read; or, if it couldn't be
    /// read, the window says why and closes, as a file that couldn't be read before a window was
    /// made did, or, its document replaced, stays open with none.
    fn opened(&mut self, id: WindowId, read: Read) {
        let (read, opened) = match &self.stand_in {
            Some(stand_in) => match stand_in.holds_opened(id, read) {
                Some(read) => read,
                None => return,
            },
            None => read,
        };
        // Closed meanwhile: there's nothing to show it in.
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        // An earlier opening's read: the document was replaced meanwhile (by the same file
        // again, perhaps, read once more since).
        if doc.opening.as_ref().is_none_or(|o| o.read != read) {
            return;
        }
        let (Some(opening), Some(file)) = (doc.opening.take(), doc.file.clone()) else {
            return;
        };
        let Opened {
            on_disk: OnDisk { disk, html },
            pictures,
            editable_file,
        } = match opened {
            Ok(opened) => opened,
            Err(reason) if opening.in_place => {
                self.emptied(id);
                return self.report_unopened(&file, &reason);
            }
            Err(reason) => {
                self.close_now(id);
                return self.report_unopened(&file, &reason);
            }
        };
        // Only text Legible can save as it was, where it can be saved.
        doc.editable = editable_file && disk.content.format().is_some();
        if matches!(disk.content, Content::Unsupported { .. }) {
            doc.note = Some(tr("chrome-encoding-unsupported"));
        }
        doc.expected = Some(disk.stamp);
        doc.disk = Some(disk);
        doc.pictures.render(&html);
        doc.document = html;
        let key = self.next_watch_key;
        self.next_watch_key += 1;
        doc.watch_epoch = self.watch.register(
            key,
            Watched {
                document: file.clone(),
                root: doc.root.clone(),
                pictures,
            },
            opening.before,
        );
        doc.watch_key = Some(key);
        self.watch_keys.insert(key, id);
        self.recent.add(file);
        self.recent_changed();
        self.serve_first(id);
    }

    /// Serves window `id`'s first page: its document's, or the empty window's.
    fn serve_first(&mut self, id: WindowId) {
        let origins = match self.grant_pictures(id) {
            Some(Next::ServeAgain(origins)) => origins,
            Some(Next::Tell) => Vec::new(),
            None => return,
        };
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        doc.pictures.served(origins.clone());
        let html = if doc.file.is_some() {
            page::document_page(
                &self.lang,
                &doc.title,
                &page::with_generation(&doc.document, 0),
                &self.settings,
                0,
                &origins,
                doc.editable,
            )
        } else {
            page::empty_page(
                &self.lang,
                &doc.title,
                &tr("window-no-document"),
                &self.settings,
            )
        };
        serve(&doc.served, html, &origins);
        let n = doc.page.fetch_add(1, Ordering::SeqCst) + 1;
        if let Err(error) = doc.webview.load_url(&page_url(n)) {
            tracing::error!(%error, "couldn't load a window's page");
        }
    }

    /// The window's WebView2, with nothing loaded yet: its page is loaded once the request
    /// filter is on.
    fn webview(
        &mut self,
        id: WindowId,
        parts: WindowParts,
        window: &Window,
    ) -> wry::Result<WebView> {
        let WindowParts {
            served,
            page,
            first_page,
            root,
            nav_guard,
            ending,
        } = parts;
        let events = self.proxy.clone();
        let drops = self.proxy.clone();
        let navigations = self.proxy.clone();
        let noted = nav_guard.clone();
        let stand_in = self.stand_in.clone();
        let focused = !self.quiet;
        let debug = cfg!(debug_assertions);
        WebViewBuilder::new_with_web_context(&mut self.context)
            .with_https_scheme(true)
            .with_custom_protocol(page::APP_PROTOCOL.into(), move |_, request| {
                let served = served
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                serve_app(&request, &served)
            })
            .with_custom_protocol(page::DOC_PROTOCOL.into(), move |_, request| {
                let root = root
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                serve_document(&request, root.as_ref())
            })
            .with_initialization_script(page::init_script())
            .with_ipc_handler(move |request: Request<String>| {
                // Only Legible's own page talks to the window: never a frame of it.
                if !is_the_page(&request.uri().to_string()) {
                    return;
                }
                let body = request.body();
                let from = page_of(&request.uri().to_string());
                // A page of a document the window has since replaced says nothing more: it
                // still shows, frozen, till the new document's page is served. Only the page the
                // window loaded last says where the reader is, or that it's drawn.
                if from.is_none_or(|n| n < first_page.load(Ordering::SeqCst))
                    || ((body == "drawn" || body.starts_with("place\n"))
                        && from != Some(page.load(Ordering::SeqCst)))
                {
                    return;
                }
                // Unsaved edits refuse a navigation from now on, before the event loop sees it.
                noted.note(body);
                // The editor's newest unsaved text, kept as Windows ends the session: taken as
                // it comes, so the window never has an older one than the page has sent.
                if let Some(text) = editor::unsaved_text(body) {
                    if let Ok(mut ending) = ending.try_borrow_mut() {
                        ending.text = Some(Rc::from(text));
                    }
                    return;
                }
                if stand_in.as_ref().is_some_and(|s| s.holds(id, from, body)) {
                    return;
                }
                let event = if let Some(href) = body.strip_prefix("link\n") {
                    UserEvent::Link(id, from, href.to_owned())
                } else if body == "drawn" {
                    UserEvent::Drawn(id, from)
                } else {
                    UserEvent::Chrome(id, from, body.clone())
                };
                let _ = events.send_event(event);
            })
            // A file dropped on the window is opened by Legible, never shown by WebView2.
            .with_drag_drop_handler(move |event| {
                if let DragDropEvent::Drop { paths, .. } = event {
                    let _ = drops.send_event(UserEvent::Dropped(id, paths));
                }
                true
            })
            .with_navigation_handler(navigation_policy(nav_guard, move |navigation| {
                let event = match navigation {
                    editor::Navigation::Allowed => UserEvent::PageLoading(id),
                    editor::Navigation::Unsaved => UserEvent::NavigationRefused(id),
                    editor::Navigation::Foreign => return,
                };
                let _ = navigations.send_event(event);
            }))
            .with_new_window_req_handler(|url, _| {
                tracing::warn!(url, "refused a new window");
                NewWindowResponse::Deny
            })
            .with_focused(focused)
            .with_devtools(debug)
            .with_browser_accelerator_keys(debug)
            // The page zooms the document itself (Ctrl +/−/0, Ctrl+wheel), not the toolbar.
            .with_hotkeys_zoom(false)
            .build(window)
    }

    /// Does what a link the page sent does.
    fn follow(&mut self, id: WindowId, href: &str, target: &EventLoopWindowTarget<UserEvent>) {
        let Some(doc) = self.windows.get(&id) else {
            return;
        };
        // A window with no document still has About's release notes, whose web links open.
        let action = match &doc.file {
            Some(file) => links::classify(href, file),
            None => links::classify_without_document(href),
        };
        let owner = doc.window.hwnd();
        match action {
            LinkAction::InPage => {}
            LinkAction::Browser(url) | LinkAction::Mail(url) => open_outside(&url),
            LinkAction::BrowserAfterAsking(url) => {
                let text = tr!("link-http-ask", url = url.clone());
                if self.message_box(Some(owner), &tr("link-http-ask-title"), &text, Message::Ask) {
                    open_outside(&url);
                }
            }
            LinkAction::OpenDocument(file) => {
                self.open(&file, target);
            }
            LinkAction::Refuse(why) => tracing::info!(href, ?why, "a link that does nothing"),
        }
    }

    /// Does what the page's chrome asks.
    fn chrome(&mut self, id: WindowId, message: &str) {
        if let Some(place) = message.strip_prefix("place\n") {
            // Where the reader is: kept for the file as the window closes.
            if let Some(doc) = self.windows.get_mut(&id) {
                doc.place = Place::parse(place);
            }
            return;
        }
        if let Some(copy) = message.strip_prefix("copy\n") {
            let mut parts = copy.splitn(3, '\n');
            if let (Some(page_id), Some(generation), Some(html)) =
                (parts.next(), parts.next(), parts.next())
                && let (Ok(page_id), Ok(generation)) = (page_id.parse(), generation.parse())
            {
                self.copy(id, page_id, generation, html.to_owned());
            }
            return;
        }
        if let Some(message) = editor::Message::parse(message) {
            return self.editor(id, message);
        }
        let mut parts = message.split('\n');
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some("open"), None, ..) => {
                let Some(owner) = self.windows.get(&id).map(|d| d.window.hwnd()) else {
                    return;
                };
                if let Some(file) =
                    win32::open_file_dialog(owner, &tr("chrome-open"), &tr("chrome-open-filter"))
                {
                    self.open_here(id, &file);
                }
            }
            (Some("mode"), Some(mode), None, _) => {
                // The editor does the work; the menu bar shows it.
                tracing::info!(mode, "a view mode was asked for");
                if let (Some(doc), Some(mode)) =
                    (self.windows.get_mut(&id), menu::Mode::parse(mode))
                {
                    doc.mode = mode;
                }
            }
            (Some("outline"), Some(shown @ ("true" | "false")), None, _) => {
                if let Some(doc) = self.windows.get_mut(&id) {
                    doc.outline = shown == "true";
                }
            }
            (Some("setting"), Some(key), Some(value), None) => match Setting::parse(key, value) {
                Some(setting) => self.change_setting(id, setting),
                None => tracing::warn!(
                    key,
                    value,
                    "the page sent a setting that doesn't make sense"
                ),
            },
            (Some(answer @ ("reload" | "keep")), Some(n), None, _) => {
                if let Ok(n) = n.parse() {
                    self.answer_change(id, answer == "reload", n);
                }
            }
            (Some("recent"), None, ..) => self.send_recent(id),
            (Some("status"), None, ..) => self.send_status(id),
            (Some("copy-path"), None, ..) => self.copy_path(id),
            (Some("save-pdf"), None, ..) => self.save_pdf(id),
            (Some("pdf-open"), None, ..) => {
                if let Some(pdf) = self.windows.get(&id).and_then(|d| d.last_pdf.clone())
                    && let Err(error) = win32::shell_open(&pdf.to_string_lossy())
                {
                    tracing::warn!(%error, "couldn't open the PDF");
                }
            }
            (Some("pdf-show"), None, ..) => {
                if let Some(pdf) = self.windows.get(&id).and_then(|d| d.last_pdf.clone()) {
                    self.actions.request(id, actions::Action::ShowInFolder, pdf);
                }
            }
            // A page starting asks whether to show the moved-or-deleted bar (it may be one served
            // again); Keep showing it is remembered for the pages after it.
            (Some("file-status"), None, ..) => {
                if self.windows.get(&id).is_some_and(|d| d.file_bar.shows()) {
                    self.run_script(id, "window.LegibleChrome?.fileGone(true)");
                }
            }
            (Some("file-keep"), None, ..) => {
                if let Some(doc) = self.windows.get_mut(&id) {
                    doc.file_bar.keep_showing();
                }
            }
            (Some("remote-images"), None, ..) => self.send_pictures(id),
            (Some("remote-images"), Some(answer @ ("once" | "always")), Some(generation), None) => {
                if let Ok(generation) = generation.parse() {
                    self.consent(id, answer == "always", generation);
                }
            }
            (Some(asked @ ("open-editor" | "show-folder")), None, ..) => {
                let action = if asked == "open-editor" {
                    actions::Action::OpenInEditor
                } else {
                    actions::Action::ShowInFolder
                };
                // The window's own file, as the shell has it: never a path from the page.
                match self.windows.get(&id).and_then(|d| d.file.clone()) {
                    Some(file) => {
                        self.actions.request(id, action, file);
                    }
                    None => tracing::info!(?action, "asked of a window with no document"),
                }
            }
            (Some("open-recent"), Some(token), Some(index), None) => {
                let chosen = token
                    .parse()
                    .ok()
                    .zip(index.parse().ok())
                    .and_then(|(t, i)| self.recent.at(t, i))
                    .map(Path::to_owned);
                match chosen {
                    Some(file) => self.open_here(id, &file),
                    // From a list that has changed since: show the current one instead.
                    None => self.send_recent(id),
                }
            }
            (Some("close"), None, ..) => {
                let _ = self.proxy.send_event(UserEvent::Close(id));
            }
            (Some("default-app"), None, ..) => self.send_default_app(id),
            (Some("default-app-open"), None, ..) => {
                match win32::shell_open(associations::DEFAULT_APPS) {
                    Ok(()) => {
                        let text = json_string(&tr("chrome-default-how"));
                        self.run_script(id, &format!("window.LegibleChrome?.notice({text})"));
                    }
                    Err(error) => tracing::warn!(%error, "couldn't open Default apps"),
                }
            }
            (Some("default-app-close"), None, ..) => {
                self.default_offer_closed = true;
                let ids: Vec<WindowId> = self.windows.keys().copied().collect();
                for id in ids {
                    self.send_default_app(id);
                }
            }
            (Some(asked @ ("default-app-register" | "default-app-unregister")), None, ..) => {
                self.register(id, asked == "default-app-register");
            }
            (Some("about"), None, ..) => self.send_about(id),
            (Some("about-copy"), None, ..) => self.copy_about(id),
            (Some("reading"), Some(name), None, _) => self.send_reading(id, name),
            _ => tracing::warn!(message, "a message from the page Legible doesn't know"),
        }
    }

    /// The values Legible registers for its file associations, for this exe.
    fn association_values(&self) -> Vec<associations::Value> {
        associations::values(
            &self.exe,
            &tr("installer-file-type"),
            &tr("installer-app-description"),
        )
    }

    /// Whether Legible is registered for `.md` and is its default, for the bar and Settings.
    fn default_app(&self) -> associations::DefaultApp {
        let registry = win32::registry::Hive::current_user();
        associations::DefaultApp {
            registered: associations::is_registered(&registry, &self.association_values()),
            is_default: associations::is_default(
                win32::registry::md_handler().as_deref(),
                &self.exe,
            ),
            portable: self.portable,
            closed: self.default_offer_closed,
        }
    }

    fn send_default_app(&self, id: WindowId) {
        let script = format!(
            "window.LegibleChrome?.defaultApp({})",
            self.default_app().to_json()
        );
        self.run_script(id, &script);
    }

    /// Registers this exe for `.md` files (or unregisters it: only a portable copy, whose
    /// registration is the user's; the installer's is the MSI's), then tells every window.
    fn register(&mut self, from: WindowId, register: bool) {
        if !register && !self.portable {
            tracing::warn!("only a portable copy unregisters itself");
            return;
        }
        let mut registry = win32::registry::Hive::current_user();
        let values = self.association_values();
        let done = if register {
            associations::register(&mut registry, &values)
        } else {
            associations::unregister(&mut registry, &values)
        };
        win32::registry::associations_changed();
        if let Err(error) = done {
            tracing::warn!(%error, register, "couldn't change the file associations");
            let text = json_string(&tr!("chrome-default-failed", reason = reason::of(&error)));
            self.run_script(from, &format!("window.LegibleChrome?.notice({text})"));
        }
        let ids: Vec<WindowId> = self.windows.keys().copied().collect();
        for id in ids {
            self.send_default_app(id);
        }
    }

    /// Does what the page's editor asks.
    fn editor(&mut self, id: WindowId, message: editor::Message) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        match message {
            editor::Message::Source => {
                // The file's text as last applied: from then on the editor's is the truth.
                doc.editor_open = true;
                let script = format!(
                    "window.LegibleEditor?.load({{\"text\":{},\"editable\":{}}})",
                    json_string(doc.disk.as_ref().map_or("", |d| d.content.text())),
                    doc.editable && doc.file.is_some()
                );
                // Saving won't keep a file's mixed line endings: said once, as it's edited.
                let mixed = doc
                    .disk
                    .as_ref()
                    .and_then(|d| d.content.format())
                    .filter(|f| f.mixed && doc.editable && !doc.told_mixed)
                    .map(|f| f.line_ending);
                self.run_script(id, &script);
                if let Some(ending) = mixed {
                    if let Some(doc) = self.windows.get_mut(&id) {
                        doc.told_mixed = true;
                    }
                    let ending = match ending {
                        legible_core::save::LineEnding::Crlf => "crlf",
                        legible_core::save::LineEnding::Cr => "cr",
                        legible_core::save::LineEnding::Lf => "lf",
                    };
                    self.notice(id, &tr!("chrome-line-endings-mixed", ending = ending));
                }
            }
            editor::Message::Render { version, text } => {
                let Some(folder) = doc.file.as_deref().and_then(Path::parent) else {
                    return;
                };
                let options = RenderOptions {
                    sourcepos: true,
                    ..RenderOptions::default()
                };
                let generation = doc.generation;
                let html =
                    page::with_generation(&render_with(text, folder, &options).html, generation);
                // With the render it belongs to, and what its pictures from the internet may do.
                let script = format!(
                    "window.LegibleEditor?.preview({version},{},{{\"generation\":{generation},\"remote\":{}}})",
                    json_string(&html),
                    self.pictures_message(id)
                );
                self.run_script(id, &script);
            }
            editor::Message::Dirty(dirty) => {
                // While dirty, live reload holds what changes on disk (reload.rs), and the page
                // isn't served again; saved, a reload that waited goes ahead.
                doc.reload.set_dirty(dirty);
                doc.show_title();
                if !dirty && let Some(origins) = doc.reload_gate.clean() {
                    self.serve_again(id, origins);
                }
            }
            editor::Message::Contents {
                request,
                version,
                text,
            } => self.contents(id, request, version, text),
            editor::Message::Save => self.save(id),
            editor::Message::SaveCopy => self.save_copy(id),
            editor::Message::Clean(clean) => self.clean(id, clean),
            editor::Message::Compare(n) => self.compare(id, n),
            editor::Message::Raced { answer, id: n } => self.race_answer(id, answer, n),
            editor::Message::Restore(restore) => self.restore_answer(id, restore),
            editor::Message::Restored => self.restored(id),
        }
    }

    /// Changes a setting, saves the settings and gives them to every window. A window's own
    /// zoom isn't touched: the default zoom is for new windows.
    fn change_setting(&mut self, from: WindowId, setting: Setting) {
        self.settings.set(setting);
        let failed = match (&self.settings_file, self.settings_writable) {
            (Some(file), true) => self.settings.save(file).err().map(|error| {
                tracing::warn!(%error, "couldn't save the settings");
                tr!("chrome-save-failed", reason = reason::of(&error))
            }),
            _ => Some(tr("chrome-save-read-only")),
        };
        let script = format!("window.LegibleChrome?.apply({})", self.settings.to_json());
        let ids: Vec<WindowId> = self.windows.keys().copied().collect();
        for &id in &ids {
            self.run_script(id, &script);
        }
        if matches!(setting, Setting::RemoteImages(_)) {
            for &id in &ids {
                self.apply_pictures(id);
            }
        }
        // Turned off: what's kept of where the reader was goes too.
        if setting == Setting::RememberPlace(false) {
            self.places.clear();
            self.save_places();
            for doc in self.windows.values_mut() {
                doc.place = None;
            }
        }
        if matches!(setting, Setting::Theme(_)) {
            self.bars_theme();
        }
        if let Some(text) = failed {
            self.run_script(
                from,
                &format!("window.LegibleChrome?.saveFailed({})", json_string(&text)),
            );
        }
    }

    /// What live reload found for the window watched under `key`.
    fn live(&mut self, key: u64, live: Live) {
        let Some(&id) = self.watch_keys.get(&key) else {
            return;
        };
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let (kind, on_disk) = match live {
            Live::ReadFailed { error } => {
                tracing::info!(error, "a changed document couldn't be read yet");
                return;
            }
            Live::Changed(epoch, _) if epoch != doc.watch_epoch => {
                tracing::debug!(
                    epoch,
                    "a live reload report from before the window registered again"
                );
                return;
            }
            Live::Changed(_, on_disk) => (Kind::Changed, Some(on_disk)),
            Live::Deleted => (Kind::Deleted, None),
        };
        match doc.reload.on_disk(kind) {
            Action::Apply(Kind::Changed) => {
                if let Some(on_disk) = on_disk {
                    self.show_from_disk(id, on_disk);
                }
            }
            Action::Apply(Kind::Deleted) => self.file_gone(id),
            Action::Hold { kind, id: n } => {
                doc.held = on_disk;
                self.announce(id, kind.as_str(), n, None);
            }
        }
    }

    /// The safe-saving prompt's answer for held change `n`: take it from disk (discarding the
    /// edits) or keep the edits. An answer for an older change does nothing.
    fn answer_change(&mut self, id: WindowId, take: bool, n: u64) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        if !take {
            let pending = doc.reload.pending();
            if doc.reload.keep(n) {
                let held = doc.held.take();
                self.kept(id, n, pending.map(|(kind, _)| (kind, held)));
            }
            return;
        }
        doc.saves.forget_conflict();
        let taken = doc.reload.reload(n);
        // Use the file discards the edits: the title is clean at once.
        doc.show_title();
        let name = doc.title_name();
        match taken {
            Some(Kind::Changed) => {
                match doc.held.take() {
                    Some(on_disk) => self.show_from_disk(id, on_disk),
                    None => {
                        if let Some(key) = doc.watch_key {
                            self.watch.rescan(key);
                        }
                    }
                }
                self.notice(id, &tr!("chrome-conflict-taken", file = name));
            }
            Some(Kind::Deleted) => self.file_gone(id),
            None => tracing::info!(n, "an answer for a change that isn't the newest"),
        }
        // The page froze its text when Use the file was confirmed: the file's text is in it
        // now (or nothing was taken), so it takes edits again.
        self.unfreeze_after_take(id);
    }

    /// Shows the document as it now is on disk. Until the editor opens, the page is sent the
    /// render; from then on the editor is sent the text, as what's saved, and renders it for
    /// its own preview, so only one of them ever writes the document.
    fn show_from_disk(&mut self, id: WindowId, on_disk: OnDisk) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        let editable_text = on_disk.disk.content.format().is_some();
        let text = on_disk.disk.content.text().to_owned();
        doc.expected = Some(on_disk.disk.stamp);
        doc.disk = Some(on_disk.disk);
        // Its encoding or line endings may have changed: the status bar says so.
        self.send_status(id);
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        if !doc.editor_open {
            return self.show_render(id, on_disk.html);
        }
        // As show_render keeps them: the pictures from the internet, the moved-or-deleted bar
        // and the page's CSP.
        doc.pictures.render(&on_disk.html);
        doc.document = on_disk.html;
        doc.file_bar.back();
        match self.grant_pictures(id) {
            // The page is served again: its editor opens on the text from there.
            Some(Next::ServeAgain(origins)) => return self.serve_again(id, origins),
            // A new generation, so pictures that changed on disk are fetched again.
            Some(Next::Tell) => {
                if let Some(doc) = self.windows.get_mut(&id) {
                    doc.generation += 1;
                }
            }
            None => return,
        }
        // Never as the editor's text what isn't the file's own (a lossy read): the page shows
        // it, and saving it is refused (its stamp has moved on).
        if !editable_text {
            let note = json_string(&tr("chrome-encoding-unsupported"));
            return self.run_script(id, &format!("window.LegibleChrome?.notice({note})"));
        }
        let script = format!("window.LegibleEditor?.fromDisk({})", json_string(&text));
        self.run_script(id, &script);
    }

    /// Sends a new render to the window's page, as a new generation. If its pictures from the
    /// internet now need another origin in the page's CSP, the page is served again instead.
    fn show_render(&mut self, id: WindowId, html: String) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        doc.pictures.render(&html);
        doc.document = html;
        doc.file_bar.back();
        match self.grant_pictures(id) {
            Some(Next::ServeAgain(origins)) => self.serve_again(id, origins),
            Some(Next::Tell) => {
                let Some(doc) = self.windows.get_mut(&id) else {
                    return;
                };
                doc.generation += 1;
                let generation = doc.generation;
                let html = page::with_generation(&doc.document, generation);
                let script = format!(
                    "window.LegibleChrome?.replaceDocument({{ generation: {}, html: {}, remote: {} }})",
                    generation,
                    json_string(&html),
                    self.pictures_message(id)
                );
                self.run_script(id, &script);
            }
            None => {}
        }
    }

    /// Window `id`'s page asks to copy `html` (the document, or a selection), as its copy
    /// `page_id`, made from render `generation`. Accepted, it's numbered in the process's order
    /// and built on the copying thread with the window's document folder as it is now.
    fn copy(&mut self, id: WindowId, page_id: u64, generation: u64, html: String) {
        let page = Ticket {
            id: page_id,
            generation,
        };
        let Some(doc) = self.windows.get(&id) else {
            return;
        };
        if doc.file.is_none() || generation != doc.generation {
            // From another render than the one shown, or no document: it never was this one's.
            self.copy_answer(id, page, "superseded", "");
            return;
        }
        let request = clipboard::Request {
            seq: self.copy_order.accept(),
            window: id,
            page,
            html,
            root: doc.root.clone(),
        };
        if let Some(replaced) = self.copier.request(request) {
            self.copy_answer(replaced.window, replaced.page, "superseded", "");
        }
    }

    /// A copy built: written to the clipboard if it's still the process's newest, by its window.
    fn copy_built(&mut self, built: Built<WindowId>) {
        match built {
            Built::Superseded { window, page } => {
                self.copy_answer(window, page, "superseded", "");
            }
            Built::Ready {
                seq,
                window,
                page,
                html,
                text,
            } => {
                if let Some(old) = self.copy_waiting.take() {
                    self.copy_answer(old.window, old.page, "superseded", "");
                }
                self.copy_write(CopyWaiting {
                    seq,
                    window,
                    page,
                    html,
                    text,
                    tries: 0,
                });
            }
        }
    }

    fn copy_retry(&mut self, seq: u64) {
        match self.copy_waiting.take() {
            Some(waiting) if waiting.seq == seq => self.copy_write(waiting),
            other => self.copy_waiting = other,
        }
    }

    fn copy_write(&mut self, mut copy: CopyWaiting) {
        if !self.copy_order.is_newest(copy.seq) {
            self.copy_answer(copy.window, copy.page, "superseded", "");
            return;
        }
        let Some(owner) = self.windows.get(&copy.window).map(|d| d.window.hwnd()) else {
            return;
        };
        let written = match &self.stand_in {
            Some(stand_in) => stand_in.clipboard(&copy.html, &copy.text),
            None => {
                let mut port = win32::clipboard::WindowsClipboard::new(owner);
                clipboard::write(&mut port, &copy.html, &copy.text)
            }
        };
        match written {
            Written::Both => self.copy_answer(copy.window, copy.page, "copied", ""),
            Written::TextOnly => self.copy_answer(copy.window, copy.page, "text-only", ""),
            Written::Busy if copy.tries + 1 < COPY_TRIES => {
                // Another app has the clipboard: try again shortly, without holding the window.
                copy.tries += 1;
                let seq = copy.seq;
                self.copy_waiting = Some(copy);
                let proxy = self.proxy.clone();
                thread::spawn(move || {
                    thread::sleep(COPY_RETRY_AFTER);
                    let _ = proxy.send_event(UserEvent::CopyRetry(seq));
                });
            }
            Written::Busy => {
                let text = tr!("chrome-copy-failed", reason = tr("chrome-copy-busy"));
                self.copy_answer(copy.window, copy.page, "failed", &text);
            }
            Written::Failed(reason) => {
                tracing::warn!(reason, "couldn't copy");
                let text = tr!("chrome-copy-failed", reason = reason);
                self.copy_answer(copy.window, copy.page, "failed", &text);
            }
        }
    }

    /// Tells window `id`'s page how its copy `page` ended: named by its number and the render
    /// it was made from, so a page served since never takes it for its own.
    fn copy_answer(&self, id: WindowId, page: Ticket, outcome: &str, text: &str) {
        let script = format!(
            "window.LegibleChrome?.copied({{ id: {}, generation: {}, outcome: {}, text: {} }})",
            page.id,
            page.generation,
            json_string(outcome),
            json_string(text)
        );
        self.run_script(id, &script);
    }

    /// Save as PDF, for window `id`: one job at a time. Its folder is found off the UI thread.
    fn save_pdf(&mut self, id: WindowId) {
        // A window still reading its file has only a blank page to print.
        let Some(file) = self
            .windows
            .get(&id)
            .filter(|d| d.opening.is_none())
            .and_then(|d| d.file.clone())
        else {
            return;
        };
        let Some(job) = self.pdf_jobs.start(id) else {
            let text = json_string(&tr("chrome-pdf-busy"));
            self.run_script(id, &format!("window.LegibleChrome?.notice({text})"));
            return;
        };
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let folder = export::default_folder(
                file.parent(),
                export::can_write_in,
                win32::pdf::documents_folder(),
            );
            let _ = proxy.send_event(UserEvent::PdfFolder(id, job, folder));
        });
    }

    /// Job `job`'s dialog, opening in `folder`; then WebView2 prints to a staging file.
    fn pdf_folder(&mut self, id: WindowId, job: u64, folder: Option<PathBuf>) {
        if !self.pdf_jobs.choosing(job) {
            return;
        }
        let Some(doc) = self.windows.get(&id) else {
            return;
        };
        let name = export::default_name(doc.file.as_deref().unwrap_or(Path::new("")));
        let chosen = win32::pdf::save_pdf_dialog(
            doc.window.hwnd(),
            &tr("window-save-pdf-title"),
            &tr("window-save-pdf-filter"),
            &name,
            folder.as_deref(),
        );
        let (Some(chosen), Some(doc)) = (chosen, self.windows.get(&id)) else {
            // Cancelled (nothing to say), or the window went meanwhile.
            self.pdf_jobs.cancelled(job);
            return;
        };
        let staging = export::staging_path(&chosen, std::process::id(), job);
        let proxy = self.proxy.clone();
        let started = win32::pdf::print_to_pdf(
            &doc.webview.webview(),
            &staging,
            win32::pdf::locale_paper(),
            Box::new(move |result| {
                let result = result.map_err(|error| {
                    tracing::warn!(%error, "WebView2 couldn't print a PDF");
                    reason::of(&error)
                });
                let _ = proxy.send_event(UserEvent::PdfPrinted(job, result));
            }),
        );
        match started {
            Ok(()) => {
                self.pdf_jobs.printing(job, staging, chosen);
                let proxy = self.proxy.clone();
                thread::spawn(move || {
                    thread::sleep(PDF_BOUND);
                    let _ = proxy.send_event(UserEvent::PdfTimeout(job));
                });
            }
            Err(error) => {
                self.pdf_jobs.cancelled(job);
                tracing::warn!(%error, "WebView2 didn't start printing a PDF");
                self.pdf_failed(id, &reason::of(&error));
            }
        }
    }

    fn pdf_printed(&mut self, job: u64, result: Result<(), String>) {
        let steps = self.pdf_jobs.printed(job, result);
        self.pdf_steps(steps);
    }

    fn pdf_committed(&mut self, job: u64, result: Result<(), String>) {
        let steps = self.pdf_jobs.committed(job, result);
        self.pdf_steps(steps);
    }

    fn pdf_timeout(&mut self, job: u64) {
        let steps = self.pdf_jobs.timed_out(job, &tr("chrome-pdf-timed-out"));
        self.pdf_steps(steps);
    }

    /// Does what Save as PDF's jobs say: file work off the UI thread, messages to windows.
    fn pdf_steps(&mut self, steps: Vec<export::Step<WindowId>>) {
        for step in steps {
            match step {
                export::Step::Commit {
                    job,
                    staging,
                    chosen,
                } => {
                    let proxy = self.proxy.clone();
                    thread::spawn(move || {
                        let committed = export::commit(&staging, &chosen);
                        let _ = proxy.send_event(UserEvent::PdfCommitted(job, committed));
                    });
                }
                export::Step::Remove(staging) => {
                    // WebView2 may hold it a moment after a print ends: tried again, for a while,
                    // and reported done either way (the jobs count it until then).
                    let proxy = self.proxy.clone();
                    thread::spawn(move || {
                        if !export::remove_staging(&staging, PDF_CLEANUP_BOUND) {
                            tracing::warn!(path = %staging.display(), "couldn't remove a PDF's staging file");
                        }
                        let _ = proxy.send_event(UserEvent::PdfRemoved);
                    });
                }
                export::Step::Saved { window, chosen } => {
                    let name = chosen
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if let Some(doc) = self.windows.get_mut(&window) {
                        doc.last_pdf = Some(chosen);
                    }
                    let script = format!(
                        "window.LegibleChrome?.pdfSaved({{ name: {} }})",
                        json_string(&name)
                    );
                    self.run_script(window, &script);
                }
                export::Step::Failed { window, reason } => self.pdf_failed(window, &reason),
            }
        }
    }

    fn pdf_failed(&self, id: WindowId, reason: &str) {
        tracing::warn!(reason, "couldn't save a PDF");
        let text = json_string(&tr!("chrome-pdf-failed", reason = reason.to_owned()));
        self.run_script(id, &format!("window.LegibleChrome?.notice({text})"));
    }

    /// Window `id`'s file was moved or deleted: its page shows the bar, and so does any page
    /// served after it.
    fn file_gone(&mut self, id: WindowId) {
        if let Some(doc) = self.windows.get_mut(&id) {
            doc.file_bar.deleted();
        }
        self.run_script(id, "window.LegibleChrome?.fileGone(true)");
    }

    /// Whether `file`'s pictures from the internet always show ("Always for this file").
    fn always_for(&self, file: Option<&Path>) -> bool {
        file.is_some_and(|f| self.always.contains(f))
    }

    /// Lets window `id`'s request filter through exactly what may load now (`consent`).
    fn grant_pictures(&mut self, id: WindowId) -> Option<Next> {
        let setting = self.settings.remote_images;
        let always = self.always_for(self.windows.get(&id)?.file.as_deref());
        Some(self.windows.get_mut(&id)?.pictures.grant(setting, always))
    }

    /// Brings window `id`'s pictures from the internet in line with what may load: the filter
    /// first (so what's taken away is refused at once), then the page, served again if its CSP
    /// must change, else told.
    fn apply_pictures(&mut self, id: WindowId) {
        match self.grant_pictures(id) {
            Some(Next::ServeAgain(origins)) => self.serve_again(id, origins),
            Some(Next::Tell) => self.send_pictures(id),
            None => {}
        }
    }

    /// Serves window `id`'s page again, its document as a new generation, with a CSP letting
    /// pictures in from `origins`; the page keeps the reader's place across it.
    ///
    /// Never while the editor has unsaved edits: a reload would lose them, so it waits for them
    /// to be saved ([`editor::ReloadGate`]), and the page says so.
    fn serve_again(&mut self, id: WindowId, origins: Vec<String>) {
        let lang = self.lang.clone();
        let settings = self.settings.clone();
        // A window still reading its file serves its first page once it has, as things are then.
        let Some(doc) = self.windows.get_mut(&id).filter(|d| d.opening.is_none()) else {
            return;
        };
        // Dirty as the window has it, or as the page has just said (the guard hears it first).
        let dirty = doc.reload.is_dirty() || doc.nav_guard.dirty();
        let Some(origins) = doc.reload_gate.request(origins, dirty) else {
            return self.run_script(id, "window.LegibleChrome?.reloadWaits(true)");
        };
        // What's put back if the navigation is refused (the editor became dirty meanwhile); a
        // serve still waiting for its own keeps the page from before it.
        let generation = doc.generation;
        doc.generation += 1;
        let name = doc
            .file
            .as_deref()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let html = page::document_page(
            &lang,
            &tr!("window-title", file = name),
            &page::with_generation(&doc.document, doc.generation),
            &settings,
            doc.generation,
            &origins,
            doc.editable,
        );
        let before = std::mem::take(&mut *doc.served.lock().unwrap_or_else(|e| e.into_inner()));
        serve(&doc.served, html, &origins);
        let pending = match doc.pending_serve.take() {
            Some(older) => PendingServe { origins, ..older },
            None => PendingServe {
                before,
                page: doc.page.load(Ordering::SeqCst),
                generation,
                origins,
            },
        };
        let n = doc.page.fetch_add(1, Ordering::SeqCst) + 1;
        if let Err(error) = doc.webview.load_url(&page_url(n)) {
            tracing::warn!(%error, "couldn't serve a window's page again");
            *doc.served.lock().unwrap_or_else(|e| e.into_inner()) = pending.before;
            doc.generation = pending.generation;
            doc.page.store(n - 1, Ordering::SeqCst);
            return;
        }
        doc.pending_serve = Some(pending);
    }

    /// The window's WebView is loading its page: a page served again takes effect (its
    /// pictures' CSP), and the new page's editor opens on the file's text, when it's asked for.
    fn page_loading(&mut self, id: WindowId) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        doc.editor_open = false;
        // A new page starts in View, its outline as it decides.
        doc.mode = menu::Mode::View;
        doc.outline = false;
        if let Some(pending) = doc.pending_serve.take() {
            doc.pictures.served(pending.origins);
        }
    }

    /// A navigation of the window's page was refused because the editor has unsaved edits. A
    /// page the window was serving again is put back as it was, and waits for the edits to be
    /// saved, as any reload does.
    fn navigation_refused(&mut self, id: WindowId) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        tracing::info!("a navigation of the page was refused: the editor has unsaved edits");
        let Some(pending) = doc.pending_serve.take() else {
            return;
        };
        *doc.served.lock().unwrap_or_else(|e| e.into_inner()) = pending.before;
        doc.generation = pending.generation;
        doc.page.store(pending.page, Ordering::SeqCst);
        doc.reload_gate.request(pending.origins, true);
        self.run_script(id, "window.LegibleChrome?.reloadWaits(true)");
    }

    /// Tells window `id`'s page what its pictures from the internet may do.
    fn send_pictures(&self, id: WindowId) {
        let script = format!(
            "window.LegibleChrome?.remoteImages({})",
            self.pictures_message(id)
        );
        self.run_script(id, &script);
    }

    fn pictures_message(&self, id: WindowId) -> String {
        let Some(doc) = self.windows.get(&id) else {
            return "null".into();
        };
        doc.pictures.message(
            doc.generation,
            self.settings.remote_images,
            self.always_for(doc.file.as_deref()),
        )
    }

    /// The bar's answer for render `generation`: Show once, or Always for this file. An
    /// answer for a render that isn't the newest is ignored (the bar asks again for that).
    fn consent(&mut self, id: WindowId, always: bool, generation: u64) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        if generation != doc.generation || doc.pictures.unavailable().is_some() {
            tracing::info!(
                generation,
                "an answer about pictures that isn't for the newest render"
            );
            return;
        }
        if always {
            let Some(file) = doc.file.clone() else {
                return;
            };
            self.always.add(file);
            if let Some(path) = &self.always_file
                && let Err(error) = self.always.save(path)
            {
                tracing::warn!(%error, "couldn't save the files that always show pictures");
            }
        } else {
            doc.pictures.show_once();
        }
        self.apply_pictures(id);
    }

    /// Window `id`'s request filter couldn't refuse a request: its document shows no pictures
    /// from the internet from now on, and says why.
    fn remote_failed(&mut self, id: WindowId) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        if doc.pictures.failed() {
            tracing::error!(
                "a request couldn't be refused: pictures from the internet are off for this document"
            );
            // The page takes them away now; its CSP is tightened when it's served again.
            self.send_pictures(id);
            self.apply_pictures(id);
        }
    }

    fn run_script(&self, id: WindowId, script: &str) {
        if let Some(doc) = self.windows.get(&id)
            && let Err(error) = doc.webview.evaluate_script(script)
        {
            tracing::warn!(%error, "couldn't run a script in a window");
        }
    }

    fn drawn(&mut self, id: WindowId) {
        // A document that replaced another opens in the mode the reader had, if it can be
        // edited.
        if let Some(doc) = self.windows.get_mut(&id)
            && let Some(mode) = doc.mode_after.take().filter(|_| doc.editable)
            && let menu::Route::Page(command) = menu::Command::Mode(mode).route()
        {
            let script = format!("window.LegibleChrome?.command({})", json_string(command));
            self.run_script(id, &script);
        }
        // Where the reader was in the file last time: once, as it opens.
        if let Some(place) = self.windows.get_mut(&id).and_then(|d| d.go_to.take()) {
            self.run_script(
                id,
                &format!("window.LegibleChrome?.goTo({})", place.to_json()),
            );
        }
        // A note the window kept for its page (a file that opens for reading only, say).
        if let Some(note) = self.windows.get_mut(&id).and_then(|d| d.note.take()) {
            self.run_script(
                id,
                &format!("window.LegibleChrome?.notice({})", json_string(&note)),
            );
        }
        // Both versions of a raced save are still kept: a page served again asks again, and so
        // does the offer of unsaved edits kept from an earlier session.
        self.announce_race(id);
        self.offer_restore(id);
        if !self.drawn_timed {
            self.drawn_timed = true;
            tracing::info!(ms = win32::process_age_ms(), "the first document is drawn");
        }
    }

    /// Closes a window, once nothing is unsaved or the reader chose Don't save
    /// ([`Shell::ask_close`] asks). With the reading place remembered, its page is asked first,
    /// in one script, to take no more edits (frozen, as a close's save freezes it) and to say
    /// whether anything is unsaved and where the reader is that moment (the scroll it sends may
    /// not have come yet). The window closes with the answer, or after [`PLACE_BOUND`] if none
    /// comes: unless an edit came after the close was decided (the window had nothing unsaved
    /// then, and has now), when it asks the close question instead, the page taking edits again.
    /// A close that drops unsaved edits as the reader chose (Don't save) goes on.
    fn close(&mut self, id: WindowId) {
        let Some(doc) = self.windows.get(&id) else {
            return;
        };
        // A window still reading its file has no page to ask.
        if !self.settings.remember_place || doc.file.is_none() || doc.opening.is_some() {
            return self.closed(id);
        }
        if self.closing.contains_key(&id) {
            return;
        }
        let dropping = doc.unsaved();
        self.next_close += 1;
        let n = self.next_close;
        let answers = self.proxy.clone();
        let asked = doc
            .webview
            .evaluate_script_with_callback(CLOSING, move |answer| {
                let _ = answers.send_event(UserEvent::PlaceTaken(id, n, answer));
            });
        if asked.is_err() {
            return self.closed(id);
        }
        self.closing.insert(id, (n, dropping));
        let overdue = self.proxy.clone();
        thread::spawn(move || {
            thread::sleep(PLACE_BOUND);
            let _ = overdue.send_event(UserEvent::PlaceTaken(id, n, String::new()));
        });
    }

    /// A closing window's page answered (`unsaved` or `clean`, then its place), or ran out of
    /// time (empty): the place is kept and the window closes, or, with an edit that came after
    /// the close was decided, it asks. The later of the answer and the bound does nothing, as
    /// does either for an earlier close (of a document replaced since).
    fn place_taken(&mut self, id: WindowId, n: u64, answer: &str) {
        let Some(&(asked, dropping)) = self.closing.get(&id) else {
            return;
        };
        if asked != n {
            return;
        }
        self.closing.remove(&id);
        let answer = places::from_script(answer);
        let unsaved = match answer.as_deref().and_then(|a| a.split_once('\n')) {
            Some((state, place)) => {
                if let Some(place) = Place::parse(place)
                    && let Some(doc) = self.windows.get_mut(&id)
                {
                    doc.place = Some(place);
                }
                state == "unsaved"
            }
            // No answer: the window's own account of the edits.
            None => self.windows.get(&id).is_some_and(DocWindow::unsaved),
        };
        if unsaved && !dropping {
            // Asked, as any close with unsaved edits is, the page taking edits again first.
            let Some(doc) = self.windows.get_mut(&id) else {
                return;
            };
            let frozen = doc.saves.frozen();
            let close = doc.saves.close(true);
            if !frozen {
                self.run_script(id, "window.LegibleEditor?.unfreeze()");
            }
            return self.carry_out(id, close);
        }
        self.closed(id);
    }

    /// A window's close is decided, nothing unsaved: it closes now, or, replacing its document,
    /// shows the new one instead.
    fn closed(&mut self, id: WindowId) {
        match self.windows.get_mut(&id).and_then(|d| d.replacing.take()) {
            Some(file) => self.replace(id, file),
            None => self.close_now(id),
        }
    }

    /// What goes with a window's document, closed or replaced. Saved, or dropped as the reader
    /// chose: no copy of its text is kept, and it stops no end of the session. Save as PDF's
    /// work for it stops, live reload stops watching its file, and where the reader was in it is
    /// kept.
    fn retire(&mut self, id: WindowId) {
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        doc.ending.borrow_mut().forget(doc.window.hwnd());
        let (watch_key, file, place) = (doc.watch_key.take(), doc.file.take(), doc.place.take());
        let steps = self.pdf_jobs.window_closed(id);
        self.pdf_steps(steps);
        if let Some(key) = watch_key {
            // Returns at once, whatever live reload is doing.
            self.watch.unregister(key);
            self.watch_keys.remove(&key);
        }
        if let Some(file) = &file {
            self.by_file.remove(file);
            if self.settings.remember_place
                && let Some(place) = place
            {
                self.places.set(file, place);
                self.save_places();
            }
        }
    }

    /// Window `id` shows `file` (canonical) in place of its document, which has nothing unsaved
    /// now. Everything that followed the old document goes with it ([`Shell::retire`]), and the
    /// window opens on the new file as a new window does: read and rendered on a thread of its
    /// own, its page served once that's done, with the bars, the outline and the status bar its
    /// own, and its own pictures from the internet asked about. Its mode stays as the reader had
    /// it, if the new file can be edited.
    fn replace(&mut self, id: WindowId, file: PathBuf) {
        // Opened in another window while the question was answered: that one comes forward, and
        // this one's page, frozen as it closed, takes edits again.
        if let Some(&open) = self.by_file.get(&file) {
            self.run_script(id, "window.LegibleEditor?.unfreeze()");
            return self.focus(open);
        }
        self.retire(id);
        // Taken before the file is read, so a change in between is live reload's first report.
        let before = self.watch.document_snapshot(&file);
        let root = DocRoot::for_document(&file).ok();
        let go_to = self.place_in(Some(&file));
        let ending = self.ending_for(Some(&file));
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        // The old page takes no more edits, and may be navigated from: what it held was saved,
        // or dropped as the reader chose.
        if let Err(error) = doc
            .webview
            .evaluate_script("window.LegibleEditor?.freeze?.()")
        {
            tracing::warn!(%error, "couldn't stop the replaced document's page taking edits");
        }
        doc.nav_guard.clear();
        // Its pages are left behind from now on, though the last shows till the new document's
        // is served: whatever they say is the old document's.
        doc.first_page
            .store(doc.page.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
        *doc.ending.borrow_mut() = ending;
        *doc.shared_root
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = root.clone();
        doc.mode_after = Some(doc.mode).filter(|m| *m != menu::Mode::View);
        let read = self.next_read + 1;
        doc.opening = Some(Opening {
            before,
            in_place: true,
            read,
        });
        doc.file = Some(file.clone());
        doc.root = root.clone();
        doc.title = title_of(Some(&file));
        doc.go_to = go_to;
        doc.editable = false;
        doc.generation = 0;
        doc.reload = reload::State::default();
        doc.held = None;
        doc.file_bar = FileBar::default();
        doc.last_pdf = None;
        doc.document = String::new();
        doc.pictures = doc.pictures.for_another_document();
        doc.disk = None;
        doc.watch_epoch = 0;
        doc.note = None;
        doc.editor_open = false;
        doc.reload_gate = editor::ReloadGate::default();
        doc.pending_serve = None;
        doc.saves = saving::Session::default();
        doc.expected = None;
        doc.asked.clear();
        doc.writing = None;
        doc.race = None;
        doc.close_after_take = false;
        doc.told_mixed = false;
        // Offered again next time Legible starts: the reader hasn't chosen.
        doc.offered.clear();
        doc.restoring.clear();
        doc.show_title();
        self.by_file.insert(file.clone(), id);
        self.read(id, file, root);
    }

    /// Whether what a page said (`from`) is of window `id`'s document: not of one it has since
    /// replaced.
    fn of_the_document(&self, id: WindowId, from: From) -> bool {
        let Some(doc) = self.windows.get(&id) else {
            return false;
        };
        from.is_none_or(|n| n >= doc.first_page.load(Ordering::SeqCst))
    }

    /// Window `id`'s new document couldn't be read: it shows none, as an empty window does.
    fn emptied(&mut self, id: WindowId) {
        let ending = self.ending_for(None);
        let Some(doc) = self.windows.get_mut(&id) else {
            return;
        };
        if let Some(file) = doc.file.take() {
            self.by_file.remove(&file);
        }
        *doc.ending.borrow_mut() = ending;
        *doc.shared_root
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        doc.root = None;
        doc.title = title_of(None);
        doc.go_to = None;
        doc.mode_after = None;
        doc.show_title();
        self.serve_first(id);
    }

    /// Reads and renders `file` for window `id` on a thread of its own: its answer is
    /// [`UserEvent::Opened`], under the read's number, which this returns.
    fn read(&mut self, id: WindowId, file: PathBuf, root: Option<DocRoot>) -> u64 {
        self.next_read += 1;
        let read = self.next_read;
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let opened = read_and_render(&file, root.as_ref());
            let _ = proxy.send_event(UserEvent::Opened(id, (read, opened)));
        });
        read
    }

    /// Where the reader was in `file` last time, unless Settings says not to remember.
    fn place_in(&self, file: Option<&Path>) -> Option<Place> {
        file.filter(|_| self.settings.remember_place)
            .and_then(|f| self.places.get(f))
            .cloned()
    }

    /// What a window's listener for the session's end starts with, for its document `file`.
    fn ending_for(&self, file: Option<&Path>) -> Ending {
        Ending {
            name: file
                .and_then(Path::file_name)
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            document: file.map(Path::to_owned),
            session: self.session,
            ..Ending::default()
        }
    }

    /// Closes a window now, remembering its size and place for the next.
    fn close_now(&mut self, id: WindowId) {
        self.retire(id);
        let Some(doc) = self.windows.remove(&id) else {
            return;
        };
        let hwnd = doc.window.hwnd();
        self.endings
            .borrow_mut()
            .retain(|(window, _)| *window != hwnd);
        // Asked as Windows ends the session, the next unsaved window asks.
        if self.round.on() {
            let _ = self.proxy.send_event(UserEvent::SessionNext);
        }
        if self.last_focused == Some(id) {
            self.last_focused = None;
        }
        let maximized = doc.window.is_maximized();
        let size = if maximized || doc.window.is_minimized() {
            // Its size now isn't the one it would be given back: keep the last one.
            self.remembered.map_or(placement::FIRST, |p| p.size)
        } else {
            let logical = doc
                .window
                .inner_size()
                .to_logical::<f64>(doc.window.scale_factor());
            Size {
                width: logical.width,
                height: logical.height,
            }
        };
        // Where it is (maximised, the monitor it's on); minimised, where it was last. A quiet
        // window still off every monitor leaves the next to open off them too.
        let position = if doc.window.is_minimized() {
            self.remembered.and_then(|p| p.position)
        } else if self.quiet && !quiet::on_a_monitor(&doc.window) {
            None
        } else {
            doc.window.outer_position().ok().map(|p| (p.x, p.y))
        };
        let placement = Placement {
            size,
            maximized,
            position,
        };
        self.remembered = Some(placement);
        if let Some(file) = &self.placement_file
            && let Err(error) = placement.save(file)
        {
            tracing::warn!(%error, "couldn't remember the window's size");
        }
    }

    // --- Windows ending the session ----------------------------------------------------------

    /// Has the window with handle `hwnd` hear Windows ending the session (`session`). Windows
    /// may ask no other window once one has stopped the end, so whichever it asks answers for
    /// them all, from what each [`Ending`] knows: a copy kept of anything unsaved, each unsaved
    /// window's reason given, and the end stopped if any is; the unsaved windows then ask in turn.
    fn listen_for_the_end(&self, hwnd: isize) {
        let endings = self.endings.clone();
        let folder = self.unsaved_folder.clone();
        let proxy = self.proxy.clone();
        let listened = win32::session::listen(hwnd, move |heard| {
            // Never borrowed across a wait: if it is, the shell is mid-change, and the end goes.
            let Ok(endings) = endings.try_borrow() else {
                return true;
            };
            let each = |act: &mut dyn FnMut(isize, &mut Ending)| {
                for (window, ending) in endings.iter() {
                    if let Ok(mut ending) = ending.try_borrow_mut() {
                        act(*window, &mut ending);
                    }
                }
            };
            match heard {
                Heard::Query { critical } => {
                    let mut stopped = false;
                    each(&mut |window, ending| {
                        stopped |= ending.query(window, critical, folder.as_deref())
                            == Answer::KeepAndStop;
                    });
                    if stopped {
                        let _ = proxy.send_event(UserEvent::SessionStopped);
                    }
                    !stopped
                }
                // Ending now: the newest text of each unsaved window is kept.
                Heard::Ended(true) => {
                    each(&mut |_, ending| {
                        if ending.unsaved {
                            ending.keep(folder.as_deref());
                        }
                    });
                    true
                }
                Heard::Ended(false) => {
                    let _ = proxy.send_event(UserEvent::SessionGoesOn);
                    true
                }
            }
        });
        if let Err(error) = listened {
            tracing::warn!(%error, "won't hear Windows ending the session: unsaved edits won't stop it");
        }
    }

    /// A window stopped Windows ending the session: the unsaved windows ask, in turn.
    fn session_stopped(&mut self) {
        if self.round.start() {
            self.ask_next_unsaved();
        }
    }

    /// The oldest unsaved window comes forward and asks its close question, while the round of
    /// them is on.
    fn ask_next_unsaved(&mut self) {
        let mut unsaved: Vec<(u64, WindowId)> = self
            .windows
            .iter()
            .filter(|(_, doc)| doc.unsaved())
            .map(|(&id, doc)| (doc.opened, id))
            .collect();
        unsaved.sort();
        let order: Vec<WindowId> = unsaved.into_iter().map(|(_, id)| id).collect();
        if let Some(id) = self.round.next(&order) {
            self.focus(id);
            self.ask_close(id);
        }
    }

    /// Windows' session goes on after all: nothing more is asked, the copies kept go (the text
    /// is still in its window), and no window stops an end any more.
    fn session_goes_on(&mut self) {
        self.round.stop();
        for doc in self.windows.values() {
            if let Ok(mut ending) = doc.ending.try_borrow_mut() {
                ending.forget(doc.window.hwnd());
            }
        }
    }

    /// Brings what each window's listener knows up to date: whether anything is unsaved, and
    /// once it isn't, its copy and its reason go. A copy kept while the text changes (the
    /// round of questions under way) holds the newest.
    fn share_ending(&mut self) {
        for doc in self.windows.values() {
            let unsaved = doc.unsaved();
            let Ok(mut ending) = doc.ending.try_borrow_mut() else {
                continue;
            };
            ending.unsaved = unsaved;
            ending.base = doc.disk.as_ref().map(|d| d.stamp.hash);
            if !unsaved {
                ending.forget(doc.window.hwnd());
            } else if ending.kept.is_some() {
                ending.keep(self.unsaved_folder.as_deref());
            }
        }
    }
}

/// The monitors a new window may go on (their work areas and scaling), and which is the
/// primary.
fn monitors(target: &EventLoopWindowTarget<UserEvent>) -> (Vec<placement::Monitor>, Option<usize>) {
    let primary = target.primary_monitor().map(|m| m.hmonitor());
    let mut monitors = Vec::new();
    let mut primary_at = None;
    for monitor in target.available_monitors() {
        let Some((x, y, width, height)) = win32::monitor_work_area(monitor.hmonitor()) else {
            continue;
        };
        if Some(monitor.hmonitor()) == primary {
            primary_at = Some(monitors.len());
        }
        monitors.push(placement::Monitor {
            work: placement::Rect {
                x,
                y,
                width,
                height,
            },
            scale: monitor.scale_factor(),
        });
    }
    (monitors, primary_at)
}

/// The document at `file`, read and rendered for a window opening on it, off the UI thread; or
/// why it couldn't be read.
fn read_and_render(file: &Path, root: Option<&DocRoot>) -> Result<Opened, String> {
    let disk = Disk::read(file).map_err(|error| {
        tracing::warn!(%error, file = %file.display(), "couldn't read a document");
        reason::of(&error)
    })?;
    let folder = file.parent().unwrap_or(file);
    let html = render(disk.content.text(), folder).html;
    Ok(Opened {
        pictures: pictures(root, &html),
        editable_file: editor::file_is_editable(file),
        on_disk: OnDisk { disk, html },
    })
}

/// Where the pictures in rendered `html` would be in the document's folder, present or not.
fn pictures(root: Option<&DocRoot>, html: &str) -> Vec<PathBuf> {
    let Some(root) = root else {
        return Vec::new();
    };
    page::picture_paths(html)
        .iter()
        .filter_map(|path| root.candidate(path))
        .collect()
}

/// A window's title for its document `file` (None: it has none, and its page says so itself:
/// [`page::empty_page`]).
fn title_of(file: Option<&Path>) -> String {
    match file {
        Some(file) => {
            let name = file.file_name().unwrap_or(file.as_os_str());
            tr!("window-title", file = name.to_string_lossy().into_owned())
        }
        None => tr("app-name"),
    }
}

/// Whether `url` is Legible's own page, the only thing a window may show.
/// The navigation handler of a window's WebView: only Legible's own page, and never while the
/// editor has unsaved edits ([`editor::NavigationGuard`]), whoever asked (the window serving
/// its page again, a reload key, the page itself). `report` is told what each was.
pub fn navigation_policy(
    guard: editor::NavigationGuard,
    report: impl Fn(editor::Navigation) + 'static,
) -> impl Fn(String) -> bool + 'static {
    move |url| {
        let navigation = guard.allows(is_app_page(&url));
        match navigation {
            editor::Navigation::Foreign => tracing::warn!(url, "refused a navigation"),
            editor::Navigation::Unsaved => {
                tracing::info!(url, "refused a navigation: the editor has unsaved edits")
            }
            editor::Navigation::Allowed => {}
        }
        report(navigation);
        navigation == editor::Navigation::Allowed
    }
}

fn is_app_page(url: &str) -> bool {
    url.strip_prefix(page::APP_ORIGIN)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether `url` is the window's page itself (`/`), not another file of Legible's origin
/// such as Mermaid's frame.
fn is_the_page(url: &str) -> bool {
    url.strip_prefix(page::APP_ORIGIN)
        .is_some_and(|rest| matches!(rest.split(['#', '?']).next(), Some("/")))
}

/// The URL of the window's page `n`: its page (`/`), numbered in the query, so the window can
/// tell which page a message is from.
fn page_url(n: u64) -> String {
    format!("{}?page={n}", page::START_URL)
}

/// The number of the window's page at `url` ([`page_url`]).
fn page_of(url: &str) -> Option<u64> {
    let query = url.split('#').next()?.split_once('?')?.1;
    query.strip_prefix("page=")?.parse().ok()
}

/// Opens a URL with the user's own app for it.
fn open_outside(url: &str) {
    if let Err(error) = win32::shell_open(url) {
        tracing::warn!(%error, url, "couldn't open a link");
    }
}

type Body = Cow<'static, [u8]>;

/// What a window's protocol serves at `/` from now on: `html`, with its CSP (`origins` letting
/// pictures in) as a header too.
fn serve(served: &Mutex<Served>, html: String, origins: &[String]) {
    let mut served = served
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    served.html = html.into_bytes();
    served.csp = format!("{}{}", page::csp(origins), page::CSP_HEADER_EXTRA);
}

/// Legible's own origin: the page at `/`, the embedded assets elsewhere.
fn serve_app(request: &Request<Vec<u8>>, served: &Served) -> Response<Body> {
    let path = request.uri().path();
    if path == "/" {
        return Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header("Content-Security-Policy", served.csp.as_str())
            .header("X-Content-Type-Options", "nosniff")
            .body(Cow::Owned(served.html.clone()))
            .unwrap_or_else(|_| not_found());
    }
    let Some(asset) = assets::get(path) else {
        return not_found();
    };
    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, asset.content_type)
        .header("X-Content-Type-Options", "nosniff");
    if path == page::MERMAID_FRAME {
        response = response.header("Content-Security-Policy", page::MERMAID_FRAME_CSP);
    }
    response
        .body(Cow::Borrowed(asset.bytes))
        .unwrap_or_else(|_| not_found())
}

/// The document's origin: its images, from inside its folder only.
fn serve_document(request: &Request<Vec<u8>>, root: Option<&DocRoot>) -> Response<Body> {
    let Some(root) = root else {
        return not_found();
    };
    let found = root
        .resolve(crate::confine::without_generation(request.uri().path()))
        .map_err(|why| why.to_string())
        .and_then(|(file, kind)| {
            fs::read(&file)
                .map(|b| (b, kind))
                .map_err(|e| e.to_string())
        });
    match found {
        Ok((bytes, content_type)) => Response::builder()
            .header(header::CONTENT_TYPE, content_type)
            .header("X-Content-Type-Options", "nosniff")
            // Never kept: the window's next document may have a picture at the same path.
            .header(header::CACHE_CONTROL, "no-store")
            // An SVG opened by itself runs no script and loads nothing.
            .header(
                "Content-Security-Policy",
                "default-src 'none'; style-src 'unsafe-inline'",
            )
            .body(Cow::Owned(bytes))
            .unwrap_or_else(|_| not_found()),
        Err(why) => {
            tracing::debug!(path = request.uri().path(), why, "refused a document file");
            not_found()
        }
    }
}

fn not_found() -> Response<Body> {
    let mut response = Response::new(Cow::Borrowed(&b""[..]));
    *response.status_mut() = StatusCode::NOT_FOUND;
    response
}

/// The exe's app icon at `size` px scaled by `scale`, for a window's title bar (16) or the
/// taskbar and Alt+Tab (32). None where the exe has no icon (a test exe), and the window keeps
/// the default.
fn app_icon(size: u32, scale: f64) -> Option<Icon> {
    let px = icon_px(size, scale);
    Icon::from_resource(APP_ICON, Some(PhysicalSize::new(px, px)))
        .inspect_err(|error| tracing::debug!(%error, "no app icon in the exe"))
        .ok()
}

/// An icon's size in pixels: `size` logical pixels at `scale`, at least 1.
fn icon_px(size: u32, scale: f64) -> u32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    ((f64::from(size) * scale).round() as u32).max(1)
}

/// Gives `window` its title-bar and taskbar icons for the monitor scale `scale`. An exe
/// without the icon (a test exe) leaves the window's icons as they are. Public for legible's
/// icon-dpi example, which checks it in a real window.
pub fn set_app_icons(window: &Window, scale: f64) {
    if let Some(icon) = app_icon(16, scale) {
        window.set_window_icon(Some(icon));
    }
    if let Some(icon) = app_icon(32, scale) {
        window.set_taskbar_icon(Some(icon));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(uri: &str) -> Request<Vec<u8>> {
        Request::builder().uri(uri).body(Vec::new()).unwrap()
    }

    #[test]
    fn the_last_window_closing_exits_only_once_owned_work_is_settled_or_overdue() {
        // Either close path, a window still open: Legible runs on.
        assert_eq!(shutdown(true, true, false), Shutdown::Run);
        assert_eq!(shutdown(true, false, true), Shutdown::Run);
        // The last one closed during a commit, a print or a cleanup: it waits...
        assert_eq!(shutdown(false, true, false), Shutdown::Wait);
        // ...until the work settles, or the wait is overdue.
        assert_eq!(shutdown(false, false, false), Shutdown::Exit);
        assert_eq!(shutdown(false, true, true), Shutdown::Exit);
    }

    #[test]
    fn the_page_is_served_with_the_policy_its_pictures_need() {
        let served = Mutex::new(Served::default());
        serve(
            &served,
            "<p>a</p>".into(),
            &["https://a.example".to_owned()],
        );
        serve(
            &served,
            "<p>b</p>".into(),
            &["https://b.example".to_owned()],
        );
        let served = served.into_inner().unwrap();
        let page = serve_app(&get("https://legible.localhost/"), &served);
        let csp = page.headers()["Content-Security-Policy"].to_str().unwrap();
        assert!(csp.contains("img-src https://legible-doc.localhost data: https://b.example;"));
        assert!(!csp.contains("a.example") && csp.ends_with("frame-ancestors 'none'"));
        assert_eq!(page.body().as_ref(), b"<p>b</p>");
    }

    #[test]
    fn icons_are_sized_for_the_windows_scale() {
        // The title bar's 16 px and the taskbar's 32 px at each of Windows' scalings.
        for (scale, small, big) in [
            (1.0, 16, 32),
            (1.25, 20, 40),
            (1.5, 24, 48),
            (1.75, 28, 56),
            (2.0, 32, 64),
            (2.25, 36, 72),
            (3.0, 48, 96),
        ] {
            assert_eq!(
                (icon_px(16, scale), icon_px(32, scale)),
                (small, big),
                "{scale}"
            );
        }
        // Nonsense from a monitor never gives a zero-sized or absurd icon.
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(icon_px(16, scale), 16, "{scale}");
        }
        assert_eq!(icon_px(16, 0.01), 1);
    }

    #[test]
    fn a_test_exe_without_the_icon_resource_has_none_to_set() {
        // Tests are exes without Legible's resources: no icon, and nothing to set.
        assert!(app_icon(16, 1.0).is_none() && app_icon(32, 1.5).is_none());
    }

    #[test]
    fn a_page_is_known_by_its_number() {
        assert_eq!(page_of(&page_url(7)), Some(7));
        assert_eq!(page_of(&format!("{}#part-2", page_url(12))), Some(12));
        assert!(is_the_page(&page_url(3)));
        assert_eq!(page_of(page::START_URL), None);
        assert_eq!(page_of(&format!("{}?page=x", page::START_URL)), None);
    }

    #[test]
    fn only_legibles_page_may_be_shown() {
        assert!(is_app_page("https://legible.localhost/"));
        assert!(is_app_page("https://legible.localhost/#notes"));
        assert!(!is_app_page("https://legible.localhost.example.com/"));
        assert!(!is_app_page("https://legible.localhostx/"));
        assert!(!is_app_page("http://legible.localhost/"));
        assert!(!is_app_page("https://legible-doc.localhost/a.png"));
        assert!(!is_app_page("https://example.com/"));
        assert!(!is_app_page("file:///C:/Windows/win.ini"));
        assert!(!is_app_page("about:blank"));
    }

    #[test]
    fn only_the_page_itself_talks_to_the_window() {
        assert!(is_the_page("https://legible.localhost/"));
        assert!(is_the_page("https://legible.localhost/#notes"));
        assert!(!is_the_page(
            "https://legible.localhost/doc/mermaid-frame.html"
        ));
        assert!(!is_the_page("https://legible.localhost"));
        assert!(!is_the_page("https://legible.localhost.example.com/"));
        assert!(!is_the_page("https://legible-doc.localhost/"));
        assert!(!is_the_page("null"));
    }

    #[test]
    fn the_app_origin_serves_the_page_and_assets_only() {
        let served = Mutex::new(Served::default());
        serve(&served, "<p>hi</p>".into(), &[]);
        let served = served.into_inner().unwrap();
        let page = serve_app(&get("https://legible.localhost/"), &served);
        assert_eq!(page.status(), StatusCode::OK);
        let csp = page.headers()["Content-Security-Policy"].to_str().unwrap();
        assert_eq!(csp, format!("{}{}", page::CSP, page::CSP_HEADER_EXTRA));
        assert_eq!(page.body().as_ref(), b"<p>hi</p>");
        let css = serve_app(&get("https://legible.localhost/chrome/window.css"), &served);
        assert_eq!(css.status(), StatusCode::OK);
        assert_eq!(
            css.headers()[header::CONTENT_TYPE],
            "text/css; charset=utf-8"
        );
        let js = serve_app(
            &get("https://legible.localhost/js/highlight.min.js"),
            &served,
        );
        assert_eq!(js.status(), StatusCode::OK);
        assert!(!js.headers().contains_key("Content-Security-Policy"));
        let frame = serve_app(
            &get("https://legible.localhost/doc/mermaid-frame.html"),
            &served,
        );
        assert_eq!(frame.status(), StatusCode::OK);
        assert_eq!(
            frame.headers()["Content-Security-Policy"],
            page::MERMAID_FRAME_CSP
        );
        for missing in ["/js/VERSIONS", "/../Cargo.toml", "/chrome/nothing.js"] {
            let response = serve_app(
                &get(&format!("https://legible.localhost{missing}")),
                &served,
            );
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{missing}");
        }
    }

    #[test]
    fn the_document_origin_serves_images_from_its_folder() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("doc")).unwrap();
        fs::write(temp.path().join("doc").join("a.png"), b"png").unwrap();
        fs::write(temp.path().join("secret.png"), b"no").unwrap();
        let root = DocRoot::new(&temp.path().join("doc")).unwrap();
        let ok = serve_document(&get("https://legible-doc.localhost/a.png"), Some(&root));
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(ok.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(ok.body().as_ref(), b"png");
        let out = serve_document(
            &get("https://legible-doc.localhost/%2e%2e/secret.png"),
            Some(&root),
        );
        assert_eq!(out.status(), StatusCode::NOT_FOUND);
        let none = serve_document(&get("https://legible-doc.localhost/a.png"), None);
        assert_eq!(none.status(), StatusCode::NOT_FOUND);
    }
}
