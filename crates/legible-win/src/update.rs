//! The update check (Settings > General > Updates): off by default, and the one thing Legible
//! itself sends over the internet. When it's on, Legible asks once as it starts (and again when
//! the reader asks, from Settings' Check now or Help > Check for updates), and only ever this:
//! a `GET` of the project's latest-release page on GitHub ([`legible_core::update::LATEST_URL`])
//! with redirects off, reading the redirect's `Location`. Nothing about the reader or their files
//! is sent, nothing is downloaded and nothing installed: a newer release is shown as a bar whose
//! button opens its page in the reader's browser.
//!
//! The request is [`crate::win32::http`]'s, with WinHTTP, part of Windows: no HTTP or TLS crate
//! ships in Legible. [`check_latest`] is the only caller, and the shell calls it only while the
//! setting is on (a test's shell never does: its stand-in answers instead).

use std::time::Duration;

use legible_core::update::{self, Outcome};
use legible_core::{i18n::tr, tr};

use crate::settings::json_string;
use crate::win32::http::{self, Target};

/// How long each step of the request may take.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Asks GitHub for the latest release and compares it with `current` (this build's version).
/// Blocks for up to a few seconds: call it off the UI thread. An error is why, for
/// `update-failed`'s `{ $reason }`.
pub fn check_latest(current: &str) -> Result<Outcome, String> {
    if crate::quiet::on() {
        // A test's process never goes online.
        return Err(tr("reason-unexpected"));
    }
    let agent = format!("Legible/{}", env!("CARGO_PKG_VERSION"));
    let target = Target {
        host: update::HOST,
        port: 443,
        secure: true,
    };
    let answer = http::redirect_location(target, update::LATEST_PATH, &agent, TIMEOUT).map_err(
        |failure| {
            tracing::info!(?failure, "the update check's request failed");
            reason(&failure)
        },
    )?;
    if !(300..400).contains(&answer.status) {
        return Err(tr!("update-failed-status", status = answer.status));
    }
    let location = answer.location;
    let Some((version, url)) = location.as_deref().and_then(update::latest_from_location) else {
        tracing::info!(?location, "GitHub named no release of Legible");
        return Ok(Outcome::UpToDate);
    };
    update::outcome(current, version, url)
        .ok_or_else(|| tr!("update-failed-version", version = current))
}

/// Why a request failed, as a `{ $reason }`: the common failures in Legible's words; another
/// in Windows' (no full stop), or by its number where Windows has no words for it.
fn reason(failure: &http::Failure) -> String {
    match failure.code {
        http::TIMEOUT => return tr("update-failed-timeout"),
        http::NAME_NOT_RESOLVED | http::CANNOT_CONNECT | http::CONNECTION_ERROR => {
            return tr("update-failed-offline");
        }
        _ => {}
    }
    let text = failure.message.trim().trim_end_matches('.').trim();
    if text.is_empty() {
        tr!("update-failed-code", code = failure.code)
    } else {
        text.to_owned()
    }
}

/// What the check knows, for Settings and the bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Not checked in this run (or turned off).
    Idle,
    Checking,
    /// A newer release, and its page.
    Newer {
        version: String,
        url: String,
    },
    /// This is the latest: this version.
    Latest {
        version: String,
    },
    /// The check didn't work, and why.
    Failed {
        reason: String,
    },
}

/// A request's number ([`Updates::start`]): only the request under way has its answer taken.
pub type Request = u64;

/// The update check for the process, generic over what a window is (`W`): whether to check,
/// what was found, whether the bar is closed, and which windows asked (each told the answer).
#[derive(Debug)]
pub struct Updates<W> {
    status: Status,
    /// A check has run, or is running, since Legible started (or the setting was turned on).
    started: bool,
    bar_closed: bool,
    asked: Vec<W>,
    /// The request under way, and the last number given: never reset, so a request from before
    /// the setting was turned off and on again can't pass for a later one.
    active: Option<Request>,
    last: Request,
}

impl<W> Default for Updates<W> {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            started: false,
            bar_closed: false,
            asked: Vec::new(),
            active: None,
            last: 0,
        }
    }
}

impl<W: Copy + PartialEq> Updates<W> {
    /// The request to start now, if one is due: only while the setting is `on`; as Legible
    /// starts, once (`asked` None); when a window asks, unless one is under way, whose answer
    /// it's told too.
    pub fn start(&mut self, on: bool, asked: Option<W>) -> Option<Request> {
        if !on {
            return None;
        }
        if let Some(window) = asked {
            if !self.asked.contains(&window) {
                self.asked.push(window);
            }
            // Asked again: a newer release shows its bar again.
            self.bar_closed = false;
        }
        if self.active.is_some() || (asked.is_none() && self.started) {
            return None;
        }
        self.started = true;
        self.status = Status::Checking;
        self.last += 1;
        self.active = Some(self.last);
        self.active
    }

