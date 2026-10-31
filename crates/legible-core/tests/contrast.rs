//! The document's and the window's colours pass WCAG AA: every text colour in
//! `web/doc/document.css` and `web/chrome/window.css`, against each background it's shown on,
//! in the light theme and the dark, is at least 4.5:1, and each bar's icon and the toolbar's
//! button group's outline at least 3:1 against what's round it (AA for graphics). The dividers
//! between the toolbar's groups are only drawn (the groups' names say what's grouped), so
//! they're held to plainly visible instead: 1.5:1.
//!
//! The colours are read from the stylesheets themselves: each is a variable written as
//! `--name: light-dark(#light, #dark);`.

use std::collections::HashMap;
use std::path::Path;

/// AA for body text.
const AA: f64 = 4.5;
/// AA for graphics (an icon) against what's round it.
const AA_GRAPHICS: f64 = 3.0;
/// A rule that's only drawn, against what it's on: plain to see, if not a control's boundary.
const DRAWN: f64 = 1.5;

/// The rules only drawn, and what each is on: the dividers between the toolbar's groups.
const DRAWN_RULES: &[(&str, &str)] = &[("toolbar-divider", "toolbar-bg")];

/// The text colours and the backgrounds each one is shown on.
const PAIRS: &[(&str, &[&str])] = &[
    ("text", &["bg", "surface", "stripe", "selection", "code-bg"]),
    ("text-muted", &["bg", "surface", "stripe"]),
    ("accent", &["bg", "stripe"]),
    ("accent-hover", &["bg", "stripe"]),
    ("code-text", &["code-bg"]),
    ("caution", &["bg"]),
    ("on-accent", &["accent"]),
    // The window: its toolbar strip (and the find bar on it), and the information bars.
    ("text", &["toolbar-bg", "info-bg", "warning-bg"]),
    ("text-muted", &["toolbar-bg"]),
];

/// Each bar's icon, and the bar it's on; Split's divider, its handle in the accent on either
/// pane (both are the page's background); and the outline of the toolbar's button group.
const ICONS: &[(&str, &str)] = &[
    ("accent", "info-bg"),
    ("accent", "bg"),
    ("warning", "warning-bg"),
    ("caution", "bg"),
    ("group-border", "toolbar-bg"),
];

const ALERTS: &[&str] = &["note", "tip", "important", "warning", "caution"];

/// Each variable's (light, dark) colour, from both stylesheets. The window's may also hold
/// colours that aren't for text (a shadow's), which aren't written as `#rrggbb`: those are left
/// out.
fn palette() -> HashMap<String, ([u8; 3], [u8; 3])> {
    let mut colours = HashMap::new();
    read_palette("../../web/doc/document.css", true, &mut colours);
    read_palette("../../web/chrome/window.css", false, &mut colours);
    colours
}

fn read_palette(file: &str, all_hex: bool, colours: &mut HashMap<String, ([u8; 3], [u8; 3])>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let css = std::fs::read_to_string(&path).unwrap();
    for line in css.lines() {
        let line = line.trim();
        let Some((name, value)) = line.strip_prefix("--").and_then(|l| l.split_once(':')) else {
            continue;
        };
        let Some(pair) = value
            .trim()
            .strip_prefix("light-dark(")
            .and_then(|v| v.strip_suffix(");"))
        else {
            continue;
        };
        if !all_hex && !pair.trim_start().starts_with('#') {
            continue;
        }
        let (light, dark) = pair.split_once(',').expect(line);
        colours.insert(name.to_owned(), (hex(light), hex(dark)));
    }
}

fn hex(value: &str) -> [u8; 3] {
    let digits = value.trim().strip_prefix('#').expect(value);
    assert_eq!(digits.len(), 6, "{value}: write colours as #rrggbb");
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).expect(value);
    [byte(0), byte(2), byte(4)]
}

fn luminance(rgb: [u8; 3]) -> f64 {
    let channel = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
}

fn ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn every_text_colour_passes_aa_in_both_themes() {
    let colours = palette();
    let get = |name: &str| {
        *colours
            .get(name)
            .unwrap_or_else(|| panic!("--{name} is missing"))
    };

    let mut pairs: Vec<(String, String)> = Vec::new();
    for (text, backgrounds) in PAIRS {
        for bg in *backgrounds {
            pairs.push(((*text).into(), (*bg).into()));
        }
    }
    // Every highlight.js token on the code panel.
    for name in colours.keys().filter(|n| n.starts_with("hl-")) {
        pairs.push((name.clone(), "code-bg".into()));
    }
    // Each alert's title on its tint, and the body text on it.
    for kind in ALERTS {
        pairs.push(((*kind).into(), format!("{kind}-bg")));
        pairs.push(("text".into(), format!("{kind}-bg")));
    }
    assert!(
        pairs.len() > 30,
        "too few pairs: the stylesheet wasn't read"
    );

    let mut failures = Vec::new();
    let checks = pairs
        .iter()
        .map(|(text, bg)| (text.as_str(), bg.as_str(), AA));
    let icons = ICONS.iter().map(|(icon, bg)| (*icon, *bg, AA_GRAPHICS));
    let drawn = DRAWN_RULES.iter().map(|(rule, bg)| (*rule, *bg, DRAWN));
    for (text, bg, least) in checks.chain(icons).chain(drawn) {
        let (text_light, text_dark) = get(text);
        let (bg_light, bg_dark) = get(bg);
        for (theme, fg, back) in [
            ("light", text_light, bg_light),
            ("dark", text_dark, bg_dark),
        ] {
            let r = ratio(fg, back);
            if r < least {
                failures.push(format!(
                    "{theme}: --{text} on --{bg} is {r:.2}:1, under {least}:1"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "too little contrast:\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_ratio_matches_wcag() {
    assert!((ratio([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
    assert!((ratio([0x76, 0x76, 0x76], [255, 255, 255]) - 4.54).abs() < 0.01);
}
