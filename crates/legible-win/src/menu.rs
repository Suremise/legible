//! The menu bar: File, Edit, View, Tools and Help, above the page's toolbar. Windows' own menus
//! (through `muda`), so Alt, the access keys and screen readers work as in any Windows app.
//!
//! **One code path:** every item does exactly what its toolbar button or shortcut does
//! ([`Command::route`]): a message the page sends the window for it ([`Route::Window`], which
//! the shell takes as if the page had sent it), the page's own function for it
//! ([`Route::Page`], through `LegibleChrome.command`), or, for Undo, Redo and the clipboard,
//! the shortcut's own keys, sent to the window's WebView alone ([`Route::Keys`]), so the page
//! and the editor handle them as they handle the keyboard. Exit closes every window in turn
//! ([`Exit`]).
//!
//! **The shortcuts stay the page's:** each item shows its keys, but the menus register none,
//! so a key does what it always did, whether or not the bar is there.
//!
//! **States follow the window** ([`State`]): an item that doesn't apply (Undo in View, Save
//! with nothing unsaved) is disabled, never hidden, and the mode, the outline, the reading
//! width and the theme are checked as they are.

use std::path::Path;

use legible_core::i18n::tr;

use crate::jumplist;
use crate::page::{Theme, Width};

/// The editor's mode, as the page says it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    View,
    Split,
    Edit,
}

impl Mode {
    /// The mode the page names (`mode\n<view|split|edit>`).
    pub fn parse(name: &str) -> Option<Mode> {
        Some(match name {
            "view" => Mode::View,
            "split" => Mode::Split,
            "edit" => Mode::Edit,
            _ => return None,
        })
    }
}

/// What an item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Open,
    Save,
    SaveAs,
    SavePdf,
    Print,
    Close,
    Exit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Find,
    Replace,
    CopyDocument,
    Mode(Mode),
    Outline,
    StatusBar,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Width(Width),
    Theme(Theme),
    Settings,
    CheckUpdates,
    About,
}

/// Every command, in the bar's order.
pub const COMMANDS: [Command; 33] = [
    Command::Open,
    Command::Save,
    Command::SaveAs,
    Command::SavePdf,
    Command::Print,
    Command::Close,
    Command::Exit,
    Command::Undo,
    Command::Redo,
    Command::Cut,
    Command::Copy,
    Command::Paste,
    Command::SelectAll,
    Command::Find,
    Command::Replace,
    Command::CopyDocument,
    Command::Mode(Mode::View),
    Command::Mode(Mode::Split),
    Command::Mode(Mode::Edit),
    Command::Outline,
    Command::StatusBar,
    Command::ZoomIn,
    Command::ZoomOut,
    Command::ZoomReset,
    Command::Width(Width::Narrow),
    Command::Width(Width::Comfortable),
    Command::Width(Width::Wide),
    Command::Theme(Theme::System),
    Command::Theme(Theme::Light),
    Command::Theme(Theme::Dark),
    Command::Settings,
    Command::CheckUpdates,
    Command::About,
];

/// How an item does what it does: always what its toolbar button or shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The message the page sends the window for it (web/chrome/chrome.js lists them).
    Window(&'static str),
    /// The page's own function for it: `LegibleChrome.command(<name>)`.
    Page(&'static str),
    /// Its shortcut's keys, sent to the window's WebView alone.
    Keys(Keys),
    /// Every window closes in turn ([`Exit`]).
    Exit,
}

/// Ctrl and a letter key (its virtual-key code is the capital letter), and Shift or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keys {
    pub letter: char,
    pub shift: bool,
}

/// A shortcut as an item shows it: Ctrl, Shift or not, and the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub shift: bool,
    pub key: Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Letter(char),
    Digit(char),
    Plus,
    Minus,
}

const fn ctrl(letter: char) -> Option<Shortcut> {
    Some(Shortcut {
        shift: false,
        key: Key::Letter(letter),
    })
}

const fn ctrl_shift(letter: char) -> Option<Shortcut> {
    Some(Shortcut {
        shift: true,
        key: Key::Letter(letter),
    })
}

const fn keys(letter: char) -> Route {
    Route::Keys(Keys {
        letter,
        shift: false,
    })
}

impl Shortcut {
    /// As the menus show it: "Ctrl+Shift+S", "Ctrl+Plus".
    pub fn text(&self) -> String {
        let key = match self.key {
            Key::Letter(c) | Key::Digit(c) => c.to_string(),
            Key::Plus => tr("menu-key-plus"),
            Key::Minus => tr("menu-key-minus"),
        };
        let mut parts = vec![tr("menu-key-ctrl")];
        if self.shift {
            parts.push(tr("menu-key-shift"));
        }
        parts.push(key);
        parts.join("+")
    }
}

