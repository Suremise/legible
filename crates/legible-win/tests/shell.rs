//! The real window (`shell`), driven end to end in this process by `shell::harness`, on files in
//! a temp folder of the run's own, with Legible's own folders (WebView2's data among them) in it
//! too. The shell has no app identity, no Jump List and no single-instance pipe, and writes to a
//! clipboard of the harness's own. Its windows are quiet: off every monitor (unless a case
//! places one), and never active or focused, which is checked after every case, so the user
//! keeps the focus wherever they had it. Each window is driven by script in its own page only,
//! never by input.
//!
//! The cases are those of the window's own wiring that need the real window: a save, then Copy or a
//! pictures answer straight away; a reload that waited for a save; Use the other version for a
//! raced save (the code's "take theirs": completed, raced by a third writer, and written over
//! before it's taken up); Keep my changes over a change a save found; a close during a save;
//! Save as, moving the window to the new file (and over a destination that changes once it's
//! chosen); and Open, Open recent and a drop replacing a window's document (unsaved changes asked about as a close asks them), while a
//! launch opens a window of its own.
//!
//! The documents are in the temp folder, where Legible won't edit a file (an attachment opens
//! from there), so the shell runs in a copy of this test of its own whose temp folder (`TMP`,
//! `TEMP`) is another folder of the run's: WebView2's temp files go there too.
#![cfg(all(windows, feature = "harness"))]

use std::fs;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use legible_core::i18n::tr;
use legible_core::save;
use legible_core::unsaved::{self, Kept};
use legible_win::menu::{self, Mode};
use legible_win::paths::AppPaths;
use legible_win::quiet;
use legible_win::settings::json_string;
use legible_win::shell::harness::{self, BOUND, Driver, Moment, Result, SaveAnswer, Window, Write};
use legible_win::win32;

/// The editor's text, in the page.
const TEXT: &str = "LegibleEditor.view().state.doc.toString()";

/// Port 9 (discard) is one the browser refuses to connect to: a picture there is offered and
/// let in like any other, and no request ever leaves the PC.
const PICTURE: &str = "https://127.0.0.1:9/p.png";

type Case = fn(&Driver, &Path) -> Result<()>;

const CASES: &[(&str, Case)] = &[
    (
        "a window opens quietly: off every monitor, and never active or focused",
        opens_quietly,
    ),
    (
        "the focus check hears a window that could take the focus, though it's gone by the check",
        focus_check_hears_between_checks,
    ),
    (
        "Open and Save as PDF show no dialog in a quiet run: each is taken as cancelled",
        dialogs_stay_unshown,
    ),
    (
        "a save, then Copy straight away: the copy is taken",
        save_then_copy,
    ),
    (
        "a save, then a pictures answer straight away: it's accepted",
        save_then_pictures_answer,
    ),
    (
        "a reload that waited while dirty: once saved, the page served again is the saved text's",
        reload_waits_for_the_save,
    ),
    (
        "Take theirs for a raced save: theirs is the file and the text, and the next save goes",
        take_theirs,
    ),
    (
        "Take theirs raced by a third writer: theirs goes in, and the third's is kept and asked about",
        take_theirs_raced_again,
    ),
    (
        "a write between Take theirs and its end: nothing is taken as clean, and the change is held",
        take_theirs_written_over,
    ),
    (
        "a change a save found, then Keep mine: saved over it",
        keep_mine_over_a_change,
    ),
    (
        "a close during a save: it waits, and the frozen page's answer lets no late edit in",
        close_during_a_save,
    ),
    (
        "Save as over a destination that changes once chosen: left alone, said, and the window stays as it was",
        save_as_over_a_changed_destination,
    ),
    (
        "Save as moves the window to the new file: its title, recent files and live reload, the edits saved there and the old file left as it was",
        save_as_moves_the_window,
    ),
    (
        "Save as from a file that can't be saved where it is (Ctrl+S offers it): the new file can be edited",
        save_as_makes_a_read_only_file_editable,
    ),
    (
        "Open and a close during Save as: Open refused, the close waits (nothing asked, the page frozen, no late edit lost), then it closes",
        close_waits_for_save_as,
    ),
    (
        "the status bar: the file's path, encoding and line endings (as a save leaves them), and Copy path copies the file's own path",
        status_bar_and_copy_path,
    ),
    (
        "About: the window's own facts, Copy puts them on the clipboard, and Release notes shows the changelog",
        about_facts_copy_and_release_notes,
    ),
    (
        "a sign-out with unsaved edits: a copy is kept, the end stopped, and the window asks",
        sign_out_stops_and_asks,
    ),
    (
        "a sign-out with nothing unsaved goes at once",
        sign_out_with_nothing_unsaved,
    ),
    (
        "a sign-out's question cancelled, then the session goes on: the copy goes",
        sign_out_cancelled,
    ),
    (
        "a sign-out with two unsaved windows: they ask in turn, the oldest first",
        sign_out_asks_each_in_turn,
    ),
    (
        "a sign-out that can't be stopped: the copy is kept, and nothing asks",
        sign_out_critical,
    ),
    (
        "a copy from an earlier session: its window opens with the bar, and Restore puts it in, unsaved",
        kept_restored,
    ),
    (
        "a copy of a file changed since: the bar says so, and Discard drops it",
        kept_discarded,
    ),
    (
        "a copy's window closed without a choice: it's offered again",
        kept_until_chosen,
    ),
    (
        "a copy of a file that has gone: saved as a new file, which opens",
        kept_elsewhere,
    ),
    (
        "a copy its file already holds: nothing is offered, and it goes",
        kept_already_there,
    ),
    (
        "an end before the preview catches up keeps the newest edit, and a first edit",
        sign_out_before_the_preview,
    ),
    (
        "an unchosen copy from an earlier run outlives a later end, and is offered again",
        sign_out_keeps_an_unchosen_copy,
    ),
    (
        "the first window asked keeps every unsaved window's copy, as Windows may ask no other",
        sign_out_protects_every_window,
    ),
    (
        "a file opens again where the reader was, by its heading, and turning it off forgets",
        reading_place_kept,
    ),
    (
        "a new window opens where the last one was, and turned off, at the first size",
        windows_reopen_where_they_were,
    ),
    (
        "an edit while a clean window closes: one after the page froze doesn't land, one before asks",
        close_keeps_a_late_edit,
    ),
    (
        "a file read once its window is made: a recent file once read, and one that can't be read closes the window and says why",
        read_once_the_window_is_made,
    ),
    (
        "a window still reading its file: only Open, Close and Exit of its file's commands, and Save as asks nothing till it's read",
        commands_wait_for_the_file,
    ),
    (
        "Open replaces a clean document: its title, pictures, recent files and live reload are the new file's",
        open_replaces_a_clean_document,
    ),
    (
        "Open over unsaved changes asks as a close does: Cancel keeps them, Don't save drops them, Save saves them, and the mode stays",
        open_over_unsaved_changes,
    ),
    (
        "Open over unsaved changes kept as Windows ends the session: the copy goes once they're dropped",
        open_drops_the_copy_kept,
    ),
    (
        "Open keeps where the reader was in the old file, and opens the new one where they were in it",
        open_keeps_the_reading_place,
    ),
    (
        "a drop replaces the document with its first file, and the others open in windows of their own",
        drop_replaces_and_opens_the_rest,
    ),
    (
        "Open of a file open in another window leaves both as they are, and a launch opens a window of its own",
        open_elsewhere_and_launch,
    ),
    (
        "a launch with no file opens an empty window of its own",
        launch_without_a_file,
    ),
    (
        "Open over unsaved changes whose save finds the file changed: the document stays, unsaved and protected, and a later close closes",
        open_over_a_save_that_conflicts,
    ),
    (
        "what the replaced document's page says while the new file is read reaches nothing: pictures consent or edits",
        replaced_page_says_nothing,
    ),
    (
        "the same file opened again while an earlier read of it is late: the late read is out of date",
        an_earlier_read_is_out_of_date,
    ),
];

/// Set (to the run's folder) only in the copy that runs the shell.
const CHILD: &str = "LEGIBLE_SHELL_TEST_RUN";

/// How long the shell's run may take, every case's waits included, before it's taken as hung.
const RUN_BOUND: Duration = Duration::from_secs(240);

