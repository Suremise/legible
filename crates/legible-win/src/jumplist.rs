//! Legible's own Jump List: the same recent files as the ⋯ menu, in a "Recent files" category
//! of the taskbar's Jump List, under Legible's app identity ([`APP_ID`]), so it works before
//! installation and in the portable zip. Not Windows' automatic Recent category.
//!
//! Each entry is a link to Legible's exe with the file as its one argument, quoted for
//! `CommandLineToArgvW` ([`quote`]), and titled with the file's name. Publishing
//! ([`publish`]) goes through [`Destinations`], Windows' `ICustomDestinationList` in the app
//! (`win32::jumplist`) and a fake in tests, so nothing here touches the real Jump List in a
//! test. How many entries Windows shows is its own choice (its settings and pinned items).

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// Legible's app identity: its taskbar button, and the Jump List that goes with it.
pub const APP_ID: &str = "Surmise.Legible";

/// One link on the Jump List.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The program it starts.
    pub exe: PathBuf,
    /// Its command-line arguments: the file, quoted.
    pub arguments: OsString,
    /// What the Jump List shows: the file's name.
    pub title: OsString,
}

/// Windows' list of custom destinations, or a test's.
pub trait Destinations {
    /// Starts a new list; returns the links the user removed from the old one.
    fn begin(&mut self) -> io::Result<Vec<Link>>;
    /// Adds a category of links.
    fn append(&mut self, category: &str, links: &[Link]) -> io::Result<()>;
    fn commit(&mut self) -> io::Result<()>;
    /// Gives up the list started by `begin`.
    fn abort(&mut self);
}

/// A path as people see it: without Windows' `\\?\` prefix.
pub fn display_path(path: &Path) -> PathBuf {
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    let unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
    if wide.starts_with(&unc) {
        let mut out: Vec<u16> = r"\\".encode_utf16().collect();
        out.extend_from_slice(&wide[unc.len()..]);
        PathBuf::from(OsString::from_wide(&out))
    } else if wide.starts_with(&verbatim) {
        PathBuf::from(OsString::from_wide(&wide[verbatim.len()..]))
    } else {
        path.to_owned()
    }
}

/// The links for `paths`, in order.
pub fn links(exe: &Path, paths: &[PathBuf]) -> Vec<Link> {
    paths
        .iter()
        .map(|path| {
            let shown = display_path(path);
            Link {
                exe: exe.to_owned(),
                arguments: quote(shown.as_os_str()),
                title: shown.file_name().unwrap_or(shown.as_os_str()).to_owned(),
            }
        })
        .collect()
}

/// `arg` as one argument for `CommandLineToArgvW`: in quotes, with each run of backslashes
/// before a quote (or the closing one) doubled, and quotes escaped. Never through a shell.
pub fn quote(arg: &OsStr) -> OsString {
    let wide: Vec<u16> = arg.encode_wide().collect();
    let (quote, backslash) = (u16::from(b'"'), u16::from(b'\\'));
    let mut out = vec![quote];
    let mut backslashes = 0;
    for &c in &wide {
        if c == backslash {
            backslashes += 1;
            continue;
        }
        if c == quote {
            out.extend(std::iter::repeat_n(backslash, backslashes * 2 + 1));
        } else {
            out.extend(std::iter::repeat_n(backslash, backslashes));
        }
        backslashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat_n(backslash, backslashes * 2));
    out.push(quote);
    OsString::from_wide(&out)
}

/// The one argument [`quote`] made, back again; None if it isn't one.
pub fn unquote(quoted: &OsStr) -> Option<OsString> {
    let wide: Vec<u16> = quoted.encode_wide().collect();
    let (quote, backslash) = (u16::from(b'"'), u16::from(b'\\'));
    let inner = wide.strip_prefix(&[quote])?.strip_suffix(&[quote])?;
    let mut out = Vec::new();
    let mut backslashes = 0;
    for &c in inner {
        if c == backslash {
            backslashes += 1;
            continue;
        }
        if c == quote {
            if backslashes % 2 == 0 {
                return None; // an unescaped quote: more than one argument
            }
            out.extend(std::iter::repeat_n(backslash, backslashes / 2));
        } else {
            out.extend(std::iter::repeat_n(backslash, backslashes));
        }
        backslashes = 0;
        out.push(c);
    }
    if backslashes % 2 == 1 {
        return None;
    }
    out.extend(std::iter::repeat_n(backslash, backslashes / 2));
    Some(OsString::from_wide(&out))
}

/// Publishes `paths` as the Jump List's `category`, links to `exe`. The files the user removed
/// from the Jump List since are left out, and returned so the recent list drops them too:
/// matched by their own argument, so removing one never removes the others. If the list
/// can't be built, it's given up and the old one stays.
pub fn publish(
    destinations: &mut dyn Destinations,
    exe: &Path,
    category: &str,
    paths: &[PathBuf],
) -> io::Result<Vec<PathBuf>> {
    let removed = destinations.begin()?;
    let gone: Vec<PathBuf> = removed
        .iter()
        .filter(|link| same_file(&link.exe, exe))
        .filter_map(|link| unquote(&link.arguments).map(PathBuf::from))
        .collect();
    let keep: Vec<PathBuf> = paths
        .iter()
        .filter(|path| !gone.iter().any(|g| same_file(g, &display_path(path))))
        .cloned()
        .collect();
    let built = if keep.is_empty() {
        Ok(())
    } else {
        destinations.append(category, &links(exe, &keep))
    };
    if let Err(error) = built.and_then(|()| destinations.commit()) {
        destinations.abort();
        return Err(error);
    }
    Ok(paths
        .iter()
        .filter(|path| !keep.contains(path))
        .cloned()
        .collect())
}