impl Command {
    /// Its label's string (with its access key).
    pub fn label(self) -> &'static str {
        match self {
            Command::Open => "menu-open",
            Command::Save => "menu-save",
            Command::SaveAs => "menu-save-as",
            Command::SavePdf => "menu-save-pdf",
            Command::Print => "menu-print",
            Command::Close => "menu-close",
            Command::Exit => "menu-exit",
            Command::Undo => "menu-undo",
            Command::Redo => "menu-redo",
            Command::Cut => "menu-cut",
            Command::Copy => "menu-copy",
            Command::Paste => "menu-paste",
            Command::SelectAll => "menu-select-all",
            Command::Find => "menu-find",
            Command::Replace => "menu-replace",
            Command::CopyDocument => "menu-copy-document",
            Command::Mode(Mode::View) => "menu-mode-view",
            Command::Mode(Mode::Split) => "menu-mode-split",
            Command::Mode(Mode::Edit) => "menu-mode-edit",
            Command::Outline => "menu-outline",
            Command::StatusBar => "menu-status-bar",
            Command::ZoomIn => "menu-zoom-in",
            Command::ZoomOut => "menu-zoom-out",
            Command::ZoomReset => "menu-zoom-reset",
            Command::Width(Width::Narrow) => "menu-width-narrow",
            Command::Width(Width::Comfortable) => "menu-width-comfortable",
            Command::Width(Width::Wide) => "menu-width-wide",
            Command::Theme(Theme::System) => "menu-theme-system",
            Command::Theme(Theme::Light) => "menu-theme-light",
            Command::Theme(Theme::Dark) => "menu-theme-dark",
            Command::Settings => "menu-settings",
            Command::CheckUpdates => "menu-check-updates",
            Command::About => "menu-about",
        }
    }

    /// The shortcut that does the same, as the page has it (chrome.js and editor.js). Ctrl+E
    /// goes round the modes, so it's none of theirs alone.
    pub fn shortcut(self) -> Option<Shortcut> {
        match self {
            Command::Open => ctrl('O'),
            Command::Save => ctrl('S'),
            Command::SaveAs => ctrl_shift('S'),
            Command::Print => ctrl('P'),
            Command::Close => ctrl('W'),
            Command::Undo => ctrl('Z'),
            Command::Redo => ctrl('Y'),
            Command::Cut => ctrl('X'),
            Command::Copy => ctrl('C'),
            Command::Paste => ctrl('V'),
            Command::SelectAll => ctrl('A'),
            Command::Find => ctrl('F'),
            Command::Replace => ctrl('H'),
            Command::CopyDocument => ctrl_shift('C'),
            Command::Outline => ctrl_shift('O'),
            Command::ZoomIn => Some(Shortcut {
                shift: false,
                key: Key::Plus,
            }),
            Command::ZoomOut => Some(Shortcut {
                shift: false,
                key: Key::Minus,
            }),
            Command::ZoomReset => Some(Shortcut {
                shift: false,
                key: Key::Digit('0'),
            }),
            Command::SavePdf
            | Command::Exit
            | Command::StatusBar
            | Command::Mode(_)
            | Command::Width(_)
            | Command::Theme(_)
            | Command::Settings
            | Command::CheckUpdates
            | Command::About => None,
        }
    }

    /// What it does: what its toolbar button or shortcut does.
    pub fn route(self) -> Route {
        match self {
            Command::Open => Route::Window("open"),
            Command::Save => Route::Window("save"),
            Command::SaveAs => Route::Window("save-as"),
            Command::SavePdf => Route::Window("save-pdf"),
            Command::Close => Route::Window("close"),
            Command::Exit => Route::Exit,
            Command::Undo => keys('Z'),
            Command::Redo => keys('Y'),
            Command::Cut => keys('X'),
            Command::Copy => keys('C'),
            Command::Paste => keys('V'),
            Command::SelectAll => keys('A'),
            Command::Print => Route::Page("print"),
            Command::Find => Route::Page("find"),
            Command::Replace => Route::Page("replace"),
            Command::CopyDocument => Route::Page("copy-document"),
            Command::Mode(Mode::View) => Route::Page("mode-view"),
            Command::Mode(Mode::Split) => Route::Page("mode-split"),
            Command::Mode(Mode::Edit) => Route::Page("mode-edit"),
            Command::Outline => Route::Page("outline"),
            Command::StatusBar => Route::Page("status-bar"),
            Command::ZoomIn => Route::Page("zoom-in"),
            Command::ZoomOut => Route::Page("zoom-out"),
            Command::ZoomReset => Route::Page("zoom-reset"),
            Command::Width(Width::Narrow) => Route::Page("width-narrow"),
            Command::Width(Width::Comfortable) => Route::Page("width-comfortable"),
            Command::Width(Width::Wide) => Route::Page("width-wide"),
            Command::Theme(Theme::System) => Route::Page("theme-system"),
            Command::Theme(Theme::Light) => Route::Page("theme-light"),
            Command::Theme(Theme::Dark) => Route::Page("theme-dark"),
            Command::Settings => Route::Page("settings"),
            // As Settings' Check now.
            Command::CheckUpdates => Route::Window("update-check"),
            Command::About => Route::Page("about"),
        }
    }

    /// Its name in an item's ID (its label's, without `menu-`).
    fn key(self) -> &'static str {
        &self.label()["menu-".len()..]
    }

    fn from_key(key: &str) -> Option<Command> {
        COMMANDS.into_iter().find(|c| c.key() == key)
    }

    /// Whether it's shown with a check, ticked or not.
    pub fn checkable(self) -> bool {
        matches!(
            self,
            Command::Mode(_)
                | Command::Outline
                | Command::StatusBar
                | Command::Width(_)
                | Command::Theme(_)
        )
    }
}

/// A window's state, as far as its menus show it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    /// It shows a document (not the empty window, nor one still reading its file).
    pub document: bool,
    /// Its file may be edited where it is (Split and Edit are offered).
    pub editable: bool,
    pub mode: Mode,
    /// The outline is shown (only ever in View).
    pub outline: bool,
    /// The editor has unsaved edits.
    pub dirty: bool,
    /// A save is under way, or waiting, or the close's own.
    pub saving: bool,
    pub theme: Theme,
    pub width: Width,
    /// The status bar shows (a setting, for every window).
    pub status_bar: bool,
    /// Settings' Check for updates is on (for every window).
    pub check_updates: bool,
}