#[test]
fn the_real_window_saves_as_takes_theirs_and_closes() {
    let folders = Folders::new();
    let me = std::env::current_exe().expect("this test's exe");
    let mut child = Command::new(me)
        .args(["--exact", "shell_run", "--nocapture", "--test-threads=1"])
        .env(CHILD, &folders.root)
        .env(quiet::VAR, "1")
        .env("TMP", &folders.tmp)
        .env("TEMP", &folders.tmp)
        .spawn()
        .expect("run the shell's copy of this test");
    let end = Instant::now() + RUN_BOUND;
    let status = loop {
        if let Some(status) = child.try_wait().expect("the shell's run") {
            break Some(status);
        }
        if Instant::now() > end {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    drop(folders);
    match status {
        Some(status) => assert!(
            status.success(),
            "the shell's run failed (its output is above)"
        ),
        None => panic!("the shell's run hung"),
    }
}

/// Run only as the copy, in the run's folder: the shell, driven through every case.
#[test]
fn shell_run() {
    let Some(root) = std::env::var_os(CHILD).map(PathBuf::from) else {
        return;
    };
    legible_core::i18n::init("en");
    let docs = root.join("docs");
    let paths = AppPaths::resolve("Legible", &root.join("app"), Some(&root.join("appdata")))
        .expect("the paths");
    assert!(paths.root().starts_with(&root));
    let failures = harness::run(paths, "en", move |d| {
        let mut failures = Vec::new();
        for (what, case) in CASES {
            let started = Instant::now();
            // However the case went, the user keeps the focus.
            let outcome = case(d, &docs).and(d.focus_kept());
            match outcome {
                Ok(()) => println!("  ok    {what} ({} ms)", started.elapsed().as_millis()),
                Err(why) => {
                    println!("  FAIL  {what}: {why}");
                    failures.push(format!("{what}: {why}"));
                }
            }
        }
        failures
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The run's folder, in the temp folder: the documents, Legible's folders (`app`, `appdata`),
/// and the shell's own temp folder, removed once the run is over, passed or failed. WebView2
/// lets go of its data a moment after the shell exits.
struct Folders {
    temp: Option<tempfile::TempDir>,
    root: PathBuf,
    tmp: PathBuf,
}

impl Folders {
    fn new() -> Folders {
        let temp = tempfile::tempdir().expect("a temp folder");
        let root = temp.path().to_owned();
        for dir in ["docs", "app", "appdata", "tmp"] {
            fs::create_dir_all(root.join(dir)).expect("a folder in the temp folder");
        }
        Folders {
            temp: Some(temp),
            tmp: root.join("tmp"),
            root,
        }
    }
}

impl Drop for Folders {
    fn drop(&mut self) {
        let Some(temp) = self.temp.take() else {
            return;
        };
        let path = temp.keep();
        let end = Instant::now() + BOUND;
        while fs::remove_dir_all(&path).is_err() && path.exists() {
            if Instant::now() > end {
                eprintln!("the temp folder couldn't be removed: {}", path.display());
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

// --- The cases ---------------------------------------------------------------------------------

fn save_then_copy(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "copy", b"# Copy\n\nThe text.\n")?;
    let w = open_editing(d, &file)?;
    type_at_start(d, w, "Saved. ")?;
    d.until(
        w,
        "the edit in the preview",
        "LegibleEditor.isDirty() && LegibleEditor.previewState().current \
         && document.getElementById('document').textContent.includes('Saved.')",
    )?;
    press(d, w, "s", false)?;
    file_is(d, &file, b"Saved. # Copy\n\nThe text.\n")?;
    d.until(w, "the text to be saved", "!LegibleEditor.isDirty()")?;
    // Straight away, nothing typed in between.
    press(d, w, "C", true)?;
    d.until_true("the copy on the clipboard", || {
        Ok(d.clipboard().iter().any(|text| text.contains("Saved.")))
    })?;
    d.until(
        w,
        "the page to say it's copied",
        &format!(
            "!document.getElementById('toast').hidden \
             && document.getElementById('toast').textContent.includes({})",
            json_string(&tr("chrome-copied"))
        ),
    )?;
    let page = d.eval(w, "document.getElementById('document').dataset.generation")?;
    let window = d.generation(w)?;
    if page != window.map(|g| format!("\"{g}\"")) {
        return Err(format!(
            "the page's render is {page:?}, the window's {window:?}"
        ));
    }
    close_clean(d, w)
}

fn about_facts_copy_and_release_notes(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "about", b"# About\n")?;
    let w = d.open(&file)?;
    d.until(w, "the chrome to be ready", "!!window.LegibleChrome?.ready")?;
    d.eval(w, "LegibleChrome.command('about')")?;
    // The table is the window's own facts: Cargo's version, how this copy runs, WebView2's
    // runtime and Windows, as they are.
    let windows = win32::registry::windows_version()
        .map(|v| v.text())
        .unwrap_or_else(|| tr("about-unknown"));
    let webview2 = legible_win::shell::webview2_version().unwrap_or_else(|| tr("about-unknown"));
    let rows = |build: &str| {
        format!(
            "Version{}|Build{build}|WebView2{webview2}|Windows{windows}",
            env!("CARGO_PKG_VERSION")
        )
    };
    let (installed, portable) = (rows("Installed"), rows("Portable"));
    d.until(
        w,
        "About's table",
        &format!(
            "(() => {{ const rows = [...document.querySelectorAll('.about-table tr')].map((r) => r.textContent).join('|'); \
             return rows === {} || rows === {}; }})()",
            json_string(&installed),
            json_string(&portable)
        ),
    )?;
    // Copy: the window's facts on the clipboard (its own, as text), and About says so.
    let before = d.clipboard().len();
    d.eval(
        w,
        "[...document.querySelectorAll('.about-button')].find((b) => b.textContent === 'Copy').click()",
    )?;
    let first = format!("Legible {}\r\nBuild: ", env!("CARGO_PKG_VERSION"));
    let last_line = format!("Windows: {windows}\r\n");
    d.until_true("the version information on the clipboard", || {
        Ok(d.clipboard().len() > before
            && d.clipboard()
                .last()
                .is_some_and(|t| t.starts_with(&first) && t.ends_with(&last_line)))
    })?;
    d.until(
        w,
        "the page to say it's copied",
        &format!(
            "document.getElementById('toast').textContent.includes({})",
            json_string(&tr("chrome-about-copied"))
        ),
    )?;
    // Release notes: CHANGELOG.md, bundled, rendered as a document.
    d.eval(
        w,
        "[...document.querySelectorAll('.about-button')].find((b) => b.textContent === 'Release notes').click()",
    )?;
    d.until(
        w,
        "the release notes",
        "document.querySelector('.reading-doc h1')?.textContent === 'Changelog' \
         && !!document.querySelector('.reading-doc a[href=\"https://keepachangelog.com/en/1.1.0/\"]')",
    )?;
    d.eval(
        w,
        "document.querySelector('dialog.reading').close(); document.querySelector('dialog.about').close()",
    )?;
    close_clean(d, w)
}

fn status_bar_and_copy_path(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "status", b"# Status\r\n\r\nSome words.\r\nMore\n")?;
    let w = d.open(&file)?;
    let canonical = fs::canonicalize(&file).map_err(|e| e.to_string())?;
    let path = legible_win::jumplist::display_path(&canonical)
        .to_string_lossy()
        .into_owned();
    // The page asks as it starts; the window tells it what's so of its own file.
    d.until(
        w,
        "the status bar to know the file",
        &format!(
            "document.querySelector('.status-path')?.title.startsWith({}) \
             && document.querySelector('.status-encoding').textContent === 'UTF-8' \
             && document.querySelector('.status-endings').textContent === {} \
             && document.querySelector('.status-note').hidden",
            json_string(&path),
            json_string(&legible_core::tr!("status-endings-mixed", kept = "CRLF"))
        ),
    )?;
    // Shown, and its path clicked: the window's own path goes on the clipboard (as text).
    d.eval(w, "LegibleChrome.command('status-bar')")?;
    let before = d.clipboard().len();
    d.eval(w, "document.querySelector('.status-path').click()")?;
    d.until_true("the path on the clipboard", || {
        Ok(d.clipboard().len() > before && d.clipboard().last() == Some(&path))
    })?;
    d.until(
        w,
        "the page to say it's copied",
        &format!(
            "!document.getElementById('toast').hidden \
             && document.getElementById('toast').textContent.includes({})",
            json_string(&tr("chrome-status-path-copied"))
        ),
    )?;
    // Saved from the editor, its line endings are all CRLF: the bar says so at once.
    d.eval(w, "LegibleEditor.setMode('split')")?;
    d.until(
        w,
        "the editor to open",
        "!!LegibleEditor.view() && LegibleEditor.previewState().current",
    )?;
    type_at_start(d, w, "Saved. ")?;
    d.until(w, "the edit to be unsaved", "LegibleEditor.isDirty()")?;
    press(d, w, "s", false)?;
    file_is(d, &file, b"Saved. # Status\r\n\r\nSome words.\r\nMore\r\n")?;
    d.until(
        w,
        "the bar to say CRLF",
        "!LegibleEditor.isDirty() && document.querySelector('.status-endings').textContent === 'CRLF'",
    )?;
    d.eval(w, "LegibleChrome.command('status-bar')")?;
    close_clean(d, w)
}

fn save_then_pictures_answer(d: &Driver, docs: &Path) -> Result<()> {
    let text = format!("# Pictures\n\n![p]({PICTURE})\n");
    let file = document(docs, "pictures", text.as_bytes())?;
    let w = open_editing(d, &file)?;
    d.until(w, "the pictures bar to ask", &pictures_bar_asks())?;
    type_at_start(d, w, "Saved. ")?;
    d.until(w, "the edit", "LegibleEditor.isDirty()")?;
    press(d, w, "s", false)?;
    file_is(d, &file, format!("Saved. {text}").as_bytes())?;
    d.until(w, "the text to be saved", "!LegibleEditor.isDirty()")?;
    // The bar asks again, for the saved text's render; answered straight away.
    d.until(w, "the pictures bar to ask again", &pictures_bar_asks())?;
    click(d, w, "#pictures-bar", &tr("chrome-pictures-once"))?;
    d.until(
        w,
        "the picture to be let in",
        &format!(
            "document.querySelector('img[data-remote-src]')?.getAttribute('src') === {}",
            json_string(PICTURE)
        ),
    )?;
    close_clean(d, w)
}

fn reload_waits_for_the_save(d: &Driver, docs: &Path) -> Result<()> {
    let text = format!("# Waits\n\n![p]({PICTURE})\n");
    let file = document(docs, "waits", text.as_bytes())?;
    let w = open_editing(d, &file)?;
    type_at_start(d, w, "Saved. ")?;
    d.until(w, "the edit", "LegibleEditor.isDirty()")?;
    // Settings' "Always show them": the page is to be served again, letting the picture in,
    // and that waits for the edit to be saved.
    set_remote_images(d, w, "always")?;
    d.until_true("the reload to wait", || d.reload_waits(w))?;
    press(d, w, "s", false)?;
    file_is(d, &file, format!("Saved. {text}").as_bytes())?;
    // The page served again shows the saved text itself, before its editor opens.
    d.until(
        w,
        "the page served again, with the saved text",
        "window.marked === undefined && !!window.LegibleChrome?.ready && !LegibleEditor.view() \
         && document.getElementById('document').textContent.includes('Saved.')",
    )?;
    if !d.served(w)?.is_some_and(|page| page.contains("Saved.")) {
        return Err("the window serves a page without the saved text".into());
    }
    set_remote_images(d, w, "ask")?;
    close_clean(d, w)
}

const OURS: &[u8] = b"# Race\n\nOurs.\n";
const MINE: &[u8] = b"Mine. # Race\n\nOurs.\n";
const THEIRS: &[u8] = b"# Theirs\n\nTheir text.\n";
const THIRD: &[u8] = b"# Third\n\nA third writer's.\n";

fn take_theirs(d: &Driver, docs: &Path) -> Result<()> {
    let (file, w) = raced(d, docs, "take")?;
    confirm_take_theirs(d, w)?;
    d.until(
        w,
        "theirs in the editor, taking edits",
        &format!(
            "{TEXT} === {} && !LegibleEditor.isDirty() && !LegibleEditor.isFrozen() \
             && !LegibleEditor.conflictState().bar",
            json_text(THEIRS)
        ),
    )?;
    file_is(d, &file, THEIRS)?;
    stamp_is_the_files(d, w, &file)?;
    // The next save goes through.
    type_at_start(d, w, "Next. ")?;
    press(d, w, "s", false)?;
    file_is(d, &file, &[b"Next. ", THEIRS].concat())?;
    d.until(w, "the text to be saved", "!LegibleEditor.isDirty()")?;
    close_clean(d, w)
}

fn take_theirs_raced_again(d: &Driver, docs: &Path) -> Result<()> {
    let (file, w) = raced(d, docs, "third")?;
    let at = file.clone();
    d.at(Write::Take, Moment::Checked, move || {
        let _ = fs::write(&at, THIRD);
    });
    confirm_take_theirs(d, w)?;
    d.until(
        w,
        "theirs in the editor, and the bar asking about the third writer's",
        &format!(
            "{TEXT} === {} && !LegibleEditor.isFrozen() \
             && LegibleEditor.conflictState().conflict?.kind === 'raced' \
             && LegibleEditor.conflictState().conflict?.id === 2 && LegibleEditor.conflictState().bar",
            json_text(THEIRS)
        ),
    )?;
    file_is(d, &file, THEIRS)?;
    kept_beside(&file, THIRD)?;
    stamp_is_the_files(d, w, &file)?;
    close_clean(d, w)
}

fn take_theirs_written_over(d: &Driver, docs: &Path) -> Result<()> {
    const OTHER: &[u8] = b"# Another\n\nWritten as theirs went in.\n";
    let (file, w) = raced(d, docs, "over")?;
    let ours = d.stamp(w)?;
    type_at_start(d, w, "More. ")?;
    d.until(w, "the edit", "LegibleEditor.isDirty()")?;
    let at = file.clone();
    d.at(Write::Take, Moment::Over, move || {
        let _ = fs::write(&at, OTHER);
    });
    confirm_take_theirs(d, w)?;
    d.until(
        w,
        "the change held, and the edits kept unsaved",
        &format!(
            "LegibleEditor.conflictState().conflict?.kind === 'changed' \
             && LegibleEditor.conflictState().bar && LegibleEditor.isDirty() \
             && !LegibleEditor.isFrozen() && {TEXT} === {}",
            json_text(&[b"More. ", MINE].concat())
        ),
    )?;
    file_is(d, &file, OTHER)?;
    if d.stamp(w)? != ours {
        return Err("the window took up a file that isn't its own".into());
    }
    close_dirty(d, w)
}

fn keep_mine_over_a_change(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "keep", OURS)?;
    let w = open_editing(d, &file)?;
    type_at_start(d, w, "Mine. ")?;
    d.until(w, "the edit", "LegibleEditor.isDirty()")?;
    // Another program writes the file before the save looks: nothing is written, and the bar
    // asks.
    fs::write(&file, THEIRS).map_err(|e| e.to_string())?;
    press(d, w, "s", false)?;
    d.until(
        w,
        "the bar to ask about the change",
        "LegibleEditor.conflictState().conflict?.kind === 'changed' && LegibleEditor.conflictState().bar \
         && LegibleEditor.isDirty()",
    )?;
    if fs::read(&file).map_err(|e| e.to_string())? != THEIRS {
        return Err("the save wrote over a change it found".into());
    }
    click(d, w, "#conflict-bar", &tr("chrome-conflict-keep"))?;
    file_is(d, &file, MINE)?;
    d.until(
        w,
        "the text saved, and the bar gone",
        "!LegibleEditor.isDirty() && !LegibleEditor.conflictState().bar",
    )?;
    stamp_is_the_files(d, w, &file)?;
    close_clean(d, w)
}

fn close_during_a_save(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "closing", b"# Close\n\nText.\n")?;
    let w = open_editing(d, &file)?;
    type_at_start(d, w, "Close. ")?;
    d.until(w, "the edit", "LegibleEditor.isDirty()")?;
    // The save is held between its check and its replace until the close has been asked for.
    let (reached, at_check) = mpsc::channel();
    let (go, released) = mpsc::channel::<()>();
    d.at(Write::Save, Moment::Checked, move || {
        let _ = reached.send(());
        let _ = released.recv_timeout(BOUND);
    });
    press(d, w, "s", false)?;
    at_check
        .recv_timeout(BOUND)
        .map_err(|_| "the save never reached its check".to_owned())?;
    d.hold("clean\n");
    let asked = d.questions();
    d.close(w)?;
    if !d.is_open(w)? || d.questions() != asked {
        return Err("the close didn't wait for the save under way".into());
    }
    let _ = go.send(());
    let (from, answer) = d.held("the page's answer to whether it's clean")?;
    if from != w || answer != "clean\ntrue" {
        return Err(format!("the page answered {answer:?}"));
    }
    // The page has answered: it's frozen, and nothing typed from now on lands.
    let late = d.eval(
        w,
        &format!(
            "(() => {{ const before = {TEXT}; \
             LegibleEditor.view().dispatch({{ changes: {{ from: 0, insert: 'Late. ' }} }}); \
             return LegibleEditor.isFrozen() && {TEXT} === before; }})()"
        ),
    )?;
    if late.as_deref() != Some("true") {
        return Err(format!("an edit after the page's answer: {late:?}"));
    }
    d.release(w, answer)?;
    d.until_true("the window to close", || Ok(!d.is_open(w)?))?;
    file_is(d, &file, b"Close. # Close\n\nText.\n")?;
    if d.questions() != asked {
        return Err("the close asked about unsaved edits".into());
    }
    Ok(())
}

fn save_as_over_a_changed_destination(d: &Driver, docs: &Path) -> Result<()> {
    const SOURCE: &[u8] = b"# Source\n\nText.\n";
    const CHANGED: &[u8] = b"Changed by another program.\n";
    let file = document(docs, "copying", SOURCE)?;
    let destination = file.with_file_name("copy.md");
    fs::write(&destination, b"An old copy.\n").map_err(|e| e.to_string())?;
    let w = open_editing(d, &file)?;
    type_at_start(d, w, "Edited. ")?;
    d.until(w, "the edit", "LegibleEditor.isDirty()")?;
    // The dialog chooses the destination (as it was then); it changes before it's written.
    d.save_as_to(&destination);
    let at = destination.clone();
    d.at(Write::SaveAs, Moment::Staged, move || {
        let _ = fs::write(&at, CHANGED);
    });
    press(d, w, "S", true)?;
    d.until(
        w,
        "the window to say the file wasn't saved",
        &format!(
            "!document.getElementById('notice').hidden \
             && document.getElementById('notice').textContent.includes({})",
            json_string(&tr("chrome-save-as-changed"))
        ),
    )?;
    if fs::read(&destination).map_err(|e| e.to_string())? != CHANGED {
        return Err("the changed destination was written over".into());
    }
    if d.window_of(&destination)?.is_some() {
        return Err("a file that wasn't saved opened".into());
    }
    if fs::read(&file).map_err(|e| e.to_string())? != SOURCE {
        return Err("Save as wrote the document".into());
    }
    if d.file_of(w)? != Some(canonical(&file)?) {
        return Err(format!("the window moved to {:?}", d.file_of(w)?));
    }
    d.until(
        w,
        "the text still unsaved, and taking edits again",
        "LegibleEditor.isDirty() && !LegibleEditor.isFrozen()",
    )?;
    close_dirty(d, w)
}

fn save_as_moves_the_window(d: &Driver, docs: &Path) -> Result<()> {
    const SOURCE: &[u8] = b"# Draft\n\nText.\n";
    let file = document(docs, "draft", SOURCE)?;
    let new = file.with_file_name("final.md");
    let w = open_editing(d, &file)?;
    edited(d, w, "Edited. ")?;
    let (windows, asked) = (d.windows()?, d.questions());
    d.save_as_to(&new);
    press(d, w, "S", true)?;
    let canonical = canonical_when_there(d, &new)?;
    d.until_true("the window to show the new file", || {
        Ok(d.file_of(w)? == Some(canonical.clone()))
    })?;
    file_is(d, &new, b"Edited. # Draft\n\nText.\n")?;
    // In Split still, the edits saved: nothing unsaved, nothing asked, no other window.
    d.until(
        w,
        "the new file's page, in Split, nothing unsaved",
        &format!(
            "{} && LegibleEditor.mode() === 'split' && !LegibleEditor.isDirty() \
             && {TEXT}.startsWith('Edited. ')",
            shows("Draft")
        ),
    )?;
    if d.windows()? != windows || d.questions() != asked {
        return Err("another window opened, or Save as asked about unsaved changes".into());
    }
    if fs::read(&file).map_err(|e| e.to_string())? != SOURCE {
        return Err("the old file changed".into());
    }
    if d.window_of(&file)?.is_some() {
        return Err("the old file still has a window".into());
    }
    if d.title(w)?.as_deref() != Some("final.md – Legible") {
        return Err(format!("the title is {:?}", d.title(w)?));
    }
    if d.recent()?.first() != Some(&canonical) {
        return Err("the new file isn't the newest recent file".into());
    }
    stamp_is_the_files(d, w, &new)?;
    // Live reload watches the new file, and the old one no more.
    fs::write(&file, b"# Old file changed\n").map_err(|e| e.to_string())?;
    fs::write(&new, b"# Final changed\n").map_err(|e| e.to_string())?;
    d.until(w, "the new file's change", &shows("Final changed"))?;
    if d.eval(w, &shows("Old file changed"))?.as_deref() != Some("false") {
        return Err("the old file's change is shown".into());
    }
    close_clean(d, w)
}

fn save_as_makes_a_read_only_file_editable(d: &Driver, docs: &Path) -> Result<()> {
    const SOURCE: &[u8] = b"# Attachment\n\nText.\n";
    let file = document(docs, "attachment", SOURCE)?;
    let _writable = ReadOnly::set(&file)?;
    let new = file.with_file_name("mine.md");
    let w = d.open(&file)?;
    let split = "document.querySelector('#toolbar [data-mode=split]')";
    let save = "document.querySelector('#toolbar [data-command=save]')";
    d.until(
        w,
        "the page, Split and Save unusable",
        &format!(
            "!!window.LegibleChrome?.ready && {split}.getAttribute('aria-disabled') === 'true' \
             && {save}.getAttribute('aria-disabled') === 'true'"
        ),
    )?;
    // Ctrl+S on a file that can't be saved where it is: Save as.
    d.save_as_to(&new);
    press(d, w, "S", false)?;
    let canonical = canonical_when_there(d, &new)?;
    d.until_true("the window to show the new file", || {
        Ok(d.file_of(w)? == Some(canonical.clone()))
    })?;
    file_is(d, &new, SOURCE)?;
    d.until(
        w,
        "the new file's page, Split usable",
        &format!(
            "!!window.LegibleChrome?.ready && {} && !{split}.hasAttribute('aria-disabled')",
            shows("Attachment")
        ),
    )?;
    let offered = d.offered(w)?;
    if !offered.contains(&menu::Command::Mode(Mode::Split)) {
        return Err("the new file can't be edited".into());
    }
    close_clean(d, w)
}

fn close_waits_for_save_as(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "closing-save-as", b"# Original\n")?;
    let new = file.with_file_name("new.md");
    let other = document(docs, "opened-during-save-as", b"# Other\n")?;
    let w = open_editing(d, &file)?;
    edited(d, w, "Saved. ")?;
    // Save as's write, held once it's staged till the test lets it go on.
    let (staged, is_staged) = mpsc::channel::<()>();
    let (release, released) = mpsc::channel::<()>();
    d.at(Write::SaveAs, Moment::Staged, move || {
        let _ = staged.send(());
        let _ = released.recv_timeout(BOUND);
    });
    d.save_as_to(&new);
    press(d, w, "S", true)?;
    is_staged
        .recv_timeout(BOUND)
        .map_err(|_| "Save as's write never started".to_owned())?;
    d.until(w, "the page frozen for Save as", "LegibleEditor.isFrozen()")?;
    // Open meanwhile: refused, the window staying on its file till Save as moves it.
    let asked = d.questions();
    d.open_here(w, &other)?;
    if d.questions() != asked || d.file_of(w)? != Some(canonical(&file)?) {
        return Err("Open went ahead during Save as".into());
    }
    if d.window_of(&other)?.is_some() {
        return Err("the file opened during Save as".into());
    }
    // Closed meanwhile: it waits, asking nothing (Cancel would have unfrozen the page).
    d.close(w)?;
    if d.questions() != asked {
        return Err("the close asked while Save as was being written".into());
    }
    d.eval(
        w,
        "LegibleEditor.view().dispatch({ changes: { from: 0, insert: 'Late. ' } })",
    )?;
    d.until(
        w,
        "the page still frozen, the late edit kept out",
        &format!("LegibleEditor.isFrozen() && !{TEXT}.startsWith('Late. ')"),
    )?;
    if !d.is_open(w)? {
        return Err("the window closed before Save as was over".into());
    }
    // Written: the window moves to the new file, and the close goes on, asking nothing.
    let _ = release.send(());
    file_is(d, &new, b"Saved. # Original\n")?;
    d.until_true("the window to close", || Ok(!d.is_open(w)?))?;
    if d.questions() != asked {
        return Err("the close asked once Save as was over".into());
    }
    file_is(d, &file, b"# Original\n")
}

/// `file`'s read-only attribute, set till this is dropped (a folder with a read-only file in it
/// can't be removed).
struct ReadOnly(PathBuf);

impl ReadOnly {
    fn set(file: &Path) -> Result<ReadOnly> {
        set_read_only(file, true)?;
        Ok(ReadOnly(file.to_owned()))
    }
}

impl Drop for ReadOnly {
    fn drop(&mut self) {
        let _ = set_read_only(&self.0, false);
    }
}

fn set_read_only(file: &Path, read_only: bool) -> Result<()> {
    let mut permissions = fs::metadata(file).map_err(|e| e.to_string())?.permissions();
    permissions.set_readonly(read_only);
    fs::set_permissions(file, permissions).map_err(|e| e.to_string())
}

/// `file`, canonical, once it's there.
fn canonical_when_there(d: &Driver, file: &Path) -> Result<PathBuf> {
    d.until_true(&format!("{} to be written", file.display()), || {
        Ok(file.exists())
    })?;
    canonical(file)
}

fn sign_out_stops_and_asks(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(
        docs,
        "sign-out",
        b"# Notes

Text.
",
    )?;
    let w = open_editing(d, &file)?;
    edited(d, w, "Unsaved. ")?;
    let asked = d.questions();
    d.answer(SaveAnswer::Save);
    // The save's text is held till the copy is checked: once saved, the window closes, and the
    // copy goes with it.
    d.hold("contents\n");
    if d.ask_to_end_session(w, false)? {
        return Err("the session's end wasn't stopped".into());
    }
    let (held, contents) = d.held("the save's text")?;
    // Kept as it answered, before anything was asked.
    let kept = kept_is(
        docs,
        &file,
        Some(
            "Unsaved. # Notes

Text.
",
        ),
    );
    d.release(held, contents)?;
    kept?;
    // Then it asks: Save saves, and the window closes; the copy goes with it.
    d.until_true("the close question", || Ok(d.questions() == asked + 1))?;
    file_is(
        d,
        &file,
        b"Unsaved. # Notes

Text.
",
    )?;
    d.until_true("the window to close", || Ok(!d.is_open(w)?))?;
    kept_is(docs, &file, None)
}

fn sign_out_with_nothing_unsaved(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(
        docs,
        "sign-out-clean",
        b"# Clean
",
    )?;
    let w = open_editing(d, &file)?;
    let asked = d.questions();
    if !d.ask_to_end_session(w, false)? {
        return Err("a window with nothing unsaved stopped the session's end".into());
    }
    kept_is(docs, &file, None)?;
    if d.questions() != asked || !d.is_open(w)? {
        return Err("a window with nothing unsaved asked, or closed".into());
    }
    close_clean(d, w)
}

fn sign_out_cancelled(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(
        docs,
        "sign-out-cancel",
        b"# Cancel
",
    )?;
    let w = open_editing(d, &file)?;
    edited(d, w, "Mine. ")?;
    let asked = d.questions();
    d.answer(SaveAnswer::Cancel);
    if d.ask_to_end_session(w, false)? {
        return Err("the session's end wasn't stopped".into());
    }
    d.until_true("the close question", || Ok(d.questions() == asked + 1))?;
    // Cancelled: the window stays, unsaved, and its copy too while the session may still end.
    d.until(w, "the text still unsaved", "LegibleEditor.isDirty()")?;
    kept_is(
        docs,
        &file,
        Some(
            "Mine. # Cancel
",
        ),
    )?;
    // Windows says the session goes on: the copy goes, as the text is still in its window.
    d.session_goes_on(w)?;
    d.until_true("the copy to go", || Ok(kept_of(docs, &file).is_none()))?;
    if fs::read(&file).map_err(|e| e.to_string())?
        != b"# Cancel
"
    {
        return Err("a cancelled question saved".into());
    }
    close_dirty(d, w)
}

fn sign_out_asks_each_in_turn(d: &Driver, docs: &Path) -> Result<()> {
    let older = document(
        docs,
        "sign-out-older",
        b"# Older
",
    )?;
    let newer = document(
        docs,
        "sign-out-newer",
        b"# Newer
",
    )?;
    let first = open_editing(d, &older)?;
    let second = open_editing(d, &newer)?;
    edited(d, first, "Dropped. ")?;
    edited(d, second, "Saved. ")?;
    // The older asks first (Don't save), then the newer (Save), whichever Windows asked.
    let asked = d.questions();
    d.answer(SaveAnswer::DontSave);
    d.answer(SaveAnswer::Save);
    if d.ask_to_end_session(second, false)? {
        return Err("the session's end wasn't stopped".into());
    }
    d.until_true("both windows to close", || {
        Ok(!d.is_open(first)? && !d.is_open(second)?)
    })?;
    if d.questions() != asked + 2 {
        return Err("each unsaved window didn't ask once".into());
    }
    file_is(
        d,
        &newer,
        b"Saved. # Newer
",
    )?;
    if fs::read(&older).map_err(|e| e.to_string())?
        != b"# Older
"
    {
        return Err(
            "the older window's Don't save saved, or they asked the other way round".into(),
        );
    }
    kept_is(docs, &older, None)?;
    kept_is(docs, &newer, None)
}

fn sign_out_critical(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(
        docs,
        "sign-out-critical",
        b"# Critical
",
    )?;
    let w = open_editing(d, &file)?;
    edited(d, w, "Kept. ")?;
    let asked = d.questions();
    if !d.ask_to_end_session(w, true)? {
        return Err("a critical end was stopped".into());
    }
    kept_is(
        docs,
        &file,
        Some(
            "Kept. # Critical
",
        ),
    )?;
    if d.questions() != asked {
        return Err("a critical end asked".into());
    }
    // Closed with Don't save: dropped as chosen, so the copy goes too.
    close_dirty(d, w)?;
    kept_is(docs, &file, None)
}

fn kept_restored(d: &Driver, docs: &Path) -> Result<()> {
    const ON_DISK: &[u8] = b"# Kept\n\nOld.\n";
    const RESTORED: &str = "Restored. # Kept\n\nOld.\n";
    let file = document(docs, "kept", ON_DISK)?;
    keep_copy(docs, &file, ON_DISK, RESTORED)?;
    d.offer_kept()?;
    let w = d.window_of(&file)?.ok_or("the copy's file didn't open")?;
    d.until(w, "the bar to offer it", &restore_bar("kept.md", false))?;
    click(d, w, "#conflict-bar", &tr("chrome-restore"))?;
    d.until(
        w,
        "the text in the editor, unsaved",
        &format!(
            "LegibleEditor.mode() === 'split' && {TEXT} === {} && LegibleEditor.isDirty() \
             && !LegibleEditor.conflictState().bar",
            json_string(RESTORED)
        ),
    )?;
    d.until_true("the copy to go", || Ok(kept_of(docs, &file).is_none()))?;
    if fs::read(&file).map_err(|e| e.to_string())? != ON_DISK {
        return Err("Restore wrote the file".into());
    }
    // Saved as any edit is.
    press(d, w, "s", false)?;
    file_is(d, &file, RESTORED.as_bytes())?;
    d.until(w, "the text to be saved", "!LegibleEditor.isDirty()")?;
    close_clean(d, w)
}

fn kept_discarded(d: &Driver, docs: &Path) -> Result<()> {
    const ON_DISK: &[u8] = b"# Changed\n\nTheirs since.\n";
    let file = document(docs, "discard", ON_DISK)?;
    keep_copy(docs, &file, b"# Changed\n", "Mine. # Changed\n")?;
    d.offer_kept()?;
    let w = d.window_of(&file)?.ok_or("the copy's file didn't open")?;
    d.until(
        w,
        "the bar to offer it, saying the file changed",
        &restore_bar("discard.md", true),
    )?;
    click(d, w, "#conflict-bar", &tr("chrome-discard"))?;
    d.until_true("the copy to go", || Ok(kept_of(docs, &file).is_none()))?;
    d.until(
        w,
        "the bar gone, nothing unsaved",
        "!LegibleEditor.conflictState().bar && !LegibleEditor.isDirty() && LegibleEditor.mode() === 'view'",
    )?;
    if fs::read(&file).map_err(|e| e.to_string())? != ON_DISK {
        return Err("Discard wrote the file".into());
    }
    close_clean(d, w)
}

fn kept_until_chosen(d: &Driver, docs: &Path) -> Result<()> {
    const ON_DISK: &[u8] = b"# Later\n";
    let file = document(docs, "later", ON_DISK)?;
    keep_copy(docs, &file, ON_DISK, "Later. # Later\n")?;
    d.offer_kept()?;
    let w = d.window_of(&file)?.ok_or("the copy's file didn't open")?;
    d.until(w, "the bar to offer it", &restore_bar("later.md", false))?;
    // Nothing unsaved in the window itself: it closes without asking, and the copy stays.
    close_clean(d, w)?;
    kept_is(docs, &file, Some("Later. # Later\n"))?;
    // Offered again (as at the next start), and dropped this time.
    d.offer_kept()?;
    let w = d
        .window_of(&file)?
        .ok_or("the copy's file didn't open again")?;
    d.until(
        w,
        "the bar to offer it again",
        &restore_bar("later.md", false),
    )?;
    click(d, w, "#conflict-bar", &tr("chrome-discard"))?;
    d.until_true("the copy to go", || Ok(kept_of(docs, &file).is_none()))?;
    close_clean(d, w)
}

fn kept_elsewhere(d: &Driver, docs: &Path) -> Result<()> {
    let dir = docs.join("gone");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let gone = dir.join("gone.md");
    let rescued = dir.join("rescued.md");
    keep_copy(docs, &gone, b"# Gone\n", "Mine. # Gone\n")?;
    let asked = d.questions();
    d.answer(SaveAnswer::Save);
    d.save_as_to(&rescued);
    d.offer_kept()?;
    if d.questions() != asked + 1 {
        return Err("a copy of a file that has gone didn't ask".into());
    }
    file_is(d, &rescued, b"Mine. # Gone\n")?;
    let w = d.window_of(&rescued)?.ok_or("the new file didn't open")?;
    if kept_of(docs, &gone).is_some() || gone.exists() {
        return Err("the copy stayed, or the file that went was written".into());
    }
    close_clean(d, w)
}

fn kept_already_there(d: &Driver, docs: &Path) -> Result<()> {
    const ON_DISK: &[u8] = b"# Same\r\n\r\nSaved after all.\r\n";
    let file = document(docs, "same", ON_DISK)?;
    // The copy's text is the file's (its line endings aside): it was saved after all.
    keep_copy(docs, &file, b"# Same\r\n", "# Same\n\nSaved after all.\n")?;
    let asked = d.questions();
    d.offer_kept()?;
    if d.window_of(&file)?.is_some() || d.questions() != asked {
        return Err("a copy its file already holds was offered".into());
    }
    kept_is(docs, &file, None)
}

fn sign_out_before_the_preview(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "before-preview", b"# Original\n")?;
    let w = open_editing(d, &file)?;
    // A first edit, its preview held back: what's kept is the edit all the same.
    d.hold("render\n");
    type_at_start(d, w, "First. ")?;
    let (from, render) = d.held("the first edit's preview")?;
    if !d.ask_to_end_session(w, true)? {
        return Err("a critical end was stopped".into());
    }
    kept_is(docs, &file, Some("First. # Original\n"))?;
    d.release(from, render)?;
    // And the newest of several, typed while the preview of an older one is held.
    d.hold("render\n");
    type_at_start(d, w, "Newer. ")?;
    let (from, render) = d.held("the newer edit's preview")?;
    type_at_start(d, w, "Newest. ")?;
    if !d.ask_to_end_session(w, true)? {
        return Err("a critical end was stopped".into());
    }
    kept_is(docs, &file, Some("Newest. Newer. First. # Original\n"))?;
    d.release(from, render)?;
    close_dirty(d, w)?;
    kept_is(docs, &file, None)
}

fn sign_out_keeps_an_unchosen_copy(d: &Driver, docs: &Path) -> Result<()> {
    const PRIOR: &str = "An earlier run's unsaved work.\n";
    let file = document(docs, "unchosen", b"# On disk\n")?;
    keep_copy(docs, &file, b"# On disk\n", PRIOR)?;
    d.offer_kept()?;
    let w = d.window_of(&file)?.ok_or("the copy's file didn't open")?;
    d.until(w, "the bar to offer it", &restore_bar("unchosen.md", false))?;
    // Neither Restore nor Discard: the file is edited, and the session ends.
    open_editing(d, &file)?;
    edited(d, w, "This run. ")?;
    if !d.ask_to_end_session(w, true)? {
        return Err("a critical end was stopped".into());
    }
    let texts = kept_texts(docs, &file);
    if texts != [PRIOR, "This run. # On disk\n"] {
        return Err(format!("the copies kept are {texts:?}"));
    }
    // Closed with Don't save: this run's copy goes, the unchosen one stays and is offered again.
    close_dirty(d, w)?;
    if kept_texts(docs, &file) != [PRIOR] {
        return Err(format!("the copies kept are {:?}", kept_texts(docs, &file)));
    }
    d.offer_kept()?;
    let w = d
        .window_of(&file)?
        .ok_or("the copy's file didn't open again")?;
    d.until(
        w,
        "the bar to offer it again",
        &restore_bar("unchosen.md", false),
    )?;
    click(d, w, "#conflict-bar", &tr("chrome-discard"))?;
    d.until_true("the copy to go", || Ok(kept_texts(docs, &file).is_empty()))?;
    close_clean(d, w)
}

fn sign_out_protects_every_window(d: &Driver, docs: &Path) -> Result<()> {
    let first = document(docs, "every-a", b"# A\n")?;
    let second = document(docs, "every-b", b"# B\n")?;
    let a = open_editing(d, &first)?;
    let b = open_editing(d, &second)?;
    edited(d, a, "Unsaved A. ")?;
    edited(d, b, "Unsaved B. ")?;
    // Only `a` is asked; its question is cancelled. `b` is kept all the same.
    d.answer(SaveAnswer::Cancel);
    if d.ask_to_end_session(a, false)? {
        return Err("the session's end wasn't stopped".into());
    }
    kept_is(docs, &first, Some("Unsaved A. # A\n"))?;
    kept_is(docs, &second, Some("Unsaved B. # B\n"))?;
    d.session_goes_on(a)?;
    d.until_true("the copies to go", || {
        Ok(kept_of(docs, &first).is_none() && kept_of(docs, &second).is_none())
    })?;
    close_dirty(d, a)?;
    close_dirty(d, b)
}

fn reading_place_kept(d: &Driver, docs: &Path) -> Result<()> {
    let mut text = String::from("# A long read\n\n");
    for part in 1..=12 {
        text.push_str(&format!("## Part {part}\n\n"));
        for _ in 0..10 {
            text.push_str(
                "A paragraph to read, long enough that the document runs well past a screen.\n\n",
            );
        }
    }
    let file = document(docs, "place", text.as_bytes())?;
    let part_seven =
        "[...document.querySelectorAll('#document h2')].find((h) => h.textContent === 'Part 7')";
    let w = d.open(&file)?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    // Closed the moment the scroll is done, before the page has even had its scroll event: the
    // close asks the page where the reader is, so the place is kept all the same.
    let scrolled = d.eval(
        w,
        &format!("(() => {{ {part_seven}.scrollIntoView(); return window.scrollY > 500; }})()"),
    )?;
    if scrolled.as_deref() != Some("true") {
        return Err(format!("the page didn't scroll: {scrolled:?}"));
    }
    close_clean(d, w)?;
    if d.places_kept()? == 0 {
        return Err("closed straight after scrolling, no place was kept".into());
    }
    // Opened again: back at Part 7, by its heading, under the chrome.
    let w = d.open(&file)?;
    d.until(
        w,
        "the page back at Part 7",
        &format!(
            "(() => {{ const top = {part_seven}?.getBoundingClientRect().top; \
             return window.scrollY > 500 && top > -4 && top < 160; }})()"
        ),
    )?;
    // Turned off: what's kept goes, the file too, and nothing more is kept.
    set_setting(d, w, "remember-place", "false")?;
    let kept = docs
        .parent()
        .ok_or("no run folder")?
        .join("appdata")
        .join("Legible")
        .join(legible_win::places::FILE);
    d.until_true("the places to be forgotten", || {
        Ok(d.places_kept()? == 0 && !kept.exists())
    })?;
    close_clean(d, w)?;
    if d.places_kept()? != 0 {
        return Err("a place was kept with the setting off".into());
    }
    // Back on, for the cases after this one.
    let w = d.open(&file)?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    set_setting(d, w, "remember-place", "true")?;
    close_clean(d, w)
}

fn opens_quietly(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "quiet", b"# Quiet\n")?;
    let w = d.open(&file)?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    if d.on_a_monitor(w)? != Some(false) {
        return Err(format!("opened on a monitor, at {:?}", d.position(w)?));
    }
    close_clean(d, w)?;
    // Closed off every monitor, the next opens off them too; its editor focused in the page,
    // still nothing of the window's takes the focus.
    let w = open_editing(d, &file)?;
    if d.on_a_monitor(w)? != Some(false) {
        return Err(format!(
            "the next opened on a monitor, at {:?}",
            d.position(w)?
        ));
    }
    d.until(
        w,
        "the editor to take the page's focus",
        "(LegibleEditor.view().focus(), \
         LegibleEditor.view().root.activeElement === LegibleEditor.view().contentDOM)",
    )
    .map_err(|why| {
        let state = d.eval(
            w,
            "JSON.stringify({ active: document.activeElement?.outerHTML?.slice(0, 120), \
             hasFocus: document.hasFocus(), mode: LegibleEditor.mode(), \
             visible: document.visibilityState, cm: !!LegibleEditor.view().contentDOM?.isConnected })",
        );
        format!("{why}; the page: {state:?}")
    })?;
    d.focus_kept()?;
    close_clean(d, w)
}

fn focus_check_hears_between_checks(d: &Driver, _: &Path) -> Result<()> {
    d.focus_kept()?;
    d.show_a_window_that_could_be_activated()?;
    d.until_true("Windows to tell the focus check", || {
        Ok(d.focus_heard()? > 0)
    })?;
    // Gone by the check, as a window that took the focus and closed would be: still heard.
    d.close_the_window_that_could_be_activated()?;
    match d.focus_kept() {
        Err(why) if why.contains("shown able to be activated") => {}
        other => return Err(format!("the focus check said {other:?}")),
    }
    // Heard once: the next check starts afresh.
    d.focus_kept()
}

fn dialogs_stay_unshown(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "dialogs", b"# Dialogs\n")?;
    let w = d.open(&file)?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    // A dialog shown would be heard by the focus check after this case, and would wait for an
    // answer: none is, and each goes as if cancelled.
    let before = win32::dialogs_held_back().len();
    d.release(w, "open".into())?;
    d.release(w, "save-pdf".into())?;
    d.until_true("both dialogs to be held back", || {
        let held = win32::dialogs_held_back().split_off(before);
        Ok(held.len() >= 2)
    })?;
    let held = win32::dialogs_held_back().split_off(before);
    if held != ["the Open dialog", "the Save dialog"] {
        return Err(format!("dialogs held back: {held:?}"));
    }
    // Held back in the turn that then cancels it: no Save as PDF is under way.
    if d.saving_pdf(w)? {
        return Err("Save as PDF is still under way".into());
    }
    if d.window_of(&file)? != Some(w) {
        return Err("the window didn't stay as it was".into());
    }
    close_clean(d, w)
}

