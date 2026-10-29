//! Where the reader was in each file (Settings' "Remember where I was in each file", on by
//! default): the place the page last said, for the [`MAX`] files closed most recently, kept in
//! `places.txt` in Legible's folder and given back to the page when a file opens again.
//!
//! A place is the page's anchor, as live reload keeps the reader's place (chrome.js
//! `takeAnchor`): the heading nearest the top of the page under the chrome, how far from it, and
//! how far down the page as a fraction (for a document without headings, or one whose heading
//! has gone), never a raw pixel offset alone. Files are kept by their full path, as the window
//! has it; a file that has gone leaves the list. Turning the setting off clears it.
//!
//! The file is a line a place, newest first: fraction, offset, heading and path, tab-separated
//! (a path can't hold a tab or a line break). Saving writes a temporary file beside it and
//! renames it over the old one.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The file the places are kept in, in Legible's folder.
pub const FILE: &str = "places.txt";
/// How many files' places are kept.
pub const MAX: usize = 100;

/// A place in a document, as the page takes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// The heading nearest the top (its `id`), if the document has one.
    pub heading: Option<String>,
    /// How far the top of the page is below that heading, in CSS pixels.
    pub offset: f64,
    /// How far down the page, from 0 to 1.
    pub fraction: f64,
}

impl Place {
    /// The place in the page's message, after its name: `<fraction>\n<offset>\n<heading>` (an
    /// empty heading: none). None if it makes no sense.
    pub fn parse(rest: &str) -> Option<Place> {
        let mut parts = rest.splitn(3, '\n');
        let fraction = parts.next()?.parse::<f64>().ok()?;
        let offset = parts.next()?.parse::<f64>().ok()?;
        let heading = parts.next()?;
        Place::of(heading, offset, fraction)
    }

    fn of(heading: &str, offset: f64, fraction: f64) -> Option<Place> {
        let sensible = offset.is_finite() && offset.abs() < 1e7 && fraction.is_finite();
        let plain = !heading.contains(['\t', '\n', '\r']) && heading.len() <= 512;
        (sensible && plain).then(|| Place {
            heading: (!heading.is_empty()).then(|| heading.to_owned()),
            offset,
            fraction: fraction.clamp(0.0, 1.0),
        })
    }

    /// The place as the page's `restoreAnchor` takes it: JSON, safe in a script.
    pub fn to_json(&self) -> String {
        let heading = self
            .heading
            .as_deref()
            .map_or("null".to_owned(), crate::settings::json_string);
        format!(
            "{{\"id\":{heading},\"offset\":{},\"fraction\":{}}}",
            self.offset, self.fraction
        )
    }
}

/// The text of a script's result that is a JSON string (as WebView2 gives a script's result):
/// the page's place as its message has it, after its name. None for anything else (`null`: no
/// place to give, as in Split or Edit).
pub fn from_script(json: &str) -> Option<String> {
    let inner = json.trim().strip_prefix('"')?.strip_suffix('"')?;
    let mut text = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match chars.next()? {
            'n' => text.push('\n'),
            't' => text.push('\t'),
            'r' => text.push('\r'),
            'b' => text.push('\u{8}'),
            'f' => text.push('\u{c}'),
            'u' => {
                let code: String = chars.by_ref().take(4).collect();
                let unit = u16::from_str_radix(&code, 16).ok()?;
                // A surrogate pair is two escapes: the second follows at once.
                if (0xD800..0xDC00).contains(&unit) {
                    let (Some('\\'), Some('u')) = (chars.next(), chars.next()) else {
                        return None;
                    };
                    let low: String = chars.by_ref().take(4).collect();
                    let low = u16::from_str_radix(&low, 16).ok()?;
                    text.push(char::decode_utf16([unit, low]).next()?.ok()?);
                } else {
                    text.push(char::from_u32(u32::from(unit))?);
                }
            }
            other => text.push(other),
        }
    }
    Some(text)
}

/// The places kept, newest first.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Places {
    entries: Vec<(PathBuf, Place)>,
}

impl Places {
    /// The places in `file`; lines that make no sense are dropped.
    pub fn load(file: &Path) -> Places {
        let Ok(text) = fs::read_to_string(file) else {
            return Places::default();
        };
        let mut places = Places::default();
        for line in text.lines() {
            let mut parts = line.splitn(4, '\t');
            let (Some(fraction), Some(offset), Some(heading), Some(path)) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let (Ok(fraction), Ok(offset)) = (fraction.parse(), offset.parse()) else {
                continue;
            };
            if path.is_empty() || places.entries.len() >= MAX {
                continue;
            }
            if let Some(place) = Place::of(heading, offset, fraction) {
                places.entries.push((PathBuf::from(path), place));
            }
        }
        places
    }