impl State {
    /// Whether `command` applies now.
    pub fn enabled(&self, command: Command) -> bool {
        let document = self.document;
        let editing = document && self.editable && self.mode != Mode::View;
        match command {
            Command::Open
            | Command::Close
            | Command::Exit
            | Command::StatusBar
            | Command::Width(_)
            | Command::Theme(_)
            | Command::Settings
            | Command::About => true,
            // Only while the reader has it on: off, Legible goes online for nothing.
            Command::CheckUpdates => self.check_updates,
            // Nothing unsaved, or a save under way: nothing (more) to save. In any mode, as
            // Ctrl+S: edits made in Split or Edit are still unsaved back in View. A file that
            // can't be saved where it is has only Save as.
            Command::Save => document && self.editable && self.dirty && !self.saving,
            Command::Undo | Command::Redo | Command::Cut | Command::Paste | Command::Replace => {
                editing
            }
            Command::Mode(Mode::View) => document,
            Command::Mode(_) => document && self.editable,
            // The editor's text takes the outline's side in Split and Edit.
            Command::Outline => document && self.mode == Mode::View,
            Command::SaveAs
            | Command::SavePdf
            | Command::Print
            | Command::Copy
            | Command::SelectAll
            | Command::Find
            | Command::CopyDocument
            | Command::ZoomIn
            | Command::ZoomOut
            | Command::ZoomReset => document,
        }
    }

    /// Whether `command` is ticked, for those shown with a check.
    pub fn checked(&self, command: Command) -> Option<bool> {
        Some(match command {
            Command::Mode(mode) => self.document && self.mode == mode,
            Command::Outline => self.outline && self.mode == Mode::View,
            Command::StatusBar => self.status_bar,
            Command::Width(width) => self.width == width,
            Command::Theme(theme) => self.theme == theme,
            _ => return None,
        })
    }
}

/// What's on the bar: each menu's label and its entries, in order.
pub const BAR: [(&str, &[Entry]); 5] = [
    (
        "menu-file",
        &[
            Entry::Item(Command::Open),
            Entry::Recent,
            Entry::Separator,
            Entry::Item(Command::Save),
            Entry::Item(Command::SaveAs),
            Entry::Item(Command::SavePdf),
            Entry::Separator,
            Entry::Item(Command::Print),
            Entry::Separator,
            Entry::Item(Command::Close),
            Entry::Item(Command::Exit),
        ],
    ),
    (
        "menu-edit",
        &[
            Entry::Item(Command::Undo),
            Entry::Item(Command::Redo),
            Entry::Separator,
            Entry::Item(Command::Cut),
            Entry::Item(Command::Copy),
            Entry::Item(Command::Paste),
            Entry::Item(Command::SelectAll),
            Entry::Separator,
            Entry::Item(Command::Find),
            Entry::Item(Command::Replace),
            Entry::Separator,
            Entry::Item(Command::CopyDocument),
        ],
    ),
    (
        "menu-view",
        &[
            Entry::Item(Command::Mode(Mode::View)),
            Entry::Item(Command::Mode(Mode::Split)),
            Entry::Item(Command::Mode(Mode::Edit)),
            Entry::Separator,
            Entry::Item(Command::Outline),
            Entry::Item(Command::StatusBar),
            Entry::Separator,
            Entry::Item(Command::ZoomIn),
            Entry::Item(Command::ZoomOut),
            Entry::Item(Command::ZoomReset),
            Entry::Separator,
            Entry::Submenu(
                "menu-width",
                &[
                    Command::Width(Width::Narrow),
                    Command::Width(Width::Comfortable),
                    Command::Width(Width::Wide),
                ],
            ),
            Entry::Submenu(
                "menu-theme",
                &[
                    Command::Theme(Theme::System),
                    Command::Theme(Theme::Light),
                    Command::Theme(Theme::Dark),
                ],
            ),
        ],
    ),
    ("menu-tools", &[Entry::Item(Command::Settings)]),
    (
        "menu-help",
        &[
            Entry::Item(Command::CheckUpdates),
            Entry::Separator,
            Entry::Item(Command::About),
        ],
    ),
];

/// An entry in a menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Item(Command),
    Separator,
    /// A submenu of commands, with its label.
    Submenu(&'static str, &'static [Command]),
    /// Open recent: the recent files, the same list as the ⋯ menu's.
    Recent,
}

/// What an item that was picked is, from its ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picked {
    Command(Command),
    /// Recent file `index` of the list sent as `token` (as `open-recent` names it).
    Recent {
        token: u64,
        index: usize,
    },
}

/// The ID of `command`'s item in the bar numbered `bar`.
pub fn item_id(bar: u64, command: Command) -> String {
    format!("{bar}/{}", command.key())
}

/// The ID of recent file `index`, of the list with `token`, in the bar numbered `bar`.
pub fn recent_id(bar: u64, token: u64, index: usize) -> String {
    format!("{bar}/recent/{token}/{index}")
}

