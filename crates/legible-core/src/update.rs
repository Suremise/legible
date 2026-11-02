//! The update check's logic: which release is the latest, and whether it's newer than this one.
//! Platform-free; `legible-win` makes the one request, and only when the reader has turned
//! "Check for updates" on in Settings.
//!
//! The request is for [`LATEST_URL`], with redirects not followed: GitHub answers with a
//! redirect to the latest release's page (`…/releases/tag/v0.3.0`), so its `Location` says the
//! version, and that page is the link the reader is shown. No API, no JSON, nothing downloaded.

/// The project's latest release on GitHub, which redirects to that release's page.
pub const LATEST_URL: &str = "https://github.com/Suremise/legible/releases/latest";

/// [`LATEST_URL`]'s host and path, for the request.
pub const HOST: &str = "github.com";
pub const LATEST_PATH: &str = "/Suremise/legible/releases/latest";

/// Where every release is listed: the link when the latest can't be told.
pub const RELEASES_URL: &str = "https://github.com/Suremise/legible/releases";

/// A release version: major, minor, patch. A pre-release (`0.2.0-beta.1`) counts as before
/// its release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// The text after `-`, if any.
    pub pre: Option<String>,
}

impl Version {
    /// Reads `0.2.0`, `v0.2.0` or `0.2.0-beta.1` (a `+build` suffix is ignored).
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let text = text.split('+').next()?;
        let (core, pre) = match text.split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, Some(p.to_owned())),
            Some(_) => return None,
            None => (text, None),
        };
        let mut parts = core.split('.');
        let mut number = || -> Option<u64> {
            let p = parts.next()?;
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            p.parse().ok()
        };
        let (major, minor, patch) = (number()?, number()?, number()?);
        if parts.next().is_some() {
            return None;
        }
        Some(Version {
            major,
            minor,
            patch,
            pre,
        })
    }

    /// Whether this version comes after `other`.
    pub fn is_newer_than(&self, other: &Version) -> bool {
        let a = (self.major, self.minor, self.patch);
        let b = (other.major, other.minor, other.patch);
        if a != b {
            return a > b;
        }
        // The same numbers: a release comes after its pre-releases.
        match (&self.pre, &other.pre) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => x > y,
            _ => false,
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A newer release, and its page.
    Newer { version: Version, url: String },
    /// This is the latest (or newer).
    UpToDate,
}

/// The latest release from the redirect GitHub answered [`LATEST_URL`] with: its `Location`
/// (`https://github.com/Suremise/legible/releases/tag/v0.2.0`). `None` if it doesn't name a
/// release of this project (no release yet redirects back to the releases list).
pub fn latest_from_location(location: &str) -> Option<(Version, String)> {
    let location = location.trim();
    let tag = location.strip_prefix("https://github.com/Suremise/legible/releases/tag/")?;
    let tag = tag.split(['?', '#']).next()?;
    if tag.is_empty() || tag.contains('/') {
        return None;
    }
    Some((Version::parse(tag)?, location.to_owned()))
}

/// Compares the latest release with `current` (this build's version).
pub fn outcome(current: &str, latest: Version, url: String) -> Option<Outcome> {
    let current = Version::parse(current)?;
    Some(if latest.is_newer_than(&current) {
        Outcome::Newer {
            version: latest,
            url,
        }
    } else {
        Outcome::UpToDate
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn reads_versions_with_or_without_a_v() {
        assert_eq!(
            v("v0.2.0"),
            Version {
                major: 0,
                minor: 2,
                patch: 0,
                pre: None
            }
        );
        assert_eq!(v("1.10.3").to_string(), "1.10.3");
        assert_eq!(v("0.2.0-beta.1").pre.as_deref(), Some("beta.1"));
        assert_eq!(v("0.2.0+abc").to_string(), "0.2.0");
        for bad in [
            "", "v", "1.2", "1.2.3.4", "1.x.3", "1.2.3-", "latest", "-1.2.3",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn newer_compares_numbers_then_pre_releases() {
        assert!(v("0.2.0").is_newer_than(&v("0.1.9")));
        assert!(v("0.10.0").is_newer_than(&v("0.9.0")));
        assert!(v("1.0.0").is_newer_than(&v("0.99.99")));
        assert!(!v("0.1.0").is_newer_than(&v("0.1.0")));
        assert!(!v("0.1.0").is_newer_than(&v("0.2.0")));
        assert!(v("0.2.0").is_newer_than(&v("0.2.0-beta.1")));
        assert!(!v("0.2.0-beta.1").is_newer_than(&v("0.2.0")));
        assert!(v("0.2.0-beta.2").is_newer_than(&v("0.2.0-beta.1")));
    }

    #[test]
    fn the_latest_comes_from_the_redirect() {
        let url = "https://github.com/Suremise/legible/releases/tag/v0.2.0";
        assert_eq!(
            latest_from_location(url),
            Some((v("0.2.0"), url.to_owned()))
        );
        // No release yet: GitHub sends the list back.
        assert_eq!(
            latest_from_location("https://github.com/Suremise/legible/releases"),
            None
        );
        // Another project's release, or something that isn't a version, isn't taken.
        assert_eq!(
            latest_from_location("https://github.com/someone/else/releases/tag/v9.0.0"),
            None
        );
        assert_eq!(
            latest_from_location("https://github.com/Suremise/legible/releases/tag/nightly"),
            None
        );
    }

    #[test]
    fn the_outcome_says_newer_only_when_it_is() {
        let url = "https://github.com/Suremise/legible/releases/tag/v0.2.0".to_owned();
        assert_eq!(
            outcome("0.1.0", v("0.2.0"), url.clone()),
            Some(Outcome::Newer {
                version: v("0.2.0"),
                url: url.clone()
            })
        );
        assert_eq!(
            outcome("0.2.0", v("0.2.0"), url.clone()),
            Some(Outcome::UpToDate)
        );
        assert_eq!(outcome("0.3.0", v("0.2.0"), url), Some(Outcome::UpToDate));
    }
}
