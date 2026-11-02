//! Screenshots of the reading chrome in a real WebView2, for checking it by eye.
//!
//! `cargo run -p legible-win --example ui-shots -- [set] [out dir]` opens a window of its own
//! on the showcase (`scripts/doc-shots/showcase.md`) and takes each shot of a set with
//! WebView2's `CapturePreview`, into `target/ui-shots/<set>` by default:
//!
//! - `reading` (the default), the reading chrome: the outline open, find active, 150% zoom, Wide
//!   and Serif, in light and dark, at 100% and 150% scaling (WebView2's rasterization scale),
//!   and Settings and About;
//! - `reload`, live reload: the "Updated just now" note and the moved-or-deleted bar, in light
//!   and dark;
//! - `editor`, the editor: Split (scrolled to the middle, the document in step) and Edit, in
//!   light and dark, at 100% and 150%; and spell-check, misspelt words marked, with the menu
//!   WebView2 offers for one written beside the shot (`spellcheck-<theme>-menu.txt`: its items'
//!   names and labels; WebView2 gives its suggestions as `spellcheck` items with no label). The
//!   window's side of the editor is stood in for: the text asked for is the showcase's, and
//!   each preview is rendered through core; and, with an unsaved change, the pictures bar
//!   saying to save first (`save-first-<theme>`);
//! - `export`, the toasts after Copy and Save as PDF, in light and dark;
//! - `saving`, saving: the conflict bar in Split with an unsaved change (the file changed on
//!   disk), its Use the file confirmation, Compare (the file in the preview), a save raced by
//!   another write, and a file that went, in light and dark, and the bar at 150%;
//! - `restore`, unsaved changes kept as Windows ended the session, offered back at the next
//!   start: the bar in light and dark and at 150%, saying too that the file changed since, and
//!   Restore chosen (the text in Split, unsaved), in light and dark;
//! - `comfort`, Settings' Comfort group (Ctrl + mouse wheel zoom, windows where they were, the
//!   reading place), in light and dark and at 150%;
//! - `recent`, recent files: the ⋯ menu open on them, in light and dark;
//! - `pictures`, pictures from the internet: the bar that asks, and the bar saying why none can
//!   show, in light and dark;
//! - `default-app`, the default app: the bar offering it, in light and dark and at 150%, and
//!   Settings' Default app section for an installed copy and for a portable one not yet
//!   registered;
//! - `updates`, the update check: a newer release's bar, in light and dark and at 150%, and
//!   Settings' Updates section off, and on with what the last check found;
//! - `divider`, Split's divider: idle, hovered and dragged, in light and dark, at 100% and 150%;
//! - `about`, About Legible: in light and dark at 100% and 150%, and its reading view (the
//!   release notes, light and dark, and the third-party notices);
//! - `status-bar`, the status bar: off, on in View (a file from an email attachment) and on in
//!   Edit, in light and dark, at 100% and 150%, and on with View and Edit scrolled to the very
//!   end;
//! - `modes`, the toolbar's View, Split and Edit as icons: each selected, Split hovered (each
//!   mode's name and tooltip written beside the shot, `<shot>-tooltips.txt`, since a tooltip
//!   isn't part of what `CapturePreview` captures) and the same on a file that can't be edited
//!   where it is, in light and dark at 100% and 150%.
//!
//! It drives only its own WebView, by running script in its page, and takes each shot once the
//! page says it's ready (the chrome ready, then the shot's own condition), never after a fixed
//! wait. The window doesn't take the focus. Spell-check's right-click, the divider's hover and
//! drag and the Split button's hover are the only input, and they go to this WebView alone,
//! through its DevTools protocol
//! (`Input.dispatchMouseEvent`), with focus emulated so the page spell-checks as if focused;
//! WebView2's `ContextMenuRequested` gives the menu's items and is marked handled, so no menu
//! opens on the desktop (a native menu isn't part of what `CapturePreview` captures anyway).