fn windows_reopen_where_they_were(d: &Driver, docs: &Path) -> Result<()> {
    let (x, y, work_width, work_height) = d.primary_work_area()?.ok_or("no primary monitor")?;
    let file = document(docs, "placed", b"# Placed\n")?;
    let w = d.open(&file)?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    let at = (x + 40, y + 30);
    d.place_window(w, at, (640.0, 480.0))?;
    d.until_true("the window to move", || {
        Ok(d.position(w)? == Some(at)
            && d.size(w)?.is_some_and(|(width, height)| {
                (width - 640.0).abs() < 1.0 && (height - 480.0).abs() < 1.0
            }))
    })?;
    close_clean(d, w)?;
    // The next window opens just there, at that size.
    let w = d.open(&file)?;
    let (position, size) = (d.position(w)?, d.size(w)?);
    let same_size = size
        .is_some_and(|(width, height)| (width - 640.0).abs() < 1.0 && (height - 480.0).abs() < 1.0);
    if position != Some(at) || !same_size {
        return Err(format!(
            "opened at {position:?}, {size:?}, not {at:?}, 640×480"
        ));
    }
    // Left hanging off the work area's bottom right: the next opens fully on it, its frame, title
    // and menu bar too.
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    let off = (x + work_width - 300, y + work_height - 200);
    d.place_window(w, off, (640.0, 480.0))?;
    d.until_true("the window to move", || Ok(d.position(w)? == Some(off)))?;
    close_clean(d, w)?;
    let fully_on = |w: Window| -> Result<()> {
        let (left, top, width, height) = d.outer_rect(w)?.ok_or("the window has gone")?;
        let on = left >= x
            && top >= y
            && left + width <= x + work_width
            && top + height <= y + work_height;
        if on {
            Ok(())
        } else {
            Err(format!(
                "opened at ({left}, {top}) {width}×{height}, off the work area ({x}, {y}) {work_width}×{work_height}"
            ))
        }
    };
    let w = d.open(&file)?;
    fully_on(w)?;
    // Left bigger than the work area: the next is no bigger than it, frame and all.
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    d.place_window(w, (x, y), (5000.0, 4000.0))?;
    d.until_true("the window to grow", || {
        Ok(d.outer_rect(w)?
            .is_some_and(|(_, _, width, height)| width > work_width || height > work_height))
    })?;
    close_clean(d, w)?;
    let w = d.open(&file)?;
    fully_on(w)?;
    // Turned off: the first size, wherever Windows puts it.
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    set_setting(d, w, "reopen-windows", "false")?;
    close_clean(d, w)?;
    let w = d.open(&file)?;
    if d.size(w)?
        .is_some_and(|(width, height)| (width - 640.0).abs() < 1.0 && (height - 480.0).abs() < 1.0)
    {
        return Err("a window opened at the last one's size with the setting off".into());
    }
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    set_setting(d, w, "reopen-windows", "true")?;
    close_clean(d, w)
}