/// The bar an item's ID is in, and what it is.
pub fn parse_id(id: &str) -> Option<(u64, Picked)> {
    let (bar, rest) = id.split_once('/')?;
    let bar = bar.parse().ok()?;
    let picked = match rest.strip_prefix("recent/") {
        Some(recent) => {
            let (token, index) = recent.split_once('/')?;
            Picked::Recent {
                token: token.parse().ok()?,
                index: index.parse().ok()?,
            }
        }
        None => Picked::Command(Command::from_key(rest)?),
    };
    Some((bar, picked))
}

/// The longest a recent file's folder is shown, in characters, before its middle is left out.
const FOLDER_SHOWN: usize = 40;

/// A recent file's label: its number (the first nine numbered as access keys), its name, and
/// after a tab (where Windows lines it up, as Office does) its folder as Windows shows it,
/// shortened in the middle when long. An `&` shows as itself.
pub fn recent_label(index: usize, path: &Path) -> String {
    let shown = jumplist::display_path(path);
    let name = shown
        .file_name()
        .unwrap_or(shown.as_os_str())
        .to_string_lossy();
    let folder = shown.parent().map(short_folder).unwrap_or_default();
    let text = format!("{name}\t{folder}").replace('&', "&&");
    match index {
        0..9 => format!("&{} {text}", index + 1),
        _ => format!("{} {text}", index + 1),
    }
}

/// `folder` within [`FOLDER_SHOWN`] characters: its drive or share, then "…", then as many of
/// its last folders as fit (always the last one).
fn short_folder(folder: &Path) -> String {
    use std::path::Component;
    let whole = folder.to_string_lossy().into_owned();
    if whole.chars().count() <= FOLDER_SHOWN {
        return whole;
    }
    let mut root = String::new();
    let mut names = Vec::new();
    for component in folder.components() {
        match component {
            Component::Prefix(p) => root.push_str(&p.as_os_str().to_string_lossy()),
            Component::RootDir => root.push('\\'),
            other => names.push(other.as_os_str().to_string_lossy().into_owned()),
        }
    }
    let mut kept: Vec<String> = Vec::new();
    for name in names.iter().rev() {
        let length = |k: &[String]| {
            root.chars().count() + 1 + k.iter().map(|n| n.chars().count() + 1).sum::<usize>()
        };
        let mut next = vec![name.clone()];
        next.extend(kept.iter().cloned());
        if !kept.is_empty() && length(&next) > FOLDER_SHOWN {
            break;
        }
        kept = next;
    }
    format!("{root}…\\{}", kept.join("\\"))
}

/// An item's label with its shortcut after a tab, where Windows shows it.
pub fn item_text(command: Command) -> String {
    let label = tr(command.label());
    match command.shortcut() {
        Some(shortcut) => format!("{label}\t{}", shortcut.text()),
        None => label,
    }
}

/// A window's menu bar: its menus, numbered so a picked item's ID names the window, and the
/// state they last showed.
pub struct Bar {
    number: u64,
    menu: muda::Menu,
    items: Vec<(Command, Item)>,
    recent: muda::Submenu,
    shown: Option<State>,
}

enum Item {
    Plain(muda::MenuItem),
    Check(muda::CheckMenuItem),
}

impl Item {
    fn new(bar: u64, command: Command) -> Item {
        let id = item_id(bar, command);
        let text = item_text(command);
        if command.checkable() {
            Item::Check(muda::CheckMenuItem::with_id(id, text, true, false, None))
        } else {
            Item::Plain(muda::MenuItem::with_id(id, text, true, None))
        }
    }

    fn as_ref(&self) -> &dyn muda::IsMenuItem {
        match self {
            Item::Plain(item) => item,
            Item::Check(item) => item,
        }
    }
}

impl Bar {
    /// The bar numbered `number` (each window's has its own), with nothing shown yet.
    pub fn new(number: u64) -> muda::Result<Bar> {
        let menu = muda::Menu::new();
        let mut items = Vec::new();
        let recent = muda::Submenu::new(tr("menu-recent"), true);
        for (label, entries) in BAR {
            let top = muda::Submenu::new(tr(label), true);
            for entry in entries {
                match *entry {
                    Entry::Item(command) => {
                        let item = Item::new(number, command);
                        top.append(item.as_ref())?;
                        items.push((command, item));
                    }
                    Entry::Separator => top.append(&muda::PredefinedMenuItem::separator())?,
                    Entry::Submenu(label, commands) => {
                        let sub = muda::Submenu::new(tr(label), true);
                        for &command in commands {
                            let item = Item::new(number, command);
                            sub.append(item.as_ref())?;
                            items.push((command, item));
                        }
                        top.append(&sub)?;
                    }
                    Entry::Recent => top.append(&recent)?,
                }
            }
            menu.append(&top)?;
        }
        Ok(Bar {
            number,
            menu,
            items,
            recent,
            shown: None,
        })
    }

    pub fn number(&self) -> u64 {
        self.number
    }

    pub fn menu(&self) -> &muda::Menu {
        &self.menu
    }

    /// Each item enabled and ticked as `state` says, if it's changed since last shown.
    pub fn show(&mut self, state: State) {
        if self.shown == Some(state) {
            return;
        }
        for (command, item) in &self.items {
            let enabled = state.enabled(*command);
            match item {
                Item::Plain(item) => item.set_enabled(enabled),
                Item::Check(item) => {
                    item.set_enabled(enabled);
                    item.set_checked(state.checked(*command).unwrap_or(false));
                }
            }
        }
        self.shown = Some(state);
    }