    /// Saves to `file`: a temporary file beside it, renamed over it. With nothing kept, the file
    /// goes.
    pub fn save(&self, file: &Path) -> io::Result<()> {
        if self.entries.is_empty() {
            return match fs::remove_file(file) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        let mut text = String::new();
        for (path, place) in &self.entries {
            text.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                place.fraction,
                place.offset,
                place.heading.as_deref().unwrap_or(""),
                path.display()
            ));
        }
        let temp = file.with_extension("txt.tmp");
        fs::write(&temp, text)?;
        fs::rename(&temp, file).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
    }

    /// Where the reader was in `path`.
    pub fn get(&self, path: &Path) -> Option<&Place> {
        self.entries
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, place)| place)
    }

    /// The reader was at `place` in `path`: it's the newest, and the oldest beyond [`MAX`] go.
    pub fn set(&mut self, path: &Path, place: Place) {
        self.entries.retain(|(p, _)| p != path);
        self.entries.insert(0, (path.to_owned(), place));
        self.entries.truncate(MAX);
    }

    /// Drops the places of `gone` files; whether any went.
    pub fn remove(&mut self, gone: &[PathBuf]) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(p, _)| !gone.contains(p));
        self.entries.len() != before
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The files kept, newest first.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.entries.iter().map(|(p, _)| p.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(heading: &str, offset: f64, fraction: f64) -> Place {
        Place::of(heading, offset, fraction).unwrap()
    }

    #[test]
    fn the_pages_message_is_its_anchor_and_nothing_else() {
        assert_eq!(
            Place::parse("0.4\n12.5\nuser-content-summary"),
            Some(place("user-content-summary", 12.5, 0.4))
        );
        // No heading: the fraction alone places it.
        assert_eq!(Place::parse("0.25\n0\n").unwrap().heading, None);
        for nonsense in [
            "",
            "x\n1\nh",
            "0.5\nNaN\nh",
            "0.5\n1",
            "0.5\n1\nh\tidden",
            "1e400\n0\nh",
        ] {
            assert_eq!(Place::parse(nonsense), None, "{nonsense:?}");
        }
        // A fraction outside the page is brought onto it.
        assert_eq!(Place::parse("7\n0\n").unwrap().fraction, 1.0);
    }

    #[test]
    fn a_closing_pages_answer_is_read_as_its_message_is() {
        assert_eq!(
            from_script(r#""0.4\n12.5\nuser-content-summary""#).as_deref(),
            Some("0.4\n12.5\nuser-content-summary")
        );
        assert_eq!(
            Place::parse(&from_script(r#""0.4\n12.5\nuser-content-summary""#).unwrap()),
            Some(place("user-content-summary", 12.5, 0.4))
        );
        // Escapes a heading's id might need, a character beyond the BMP included.
        assert_eq!(
            from_script(r#""1\n0\nq\"\\é😀""#).as_deref(),
            Some("1\n0\nq\"\\é😀")
        );
        // Nothing to give (Split or Edit, the setting off), or nothing came.
        for none in ["null", "", "{}", r#""unfinished\"#, r#""\ud83d""#] {
            assert_eq!(from_script(none), None, "{none:?}");
        }
    }

    #[test]
    fn a_place_is_json_the_page_can_take_safely() {
        assert_eq!(
            place("a\"</script>", -3.0, 0.5).to_json(),
            r#"{"id":"a\"\u003c/script\u003e","offset":-3,"fraction":0.5}"#
        );
        assert_eq!(
            place("", 0.0, 1.0).to_json(),
            r#"{"id":null,"offset":0,"fraction":1}"#
        );
    }

    #[test]
    fn the_newest_hundred_files_are_kept_newest_first() {
        let mut places = Places::default();
        for n in 0..(MAX + 20) {
            places.set(
                &PathBuf::from(format!(r"C:\Docs\{n}.md")),
                place("h", 0.0, 0.5),
            );
        }
        let paths = places.paths();
        assert_eq!(paths.len(), MAX);
        assert_eq!(paths[0], PathBuf::from(r"C:\Docs\119.md"));
        assert_eq!(paths[MAX - 1], PathBuf::from(r"C:\Docs\20.md"));
        assert!(
            places.get(Path::new(r"C:\Docs\19.md")).is_none(),
            "the oldest went"
        );
        // A file's place again: it's the newest, once.
        places.set(Path::new(r"C:\Docs\50.md"), place("later", 4.0, 0.9));
        let paths = places.paths();
        assert_eq!(
            (paths.len(), &paths[0]),
            (MAX, &PathBuf::from(r"C:\Docs\50.md"))
        );
        assert_eq!(
            places.get(Path::new(r"C:\Docs\50.md")),
            Some(&place("later", 4.0, 0.9))
        );
    }

    #[test]
    fn places_survive_a_save_files_that_went_leave_and_clearing_removes_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join(FILE);
        assert_eq!(Places::load(&file), Places::default());
        let mut places = Places::default();
        let (a, b) = (
            PathBuf::from(r"C:\Docs\a b.md"),
            PathBuf::from(r"\\server\share\b.md"),
        );
        places.set(&a, place("user-content-intro", 8.0, 0.1));
        places.set(&b, place("", 0.0, 0.75));
        places.save(&file).unwrap();
        let back = Places::load(&file);
        assert_eq!(back, places);
        assert_eq!(back.paths(), [b.clone(), a.clone()]);
        // Lines that make no sense are dropped, the rest kept.
        fs::write(
            &file,
            "x\t1\th\tC:\\x.md\n0.5\t1\th\n0.5\t2\th\tC:\\ok.md\n",
        )
        .unwrap();
        assert_eq!(Places::load(&file).paths(), [PathBuf::from(r"C:\ok.md")]);
        // A file that has gone leaves the list.
        let mut places = back;
        assert!(places.remove(std::slice::from_ref(&b)));
        assert!(!places.remove(std::slice::from_ref(&b)));
        assert_eq!(places.paths(), [a]);
        // Cleared (the setting turned off): nothing kept, and no file.
        places.save(&file).unwrap();
        places.clear();
        places.save(&file).unwrap();
        assert!(!file.exists());
        places.save(&file).unwrap();
    }
}
