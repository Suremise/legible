//! The menu bar's Win32 side ([`crate::menu`]): putting `muda`'s menus on a window, their theme,
//! and an Edit item's shortcut sent to the window's own WebView.
//!
//! **Sized with its menu:** a window is created with a placeholder menu ([`placeholder`]), so
//! the size it's given is its page's, the menu bar outside it; [`attach`] then puts the real
//! menus in its place.
//!
//! **The theme:** `muda` draws the bar itself dark or light; the menus that open from it are
//! Windows' own, dark only as the app's preferred mode says (uxtheme's `SetPreferredAppMode`,
//! not documented but what Windows' own apps use, and what tao and muda already call), so
//! [`set_app_theme`] sets that from Legible's theme, for every window at once.

use std::io;

use muda::MenuTheme;
use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{CreateMenu, DestroyMenu, HMENU};
use windows::core::{HSTRING, PCSTR, w};

use super::io_error;
use crate::menu::Keys;
use crate::page::Theme;

/// An empty menu for a window to be created with ([`attach`] destroys it).
pub fn placeholder() -> io::Result<HMENU> {
    // SAFETY: no arguments; the menu is destroyed by `attach`, or with its window.
    unsafe { CreateMenu() }.map_err(|e| io_error(&e))
}

/// Puts `menu` on `window` in place of its `placeholder`, which is destroyed. The menu must
/// outlive the window (it handles the window's menu messages until the window is destroyed).
pub fn attach(menu: &muda::Menu, window: HWND, placeholder: HMENU, theme: Theme) -> io::Result<()> {
    // SAFETY: `window` is a live window of this thread's; muda subclasses it to hear its menu's
    // commands, and lets go when it's destroyed.
    let attached = unsafe { menu.init_for_hwnd_with_theme(window.0 as isize, bar_theme(theme)) };
    // SAFETY: the placeholder is no longer the window's menu (or was never set, if muda failed,
    // and the window then has none); nothing else holds it.
    let _ = unsafe { DestroyMenu(placeholder) };
    attached.map_err(io::Error::other)
}

/// The bar's theme on `window`, as Legible's `theme`.
pub fn set_bar_theme(menu: &muda::Menu, window: HWND, theme: Theme) -> io::Result<()> {
    // SAFETY: `window` is a live window of this thread's, which `menu` was attached to.
    unsafe { menu.set_theme_for_hwnd(window.0 as isize, bar_theme(theme)) }
        .map_err(io::Error::other)
}

fn bar_theme(theme: Theme) -> MenuTheme {
    match theme {
        Theme::System => MenuTheme::Auto,
        Theme::Light => MenuTheme::Light,
        Theme::Dark => MenuTheme::Dark,
    }
}

/// uxtheme's `PreferredAppMode`.
#[repr(C)]
#[allow(dead_code)]
enum AppMode {
    Default,
    AllowDark,
    ForceDark,
    ForceLight,
}

/// The menus that open from the bar, dark or light as `theme` says (Windows' own, for
/// System). For the whole process: Legible has one theme for every window.
pub fn set_app_theme(theme: Theme) {
    const SET_PREFERRED_APP_MODE: usize = 135;
    const FLUSH_MENU_THEMES: usize = 136;
    let mode = match theme {
        Theme::System => AppMode::AllowDark,
        Theme::Light => AppMode::ForceLight,
        Theme::Dark => AppMode::ForceDark,
    };
    // SAFETY: uxtheme is part of Windows and stays loaded; each function is looked up by the
    // ordinal it has had since Windows 10 1903, and called with the signature it has there.
    // Not found (an older Windows), nothing is called.
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else {
            return;
        };
        if let Some(set) = GetProcAddress(uxtheme, PCSTR(SET_PREFERRED_APP_MODE as *const u8)) {
            let set: unsafe extern "system" fn(AppMode) -> AppMode = std::mem::transmute(set);
            set(mode);
        }
        if let Some(flush) = GetProcAddress(uxtheme, PCSTR(FLUSH_MENU_THEMES as *const u8)) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
            flush();
        }
    }
}

/// Ctrl and `keys`, pressed in `core`'s page alone (its DevTools protocol's
/// `Input.dispatchKeyEvent`): the page, the editor and the browser's own editing take them as
/// they take the keyboard (Ctrl+V pastes, Ctrl+Z undoes in the editor). Nothing goes through
/// Windows' input: whatever else is in the foreground never sees them.
pub fn send_keys(core: &ICoreWebView2, keys: Keys) -> io::Result<()> {
    let letter = keys.letter.to_ascii_uppercase();
    let (key, modifiers) = if keys.shift {
        (letter, 2 | 8)
    } else {
        (letter.to_ascii_lowercase(), 2)
    };
    let code = u32::from(letter);
    for kind in ["rawKeyDown", "keyUp"] {
        let params = format!(
            r#"{{"type":"{kind}","modifiers":{modifiers},"windowsVirtualKeyCode":{code},"nativeVirtualKeyCode":{code},"code":"Key{letter}","key":"{key}"}}"#
        );
        let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
        // SAFETY: a COM call on this thread's own WebView, with strings that live for the call.
        unsafe {
            core.CallDevToolsProtocolMethod(
                &HSTRING::from("Input.dispatchKeyEvent"),
                &HSTRING::from(params),
                &handler,
            )
        }
        .map_err(|e| io_error(&e))?;
    }
    Ok(())
}
