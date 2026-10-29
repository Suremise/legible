//! A document window's size and place: a comfortable size for reading the first time, then the
//! size, position and maximised state the user last left a window at, kept in `window.toml` in
//! Legible's folder (Settings' "Open windows at their last size and position", on by default).
//!
//! Sizes are in logical pixels (at 100% scaling), so they mean the same after the scaling or
//! the screen changes. Positions are the window's outer top-left in physical pixels on the
//! virtual screen, as Windows gives them. A remembered position goes back on the monitor the
//! window was on, moved fully onto its work area; if that monitor has gone (or the position is
//! off every screen), the window is centred on the primary. With nothing remembered, or the
//! setting off, Windows places it, cascading.

use std::fs;
use std::io;
use std::path::Path;

/// The file the size is kept in, in Legible's folder.
pub const FILE: &str = "window.toml";

/// The first window's size, before any is remembered: about a page's width, and tall.
pub const FIRST: Size = Size {
    width: 900.0,
    height: 1000.0,
};
/// The most of the work area the first window takes.
pub const FIRST_SHARE: f64 = 0.85;
/// The smallest size a window is given.
pub const MIN: Size = Size {
    width: 400.0,
    height: 300.0,
};

/// A size in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// A rectangle on the virtual screen, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    fn overlap(&self, other: &Rect) -> i64 {
        let width = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let height = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        if width <= 0 || height <= 0 {
            0
        } else {
            i64::from(width) * i64::from(height)
        }
    }
}

/// A monitor, as a new window is placed on it: its work area (less the taskbar) and its
/// scaling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    pub work: Rect,
    pub scale: f64,
}

impl Monitor {
    /// Its work area in logical pixels.
    fn logical_work(&self) -> Size {
        Size {
            width: f64::from(self.work.width) / self.scale,
            height: f64::from(self.work.height) / self.scale,
        }
    }

    /// `size` (logical) in this monitor's physical pixels.
    fn physical(&self, size: Size) -> (i32, i32) {
        (
            (size.width * self.scale).round() as i32,
            (size.height * self.scale).round() as i32,
        )
    }
}

/// A window's size, where it was, and whether it was maximised.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub size: Size,
    pub maximized: bool,
    /// Its outer top-left, in physical pixels; None: Windows places it.
    pub position: Option<(i32, i32)>,
}

impl Placement {
    /// The placement for a new window, on `monitors` (the primary one at `primary`): the
    /// remembered one, its size kept within the work area of the monitor it goes on, and its
    /// position moved fully onto that monitor, or centred on the primary if that monitor has
    /// gone; or, with nothing remembered, the first size, within 85% of the primary's work area,
    /// placed by Windows.
    pub fn for_new_window(
        remembered: Option<Placement>,
        monitors: &[Monitor],
        primary: Option<usize>,
    ) -> Placement {
        let primary = primary.and_then(|i| monitors.get(i));
        let Some(p) = remembered else {
            return Placement {
                size: fit(FIRST, primary.map(Monitor::logical_work), FIRST_SHARE),
                maximized: false,
                position: None,
            };
        };
        let Some(position) = p.position else {
            return Placement {
                size: fit(p.size, primary.map(Monitor::logical_work), 1.0),
                position: None,
                ..p
            };
        };
        // The monitor it was on: the one the remembered window overlaps most.
        let on = monitors
            .iter()
            .map(|m| {
                let (width, height) = m.physical(p.size);
                let rect = Rect {
                    x: position.0,
                    y: position.1,
                    width,
                    height,
                };
                (rect.overlap(&m.work), m)
            })
            .filter(|(overlap, _)| *overlap > 0)
            .max_by_key(|(overlap, _)| *overlap)
            .map(|(_, m)| m);
        let Some(monitor) = on.or(primary) else {
            return Placement {
                size: fit(p.size, None, 1.0),
                position: None,
                ..p
            };
        };
        let size = fit(p.size, Some(monitor.logical_work()), 1.0);
        let (width, height) = monitor.physical(size);
        let work = monitor.work;
        let position = if on.is_some() {
            (
                position
                    .0
                    .clamp(work.x, work.x + (work.width - width).max(0)),
                position
                    .1
                    .clamp(work.y, work.y + (work.height - height).max(0)),
            )
        } else {
            (
                work.x + (work.width - width).max(0) / 2,
                work.y + (work.height - height).max(0) / 2,
            )
        };
        Placement {
            size,
            maximized: p.maximized,
            position: Some(position),
        }
    }