fn close_keeps_a_late_edit(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "late", b"# Late\n\nText.\n")?;
    // An edit tried while the closing page says where the reader is: it's frozen by then, so
    // the edit doesn't land, and the window closes without asking. The page says how it went.
    let w = open_editing(d, &file)?;
    d.hold("probe-late\n");
    d.eval(
        w,
        "(() => { const place = LegibleChrome.place; LegibleChrome.place = () => { \
         LegibleEditor.view().dispatch({ changes: { from: 0, insert: 'Late. ' } }); \
         const landed = LegibleEditor.view().state.doc.toString().startsWith('Late. '); \
         window.ipc.postMessage(`probe-late\\n${landed}`); return place(); }; })()",
    )?;
    let asked = d.questions();
    d.close(w)?;
    let (_, landed) = d.held("the page's word on the late edit")?;
    if landed != "probe-late\nfalse" {
        return Err(format!("an edit landed as the window closed: {landed:?}"));
    }
    d.until_true("the window to close", || Ok(!d.is_open(w)?))?;
    if d.questions() != asked || fs::read(&file).map_err(|e| e.to_string())? != b"# Late\n\nText.\n"
    {
        return Err("a clean window asked, or wrote, as it closed".into());
    }
    // An edit that lands just before the closing page freezes (typed as Close was clicked):
    // the window asks, as any close with unsaved edits does; Cancel keeps it, taking edits.
    let w = open_editing(d, &file)?;
    d.eval(
        w,
        "(() => { const freeze = LegibleEditor.freeze; LegibleEditor.freeze = () => { \
         LegibleEditor.view().dispatch({ changes: { from: 0, insert: 'Typed. ' } }); freeze(); }; })()",
    )?;
    let asked = d.questions();
    d.answer(SaveAnswer::Cancel);
    d.close(w)?;
    d.until_true("the close question", || Ok(d.questions() == asked + 1))?;
    if !d.is_open(w)? {
        return Err("a window with an edit closed without asking".into());
    }
    d.until(
        w,
        "the edit kept, unsaved, the text taking edits",
        &format!(
            "{TEXT}.startsWith('Typed. ') && LegibleEditor.isDirty() && !LegibleEditor.isFrozen()"
        ),
    )?;
    close_dirty(d, w)
}

