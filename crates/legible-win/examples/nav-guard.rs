//! Proves, in a real WebView2, that a window's page is never reloaded or navigated while its
//! editor has unsaved edits.
//!
//! `cargo run -p legible-win --example nav-guard` opens a window of its own (not focused) on a
//! short document, with the window's own navigation policy (`shell::navigation_policy`) and the
//! hook that tells it of unsaved edits (`editor::NavigationGuard`), and stands in for the
//! window's answer to the editor's `source`. Then, driving only its own page by script:
//!
//! 1. the editor's text is changed: a reload, a navigation to the page, and the host loading the
//!    page (as serving it again does) are each refused, and the page and its text stay;
//! 2. the edit is undone: a reload goes ahead;
//! 3. an edit and a reload in the same script (the reload starting right after the edit's
//!    `dirty` message): refused, the text kept.
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
    use std::path::Path;
    use std::time::{Duration, Instant};

    use legible_core::{assets, render::render};
    use legible_win::editor::{Message, Navigation, NavigationGuard};
    use legible_win::settings::{Settings, json_string};
    use legible_win::{page, shell};
    use tao::event::{Event, StartCause};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::WindowBuilder;
    use wry::http::{Request, Response, header};
    use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows};

    enum Ev {
        /// A message from the page.
        Ipc(String),
        /// What the navigation policy did.
        Navigated(Navigation),
        /// The answer to a poll.
        Answer(String),
    }

    /// A step: script to run once, then a condition polled until it's "true".
    struct Step {
        what: &'static str,
        run: &'static str,
        until: &'static str,
        /// The navigation the step's run must have met, if it tries one.
        refused: bool,
        /// The host loads the page itself (as serving it again does) after the run.
        host_load: bool,
    }

    const TEXT: &str = "LegibleEditor.view()?.state.doc.toString() ?? ''";

    const STEPS: &[Step] = &[
        Step {
            what: "the page loaded, marked",
            run: "",
            until: "(() => { if (!window.LegibleChrome?.ready) return false; window.marked = true; return true; })()",
            refused: false,
            host_load: false,
        },
        Step {
            what: "the editor open",
            run: r#"LegibleEditor.setMode("edit")"#,
            until: "!!LegibleEditor.view()",
            refused: false,
            host_load: false,
        },
        Step {
            what: "an unsaved change",
            run: r#"LegibleEditor.view().dispatch({ changes: { from: 0, insert: "Unsaved. " } })"#,
            until: "LegibleEditor.isDirty()",
            refused: false,
            host_load: false,
        },
        Step {
            what: "a reload refused while dirty",
            run: "location.reload()",
            until: "window.marked === true && (TEXT).startsWith('Unsaved. ')",
            refused: true,
            host_load: false,
        },
        Step {
            what: "a navigation to the page refused while dirty",
            run: r#"location.href = "https://legible.localhost/""#,
            until: "window.marked === true && (TEXT).startsWith('Unsaved. ')",
            refused: true,
            host_load: false,
        },
        Step {
            what: "the host loading the page refused while dirty",
            run: "",
            until: "window.marked === true && (TEXT).startsWith('Unsaved. ')",
            refused: true,
            host_load: true,
        },
        Step {
            what: "the change undone",
            run: "window.CM.commands.undo(LegibleEditor.view())",
            until: "!LegibleEditor.isDirty()",
            refused: false,
            host_load: false,
        },
        Step {
            what: "a reload once clean, going ahead",
            run: "location.reload()",
            until: "window.marked === undefined && !!window.LegibleChrome?.ready",
            refused: false,
            host_load: false,
        },
        Step {
            what: "the page marked again, its editor open",
            run: r#"window.marked = true; LegibleEditor.setMode("edit")"#,
            until: "!!LegibleEditor.view()",
            refused: false,
            host_load: false,
        },
        Step {
            what: "an edit and a reload in one script: refused, the text kept",
            run: r#"LegibleEditor.view().dispatch({ changes: { from: 0, insert: "Race. " } }); location.reload()"#,
            until: "window.marked === true && (TEXT).startsWith('Race. ')",
            refused: true,
            host_load: false,
        },
    ];

    /// Ends the check: the WebView closed, its temp folder removed once nothing holds it (its
    /// browser processes let go of it a moment after), within 30 s.
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
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        legible_core::i18n::init("en");
        let docs = repo.join("scripts").join("chrome-tests");
        let markdown = std::fs::read_to_string(docs.join("short.md")).expect("the document");
        let document = render(&markdown, &docs).html;
        let html = page::page("en", "short.md – Legible", &document, &Settings::default());

        let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        let window = legible_win::quiet::window(
            WindowBuilder::new().with_title("Legible navigation check"),
            &event_loop,
        )
        .build(&event_loop)
        .expect("a window");
        // WebView2's data, in a temp folder of this run's own, removed once it's done.
        let data = std::env::temp_dir().join(format!("legible-nav-guard-{}", std::process::id()));
        let mut context = WebContext::new(Some(data.clone()));
        let guard = NavigationGuard::default();
        let noted = guard.clone();
        let messages = proxy.clone();
        let navigations = proxy.clone();
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
                // As the window's own handler: the guard hears of unsaved edits first.
                noted.note(request.body());
                let _ = messages.send_event(Ev::Ipc(request.body().clone()));
            })
            .with_navigation_handler(shell::navigation_policy(guard.clone(), move |navigation| {
                let _ = navigations.send_event(Ev::Navigated(navigation));
            }))
            .build(&window)
            .expect("a WebView");

        let mut step = 0;
        let mut ran = false;
        let mut refused = false;
        let mut deadline = Instant::now() + Duration::from_secs(30);
        let mut next_poll = Instant::now();
        // Dropped to finish: its browser processes then let go of the temp folder.
        let mut webview = Some(webview);
        let mut context = Some(context);

        event_loop.run(move |event, _, flow| {
            *flow = ControlFlow::WaitUntil(next_poll);
            match event {
                Event::NewEvents(StartCause::Init) => {
                    webview.as_ref().unwrap().load_url(page::START_URL).expect("load the page");
                }
                Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                    if Instant::now() > deadline {
                        eprintln!("FAIL  {}: timed out", STEPS[step].what);
                        finish(false, &data, webview.take(), context.take());
                    }
                    let current = &STEPS[step];
                    if !ran {
                        ran = true;
                        if !current.run.is_empty() {
                            let _ = webview.as_ref().unwrap().evaluate_script(current.run);
                        }
                        if current.host_load && let Err(error) = webview.as_ref().unwrap().load_url(page::START_URL) {
                            eprintln!("FAIL  {}: {error}", current.what);
                            finish(false, &data, webview.take(), context.take());
                        }
                    }
                    let condition = current.until.replace("TEXT", TEXT);
                    let proxy = proxy.clone();
                    let _ = webview.as_ref().unwrap().evaluate_script_with_callback(
                        &format!("(() => {{ try {{ return {condition}; }} catch {{ return false; }} }})()"),
                        move |answer| {
                            let _ = proxy.send_event(Ev::Answer(answer));
                        },
                    );
                    next_poll = Instant::now() + Duration::from_millis(100);
                }
                Event::UserEvent(Ev::Navigated(navigation)) => {
                    if navigation == Navigation::Unsaved {
                        refused = true;
                    }
                }
                Event::UserEvent(Ev::Ipc(body)) => {
                    // The window's answer to the editor's `source`.
                    if Message::parse(&body) == Some(Message::Source) {
                        let script = format!(
                            "LegibleEditor.load({{\"text\":{},\"editable\":true}})",
                            json_string(&markdown)
                        );
                        let _ = webview.as_ref().unwrap().evaluate_script(&script);
                    }
                }
                Event::UserEvent(Ev::Answer(answer)) => {
                    let current = &STEPS[step];
                    // A step that tries a navigation waits for the policy's refusal too.
                    if answer.trim() != "true" || (current.refused && !refused) {
                        return;
                    }
                    println!("  ok    {}", current.what);
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
                    refused = false;
                    deadline = Instant::now() + Duration::from_secs(30);
                }
                _ => {}
            }
        });
    }
}