    /// The remembered placement in `file`, if there is one that makes sense.
    pub fn load(file: &Path) -> Option<Placement> {
        let text = fs::read_to_string(file).ok()?;
        let (mut width, mut height, mut maximized) = (None, None, false);
        let (mut x, mut y) = (None, None);
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "width" => width = value.parse::<f64>().ok(),
                "height" => height = value.parse::<f64>().ok(),
                "maximized" => maximized = value == "true",
                "x" => x = value.parse::<i32>().ok(),
                "y" => y = value.parse::<i32>().ok(),
                _ => {}
            }
        }
        let size = Size {
            width: width?,
            height: height?,
        };
        let sensible = |v: f64| v.is_finite() && v > 0.0 && v < 100_000.0;
        (sensible(size.width) && sensible(size.height)).then_some(Placement {
            size,
            maximized,
            position: x.zip(y),
        })
    }

    pub fn save(&self, file: &Path) -> io::Result<()> {
        let mut text = format!(
            "# The size and place Legible's windows open at, kept from the last one closed.\n\
             width = {}\nheight = {}\nmaximized = {}\n",
            self.size.width.round(),
            self.size.height.round(),
            self.maximized
        );
        if let Some((x, y)) = self.position {
            text.push_str(&format!("x = {x}\ny = {y}\n"));
        }
        fs::write(file, text)
    }
}

/// Where a new window whose whole outer rectangle (its frame, title and menu bar included) is
/// `outer` goes to be fully on the monitor it overlaps most: that rectangle, no bigger than the
/// work area and moved onto it. None if it's fully on it already, or on no monitor (Windows has
/// put it somewhere of its own).
pub fn onto_monitor(outer: Rect, monitors: &[Monitor]) -> Option<Rect> {
    let monitor = monitors
        .iter()
        .map(|m| (outer.overlap(&m.work), m))
        .filter(|(overlap, _)| *overlap > 0)
        .max_by_key(|(overlap, _)| *overlap)
        .map(|(_, m)| m)?;
    let work = monitor.work;
    let (width, height) = (outer.width.min(work.width), outer.height.min(work.height));
    let fitted = Rect {
        x: outer.x.clamp(work.x, work.x + work.width - width),
        y: outer.y.clamp(work.y, work.y + work.height - height),
        width,
        height,
    };
    (fitted != outer).then_some(fitted)
}