// --- Steps -------------------------------------------------------------------------------------

fn read_once_the_window_is_made(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(
        docs, "read", b"# Read
",
    )?;
    let canonical = fs::canonicalize(&file).map_err(|e| e.to_string())?;
    let w = d.open(&file)?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    if d.recent()?.first() != Some(&canonical) {
        return Err(format!("{} isn't the newest recent file", file.display()));
    }
    close_clean(d, w)?;

    // Held open with no sharing, as another program may hold it: it's there, but can't be read.
    let file = document(
        docs,
        "locked",
        b"# Locked
",
    )?;
    let canonical = fs::canonicalize(&file).map_err(|e| e.to_string())?;
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&file)
        .map_err(|e| e.to_string())?;
    let said = d.messages().len();
    let w = d.open(&file)?;
    let closed = d.until_true("the window to close", || Ok(!d.is_open(w)?));
    drop(held);
    closed?;
    let messages = d.messages();
    if !messages[said..].iter().any(|m| m.contains("locked.md")) {
        return Err(format!("nothing said why: {:?}", &messages[said..]));
    }
    if d.window_of(&file)?.is_some() {
        return Err("the file still has a window".into());
    }
    if d.recent()?.contains(&canonical) {
        return Err("a file that couldn't be read is a recent file".into());
    }
    Ok(())
}

