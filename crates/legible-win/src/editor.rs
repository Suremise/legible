//! The window's side of the editor (`web/editor/editor.js`): the messages the page sends about
//! it, and what decides whether a file may be edited at all.
//!
//! The page asks for the file's text only when the editor first opens (`source`), sends its text
//! back to be rendered for the Split preview (`render`), says when it has unsaved changes
//! (`dirty`), and answers a request for its contents (`contents`). It asks to save (`save`,
//! Ctrl+S) or save as a new file (`save-as`), answers the conflict bar (`compare`, and `raced` for a
//! save another program raced), and, frozen for a close, says whether it's clean (`clean`). It
//! answers the offer of unsaved edits kept as Windows ended the session (`restore`). The window
//! answers through `window.LegibleEditor`: `load({text, editable})`,
//! `preview(version, html, info)`, `markSaved(version)` or `saveAbandoned(version)` once a save
//! is over, `compare(id, html, info)`, `freeze()`, `closeCheck()` and `unfreeze()`, and
//! `offerRestore(question)` and `restore(text)`.
//!
//! **The page is never served again while there are unsaved edits** ([`ReloadGate`]): a reload
//! (a new CSP for pictures from the internet) waits for them to be saved. And the WebView never
//! navigates while there are ([`NavigationGuard`]), whoever asks: the window, a reload key, the
//! page itself.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A message from the page about the editor, as `chrome.js` and `editor.js` send it: lines
/// separated by `\n`, the first naming it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message<'a> {
    /// The editor opened: it wants the file's text.
    Source,
    /// The text at `version`, to render for the preview.
    Render { version: u64, text: &'a str },
    /// The editor's text differs from what was last loaded or saved (or no longer does).
    Dirty(bool),
    /// The answer to a request for the contents: `request` as it was asked, the `version` the
    /// text is, and the text.
    Contents {
        request: u64,
        version: u64,
        text: &'a str,
    },
    /// Ctrl+S.
    Save,
    /// "Save as…" (Ctrl+Shift+S, or the toolbar's Save as): the window switches to the file
    /// it writes.
    SaveAs,
    /// The frozen page's answer to `closeCheck()`: whether nothing is unsaved.
    Clean(bool),
    /// The conflict bar's Compare, for held change `id`.
    Compare(u64),
    /// The bar's answer for the save another program raced, `id`.
    Raced { answer: RaceAnswer, id: u64 },
    /// The bar's answer to the offer of unsaved edits kept as Windows ended the session:
    /// Restore (`true`) or Discard.
    Restore(bool),
    /// Restore's text is in the editor.
    Restored,
}

/// The editor's text, as the page sends it after every edit while it has unsaved changes
/// (`text\n<version>\n<text>`), or None if `body` isn't that. The window keeps it, as it comes,
/// for Windows ending the session (`crate::session`); it never reaches the event loop.
pub fn unsaved_text(body: &str) -> Option<&str> {
    let (version, text) = body.strip_prefix("text\n")?.split_once('\n')?;
    version.parse::<u64>().ok()?;
    Some(text)
}

/// What the reader chose for a save another program raced: both versions are kept until then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceAnswer {
    /// Mine stays in the file; theirs (the backup) can go.
    Keep,
    /// Theirs goes back in the file.
    Take,
    /// Theirs in the preview.
    Compare,
}