    /// The commands whose items are enabled now, as the bar shows them.
    #[cfg(feature = "harness")]
    pub fn offered(&self) -> Vec<Command> {
        self.items
            .iter()
            .filter(|(_, item)| match item {
                Item::Plain(item) => item.is_enabled(),
                Item::Check(item) => item.is_enabled(),
            })
            .map(|(command, _)| *command)
            .collect()
    }

    /// An item was picked: Windows ticked or unticked it itself, so it's shown afresh next.
    pub fn picked(&mut self) {
        self.shown = None;
    }

    /// Open recent's files: `paths`, from the list with `token` (none: a disabled item says so).
    pub fn set_recent(&self, token: u64, paths: &[std::path::PathBuf]) -> muda::Result<()> {
        while self.recent.remove_at(0).is_some() {}
        if paths.is_empty() {
            let none = muda::MenuItem::new(tr("chrome-recent-none"), false, None);
            return self.recent.append(&none);
        }
        for (index, path) in paths.iter().enumerate() {
            let id = recent_id(self.number, token, index);
            let item = muda::MenuItem::with_id(id, recent_label(index, path), true, None);
            self.recent.append(&item)?;
        }
        Ok(())
    }
}

/// How a window stands in an [`Exit`], as its owner sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Closed (or never there).
    Closed,
    /// Closing: asked about, its save under way, or its page answering whether it's clean.
    Closing,
    /// Open, and not closing.
    Open,
}

/// What an [`Exit`] wants next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step<W> {
    /// Close this window, as its own Close does (it asks about unsaved edits).
    Close(W),
    /// The window it closes is still closing: ask again once something has happened.
    Wait,
    /// Every window has closed.
    Done,
    /// This window stayed open (Cancel, or a save that didn't work): the exit is over.
    Stopped(W),
}

/// File's Exit: every window closes in turn, each as its own Close does, asking about its
/// unsaved edits; the first that stays open ends it, and the windows after it aren't asked.
#[derive(Debug)]
pub struct Exit<W> {
    waiting: Vec<W>,
    current: Option<W>,
}

impl<W: Copy + Eq> Exit<W> {
    /// The windows, in the order they're to close.
    pub fn new(windows: impl IntoIterator<Item = W>) -> Self {
        let mut waiting: Vec<W> = windows.into_iter().collect();
        waiting.reverse();
        Self {
            waiting,
            current: None,
        }
    }