fn commands_wait_for_the_file(d: &Driver, docs: &Path) -> Result<()> {
    const SOURCE: &[u8] = b"# Opening

Text.
";
    const NEED_IT: [menu::Command; 6] = [
        menu::Command::SaveAs,
        menu::Command::SavePdf,
        menu::Command::Print,
        menu::Command::CopyDocument,
        menu::Command::Find,
        menu::Command::Mode(Mode::View),
    ];
    let file = document(docs, "opening", SOURCE)?;
    let destination = file.with_file_name("copy.md");
    // Its file is read, and held there: the window stays opening till it's handed on.
    d.hold_opening();
    let w = d.open(&file)?;
    if d.opening_held()? != w {
        return Err("another window's file was held".into());
    }
    let offered = d.offered(w)?;
    for command in [
        menu::Command::Open,
        menu::Command::Close,
        menu::Command::Exit,
    ] {
        if !offered.contains(&command) {
            return Err(format!("{command:?} isn't offered while the file is read"));
        }
    }
    if let Some(command) = NEED_IT.iter().find(|c| offered.contains(c)) {
        return Err(format!("{command:?} is offered before the file is read"));
    }
    // Save as asked for anyway shows no dialog: the destination is the next one's. The
    // window hears it before its file, in the order they're sent.
    d.save_as_to(&destination);
    d.release(w, "save-as".into())?;
    d.finish_opening()?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    let offered = d.offered(w)?;
    if let Some(command) = NEED_IT.iter().find(|c| !offered.contains(c)) {
        return Err(format!("{command:?} isn't offered once the file is read"));
    }
    d.release(w, "save-as".into())?;
    // The window moves to the new file: no window of its own.
    let windows = d.windows()?;
    d.until_true("the window to show the new file", || {
        Ok(d.window_of(&destination)? == Some(w))
    })?;
    if fs::read(&destination).map_err(|e| e.to_string())? != SOURCE {
        return Err("the new file isn't the file".into());
    }
    if d.windows()? != windows {
        return Err("Save as opened a window".into());
    }
    close_clean(d, w)
}

fn open_replaces_a_clean_document(d: &Driver, docs: &Path) -> Result<()> {
    // A picture at the same path beside each: the new document's shows, never the old one's.
    let old = document(docs, "replaced", b"# Replaced\n\n![p](pic.svg)\n")?;
    picture_beside(&old, 10)?;
    let new = document(docs, "replacing", b"# Replacing\n\n![p](pic.svg)\n")?;
    picture_beside(&new, 20)?;
    let w = d.open(&old)?;
    d.until(w, "the old document's picture", &picture_is(10))?;
    let (windows, asked) = (d.windows()?, d.questions());
    d.open_here(w, &new)?;
    d.until(
        w,
        "the new document, with its own picture",
        &format!("{} && {}", shows("Replacing"), picture_is(20)),
    )?;
    if d.windows()? != windows || d.questions() != asked {
        return Err("another window opened, or a clean document asked".into());
    }
    let canonical = canonical(&new)?;
    if d.file_of(w)? != Some(canonical.clone()) || d.window_of(&old)?.is_some() {
        return Err(format!("the window shows {:?}", d.file_of(w)?));
    }
    if d.title(w)?.as_deref() != Some("replacing.md – Legible") {
        return Err(format!("the title is {:?}", d.title(w)?));
    }
    if d.recent()?.first() != Some(&canonical) {
        return Err("the new file isn't the newest recent file".into());
    }
    // Live reload watches the new file, and the old one no more.
    fs::write(&old, b"# Old file changed\n").map_err(|e| e.to_string())?;
    fs::write(&new, b"# Replacing changed\n").map_err(|e| e.to_string())?;
    d.until(w, "the new file's change", &shows("Replacing changed"))?;
    if d.eval(w, &shows("Old file changed"))?.as_deref() != Some("false") {
        return Err("the old file's change is shown".into());
    }
    close_clean(d, w)
}

fn open_over_unsaved_changes(d: &Driver, docs: &Path) -> Result<()> {
    let first = document(docs, "unsaved-first", b"# First\n")?;
    let second = document(docs, "unsaved-second", b"# Second\n")?;
    let third = document(docs, "unsaved-third", b"# Third\n")?;
    let w = open_editing(d, &first)?;
    edited(d, w, "Kept. ")?;
    // Cancel: the window stays as it was, the edit unsaved.
    let asked = d.questions();
    d.answer(SaveAnswer::Cancel);
    d.open_here(w, &second)?;
    if d.questions() != asked + 1 {
        return Err("unsaved changes weren't asked about".into());
    }
    if d.file_of(w)? != Some(canonical(&first)?) || d.window_of(&second)?.is_some() {
        return Err("Cancel didn't keep the document".into());
    }
    d.until(
        w,
        "the edit, still unsaved and taking edits",
        &format!(
            "LegibleEditor.isDirty() && {TEXT}.startsWith('Kept. ') && !LegibleEditor.isFrozen()"
        ),
    )?;
    // Don't save: dropped, and the next file shows, in the mode the reader had.
    d.answer(SaveAnswer::DontSave);
    d.open_here(w, &second)?;
    d.until(
        w,
        "the second file, in Split, nothing unsaved",
        &format!(
            "{} && LegibleEditor.mode() === 'split' && !LegibleEditor.isDirty()",
            shows("Second")
        ),
    )?;
    file_is(d, &first, b"# First\n")?;
    if d.title(w)?.as_deref() != Some("unsaved-second.md – Legible") {
        return Err(format!("the title is {:?}", d.title(w)?));
    }
    // Save: saved, then the next file shows.
    d.until(
        w,
        "the second file's text",
        &format!("{TEXT} === '# Second\\n'"),
    )?;
    edited(d, w, "Saved. ")?;
    d.answer(SaveAnswer::Save);
    d.open_here(w, &third)?;
    file_is(d, &second, b"Saved. # Second\n")?;
    d.until(
        w,
        "the third file, in Split",
        &format!("{} && LegibleEditor.mode() === 'split'", shows("Third")),
    )?;
    if d.questions() != asked + 3 {
        return Err(format!("asked {} times, not 3", d.questions() - asked));
    }
    close_clean(d, w)
}

