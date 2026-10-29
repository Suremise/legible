//! Markdown to safe HTML: the document as the view shows it.
//!
//! ```
//! use std::path::Path;
//! use legible_core::render::render;
//!
//! let doc = render("# Hello\n\n![chart](img/chart.png)", Path::new("/docs/report"));
//! assert_eq!(doc.outline[0].id, "user-content-hello");
//! assert!(doc.html.contains(r#"src="https://legible-doc.localhost/img/chart.png""#));
//! ```
//!
//! The steps: front matter is taken off the top ([`crate::frontmatter`]); comrak parses the
//! rest as GitHub-flavoured Markdown (tables, task lists, strikethrough, autolinks, footnotes,
//! alerts), with the document's raw HTML kept for the next step; of a footnote label defined
//! more than once, the first definition is kept, as GitHub keeps it; alerts become
//! `<aside class="alert note">` with a translated title, and ```` ```mermaid ```` blocks become
//! `<div class="mermaid" data-source="…">`; headings get ids as they're written
//! ([`crate::outline`]); then everything is sanitised ([`crate::sanitize`], which says what's
//! kept), and the properties table is put at the top.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::Path;

use comrak::nodes::{AlertType, AstNode, NodeHtmlBlock, NodeValue};
use comrak::options::Plugins;
use comrak::{Arena, Options};

use crate::frontmatter::{self, Property, escape};
use crate::i18n;
use crate::outline::{Heading, HeadingIds};
use crate::sanitize::sanitize;

/// Where the view serves the document's folder: a relative image `img/a.png` becomes
/// `<DOC_BASE>img/a.png`. WebView2 serves a custom scheme `legible-doc` at
/// `https://legible-doc.localhost/` (with wry's https option, so the page is a secure context).
pub const DOC_BASE: &str = "https://legible-doc.localhost/";

/// A rendered document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// The document as sanitised HTML: a fragment for the page's body, the properties table
    /// first if it has front matter.
    pub html: String,
    /// Its headings, in order, with the ids their elements carry.
    pub outline: Vec<Heading>,
    /// Its front matter's properties, in order; empty without front matter.
    pub properties: Vec<Property>,
    /// Whether an image is waiting for the user's say-so: an `<img data-remote-src="…">` with no
    /// `src`. Set `src` from it to show one.
    pub has_remote_images: bool,
    /// Whether it has a Mermaid diagram (`<div class="mermaid" data-source="…">`), so the view
    /// loads Mermaid only when it needs it.
    pub has_mermaid: bool,
}

/// How to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    /// Where the view serves the document's folder, ending in `/`. Default: [`DOC_BASE`].
    pub doc_base: String,
    /// Mark each block with the lines of the file it came from, as
    /// `data-sourcepos="<first line>:<column>-<last line>:<column>"` (lines counted from 1 in
    /// the whole file, front matter included), for the editor's preview to keep its scrolling
    /// in step with the text. Default: off.
    pub sourcepos: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            doc_base: DOC_BASE.to_owned(),
            sourcepos: false,
        }
    }
}

/// The strings `web/doc/document.js` shows (its Copy button, the placeholders, Mermaid's
/// toggle), translated, as a JSON object for the page to give it in
/// `<script type="application/json" id="legible-doc-strings">`. Safe inside that element: `<`,
/// `>` and `&` are escaped.
pub fn script_strings() -> String {
    let strings = [
        ("copy", i18n::tr("doc-copy")),
        ("copied", i18n::tr("doc-copied")),
        ("copy-failed", i18n::tr("doc-copy-failed")),
        ("remote-image", i18n::tr("doc-remote-image")),
        ("missing-image", i18n::tr("doc-missing-image")),
        ("show-source", i18n::tr("doc-show-source")),
        ("diagram-failed", i18n::tr("doc-diagram-failed")),
    ];
    let fields: Vec<String> = strings
        .iter()
        .map(|(key, text)| format!("{}:{}", json_string(key), json_string(text)))
        .collect();
    format!("{{{}}}", fields.join(","))
}

/// What a document's own raw HTML calls `data-sourcepos` becomes, so the sanitiser drops it:
/// only the renderer's marks say where a block came from.
const NOT_SOURCEPOS: &str = "data-not-sourcepos";

