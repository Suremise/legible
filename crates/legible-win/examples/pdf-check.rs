//! Checks Save as PDF's printing in a real WebView2: the showcase as a window shows it
//! (`page::page`, the chrome's real script and print sheet), printed by the production
//! `win32::pdf::print_to_pdf` into a temp folder, then checked: `%PDF-` first, pages in it,
//! `%%EOF` last.
//!
//! `cargo run -p legible-win --example pdf-check -- [out dir]` copies the PDF to the out dir
//! (`target/ui-shots/export` by default) for reading back. It runs as a child of itself with a
//! temp folder of its run's own, which the parent removes once WebView2 has let go of it,
//! whether the check passed, failed or overran. Its window isn't focused; it's driven only by
//! script in its own WebView, and waits on conditions, with bounds.

#[cfg(windows)]
#[allow(unsafe_code)]
fn main() {
    check::main();
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod check {
    use std::borrow::Cow;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use legible_core::{assets, render::render};
    use legible_win::export::Paper;
    use legible_win::page;
    use legible_win::settings::Settings;
    use tao::event::{Event, StartCause};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::WindowBuilder;
    use wry::http::{Request, Response, header};
    use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

    const CHILD: &str = "LEGIBLE_PDF_CHECK_ROOT";
    const RUN_BOUND: Duration = Duration::from_secs(120);
    const RELEASE_BOUND: Duration = Duration::from_secs(30);

    pub fn main() {
        // A window of its own: quiet, so it never takes the user's focus, and checked
        // never to.
        legible_win::quiet::enable();
        let _focus = legible_win::quiet::watch_focus();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        let out = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| repo.join("target").join("ui-shots").join("export"));
        match std::env::var_os(CHILD) {
            Some(root) => run(&repo, &out, Path::new(&root)),
            None => std::process::exit(supervise()),
        }
    }

    /// Runs the check as a child with its own temp folder, and removes the folder after.
    fn supervise() -> i32 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let root =
            std::env::temp_dir().join(format!("legible-pdf-check-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).expect("the check's temp folder");
        let mut child = Command::new(std::env::current_exe().expect("this exe"))
            .args(std::env::args().skip(1))
            .env(CHILD, &root)
            .spawn()
            .expect("the check's run");
        let deadline = Instant::now() + RUN_BOUND;
        let mut code = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code().unwrap_or(1),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                _ => {
                    println!("the check overran: ended");
                    let _ = child.kill();
                    let _ = child.wait();
                    break 1;
                }
            }
        };
        let deadline = Instant::now() + RELEASE_BOUND;
        loop {
            match std::fs::remove_dir_all(&root) {
                Ok(()) => break,
                Err(_) if !root.exists() => break,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(_) => {
                    println!("couldn't remove the check's temp folder {}", root.display());
                    code = 1;
                    break;
                }
            }
        }
        code
    }

    enum Ev {
        Ready(bool),
        Printed(Result<(), String>),
    }

    fn run(repo: &Path, out: &Path, root: &Path) {
        legible_core::i18n::init("en");
        std::fs::create_dir_all(out).expect("the out folder");
        let docs = repo.join("scripts").join("doc-shots");
        let markdown = std::fs::read_to_string(docs.join("showcase.md")).expect("the showcase");
        let html = page::page(
            "en",
            "showcase.md – Legible",
            &render(&markdown, &docs).html,
            &Settings::default(),
        );
        let served = Arc::new(Mutex::new(html.into_bytes()));
        let pdf = root.join("showcase.pdf");
        let out = out.to_owned();

        let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        let window = legible_win::quiet::window(
            WindowBuilder::new().with_title("Legible: PDF check"),
            &event_loop,
        )
        .build(&event_loop)
        .expect("a window");
        let mut context = WebContext::new(Some(root.join("WebView2")));
        let page = served.clone();
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
                            .body(Cow::Owned(page.lock().unwrap().clone()))
                            .unwrap();
                    }
                    match assets::get(&path) {
                        Some(asset) => {
                            let mut response = Response::builder()
                                .header(header::CONTENT_TYPE, asset.content_type);
                            if path == page::MERMAID_FRAME {
                                response = response
                                    .header("Content-Security-Policy", page::MERMAID_FRAME_CSP);
                            }
                            response.body(Cow::Borrowed(asset.bytes)).unwrap()
                        }
                        None => Response::builder()
                            .status(404)
                            .body(Cow::Borrowed(&[][..]))
                            .unwrap(),
                    }
                },
            )
            .with_url(page::START_URL)
            .build(&window)
            .expect("a WebView2");

        let started = Instant::now();
        let mut printing = false;
        event_loop.run(move |event, _, flow| {
            *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(50));
            match event {
                Event::NewEvents(StartCause::ResumeTimeReached { .. }) if !printing => {
                    // Ready once the chrome is, diagrams drawn.
                    let ready = proxy.clone();
                    let _ = webview.evaluate_script_with_callback(
                        "window.LegibleChrome?.ready ? (LegibleChrome.ready.then(() => (window.__ready = true)), !!window.__ready) : false",
                        move |value| {
                            let _ = ready.send_event(Ev::Ready(value == "true"));
                        },
                    );
                    if started.elapsed() > Duration::from_secs(60) {
                        println!("  FAIL  the page never got ready");
                        std::process::exit(1);
                    }
                }
                Event::UserEvent(Ev::Ready(true)) if !printing => {
                    printing = true;
                    let done = proxy.clone();
                    let begun = legible_win::win32::pdf::print_to_pdf(
                        &webview.webview(),
                        &pdf,
                        Paper::A4,
                        Box::new(move |result| {
                            let _ = done.send_event(Ev::Printed(result.map_err(|e| e.to_string())));
                        }),
                    );
                    if let Err(error) = begun {
                        println!("  FAIL  PrintToPdf didn't start: {error}");
                        std::process::exit(1);
                    }
                }
                Event::UserEvent(Ev::Printed(result)) => {
                    let code = check_pdf(result, &pdf, &out);
                    if let Some(why) = legible_win::quiet::focus_taken() {
                        println!("  FAIL  {why}");
                        std::process::exit(1);
                    }
                    std::process::exit(code);
                }
                _ => {}
            }
        });
    }

    /// The printed PDF's structure, and a copy of it in `out`.
    fn check_pdf(result: Result<(), String>, pdf: &Path, out: &Path) -> i32 {
        if let Err(error) = result {
            println!("  FAIL  PrintToPdf: {error}");
            return 1;
        }
        let bytes = match std::fs::read(pdf) {
            Ok(bytes) => bytes,
            Err(error) => {
                println!("  FAIL  no PDF: {error}");
                return 1;
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        let pages = text
            .match_indices("/Type /Page")
            .filter(|(at, _)| !text[at + "/Type /Page".len()..].starts_with('s'))
            .count();
        let checks = [
            (bytes.starts_with(b"%PDF-"), "starts with %PDF-".to_owned()),
            (pages > 0, format!("{pages} pages")),
            (
                text.trim_end().ends_with("%%EOF"),
                "ends with %%EOF".to_owned(),
            ),
        ];
        let mut failed = 0;
        for (ok, what) in checks {
            println!("  {}  {what}", if ok { "ok  " } else { "FAIL" });
            failed += usize::from(!ok);
        }
        let copy = out.join("showcase.pdf");
        match std::fs::copy(pdf, &copy) {
            Ok(_) => println!("  the PDF: {}", copy.display()),
            Err(error) => {
                println!("  FAIL  couldn't copy the PDF out: {error}");
                failed += 1;
            }
        }
        i32::from(failed != 0)
    }
}