fn open_drops_the_copy_kept(d: &Driver, docs: &Path) -> Result<()> {
    let old = document(docs, "kept-replaced", b"# Kept\n")?;
    let new = document(docs, "kept-replacing", b"# Next\n")?;
    let w = open_editing(d, &old)?;
    edited(d, w, "Mine. ")?;
    let asked = d.questions();
    d.answer(SaveAnswer::Cancel);
    if d.ask_to_end_session(w, false)? {
        return Err("the session's end wasn't stopped".into());
    }
    d.until_true("the close question", || Ok(d.questions() == asked + 1))?;
    kept_is(docs, &old, Some("Mine. # Kept\n"))?;
    // Dropped as the next file opens: the copy goes, and the window has nothing unsaved.
    d.answer(SaveAnswer::DontSave);
    d.open_here(w, &new)?;
    d.until(w, "the next file", &shows("Next"))?;
    kept_is(docs, &old, None)?;
    if !d.ask_to_end_session(w, false)? {
        return Err("the new document stopped the session's end".into());
    }
    kept_is(docs, &new, None)?;
    close_clean(d, w)
}

fn open_keeps_the_reading_place(d: &Driver, docs: &Path) -> Result<()> {
    let long = |title: &str| {
        let mut text = format!("# {title}\n\n");
        for part in 1..=12 {
            text.push_str(&format!("## Part {part}\n\n"));
            for _ in 0..10 {
                text.push_str(
                    "A paragraph to read, long enough that the document runs well past a screen.\n\n",
                );
            }
        }
        text
    };
    let old = document(docs, "place-old", long("Old").as_bytes())?;
    let new = document(docs, "place-new", long("New").as_bytes())?;
    let part_seven =
        "[...document.querySelectorAll('#document h2')].find((h) => h.textContent === 'Part 7')";
    let w = d.open(&old)?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    let scrolled = d.eval(
        w,
        &format!("(() => {{ {part_seven}.scrollIntoView(); return window.scrollY > 500; }})()"),
    )?;
    if scrolled.as_deref() != Some("true") {
        return Err(format!("the page didn't scroll: {scrolled:?}"));
    }
    let kept = d.places_kept()?;
    d.open_here(w, &new)?;
    d.until(
        w,
        "the new file, from the top",
        &format!(
            "!!window.LegibleChrome?.ready && {} && window.scrollY === 0",
            shows("New")
        ),
    )?;
    if d.places_kept()? != kept + 1 {
        return Err("the old file's place wasn't kept".into());
    }
    // The old file again, in the same window: back at Part 7.
    d.open_here(w, &old)?;
    d.until(
        w,
        "the old file back at Part 7",
        &format!(
            "(() => {{ const top = {part_seven}?.getBoundingClientRect().top; \
             return window.scrollY > 500 && top > -4 && top < 160; }})()"
        ),
    )?;
    close_clean(d, w)
}

fn drop_replaces_and_opens_the_rest(d: &Driver, docs: &Path) -> Result<()> {
    let target = document(docs, "dropped-on", b"# Dropped on\n")?;
    let first = document(docs, "dropped-first", b"# Dropped first\n")?;
    let second = document(docs, "dropped-second", b"# Dropped second\n")?;
    let w = d.open(&target)?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    d.drop_on(w, vec![first.clone(), second.clone()])?;
    d.until(w, "the first file in this window", &shows("Dropped first"))?;
    d.until_true("the second file's window", || {
        Ok(d.window_of(&second)?.is_some())
    })?;
    let other = d
        .window_of(&second)?
        .ok_or("no window for the second file")?;
    if other == w || d.window_of(&first)? != Some(w) || d.window_of(&target)?.is_some() {
        return Err("the drop didn't replace the document and open the rest".into());
    }
    close_clean(d, other)?;
    close_clean(d, w)
}

fn open_elsewhere_and_launch(d: &Driver, docs: &Path) -> Result<()> {
    let here = document(docs, "elsewhere-here", b"# Here\n")?;
    let there = document(docs, "elsewhere-there", b"# There\n")?;
    let launched = document(docs, "launched", b"# Launched\n")?;
    let w = d.open(&here)?;
    let other = d.open(&there)?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    d.until(other, "the page to start", "!!window.LegibleChrome?.ready")?;
    let (windows, asked) = (d.windows()?, d.questions());
    d.open_here(w, &there)?;
    if d.window_of(&there)? != Some(other)
        || d.file_of(w)? != Some(canonical(&here)?)
        || d.windows()? != windows
        || d.questions() != asked
    {
        return Err("a file open elsewhere changed a window, or opened twice".into());
    }
    // A later launch's file opens in a window of its own, whatever else is open.
    d.launch(vec![launched.clone()])?;
    d.until_true("the launched file's window", || {
        Ok(d.window_of(&launched)?.is_some())
    })?;
    let new = d
        .window_of(&launched)?
        .ok_or("no window for the launched file")?;
    if new == w || new == other || d.file_of(w)? != Some(canonical(&here)?) {
        return Err("the launched file didn't open in a window of its own".into());
    }
    d.until(new, "the page to start", "!!window.LegibleChrome?.ready")?;
    close_clean(d, new)?;
    close_clean(d, other)?;
    close_clean(d, w)
}

fn launch_without_a_file(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "launch-empty", b"# Open already\n")?;
    let w = d.open(&file)?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    let windows = d.windows()?;
    // The shortcut double-clicked, with Legible running: its hand-off names no file.
    d.launch(Vec::new())?;
    d.until_true("an empty window", || Ok(d.empty_window()?.is_some()))?;
    let empty = d.empty_window()?.ok_or("no empty window")?;
    if d.windows()? != windows + 1 || d.file_of(w)? != Some(canonical(&file)?) {
        return Err("the launch didn't open a window of its own".into());
    }
    d.until(
        empty,
        "the empty page",
        "document.readyState === 'complete'",
    )?;
    close_clean(d, empty)?;
    close_clean(d, w)
}

fn open_over_a_save_that_conflicts(d: &Driver, docs: &Path) -> Result<()> {
    let file = document(docs, "replace-conflict", b"# Mine\n")?;
    let next = document(docs, "replace-conflict-next", b"# Next\n")?;
    let w = open_editing(d, &file)?;
    edited(d, w, "Unsaved. ")?;
    // Another program writes the file once the save's bytes are staged: the save finds it
    // changed, and writes nothing.
    let at = file.clone();
    d.at(Write::Save, Moment::Staged, move || {
        let _ = fs::write(&at, b"# Theirs\n");
    });
    let asked = d.questions();
    d.answer(SaveAnswer::Save);
    d.open_here(w, &next)?;
    d.until(
        w,
        "the conflict bar, the edit unsaved and taking edits",
        &format!(
            "LegibleEditor.conflictState().bar && LegibleEditor.isDirty() \
             && !LegibleEditor.isFrozen() && {TEXT}.startsWith('Unsaved. ')"
        ),
    )?;
    file_is(d, &file, b"# Theirs\n")?;
    if d.file_of(w)? != Some(canonical(&file)?) || d.window_of(&next)?.is_some() {
        return Err("the next file took the window".into());
    }
    if d.questions() != asked + 1 {
        return Err("unsaved changes weren't asked about".into());
    }
    // Still protected as Windows ends the session: a copy is kept, and the window asks.
    d.answer(SaveAnswer::Cancel);
    if d.ask_to_end_session(w, false)? {
        return Err("the session's end wasn't stopped".into());
    }
    kept_is(docs, &file, Some("Unsaved. # Mine\n"))?;
    d.until_true("the close question", || Ok(d.questions() == asked + 2))?;
    d.session_goes_on(w)?;
    d.until_true("the copy to go", || Ok(kept_of(docs, &file).is_none()))?;
    // A later close closes the window: the replace given up isn't carried out.
    close_dirty(d, w)?;
    if d.window_of(&next)?.is_some() {
        return Err("the next file opened as the window closed".into());
    }
    Ok(())
}

fn replaced_page_says_nothing(d: &Driver, docs: &Path) -> Result<()> {
    let text = |title: &str| format!("# {title}\n\n![p]({PICTURE})\n");
    let old = document(docs, "left-behind", text("Left behind").as_bytes())?;
    let new = document(docs, "taking-over", text("Taking over").as_bytes())?;
    let w = d.open(&old)?;
    d.until(
        w,
        "the old document's pictures bar to ask",
        &pictures_bar_asks(),
    )?;
    // Always for this file, chosen on the old page, is heard only once the new file is opening.
    d.hold("remote-images\nalways\n");
    click(d, w, "#pictures-bar", &tr("chrome-pictures-always"))?;
    let (held, always) = d.held("the old page's Always for this file")?;
    d.hold_opening();
    d.open_here(w, &new)?;
    if d.opening_held()? != w {
        return Err("another window's file was held".into());
    }
    d.release(held, always)?;
    // And while the new file is read, the old page, still shown, says the same, and that it has
    // unsaved edits (which would refuse the new page's load).
    d.eval(
        w,
        "window.ipc.postMessage('remote-images\\nalways\\n0'); \
         window.ipc.postMessage('dirty\\ntrue'); true",
    )?;
    d.finish_opening()?;
    d.until(
        w,
        "the new document, its pictures bar asking",
        &format!("{} && {}", shows("Taking over"), pictures_bar_asks()),
    )?;
    if d.title(w)?.as_deref() != Some("taking-over.md – Legible") {
        return Err(format!("the title is {:?}", d.title(w)?));
    }
    close_clean(d, w)?;
    // Neither file's pictures always show.
    for file in [&new, &old] {
        let w = d.open(file)?;
        d.until(w, "the pictures bar to ask again", &pictures_bar_asks())?;
        close_clean(d, w)?;
    }
    Ok(())
}

fn an_earlier_read_is_out_of_date(d: &Driver, docs: &Path) -> Result<()> {
    let start = document(docs, "read-start", b"# Start\n")?;
    let again = document(docs, "read-again", b"# First read\n")?;
    let between = document(docs, "read-between", b"# Between\n")?;
    let w = d.open(&start)?;
    d.until(w, "the page to start", "!!window.LegibleChrome?.ready")?;
    // The file's first read is held, and the window replaces it before it's heard.
    d.hold_opening();
    d.open_here(w, &again)?;
    d.openings_held(1)?;
    d.open_here(w, &between)?;
    d.until(w, "the file between", &shows("Between"))?;
    // The file changes, and opens again, its second read held too.
    fs::write(&again, b"# Second read\n").map_err(|e| e.to_string())?;
    d.hold_opening();
    d.open_here(w, &again)?;
    d.openings_held(2)?;
    // The first read, heard while the second is awaited, is out of date: the window waits on.
    d.finish_opening()?;
    if !d.is_opening(w)? {
        return Err("the window took the file's earlier read".into());
    }
    d.finish_opening()?;
    d.until(w, "the file as it is now", &shows("Second read"))?;
    stamp_is_the_files(d, w, &again)?;
    close_clean(d, w)
}