/// A raw HTML `literal` with each attribute named `data-sourcepos` (in any case) renamed
/// [`NOT_SOURCEPOS`], or None if it has none. Only attribute names change, read as the HTML
/// tokenizer reads tags (a comment, a quoted value or a `>` in one stays as it is), so the
/// document's text, its attribute values and other attributes are untouched.
fn without_sourcepos(literal: &str) -> Option<String> {
    const NAME: &str = "data-sourcepos";
    let names = sourcepos_attributes(literal, NAME);
    if names.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(literal.len() + 4 * names.len());
    let mut from = 0;
    for at in names {
        out.push_str(&literal[from..at]);
        out.push_str(NOT_SOURCEPOS);
        from = at + NAME.len();
    }
    out.push_str(&literal[from..]);
    Some(out)
}

/// Where each attribute named `name` (ASCII, any case) starts in the tags of `html`.
fn sourcepos_attributes(html: &str, name: &str) -> Vec<usize> {
    let b = html.as_bytes();
    let space = |c: u8| matches!(c, b'\t' | b'\n' | b'\x0c' | b'\r' | b' ');
    let after = |from: usize, end: &str| {
        html[from..]
            .find(end)
            .map_or(b.len(), |at| from + at + end.len())
    };
    let mut found = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        match b.get(i + 1) {
            _ if html[i..].starts_with("<!--") => {
                i = after(i + 4, "-->");
                continue;
            }
            // Doctypes, CDATA and processing instructions: no attributes.
            Some(b'!' | b'?') => {
                i = after(i + 2, ">");
                continue;
            }
            Some(b'/') if b.get(i + 2).is_some_and(u8::is_ascii_alphabetic) => i += 2,
            Some(c) if c.is_ascii_alphabetic() => i += 1,
            _ => {
                i += 1;
                continue;
            }
        }
        // The tag's name, then its attributes up to the `>`.
        while i < b.len() && !space(b[i]) && b[i] != b'/' && b[i] != b'>' {
            i += 1;
        }
        loop {
            while i < b.len() && (space(b[i]) || b[i] == b'/') {
                i += 1;
            }
            if i >= b.len() || b[i] == b'>' {
                i += 1;
                break;
            }
            // A name's first character is part of it, even an `=`.
            let start = i;
            i += 1;
            while i < b.len() && !space(b[i]) && !matches!(b[i], b'/' | b'>' | b'=') {
                i += 1;
            }
            if html[start..i].eq_ignore_ascii_case(name) {
                found.push(start);
            }
            while i < b.len() && space(b[i]) {
                i += 1;
            }
            if i < b.len() && b[i] == b'=' {
                i += 1;
                while i < b.len() && space(b[i]) {
                    i += 1;
                }
                match b.get(i) {
                    Some(&quote @ (b'"' | b'\'')) => {
                        i = after(i + 1, if quote == b'"' { "\"" } else { "'" });
                    }
                    _ => {
                        while i < b.len() && !space(b[i]) && b[i] != b'>' {
                            i += 1;
                        }
                    }
                }
            }
        }
    }
    found
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", u32::from(c)))
            }
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Renders `markdown`, a document in the folder `base_dir`, with the default options. `base_dir`
/// bounds what the document can reach: images and `file:` links outside it are dropped. It's
/// only compared as text, never read.
pub fn render(markdown: &str, base_dir: &Path) -> Rendered {
    render_with(markdown, base_dir, &RenderOptions::default())
}

