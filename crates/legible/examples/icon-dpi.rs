//! Checks that a window's icons follow its monitor's scaling, as Legible's windows set them
//! (`legible_win::shell::set_app_icons`, from the exe's own icon, linked into this example as
//! into legible.exe).
//!
//! `cargo run -p legible --example icon-dpi` opens a window of its own (quiet: off every monitor,
//! and never focused), and tells
//! it its DPI changed, as Windows does when it moves to a monitor with another scaling: the
//! message is sent in-process to this window alone (Windows won't take WM_DPICHANGED from
//! another process), and no input goes anywhere. After each change it waits, with a bound, for
//! both icons (WM_GETICON) to be the new size: 16 and 32 px scaled. Exits 1 if any isn't.

#[cfg(windows)]
#[allow(unsafe_code)]
fn main() {
    check::main();
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[allow(unsafe_code)]
mod check {
    use std::time::{Duration, Instant};

    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::platform::windows::WindowExtWindows;
    use tao::window::WindowBuilder;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW, HGDIOBJ};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetIconInfo, GetWindowRect, HICON, ICON_BIG, ICON_SMALL, ICONINFO, SendMessageW,
        WM_DPICHANGED, WM_GETICON,
    };

    /// The DPIs the window is told it has, one after another (96 is 100%).
    const STEPS: [u32; 4] = [144, 192, 120, 96];
    const BOUND: Duration = Duration::from_secs(10);

    /// The width of the window's icon of `kind`, if it has one.
    fn icon_width(hwnd: HWND, kind: u32) -> Option<i32> {
        // SAFETY: our own window; the icon's bitmaps are copies, deleted after reading.
        unsafe {
            let icon = SendMessageW(
                hwnd,
                WM_GETICON,
                Some(WPARAM(kind as usize)),
                Some(LPARAM(0)),
            );
            if icon.0 == 0 {
                return None;
            }
            let mut info = ICONINFO::default();
            GetIconInfo(HICON(icon.0 as _), &mut info).ok()?;
            let mut bitmap = BITMAP::default();
            let from = if info.hbmColor.is_invalid() {
                info.hbmMask
            } else {
                info.hbmColor
            };
            GetObjectW(
                HGDIOBJ(from.0),
                size_of::<BITMAP>() as i32,
                Some((&raw mut bitmap).cast()),
            );
            for handle in [info.hbmColor, info.hbmMask] {
                if !handle.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(handle.0));
                }
            }
            Some(bitmap.bmWidth)
        }
    }

    fn sizes(hwnd: HWND) -> (Option<i32>, Option<i32>) {
        (icon_width(hwnd, ICON_SMALL), icon_width(hwnd, ICON_BIG))
    }

    fn expected(dpi: u32) -> (Option<i32>, Option<i32>) {
        let px = |size: u32| Some((f64::from(size) * f64::from(dpi) / 96.0).round() as i32);
        (px(16), px(32))
    }

    pub fn main() {
        // A window of its own: quiet, so it never takes the user's focus, and checked
        // never to.
        legible_win::quiet::enable();
        let _focus = legible_win::quiet::watch_focus();
        let event_loop = EventLoop::new();
        let window = legible_win::quiet::window(
            WindowBuilder::new().with_title("Legible: icon DPI check"),
            &event_loop,
        )
        .build(&event_loop)
        .expect("a window");
        legible_win::shell::set_app_icons(&window, window.scale_factor());
        let hwnd = HWND(window.hwnd() as _);
        let start_dpi = (window.scale_factor() * 96.0).round() as u32;
        let first = sizes(hwnd);
        let mut failures = 0;
        let ok = first == expected(start_dpi);
        println!(
            "  {}  at {start_dpi} DPI: icons {first:?}",
            if ok { "ok  " } else { "FAIL" }
        );
        if first.0.is_none() {
            println!("  this example has no app icon: build.rs didn't link the resources into it");
            std::process::exit(1);
        }
        failures += u32::from(!ok);

        // The step sent and waiting to be seen, and when it was sent.
        let mut step = 0;
        let mut sent: Option<Instant> = None;
        event_loop.run(move |event, _, flow| {
            *flow = ControlFlow::Poll;
            // As Legible's windows do (legible_win::shell): new icons for the new scaling.
            if let Event::WindowEvent {
                event: WindowEvent::ScaleFactorChanged { scale_factor, .. },
                ..
            } = event
            {
                legible_win::shell::set_app_icons(&window, scale_factor);
                return;
            }
            if !matches!(event, Event::MainEventsCleared) {
                return;
            }
            let Some(&dpi) = STEPS.get(step) else {
                if let Some(why) = legible_win::quiet::focus_taken() {
                    println!("  FAIL  {why}");
                    failures += 1;
                }
                println!(
                    "{}",
                    if failures == 0 {
                        "All passed."
                    } else {
                        "Failed."
                    }
                );
                std::process::exit(i32::from(failures != 0));
            };
            match sent {
                None => {
                    // What Windows sends: the new DPI, and the window's suggested rectangle.
                    let mut rect = RECT::default();
                    // SAFETY: our own window, and a rectangle that lives for the call.
                    unsafe {
                        let _ = GetWindowRect(hwnd, &mut rect);
                        SendMessageW(
                            hwnd,
                            WM_DPICHANGED,
                            Some(WPARAM(((dpi << 16) | dpi) as usize)),
                            Some(LPARAM(&raw const rect as isize)),
                        );
                    }
                    sent = Some(Instant::now());
                }
                Some(at) => {
                    let now = sizes(hwnd);
                    let done = now == expected(dpi);
                    if done || at.elapsed() > BOUND {
                        println!(
                            "  {}  after WM_DPICHANGED {dpi}: icons {now:?}, expected {:?}",
                            if done { "ok  " } else { "FAIL" },
                            expected(dpi)
                        );
                        failures += u32::from(!done);
                        step += 1;
                        sent = None;
                    }
                }
            }
        });
    }
}