/// `size`, no bigger than `share` of `work` (when it's known) and no smaller than [`MIN`].
fn fit(size: Size, work: Option<Size>, share: f64) -> Size {
    let (mut width, mut height) = (size.width, size.height);
    if let Some(work) = work {
        width = width.min(work.width * share);
        height = height.min(work.height * share);
    }
    Size {
        width: width.max(MIN.width).round(),
        height: height.max(MIN.height).round(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: f64, height: f64) -> Size {
        Size { width, height }
    }

    fn monitor(x: i32, y: i32, width: i32, height: i32, scale: f64) -> Monitor {
        Monitor {
            work: Rect {
                x,
                y,
                width,
                height,
            },
            scale,
        }
    }

    /// A 2560×1440 primary at 100% (its taskbar 48 px), and a 1920×1080 one at 150% to its left.
    fn two_monitors() -> Vec<Monitor> {
        vec![
            monitor(0, 0, 2560, 1392, 1.0),
            monitor(-1920, 0, 1920, 1032, 1.5),
        ]
    }

    fn remembered(width: f64, height: f64, position: Option<(i32, i32)>) -> Placement {
        Placement {
            size: size(width, height),
            maximized: false,
            position,
        }
    }

    #[test]
    fn the_first_window_fits_within_85_percent_and_windows_places_it() {
        let big = Placement::for_new_window(None, &[monitor(0, 0, 2560, 1400, 1.0)], Some(0));
        assert_eq!((big.size, big.position), (FIRST, None));
        // A 1366×768 laptop's work area, less the taskbar.
        let small = Placement::for_new_window(None, &[monitor(0, 0, 1366, 720, 1.0)], Some(0));
        assert_eq!(small.size, size(900.0, 612.0));
        assert!(!small.maximized);
        assert_eq!(Placement::for_new_window(None, &[], None).size, FIRST);
    }

    #[test]
    fn a_remembered_window_goes_back_where_it_was() {
        let monitors = two_monitors();
        let on_primary = remembered(1000.0, 800.0, Some((300, 200)));
        let placed = Placement::for_new_window(Some(on_primary), &monitors, Some(0));
        assert_eq!(placed, on_primary);
        // On the second monitor, at its scaling: 1000×600 logical is 1500×900 physical there.
        let on_second = remembered(1000.0, 600.0, Some((-1800, 100)));
        let placed = Placement::for_new_window(Some(on_second), &monitors, Some(0));
        assert_eq!(placed, on_second);
    }

    #[test]
    fn a_window_partly_off_its_monitor_is_moved_fully_onto_it() {
        let monitors = two_monitors();
        // Hanging off the primary's right and bottom edges: moved in, the same size.
        let off = remembered(1000.0, 800.0, Some((2000, 900)));
        let placed = Placement::for_new_window(Some(off), &monitors, Some(0));
        assert_eq!(placed.position, Some((1560, 592)));
        assert_eq!(placed.size, size(1000.0, 800.0));
        // Mostly on the second monitor, but over its left edge: moved onto it, not the primary.
        let left = remembered(1000.0, 600.0, Some((-2100, 50)));
        let placed = Placement::for_new_window(Some(left), &monitors, Some(0));
        assert_eq!(placed.position, Some((-1920, 50)));
        // Bigger than the monitor it goes on: as big as its work area, at its corner.
        let huge = remembered(3000.0, 2000.0, Some((100, 100)));
        let placed = Placement::for_new_window(Some(huge), &monitors, Some(0));
        assert_eq!(
            (placed.size, placed.position),
            (size(2560.0, 1392.0), Some((0, 0)))
        );
    }

    #[test]
    fn a_window_whose_monitor_has_gone_is_centred_on_the_primary() {
        // It was on the second monitor, which is unplugged now.
        let gone = remembered(1000.0, 600.0, Some((-1800, 100)));
        let primary_only = [monitor(0, 0, 2560, 1392, 1.0)];
        let placed = Placement::for_new_window(Some(gone), &primary_only, Some(0));
        assert_eq!(placed.position, Some((780, 396)));
        // Off every screen (a position from a screen far away): centred too, kept maximised.
        let far = Placement {
            maximized: true,
            ..remembered(800.0, 600.0, Some((90_000, -90_000)))
        };
        let placed = Placement::for_new_window(Some(far), &two_monitors(), Some(0));
        assert_eq!(placed.position, Some((880, 396)));
        assert!(placed.maximized);
        // No monitors known: Windows places it, at the remembered size.
        let placed = Placement::for_new_window(Some(gone), &[], None);
        assert_eq!((placed.size, placed.position), (size(1000.0, 600.0), None));
    }

    #[test]
    fn a_new_windows_frame_is_kept_on_its_monitor_too() {
        let monitors = two_monitors();
        let rect = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        // Its inner size fitted, but the frame, title and menu bar hang 16 px right and 59 px
        // below the primary's work area: moved up and left by just that.
        assert_eq!(
            onto_monitor(rect(1920, 912, 656, 539), &monitors),
            Some(rect(1904, 853, 656, 539))
        );
        // Fully on its monitor already (the second one, at 150%): left where it is.
        assert_eq!(onto_monitor(rect(-1800, 100, 1516, 900), &monitors), None);
        // Over the second's left edge: onto it.
        assert_eq!(
            onto_monitor(rect(-1950, 0, 1516, 959), &monitors),
            Some(rect(-1920, 0, 1516, 959))
        );
        // Its inner size the whole work area, its frame more: as big as the work area, on it.
        assert_eq!(
            onto_monitor(rect(-8, 0, 2576, 1451), &monitors),
            Some(rect(0, 0, 2560, 1392))
        );
        // On no monitor: Windows' own place, left alone.
        assert_eq!(onto_monitor(rect(90_000, 0, 600, 400), &monitors), None);
    }

    #[test]
    fn a_remembered_size_without_a_place_is_kept_within_the_work_area() {
        let remembered = Placement {
            size: size(1800.0, 1200.0),
            maximized: true,
            position: None,
        };
        let placed =
            Placement::for_new_window(Some(remembered), &[monitor(0, 0, 1600, 860, 1.0)], Some(0));
        assert_eq!((placed.size, placed.position), (size(1600.0, 860.0), None));
        assert!(placed.maximized);
        let tiny = Placement {
            size: size(10.0, 10.0),
            maximized: false,
            position: None,
        };
        assert_eq!(Placement::for_new_window(Some(tiny), &[], None).size, MIN);
    }

    #[test]
    fn it_survives_a_save_and_ignores_nonsense() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join(FILE);
        assert_eq!(Placement::load(&file), None);
        let placement = Placement {
            size: size(1000.0, 800.0),
            maximized: true,
            position: Some((-1500, 40)),
        };
        placement.save(&file).unwrap();
        assert_eq!(Placement::load(&file), Some(placement));
        // A file from before positions were kept: the size alone.
        fs::write(&file, "width = 1000\nheight = 800\nmaximized = false\n").unwrap();
        assert_eq!(
            Placement::load(&file),
            Some(remembered(1000.0, 800.0, None))
        );
        fs::write(&file, "width = -5\nheight = 800\n").unwrap();
        assert_eq!(Placement::load(&file), None);
        fs::write(&file, "width = NaN\nheight = 800\n").unwrap();
        assert_eq!(Placement::load(&file), None);
        fs::write(&file, "nonsense").unwrap();
        assert_eq!(Placement::load(&file), None);
    }
}