/// Whether two paths name the same file, as Windows compares names: ignoring case.
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    let fold = |p: &Path| -> Vec<u16> {
        display_path(p)
            .as_os_str()
            .encode_wide()
            .map(|c| {
                if (u16::from(b'a')..=u16::from(b'z')).contains(&c) {
                    c - 32
                } else {
                    c
                }
            })
            .collect()
    };
    fold(a) == fold(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_quoted_for_command_line_to_argv() {
        for (arg, quoted) in [
            (r"C:\Notes\a b.md", r#""C:\Notes\a b.md""#),
            (r"C:\Notes\", r#""C:\Notes\\""#),
            (r#"odd"name"#, r#""odd\"name""#),
            (r#"a\"b"#, r#""a\\\"b""#),
            ("C:\\Привет\\笔记 ü.md", "\"C:\\Привет\\笔记 ü.md\""),
        ] {
            assert_eq!(quote(OsStr::new(arg)), OsString::from(quoted), "{arg}");
            assert_eq!(
                unquote(OsStr::new(quoted)).as_deref(),
                Some(OsStr::new(arg)),
                "{quoted}"
            );
        }
        assert_eq!(unquote(OsStr::new(r#""a" "b""#)), None);
        assert_eq!(unquote(OsStr::new("no quotes")), None);
    }

    #[test]
    fn a_path_without_the_verbatim_prefix() {
        assert_eq!(
            display_path(Path::new(r"\\?\C:\a\b.md")),
            Path::new(r"C:\a\b.md")
        );
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share\b.md")),
            Path::new(r"\\server\share\b.md")
        );
        assert_eq!(display_path(Path::new(r"C:\a.md")), Path::new(r"C:\a.md"));
    }

    #[derive(Default)]
    struct Fake {
        removed: Vec<Link>,
        appended: Vec<(String, Vec<Link>)>,
        committed: bool,
        aborted: bool,
        fail_append: bool,
    }

    impl Destinations for Fake {
        fn begin(&mut self) -> io::Result<Vec<Link>> {
            Ok(self.removed.clone())
        }
        fn append(&mut self, category: &str, links: &[Link]) -> io::Result<()> {
            if self.fail_append {
                return Err(io::Error::other("the shell refused"));
            }
            self.appended.push((category.to_owned(), links.to_vec()));
            Ok(())
        }
        fn commit(&mut self) -> io::Result<()> {
            self.committed = true;
            Ok(())
        }
        fn abort(&mut self) {
            self.aborted = true;
        }
    }

    fn paths() -> Vec<PathBuf> {
        vec![
            PathBuf::from(r"\\?\C:\Notes\b.md"),
            PathBuf::from(r"C:\Notes\a b.md"),
        ]
    }

    #[test]
    fn the_jump_list_is_the_recent_files_in_order() {
        let exe = Path::new(r"C:\Apps\legible.exe");
        let mut fake = Fake::default();
        assert_eq!(
            publish(&mut fake, exe, "Recent files", &paths()).unwrap(),
            Vec::<PathBuf>::new()
        );
        let (category, links) = &fake.appended[0];
        assert_eq!(category, "Recent files");
        assert_eq!(
            links
                .iter()
                .map(|l| (l.title.clone(), l.arguments.clone()))
                .collect::<Vec<_>>(),
            [
                (OsString::from("b.md"), OsString::from(r#""C:\Notes\b.md""#)),
                (
                    OsString::from("a b.md"),
                    OsString::from(r#""C:\Notes\a b.md""#)
                ),
            ]
        );
        assert!(links.iter().all(|l| l.exe == exe));
        assert!(fake.committed && !fake.aborted);
    }

    #[test]
    fn a_file_removed_from_the_jump_list_leaves_the_others() {
        let exe = Path::new(r"C:\Apps\legible.exe");
        let mut fake = Fake {
            removed: vec![
                Link {
                    exe: PathBuf::from(r"c:\apps\LEGIBLE.EXE"),
                    arguments: OsString::from(r#""C:\Notes\b.md""#),
                    title: OsString::from("b.md"),
                },
                // Another program's link, with the same argument: not ours.
                Link {
                    exe: PathBuf::from(r"C:\Other\app.exe"),
                    arguments: OsString::from(r#""C:\Notes\a b.md""#),
                    title: OsString::from("a b.md"),
                },
            ],
            ..Fake::default()
        };
        let gone = publish(&mut fake, exe, "Recent files", &paths()).unwrap();
        assert_eq!(gone, [PathBuf::from(r"\\?\C:\Notes\b.md")]);
        assert_eq!(fake.appended[0].1.len(), 1);
        assert_eq!(fake.appended[0].1[0].title, "a b.md");
    }

    #[test]
    fn a_list_that_cant_be_built_is_given_up() {
        let mut fake = Fake {
            fail_append: true,
            ..Fake::default()
        };
        assert!(
            publish(
                &mut fake,
                Path::new("legible.exe"),
                "Recent files",
                &paths()
            )
            .is_err()
        );
        assert!(fake.aborted && !fake.committed);
    }

    #[test]
    fn no_recent_files_is_an_empty_list() {
        let mut fake = Fake::default();
        publish(&mut fake, Path::new("legible.exe"), "Recent files", &[]).unwrap();
        assert!(fake.appended.is_empty() && fake.committed);
    }
}
