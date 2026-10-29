//! The README's pictures: the real window, its title bar and menu bar included, on the sample
//! document `docs/screenshots/project-update.md`, in View and in Split, light and dark.
//!
//! `cargo run -p legible-win --example readme-shots -- [out dir]` runs the shell through its
//! harness (`shell::harness`: its window never takes the focus, and Legible's folders are in a
//! temp folder of the run's own), and copies the window, frame and all, with `PrintWindow`, into
//! `target/ui-shots/readme` by default. Copy them into `docs/screenshots/` once they're approved.
//!
//! It drives only its own window: script in its own page, and its own window's handle. Each shot
//! is taken once the page says it's ready (the diagram drawn, and in Split the preview too),
//! never after a fixed wait.

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
    use std::path::{Path, PathBuf};

    use legible_win::paths::AppPaths;
    use legible_win::shell::harness::{self, Driver, Window};
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Dwm::{
        DWMWA_EXTENDED_FRAME_BOUNDS, DwmFlush, DwmGetWindowAttribute,
    };
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, HBITMAP,
        HPALETTE, RDW_ALLCHILDREN, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow,
        ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows::Win32::Graphics::GdiPlus::{
        GdipCreateBitmapFromHBITMAP, GdipDisposeImage, GdipSaveImageToFile, GdiplusShutdown,
        GdiplusStartup, GdiplusStartupInput, GpBitmap, GpImage, Status,
    };
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    };
    use windows::core::{BOOL, GUID, HSTRING};

    /// The window's inner size, in logical pixels: the sample, its table and diagram in view.
    const SIZE: (f64, f64) = (1060.0, 900.0);
    /// `PrintWindow`'s flag for content DirectComposition draws, such as WebView2's.
    const PW_RENDERFULLCONTENT: u32 = 2;
    /// GDI+'s PNG encoder.
    const PNG: GUID = GUID::from_u128(0x557cf406_1a04_11d3_9a73_0000f81ef32e);

    /// The page is drawn: its fonts loaded, and every diagram drawn in each theme.
    const DRAWN: &str = "document.fonts.status === 'loaded' && \
        [...document.querySelectorAll('#document div.mermaid')].length > 0 && \
        [...document.querySelectorAll('#document div.mermaid')].every((d) => LegibleDoc.isDrawn(d))";

    /// Each shot: its file's name, the theme, and whether it's in Split.
    const SHOTS: &[(&str, &str, bool)] = &[
        ("legible-light", "light", false),
        ("legible-dark", "dark", false),
        ("legible-split-dark", "dark", true),
        ("legible-split-light", "light", true),
    ];

    pub fn main() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        let out = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| repo.join("target").join("ui-shots").join("readme"));
        std::fs::create_dir_all(&out).expect("the output folder");
        let document = repo
            .join("docs")
            .join("screenshots")
            .join("project-update.md")
            .canonicalize()
            .expect("the sample document");

        let folder = tempfile::tempdir().expect("a temp folder");
        let paths = AppPaths::resolve(
            "Legible",
            &folder.path().join("app"),
            Some(&folder.path().join("appdata")),
        )
        .expect("the paths");
        std::fs::create_dir_all(paths.root()).expect("Legible's folder");
        // Nothing but the document: no default-app bar, the standard window, the file's top.
        std::fs::write(
            paths.settings_file(),
            "offer_default_app = false\nreopen_windows = false\nremember_place = false\n",
        )
        .expect("the settings");

        let written = harness::run(paths, "en", move |d| -> harness::Result<Vec<PathBuf>> {
            let window = d.open(&document)?;
            if let Some((x, y, _, _)) = d.primary_work_area()? {
                d.place_window(window, (x + 40, y + 40), SIZE)?;
            }
            d.until(window, "the document drawn", DRAWN)?;
            let mut written = Vec::new();
            for (name, theme, split) in SHOTS {
                set_theme(d, window, theme)?;
                if *split {
                    split_mode(d, window)?;
                }
                let file = out.join(format!("{name}.png"));
                capture(d, &file)?;
                written.push(file);
            }
            Ok(written)
        });
        drop(folder);
        match written {
            Ok(files) => {
                for file in files {
                    println!("{}", file.display());
                }
            }
            Err(why) => {
                eprintln!("readme-shots: {why}");
                std::process::exit(1);
            }
        }
    }

    /// The theme as Settings sets it (the page's own message; the window then tells every page),
    /// once the page wears it.
    fn set_theme(d: &Driver, window: Window, theme: &str) -> harness::Result<()> {
        d.eval(
            window,
            &format!("window.ipc.postMessage('setting\\ntheme\\n{theme}'); true"),
        )?;
        d.until(
            window,
            "the theme applied",
            &format!("document.documentElement.classList.contains('theme-{theme}')"),
        )?;
        // A round trip through the window's event loop, so its frame and menu bar are drawn too.
        d.eval(window, "true")?;
        Ok(())
    }

    /// Split, as its toolbar button does it, once the text is in and the preview drawn.
    fn split_mode(d: &Driver, window: Window) -> harness::Result<()> {
        d.eval(
            window,
            "document.querySelector('[data-mode=split]').click(); true",
        )?;
        d.until(
            window,
            "Split, its preview drawn",
            &format!(
                "document.documentElement.classList.contains('mode-split') && \
                 (document.getElementById('editor')?.shadowRoot?.querySelector('.cm-content')\
                 ?.textContent.length ?? 0) > 100 && \
                 !!document.querySelector('#document [data-sourcepos]') && {DRAWN}"
            ),
        )?;
        // The text's focus taken off, so no caret blinks in the picture.
        d.eval(window, "document.activeElement?.blur(); true")?;
        Ok(())
    }

    /// This process's visible window whose title names the sample.
    fn own_window() -> Option<HWND> {
        unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> BOOL {
            let found = unsafe { &mut *(found.0 as *mut Option<HWND>) };
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            if pid == unsafe { GetCurrentProcessId() } && unsafe { IsWindowVisible(hwnd) }.as_bool()
            {
                let mut title = [0u16; 256];
                let n = unsafe { GetWindowTextW(hwnd, &mut title) };
                if String::from_utf16_lossy(&title[..n.max(0) as usize])
                    .starts_with("project-update.md")
                {
                    *found = Some(hwnd);
                    return BOOL(0);
                }
            }
            BOOL(1)
        }
        let mut found: Option<HWND> = None;
        let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut found as *mut _ as isize)) };
        found
    }

    /// Copies the window as it is on screen, frame and all, to `file` as a PNG: `PrintWindow`
    /// into a bitmap of its whole rectangle, cut to the frame DWM draws (without the invisible
    /// resize borders).
    fn capture(d: &Driver, file: &Path) -> harness::Result<()> {
        d.until_true("the window's handle", || Ok(own_window().is_some()))?;
        let hwnd = own_window().ok_or("the window has gone")?;
        unsafe {
            // The frame drawn afresh (its title bar follows the theme a moment after the page),
            // then the next composition waited for, so the copy is of the window as it is now.
            let _ = RedrawWindow(
                Some(hwnd),
                None,
                None,
                RDW_FRAME | RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW,
            );
            DwmFlush().map_err(|e| e.to_string())?;
            let mut whole = RECT::default();
            GetWindowRect(hwnd, &mut whole).map_err(|e| e.to_string())?;
            let mut frame = RECT::default();
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut frame as *mut RECT as *mut _,
                std::mem::size_of::<RECT>() as u32,
            )
            .map_err(|e| e.to_string())?;
            let (w, h) = (whole.right - whole.left, whole.bottom - whole.top);
            let (fw, fh) = (frame.right - frame.left, frame.bottom - frame.top);

            let screen = GetDC(None);
            let all = CreateCompatibleDC(Some(screen));
            let all_bits = CreateCompatibleBitmap(screen, w, h);
            let old_all = SelectObject(all, all_bits.into());
            let printed = PrintWindow(hwnd, all, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT));
            let cut = CreateCompatibleDC(Some(screen));
            let cut_bits = CreateCompatibleBitmap(screen, fw, fh);
            let old_cut = SelectObject(cut, cut_bits.into());
            let copied = BitBlt(
                cut,
                0,
                0,
                fw,
                fh,
                Some(all),
                frame.left - whole.left,
                frame.top - whole.top,
                SRCCOPY,
            );
            SelectObject(cut, old_cut);
            SelectObject(all, old_all);
            let _ = DeleteDC(cut);
            let _ = DeleteDC(all);
            ReleaseDC(None, screen);
            let _ = DeleteObject(all_bits.into());
            let saved = if !printed.as_bool() {
                Err("PrintWindow failed".to_string())
            } else if let Err(e) = copied {
                Err(e.to_string())
            } else {
                save_png(cut_bits, file)
            };
            let _ = DeleteObject(cut_bits.into());
            saved
        }
    }

    /// Writes `bitmap` to `file` as a PNG, with GDI+.
    unsafe fn save_png(bitmap: HBITMAP, file: &Path) -> harness::Result<()> {
        let check = |status: Status, what: &str| {
            if status.0 == 0 {
                Ok(())
            } else {
                Err(format!("GDI+ couldn't {what} ({})", status.0))
            }
        };
        unsafe {
            let mut token = 0usize;
            let input = GdiplusStartupInput {
                GdiplusVersion: 1,
                ..Default::default()
            };
            check(
                GdiplusStartup(&mut token, &input, std::ptr::null_mut()),
                "start",
            )?;
            let mut image: *mut GpBitmap = std::ptr::null_mut();
            let mut result = check(
                GdipCreateBitmapFromHBITMAP(bitmap, HPALETTE::default(), &mut image),
                "read the bitmap",
            );
            if result.is_ok() {
                result = check(
                    GdipSaveImageToFile(
                        image as *mut GpImage,
                        &HSTRING::from(file.as_os_str()),
                        &PNG,
                        std::ptr::null(),
                    ),
                    "write the PNG",
                );
                GdipDisposeImage(image as *mut GpImage);
            }
            GdiplusShutdown(token);
            result
        }
    }
}
