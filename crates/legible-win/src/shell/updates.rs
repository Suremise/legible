//! The update check, carried out ([`crate::update`]): only while Settings has it on. Once as
//! Legible starts, again when it's turned on, and when the reader asks (Settings' Check now, or
//! Help > Check for updates); the request runs on a thread of its own. Every window is told what's
//! known (`LegibleChrome.updates`): Settings' line about it, and the bar of a newer release, whose
//! button opens that release's page in the browser. A window that asked is told the answer too.

use std::thread;

use legible_core::i18n::tr;
use legible_core::update::Outcome;
use tao::window::WindowId;

use super::{Shell, UserEvent};
use crate::update;
use crate::win32;

/// This build's version, compared with the latest release's.
const VERSION: &str = env!("CARGO_PKG_VERSION");

impl Shell {
    /// Starts a check if one is due ([`update::Updates::start`]): `asked` by a window, or as
    /// Legible starts (None). Nothing at all while the setting is off.
    pub(super) fn check_updates(&mut self, asked: Option<WindowId>) {
        let Some(request) = self.updates.start(self.settings.check_updates, asked) else {
            if asked.is_some() && !self.settings.check_updates {
                tracing::info!("asked to check for updates with the check off: nothing sent");
            }
            return;
        };
        tracing::info!(request, "checking for updates");
        self.send_updates_everywhere();
        let proxy = self.proxy.clone();
        if let Some(stand_in) = &self.stand_in {
            // A test's shell never goes online: its stand-in answers.
            let _ = proxy.send_event(UserEvent::Updated(request, stand_in.update()));
            return;
        }
        let spawned = thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let answer = update::check_latest(VERSION);
                let _ = proxy.send_event(UserEvent::Updated(request, answer));
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "couldn't start the update check");
            self.updated(request, Err(tr("reason-unexpected")));
        }
    }

    /// Request `request`'s answer: every window shows what's known, and those that asked are
    /// told. An answer to an earlier request (from before the check was turned off) is dropped.
    pub(super) fn updated(&mut self, request: update::Request, answer: Result<Outcome, String>) {
        match &answer {
            Ok(Outcome::Newer { version, url }) => {
                tracing::info!(%version, url, "a newer Legible is out")
            }
            Ok(Outcome::UpToDate) => tracing::info!("Legible is up to date"),
            Err(reason) => tracing::info!(reason, "couldn't check for updates"),
        }
        let asked = self.updates.finish(request, answer, VERSION);
        self.send_updates_everywhere();
        if let Some(text) = self.updates.told() {
            for id in asked {
                self.notice(id, &text);
            }
        }
    }

    /// The setting changed: turned on, it checks now (once in this run, as at start); turned
    /// off, what was found goes from every window.
    pub(super) fn updates_turned(&mut self, on: bool) {
        if on {
            self.check_updates(None);
        } else {
            self.updates.turn_off();
        }
        self.send_updates_everywhere();
    }

    /// The bar's See what's new: the newer release's page, as GitHub named it (never a URL
    /// from the page), in the reader's browser.
    pub(super) fn open_update(&self) {
        match self.updates.url() {
            Some(url) => {
                if let Err(error) = win32::shell_open(url) {
                    tracing::warn!(%error, "couldn't open the release's page");
                }
            }
            None => tracing::info!("no newer release to open"),
        }
    }

    /// The bar's Close: gone from every window for this run.
    pub(super) fn close_update_bar(&mut self) {
        self.updates.close_bar();
        self.send_updates_everywhere();
    }

    /// What's known of updates, for window `id`'s page.
    pub(super) fn send_updates(&self, id: WindowId) {
        let json = self.updates.to_json(self.settings.check_updates);
        self.run_script(id, &format!("window.LegibleChrome?.updates({json})"));
    }

    fn send_updates_everywhere(&self) {
        for &id in self.windows.keys() {
            self.send_updates(id);
        }
    }
}