/// As [`render`], with `options`.
pub fn render_with(markdown: &str, base_dir: &Path, options: &RenderOptions) -> Rendered {
    let (properties, body) = frontmatter::split(markdown);

    let mut comrak = Options::default();
    comrak.extension.strikethrough = true;
    comrak.extension.table = true;
    comrak.extension.autolink = true;
    comrak.extension.tasklist = true;
    comrak.extension.footnotes = true;
    // Resolved here instead: of a label defined twice, the first definition wins.
    comrak.parse.leave_footnote_definitions = true;
    comrak.extension.alerts = true;
    // The document's raw HTML goes on to the sanitiser, which decides what's kept.
    comrak.render.r#unsafe = true;
    comrak.render.sourcepos = options.sourcepos;
    // Lines before the body (the front matter), which comrak doesn't see.
    let line_offset = markdown[..markdown.len() - body.len()]
        .matches('\n')
        .count();

    let arena = Arena::new();
    let root = comrak::parse_document(&arena, body, &comrak);
    resolve_footnotes(root);
    let mut has_mermaid = false;
    let mut alerts = Vec::new();
    for node in root.descendants() {
        let mut ast = node.data.borrow_mut();
        if options.sourcepos {
            // The file's lines, front matter included.
            let pos = &mut ast.sourcepos;
            pos.start.line = pos.start.line.saturating_add(line_offset);
            pos.end.line = pos.end.line.saturating_add(line_offset);
        }
        let pos = ast.sourcepos;
        match &mut ast.value {
            NodeValue::HtmlBlock(NodeHtmlBlock { literal, .. })
            | NodeValue::HtmlInline(literal)
                if options.sourcepos =>
            {
                if let Some(renamed) = without_sourcepos(literal) {
                    *literal = renamed;
                }
            }
            NodeValue::CodeBlock(block)
                if block.info.split_whitespace().next() == Some("mermaid") =>
            {
                has_mermaid = true;
                // comrak marks no raw HTML block with its lines: the diagram carries its own.
                let lines = if options.sourcepos {
                    format!(
                        " data-sourcepos=\"{}:{}-{}:{}\"",
                        pos.start.line, pos.start.column, pos.end.line, pos.end.column
                    )
                } else {
                    String::new()
                };
                let literal = format!(
                    "<div class=\"mermaid\" data-source=\"{}\"{lines}></div>\n",
                    escape(&block.literal)
                );
                ast.value = NodeValue::HtmlBlock(NodeHtmlBlock {
                    block_type: 6,
                    literal,
                });
            }
            NodeValue::Alert(_) => alerts.push(node),
            _ => {}
        }
    }
    // An alert becomes its own HTML around its children: `<aside class="alert note">`, a title,
    // the children, `</aside>`.
    for alert in alerts {
        let NodeValue::Alert(info) = alert.data.borrow().value.clone() else {
            continue;
        };
        let (class, key) = match info.alert_type {
            AlertType::Note => ("note", "alert-note"),
            AlertType::Tip => ("tip", "alert-tip"),
            AlertType::Important => ("important", "alert-important"),
            AlertType::Warning => ("warning", "alert-warning"),
            AlertType::Caution => ("caution", "alert-caution"),
        };
        let title = info.title.unwrap_or_else(|| i18n::tr(key));
        let html = |literal: String| {
            arena.alloc(
                NodeValue::HtmlBlock(NodeHtmlBlock {
                    block_type: 6,
                    literal,
                })
                .into(),
            )
        };
        alert.insert_before(html(format!(
            "<aside class=\"alert {class}\">\n<p class=\"alert-title\">{}</p>\n",
            escape(&title)
        )));
        let children: Vec<_> = alert.children().collect();
        for child in children {
            child.detach();
            alert.insert_before(child);
        }
        alert.insert_before(html("</aside>\n".to_owned()));
        alert.detach();
    }

    let headings = HeadingIds::default();
    let mut plugins = Plugins::default();
    plugins.render.heading_adapter = Some(&headings);
    let mut html = String::new();
    // Writing to a String can't fail.
    let _ = comrak::html::format_document_with_plugins(root, &comrak, &mut html, &plugins);
    drop(plugins);

    let clean = sanitize(&html, base_dir, &options.doc_base);
    Rendered {
        html: frontmatter::to_html(&properties) + &clean.html,
        outline: headings.into_outline(),
        properties,
        has_remote_images: clean.has_remote_images,
        has_mermaid,
    }
}

/// A footnote label as comrak writes it: trimmed, each run of spaces one space.
fn footnote_label(name: &str) -> String {
    let mut label = String::with_capacity(name.len());
    let mut space = false;
    for c in name.trim_matches(|c: char| c.is_ascii_whitespace()).chars() {
        if c.is_whitespace() {
            if !space {
                label.push(' ');
            }
            space = true;
        } else {
            label.push(c);
            space = false;
        }
    }
    label
}