impl<'a> Message<'a> {
    /// The message in `body`, or None if it isn't one of the editor's. The text, always last,
    /// may hold any characters, line breaks included.
    pub fn parse(body: &'a str) -> Option<Message<'a>> {
        let (name, rest) = body.split_once('\n').unwrap_or((body, ""));
        let number = |s: &str| s.parse::<u64>().ok();
        Some(match name {
            "source" if rest.is_empty() => Message::Source,
            "save" if rest.is_empty() => Message::Save,
            "save-as" if rest.is_empty() => Message::SaveAs,
            "restored" if rest.is_empty() => Message::Restored,
            "clean" => Message::Clean(match rest {
                "true" => true,
                "false" => false,
                _ => return None,
            }),
            "compare" => Message::Compare(number(rest)?),
            "restore" => Message::Restore(match rest {
                "yes" => true,
                "no" => false,
                _ => return None,
            }),
            "raced" => {
                let (answer, id) = rest.split_once('\n')?;
                let answer = match answer {
                    "keep" => RaceAnswer::Keep,
                    "take" => RaceAnswer::Take,
                    "compare" => RaceAnswer::Compare,
                    _ => return None,
                };
                Message::Raced {
                    answer,
                    id: number(id)?,
                }
            }
            "dirty" => Message::Dirty(match rest {
                "true" => true,
                "false" => false,
                _ => return None,
            }),
            "render" => {
                let (version, text) = rest.split_once('\n')?;
                Message::Render {
                    version: number(version)?,
                    text,
                }
            }
            "contents" => {
                let (request, rest) = rest.split_once('\n')?;
                let (version, text) = rest.split_once('\n')?;
                Message::Contents {
                    request: number(request)?,
                    version: number(version)?,
                    text,
                }
            }
            _ => return None,
        })
    }
}

/// Whether a window's page may be served again (reloaded), given its unsaved edits: never while
/// the editor has some, since a reload would lose them. A reload wanted meanwhile waits, with
/// the newest pictures' origins, and goes ahead once the edits are saved (or undone).
#[derive(Debug, Default)]
pub struct ReloadGate {
    waiting: Option<Vec<String>>,
}

impl ReloadGate {
    /// The page is to be served again with pictures from `origins`: now (returned), or, while
    /// `dirty`, once the edits are saved.
    pub fn request(&mut self, origins: Vec<String>, dirty: bool) -> Option<Vec<String>> {
        if dirty {
            self.waiting = Some(origins);
            None
        } else {
            self.waiting = None;
            Some(origins)
        }
    }

    /// The edits are saved, or undone: a reload that waited goes ahead now.
    pub fn clean(&mut self) -> Option<Vec<String>> {
        self.waiting.take()
    }

    /// Whether a reload is waiting for the edits to be saved.
    pub fn waiting(&self) -> bool {
        self.waiting.is_some()
    }
}

/// Whether the page's editor has unsaved edits, as the WebView's own handlers see it: set from
/// the page's `dirty` message the moment it arrives (before the window's event loop takes it),
/// so a navigation that starts after it is refused. Cloning it shares the one flag.
#[derive(Debug, Clone, Default)]
pub struct NavigationGuard {
    dirty: Arc<AtomicBool>,
}

/// What a navigation of a window's page may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Navigation {
    /// Legible's own page, with nothing unsaved: it loads.
    Allowed,
    /// Legible's own page, with unsaved edits: refused, so they're kept.
    Unsaved,
    /// Anywhere else: refused, always.
    Foreign,
}

impl NavigationGuard {
    /// The page's message `body` as it arrives: a `dirty` one sets the flag (true if it did).
    pub fn note(&self, body: &str) -> bool {
        match Message::parse(body) {
            Some(Message::Dirty(dirty)) => {
                self.dirty.store(dirty, Ordering::SeqCst);
                true
            }
            _ => false,
        }
    }

    /// Whether the editor has unsaved edits.
    pub fn dirty(&self) -> bool {
        self.dirty.load(Ordering::SeqCst)
    }

    /// The window's document is replaced: its edits were saved, or dropped as the reader chose.
    pub fn clear(&self) {
        self.dirty.store(false, Ordering::SeqCst);
    }

    /// What a navigation of the page may do: `ours`, whether it's to Legible's own page.
    pub fn allows(&self, ours: bool) -> Navigation {
        match (ours, self.dirty()) {
            (false, _) => Navigation::Foreign,
            (true, true) => Navigation::Unsaved,
            (true, false) => Navigation::Allowed,
        }
    }
}

/// A window's title: `title`, with the unsaved dot while `dirty`.
pub fn window_title(title: &str, dirty: bool) -> String {
    if dirty {
        legible_core::tr!("window-title-unsaved", title = title.to_owned())
    } else {
        title.to_owned()
    }
}

/// Whether `file` (canonical) may be edited where it is: not when it's read-only, nor when it's
/// in the temp folder (`temp`, canonical) or a browser's or Outlook's cache, where an email's
/// attachment or a download opens from and an edit would be lost. Such a file offers "Save
/// as…" instead.
pub fn editable(file: &Path, read_only: bool, temp: Option<&Path>) -> bool {
    !read_only && !temp.is_some_and(|temp| file.starts_with(temp)) && !in_a_cache(file)
}

/// The folders attachments open from outside the temp folder: Outlook's (`Content.Outlook`,
/// inside `INetCache` or, on older Windows, `Temporary Internet Files`) and the browser cache.
pub(crate) const CACHES: [&str; 3] = ["Content.Outlook", "INetCache", "Temporary Internet Files"];