    /// The next step, given how each window stands now. After [`Step::Close`], call again once
    /// the close was asked for: a window that closed at once is followed by the next.
    pub fn step(&mut self, standing: impl Fn(W) -> Standing) -> Step<W> {
        if let Some(current) = self.current {
            match standing(current) {
                Standing::Closing => return Step::Wait,
                Standing::Open => {
                    self.waiting.clear();
                    self.current = None;
                    return Step::Stopped(current);
                }
                Standing::Closed => self.current = None,
            }
        }
        while let Some(next) = self.waiting.pop() {
            // One closed meanwhile (by its own Close) needs nothing.
            if standing(next) != Standing::Closed {
                self.current = Some(next);
                return Step::Close(next);
            }
        }
        Step::Done
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::saving::{Choice, Close, Session};

    #[test]
    fn every_command_does_what_its_toolbar_button_or_shortcut_does() {
        let expected = [
            (Command::Open, Route::Window("open")),
            (Command::Save, Route::Window("save")),
            (Command::SaveAs, Route::Window("save-as")),
            (Command::SavePdf, Route::Window("save-pdf")),
            (Command::Print, Route::Page("print")),
            (Command::Close, Route::Window("close")),
            (Command::Exit, Route::Exit),
            (Command::Undo, keys('Z')),
            (Command::Redo, keys('Y')),
            (Command::Cut, keys('X')),
            (Command::Copy, keys('C')),
            (Command::Paste, keys('V')),
            (Command::SelectAll, keys('A')),
            (Command::Find, Route::Page("find")),
            (Command::Replace, Route::Page("replace")),
            (Command::CopyDocument, Route::Page("copy-document")),
            (Command::Mode(Mode::View), Route::Page("mode-view")),
            (Command::Mode(Mode::Split), Route::Page("mode-split")),
            (Command::Mode(Mode::Edit), Route::Page("mode-edit")),
            (Command::Outline, Route::Page("outline")),
            (Command::StatusBar, Route::Page("status-bar")),
            (Command::ZoomIn, Route::Page("zoom-in")),
            (Command::ZoomOut, Route::Page("zoom-out")),
            (Command::ZoomReset, Route::Page("zoom-reset")),
            (Command::Width(Width::Narrow), Route::Page("width-narrow")),
            (
                Command::Width(Width::Comfortable),
                Route::Page("width-comfortable"),
            ),
            (Command::Width(Width::Wide), Route::Page("width-wide")),
            (Command::Theme(Theme::System), Route::Page("theme-system")),
            (Command::Theme(Theme::Light), Route::Page("theme-light")),
            (Command::Theme(Theme::Dark), Route::Page("theme-dark")),
            (Command::Settings, Route::Page("settings")),
            (Command::CheckUpdates, Route::Window("update-check")),
            (Command::About, Route::Page("about")),
        ];
        assert_eq!(expected.len(), COMMANDS.len());
        for (command, route) in expected {
            assert_eq!(command.route(), route, "{command:?}");
        }
        // The window messages are the ones the shell handles for the page (shell::Shell::chrome
        // and editor::Message), and each page command one chrome.js's command() knows: the
        // chrome tests run each of those (scripts/chrome-tests.mjs).
        for command in COMMANDS {
            if let Route::Window(message) = command.route() {
                let known = ["open", "save-pdf", "close", "update-check"].contains(&message)
                    || crate::editor::Message::parse(message).is_some();
                assert!(known, "{message}");
            }
        }
    }

    #[test]
    fn a_key_route_sends_the_shortcut_the_menu_shows() {
        for command in COMMANDS {
            if let Route::Keys(keys) = command.route() {
                assert_eq!(
                    command.shortcut(),
                    Some(Shortcut {
                        shift: keys.shift,
                        key: Key::Letter(keys.letter),
                    }),
                    "{command:?}"
                );
            }
        }
    }

    #[test]
    fn the_shortcuts_shown_are_the_pages() {
        let shown: Vec<(Command, String)> = COMMANDS
            .into_iter()
            .filter_map(|c| c.shortcut().map(|s| (c, s.text())))
            .collect();
        let expected = [
            (Command::Open, "Ctrl+O"),
            (Command::Save, "Ctrl+S"),
            (Command::SaveAs, "Ctrl+Shift+S"),
            (Command::Print, "Ctrl+P"),
            (Command::Close, "Ctrl+W"),
            (Command::Undo, "Ctrl+Z"),
            (Command::Redo, "Ctrl+Y"),
            (Command::Cut, "Ctrl+X"),
            (Command::Copy, "Ctrl+C"),
            (Command::Paste, "Ctrl+V"),
            (Command::SelectAll, "Ctrl+A"),
            (Command::Find, "Ctrl+F"),
            (Command::Replace, "Ctrl+H"),
            (Command::CopyDocument, "Ctrl+Shift+C"),
            (Command::Outline, "Ctrl+Shift+O"),
            (Command::ZoomIn, "Ctrl+Plus"),
            (Command::ZoomOut, "Ctrl+Minus"),
            (Command::ZoomReset, "Ctrl+0"),
        ];
        let expected: Vec<(Command, String)> =
            expected.iter().map(|(c, s)| (*c, s.to_string())).collect();
        assert_eq!(shown, expected);
        assert_eq!(item_text(Command::Open), "&Open…\tCtrl+O");
        assert_eq!(item_text(Command::Exit), "E&xit");
    }

    #[test]
    fn every_label_is_a_string_with_its_own_access_key_in_its_menu() {
        for (label, entries) in BAR {
            assert_ne!(tr(label), label);
            let mut texts = Vec::new();
            for entry in entries {
                match *entry {
                    Entry::Item(command) => texts.push(tr(command.label())),
                    Entry::Submenu(label, commands) => {
                        texts.push(tr(label));
                        let inner: Vec<String> = commands.iter().map(|c| tr(c.label())).collect();
                        assert_unique_access_keys(&inner);
                    }
                    Entry::Recent => texts.push(tr("menu-recent")),
                    Entry::Separator => {}
                }
            }
            assert_unique_access_keys(&texts);
        }
        let tops: Vec<String> = BAR.iter().map(|(label, _)| tr(label)).collect();
        assert_unique_access_keys(&tops);
    }

    fn assert_unique_access_keys(texts: &[String]) {
        let mut seen = Vec::new();
        for text in texts {
            let key = text
                .replace("&&", "")
                .split_once('&')
                .and_then(|(_, after)| after.chars().next())
                .map(|c| c.to_lowercase().to_string());
            let key = key.unwrap_or_else(|| panic!("{text} has no access key"));
            assert!(!seen.contains(&key), "{text}: {key} is taken");
            seen.push(key);
        }
    }

    #[test]
    fn every_command_is_on_the_bar_once() {
        let mut on_bar = Vec::new();
        for (_, entries) in BAR {
            for entry in entries {
                match *entry {
                    Entry::Item(command) => on_bar.push(command),
                    Entry::Submenu(_, commands) => on_bar.extend_from_slice(commands),
                    Entry::Recent | Entry::Separator => {}
                }
            }
        }
        assert_eq!(on_bar, COMMANDS);
    }

    #[test]
    fn an_items_id_says_its_bar_and_what_it_is() {
        for command in COMMANDS {
            assert_eq!(
                parse_id(&item_id(7, command)),
                Some((7, Picked::Command(command)))
            );
        }
        assert_eq!(
            parse_id(&recent_id(3, 12, 4)),
            Some((
                3,
                Picked::Recent {
                    token: 12,
                    index: 4
                }
            ))
        );
        for id in ["", "7", "x/open", "7/nothing", "7/recent/1", "7/recent/a/1"] {
            assert_eq!(parse_id(id), None, "{id}");
        }
    }

    #[test]
    fn recent_files_are_numbered_name_first_then_their_folder_shortened_in_the_middle() {
        let path = PathBuf::from(r"\\?\C:\R&D\Q&A.md");
        assert_eq!(recent_label(0, &path), "&1 Q&&A.md\tC:\\R&&D");
        assert_eq!(recent_label(8, &path), "&9 Q&&A.md\tC:\\R&&D");
        assert_eq!(recent_label(9, &path), "10 Q&&A.md\tC:\\R&&D");
        let long = PathBuf::from(r"\\?\C:\Users\Someone\AppData\Local\Temp\old-reports\docs\a.md");
        assert_eq!(
            recent_label(0, &long),
            "&1 a.md\tC:\\…\\AppData\\Local\\Temp\\old-reports\\docs"
        );
        // A share keeps its server and share; a folder too long on its own is kept whole.
        let share = PathBuf::from(r"\\?\UNC\server\team\Finance\Reports\2026\Quarterly\q3.md");
        assert_eq!(
            recent_label(0, &share),
            "&1 q3.md\t\\\\server\\team\\…\\Reports\\2026\\Quarterly"
        );
        let deep = PathBuf::from(format!(r"C:\a\{}\b.md", "x".repeat(50)));
        assert_eq!(
            recent_label(0, &deep),
            format!("&1 b.md\tC:\\…\\{}", "x".repeat(50))
        );
    }

    fn reading() -> State {
        State {
            document: true,
            editable: true,
            ..State::default()
        }
    }

    fn enabled(state: &State) -> Vec<Command> {
        COMMANDS.into_iter().filter(|c| state.enabled(*c)).collect()
    }

    #[test]
    fn view_offers_reading_and_no_editing() {
        let state = reading();
        for command in [
            Command::Undo,
            Command::Redo,
            Command::Cut,
            Command::Paste,
            Command::Replace,
            Command::Save,
        ] {
            assert!(!state.enabled(command), "{command:?}");
        }
        for command in [
            Command::Copy,
            Command::SelectAll,
            Command::Find,
            Command::Outline,
            Command::Mode(Mode::Split),
            Command::Mode(Mode::Edit),
            Command::SaveAs,
            Command::Print,
        ] {
            assert!(state.enabled(command), "{command:?}");
        }
        assert_eq!(state.checked(Command::Mode(Mode::View)), Some(true));
        assert_eq!(state.checked(Command::Mode(Mode::Split)), Some(false));
        assert_eq!(state.checked(Command::Outline), Some(false));
        let outline = State {
            outline: true,
            ..state
        };
        assert_eq!(outline.checked(Command::Outline), Some(true));
        assert_eq!(state.checked(Command::Open), None);
    }

    #[test]
    fn split_and_edit_offer_editing_and_no_outline() {
        for mode in [Mode::Split, Mode::Edit] {
            let state = State {
                mode,
                outline: true,
                ..reading()
            };
            for command in [
                Command::Undo,
                Command::Redo,
                Command::Cut,
                Command::Copy,
                Command::Paste,
                Command::SelectAll,
                Command::Find,
                Command::Replace,
            ] {
                assert!(state.enabled(command), "{mode:?} {command:?}");
            }
            assert!(!state.enabled(Command::Outline), "{mode:?}");
            // The outline keeps its state for View; it's shown unticked meanwhile.
            assert_eq!(state.checked(Command::Outline), Some(false));
            for m in [Mode::View, Mode::Split, Mode::Edit] {
                assert_eq!(state.checked(Command::Mode(m)), Some(m == mode));
            }
        }
    }

    #[test]
    fn save_is_offered_only_with_unsaved_edits_and_none_being_saved() {
        let split = State {
            mode: Mode::Split,
            ..reading()
        };
        assert!(!split.enabled(Command::Save));
        let dirty = State {
            dirty: true,
            ..split
        };
        assert!(dirty.enabled(Command::Save));
        let saving = State {
            saving: true,
            ..dirty
        };
        assert!(!saving.enabled(Command::Save));
        // Edited in Split, then back in View: still unsaved, and Save saves them, as Ctrl+S does.
        let back_in_view = State {
            mode: Mode::View,
            ..dirty
        };
        assert!(back_in_view.enabled(Command::Save));
        // Save as is there throughout.
        for state in [split, dirty, saving, back_in_view] {
            assert!(state.enabled(Command::SaveAs));
        }
    }

    #[test]
    fn a_file_that_cant_be_edited_offers_view_and_save_as() {
        let state = State {
            editable: false,
            ..reading()
        };
        assert!(state.enabled(Command::Mode(Mode::View)));
        assert!(!state.enabled(Command::Mode(Mode::Split)));
        assert!(!state.enabled(Command::Mode(Mode::Edit)));
        assert!(!state.enabled(Command::Save));
        assert!(!state.enabled(Command::Replace));
        assert!(state.enabled(Command::SaveAs));
        assert!(state.enabled(Command::CopyDocument));
    }

    #[test]
    fn the_empty_window_offers_what_needs_no_document() {
        let state = State::default();
        assert_eq!(
            enabled(&state),
            [
                Command::Open,
                Command::Close,
                Command::Exit,
                Command::StatusBar,
                Command::Width(Width::Narrow),
                Command::Width(Width::Comfortable),
                Command::Width(Width::Wide),
                Command::Theme(Theme::System),
                Command::Theme(Theme::Light),
                Command::Theme(Theme::Dark),
                Command::Settings,
                Command::About,
            ]
        );
        assert_eq!(state.checked(Command::Mode(Mode::View)), Some(false));
    }

    #[test]
    fn check_for_updates_is_offered_only_while_it_is_on() {
        for state in [State::default(), reading()] {
            assert!(!state.enabled(Command::CheckUpdates));
            assert_eq!(state.checked(Command::CheckUpdates), None);
            let on = State {
                check_updates: true,
                ..state
            };
            assert!(on.enabled(Command::CheckUpdates));
        }
    }

    #[test]
    fn the_status_bar_is_ticked_as_set_in_any_window() {
        for state in [State::default(), reading()] {
            assert!(state.enabled(Command::StatusBar));
            assert_eq!(state.checked(Command::StatusBar), Some(false));
            let on = State {
                status_bar: true,
                ..state
            };
            assert_eq!(on.checked(Command::StatusBar), Some(true));
        }
    }

    #[test]
    fn the_width_and_theme_are_ticked_as_set() {
        let state = State {
            width: Width::Wide,
            theme: Theme::Dark,
            ..reading()
        };
        for width in [Width::Narrow, Width::Comfortable, Width::Wide] {
            assert_eq!(
                state.checked(Command::Width(width)),
                Some(width == Width::Wide)
            );
        }
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            assert_eq!(
                state.checked(Command::Theme(theme)),
                Some(theme == Theme::Dark)
            );
        }
    }