/// The footnotes, resolved as GitHub resolves them. comrak gives a label defined more than once
/// to its last definition and leaves the earlier ones where they were written, so its footnotes
/// list, opened at the first of them, took in every block after it. So comrak leaves the
/// definitions be (`leave_footnote_definitions`) and they're resolved here, as comrak resolves
/// them but for which definition wins: of a label's definitions the first is kept and the rest
/// dropped, before any reference is counted, so one in a dropped definition counts for nothing.
/// Labels match as comrak matches them (case folded, spaces run together); references are
/// numbered in the document's order; a definition referenced goes to the end, in that order,
/// with its count for the back-links, and one never referenced goes.
fn resolve_footnotes<'a>(root: &'a AstNode<'a>) {
    let fold = |name: &str| caseless::default_case_fold_str(&footnote_label(name));
    struct Kept<'a> {
        node: &'a AstNode<'a>,
        name: String,
        ix: u32,
        references: u32,
    }
    let mut kept: HashMap<String, Kept<'a>> = HashMap::new();
    let definitions: Vec<_> = root
        .descendants()
        .filter(|node| matches!(node.data.borrow().value, NodeValue::FootnoteDefinition(_)))
        .collect();
    for node in definitions {
        // One inside a definition already dropped went with it.
        if !node.ancestors().any(|above| std::ptr::eq(above, root)) {
            continue;
        }
        let NodeValue::FootnoteDefinition(definition) = &node.data.borrow().value else {
            continue;
        };
        match kept.entry(fold(&definition.name)) {
            Entry::Occupied(_) => node.detach(),
            Entry::Vacant(entry) => {
                entry.insert(Kept {
                    node,
                    name: footnote_label(&definition.name),
                    ix: 0,
                    references: 0,
                });
            }
        }
    }

    let mut next_ix = 0;
    for node in root.descendants() {
        let mut ast = node.data.borrow_mut();
        let NodeValue::FootnoteReference(reference) = &mut ast.value else {
            continue;
        };
        let Some(footnote) = kept.get_mut(&fold(&reference.name)) else {
            continue;
        };
        if footnote.ix == 0 {
            next_ix += 1;
            footnote.ix = next_ix;
        }
        footnote.references += 1;
        reference.ix = footnote.ix;
        reference.ref_num = footnote.references;
        reference.name = footnote.name.clone();
    }

    let mut footnotes: Vec<_> = kept.into_values().collect();
    footnotes.sort_unstable_by_key(|footnote| footnote.ix);
    for footnote in footnotes {
        if footnote.ix == 0 {
            footnote.node.detach();
            continue;
        }
        if let NodeValue::FootnoteDefinition(definition) =
            &mut footnote.node.data.borrow_mut().value
        {
            definition.name = footnote.name;
            definition.total_references = footnote.references;
        }
        root.append(footnote.node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_lines(markdown: &str) -> String {
        let options = RenderOptions {
            sourcepos: true,
            ..RenderOptions::default()
        };
        render_with(markdown, Path::new("/docs"), &options).html
    }

    /// Where `text` is in `html` (it panics if it isn't).
    fn at(html: &str, text: &str) -> usize {
        html.find(text)
            .unwrap_or_else(|| panic!("{text:?} isn't in {html}"))
    }

    #[test]
    fn a_footnote_label_defined_twice_keeps_every_block_in_place_and_the_first_definition() {
        let markdown = "# Top\n\nA line.[^note]\n\n[^note]: The first note.\n\n## Part 2\n\n\
                        Another line.[^note]\n\n[^note]: The second note.\n\nLast paragraph.\n";
        let html = render(markdown, Path::new("/docs")).html;
        // Every block where it was written, the footnotes after them all.
        let footnotes = at(&html, "<section class=\"footnotes\"");
        for block in ["<h2", "Another line.", "Last paragraph."] {
            assert!(
                at(&html, block) < footnotes,
                "{block} is in the footnotes: {html}"
            );
        }
        // One footnote, the first definition's, with both references back to it.
        assert_eq!(
            html.matches("<li id=\"user-content-fn-note\"").count(),
            1,
            "{html}"
        );
        assert!(at(&html, "The first note.") > footnotes, "{html}");
        assert!(!html.contains("The second note."), "{html}");
        assert_eq!(
            html.matches("class=\"footnote-backref\"").count(),
            2,
            "{html}"
        );
    }

    #[test]
    fn of_several_definitions_the_first_wins_wherever_it_is_and_however_its_cased() {
        let three = render(
            "A.[^n]\n\n[^n]: One.\n\nB.[^n]\n\n[^n]: Two.\n\nC.\n\n[^n]: Three.\n\nD.\n",
            Path::new("/docs"),
        )
        .html;
        assert!(
            three.contains("One.") && !three.contains("Two.") && !three.contains("Three."),
            "{three}"
        );
        assert!(
            at(&three, "D.") < at(&three, "<section class=\"footnotes\""),
            "{three}"
        );

        let cased = render(
            "A.[^Note]\n\n[^note]: Lower.\n\nB.\n\n[^NOTE]: Upper.\n\nC.\n",
            Path::new("/docs"),
        )
        .html;
        assert!(
            cased.contains("Lower.") && !cased.contains("Upper."),
            "{cased}"
        );
        assert!(
            at(&cased, "C.") < at(&cased, "<section class=\"footnotes\""),
            "{cased}"
        );

        // The first inside a quote: its content moves to the footnotes, the quote stays.
        let quoted = render(
            "A.[^n]\n\n> [^n]: In a quote.\n\nB.\n\n[^n]: Outside.\n\nC.\n",
            Path::new("/docs"),
        )
        .html;
        assert!(!quoted.contains("Outside."), "{quoted}");
        assert!(
            at(&quoted, "In a quote.") > at(&quoted, "<section class=\"footnotes\""),
            "{quoted}"
        );
        assert!(
            at(&quoted, "C.") < at(&quoted, "<section class=\"footnotes\""),
            "{quoted}"
        );
    }

    #[test]
    fn labels_match_as_comrak_folds_them_beyond_lower_case() {
        // ß folds to ss: one label, so the first definition wins.
        let html = render(
            "A.[^STRASSE]\n\n[^Straße]: First definition.\n\nB.\n\n[^STRASSE]: Second definition.\n",
            Path::new("/docs"),
        )
        .html;
        assert!(html.contains("First definition."), "{html}");
        assert!(!html.contains("Second definition."), "{html}");
    }

    #[test]
    fn a_reference_in_a_dropped_definition_counts_for_nothing_one_in_the_kept_one_counts() {
        let dropped = render(
            "A.[^a]\n\n[^a]: First definition.\n\n[^a]: Second definition refers to B.[^b]\n\n\
             [^b]: The B footnote.\n\nLast paragraph.\n",
            Path::new("/docs"),
        )
        .html;
        assert!(!dropped.contains("The B footnote."), "{dropped}");
        assert!(
            !dropped.contains("fnref-b"),
            "a back-link to a reference that's gone: {dropped}"
        );
        assert_eq!(dropped.matches("<li id=").count(), 1, "{dropped}");

        let kept = render(
            "A.[^a]\n\n[^a]: First definition refers to B.[^b]\n\n[^a]: Second definition.\n\n\
             [^b]: The B footnote.\n\nLast paragraph.\n",
            Path::new("/docs"),
        )
        .html;
        assert!(kept.contains("The B footnote."), "{kept}");
        assert!(kept.contains("href=\"#user-content-fnref-b\""), "{kept}");
        assert!(kept.contains("id=\"user-content-fnref-b\""), "{kept}");
        // Numbered as they're first referenced: a, then b.
        assert!(
            at(&kept, "id=\"user-content-fn-a\"") < at(&kept, "id=\"user-content-fn-b\""),
            "{kept}"
        );
        assert!(!kept.contains("Second definition."), "{kept}");
    }

    #[test]
    fn a_label_defined_twice_and_never_referenced_shows_no_footnotes() {
        let html = render(
            "A.\n\n[^n]: One.\n\nB.\n\n[^n]: Two.\n\nC.\n",
            Path::new("/docs"),
        )
        .html;
        assert!(!html.contains("footnotes"), "{html}");
        assert!(!html.contains("One.") && !html.contains("Two."), "{html}");
        assert!(html.contains("<p>C.</p>"), "{html}");
    }

    #[test]
    fn blocks_carry_their_lines_only_when_asked() {
        let markdown =
            "# Title\n\nText\non two lines.\n\n```mermaid\nflowchart LR\n  A --> B\n```\n";
        assert!(
            !render(markdown, Path::new("/docs"))
                .html
                .contains("data-sourcepos")
        );
        let html = with_lines(markdown);
        assert!(html.contains(r#"<h1 data-sourcepos="1:1-1:7""#), "{html}");
        assert!(html.contains(r#"<p data-sourcepos="3:1-4:13">"#), "{html}");
        assert!(
            html.contains(r#"data-sourcepos="6:1-9:3"></div>"#),
            "{html}"
        );
    }

    #[test]
    fn lines_are_the_files_with_front_matter_counted() {
        let html = with_lines("---\ntitle: A\nowner: B\n---\n# Title\n\nText\n");
        assert!(html.contains(r#"<h1 data-sourcepos="5:1-5:7""#), "{html}");
        assert!(html.contains(r#"<p data-sourcepos="7:1-7:4">"#), "{html}");
        let diagram = with_lines("---\ntitle: A\n---\n```mermaid\nflowchart LR\n```\n");
        assert!(
            diagram.contains(r#"data-sourcepos="4:1-6:3"></div>"#),
            "{diagram}"
        );
    }

    #[test]
    fn a_documents_own_positions_are_dropped_whatever_they_say() {
        let huge = u64::MAX;
        let markdown = format!(
            "---\ntitle: A\n---\n<p data-sourcepos=\"{huge}:1-{huge}:1\">raw</p>\n\n\
             Text with <span DATA-SOURCEPOS=\"1:1-1:1\">inline</span> HTML.\n"
        );
        let html = with_lines(&markdown);
        assert!(!html.contains(&huge.to_string()), "{html}");
        assert!(!html.contains("1:1-1:1"), "{html}");
        assert!(
            html.contains(">raw</p>") && html.contains(">inline</span>"),
            "{html}"
        );
        // Only the renderer's mark is left: the paragraph's, on the file's line 6.
        assert_eq!(html.matches("data-sourcepos").count(), 1, "{html}");
        assert!(html.contains(r#"<p data-sourcepos="6:1-6:"#), "{html}");
        // HTML with no such attribute is left as it was.
        assert_eq!(without_sourcepos("<b>plain</b>"), None);
    }

    #[test]
    fn only_an_attribute_named_data_sourcepos_is_renamed() {
        let renamed = |html: &str| without_sourcepos(html).unwrap_or_else(|| html.to_owned());
        // Its name, in any case, quoted, unquoted, bare, or after a `>` inside a quoted value.
        assert_eq!(
            renamed(r#"<p data-sourcepos="1:1-1:1">a</p>"#),
            r#"<p data-not-sourcepos="1:1-1:1">a</p>"#
        );
        assert_eq!(
            renamed("<span Data-SourcePos=9:9-9:9 data-sourcepos>a</span>"),
            "<span data-not-sourcepos=9:9-9:9 data-not-sourcepos>a</span>"
        );
        assert_eq!(
            renamed(r#"<a title='a > b' data-sourcepos = "1:1-1:1">x</a>"#),
            r#"<a title='a > b' data-not-sourcepos = "1:1-1:1">x</a>"#
        );
        // Everything else stays: text, values, other names, comments, and text that only looks
        // like a tag.
        for html in [
            "<div>Read the data-sourcepos documentation.</div>",
            r#"<a href="data-sourcepos.md" title="data-sourcepos">Link</a>"#,
            r#"<a href=data-sourcepos.md title='DATA-SOURCEPOS'>Link</a>"#,
            r#"<p data-sourcepos-x="1" x-data-sourcepos="2" data-sourceposition="3">a</p>"#,
            "<!-- <p data-sourcepos=\"1:1-1:1\"> -->",
            "<!DOCTYPE data-sourcepos>",
            "a < data-sourcepos b",
        ] {
            assert_eq!(without_sourcepos(html), None, "{html}");
        }
    }

    #[test]
    fn a_documents_text_and_links_survive_its_lines_being_marked() {
        let html = with_lines(
            "---\ntitle: A\n---\n<div>Read the data-sourcepos documentation.</div>\n\
             <a href=\"data-sourcepos.md\" title=\"Data-Sourcepos\">Link</a>\n",
        );
        assert!(
            html.contains("Read the data-sourcepos documentation."),
            "{html}"
        );
        assert!(html.contains(r#"href="data-sourcepos.md""#), "{html}");
        assert!(html.contains(r#"title="Data-Sourcepos""#), "{html}");
        assert!(!html.contains("not-sourcepos"), "{html}");
    }

    #[test]
    fn the_script_strings_are_json_safe_in_a_script_element() {
        let json = script_strings();
        assert!(json.starts_with("{\"copy\":\"Copy\","), "{json}");
        assert!(json.contains("\"remote-image\":\"Picture from the internet, not shown\""));
        let u = |hex: &str| format!("{}u{hex}", '\\');
        let expected = format!(
            "\"a{bs}\"b{bs}{bs}c{}/script{}{}{}\"",
            u("003c"),
            u("003e"),
            u("0026"),
            u("000a"),
            bs = '\\'
        );
        assert_eq!(json_string("a\"b\\c</script>&\n"), expected);
    }
}