fn in_a_cache(file: &Path) -> bool {
    file.parent().is_some_and(|folder| {
        folder.components().any(|part| {
            CACHES
                .iter()
                .any(|cache| part.as_os_str().eq_ignore_ascii_case(cache))
        })
    })
}

/// Whether `file` may be edited, asking Windows: its read-only attribute, and the temp folder.
pub fn file_is_editable(file: &Path) -> bool {
    let read_only = std::fs::metadata(file).is_ok_and(|m| m.permissions().readonly());
    let temp = std::fs::canonicalize(std::env::temp_dir()).ok();
    editable(file, read_only, temp.as_deref())
}

/// Whether Save as starts in `file`'s own folder: not when that's the temp folder or a cache,
/// where an attachment or a download opens from (it starts in Documents instead). A read-only
/// file's folder is fine: the new file is another.
pub fn saves_beside(file: &Path) -> bool {
    let temp = std::fs::canonicalize(std::env::temp_dir()).ok();
    editable(file, false, temp.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reload::{Action, Kind, State};

    #[test]
    fn no_navigation_while_dirty_and_none_but_legibles_page_ever() {
        let guard = NavigationGuard::default();
        let shared = guard.clone();
        assert_eq!(guard.allows(true), Navigation::Allowed);
        assert_eq!(guard.allows(false), Navigation::Foreign);
        // The page's own message sets it, on the flag every clone shares.
        assert!(guard.note("dirty\ntrue"));
        assert!(shared.dirty());
        assert_eq!(shared.allows(true), Navigation::Unsaved);
        assert_eq!(shared.allows(false), Navigation::Foreign);
        // Other messages don't touch it.
        assert!(!guard.note("render\n3\ndirty\nfalse"));
        assert!(!guard.note("dirty\nmaybe"));
        assert!(shared.dirty());
        assert!(guard.note("dirty\nfalse"));
        assert_eq!(shared.allows(true), Navigation::Allowed);
    }

    #[test]
    fn a_dirty_editor_holds_every_reload_until_its_edits_are_saved() {
        let origins = |o: &[&str]| -> Vec<String> { o.iter().map(|s| (*s).to_owned()).collect() };
        let mut gate = ReloadGate::default();
        // Clean: at once.
        assert_eq!(
            gate.request(origins(&["https://a"]), false),
            Some(origins(&["https://a"]))
        );
        assert!(!gate.waiting());
        // Dirty: every reload waits, whatever asked for it (a consent, a setting, a picture
        // that couldn't be checked), the newest winning.
        assert_eq!(gate.request(origins(&["https://a"]), true), None);
        assert_eq!(gate.request(origins(&["https://b"]), true), None);
        assert!(gate.waiting());
        // Saved: it goes ahead, once.
        assert_eq!(gate.clean(), Some(origins(&["https://b"])));
        assert_eq!(gate.clean(), None);
        assert!(!gate.waiting());
        // A reload that can go now drops one still waiting.
        gate.request(origins(&["https://c"]), true);
        assert_eq!(gate.request(origins(&[]), false), Some(Vec::new()));
        assert_eq!(gate.clean(), None);
    }

    #[test]
    fn the_title_follows_unsaved_edits_whichever_side_clears_them() {
        // As the window keeps it: the title is shown from the state after every change.
        let mut state = State::default();
        let title = |state: &State| window_title("notes.md – Legible", state.is_dirty());
        assert_eq!(title(&state), "notes.md – Legible");
        // The editor says dirty: the dot.
        state.set_dirty(true);
        assert_eq!(title(&state), "● notes.md – Legible");
        // A change on disk is held, and the prompt takes theirs: clean at once, before the
        // editor's own `dirty false` arrives.
        let Action::Hold { id, .. } = state.on_disk(Kind::Changed) else {
            panic!("held while dirty");
        };
        assert_eq!(state.reload(id), Some(Kind::Changed));
        assert_eq!(title(&state), "notes.md – Legible");
        state.set_dirty(false);
        assert_eq!(title(&state), "notes.md – Legible");
    }

    #[test]
    fn the_editors_messages_parse() {
        assert_eq!(Message::parse("source"), Some(Message::Source));
        assert_eq!(Message::parse("save"), Some(Message::Save));
        assert_eq!(Message::parse("save-as"), Some(Message::SaveAs));
        assert_eq!(Message::parse("clean\ntrue"), Some(Message::Clean(true)));
        assert_eq!(Message::parse("compare\n4"), Some(Message::Compare(4)));
        assert_eq!(Message::parse("restore\nyes"), Some(Message::Restore(true)));
        assert_eq!(Message::parse("restore\nno"), Some(Message::Restore(false)));
        assert_eq!(Message::parse("restored"), Some(Message::Restored));
        assert_eq!(
            Message::parse("raced\ntake\n2"),
            Some(Message::Raced {
                answer: RaceAnswer::Take,
                id: 2
            })
        );
        assert_eq!(Message::parse("dirty\ntrue"), Some(Message::Dirty(true)));
        assert_eq!(Message::parse("dirty\nfalse"), Some(Message::Dirty(false)));
        assert_eq!(
            Message::parse("render\n7\n# A\n\ntext\n"),
            Some(Message::Render {
                version: 7,
                text: "# A\n\ntext\n"
            })
        );
        assert_eq!(
            Message::parse("render\n8\n"),
            Some(Message::Render {
                version: 8,
                text: ""
            })
        );
        assert_eq!(
            Message::parse("contents\n3\n12\nline\nline"),
            Some(Message::Contents {
                request: 3,
                version: 12,
                text: "line\nline"
            })
        );
    }

    #[test]
    fn the_unsaved_text_is_taken_whole_and_nothing_else_is() {
        assert_eq!(
            unsaved_text("text\n4\n# A\n\nline\n"),
            Some("# A\n\nline\n")
        );
        assert_eq!(unsaved_text("text\n5\n"), Some(""));
        for body in [
            "text",
            "text\n",
            "text\nx\nabc",
            "text\n4",
            "render\n4\nabc",
            "dirty\ntrue",
        ] {
            assert_eq!(unsaved_text(body), None, "{body:?}");
        }
        // Never one of the editor's other messages.
        assert_eq!(Message::parse("text\n4\nabc"), None);
    }

    #[test]
    fn anything_else_isnt_the_editors() {
        for body in [
            "",
            "open",
            "mode\nsplit",
            "source\nx",
            "dirty",
            "dirty\nyes",
            "render",
            "render\nx\ntext",
            "render\n-1\ntext",
            "render\n7",
            "contents\n1\n2",
            "contents\nx\n2\ntext",
            "save-as\nnow",
            // Save a copy is gone: Save as took its place.
            "save-copy",
            "save\nnow",
            "clean",
            "clean\nyes",
            "compare\nx",
            "raced\nmaybe\n1",
            "raced\nkeep",
            "restore",
            "restore\nmaybe",
            "restored\nnow",
        ] {
            assert_eq!(Message::parse(body), None, "{body:?}");
        }
    }

    #[test]
    fn read_only_files_and_the_temp_folders_arent_edited_in_place() {
        let temp = Path::new(r"C:\Users\a\AppData\Local\Temp");
        let doc = Path::new(r"C:\Users\a\Documents\notes.md");
        let attachment = Path::new(r"C:\Users\a\AppData\Local\Temp\Outlook\notes.md");
        assert!(editable(doc, false, Some(temp)));
        assert!(!editable(doc, true, Some(temp)));
        assert!(!editable(attachment, false, Some(temp)));
        assert!(editable(attachment, false, None));
        // Outlook's attachments and the browser's cache, wherever they are.
        for cached in [
            r"C:\Users\a\AppData\Local\Microsoft\Windows\INetCache\Content.Outlook\X1\notes.md",
            r"C:\Users\a\AppData\Local\Microsoft\Windows\inetcache\IE\notes.md",
            r"C:\Users\a\AppData\Local\Microsoft\Windows\Temporary Internet Files\Content.Outlook\notes.md",
        ] {
            assert!(!editable(Path::new(cached), false, None), "{cached}");
        }
        // A file only named like one is edited.
        assert!(editable(
            Path::new(r"C:\Users\a\Documents\INetCache.md"),
            false,
            None
        ));
        // A folder whose name only starts the same isn't inside it.
        assert!(editable(
            Path::new(r"C:\Users\a\AppData\Local\Temporary\notes.md"),
            false,
            Some(temp)
        ));
    }

    #[test]
    fn a_file_in_the_real_temp_folder_isnt_edited_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.md");
        std::fs::write(&file, "x").unwrap();
        assert!(!file_is_editable(&std::fs::canonicalize(&file).unwrap()));
    }
}