    /// Request `request`'s answer: the windows that asked for it, to be told. An answer to any
    /// request but the one under way (one from before the setting was turned off) is dropped.
    pub fn finish(
        &mut self,
        request: Request,
        answer: Result<Outcome, String>,
        current: &str,
    ) -> Vec<W> {
        if self.active != Some(request) {
            return Vec::new();
        }
        self.active = None;
        self.status = match answer {
            Ok(Outcome::Newer { version, url }) => Status::Newer {
                version: version.to_string(),
                url,
            },
            Ok(Outcome::UpToDate) => Status::Latest {
                version: current.to_owned(),
            },
            Err(reason) => Status::Failed { reason },
        };
        std::mem::take(&mut self.asked)
    }

    /// The setting was turned off: nothing found is shown, the request under way is forgotten
    /// (its answer will be dropped), and turning it on checks again.
    pub fn turn_off(&mut self) {
        *self = Self {
            last: self.last,
            ..Self::default()
        };
    }

    /// The bar's Close: not shown again in this run unless the reader asks again.
    pub fn close_bar(&mut self) {
        self.bar_closed = true;
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// The newer release's page, for the bar's button: never a URL from the page.
    pub fn url(&self) -> Option<&str> {
        match &self.status {
            Status::Newer { url, .. } => Some(url),
            _ => None,
        }
    }

    /// Whether the bar shows: a newer release, and the bar not closed.
    pub fn bar(&self) -> bool {
        matches!(self.status, Status::Newer { .. }) && !self.bar_closed
    }

    /// What the window that asked is told: that this is the latest, or why the check failed (a
    /// newer release has its bar).
    pub fn told(&self) -> Option<String> {
        match &self.status {
            Status::Latest { version } => Some(tr!("update-latest", version = version.as_str())),
            Status::Failed { reason } => Some(tr!("update-failed", reason = reason.as_str())),
            _ => None,
        }
    }

    /// For the page (`LegibleChrome.updates`): whether the setting is `on`, whether a check is
    /// under way, Settings' line about the last check, and the bar's text (empty: no bar).
    pub fn to_json(&self, on: bool) -> String {
        let status = match &self.status {
            Status::Idle => String::new(),
            Status::Checking => tr("update-checking"),
            Status::Newer { version, .. } => tr!("update-available", version = version.as_str()),
            Status::Latest { version } => tr!("update-latest", version = version.as_str()),
            Status::Failed { reason } => tr!("update-failed", reason = reason.as_str()),
        };
        let bar = match &self.status {
            Status::Newer { version, .. } if self.bar() => {
                tr!("update-available", version = version.as_str())
            }
            _ => String::new(),
        };
        format!(
            "{{\"on\":{on},\"checking\":{},\"status\":{},\"bar\":{}}}",
            self.status == Status::Checking,
            json_string(&status),
            json_string(&bar)
        )
    }
}

#[cfg(test)]
mod tests {
    use legible_core::update::Version;

    use super::*;

    const PAGE: &str = "https://github.com/Suremise/legible/releases/tag/v9.0.0";

    fn newer() -> Result<Outcome, String> {
        Ok(Outcome::Newer {
            version: Version::parse("9.0.0").unwrap(),
            url: PAGE.to_owned(),
        })
    }

    #[test]
    fn nothing_is_checked_while_it_is_off() {
        let mut u = Updates::<u8>::default();
        assert_eq!(u.start(false, None), None);
        assert_eq!(u.start(false, Some(1)), None);
        assert_eq!(u.status(), &Status::Idle);
        assert!(!u.bar());
        assert_eq!(
            u.to_json(false),
            r#"{"on":false,"checking":false,"status":"","bar":""}"#
        );
    }

