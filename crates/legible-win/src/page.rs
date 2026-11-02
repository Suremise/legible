//! The page a document window shows, and the rules it runs under.
//!
//! Each window's WebView2 has two origins, served by custom protocols (wry gives a protocol
//! `name` the origin `https://name.localhost` on Windows):
//!
//! - **`https://legible.localhost`**, Legible's own: the page itself at `/`, and the embedded
//!   assets (`/doc/…`, `/chrome/…`, `/js/…`, from [`legible_core::assets`]);
//! - **`https://legible-doc.localhost`**, the document's images, only from inside its folder
//!   ([`crate::confine`]).
//!
//! The [`CSP`] lets the page load scripts and styles only from Legible's origin, images only
//! from the document's origin or `data:`, and one frame, Mermaid's ([`MERMAID_FRAME`]);
//! nothing else, from anywhere. Mermaid draws diagrams in that frame, sandboxed under a CSP of
//! its own ([`MERMAID_FRAME_CSP`]), so it never touches the page (docs/ARCHITECTURE.md, "The
//! document's look").

use legible_core::assets;
use legible_core::i18n::{PluralForms, tr, tr_plurals};
use legible_core::render::script_strings;
use legible_core::tr;

use crate::settings::{Settings, json_string};

/// Legible's own origin: the page and its assets.
pub const APP_ORIGIN: &str = "https://legible.localhost";
/// The document's images.
pub const DOC_ORIGIN: &str = "https://legible-doc.localhost";
/// The protocol names behind those origins.
pub const APP_PROTOCOL: &str = "legible";
pub const DOC_PROTOCOL: &str = "legible-doc";
/// The URL a window's page is loaded from: `/` on [`APP_ORIGIN`], where wry serves Legible's
/// protocol (`load_url` takes it as it is, so it's the https form).
pub const START_URL: &str = "https://legible.localhost/";

/// The page's Content Security Policy, in its `<meta>` and its response header alike.
pub const CSP: &str = "default-src 'none'; \
    script-src https://legible.localhost; \
    style-src https://legible.localhost; \
    img-src https://legible-doc.localhost data:; \
    frame-src https://legible.localhost/doc/mermaid-frame.html; \
    base-uri 'none'; \
    form-action 'none'";
/// [`CSP`]'s `img-src`, which consented pictures' origins are added to ([`csp`]).
const IMG_SRC: &str = "img-src https://legible-doc.localhost data:";
/// What the header adds, which a `<meta>` can't carry: no page may frame this one.
pub const CSP_HEADER_EXTRA: &str = "; frame-ancestors 'none'";

/// The page Mermaid draws diagrams in, in a hidden frame of the window's page.
pub const MERMAID_FRAME: &str = "/doc/mermaid-frame.html";
/// Its Content Security Policy, as its response header: sandboxed (an opaque origin, even if
/// it's opened by itself), Legible's scripts, the inline styles Mermaid lays diagrams out
/// with, and nothing else; only Legible's page may frame it.
pub const MERMAID_FRAME_CSP: &str = "default-src 'none'; \
    script-src https://legible.localhost; \
    style-src 'unsafe-inline'; \
    base-uri 'none'; \
    form-action 'none'; \
    frame-ancestors https://legible.localhost; \
    sandbox allow-scripts";

/// The theme: Windows' own, or one chosen over it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// The reading width.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Width {
    Narrow,
    #[default]
    Comfortable,
    Wide,
}

/// How the document is shown: classes on `<html>` that `web/doc/document.css` reads
/// (docs/ARCHITECTURE.md, "The document's look").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct View {
    pub theme: Theme,
    pub width: Width,
    /// The serif reading style.
    pub serif: bool,
}

