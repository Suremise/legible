//! What the ⋯ menu does with the file a window shows: **Open in editor** and **Show in folder**.
//!
//! - **The editor** is `.md`'s, other than Legible: the `edit` verb's handler, else the `open`
//!   verb's (the user's default), else Notepad. A handler is Legible if it's this exe or any
//!   `legible.exe` (another copy, installed or portable). The handler is launched through the
//!   same `.md` association that passed that check ([`Handler::class`]), whatever the file's
//!   own suffix, so a `.markdown` file can't reach Legible through its own association. The
//!   file goes to Windows as a path, never in a command line Legible builds; Notepad gets it
//!   as its one argument.
//! - **Show in folder** opens Explorer with the file selected; if the file has gone, its folder.
//!
//! Both go through [`Platform`]: Windows' shell in the app (`win32::actions`), a recording fake
//! in tests, so no test launches anything. They run on [`Actions`], one thread for the process
//! with a short queue, since the shell can stall on a share: never on the UI thread, and never
//! a thread per click.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread;

use crate::jumplist;

/// How many requests may wait for the thread; more are dropped.
const QUEUE: usize = 4;

/// A verb of `.md`'s association.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Edit,
    Open,
}

impl Verb {
    pub fn as_str(self) -> &'static str {
        match self {
            Verb::Edit => "edit",
            Verb::Open => "open",
        }
    }
}

/// A handler of `.md`'s association for a verb: its program, for the check that it isn't
/// Legible, and the association itself, kept to launch it by.
pub struct Handler<C> {
    pub exe: PathBuf,
    pub class: C,
}

/// A launch, as it's handed to Windows.
#[derive(Debug, PartialEq, Eq)]
pub enum Launch<'a, C> {
    /// `file` with `verb` of the association `class` (`.md`'s).
    Verb {
        verb: Verb,
        class: &'a C,
        file: &'a Path,
    },
    /// Notepad, with `file` as its one argument.
    Notepad { exe: PathBuf, file: &'a Path },
}

/// Windows' shell, or a test's.
pub trait Platform {
    /// An association, kept open for a launch.
    type Class;
    /// `.md`'s handler for `verb`, if it has one. Always `.md`'s: there's no asking about
    /// another suffix.
    fn md_handler(&self, verb: Verb) -> Option<Handler<Self::Class>>;
    fn launch(&self, launch: &Launch<'_, Self::Class>) -> io::Result<()>;
    /// Notepad, in Windows' own folder.
    fn notepad(&self) -> io::Result<PathBuf>;
    fn exists(&self, path: &Path) -> bool;
    /// Opens the file's folder with the file selected.
    fn show_item(&self, file: &Path) -> io::Result<()>;
    /// Opens a folder.
    fn open_folder(&self, folder: &Path) -> io::Result<()>;
}

/// Whether `exe` is Legible: this copy, or any other.
pub fn is_legible(exe: &Path, me: &Path) -> bool {
    jumplist::same_file(exe, me)
        || exe
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("legible.exe"))
}

/// Opens `file` in `.md`'s editor (see the module's notes); a handler that fails to start gives
/// way to the next, and Notepad is the last.
pub fn open_in_editor<P: Platform>(platform: &P, me: &Path, file: &Path) -> io::Result<()> {
    let file = &shell_path(file);
    for verb in [Verb::Edit, Verb::Open] {
        let Some(handler) = platform.md_handler(verb) else {
            continue;
        };
        if is_legible(&handler.exe, me) {
            continue;
        }
        let launch = Launch::Verb {
            verb,
            class: &handler.class,
            file,
        };
        match platform.launch(&launch) {
            Ok(()) => return Ok(()),
            Err(error) => tracing::warn!(%error, verb = verb.as_str(), "the editor didn't start"),
        }
    }
    let exe = platform.notepad()?;
    platform.launch(&Launch::Notepad { exe, file })
}

/// `path` as the shell and other apps take it. The shell's canonical paths are verbatim
/// (`\\?\C:\…`, `\\?\UNC\…`), which the shell's own parsing (an item ID list, a verb's `%1`)
/// and many editors refuse, so the prefix comes off. A path longer than the old 260-character
/// limit may still be refused then, by the shell or an app that doesn't take long paths.
pub fn shell_path(path: &Path) -> PathBuf {
    jumplist::display_path(path)
}