/// `pic.svg` beside `file`, `width` pixels wide.
fn picture_beside(file: &Path, width: u32) -> Result<()> {
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{width}\"></svg>"
    );
    fs::write(file.with_file_name("pic.svg"), svg).map_err(|e| e.to_string())
}

/// The document's picture has loaded, `width` pixels wide.
fn picture_is(width: u32) -> String {
    format!(
        "(() => {{ const p = document.querySelector('#document img'); \
         return !!p && p.complete && p.naturalWidth === {width}; }})()"
    )
}

/// The page's document shows `text`.
fn shows(text: &str) -> String {
    format!(
        "!!document.getElementById('document')?.textContent.includes({})",
        json_string(text)
    )
}

fn canonical(file: &Path) -> Result<PathBuf> {
    fs::canonicalize(file).map_err(|e| e.to_string())
}

/// `text` as `name`.md, in a folder of the case's own.
fn document(docs: &Path, name: &str, text: &[u8]) -> Result<PathBuf> {
    let dir = docs.join(name);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{name}.md"));
    fs::write(&file, text).map_err(|e| e.to_string())?;
    Ok(file)
}

/// Opens `file`, its page marked (a page served again is a new one, unmarked), in Split.
fn open_editing(d: &Driver, file: &Path) -> Result<Window> {
    let w = d.open(file)?;
    // Asked once the page's scripts have all run.
    d.until(
        w,
        "the editor to open",
        "(() => { if (!window.LegibleChrome?.ready || !window.LegibleEditor) return false; \
         if (!window.marked) { window.marked = true; LegibleEditor.setMode('split'); } \
         return !!LegibleEditor.view() && LegibleEditor.previewState().current; })()",
    )
    .map_err(|why| {
        let state = d.eval(
            w,
            "JSON.stringify({ ready: !!window.LegibleChrome?.ready, editor: !!window.LegibleEditor, \
             marked: window.marked, view: !!window.LegibleEditor?.view(), mode: window.LegibleEditor?.mode(), \
             preview: window.LegibleEditor?.previewState(), url: location.href })",
        );
        format!("{why}; the page: {state:?}")
    })?;
    Ok(w)
}

fn type_at_start(d: &Driver, w: Window, text: &str) -> Result<()> {
    d.eval(
        w,
        &format!(
            "LegibleEditor.view().dispatch({{ changes: {{ from: 0, insert: {} }} }})",
            json_string(text)
        ),
    )
    .map(drop)
}

/// Ctrl+`key` (with Shift), as the page's own keydown handlers see it.
fn press(d: &Driver, w: Window, key: &str, shift: bool) -> Result<()> {
    d.eval(
        w,
        &format!(
            "document.dispatchEvent(new KeyboardEvent('keydown', {{ key: {}, ctrlKey: true, shiftKey: {shift}, bubbles: true }}))",
            json_string(key)
        ),
    )
    .map(drop)
}

/// Clicks the button labelled `label` in `scope`.
fn click(d: &Driver, w: Window, scope: &str, label: &str) -> Result<()> {
    let clicked = d.eval(
        w,
        &format!(
            "(() => {{ const b = [...document.querySelectorAll({})].find((b) => b.textContent === {}); b?.click(); return !!b; }})()",
            json_string(&format!("{scope} button")),
            json_string(label)
        ),
    )?;
    match clicked.as_deref() {
        Some("true") => Ok(()),
        _ => Err(format!("no {label:?} button in {scope}")),
    }
}

fn pictures_bar_asks() -> String {
    format!(
        "!document.getElementById('pictures-bar').hidden \
         && [...document.querySelectorAll('#pictures-bar button')].some((b) => b.textContent === {} && !b.closest('[hidden]'))",
        json_string(&tr("chrome-pictures-once"))
    )
}

/// The Settings dialog's change of "Pictures from the internet".
fn set_remote_images(d: &Driver, w: Window, value: &str) -> Result<()> {
    set_setting(d, w, "remote-images", value)
}

/// A setting changed, as the Settings dialog sends it.
fn set_setting(d: &Driver, w: Window, key: &str, value: &str) -> Result<()> {
    d.eval(
        w,
        &format!(
            "window.ipc.postMessage({})",
            json_string(&format!("setting\n{key}\n{value}"))
        ),
    )
    .map(drop)
}

/// A file opened, edited and saved while another program wrote it between the save's check and
/// its replace: the window's save is the file, theirs is kept beside it, and the bar says so.
fn raced(d: &Driver, docs: &Path, name: &str) -> Result<(PathBuf, Window)> {
    let file = document(docs, name, OURS)?;
    let w = open_editing(d, &file)?;
    type_at_start(d, w, "Mine. ")?;
    d.until(w, "the edit", "LegibleEditor.isDirty()")?;
    let at = file.clone();
    d.at(Write::Save, Moment::Checked, move || {
        let _ = fs::write(&at, THEIRS);
    });
    press(d, w, "s", false)?;
    d.until(
        w,
        "the bar to say both versions are kept",
        "LegibleEditor.conflictState().conflict?.kind === 'raced' && LegibleEditor.conflictState().bar \
         && !LegibleEditor.isDirty()",
    )?;
    file_is(d, &file, MINE)?;
    kept_beside(&file, THEIRS)?;
    Ok((file, w))
}

/// The raced save's bar's Use the other version, and its confirmation.
fn confirm_take_theirs(d: &Driver, w: Window) -> Result<()> {
    let take = tr("chrome-conflict-take-other");
    click(d, w, "#conflict-bar", &take)?;
    d.until(
        w,
        "Use the other version to ask first",
        &format!(
            "document.querySelector('#conflict-bar .conflict-text')?.textContent === {}",
            json_string(&tr("chrome-conflict-take-raced"))
        ),
    )?;
    click(d, w, "#conflict-bar", &take)
}

fn file_is(d: &Driver, file: &Path, bytes: &[u8]) -> Result<()> {
    d.until_true(
        &format!(
            "{} to be {:?}",
            file.display(),
            String::from_utf8_lossy(bytes)
        ),
        || Ok(fs::read(file).is_ok_and(|b| b == bytes)),
    )
}

/// A file beside `file` holds `bytes`: a version Legible kept.
fn kept_beside(file: &Path, bytes: &[u8]) -> Result<()> {
    let dir = file.parent().ok_or("no folder")?;
    let kept = fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .any(|path| path != file && fs::read(&path).is_ok_and(|b| b == bytes));
    if kept {
        Ok(())
    } else {
        Err(format!(
            "no copy of {:?} beside the file",
            String::from_utf8_lossy(bytes)
        ))
    }
}

/// The window's next save expects the file exactly as it is on disk.
fn stamp_is_the_files(d: &Driver, w: Window, file: &Path) -> Result<()> {
    let (_, on_disk) = save::read(file, &win32::identity).map_err(|e| e.to_string())?;
    match d.stamp(w)? {
        Some(stamp) if stamp == on_disk => Ok(()),
        other => Err(format!(
            "the window expects {other:?}, the file is {on_disk:?}"
        )),
    }
}

/// Closes a window with nothing unsaved: it closes without asking.
fn close_clean(d: &Driver, w: Window) -> Result<()> {
    let asked = d.questions();
    d.close(w)?;
    d.until_true("the window to close", || Ok(!d.is_open(w)?))?;
    if d.questions() != asked {
        return Err("a window with nothing unsaved asked before closing".into());
    }
    Ok(())
}

/// Closes a window with unsaved edits: it asks, and Don't save closes it.
fn close_dirty(d: &Driver, w: Window) -> Result<()> {
    let asked = d.questions();
    d.answer(SaveAnswer::DontSave);
    d.close(w)?;
    d.until_true("the window to close", || Ok(!d.is_open(w)?))?;
    if d.questions() != asked + 1 {
        return Err("a window with unsaved edits closed without asking".into());
    }
    Ok(())
}

/// Types `text` at the start: unsaved now, whether or not its preview has caught up.
fn edited(d: &Driver, w: Window, text: &str) -> Result<()> {
    type_at_start(d, w, text)?;
    d.until(w, "the edit", "LegibleEditor.isDirty()")
}

/// The copy of unsaved edits kept for `file`, in Legible's folder (the run's `appdata`).
fn kept_of(docs: &Path, file: &Path) -> Option<Kept> {
    let folder = docs
        .parent()?
        .join("appdata")
        .join("Legible")
        .join(unsaved::FOLDER);
    let file = fs::canonicalize(file).ok()?;
    unsaved::list(&folder)
        .into_iter()
        .find(|kept| fs::canonicalize(&kept.document).is_ok_and(|d| d == file))
}

/// A copy of `text`, unsaved edits of `file` as it was `base`, kept in Legible's folder as an
/// earlier session's end would have kept it.
fn keep_copy(docs: &Path, file: &Path, base: &[u8], text: &str) -> Result<PathBuf> {
    let folder = docs
        .parent()
        .ok_or("no run folder")?
        .join("appdata")
        .join("Legible")
        .join(unsaved::FOLDER);
    let document = fs::canonicalize(file).unwrap_or_else(|_| file.to_owned());
    unsaved::keep(&folder, 1, &document, Some(save::hash(base)), text).map_err(|e| e.to_string())
}

/// The bar offering back the copy of `name`'s unsaved edits (`changed`: saying the file
/// changed since).
fn restore_bar(name: &str, changed: bool) -> String {
    let mut question = legible_core::tr!("restore-question", name = name.to_owned());
    if changed {
        let changed = legible_core::tr!("restore-changed", name = name.to_owned());
        question = format!("{question} {changed}");
    }
    format!(
        "LegibleEditor.conflictState().bar \
         && document.querySelector('#conflict-bar .conflict-text')?.textContent === {}",
        json_string(&question)
    )
}

/// The texts of every copy kept for `file`, oldest run first.
fn kept_texts(docs: &Path, file: &Path) -> Vec<String> {
    let Some(folder) = docs.parent() else {
        return Vec::new();
    };
    let folder = folder.join("appdata").join("Legible").join(unsaved::FOLDER);
    let Ok(file) = fs::canonicalize(file) else {
        return Vec::new();
    };
    unsaved::list(&folder)
        .into_iter()
        .filter(|kept| fs::canonicalize(&kept.document).is_ok_and(|d| d == file))
        .map(|kept| kept.text)
        .collect()
}

/// The copy kept for `file` holds `text` (None: there's none).
fn kept_is(docs: &Path, file: &Path, text: Option<&str>) -> Result<()> {
    let kept = kept_of(docs, file);
    if kept.as_ref().map(|k| k.text.as_str()) == text {
        Ok(())
    } else {
        Err(format!("the copy kept is {kept:?}, not {text:?}"))
    }
}

/// `bytes` as a JavaScript string.
fn json_text(bytes: &[u8]) -> String {
    json_string(&String::from_utf8_lossy(bytes))
}