impl View {
    /// Its classes, space-separated; none for Windows' theme, Comfortable and Sans.
    pub fn classes(&self) -> String {
        let theme = match self.theme {
            Theme::System => None,
            Theme::Light => Some("theme-light"),
            Theme::Dark => Some("theme-dark"),
        };
        let width = match self.width {
            Width::Narrow => Some("width-narrow"),
            Width::Comfortable => None,
            Width::Wide => Some("width-wide"),
        };
        let serif = self.serif.then_some("font-serif");
        [theme, width, serif]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The page: `document` (sanitised HTML) in Legible's window page, titled `title`, in
/// language `lang`, shown as `settings` say. It loads the document's look (`/doc/document.css`
/// and `/doc/document.js`), the window's chrome (`/chrome/chrome.js`) and the editor
/// (`/editor/editor.css` and `/editor/editor.js`), and gives each its strings, and the chrome
/// the settings, in JSON data blocks. The document is a file that may be edited where it is.
pub fn page(lang: &str, title: &str, document: &str, settings: &Settings) -> String {
    document_page(lang, title, document, settings, 0, &[], true)
}

/// The page as [`page`] makes it, its document being render `generation`, its CSP letting
/// pictures load from `picture_origins` (the consented pictures' origins, [`csp`]), and the
/// editor told whether the file may be edited where it is (`editable`: otherwise Split and Edit
/// offer "Save as…").
pub fn document_page(
    lang: &str,
    title: &str,
    document: &str,
    settings: &Settings,
    generation: u64,
    picture_origins: &[String],
    editable: bool,
) -> String {
    let csp = csp(picture_origins);
    filled(
        lang,
        title,
        document,
        settings,
        settings.view().classes(),
        &csp,
        generation,
        editable,
    )
}

/// The page's CSP, [`CSP`] with pictures also allowed from `picture_origins`: each an
/// `https://host[:port]` origin (`consent::img_sources`). Anything else is left out.
pub fn csp(picture_origins: &[String]) -> String {
    let safe = |o: &&String| {
        o.starts_with("https://")
            && o.len() > "https://".len()
            && o.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-:/".contains(&b))
    };
    let extra: Vec<&str> = picture_origins
        .iter()
        .filter(safe)
        .map(String::as_str)
        .collect();
    if extra.is_empty() {
        return CSP.to_owned();
    }
    CSP.replacen(IMG_SRC, &format!("{IMG_SRC} {}", extra.join(" ")), 1)
}

/// The page of a window with no document: a line saying so, and `no-document` on `<html>`,
/// so the chrome offers nothing that needs a file (nothing to edit either).
pub fn empty_page(lang: &str, title: &str, text: &str, settings: &Settings) -> String {
    let classes = settings.view().classes();
    let classes = if classes.is_empty() {
        "no-document".to_owned()
    } else {
        format!("{classes} no-document")
    };
    let document = format!("<p class=\"empty\">{}</p>", escape(text));
    filled(lang, title, &document, settings, classes, CSP, 0, false)
}

#[allow(clippy::too_many_arguments)] // The page's parts, each its own; callers name them.
fn filled(
    lang: &str,
    title: &str,
    document: &str,
    settings: &Settings,
    classes: String,
    csp: &str,
    generation: u64,
    editable: bool,
) -> String {
    let view = if classes.is_empty() {
        String::new()
    } else {
        format!(" class=\"{classes}\"")
    };
    // The strings are escaped for a JSON data block, which the CSP lets be: it isn't run.
    let values = [
        ("lang", escape(lang)),
        ("view", view),
        ("csp", escape(csp)),
        ("generation", generation.to_string()),
        ("title", escape(title)),
        ("strings", script_strings()),
        ("chrome-strings", chrome_strings()),
        ("settings", settings.to_json()),
        ("editor", format!("{{\"editable\":{editable}}}")),
        ("document", document.to_owned()),
    ];
    fill(asset_text("chrome/window.html"), &values)
}

/// The chrome's strings (`chrome-*` in `locales/en/window.ftl`) as a JSON object keyed without
/// the prefix, for `chrome.js`. Those with numbers in keep their places (`find-count`'s
/// `{current}` and `{total}`, say) for the script to fill with numbers formatted for the page's
/// language; `status-words` is in each plural form of its language ([`plurals_json`]).
/// `version` is Legible's.
pub fn chrome_strings() -> String {
    const KEYS: &[&str] = &[
        "toolbar",
        "open",
        "mode",
        "mode-view",
        "mode-split",
        "mode-edit",
        "mode-cycles",
        "coming-soon",
        "outline",
        "find",
        "zoom-out",
        "zoom-in",
        "zoom-reset",
        "reading",
        "width-narrow",
        "width-comfortable",
        "width-wide",
        "style-sans",
        "style-serif",
        "print",
        "copy",
        "more",
        "save-pdf",
        "open-editor",
        "show-folder",
        "settings",
        "about",
        "close",
        "outline-panel",
        "outline-resize",
        "outline-in-view",
        "find-field",
        "find-previous",
        "find-next",
        "find-none",
        "settings-tab-appearance",
        "settings-tab-reading",
        "settings-tab-general",
        "settings-theme",
        "theme-system",
        "theme-light",
        "theme-dark",
        "settings-style",
        "settings-width",
        "settings-zoom",
        "settings-remote",
        "remote-ask",
        "remote-never",
        "remote-always",
        "settings-outline",
        "outline-automatic",
        "outline-always",
        "outline-never",
        "settings-comfort",
        "settings-wheel-zoom",
        "settings-reopen-windows",
        "settings-remember-place",
        "settings-default-app",
        "default-heading",
        "default-is",
        "default-register",
        "default-unregister",
        "default-portable",
        "default-offer",
        "default-dismiss",
        "save-read-only",
        "about-tagline",
        "about-mit",
        "about-third-party",
        "about-privacy",
        "about-notice",
        "about-copy",
        "about-release-notes",
        "about-ok",
        "about-copied",
        "about-licence-title",
        "about-notices-title",
        "about-release-notes-title",
        "about-back",
        "updated",
        "file-gone",
        "file-keep",
        "recent",
        "recent-none",
        "pictures-ask",
        "pictures-once",
        "pictures-always",
        "pictures-label",
        "pictures-off-runtime",
        "pictures-off-failed",
        "pictures-save-first",
        "copied",
        "copied-text-only",
        "copy-diagram",
        "open-file",
        "unsaved",
        "mode-not-editable",
        "save",
        "save-as",
        "save-nothing",
        "save-not-editable",
        "undo",
        "redo",
        "group-file",
        "group-editing",
        "group-find",
        "group-look",
        "group-out",
        "status-copy-path",
        "status-path-copied",
        "status-label",
        "settings-status",
        "settings-status-bar",
        "settings-updates",
        "settings-check-updates",
        "settings-updates-hint",
        "update-check-now",
        "update-label",
        "update-whats-new",
        "editor",
        "editor-resize",
        "settings-editor-font",
        "editor-font-reading",
        "editor-font-monospace",
        "editor-find",
        "editor-replace-with",
        "editor-next",
        "editor-previous",
        "editor-select-all",
        "editor-match-case",
        "editor-regexp",
        "editor-by-word",
        "editor-replace",
        "editor-replace-all",
        "editor-close",
        "editor-current-match",
        "editor-on-line",
        "editor-replaced-one",
        "editor-replaced-many",
        "editor-go-to-line",
        "editor-go",
        "editor-control-character",
        "editor-selection-deleted",
        "conflict-keep",
        "conflict-take",
        "conflict-take-other",
        "conflict-compare",
        "conflict-stop-comparing",
        "conflict-cancel",
        "conflict-take-confirm",
        "conflict-take-raced",
        "conflict-comparing",
        "conflict-comparing-raced",
        "restore",
        "discard",
    ];
    let mut fields: Vec<String> = KEYS
        .iter()
        .map(|key| {
            format!(
                "{}:{}",
                json_string(key),
                json_string(&tr(&format!("chrome-{key}")))
            )
        })
        .collect();
    let count = tr!(
        "chrome-find-count",
        current = "{current}",
        total = "{total}"
    );
    fields.push(format!(
        "{}:{}",
        json_string("find-count"),
        json_string(&count)
    ));
    // A tooltip: a button's name with its shortcut, or with a note; their places kept.
    let tips = [
        (
            "tip-shortcut",
            tr!("chrome-tip-shortcut", label = "{label}", keys = "{keys}"),
        ),
        (
            "tip-note",
            tr!("chrome-tip-note", label = "{label}", note = "{note}"),
        ),
    ];
    // The keys a tooltip's shortcut names, as the menus name them.
    let keys = [
        ("key-ctrl", tr("menu-key-ctrl")),
        ("key-shift", tr("menu-key-shift")),
        ("key-enter", tr("menu-key-enter")),
        ("key-esc", tr("menu-key-esc")),
    ];
    for (key, text) in tips.into_iter().chain(keys) {
        fields.push(format!("{}:{}", json_string(key), json_string(&text)));
    }
    // About's licence line: its two links' places kept for the script.
    let licence = tr!(
        "chrome-about-licence-line",
        mit = "{mit}",
        notices = "{notices}"
    );
    fields.push(format!(
        "{}:{}",
        json_string("about-licence-line"),
        json_string(&licence)
    ));
    // About's icon, Legible's own, as a picture the page's CSP lets in (data:).
    fields.push(format!(
        "{}:{}",
        json_string("about-icon"),
        json_string(&svg_data_url(include_str!(
            "../../../assets/icon/legible.svg"
        )))
    ));
    // Printed on each page, and said after Save as PDF: their placeables kept for the script.
    let print_page = tr!("chrome-print-page", page = "{page}", pages = "{pages}");
    fields.push(format!(
        "{}:{}",
        json_string("print-page"),
        json_string(&print_page)
    ));
    let saved = tr!("chrome-saved", name = "{name}");
    fields.push(format!("{}:{}", json_string("saved"), json_string(&saved)));
    // The status bar's word count, in each plural form: its number kept for the script.
    fields.push(format!(
        "{}:{}",
        json_string("status-words"),
        plurals_json(&tr_plurals("chrome-status-words", "count"))
    ));
    // The zoom: its number kept for the script.
    let zoom = tr!("chrome-zoom-level", zoom = "{zoom}");
    fields.push(format!(
        "{}:{}",
        json_string("zoom-level"),
        json_string(&zoom)
    ));
    // The status bar path's tooltip and accessible name: the path's place kept for the script.
    let action = tr("chrome-status-copy-path");
    let tooltip = tr!(
        "chrome-status-path-tooltip",
        path = "{path}",
        action = action.as_str()
    );
    let label = tr!(
        "chrome-status-path-label",
        path = "{path}",
        action = action.as_str()
    );
    for (field, text) in [
        ("status-path-tooltip", tooltip),
        ("status-path-label", label),
    ] {
        fields.push(format!("{}:{}", json_string(field), json_string(&text)));
    }
    // The editor bar's text while comparing: its two parts' places kept for the script.
    let comparing = tr!(
        "chrome-conflict-with-comparing",
        text = "{text}",
        comparing = "{comparing}"
    );
    fields.push(format!(
        "{}:{}",
        json_string("conflict-with-comparing"),
        json_string(&comparing)
    ));
    // The app's name, as everywhere else (common.ftl), for About.
    fields.push(format!(
        "{}:{}",
        json_string("app-name"),
        json_string(&tr("app-name"))
    ));
    format!("{{{}}}", fields.join(","))
}

/// A message's plural forms as JSON, for the script to choose from by its count: the exact
/// number's form if there is one, else its category's by `Intl.PluralRules`.
/// `{"lang": tag, "forms": {category: text, ...}, "exact": {number: text, ...}}`.
fn plurals_json(plurals: &PluralForms) -> String {
    let forms = json_object(plurals.forms.iter().map(|(c, t)| (*c, t.as_str())));
    let exact = json_object(plurals.exact.iter().map(|(n, t)| (n.as_str(), t.as_str())));
    format!(
        "{{\"lang\":{},\"forms\":{forms},\"exact\":{exact}}}",
        json_string(&plurals.language)
    )
}

/// (name, text) pairs as a JSON object.
fn json_object<'a>(pairs: impl Iterator<Item = (&'a str, &'a str)>) -> String {
    let fields: Vec<String> = pairs
        .map(|(name, text)| format!("{}:{}", json_string(name), json_string(text)))
        .collect();
    format!("{{{}}}", fields.join(","))
}

/// `svg` as a `data:` URL: every byte but a few plain ones percent-encoded.
fn svg_data_url(svg: &str) -> String {
    let mut url = String::from("data:image/svg+xml,");
    for byte in svg.bytes() {
        if byte.is_ascii_alphanumeric() || b" -._~/:=;,()!*'".contains(&byte) {
            url.push(char::from(byte));
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

/// How a picture on the document's origin starts in the rendered HTML.
const DOC_SRC: &str = "src=\"https://legible-doc.localhost/";

/// `html` with each picture on the document's origin under `/~<generation>/`, so a picture
/// that changed on disk is fetched afresh after a live reload (the document protocol takes the
/// part off again: [`crate::confine::without_generation`]).
pub fn with_generation(html: &str, generation: u64) -> String {
    html.replace(DOC_SRC, &format!("{DOC_SRC}~{generation}/"))
}

/// The URL paths (`/img/a.png`) of the pictures on the document's origin in rendered `html`,
/// for live reload to watch.
pub fn picture_paths(html: &str) -> Vec<String> {
    html.match_indices(DOC_SRC)
        .filter_map(|(at, _)| {
            let rest = &html[at + DOC_SRC.len()..];
            let end = rest.find('"')?;
            Some(format!("/{}", rest[..end].replace("&amp;", "&")))
        })
        .collect()
}

/// `template` with each `{{name}}` in it replaced by its value, in one pass, so nothing a value
/// brings in (a file name in the title, the document) is ever taken for a placeholder.
fn fill(template: &str, values: &[(&str, String)]) -> String {
    let mut out =
        String::with_capacity(template.len() + values.iter().map(|(_, v)| v.len()).sum::<usize>());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let value = after.find("}}").and_then(|end| {
            values
                .iter()
                .find(|(name, _)| *name == &after[..end])
                .map(|(_, v)| (v, end))
        });
        match value {
            Some((value, end)) => {
                out.push_str(value);
                rest = &after[end + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The script run in every page before its own: links go to the window (`chrome/links.js`).
pub fn init_script() -> &'static str {
    asset_text("chrome/links.js")
}

fn asset_text(path: &str) -> &'static str {
    let asset = assets::get(path).unwrap_or_else(|| panic!("{path} is embedded"));
    std::str::from_utf8(asset.bytes).unwrap_or_else(|_| panic!("{path} is UTF-8"))
}

/// `text` escaped for HTML text and attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Setting;

    #[test]
    fn the_editor_is_told_whether_the_file_may_be_edited() {
        let editable = |page: &str| {
            let start = page.find(r#"id="legible-editor">"#).unwrap();
            page[start..].split_once("</script>").unwrap().0.to_owned()
        };
        let no = Settings::default();
        assert!(editable(&page("en", "t", "", &no)).ends_with(r#"{"editable":true}"#));
        let read_only = document_page("en", "t", "", &no, 3, &[], false);
        assert!(editable(&read_only).ends_with(r#"{"editable":false}"#));
        // A window with no document has nothing to edit.
        let empty = empty_page("en", "Legible", "None", &no);
        assert!(editable(&empty).ends_with(r#"{"editable":false}"#));
    }

    #[test]
    fn the_page_carries_the_policy_before_anything_else_loads() {
        let page = page(
            "en-GB",
            "a <b> – Legible",
            "<p>{{title}}</p>",
            &Settings::default(),
        );
        let csp = page.find("Content-Security-Policy").unwrap();
        assert!(csp < page.find("<link").unwrap());
        assert!(page.contains("<title>a &lt;b&gt; – Legible</title>"));
        assert!(page.contains("lang=\"en-GB\""));
        // A placeholder in the document is the document's text, not filled in.
        assert!(
            page.contains("<main id=\"document\" data-generation=\"0\"><p>{{title}}</p></main>")
        );
        assert!(page.contains("script-src https://legible.localhost;"));
    }

    #[test]
    fn the_policy_allows_only_legibles_own_scripts_and_the_documents_images() {
        let directives: Vec<&str> = CSP.split(';').map(str::trim).collect();
        assert!(directives.contains(&"default-src 'none'"));
        assert!(directives.contains(&"script-src https://legible.localhost"));
        assert!(directives.contains(&"style-src https://legible.localhost"));
        assert!(directives.contains(&"img-src https://legible-doc.localhost data:"));
        assert!(directives.contains(&"frame-src https://legible.localhost/doc/mermaid-frame.html"));
        assert!(!CSP.contains("unsafe"));
        assert!(!CSP.contains('*'));
    }

    #[test]
    fn mermaids_frame_is_sandboxed_and_loads_nothing_else() {
        let directives: Vec<&str> = MERMAID_FRAME_CSP.split(';').map(str::trim).collect();
        assert!(directives.contains(&"default-src 'none'"));
        assert!(directives.contains(&"script-src https://legible.localhost"));
        assert!(directives.contains(&"sandbox allow-scripts"));
        assert!(directives.contains(&"frame-ancestors https://legible.localhost"));
        // Its inline styles are the frame's alone: the page's own CSP allows none.
        assert!(!MERMAID_FRAME_CSP.contains("unsafe-eval"));
        assert!(!MERMAID_FRAME_CSP.contains("allow-same-origin"));
        assert!(!MERMAID_FRAME_CSP.contains('*'));
        assert!(CSP.contains(&format!("frame-src {APP_ORIGIN}{MERMAID_FRAME};")));
        assert!(assets::get(MERMAID_FRAME).is_some());
    }

    #[test]
    fn the_page_is_loaded_from_legibles_own_origin() {
        assert_eq!(START_URL, format!("{APP_ORIGIN}/"));
    }

    #[test]
    fn consented_pictures_widen_img_src_and_nothing_else() {
        assert!(CSP.contains(&format!("{IMG_SRC};")));
        assert_eq!(csp(&[]), CSP);
        let origins = [
            "https://a.example".to_owned(),
            "https://b.example:8443".to_owned(),
        ];
        let widened = csp(&origins);
        assert_eq!(
            widened,
            CSP.replace(
                IMG_SRC,
                "img-src https://legible-doc.localhost data: https://a.example https://b.example:8443"
            )
        );
        let directives = |c: &str| -> Vec<String> {
            c.split(';')
                .map(|d| d.trim().to_owned())
                .filter(|d| !d.starts_with("img-src"))
                .collect()
        };
        assert_eq!(directives(&widened), directives(CSP));
        // Nothing that isn't a plain https origin gets in.
        for bad in [
            "http://a.example",
            "https:",
            "https://a.example; script-src *",
            "https://a.example *",
            "*",
            "'unsafe-inline'",
        ] {
            assert_eq!(csp(&[bad.to_owned()]), CSP, "{bad}");
        }
        let page = document_page(
            "en",
            "t",
            "<p>x</p>",
            &Settings::default(),
            7,
            &origins,
            true,
        );
        assert!(page.contains(&escape(&widened)));
        assert!(page.contains(r#"<main id="document" data-generation="7">"#));
    }

    #[test]
    fn a_window_with_no_document_says_so_to_the_chrome() {
        let empty = empty_page("en", "Legible", "No <document>", &Settings::default());
        assert!(empty.contains(r#"<html lang="en" class="width-wide no-document">"#));
        assert!(empty.contains(r#"<p class="empty">No &lt;document&gt;</p>"#));
        let mut settings = Settings::default();
        settings.set(Setting::Theme(Theme::Dark));
        let empty = empty_page("en", "Legible", "None", &settings);
        assert!(empty.contains(r#"class="theme-dark width-wide no-document""#));
        assert!(!page("en", "t", "<p>x</p>", &settings).contains("no-document"));
    }

    #[test]
    fn the_page_loads_the_documents_look_and_its_strings() {
        let page = page("en", "t", "<p>x</p>", &Settings::default());
        assert!(page.contains(r#"<link rel="stylesheet" href="/doc/document.css">"#));
        assert!(page.contains(r#"<script src="/doc/document.js" defer></script>"#));
        let strings = format!(
            r#"<script type="application/json" id="legible-doc-strings">{}</script>"#,
            script_strings()
        );
        assert!(page.contains(&strings));
        // Windows' theme and Sans have no class; Wide, the default width, has its own.
        assert!(page.contains(r#"<html lang="en" class="width-wide">"#));
    }

    #[test]
    fn the_chrome_gets_its_strings_and_the_settings() {
        let page = page("en", "t", "", &Settings::default());
        let strings = format!(
            r#"<script type="application/json" id="legible-chrome-strings">{}</script>"#,
            chrome_strings()
        );
        assert!(page.contains(&strings));
        let settings = format!(
            r#"<script type="application/json" id="legible-settings">{}</script>"#,
            Settings::default().to_json()
        );
        assert!(page.contains(&settings));
        assert!(page.contains(r#"<script src="/chrome/chrome.js" defer></script>"#));
        // Every string the chrome asks for is in the Fluent files.
        assert!(
            !chrome_strings().contains(r#":"chrome-"#),
            "a chrome string is missing"
        );
        assert!(chrome_strings().contains(r#""find-count":"{current} of {total}""#));
        assert!(chrome_strings().contains(
            r#""status-words":{"lang":"en","forms":{"zero":"{count} words","one":"{count} word","two":"{count} words","few":"{count} words","many":"{count} words","other":"{count} words"},"exact":{}}"#
        ));
        assert!(chrome_strings().contains(r#""zoom-level":"{zoom}%""#));
        // The tooltip's two lines, as JSON writes the line break.
        assert!(
            chrome_strings().contains(r#""status-path-tooltip":"{path}\nClick to copy the path""#)
        );
        assert!(chrome_strings().contains(r#""conflict-with-comparing":"{text} {comparing}""#));
        assert!(chrome_strings().contains(&format!(
            "{}:{}",
            json_string("app-name"),
            json_string(&tr("app-name"))
        )));
    }

    #[test]
    fn placeholders_in_a_title_or_the_document_stay_as_they_are() {
        let page = page(
            "en",
            "{{strings}} {{csp}}.md",
            "<p>{{document}} {{title}}</p>",
            &Settings::default(),
        );
        assert!(page.contains("<title>{{strings}} {{csp}}.md</title>"));
        assert!(page.contains(
            "<main id=\"document\" data-generation=\"0\"><p>{{document}} {{title}}</p></main>"
        ));
        assert_eq!(page.matches(&script_strings()).count(), 1);
    }

    #[test]
    fn every_placeholder_in_the_template_is_filled() {
        let page = page("en", "t", "", &Settings::default());
        assert!(!page.contains("{{"), "a placeholder left in the page");
    }

    #[test]
    fn pictures_get_a_generation_and_are_found_for_watching() {
        let html = r#"<p><img src="https://legible-doc.localhost/img/a%20b.png" alt=""><img src="https://legible-doc.localhost/x&amp;y.png"><img data-remote-src="https://example.com/c.png"></p>"#;
        assert_eq!(picture_paths(html), ["/img/a%20b.png", "/x&y.png"]);
        let moved = with_generation(html, 7);
        assert!(moved.contains(r#"src="https://legible-doc.localhost/~7/img/a%20b.png""#));
        assert!(moved.contains(r#"data-remote-src="https://example.com/c.png""#));
    }

    #[test]
    fn the_view_is_classes_on_the_root() {
        let mut settings = Settings::default();
        settings.set(Setting::Theme(Theme::Dark));
        settings.set(Setting::Width(Width::Wide));
        settings.set(Setting::Serif(true));
        let page = page("en", "t", "", &settings);
        assert!(page.contains(r#"<html lang="en" class="theme-dark width-wide font-serif">"#));
        let light_narrow = View {
            theme: Theme::Light,
            width: Width::Narrow,
            serif: false,
        };
        assert_eq!(light_narrow.classes(), "theme-light width-narrow");
    }
}