/// Notepad's command for `file`: the path as its one argument, which std quotes by
/// `CommandLineToArgvW`'s rules; nothing is put through a shell.
pub fn notepad_command(exe: &Path, file: &Path) -> Command {
    let mut command = Command::new(exe);
    command.arg(file);
    command
}

/// What Show in folder opens.
#[derive(Debug, PartialEq, Eq)]
pub enum FolderTarget {
    /// The file, selected in its folder.
    Select(PathBuf),
    /// The folder alone: the file has gone.
    Folder(PathBuf),
}

/// What Show in folder opens for `file`: the file if it's there, else its folder if that is,
/// else nothing.
pub fn folder_target(file: &Path, exists: impl Fn(&Path) -> bool) -> Option<FolderTarget> {
    if exists(file) {
        return Some(FolderTarget::Select(file.to_owned()));
    }
    let folder = file.parent().filter(|f| !f.as_os_str().is_empty())?;
    exists(folder).then(|| FolderTarget::Folder(folder.to_owned()))
}

/// Shows `file` in its folder (see [`folder_target`]).
pub fn show_in_folder<P: Platform>(platform: &P, file: &Path) -> io::Result<()> {
    match folder_target(&shell_path(file), |p| platform.exists(p)) {
        Some(FolderTarget::Select(file)) => platform.show_item(&file),
        Some(FolderTarget::Folder(folder)) => platform.open_folder(&folder),
        None => Err(io::Error::from(io::ErrorKind::NotFound)),
    }
}

/// What a window asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    OpenInEditor,
    ShowInFolder,
}

/// The thread the actions run on, with its queue. It ends when this is dropped.
pub struct Actions<W> {
    sender: SyncSender<(W, Action, PathBuf)>,
}

