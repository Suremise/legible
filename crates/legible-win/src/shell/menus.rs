//! A window's menu bar ([`crate::menu`]), carried out: each item does what its toolbar button or
//! shortcut does, the items follow the window's state, and Exit closes every window in turn.

use tao::event_loop::ControlFlow;
use tao::platform::windows::WindowExtWindows;
use tao::window::{Window, WindowId};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::HMENU;
use wry::WebViewExtWindows;

use super::{DocWindow, Shell, UserEvent};
use crate::menu::{self, Bar, Exit, Picked, Route, Standing, Step};
use crate::page::Theme;
use crate::settings::json_string;
use crate::win32;

impl Shell {
    /// Picked items come to the event loop, as [`UserEvent::Menu`] with their IDs.
    pub(super) fn hear_menus(&self) {
        let proxy = std::sync::Mutex::new(self.proxy.clone());
        muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
            if let Ok(proxy) = proxy.lock() {
                let _ = proxy.send_event(UserEvent::Menu(event.id.0));
            }
        }));
        win32::menu::set_app_theme(self.settings.theme);
    }

    /// A new window's bar, and the placeholder menu the window is created with so it's sized
    /// with a menu bar. None: the window has no bar (logged).
    pub(super) fn new_bar(&mut self) -> Option<(Bar, HMENU)> {
        self.next_bar += 1;
        let made = Bar::new(self.next_bar)
            .map_err(std::io::Error::other)
            .and_then(|bar| Ok((bar, win32::menu::placeholder()?)));
        match made {
            Ok(made) => Some(made),
            Err(error) => {
                tracing::warn!(%error, "couldn't make a window's menu bar");
                None
            }
        }
    }

    /// Puts `bar` on `window`, in place of the `placeholder` it was created with.
    pub(super) fn attach_bar(&self, bar: Bar, placeholder: HMENU, window: &Window) -> Option<Bar> {
        let hwnd = HWND(window.hwnd() as _);
        match win32::menu::attach(bar.menu(), hwnd, placeholder, self.settings.theme) {
            Ok(()) => {
                if let Err(error) = bar.set_recent(self.recent.token(), self.recent.paths()) {
                    tracing::warn!(%error, "couldn't list the recent files in the menu");
                }
                Some(bar)
            }
            Err(error) => {
                tracing::warn!(%error, "couldn't put the menu bar on a window");
                None
            }
        }
    }

    /// An item was picked: it does what its toolbar button or shortcut does.
    pub(super) fn menu(&mut self, item: &str) {
        let Some((number, picked)) = menu::parse_id(item) else {
            return tracing::warn!(item, "a menu item Legible doesn't know");
        };
        let Some(id) = self.windows.iter_mut().find_map(|(id, doc)| {
            let bar = doc.bar.as_mut().filter(|b| b.number() == number)?;
            bar.picked();
            Some(*id)
        }) else {
            return;
        };
        let command = match picked {
            Picked::Recent { token, index } => {
                return self.chrome(id, &format!("open-recent\n{token}\n{index}"));
            }
            Picked::Command(command) => command,
        };
        match command.route() {
            Route::Window(message) => self.chrome(id, message),
            Route::Page(name) => {
                let script = format!("window.LegibleChrome?.command({})", json_string(name));
                self.run_script(id, &script);
            }
            Route::Keys(keys) => {
                if let Some(doc) = self.windows.get(&id)
                    && let Err(error) = win32::menu::send_keys(&doc.webview.webview(), keys)
                {
                    tracing::warn!(%error, ?command, "couldn't send a menu item's keys");
                }
            }
            Route::Exit => {
                // The window it was asked from first, then the rest.
                let mut order: Vec<WindowId> = vec![id];
                order.extend(self.windows.keys().filter(|w| **w != id));
                self.exit = Some(Exit::new(order));
            }
        }
    }

    /// After anything that happens: Exit goes on to its next window (Legible exits once the
    /// last has closed), and each window's items follow its state.
    pub(super) fn menus_follow(&mut self, flow: &mut ControlFlow) {
        if self.exit_follow() {
            self.exit_when_settled(flow);
        }
        let states: Vec<(WindowId, menu::State)> = self
            .windows
            .iter()
            .map(|(id, doc)| (*id, self.menu_state(doc)))
            .collect();
        for (id, state) in states {
            if let Some(bar) = self.windows.get_mut(&id).and_then(|d| d.bar.as_mut()) {
                bar.show(state);
            }
        }
    }

    /// Exit's next step; whether it closed a window.
    fn exit_follow(&mut self) -> bool {
        let mut closed = false;
        loop {
            let windows = &self.windows;
            let Some(exit) = self.exit.as_mut() else {
                return closed;
            };
            let step = exit.step(|w| match windows.get(&w) {
                None => Standing::Closed,
                Some(doc)
                    if doc.saves.closing() || doc.close_after_take || doc.close_after_save_as =>
                {
                    Standing::Closing
                }
                Some(_) => Standing::Open,
            });
            match step {
                Step::Close(w) => {
                    closed = true;
                    self.focus(w);
                    self.ask_close(w);
                }
                Step::Wait => return closed,
                Step::Done | Step::Stopped(_) => {
                    self.exit = None;
                    return closed;
                }
            }
        }
    }

    fn menu_state(&self, doc: &DocWindow) -> menu::State {
        menu::State {
            // Still reading its file: nothing is shown yet to save, print or copy.
            document: doc.file.is_some() && doc.opening.is_none(),
            editable: doc.editable,
            mode: doc.mode,
            outline: doc.outline,
            dirty: doc.reload.is_dirty(),
            saving: doc.saves.busy() || doc.saves.closing(),
            theme: self.settings.theme,
            width: self.settings.width,
            status_bar: self.settings.status_bar,
            check_updates: self.settings.check_updates,
        }
    }

    /// The recent files changed: every window's Open recent shows them.
    pub(super) fn bars_recent(&self) {
        for doc in self.windows.values() {
            if let Some(bar) = &doc.bar
                && let Err(error) = bar.set_recent(self.recent.token(), self.recent.paths())
            {
                tracing::warn!(%error, "couldn't list the recent files in the menu");
            }
        }
    }

    /// The theme changed: every window's bar, title bar and menus take it.
    pub(super) fn bars_theme(&self) {
        let theme = self.settings.theme;
        win32::menu::set_app_theme(theme);
        for doc in self.windows.values() {
            doc.window.set_theme(window_theme(theme));
            if let Some(bar) = &doc.bar
                && let Err(error) =
                    win32::menu::set_bar_theme(bar.menu(), HWND(doc.window.hwnd() as _), theme)
            {
                tracing::warn!(%error, "couldn't change the menu bar's theme");
            }
        }
    }
}

/// The window's own theme (its title bar), as Legible's.
pub(super) fn window_theme(theme: Theme) -> Option<tao::window::Theme> {
    match theme {
        Theme::System => None,
        Theme::Light => Some(tao::window::Theme::Light),
        Theme::Dark => Some(tao::window::Theme::Dark),
    }
}