#[cfg(windows)]
#[allow(unsafe_code)]
fn main() {
    shots::main();
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[allow(unsafe_code)]
mod shots {
    use std::borrow::Cow;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use legible_core::assets;
    use legible_core::render::{RenderOptions, render, render_with};
    use legible_win::editor::Message;
    use legible_win::page;
    use legible_win::settings::{Setting, Settings};
    use tao::dpi::PhysicalSize;
    use tao::event::{Event, StartCause};
    use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
    use tao::window::WindowBuilder;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, ICoreWebView2_11,
        ICoreWebView2ContextMenuRequestedEventArgs, ICoreWebView2Controller3,
    };
    use webview2_com::{
        CallDevToolsProtocolMethodCompletedHandler, CapturePreviewCompletedHandler, CoTaskMemPWSTR,
        ContextMenuRequestedEventHandler,
    };
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    use windows::Win32::System::Com::{STGM_CREATE, STGM_WRITE};
    use windows::Win32::UI::Shell::SHCreateStreamOnFileEx;
    use windows::core::{HSTRING, Interface, PWSTR};
    use wry::http::{Request, Response, header};
    use wry::{WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

    /// The page's width and height at 100%, in device-independent pixels.
    const WIDTH: f64 = 1100.0;
    const HEIGHT: f64 = 820.0;

    /// The reading view: the outline open (the showcase is long), find on "quarter", 150%.
    const READING: &str = r#"
        for (let i = 0; i < 3; i++)
          document.dispatchEvent(new KeyboardEvent("keydown", { key: "=", ctrlKey: true, bubbles: true }));
        document.dispatchEvent(new KeyboardEvent("keydown", { key: "f", ctrlKey: true, bubbles: true }));
        const field = document.querySelector(".find-field");
        field.value = "quarter";
        field.dispatchEvent(new Event("input"));
        document.querySelector(".find-count").textContent.includes("of") &&
          document.querySelector(".zoom-level").textContent === "150%" &&
          !document.getElementById("outline").hidden"#;
    const SETTINGS: &str = r#"
        [...document.querySelectorAll(".menu-item")].find((m) => m.textContent.includes("Settings")).click();
        !!document.querySelector("dialog[open]")"#;
    /// Settings, scrolled to its Comfort group (Ctrl+wheel zoom, windows where they were, the
    /// reading place).
    const COMFORT: &str = r#"
        [...document.querySelectorAll(".menu-item")].find((m) => m.textContent.includes("Settings")).click();
        document.querySelector("dialog[open] fieldset.comfort").scrollIntoView({ block: "center" });
        !!document.querySelector("dialog[open] fieldset.comfort")"#;
    /// About, its table the facts this example answers with, as the window would.
    const ABOUT: &str = r#"
        [...document.querySelectorAll(".menu-item")].find((m) => m.textContent.includes("About")).click();
        document.querySelectorAll("dialog.about .about-table tr").length === 4
          && document.querySelector(".about-icon").naturalWidth > 0"#;
    /// About's Release notes: the bundled changelog in the reading view, over About.
    const RELEASE_NOTES: &str = r#"
        [...document.querySelectorAll(".menu-item")].find((m) => m.textContent.includes("About")).click();
        [...document.querySelectorAll(".about-button")].find((b) => b.textContent === "Release notes").click();
        document.querySelector(".reading-doc h1")?.textContent === "Changelog""#;
    /// About's third-party software link: the notices in the reading view.
    const NOTICES: &str = r#"
        [...document.querySelectorAll(".menu-item")].find((m) => m.textContent.includes("About")).click();
        document.querySelectorAll(".about-licence .link")[1].click();
        document.querySelector(".reading-text").textContent.length > 1000"#;

    struct Shot {
        name: &'static str,
        dark: bool,
        scale: f64,
        reading: bool,
        script: &'static str,
        /// Right-click the misspelt word before the shot, and write the menu offered for it.
        spell: bool,
        /// What the pointer does before the shot.
        pointer: Pointer,
        /// The file can't be edited where it is: Split and Edit unusable.
        read_only: bool,
    }

    /// The pointer: nothing, over Split's divider or dragging it, or over the toolbar's Split
    /// button.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Pointer {
        None,
        Hover,
        Drag,
        Tool,
    }

    /// Split, its text scrolled to the middle: ready once the document has followed and
    /// stopped (the same place on two polls running).
    const SPLIT: &str = r#"
        LegibleEditor.setMode("split");
        window.shotStep = () => {
          const view = LegibleEditor.view();
          if (LegibleEditor.mode() !== "split" || !view || !LegibleEditor.previewState().prepared) return false;
          if (!window.scrolled) {
            window.scrolled = true;
            view.scrollDOM.scrollTop = (view.scrollDOM.scrollHeight - view.scrollDOM.clientHeight) / 2;
            return false;
          }
          const at = document.getElementById("document").scrollTop + "," + view.scrollDOM.scrollTop;
          const still = at === window.lastAt;
          window.lastAt = at;
          return still && document.getElementById("document").scrollTop > 0;
        };
        window.shotStep()"#;
    const EDIT: &str = r#"
        LegibleEditor.setMode("edit");
        window.shotStep = () => LegibleEditor.mode() === "edit" && !!LegibleEditor.view();
        window.shotStep()"#;
    /// Edit, with misspelt words typed into the first paragraph, and the text left.
    const SPELL: &str = r#"
        LegibleEditor.setMode("edit");
        window.shotStep = () => {
          const view = LegibleEditor.view();
          if (LegibleEditor.mode() !== "edit" || !view) return false;
          if (!window.typed) {
            window.typed = true;
            const at = view.state.doc.toString().indexOf("This review covers");
            view.dispatch({ changes: { from: at, insert: "Please recieve the notes we discused. " } });
            view.contentDOM.blur();
            return false;
          }
          return document.activeElement !== document.getElementById("editor");
        };
        window.shotStep()"#;
    /// Takes the focus off the text: true once it's off.
    const UNFOCUS: &str = r#"(() => {
        LegibleEditor.view().contentDOM.blur();
        return document.activeElement !== document.getElementById("editor");
      })()"#;
    /// Split with an unsaved change, and the pictures bar asking about pictures from the internet:
    /// it says to save first, since showing them reloads the page.
    const SAVE_FIRST: &str = r#"
        LegibleEditor.setMode("split");
        window.shotStep = () => {
          const view = LegibleEditor.view();
          if (LegibleEditor.mode() !== "split" || !view || !LegibleEditor.previewState().prepared) return false;
          if (!window.typed) {
            window.typed = true;
            const at = view.state.doc.toString().indexOf("This review covers");
            view.dispatch({ changes: { from: at, insert: "Unsaved. " } });
            LegibleChrome.remoteImages({ generation: 0, show: [], ask: true });
            return false;
          }
          const bar = document.getElementById("pictures-bar");
          return !bar.hidden && bar.textContent.includes("Save your changes") && LegibleEditor.isDirty();
        };
        window.shotStep()"#;
    /// Where the misspelt word is, in CSS pixels from the top left of the page.
    const MISSPELT_AT: &str = r#"(() => {
        const view = LegibleEditor.view();
        const at = view.coordsAtPos(view.state.doc.toString().indexOf("recieve") + 3);
        return [(at.left + at.right) / 2, (at.top + at.bottom) / 2];
      })()"#;

    const SHOTS_EDITOR: &[Shot] = &[
        Shot::of("split-light-100", false, 1.0, SPLIT),
        Shot::of("split-dark-100", true, 1.0, SPLIT),
        Shot::of("split-light-150", false, 1.5, SPLIT),
        Shot::of("split-dark-150", true, 1.5, SPLIT),
        Shot::of("edit-light-100", false, 1.0, EDIT),
        Shot::of("edit-dark-100", true, 1.0, EDIT),
        Shot::of("edit-light-150", false, 1.5, EDIT),
        Shot::of("edit-dark-150", true, 1.5, EDIT),
        Shot {
            spell: true,
            ..Shot::of("spellcheck-light", false, 1.0, SPELL)
        },
        Shot {
            spell: true,
            ..Shot::of("spellcheck-dark", true, 1.0, SPELL)
        },
        Shot::of("save-first-light", false, 1.0, SAVE_FIRST),
        Shot::of("save-first-dark", true, 1.0, SAVE_FIRST),
    ];

    /// Split with an unsaved change, then `then` (script, given `view`), ready once `ready` (an
    /// expression) holds and the preview is the text's.
    macro_rules! conflict {
        ($then:literal, $ready:literal) => {
            concat!(
                r#"
        LegibleEditor.setMode("split");
        window.shotStep = () => {
          const view = LegibleEditor.view();
          if (LegibleEditor.mode() !== "split" || !view || !LegibleEditor.previewState().prepared) return false;
          if (!window.typed) {
            window.typed = true;
            const at = view.state.doc.toString().indexOf("This review covers");
            view.dispatch({ changes: { from: at, insert: "My unsaved change. " } });
            return false;
          }
          if (!window.then) {
            if (!LegibleEditor.previewState().current) return false;
            window.then = true;
            view.contentDOM.blur();
            "#,
                $then,
                r#"
            return false;
          }
          return "#,
                $ready,
                r#";
        };
        window.shotStep()"#
            )
        };
    }
    const CHANGED: &str = conflict!(
        r#"window.dispatchEvent(new CustomEvent("legible:external-change", { detail: { kind: "changed", id: 1, text: "showcase.md changed while you were editing it." } }));"#,
        r#"LegibleEditor.conflictState().bar"#
    );
    const CONFIRM: &str = conflict!(
        r#"window.dispatchEvent(new CustomEvent("legible:external-change", { detail: { kind: "changed", id: 1, text: "showcase.md changed while you were editing it." } }));
            [...document.querySelectorAll('#conflict-bar button')].find((b) => b.textContent === "Use the file").click();"#,
        r#"document.querySelector('#conflict-bar .conflict-text').textContent.startsWith("Use the file as it is now")"#
    );
    /// The file: the document with its first heading and paragraph changed, as something else
    /// might have saved it.
    const COMPARE: &str = conflict!(
        r#"window.dispatchEvent(new CustomEvent("legible:external-change", { detail: { kind: "changed", id: 1, text: "showcase.md changed while you were editing it." } }));
            const theirs = document.createElement("div");
            theirs.innerHTML = document.getElementById("document").innerHTML;
            theirs.querySelector("h1").textContent += " (revised)";
            const p = [...theirs.querySelectorAll("p")].find((p) => p.textContent.includes("My unsaved change."));
            if (p) p.textContent = p.textContent.replace("My unsaved change. ", "A change saved meanwhile. ");
            LegibleEditor.compare("changed", 1, theirs.innerHTML, { generation: 0, remote: null })
              .then((shown) => (window.compared = shown));"#,
        r#"window.compared === true"#
    );
    const RACED: &str = conflict!(
        r#"LegibleEditor.markSaved(LegibleEditor.contents().version);
            window.dispatchEvent(new CustomEvent("legible:external-change", { detail: { kind: "raced", id: 1,
              text: 'showcase.md was saved at the same moment you saved. Both are kept: your changes are in showcase.md, and the other version is in the copy "showcase (other version).md".' } }));"#,
        r#"LegibleEditor.conflictState().bar && !LegibleEditor.isDirty()"#
    );
    const DELETED: &str = conflict!(
        r#"window.dispatchEvent(new CustomEvent("legible:external-change", { detail: { kind: "deleted", id: 1, text: "showcase.md was moved or deleted. Keep your changes to save it again, or close it." } }));"#,
        r#"LegibleEditor.conflictState().bar"#
    );

    /// The bar offering back unsaved changes kept as Windows ended the session, as the window
    /// sends it at the next start: over the document, in View.
    const RESTORE: &str = r#"
        window.scrollTo(0, 0);
        LegibleEditor.offerRestore("Restore unsaved changes to showcase.md?");
        LegibleEditor.conflictState().bar"#;
    const RESTORE_CHANGED: &str = r#"
        window.scrollTo(0, 0);
        LegibleEditor.offerRestore("Restore unsaved changes to showcase.md? showcase.md has changed since then.");
        LegibleEditor.conflictState().bar"#;
    /// Restore chosen: the kept text (the showcase with a change) in the editor, in Split,
    /// unsaved, ready once the preview is drawn.
    const RESTORED: &str = r#"
        LegibleEditor.setMode("split");
        window.shotStep = () => {
          const view = LegibleEditor.view();
          if (!view) return false;
          if (!window.restoring) {
            window.restoring = true;
            const text = view.state.doc.toString();
            const at = text.indexOf("This review covers");
            void LegibleEditor.restore(text.slice(0, at) + "My unsaved change. " + text.slice(at));
            return false;
          }
          if (!LegibleEditor.isDirty() || !LegibleEditor.previewState().current) return false;
          view.contentDOM.blur();
          return true;
        };
        window.shotStep()"#;

    /// Split, the text unfocused, ready once the preview is drawn.
    const DIVIDER: &str = r#"
        LegibleEditor.setMode("split");
        window.shotStep = () => {
          const view = LegibleEditor.view();
          if (LegibleEditor.mode() !== "split" || !view || !LegibleEditor.previewState().prepared) return false;
          view.contentDOM.blur();
          return document.activeElement !== document.getElementById("editor");
        };
        window.shotStep()"#;
    /// The middle of Split's divider, in CSS pixels from the top left of the page.
    const DIVIDER_AT: &str = r#"(() => {
        const r = document.querySelector(".editor-divider").getBoundingClientRect();
        return [r.left + r.width / 2, r.top + r.height / 2];
      })()"#;
    /// Whether the divider shows its handle (hovered or dragged), and whether it's being dragged.
    const HANDLE_SHOWN: &str =
        r#"getComputedStyle(document.querySelector(".editor-divider"), "::after").opacity === "1""#;
    const DRAGGING: &str =
        r#"document.querySelector(".editor-divider").classList.contains("dragging")"#;

    /// Split's divider: idle, hovered and dragged, in light and dark at 100% and 150%.
    const SHOTS_DIVIDER: &[Shot] = &[
        Shot::of("divider-idle-light-100", false, 1.0, DIVIDER),
        Shot::pointing("divider-hover-light-100", false, 1.0, Pointer::Hover),
        Shot::pointing("divider-drag-light-100", false, 1.0, Pointer::Drag),
        Shot::of("divider-idle-dark-100", true, 1.0, DIVIDER),
        Shot::pointing("divider-hover-dark-100", true, 1.0, Pointer::Hover),
        Shot::pointing("divider-drag-dark-100", true, 1.0, Pointer::Drag),
        Shot::of("divider-idle-light-150", false, 1.5, DIVIDER),
        Shot::pointing("divider-hover-light-150", false, 1.5, Pointer::Hover),
        Shot::pointing("divider-drag-light-150", false, 1.5, Pointer::Drag),
        Shot::of("divider-idle-dark-150", true, 1.5, DIVIDER),
        Shot::pointing("divider-hover-dark-150", true, 1.5, Pointer::Hover),
        Shot::pointing("divider-drag-dark-150", true, 1.5, Pointer::Drag),
    ];

    /// View, its button pressed.
    const MODE_VIEW: &str = r#"
        window.scrollTo(0, 0);
        LegibleEditor.mode() === "view" && document.querySelector('.segmented [data-mode="view"]').getAttribute("aria-pressed") === "true""#;
    /// Edit, the text unfocused.
    const MODE_EDIT: &str = r#"
        LegibleEditor.setMode("edit");
        window.shotStep = () => {
          const view = LegibleEditor.view();
          if (LegibleEditor.mode() !== "edit" || !view) return false;
          view.contentDOM.blur();
          return document.activeElement !== document.getElementById("editor");
        };
        window.shotStep()"#;
    /// The middle of the toolbar's Split button, in CSS pixels from the top left of the page.
    const TOOL_AT: &str = r#"(() => {
        const r = document.querySelector('.segmented [data-mode="split"]').getBoundingClientRect();
        return [r.left + r.width / 2, r.top + r.height / 2];
      })()"#;
    const TOOL_HOVERED: &str =
        r#"document.querySelector('.segmented [data-mode="split"]').matches(":hover")"#;
    /// Each mode button's name and tooltip, a line each: CapturePreview doesn't capture the
    /// tooltip a hover shows, so it's written beside the shot.
    const TOOLTIPS: &str = r#"[...document.querySelectorAll(".segmented button")]
        .map((b) => `${b.getAttribute("aria-label")}: ${b.title}`).join("\n")"#;

    /// The toolbar's View, Split and Edit as icons: each selected, Split hovered (the
    /// tooltips written beside the shot), and a file that can't be edited where it is, in light
    /// and dark at 100% and 150%.
    const SHOTS_MODES: &[Shot] = &[
        Shot::of("view-light-100", false, 1.0, MODE_VIEW),
        Shot::of("split-light-100", false, 1.0, DIVIDER),
        Shot::of("edit-light-100", false, 1.0, MODE_EDIT),
        Shot::of("view-dark-100", true, 1.0, MODE_VIEW),
        Shot::of("split-dark-100", true, 1.0, DIVIDER),
        Shot::of("edit-dark-100", true, 1.0, MODE_EDIT),
        Shot::of("view-light-150", false, 1.5, MODE_VIEW),
        Shot::of("split-light-150", false, 1.5, DIVIDER),
        Shot::of("edit-light-150", false, 1.5, MODE_EDIT),
        Shot::of("view-dark-150", true, 1.5, MODE_VIEW),
        Shot::of("split-dark-150", true, 1.5, DIVIDER),
        Shot::of("edit-dark-150", true, 1.5, MODE_EDIT),
        Shot::hovering("hover-light-100", false, 1.0, false),
        Shot::hovering("hover-dark-100", true, 1.0, false),
        Shot::hovering("hover-light-150", false, 1.5, false),
        Shot::hovering("hover-dark-150", true, 1.5, false),
        Shot::hovering("read-only-light-100", false, 1.0, true),
        Shot::hovering("read-only-dark-100", true, 1.0, true),
        Shot::hovering("read-only-light-150", false, 1.5, true),
        Shot::hovering("read-only-dark-150", true, 1.5, true),
    ];

    /// The status bar off: the page as it is by default.
    const STATUS_OFF: &str = r#"
        window.scrollTo(0, 0);
        document.getElementById("status-bar").hidden"#;
    /// The status bar on, in View, for a file opened from an email attachment, as a window
    /// would tell it.
    const STATUS_VIEW: &str = r#"
        window.scrollTo(0, 0);
        LegibleChrome.command("status-bar");
        LegibleChrome.status({ path: "C:\\Users\\Sam\\AppData\\Local\\Microsoft\\Windows\\INetCache\\Content.Outlook\\Q7RT2K\\Quarterly operations review.md",
          note: "Opened from an email attachment", encoding: null, endings: null });
        !document.getElementById("status-bar").hidden && document.querySelector(".status-path").textContent !== ""
          && getComputedStyle(document.body).paddingBottom !== "0px""#;
    /// The status bar on, in Edit: the words, encoding and line endings.
    const STATUS_EDIT: &str = r#"
        LegibleChrome.command("status-bar");
        LegibleChrome.status({ path: "C:\\Users\\Sam\\Documents\\Reports\\Quarterly operations review.md",
          note: null, encoding: "UTF-8", endings: "CRLF" });
        LegibleEditor.setMode("edit");
        window.shotStep = () => {
          const view = LegibleEditor.view();
          if (LegibleEditor.mode() !== "edit" || !view) return false;
          view.contentDOM.blur();
          return document.activeElement !== document.getElementById("editor")
            && !document.querySelector(".status-edit").hidden
            && document.querySelector(".status-words").textContent !== "";
        };
        window.shotStep()"#;

    /// The status bar on in View, the page scrolled to its very end: its last line above the bar.
    const STATUS_VIEW_END: &str = r#"
        LegibleChrome.command("status-bar");
        LegibleChrome.status({ path: "C:\\Users\\Sam\\Documents\\Reports\\Quarterly operations review.md",
          note: null, encoding: null, endings: null });
        window.shotStep = () => {
          if (document.getElementById("status-bar").hidden || getComputedStyle(document.body).paddingBottom === "0px") return false;
          const end = document.documentElement.scrollHeight - innerHeight;
          window.scrollTo(0, end);
          const at = String(window.scrollY);
          const still = at === window.lastAt && Math.abs(window.scrollY - end) < 1;
          window.lastAt = at;
          return still;
        };
        window.shotStep()"#;
    /// The status bar on in Edit, the text scrolled to its very end.
    const STATUS_EDIT_END: &str = r#"
        LegibleChrome.command("status-bar");
        LegibleChrome.status({ path: "C:\\Users\\Sam\\Documents\\Reports\\Quarterly operations review.md",
          note: null, encoding: "UTF-8", endings: "CRLF" });
        LegibleEditor.setMode("edit");
        window.shotStep = () => {
          const view = LegibleEditor.view();
          if (LegibleEditor.mode() !== "edit" || !view || document.querySelector(".status-edit").hidden) return false;
          view.contentDOM.blur();
          const s = view.scrollDOM;
          s.scrollTop = s.scrollHeight;
          const at = String(s.scrollTop);
          const still = at === window.lastAt && s.scrollTop + s.clientHeight >= s.scrollHeight - 1;
          window.lastAt = at;
          return still && document.activeElement !== document.getElementById("editor");
        };
        window.shotStep()"#;

    /// The status bar: off, on in View, on in Edit, in light and dark at 100% and 150%;
    /// and on, scrolled to the very end, in View and Edit, light and dark.
    const SHOTS_STATUS_BAR: &[Shot] = &[
        Shot::of("status-off-light-100", false, 1.0, STATUS_OFF),
        Shot::of("status-view-light-100", false, 1.0, STATUS_VIEW),
        Shot::of("status-edit-light-100", false, 1.0, STATUS_EDIT),
        Shot::of("status-off-dark-100", true, 1.0, STATUS_OFF),
        Shot::of("status-view-dark-100", true, 1.0, STATUS_VIEW),
        Shot::of("status-edit-dark-100", true, 1.0, STATUS_EDIT),
        Shot::of("status-off-light-150", false, 1.5, STATUS_OFF),
        Shot::of("status-view-light-150", false, 1.5, STATUS_VIEW),
        Shot::of("status-edit-light-150", false, 1.5, STATUS_EDIT),
        Shot::of("status-off-dark-150", true, 1.5, STATUS_OFF),
        Shot::of("status-view-dark-150", true, 1.5, STATUS_VIEW),
        Shot::of("status-edit-dark-150", true, 1.5, STATUS_EDIT),
        Shot::of("status-view-end-light-100", false, 1.0, STATUS_VIEW_END),
        Shot::of("status-view-end-dark-100", true, 1.0, STATUS_VIEW_END),
        Shot::of("status-edit-end-light-100", false, 1.0, STATUS_EDIT_END),
        Shot::of("status-edit-end-dark-100", true, 1.0, STATUS_EDIT_END),
    ];

    /// About Legible: light and dark, 100% and 150%, and its reading view.
    const SHOTS_ABOUT: &[Shot] = &[
        Shot::of("about-light-100", false, 1.0, ABOUT),
        Shot::of("about-dark-100", true, 1.0, ABOUT),
        Shot::of("about-light-150", false, 1.5, ABOUT),
        Shot::of("about-dark-150", true, 1.5, ABOUT),
        Shot::of("release-notes-light-100", false, 1.0, RELEASE_NOTES),
        Shot::of("release-notes-dark-100", true, 1.0, RELEASE_NOTES),
        Shot::of("notices-light-100", false, 1.0, NOTICES),
    ];

    const SHOTS_SAVING: &[Shot] = &[
        Shot::of("changed-light", false, 1.0, CHANGED),
        Shot::of("changed-dark", true, 1.0, CHANGED),
        Shot::of("changed-light-150", false, 1.5, CHANGED),
        Shot::of("confirm-light", false, 1.0, CONFIRM),
        Shot::of("confirm-dark", true, 1.0, CONFIRM),
        Shot::of("compare-light", false, 1.0, COMPARE),
        Shot::of("compare-dark", true, 1.0, COMPARE),
        Shot::of("raced-light", false, 1.0, RACED),
        Shot::of("raced-dark", true, 1.0, RACED),
        Shot::of("deleted-light", false, 1.0, DELETED),
        Shot::of("deleted-dark", true, 1.0, DELETED),
    ];

    const SHOTS_COMFORT: &[Shot] = &[
        Shot::of("comfort-light", false, 1.0, COMFORT),
        Shot::of("comfort-dark", true, 1.0, COMFORT),
        Shot::of("comfort-light-150", false, 1.5, COMFORT),
    ];

    const SHOTS_RESTORE: &[Shot] = &[
        Shot::of("restore-light", false, 1.0, RESTORE),
        Shot::of("restore-dark", true, 1.0, RESTORE),
        Shot::of("restore-light-150", false, 1.5, RESTORE),
        Shot::of("changed-light", false, 1.0, RESTORE_CHANGED),
        Shot::of("changed-dark", true, 1.0, RESTORE_CHANGED),
        Shot::of("restored-light", false, 1.0, RESTORED),
        Shot::of("restored-dark", true, 1.0, RESTORED),
    ];

    impl Shot {
        const fn of(name: &'static str, dark: bool, scale: f64, script: &'static str) -> Shot {
            Shot {
                name,
                dark,
                scale,
                reading: false,
                script,
                spell: false,
                pointer: Pointer::None,
                read_only: false,
            }
        }

        /// Split, with the pointer doing `pointer` to its divider.
        const fn pointing(name: &'static str, dark: bool, scale: f64, pointer: Pointer) -> Shot {
            Shot {
                pointer,
                ..Shot::of(name, dark, scale, DIVIDER)
            }
        }

        /// View, with the pointer over the toolbar's Split button, on a file that can be edited
        /// or (`read_only`) can't.
        const fn hovering(name: &'static str, dark: bool, scale: f64, read_only: bool) -> Shot {
            Shot {
                pointer: Pointer::Tool,
                read_only,
                ..Shot::of(name, dark, scale, MODE_VIEW)
            }
        }
    }

    /// Live reload's replacement, as the window sends it, with the same document.
    const UPDATED: &str = r#"
        window.scrollTo(0, 0);
        LegibleChrome.replaceDocument({ generation: 1, html: window.originalHtml });
        !document.getElementById("updated").hidden"#;
    const GONE: &str = r#"
        LegibleChrome.fileGone(true);
        !document.getElementById("file-bar").hidden"#;

    /// The ⋯ menu open on recent files, as a window would send them.
    const RECENT: &str = r#"
        LegibleChrome.setRecent({ token: 1, items: [
          { name: "Quarterly plan.md", path: "C:\\Users\\Sam\\Documents\\Quarterly plan.md" },
          { name: "Meeting notes.md", path: "C:\\Users\\Sam\\Documents\\Meeting notes.md" },
          { name: "README.md", path: "D:\\Projects\\Handbook\\README.md" },
          { name: "README.md", path: "D:\\Projects\\Website\\README.md" } ] });
        document.querySelector('[aria-label="More"]').click();
        [...document.querySelectorAll(".menu:not([hidden]) .menu-item")].some((m) => m.textContent === "Meeting notes.md")"#;

    /// The bar asking about the showcase's picture from the internet, and the bar saying why
    /// none can show, as a window would send them.
    const PICTURES_ASK: &str = r#"
        window.scrollTo(0, 0);
        LegibleChrome.remoteImages({ generation: 0, show: [], ask: true });
        !document.getElementById("pictures-bar").hidden"#;
    const PICTURES_FAILED: &str = r#"
        window.scrollTo(0, 0);
        LegibleChrome.remoteImages({ generation: 0, show: [], ask: false, unavailable: "failed" });
        document.getElementById("pictures-bar").textContent.includes("stopped")"#;

    const SHOTS_PICTURES: &[Shot] = &[
        Shot {
            name: "pictures-ask-light",
            dark: false,
            scale: 1.0,
            reading: false,
            script: PICTURES_ASK,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "pictures-ask-dark",
            dark: true,
            scale: 1.0,
            reading: false,
            script: PICTURES_ASK,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "pictures-failed-light",
            dark: false,
            scale: 1.0,
            reading: false,
            script: PICTURES_FAILED,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "pictures-failed-dark",
            dark: true,
            scale: 1.0,
            reading: false,
            script: PICTURES_FAILED,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
    ];

    /// The bar offering to make Legible the default, and Settings' section, as a window would
    /// say what's so.
    const DEFAULT_BAR: &str = r#"
        window.scrollTo(0, 0);
        LegibleChrome.defaultApp({ offer: true, registered: true, isDefault: false, portable: false });
        !document.getElementById("default-bar").hidden"#;
    const DEFAULT_SETTINGS: &str = r#"
        [...document.querySelectorAll(".menu-item")].find((m) => m.textContent.includes("Settings")).click();
        LegibleChrome.defaultApp({ offer: true, registered: true, isDefault: false, portable: false });
        document.querySelector(".default-app").scrollIntoView({ block: "end" });
        !!document.querySelector("dialog[open]")"#;
    const DEFAULT_PORTABLE: &str = r#"
        [...document.querySelectorAll(".menu-item")].find((m) => m.textContent.includes("Settings")).click();
        LegibleChrome.defaultApp({ offer: false, registered: false, isDefault: false, portable: true });
        document.querySelector(".default-app").scrollIntoView({ block: "end" });
        !!document.querySelector("dialog[open]")"#;

    const UPDATE_BAR: &str = r#"
        window.scrollTo(0, 0);
        LegibleChrome.updates({ on: true, checking: false, status: "Legible 0.4.0 is available.", bar: "Legible 0.4.0 is available." });
        !document.getElementById("update-bar").hidden"#;
    const UPDATE_SETTINGS_OFF: &str = r#"
        [...document.querySelectorAll(".menu-item")].find((m) => m.textContent.includes("Settings")).click();
        document.getElementById("settings-tab-general").click();
        document.querySelector(".updates").scrollIntoView({ block: "end" });
        !!document.querySelector("dialog[open]")"#;
    const UPDATE_SETTINGS: &str = r#"
        LegibleChrome.apply({ ...JSON.parse(document.getElementById("legible-settings").textContent), "check-updates": true });
        [...document.querySelectorAll(".menu-item")].find((m) => m.textContent.includes("Settings")).click();
        document.getElementById("settings-tab-general").click();
        LegibleChrome.updates({ on: true, checking: false, status: "Legible 0.3.0 is the latest version.", bar: "" });
        document.querySelector(".updates").scrollIntoView({ block: "end" });
        !!document.querySelector("dialog[open]")"#;

    const SHOTS_UPDATES: &[Shot] = &[
        Shot {
            name: "update-bar-light-100",
            dark: false,
            scale: 1.0,
            reading: false,
            script: UPDATE_BAR,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "update-bar-dark-100",
            dark: true,
            scale: 1.0,
            reading: false,
            script: UPDATE_BAR,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "update-bar-light-150",
            dark: false,
            scale: 1.5,
            reading: false,
            script: UPDATE_BAR,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "update-settings-off-light-100",
            dark: false,
            scale: 1.0,
            reading: false,
            script: UPDATE_SETTINGS_OFF,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "update-settings-light-100",
            dark: false,
            scale: 1.0,
            reading: false,
            script: UPDATE_SETTINGS,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "update-settings-dark-100",
            dark: true,
            scale: 1.0,
            reading: false,
            script: UPDATE_SETTINGS,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "update-settings-light-150",
            dark: false,
            scale: 1.5,
            reading: false,
            script: UPDATE_SETTINGS,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
    ];

    const SHOTS_DEFAULT_APP: &[Shot] = &[
        Shot {
            name: "default-bar-light-100",
            dark: false,
            scale: 1.0,
            reading: false,
            script: DEFAULT_BAR,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "default-bar-dark-100",
            dark: true,
            scale: 1.0,
            reading: false,
            script: DEFAULT_BAR,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "default-bar-light-150",
            dark: false,
            scale: 1.5,
            reading: false,
            script: DEFAULT_BAR,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "default-settings-light-100",
            dark: false,
            scale: 1.0,
            reading: false,
            script: DEFAULT_SETTINGS,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "default-portable-dark-100",
            dark: true,
            scale: 1.0,
            reading: false,
            script: DEFAULT_PORTABLE,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
    ];

    /// The toasts after Copy and Save as PDF, as a window answers them.
    const COPIED: &str = r#"
        window.scrollTo(0, 0);
        document.querySelector('[aria-label="Copy"]').click();
        LegibleChrome.copied({ id: 1, generation: 0, outcome: "copied" });
        !document.getElementById("toast").hidden"#;
    const SAVED: &str = r#"
        window.scrollTo(0, 0);
        LegibleChrome.pdfSaved({ name: "showcase.pdf" });
        document.querySelectorAll('#toast button').length === 2"#;

    const SHOTS_EXPORT: &[Shot] = &[
        Shot {
            name: "copied-light",
            dark: false,
            scale: 1.0,
            reading: false,
            script: COPIED,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "copied-dark",
            dark: true,
            scale: 1.0,
            reading: false,
            script: COPIED,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "saved-light",
            dark: false,
            scale: 1.0,
            reading: false,
            script: SAVED,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "saved-dark",
            dark: true,
            scale: 1.0,
            reading: false,
            script: SAVED,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
    ];

    const SHOTS_RECENT: &[Shot] = &[
        Shot {
            name: "recent-light",
            dark: false,
            scale: 1.0,
            reading: false,
            script: RECENT,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "recent-dark",
            dark: true,
            scale: 1.0,
            reading: false,
            script: RECENT,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
    ];

    const SHOTS_RELOAD: &[Shot] = &[
        Shot {
            name: "updated-light",
            dark: false,
            scale: 1.0,
            reading: false,
            script: UPDATED,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "updated-dark",
            dark: true,
            scale: 1.0,
            reading: false,
            script: UPDATED,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "gone-light",
            dark: false,
            scale: 1.0,
            reading: false,
            script: GONE,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "gone-dark",
            dark: true,
            scale: 1.0,
            reading: false,
            script: GONE,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
    ];

    const SHOTS: &[Shot] = &[
        Shot {
            name: "reading-light-100",
            dark: false,
            scale: 1.0,
            reading: true,
            script: READING,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "reading-dark-100",
            dark: true,
            scale: 1.0,
            reading: true,
            script: READING,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "reading-light-150",
            dark: false,
            scale: 1.5,
            reading: true,
            script: READING,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "reading-dark-150",
            dark: true,
            scale: 1.5,
            reading: true,
            script: READING,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "settings-light-100",
            dark: false,
            scale: 1.0,
            reading: false,
            script: SETTINGS,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "settings-dark-100",
            dark: true,
            scale: 1.0,
            reading: false,
            script: SETTINGS,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "settings-light-150",
            dark: false,
            scale: 1.5,
            reading: false,
            script: SETTINGS,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "about-light-100",
            dark: false,
            scale: 1.0,
            reading: false,
            script: ABOUT,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "about-dark-100",
            dark: true,
            scale: 1.0,
            reading: false,
            script: ABOUT,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
        Shot {
            name: "about-dark-150",
            dark: true,
            scale: 1.5,
            reading: false,
            script: ABOUT,
            spell: false,
            pointer: Pointer::None,
            read_only: false,
        },
    ];

    enum Step {
        /// Waiting for the page to load and the chrome to be ready.
        Loading,
        /// The shot's script ran; waiting for its condition.
        Setting,
        /// The misspelt word was right-clicked; waiting for the menu WebView2 offered.
        Menu,
        /// Waiting for the text to lose the focus: then the word is right-clicked again
        /// (`retry`, its words not checked yet), or the shot is taken.
        Unfocusing { retry: bool },
        /// The pointer went over the divider, or pressed it and moved: waiting for its handle.
        Pointing,
        /// CapturePreview is writing the file.
        Capturing,
    }

    enum Ev {
        /// What the page answered to a poll: true when the step's condition holds.
        Answer(bool),
        Captured(Result<(), String>),
        /// A message from the page (the editor's, answered as the window would).
        Ipc(String),
        /// Where the misspelt word is, as the page answered.
        Misspelt(String),
        /// The context menu's items (name: label), in order.
        Menu(Vec<String>),
        /// Where to point (the divider, or the Split button), as the page answered.
        Pointer(String),
    }

    pub fn main() {
        // A window of its own: quiet, so it never takes the user's focus, and checked
        // never to.
        legible_win::quiet::enable();
        let _focus = legible_win::quiet::watch_focus();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        let set = std::env::args().nth(1).unwrap_or_else(|| "reading".into());
        let shots: &'static [Shot] = match set.as_str() {
            "reading" => SHOTS,
            "reload" => SHOTS_RELOAD,
            "editor" => SHOTS_EDITOR,
            "export" => SHOTS_EXPORT,
            "saving" => SHOTS_SAVING,
            "restore" => SHOTS_RESTORE,
            "comfort" => SHOTS_COMFORT,
            "recent" => SHOTS_RECENT,
            "pictures" => SHOTS_PICTURES,
            "default-app" => SHOTS_DEFAULT_APP,
            "updates" => SHOTS_UPDATES,
            "divider" => SHOTS_DIVIDER,
            "status-bar" => SHOTS_STATUS_BAR,
            "about" => SHOTS_ABOUT,
            "modes" => SHOTS_MODES,
            other => panic!("no set of shots called {other}"),
        };
        let out = std::env::args()
            .nth(2)
            .map(PathBuf::from)
            .unwrap_or_else(|| repo.join("target").join("ui-shots").join(&set));
        std::fs::create_dir_all(&out).expect("the output folder");
        legible_core::i18n::init("en");
        let docs = repo.join("scripts").join("doc-shots");
        let markdown = std::fs::read_to_string(docs.join("showcase.md")).expect("the showcase");
        let document = render(&markdown, &docs).html;

        let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        let window = legible_win::quiet::window(
            WindowBuilder::new().with_title("Legible UI shots"),
            &event_loop,
        )
        .with_inner_size(PhysicalSize::new(WIDTH as u32, HEIGHT as u32))
        .build(&event_loop)
        .expect("a window");

        let html: Arc<Mutex<Vec<u8>>> = Arc::default();
        let served = html.clone();
        let doc_folder = docs.clone();
        let messages = proxy.clone();
        // The document, for live reload's replacement.
        let notices = format!(
            "window.originalHtml = {};",
            legible_win::settings::json_string(&document)
        );
        // WebView2's data, in a temp folder of the example's own, cleared at each start (its
        // browser processes outlive the example by a moment, so it isn't removed at the end).
        let data = std::env::temp_dir().join("legible-ui-shots");
        let _ = std::fs::remove_dir_all(&data);
        let mut context = WebContext::new(Some(data));
        let webview = WebViewBuilder::new_with_web_context(&mut context)
            .with_focused(false)
            .with_https_scheme(true)
            .with_custom_protocol(
                page::APP_PROTOCOL.into(),
                move |_, request: Request<Vec<u8>>| {
                    let path = request.uri().path().to_owned();
                    if path == "/" {
                        let body = served.lock().unwrap().clone();
                        return Response::builder()
                            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                            .header("Content-Security-Policy", page::CSP)
                            .body(Cow::Owned(body))
                            .unwrap();
                    }
                    match assets::get(&path) {
                        Some(asset) => Response::builder()
                            .header(header::CONTENT_TYPE, asset.content_type)
                            .body(Cow::Borrowed(asset.bytes))
                            .unwrap(),
                        None => Response::builder()
                            .status(404)
                            .body(Cow::Borrowed(&[][..]))
                            .unwrap(),
                    }
                },
            )
            .with_custom_protocol(
                page::DOC_PROTOCOL.into(),
                move |_, request: Request<Vec<u8>>| {
                    let name = request.uri().path().trim_start_matches('/').to_owned();
                    match std::fs::read(doc_folder.join(&name)) {
                        Ok(bytes) if name.ends_with(".svg") && !name.contains("..") => {
                            Response::builder()
                                .header(header::CONTENT_TYPE, "image/svg+xml")
                                .body(Cow::Owned(bytes))
                                .unwrap()
                        }
                        _ => Response::builder()
                            .status(404)
                            .body(Cow::Borrowed(&[][..]))
                            .unwrap(),
                    }
                },
            )
            .with_initialization_script(&notices)
            .with_ipc_handler(move |request| {
                let _ = messages.send_event(Ev::Ipc(request.body().clone()));
            })
            .with_hotkeys_zoom(false)
            .build(&window)
            .expect("a WebView");
        on_context_menu(&webview, proxy.clone());
        // Spell-check runs as if the page had the focus, which the window never takes.
        devtools(
            &webview,
            "Emulation.setFocusEmulationEnabled",
            r#"{"enabled":true}"#,
        );

        let mut index = 0;
        let mut step = Step::Loading;
        let mut deadline = Instant::now();
        let mut failures = 0;
        let mut next_poll = Instant::now();

        let load = move |index: usize, webview: &WebView, window: &tao::window::Window| {
            let shot = &shots[index];
            let mut settings = Settings::default();
            settings.set(Setting::Theme(if shot.dark {
                legible_win::page::Theme::Dark
            } else {
                legible_win::page::Theme::Light
            }));
            if shot.reading {
                settings.set(Setting::Width(legible_win::page::Width::Wide));
                settings.set(Setting::Serif(true));
            }
            *html.lock().unwrap() = page::document_page(
                "en",
                "showcase.md – Legible",
                &document,
                &settings,
                0,
                &[],
                !shot.read_only,
            )
            .into_bytes();
            set_scale(webview, shot.scale);
            window.set_inner_size(PhysicalSize::new(
                (WIDTH * shot.scale).round() as u32,
                (HEIGHT * shot.scale).round() as u32,
            ));
            webview
                .load_url(&format!("{}/?shot={index}", page::APP_ORIGIN))
                .expect("load the page");
        };

        event_loop.run(move |event, _, flow| {
            *flow = ControlFlow::WaitUntil(next_poll);
            match event {
                Event::NewEvents(StartCause::Init) => {
                    load(0, &webview, &window);
                    deadline = Instant::now() + Duration::from_secs(30);
                    next_poll = Instant::now() + Duration::from_millis(100);
                }
                Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                    if Instant::now() > deadline {
                        eprintln!("{}: timed out", shots[index].name);
                        failures += 1;
                        index += 1;
                        if index == shots.len() {
                            std::process::exit(1);
                        }
                        step = Step::Loading;
                        load(index, &webview, &window);
                        deadline = Instant::now() + Duration::from_secs(30);
                    }
                    let condition = match step {
                        Step::Loading => {
                            "(() => { if (!window.LegibleChrome?.ready) return false; \
                             if (!window.shotReady) { window.shotReady = 'waiting'; \
                             LegibleChrome.ready.then(() => document.fonts.ready).then(() => (window.shotReady = 'yes')); } \
                             return window.shotReady === 'yes'; })()"
                                .to_string()
                        }
                        Step::Setting => format!("(() => {{ {} }})()", condition_only(shots[index].script)),
                        Step::Unfocusing { .. } => UNFOCUS.to_string(),
                        Step::Pointing if shots[index].pointer == Pointer::Drag => {
                            format!("{HANDLE_SHOWN} && {DRAGGING}")
                        }
                        Step::Pointing if shots[index].pointer == Pointer::Tool => {
                            TOOL_HOVERED.to_string()
                        }
                        Step::Pointing => HANDLE_SHOWN.to_string(),
                        Step::Capturing | Step::Menu => String::new(),
                    };
                    if !condition.is_empty() {
                        let proxy = proxy.clone();
                        let _ = webview.evaluate_script_with_callback(&condition, move |answer| {
                            let _ = proxy.send_event(Ev::Answer(answer.trim() == "true"));
                        });
                    }
                    next_poll = Instant::now() + Duration::from_millis(100);
                }
                Event::UserEvent(Ev::Answer(true)) => match step {
                    Step::Loading => {
                        let _ = webview.evaluate_script(&action_only(shots[index].script));
                        step = Step::Setting;
                    }
                    Step::Setting | Step::Unfocusing { retry: true } if shots[index].spell => {
                        step = Step::Menu;
                        let proxy = proxy.clone();
                        let _ = webview.evaluate_script_with_callback(MISSPELT_AT, move |at| {
                            let _ = proxy.send_event(Ev::Misspelt(at));
                        });
                    }
                    Step::Setting if shots[index].pointer != Pointer::None => {
                        step = Step::Pointing;
                        let proxy = proxy.clone();
                        let at = if shots[index].pointer == Pointer::Tool {
                            TOOL_AT
                        } else {
                            DIVIDER_AT
                        };
                        let _ = webview.evaluate_script_with_callback(at, move |at| {
                            let _ = proxy.send_event(Ev::Pointer(at));
                        });
                    }
                    Step::Setting | Step::Pointing => {
                        // The condition is checked on a poll after the script ran, so what it
                        // checked has been painted.
                        step = Step::Capturing;
                        if shots[index].pointer == Pointer::Tool {
                            let file = out.join(format!("{}-tooltips.txt", shots[index].name));
                            let _ = webview.evaluate_script_with_callback(TOOLTIPS, move |tips| {
                                // A JSON string; the tooltips have no backslashes of their own.
                                let tips = tips.trim().trim_matches('"').replace("\\n", "\n");
                                let tips = tips.replace("\\\"", "\"");
                                std::fs::write(&file, tips + "\n").expect("the tooltips");
                            });
                        }
                        capture(&webview, &out.join(format!("{}.png", shots[index].name)), proxy.clone());
                    }
                    Step::Unfocusing { .. } => {
                        step = Step::Capturing;
                        let name = shots[index].name;
                        capture(&webview, &out.join(format!("{name}.png")), proxy.clone());
                    }
                    Step::Capturing | Step::Menu => {}
                },
                Event::UserEvent(Ev::Answer(false)) => {}
                Event::UserEvent(Ev::Ipc(body)) => {
                    if let Some(answer) = answer_editor(&body, &markdown, &docs) {
                        let _ = webview.evaluate_script(&answer);
                    }
                }
                Event::UserEvent(Ev::Misspelt(at)) => {
                    let at: Vec<f64> = at
                        .trim_matches(['[', ']'])
                        .split(',')
                        .filter_map(|n| n.trim().parse().ok())
                        .collect();
                    let [x, y] = at[..] else {
                        eprintln!("{}: the misspelt word wasn't found", shots[index].name);
                        std::process::exit(1);
                    };
                    for kind in ["mousePressed", "mouseReleased"] {
                        let params = format!(
                            r#"{{"type":"{kind}","x":{x},"y":{y},"button":"right","buttons":2,"clickCount":1}}"#
                        );
                        devtools(&webview, "Input.dispatchMouseEvent", &params);
                    }
                }
                Event::UserEvent(Ev::Pointer(at)) => {
                    let at: Vec<f64> = at
                        .trim_matches(['[', ']'])
                        .split(',')
                        .filter_map(|n| n.trim().parse().ok())
                        .collect();
                    let [x, y] = at[..] else {
                        eprintln!("{}: what to point at wasn't found", shots[index].name);
                        std::process::exit(1);
                    };
                    // Over it; or pressed on the divider and moved a little left, still held.
                    let moves: &[(&str, f64, u8)] = match shots[index].pointer {
                        Pointer::Drag => &[("mousePressed", 0.0, 1), ("mouseMoved", -60.0, 1)],
                        _ => &[("mouseMoved", 0.0, 0)],
                    };
                    for (kind, dx, buttons) in moves {
                        let params = format!(
                            r#"{{"type":"{kind}","x":{},"y":{y},"button":"left","buttons":{buttons},"clickCount":1}}"#,
                            x + dx
                        );
                        devtools(&webview, "Input.dispatchMouseEvent", &params);
                    }
                }
                Event::UserEvent(Ev::Menu(items)) => {
                    if !matches!(step, Step::Menu) {
                        return;
                    }
                    // Chromium checks the words of an editable element once it loses the focus:
                    // until it has, the menu has no suggestions, and the word is tried again
                    // after the focus is off (the shot's deadline bounds it).
                    let checked = items.iter().any(|item| item.starts_with("spellcheck:"));
                    if checked {
                        let name = shots[index].name;
                        let file = out.join(format!("{name}-menu.txt"));
                        std::fs::write(&file, items.join("\n") + "\n").expect("the menu's items");
                    }
                    // The right-click put the focus back in the text: off it again, so its
                    // marks show in the shot.
                    step = Step::Unfocusing { retry: !checked };
                }
                Event::UserEvent(Ev::Captured(result)) => {
                    if !matches!(step, Step::Capturing) {
                        return;
                    }
                    // Let the divider go, and take the pointer off it, before the next shot.
                    if shots[index].pointer != Pointer::None {
                        if shots[index].pointer == Pointer::Drag {
                            let release = r#"{"type":"mouseReleased","x":5,"y":5,"button":"left","buttons":0,"clickCount":1}"#;
                            devtools(&webview, "Input.dispatchMouseEvent", release);
                        }
                        let away = r#"{"type":"mouseMoved","x":5,"y":5,"button":"none","buttons":0}"#;
                        devtools(&webview, "Input.dispatchMouseEvent", away);
                    }
                    match result {
                        Ok(()) => println!("  {}", shots[index].name),
                        Err(error) => {
                            eprintln!("{}: {error}", shots[index].name);
                            failures += 1;
                        }
                    }
                    index += 1;
                    if index == shots.len() {
                        if let Some(why) = legible_win::quiet::focus_taken() {
                            println!("  FAIL  {why}");
                            failures += 1;
                        }
                        println!("Wrote {} shots to {}", shots.len() - failures, out.display());
                        std::process::exit(if failures == 0 { 0 } else { 1 });
                    }
                    step = Step::Loading;
                    load(index, &webview, &window);
                    deadline = Instant::now() + Duration::from_secs(30);
                }
                _ => {}
            }
        });
    }

    /// The window's answer to the editor's message `body`, as script for the page: the
    /// showcase's text when it's asked for, and each preview rendered through core.
    fn answer_editor(body: &str, markdown: &str, docs: &Path) -> Option<String> {
        let json = legible_win::settings::json_string;
        // About's facts, this PC's own as the window gathers them, and the texts it opens.
        if body == "about" {
            let facts = legible_win::about::Facts {
                version: env!("CARGO_PKG_VERSION").to_owned(),
                portable: false,
                webview2: legible_win::shell::webview2_version(),
                windows: legible_win::win32::registry::windows_version(),
            };
            return Some(format!("LegibleChrome.about({})", facts.json()));
        }
        if let Some(name) = body.strip_prefix("reading\n") {
            let reading = legible_win::about::Reading::parse(name)?;
            let html = reading.html().map_or("null".to_owned(), |h| json(&h));
            return Some(format!(
                "LegibleChrome.reading({},{},{html})",
                json(name),
                json(reading.text())
            ));
        }
        match Message::parse(body)? {
            Message::Source => Some(format!(
                "LegibleEditor.load({{\"text\":{},\"editable\":true}})",
                json(markdown)
            )),
            Message::Render { version, text } => {
                let options = RenderOptions {
                    sourcepos: true,
                    ..RenderOptions::default()
                };
                let html = render_with(text, docs, &options).html;
                Some(format!("LegibleEditor.preview({version},{})", json(&html)))
            }
            _ => None,
        }
    }

    /// Calls `method` of the WebView's DevTools protocol on its own page; the answer isn't
    /// needed.
    fn devtools(webview: &WebView, method: &str, params: &str) {
        let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
        // SAFETY: a COM call on this thread's own WebView, with strings that live for the call.
        let called = unsafe {
            webview.webview().CallDevToolsProtocolMethod(
                &HSTRING::from(method),
                &HSTRING::from(params),
                &handler,
            )
        };
        if let Err(error) = called {
            eprintln!("{method}: {error}");
        }
    }

    /// Sends each context menu's items as `Menu`, and marks the menu handled, so it never
    /// opens.
    fn on_context_menu(webview: &WebView, proxy: EventLoopProxy<Ev>) {
        let webview: ICoreWebView2_11 = webview.webview().cast().expect("ICoreWebView2_11");
        let handler = ContextMenuRequestedEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                let _ = proxy.send_event(Ev::Menu(menu_items(&args).unwrap_or_default()));
                // SAFETY: a COM call on the event's own arguments, during the event.
                unsafe { args.SetHandled(true)? };
            }
            Ok(())
        }));
        let mut token = 0;
        // SAFETY: a COM call on this thread's own WebView; the handler is kept by it.
        unsafe { webview.add_ContextMenuRequested(&handler, &mut token) }
            .expect("the context menu's event");
    }

    /// The menu's items, as `name: label`.
    fn menu_items(
        args: &ICoreWebView2ContextMenuRequestedEventArgs,
    ) -> windows::core::Result<Vec<String>> {
        // SAFETY: COM calls on the event's own arguments; each string WebView2 returns is
        // taken by CoTaskMemPWSTR, which frees it.
        unsafe {
            let items = args.MenuItems()?;
            let mut count = 0;
            items.Count(&mut count)?;
            let mut out = Vec::new();
            for i in 0..count {
                let item = items.GetValueAtIndex(i)?;
                let (mut name, mut label) = (PWSTR::null(), PWSTR::null());
                item.Name(&mut name)?;
                item.Label(&mut label)?;
                out.push(format!(
                    "{}: {}",
                    CoTaskMemPWSTR::from(name),
                    CoTaskMemPWSTR::from(label)
                ));
            }
            Ok(out)
        }
    }

    /// A shot script's actions: every line but the last.
    fn action_only(script: &str) -> String {
        let script = script.trim();
        let last = script.rfind(";\n").map_or(0, |i| i + 1);
        script[..last].to_string()
    }

    /// A shot script's condition: its last statement, returned.
    fn condition_only(script: &str) -> String {
        let script = script.trim();
        let last = script.rfind(";\n").map_or(0, |i| i + 2);
        format!("return {};", script[last..].trim())
    }

    /// The page drawn at `scale` times its size (as at 150% scaling), whatever the monitor's.
    fn set_scale(webview: &WebView, scale: f64) {
        let controller = webview.controller();
        let controller: ICoreWebView2Controller3 =
            controller.cast().expect("ICoreWebView2Controller3");
        // SAFETY: COM calls on this thread's own controller, with plain values.
        unsafe {
            controller
                .SetShouldDetectMonitorScaleChanges(false)
                .expect("scale changes");
            controller
                .SetRasterizationScale(scale)
                .expect("rasterization scale");
        }
    }

    /// Writes what the WebView shows to `path` as a PNG, then sends `Captured`.
    fn capture(webview: &WebView, path: &Path, proxy: EventLoopProxy<Ev>) {
        let target = HSTRING::from(path.as_os_str());
        // SAFETY: the path string lives for the call; the stream and handler are COM objects
        // the call keeps alive until it completes.
        let started = unsafe {
            SHCreateStreamOnFileEx(
                &target,
                (STGM_CREATE | STGM_WRITE).0,
                FILE_ATTRIBUTE_NORMAL.0,
                true,
                None,
            )
        }
        .and_then(|stream| {
            let done = proxy.clone();
            let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
                let _ = done.send_event(Ev::Captured(result.map_err(|e| e.to_string())));
                Ok(())
            }));
            // SAFETY: as above.
            unsafe {
                webview.webview().CapturePreview(
                    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                    &stream,
                    &handler,
                )
            }
        });
        if let Err(error) = started {
            let _ = proxy.send_event(Ev::Captured(Err(error.to_string())));
        }
    }
}