    /// Windows closing as the shell closes them: each with its own [`Session`], asking the
    /// reader (answers given in turn) as the shell's close does.
    struct Windows {
        sessions: HashMap<u8, (Session, bool)>,
        answers: Vec<Choice>,
        asked: Vec<u8>,
        /// Saves under way, by window: done once `finish_saves` is called.
        saving: Vec<u8>,
    }

    impl Windows {
        fn new(windows: &[(u8, bool)], answers: &[Choice]) -> Self {
            Self {
                sessions: windows
                    .iter()
                    .map(|&(w, dirty)| (w, (Session::default(), dirty)))
                    .collect(),
                answers: answers.iter().rev().copied().collect(),
                asked: Vec::new(),
                saving: Vec::new(),
            }
        }

        fn standing(&self, w: u8) -> Standing {
            match self.sessions.get(&w) {
                None => Standing::Closed,
                Some((session, _)) if session.closing() => Standing::Closing,
                Some(_) => Standing::Open,
            }
        }

        /// As `Shell::ask_close`, then `carry_out`.
        fn close(&mut self, w: u8) {
            let (session, dirty) = self.sessions.get_mut(&w).unwrap();
            let mut close = session.close(*dirty);
            loop {
                close = match close {
                    Close::Now => {
                        self.sessions.remove(&w);
                        return;
                    }
                    Close::Ask => {
                        self.asked.push(w);
                        let answer = self.answers.pop().expect("an answer");
                        self.sessions.get_mut(&w).unwrap().0.choose(answer)
                    }
                    Close::FreezeAndSave => {
                        let session = &mut self.sessions.get_mut(&w).unwrap().0;
                        let save = crate::saving::Save {
                            version: 1,
                            text: String::new(),
                        };
                        assert!(session.save(save).is_some());
                        self.saving.push(w);
                        return;
                    }
                    // Cancel: the window stays.
                    Close::Stay => return,
                    other => panic!("{other:?}"),
                };
            }
        }