    #[test]
    fn at_start_it_checks_once_and_a_newer_release_shows_its_bar() {
        let mut u = Updates::<u8>::default();
        let r = u.start(true, None).unwrap();
        assert_eq!(
            u.start(true, None),
            None,
            "a check under way isn't started again"
        );
        assert!(u.to_json(true).contains(r#""checking":true"#));
        assert_eq!(u.finish(r, newer(), "0.3.0"), Vec::<u8>::new());
        assert!(u.bar());
        assert_eq!(u.url(), Some(PAGE));
        let json = u.to_json(true);
        assert!(json.contains("Legible 9.0.0 is available."), "{json}");
        assert!(json.contains(r#""checking":false"#), "{json}");
        // Once a run, as Legible starts: another window starting doesn't ask again.
        assert_eq!(u.start(true, None), None);
    }

    #[test]
    fn closing_the_bar_hides_it_till_the_reader_asks_again() {
        let mut u = Updates::<u8>::default();
        let r = u.start(true, None).unwrap();
        u.finish(r, newer(), "0.3.0");
        u.close_bar();
        assert!(!u.bar());
        assert!(u.to_json(true).ends_with(r#""bar":""}"#));
        // Settings still says what was found.
        assert!(u.to_json(true).contains("Legible 9.0.0 is available."));
        let r = u.start(true, Some(2)).unwrap();
        u.finish(r, newer(), "0.3.0");
        assert!(u.bar());
    }

    #[test]
    fn the_windows_that_asked_are_told_the_answer() {
        let mut u = Updates::<u8>::default();
        let r = u.start(true, Some(1)).unwrap();
        // Another window asks while it's under way: no second request, and it's told too.
        assert_eq!(u.start(true, Some(2)), None);
        assert_eq!(u.start(true, Some(1)), None);
        assert_eq!(u.finish(r, Ok(Outcome::UpToDate), "0.3.0"), vec![1, 2]);
        assert_eq!(
            u.told().as_deref(),
            Some("Legible 0.3.0 is the latest version.")
        );
        assert!(!u.bar());
        // Asked again later: a new request; the old one's answer, again, is nobody's.
        let again = u.start(true, Some(1)).unwrap();
        assert_ne!(again, r);
        assert_eq!(
            u.finish(r, Ok(Outcome::UpToDate), "0.3.0"),
            Vec::<u8>::new()
        );
        assert_eq!(u.finish(again, Err("no route".into()), "0.3.0"), vec![1]);
        assert_eq!(
            u.told().as_deref(),
            Some("Legible couldn't check for updates (no route).")
        );
    }

    #[test]
    fn turned_off_it_forgets_and_drops_a_late_answer() {
        let mut u = Updates::<u8>::default();
        let r = u.start(true, Some(1)).unwrap();
        u.turn_off();
        assert_eq!(u.finish(r, newer(), "0.3.0"), Vec::<u8>::new());
        assert_eq!(u.status(), &Status::Idle);
        assert!(!u.bar());
        // Turned on again: it checks again.
        assert!(u.start(true, None).is_some());
    }

    /// Off and on again while a request is under way: the old request's answer is never taken
    /// for the new one's, whichever comes back first.
    #[test]
    fn an_answer_from_before_off_and_on_is_never_the_new_ones() {
        // The old one first: dropped, and the new one's taken.
        let mut u = Updates::<u8>::default();
        let old = u.start(true, Some(1)).unwrap();
        u.turn_off();
        let new = u.start(true, Some(2)).unwrap();
        assert_ne!(old, new);
        assert_eq!(
            u.finish(old, Err("old request failed".into()), "0.3.0"),
            Vec::<u8>::new()
        );
        assert_eq!(u.status(), &Status::Checking);
        assert_eq!(u.finish(new, Ok(Outcome::UpToDate), "0.3.0"), vec![2]);
        assert_eq!(
            u.status(),
            &Status::Latest {
                version: "0.3.0".into()
            }
        );

        // The new one first: taken, and the old one's, coming after, changes nothing.
        let mut u = Updates::<u8>::default();
        let old = u.start(true, None).unwrap();
        u.turn_off();
        let new = u.start(true, Some(2)).unwrap();
        assert_eq!(u.finish(new, Ok(Outcome::UpToDate), "0.3.0"), vec![2]);
        assert_eq!(u.finish(old, newer(), "0.3.0"), Vec::<u8>::new());
        assert!(!u.bar());
        assert_eq!(
            u.status(),
            &Status::Latest {
                version: "0.3.0".into()
            }
        );
    }

    #[test]
    fn a_failed_request_says_why_in_legibles_words_or_windows() {
        let failure = |code: u32, message: &str| http::Failure {
            code,
            message: message.to_owned(),
        };
        assert_eq!(
            reason(&failure(http::TIMEOUT, "")),
            "github.com didn't answer in time"
        );
        for code in [
            http::NAME_NOT_RESOLVED,
            http::CANNOT_CONNECT,
            http::CONNECTION_ERROR,
        ] {
            assert_eq!(
                reason(&failure(code, "")),
                "this PC couldn't reach github.com"
            );
        }
        assert_eq!(
            reason(&failure(5, "Access is denied.\r\n")),
            "Access is denied"
        );
        assert_eq!(reason(&failure(12175, "")), "error 12175");
    }
}
