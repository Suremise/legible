//! Proves, in a real WebView2, that saving works end to end: the page's Ctrl+S, the editor's
//! contents, [`saving::Session`], `legible_core::save::write` with Windows' replace, and what a
//! saved version unlocks (a reload that waited for the save), on a file in a temp folder.
//!
//! `cargo run -p legible-win --example save-check` opens a window of its own (not focused) on a
//! file saved as UTF-8 with a BOM, CRLF line endings and no final newline, and stands in for the
//! window as `shell` answers the page (`source`, `save`, `contents`, `keep`, `clean`). Then,
//! driving only its own page by script:
//!
//! 1. an edit, and a reload asked for meanwhile: it waits;
//! 2. Ctrl+S: the file is the edited text, byte for byte in its own format, the text is clean,
//!    and the reload that waited goes ahead; the editor opens again on the saved text;
//! 3. another program writes the file, then an edit and Ctrl+S: nothing is written, and the
//!    conflict bar asks; Keep mine saves over theirs;
//! 4. closing with an edit, Save chosen: the editor is frozen, an edit made while the save is
//!    written doesn't land, and the window closes only once the page says it's clean.
//!
//! Each step waits for what it expects, within 30 s; it exits 0 if all held, 1 if not.

#[cfg(windows)]
fn main() {
    check::main();
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod check {
    use std::borrow::Cow;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use legible_core::save::{self, NoHooks, Stamp};
    use legible_core::{assets, render::render};
    use legible_win::disk::Disk;
    use legible_win::editor::{Message, NavigationGuard, ReloadGate};
    use legible_win::reload::{self, Action, Kind};
    use legible_win::saving::{Choice, Close, Outcome, Save, Session};
    use legible_win::settings::{Settings, json_string};
    use legible_win::{page, shell, win32};
    use tao::event::{Event, StartCause};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::WindowBuilder;
    use wry::http::{Request, Response, header};
    use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows};

    enum Ev {
        /// A message from the page.
        Ipc(String),
        /// The answer to a poll.
        Answer(String),
    }

    /// What the stand-in window does for a step, besides its script.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Act {
        None,
        /// The page is to be served again (pictures from the internet consented to, say).
        Reload,
        /// Another program writes the file.
        WriteTheirs,
        /// The window is closed, and Save chosen.
        CloseAndSave,
    }

    /// What the stand-in window must have seen for a step to pass, besides its condition.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Host {
        Nothing,
        /// The reload waits.
        ReloadWaits,
        /// The file is exactly these bytes.
        File(&'static [u8]),
        /// The file is these bytes, and the window closed.
        Closed(&'static [u8]),
    }

    struct Step {
        what: &'static str,
        run: &'static str,
        act: Act,
        until: &'static str,
        host: Host,
    }

    /// UTF-8 with a BOM, CRLF, no final newline.
    const ORIGINAL: &[u8] = b"\xEF\xBB\xBF# Notes\r\n\r\nOne line";
    const SAVED: &[u8] = b"\xEF\xBB\xBFSaved. # Notes\r\n\r\nOne line";
    const THEIRS: &[u8] = b"\xEF\xBB\xBF# Theirs\r\n";
    const MINE: &[u8] = b"\xEF\xBB\xBFMine. Saved. # Notes\r\n\r\nOne line";
    const CLOSED: &[u8] = b"\xEF\xBB\xBFClose. Mine. Saved. # Notes\r\n\r\nOne line";

    const TEXT: &str = "(LegibleEditor.view()?.state.doc.toString() ?? '')";
    const CTRL_S: &str = r#"document.dispatchEvent(new KeyboardEvent("keydown", { key: "s", code: "KeyS", ctrlKey: true, bubbles: true }))"#;

    const STEPS: &[Step] = &[
        Step {
            what: "the page loaded, marked, its editor open",
            run: "",
            act: Act::None,
            until: r#"(() => { if (!window.LegibleChrome?.ready) return false; if (!window.marked) { window.marked = true; LegibleEditor.setMode("edit"); } return !!LegibleEditor.view(); })()"#,
            host: Host::File(ORIGINAL),
        },
        Step {
            what: "an edit, and a reload asked for meanwhile: it waits",
            run: r#"LegibleEditor.view().dispatch({ changes: { from: 0, insert: "Saved. " } })"#,
            act: Act::Reload,
            until: "LegibleEditor.isDirty() && window.marked === true",
            host: Host::ReloadWaits,
        },
        Step {
            what: "Ctrl+S: saved byte for byte, and the reload that waited goes ahead",
            run: CTRL_S,
            act: Act::None,
            until: "window.marked === undefined && !!window.LegibleChrome?.ready",
            host: Host::File(SAVED),
        },
        Step {
            what: "the editor opens again on the saved text",
            run: r#"window.marked = true; LegibleEditor.setMode("edit")"#,
            act: Act::None,
            until: "TEXT === 'Saved. # Notes\\n\\nOne line' && !LegibleEditor.isDirty()",
            host: Host::Nothing,
        },
        Step {
            what: "something else writes the file; an edit and Ctrl+S: the bar asks, nothing written",
            run: r#"LegibleEditor.view().dispatch({ changes: { from: 0, insert: "Mine. " } });
                document.dispatchEvent(new KeyboardEvent("keydown", { key: "s", code: "KeyS", ctrlKey: true, bubbles: true }))"#,
            act: Act::WriteTheirs,
            until: r##"!document.getElementById("conflict-bar").hidden && document.querySelector("#conflict-bar .conflict-text").textContent === "notes.md changed while you were editing it." && LegibleEditor.isDirty()"##,
            host: Host::File(THEIRS),
        },
        Step {
            what: "Keep my changes: saved over the file",
            run: r##"[...document.querySelectorAll("#conflict-bar button")].find((b) => b.textContent === "Keep my changes").click()"##,
            act: Act::None,
            until: "!LegibleEditor.isDirty() && document.getElementById('conflict-bar').hidden",
            host: Host::File(MINE),
        },
        Step {
            what: "closed with an edit, Save: frozen, a late edit kept out, closed once clean",
            run: r#"LegibleEditor.view().dispatch({ changes: { from: 0, insert: "Close. " } })"#,
            act: Act::CloseAndSave,
            until: "true",
            host: Host::Closed(CLOSED),
        },
    ];

    /// The stand-in window's state: what `shell` keeps for a window's saving.
    struct Window {
        file: PathBuf,
        disk: Disk,
        expected: Option<Stamp>,
        session: Session,
        reload: reload::State,
        gate: ReloadGate,
        guard: NavigationGuard,
        /// The held change's stamp, for Keep mine.
        held: Option<Stamp>,
        closed: bool,
        /// A request for the contents, as the close's save.
        next_request: u64,
    }

    impl Window {
        fn dirty(&self) -> bool {
            self.reload.is_dirty() || self.guard.dirty()
        }
    }

    fn finish(
        ok: bool,
        data: &Path,
        webview: Option<wry::WebView>,
        context: Option<WebContext>,
    ) -> ! {
        drop(webview);
        drop(context);
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            let _ = std::fs::remove_dir_all(data);
            if !data.exists() {
                break;
            }
            if Instant::now() > end {
                eprintln!("the temp folder couldn't be removed: {}", data.display());
                std::process::exit(1);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        std::process::exit(if ok { 0 } else { 1 })
    }

    pub fn main() {
        // A window of its own: quiet, so it never takes the user's focus, and checked
        // never to.
        legible_win::quiet::enable();
        let _focus = legible_win::quiet::watch_focus();
        legible_core::i18n::init("en");
        // The document, and WebView2's data, in a temp folder of this run's own.
        let data = std::env::temp_dir().join(format!("legible-save-check-{}", std::process::id()));
        std::fs::create_dir_all(&data).expect("the temp folder");
        let file = data.join("notes.md");
        std::fs::write(&file, ORIGINAL).expect("the document");
        let disk = Disk::read(&file).expect("read the document");
        let document = render(disk.content.text(), &data).html;
        let html = page::document_page(
            "en",
            "notes.md – Legible",
            &page::with_generation(&document, 0),
            &Settings::default(),
            0,
            &[],
            true,
        );

        let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        let window = legible_win::quiet::window(
            WindowBuilder::new().with_title("Legible save check"),
            &event_loop,
        )
        .build(&event_loop)
        .expect("a window");
        let mut context = WebContext::new(Some(data.join("WebView2")));
        let guard = NavigationGuard::default();
        let noted = guard.clone();
        let messages = proxy.clone();
        let webview = WebViewBuilder::new_with_web_context(&mut context)
            .with_focused(false)
            .with_https_scheme(true)
            .with_custom_protocol(
                page::APP_PROTOCOL.into(),
                move |_, request: Request<Vec<u8>>| {
                    let path = request.uri().path().to_owned();
                    if path == "/" {
                        return Response::builder()
                            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                            .header("Content-Security-Policy", page::CSP)
                            .body(Cow::Owned(html.clone().into_bytes()))
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
            .with_ipc_handler(move |request| {
                noted.note(request.body());
                let _ = messages.send_event(Ev::Ipc(request.body().clone()));
            })
            .with_navigation_handler(shell::navigation_policy(guard.clone(), |_| {}))
            .build(&window)
            .expect("a WebView");

        let mut state = Window {
            expected: Some(disk.stamp),
            file,
            disk,
            session: Session::default(),
            reload: reload::State::default(),
            gate: ReloadGate::default(),
            guard,
            held: None,
            closed: false,
            next_request: 0,
        };
        let mut step = 0;
        let mut ran = false;
        let mut reload_waited = false;
        let mut deadline = Instant::now() + Duration::from_secs(30);
        let mut next_poll = Instant::now();
        let mut webview = Some(webview);
        let mut context = Some(context);

        event_loop.run(move |event, _, flow| {
            *flow = ControlFlow::WaitUntil(next_poll);
            let view = webview.as_ref().unwrap();
            let run = |script: &str| {
                let _ = view.evaluate_script(script);
            };
            match event {
                Event::NewEvents(StartCause::Init) => view.load_url(page::START_URL).expect("load the page"),
                Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                    if Instant::now() > deadline {
                        eprintln!("FAIL  {}: timed out", STEPS[step].what);
                        finish(false, &data, webview.take(), context.take());
                    }
                    let current = &STEPS[step];
                    if !ran {
                        ran = true;
                        match current.act {
                            Act::None | Act::Reload => {}
                            // Before the step's edit and save: a save then finds it changed.
                            Act::WriteTheirs => std::fs::write(&state.file, THEIRS).expect("their write"),
                            Act::CloseAndSave => {}
                        }
                        if !current.run.is_empty() {
                            run(current.run);
                        }
                    }
                    let host = match current.host {
                        Host::Nothing => true,
                        Host::ReloadWaits => reload_waited && state.gate.waiting(),
                        Host::File(bytes) => std::fs::read(&state.file).is_ok_and(|b| b == bytes),
                        Host::Closed(bytes) => state.closed && std::fs::read(&state.file).is_ok_and(|b| b == bytes),
                    };
                    // Asked once the window knows of the edit, as a consent's reload is.
                    if current.act == Act::Reload && state.dirty() && !reload_waited {
                        reload_waited = state.gate.request(Vec::new(), true).is_none();
                    }
                    if current.act == Act::CloseAndSave && state.dirty() && !state.session.frozen() && !state.closed {
                        // The window's close: dirty, so it asks; Save is chosen.
                        assert_eq!(state.session.close(true), Close::Ask);
                        assert_eq!(state.session.choose(Choice::Save), Close::FreezeAndSave);
                        state.next_request += 1;
                        run(&format!("LegibleEditor.freeze(); LegibleEditor.contents({})", state.next_request));
                    }
                    let condition = current.until.replace("TEXT", TEXT);
                    let proxy = proxy.clone();
                    let _ = view.evaluate_script_with_callback(
                        &format!("(() => {{ try {{ return {condition}; }} catch {{ return false; }} }})()"),
                        move |answer| {
                            let _ = proxy.send_event(Ev::Answer(format!("{}{answer}", if host { "" } else { "host:" })));
                        },
                    );
                    next_poll = Instant::now() + Duration::from_millis(100);
                }
                Event::UserEvent(Ev::Ipc(body)) => match Message::parse(&body) {
                    Some(Message::Source) => run(&format!(
                        "LegibleEditor.load({{\"text\":{},\"editable\":true}})",
                        json_string(state.disk.content.text())
                    )),
                    Some(Message::Dirty(dirty)) => {
                        state.reload.set_dirty(dirty);
                        if !dirty && let Some(_origins) = state.gate.clean() {
                            // Served again, as the window does.
                            view.load_url(page::START_URL).expect("serve the page again");
                        }
                    }
                    Some(Message::Save) => {
                        state.next_request += 1;
                        run(&format!("LegibleEditor.contents({})", state.next_request));
                    }
                    Some(Message::Contents { version, text, .. }) => {
                        let Some(save) = state.session.save(Save { version, text: text.to_owned() }) else {
                            return;
                        };
                        if state.session.frozen() {
                            // A late edit while the close's save is written: it can't land.
                            run(r#"LegibleEditor.view().dispatch({ changes: { from: 0, insert: "Late. " } })"#);
                        }
                        let format = *state.disk.content.format().expect("a text format");
                        let bytes = save::encode(&save.text, &format);
                        let written = save::write(
                            &state.file,
                            &bytes,
                            state.expected.as_ref(),
                            &win32::replace::Windows,
                            &NoHooks,
                        );
                        let outcome = match written {
                            Ok(stamp) => {
                                state.disk = Disk::of(bytes, stamp);
                                state.expected = Some(stamp);
                                run(&format!("LegibleEditor.markSaved({version})"));
                                Outcome::Saved
                            }
                            Err(save::WriteError::Conflict(_)) => {
                                run(&format!("LegibleEditor.saveAbandoned({version})"));
                                let theirs = save::read(&state.file, &win32::identity).expect("theirs");
                                state.held = Some(theirs.1);
                                if let Action::Hold { kind, id } = state.reload.on_disk(Kind::Changed) {
                                    state.session.conflict(id);
                                    run(&format!(
                                        "window.dispatchEvent(new CustomEvent('legible:external-change', {{ detail: {{ kind: {}, id: {id}, text: {} }} }}))",
                                        json_string(kind.as_str()),
                                        // The bar's words, as the window says them.
                                        json_string(&legible_core::tr!(
                                            "chrome-conflict-changed",
                                            file = "notes.md"
                                        ))
                                    ));
                                }
                                Outcome::Conflict
                            }
                            Err(error) => {
                                eprintln!("FAIL  {}: the save failed: {error:?}", STEPS[step].what);
                                finish(false, &data, webview.take(), context.take());
                            }
                        };
                        let done = state.session.done(version, outcome);
                        if done.close == Close::CheckClean {
                            run("LegibleEditor.closeCheck()");
                        }
                    }
                    Some(Message::Clean(clean)) => {
                        state.closed = state.session.clean(clean && !state.dirty()) == Close::Now;
                    }
                    None => {
                        let mut parts = body.split('\n');
                        if let (Some("keep"), Some(n)) = (parts.next(), parts.next())
                            && let Ok(n) = n.parse()
                            && state.reload.keep(n)
                        {
                            state.expected = state.held.take();
                            if state.session.kept(n) {
                                state.next_request += 1;
                                run(&format!("LegibleEditor.contents({})", state.next_request));
                            }
                        }
                    }
                    _ => {}
                },
                Event::UserEvent(Ev::Answer(answer)) => {
                    if answer.trim() != "true" {
                        return;
                    }
                    println!("  ok    {}", STEPS[step].what);
                    step += 1;
                    if step == STEPS.len() {
                        let taken = legible_win::quiet::focus_taken();
                        match &taken {
                            Some(why) => println!("  FAIL  {why}"),
                            None => println!("All passed."),
                        }
                        finish(taken.is_none(), &data, webview.take(), context.take());
                    }
                    ran = false;
                    deadline = Instant::now() + Duration::from_secs(30);
                }
                _ => {}
            }
        });
    }
}