impl<W: Send + 'static> Actions<W> {
    /// Starts the thread. `platform` makes its [`Platform`] there (COM is set up per thread);
    /// `failed` hears of each action that didn't work, with the window that asked.
    pub fn new<P: Platform>(
        platform: impl FnOnce() -> io::Result<P> + Send + 'static,
        me: PathBuf,
        failed: impl Fn(W, Action) + Send + 'static,
    ) -> Actions<W> {
        let (sender, requests) = mpsc::sync_channel::<(W, Action, PathBuf)>(QUEUE);
        let started = thread::Builder::new()
            .name("actions".into())
            .spawn(move || {
                let platform = match platform() {
                    Ok(platform) => Some(platform),
                    Err(error) => {
                        tracing::warn!(%error, "the shell actions can't start");
                        None
                    }
                };
                for (window, action, file) in requests {
                    let done = match &platform {
                        Some(p) => match action {
                            Action::OpenInEditor => open_in_editor(p, &me, &file),
                            Action::ShowInFolder => show_in_folder(p, &file),
                        },
                        None => Err(io::Error::other("the shell actions didn't start")),
                    };
                    if let Err(error) = done {
                        tracing::warn!(%error, ?action, file = %file.display(), "an action failed");
                        failed(window, action);
                    }
                }
            });
        if let Err(error) = started {
            tracing::warn!(%error, "couldn't start the shell actions' thread");
        }
        Actions { sender }
    }

    /// Asks for `action` on `file`; says whether it was taken (not if the queue is full).
    pub fn request(&self, window: W, action: Action, file: PathBuf) -> bool {
        match self.sender.try_send((window, action, file)) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                tracing::warn!(?action, "the shell actions are busy: dropped");
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    use super::*;

    /// A launch as the fake recorded it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Recorded {
        Verb {
            verb: Verb,
            class: &'static str,
            file: PathBuf,
        },
        Notepad {
            exe: PathBuf,
            file: PathBuf,
        },
        Show(PathBuf),
        Folder(PathBuf),
    }

    /// `.md`'s association as the fake has it: a handler per verb, named by its class.
    #[derive(Default)]
    struct Fake {
        edit: Option<&'static str>,
        open: Option<&'static str>,
        /// The launches that fail: by verb, and Notepad.
        failing: Vec<&'static str>,
        present: HashSet<PathBuf>,
        launched: RefCell<Vec<Recorded>>,
    }

    const NOTEPAD: &str = r"C:\Windows\System32\notepad.exe";

    impl Fake {
        fn with(edit: Option<&'static str>, open: Option<&'static str>) -> Fake {
            Fake {
                edit,
                open,
                ..Fake::default()
            }
        }
        fn launched(&self) -> Vec<Recorded> {
            self.launched.borrow().clone()
        }
    }

    impl Platform for Fake {
        /// The handler's exe, standing for its class key.
        type Class = &'static str;
        fn md_handler(&self, verb: Verb) -> Option<Handler<&'static str>> {
            let exe = match verb {
                Verb::Edit => self.edit,
                Verb::Open => self.open,
            }?;
            Some(Handler {
                exe: PathBuf::from(exe),
                class: exe,
            })
        }
        fn launch(&self, launch: &Launch<'_, &'static str>) -> io::Result<()> {
            let (record, key) = match launch {
                Launch::Verb { verb, class, file } => (
                    Recorded::Verb {
                        verb: *verb,
                        class,
                        file: file.to_path_buf(),
                    },
                    verb.as_str(),
                ),
                Launch::Notepad { exe, file } => (
                    Recorded::Notepad {
                        exe: exe.clone(),
                        file: file.to_path_buf(),
                    },
                    "notepad",
                ),
            };
            self.launched.borrow_mut().push(record);
            if self.failing.contains(&key) {
                Err(io::Error::other("didn't start"))
            } else {
                Ok(())
            }
        }
        fn notepad(&self) -> io::Result<PathBuf> {
            Ok(PathBuf::from(NOTEPAD))
        }
        fn exists(&self, path: &Path) -> bool {
            self.present.contains(path)
        }
        fn show_item(&self, file: &Path) -> io::Result<()> {
            self.launched
                .borrow_mut()
                .push(Recorded::Show(file.to_owned()));
            Ok(())
        }
        fn open_folder(&self, folder: &Path) -> io::Result<()> {
            self.launched
                .borrow_mut()
                .push(Recorded::Folder(folder.to_owned()));
            Ok(())
        }
    }

    const ME: &str = r"C:\Tools\Legible\legible.exe";
    const CODE: &str = r"C:\Program Files\Microsoft VS Code\Code.exe";
    const TYPORA: &str = r"C:\Program Files\Typora\Typora.exe";

    fn verb(verb: Verb, class: &'static str, file: &Path) -> Recorded {
        Recorded::Verb {
            verb,
            class,
            file: file.to_owned(),
        }
    }
    fn notepad(file: &Path) -> Recorded {
        Recorded::Notepad {
            exe: PathBuf::from(NOTEPAD),
            file: file.to_owned(),
        }
    }

    #[test]
    fn the_edit_verb_comes_first_then_the_default_then_notepad() {
        let file = Path::new(r"C:\Notes\Plan.md");
        let fake = Fake::with(Some(CODE), Some(TYPORA));
        open_in_editor(&fake, Path::new(ME), file).unwrap();
        assert_eq!(fake.launched(), [verb(Verb::Edit, CODE, file)]);

        let fake = Fake::with(None, Some(TYPORA));
        open_in_editor(&fake, Path::new(ME), file).unwrap();
        assert_eq!(fake.launched(), [verb(Verb::Open, TYPORA, file)]);

        let fake = Fake::with(None, None);
        open_in_editor(&fake, Path::new(ME), file).unwrap();
        assert_eq!(fake.launched(), [notepad(file)]);
    }

    #[test]
    fn legible_is_never_the_editor() {
        let file = Path::new(r"C:\Notes\Plan.md");
        // This copy, named another way; and another copy of Legible altogether.
        for legible in [
            r"\\?\C:\TOOLS\legible\Legible.EXE",
            r"C:\Program Files\Legible\legible.exe",
        ] {
            assert!(is_legible(Path::new(legible), Path::new(ME)), "{legible}");
            let fake = Fake::with(Some(legible), Some(TYPORA));
            open_in_editor(&fake, Path::new(ME), file).unwrap();
            assert_eq!(fake.launched(), [verb(Verb::Open, TYPORA, file)]);
            let fake = Fake::with(Some(CODE), Some(legible));
            open_in_editor(&fake, Path::new(ME), file).unwrap();
            assert_eq!(fake.launched(), [verb(Verb::Edit, CODE, file)]);
            let fake = Fake::with(Some(legible), Some(legible));
            open_in_editor(&fake, Path::new(ME), file).unwrap();
            assert_eq!(fake.launched(), [notepad(file)]);
        }
        assert!(!is_legible(
            Path::new(r"C:\Tools\legible-notes.exe"),
            Path::new(ME)
        ));
    }

    #[test]
    fn every_suffix_opens_with_mds_own_handler() {
        // The fake knows `.md` alone: whatever the file is called, the launch carries `.md`'s
        // handler and the file's own path, unchanged.
        for name in [
            r"C:\Notes\Plan.markdown",
            r"C:\Notes\Old.MDOWN",
            r"C:\Notes\Plan.md",
            r"C:\Notes\Rough notes & 100% ^done!.md",
            r"C:\Notes\Ünïcödé 日本語\Plan.md",
            r"\\server\share\Team Notes\Plan.md",
        ] {
            let file = Path::new(name);
            let fake = Fake::with(Some(CODE), None);
            open_in_editor(&fake, Path::new(ME), file).unwrap();
            assert_eq!(fake.launched(), [verb(Verb::Edit, CODE, file)], "{name}");
        }
    }

    #[test]
    fn the_shell_gets_paths_without_the_verbatim_prefix() {
        let plain = Path::new(r"C:\Notes\Plan.md");
        let verbatim = Path::new(r"\\?\C:\Notes\Plan.md");
        let fake = Fake::with(Some(CODE), None);
        open_in_editor(&fake, Path::new(ME), verbatim).unwrap();
        assert_eq!(fake.launched(), [verb(Verb::Edit, CODE, plain)]);
        let fake = Fake::with(None, None);
        open_in_editor(
            &fake,
            Path::new(ME),
            Path::new(r"\\?\UNC\server\share\Plan.md"),
        )
        .unwrap();
        assert_eq!(
            fake.launched(),
            [notepad(Path::new(r"\\server\share\Plan.md"))]
        );

        let folder = PathBuf::from(r"C:\Notes");
        let fake = Fake {
            present: [plain.to_owned(), folder.clone()].into(),
            ..Fake::default()
        };
        show_in_folder(&fake, verbatim).unwrap();
        assert_eq!(fake.launched(), [Recorded::Show(plain.to_owned())]);
        let fake = Fake {
            present: [folder.clone()].into(),
            ..Fake::default()
        };
        show_in_folder(&fake, verbatim).unwrap();
        assert_eq!(fake.launched(), [Recorded::Folder(folder)]);
    }

    #[test]
    fn a_handler_that_fails_gives_way_and_notepad_is_last() {
        let file = Path::new(r"C:\Notes\Plan.md");
        let mut fake = Fake::with(Some(CODE), Some(TYPORA));
        fake.failing = vec!["edit"];
        open_in_editor(&fake, Path::new(ME), file).unwrap();
        assert_eq!(
            fake.launched(),
            [verb(Verb::Edit, CODE, file), verb(Verb::Open, TYPORA, file)]
        );

        let mut fake = Fake::with(Some(CODE), Some(TYPORA));
        fake.failing = vec!["edit", "open", "notepad"];
        assert!(open_in_editor(&fake, Path::new(ME), file).is_err());
        assert_eq!(
            fake.launched(),
            [
                verb(Verb::Edit, CODE, file),
                verb(Verb::Open, TYPORA, file),
                notepad(file)
            ]
        );
    }

    #[test]
    fn notepad_gets_the_path_as_its_one_argument() {
        // Quotes can't be in a Windows file name, but the quoting is checked with one anyway,
        // and with backslashes before it and at the end.
        for name in [
            r"C:\Notes\Plan.md",
            r"C:\My Notes\Rough notes & 100% ^done! (v2).md",
            r"C:\Notes\Ünïcödé 日本語.md",
            r"\\server\share\Team Notes\Plan.md",
            r"\\?\C:\a very long folder name that goes on\and on\Plan.md",
            r#"C:\odd\say "hi"\.md"#,
            r"C:\odd\ends in backslashes\\",
        ] {
            let command = notepad_command(Path::new(NOTEPAD), Path::new(name));
            assert_eq!(command.get_program(), NOTEPAD);
            let args: Vec<_> = command.get_args().collect();
            assert_eq!(args, [std::ffi::OsStr::new(name)], "{name}");
            // And it survives the quoting Windows' own parser undoes.
            let quoted = jumplist::quote(std::ffi::OsStr::new(name));
            assert_eq!(
                jumplist::unquote(&quoted).as_deref(),
                Some(std::ffi::OsStr::new(name))
            );
        }
    }

    #[test]
    fn show_in_folder_selects_the_file_or_opens_its_folder() {
        let file = PathBuf::from(r"C:\Notes\Plan.md");
        let folder = PathBuf::from(r"C:\Notes");
        let mut fake = Fake {
            present: [file.clone(), folder.clone()].into(),
            ..Fake::default()
        };
        show_in_folder(&fake, &file).unwrap();
        assert_eq!(fake.launched(), [Recorded::Show(file.clone())]);

        // The file has gone: its folder itself opens.
        fake.present = [folder.clone()].into();
        fake.launched.borrow_mut().clear();
        show_in_folder(&fake, &file).unwrap();
        assert_eq!(fake.launched(), [Recorded::Folder(folder)]);

        // Both gone: nothing opens.
        fake.present.clear();
        fake.launched.borrow_mut().clear();
        assert_eq!(
            show_in_folder(&fake, &file).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(fake.launched().is_empty());
        assert_eq!(folder_target(Path::new("Plan.md"), |_| false), None);
    }

    /// A platform for the thread whose launches wait at a gate, counting those started.
    struct Gated {
        gate: Arc<(Mutex<bool>, Condvar)>,
        started: Arc<AtomicUsize>,
    }

    impl Platform for Gated {
        type Class = ();
        fn md_handler(&self, _: Verb) -> Option<Handler<()>> {
            None
        }
        fn launch(&self, _: &Launch<'_, ()>) -> io::Result<()> {
            self.started.fetch_add(1, Ordering::SeqCst);
            let (open, opened) = &*self.gate;
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut is_open = open.lock().unwrap();
            while !*is_open && Instant::now() < deadline {
                is_open = opened
                    .wait_timeout(is_open, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
            Err(io::Error::other("no editor here"))
        }
        fn notepad(&self) -> io::Result<PathBuf> {
            Ok(PathBuf::from(NOTEPAD))
        }
        fn exists(&self, _: &Path) -> bool {
            false
        }
        fn show_item(&self, _: &Path) -> io::Result<()> {
            Ok(())
        }
        fn open_folder(&self, _: &Path) -> io::Result<()> {
            Ok(())
        }
    }

    /// Opens the gate when dropped, so a failing test can't leave the thread waiting.
    struct OpenOnDrop(Arc<(Mutex<bool>, Condvar)>);
    impl Drop for OpenOnDrop {
        fn drop(&mut self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }

    fn until(what: &str, check: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_stuck_action_holds_a_short_queue_and_failures_are_reported() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let started = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(Mutex::new(Vec::new()));
        let actions = {
            let (gate, started, failed) = (gate.clone(), started.clone(), failed.clone());
            Actions::new(
                move || Ok(Gated { gate, started }),
                PathBuf::from(ME),
                move |window: u32, action| failed.lock().unwrap().push((window, action)),
            )
        };
        let release = OpenOnDrop(gate.clone());
        let file = PathBuf::from(r"C:\Notes\Plan.md");
        // The first is taken by the thread and sticks at the gate; then the queue fills, and
        // what comes after is dropped, not queued without end.
        assert!(actions.request(0, Action::OpenInEditor, file.clone()));
        until("the first launch to start", || {
            started.load(Ordering::SeqCst) == 1
        });
        let taken = (1..=20)
            .filter(|&w| actions.request(w, Action::OpenInEditor, file.clone()))
            .count();
        assert_eq!(
            taken, QUEUE,
            "only the queue's worth is taken while one is stuck"
        );
        assert_eq!(started.load(Ordering::SeqCst), 1);
        drop(release);
        // Each taken request fails (the fake has no editor), and its window hears of it.
        until("every taken request to be reported", || {
            failed.lock().unwrap().len() == taken + 1
        });
        assert_eq!(failed.lock().unwrap()[0], (0, Action::OpenInEditor));
        // Show in folder of a file that's gone, folder and all: reported too.
        assert!(actions.request(99, Action::ShowInFolder, file));
        until("the folder's failure", || {
            failed.lock().unwrap().last() == Some(&(99, Action::ShowInFolder))
        });
    }
}