        /// The saves under way are written, and each frozen page says it's clean.
        fn finish_saves(&mut self) {
            for w in std::mem::take(&mut self.saving) {
                let (session, dirty) = self.sessions.get_mut(&w).unwrap();
                let done = session.done(1, crate::saving::Outcome::Saved);
                assert_eq!(done.close, Close::CheckClean);
                *dirty = false;
                if session.clean(true) == Close::Now {
                    self.sessions.remove(&w);
                }
            }
        }
    }

    /// Runs `exit` as the shell does: a step after each thing that happens.
    fn run(exit: &mut Exit<u8>, windows: &RefCell<Windows>) -> Step<u8> {
        loop {
            let step = exit.step(|w| windows.borrow().standing(w));
            match step {
                Step::Close(w) => windows.borrow_mut().close(w),
                other => return other,
            }
        }
    }

    #[test]
    fn exit_with_two_dirty_windows_asks_twice_and_stops_on_cancel() {
        let windows = RefCell::new(Windows::new(
            &[(1, true), (2, true), (3, true)],
            &[Choice::DontSave, Choice::Cancel],
        ));
        let mut exit = Exit::new([1, 2, 3]);
        assert_eq!(run(&mut exit, &windows), Step::Stopped(2));
        let windows = windows.into_inner();
        assert_eq!(windows.asked, [1, 2]);
        // The first closed; the second stayed, and the third was never asked.
        assert_eq!(windows.standing(1), Standing::Closed);
        assert_eq!(windows.standing(2), Standing::Open);
        assert_eq!(windows.standing(3), Standing::Open);
        // Over: nothing more is asked.
        assert_eq!(exit.step(|_| Standing::Open), Step::Done);
    }

    #[test]
    fn exit_closes_clean_windows_at_once_and_waits_for_a_save() {
        let windows = RefCell::new(Windows::new(
            &[(1, false), (2, true), (3, false)],
            &[Choice::Save],
        ));
        let mut exit = Exit::new([1, 2, 3]);
        assert_eq!(run(&mut exit, &windows), Step::Wait);
        assert_eq!(windows.borrow().standing(1), Standing::Closed);
        assert_eq!(windows.borrow().standing(2), Standing::Closing);
        // Nothing else is asked while its save is under way.
        assert_eq!(run(&mut exit, &windows), Step::Wait);
        windows.borrow_mut().finish_saves();
        assert_eq!(run(&mut exit, &windows), Step::Done);
        let windows = windows.into_inner();
        assert_eq!(windows.asked, [2]);
        assert!(windows.sessions.is_empty());
    }

    #[test]
    fn exit_skips_a_window_closed_meanwhile() {
        let windows = RefCell::new(Windows::new(&[(1, false), (2, false)], &[]));
        let mut exit = Exit::new([1, 2]);
        windows.borrow_mut().sessions.remove(&1);
        assert_eq!(run(&mut exit, &windows), Step::Done);
        assert!(windows.into_inner().sessions.is_empty());
    }
}
