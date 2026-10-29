//! The clipboard's HTML: a document, or a selection of it, made fit to paste into Outlook, Word
//! and OneNote, with a plain-text version beside it.
//!
//! Those apps ignore `<style>` blocks, classes and CSS variables in pasted HTML and keep only
//! inline styles and simple HTML, so [`build`] rewrites the view's HTML with every style inline,
//! in the light theme always (the colours of `web/doc/document.css`):
//!
//! ```
//! use legible_core::clipboard_html::{Limits, build, cf_html};
//!
//! let copied = build("<h1>Plan</h1><p>Ship <code>v1</code>.</p>", &|_| None, &Limits::default());
//! assert!(copied.html.contains("<h1 style="));
//! assert_eq!(copied.text, "Plan\n\nShip v1.\n");
//! let bytes = cf_html(&copied.html); // for the clipboard's "HTML Format"
//! assert!(bytes.starts_with(b"Version:0.9\r\n"));
//! ```
//!
//! **The input** is the rendered HTML ([`crate::render`]) or what the view holds after
//! `web/doc/document.js` has prepared it (table frames, Copy buttons, highlighted code, diagram
//! figures): the whole document, or a selection's fragment. What becomes what:
//! - headings, paragraphs, lists, quotes, links and the rest get inline styles;
//! - tables get borders, a header fill and padding both as attributes and as styles (Outlook
//!   needs both), and zebra rows; a numeric column (`class="num"`) stays right-aligned;
//! - an alert becomes a one-cell table with a tinted fill, a coloured bar and its icon as a
//!   character; a code block becomes a shaded one-cell table with `<pre>` (wrapping), and
//!   highlight.js's colours become inline colours;
//! - a task list's tick box becomes ☑ or ☐;
//! - **pictures** are embedded as `data:` URIs (PNG, JPEG, GIF or WebP), at most [`MAX_WIDTH`]
//!   wide, read by the caller's `images` function from their `src`; one that can't be read is
//!   its alt text, and one over [`Limits`] its alt text with a note. A picture from the internet
//!   that isn't shown is its alt text (or nothing without one);
//! - a **Mermaid diagram** is the `<img>` the view put in it (a PNG, as a `data:` URI), or else
//!   its source as code;
//! - every `class`, `id` and `data-` attribute goes; `<script>`, `<style>`, SVG, buttons and form
//!   controls go with their content, and so do the view's own placeholders and Copy buttons.
//!
//! **The plain text** ([`Copied::text`]) reads on its own: blank lines between blocks, `•` or
//! `1.` before list items (indented when nested), table cells separated by tabs and rows by line
//! ends, code as it is, and a link's address after its text when they differ.

use std::cell::RefCell;
use std::fmt::Write as _;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

use crate::tr;

/// The widest a picture is pasted, in CSS pixels: about the document's reading width.
pub const MAX_WIDTH: u32 = 640;

/// How much picture data a copy may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most bytes one picture may have. Default 5 MB.
    pub per_image: usize,
    /// The most bytes all of them together may have. Default 20 MB.
    pub total: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_image: 5 * 1024 * 1024,
            total: 20 * 1024 * 1024,
        }
    }
}

/// What goes on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    /// The HTML fragment, every style inline. Frame it with [`cf_html`] for the clipboard.
    pub html: String,
    /// The plain-text version, with `\n` line ends.
    pub text: String,
}

/// The light theme's colours, as `web/doc/document.css` has them (a test keeps them equal).
pub(crate) mod colour {
    pub const TEXT: &str = "#1b1b1b";
    pub const TEXT_MUTED: &str = "#5d5d5d";
    pub const ACCENT: &str = "#005fb8";
    pub const RULE: &str = "#e0e0e0";
    pub const SURFACE: &str = "#f4f4f4";
    pub const STRIPE: &str = "#f9f9f9";
    pub const QUOTE_BAR: &str = "#c8c8c8";
    pub const CODE_BG: &str = "#f4f4f4";
    pub const CODE_TEXT: &str = "#1b1b1b";
    pub const HL_KEYWORD: &str = "#7a3e9d";
    pub const HL_STRING: &str = "#2e6b30";
    pub const HL_NUMBER: &str = "#a04400";
    pub const HL_COMMENT: &str = "#5f6368";
    pub const HL_TITLE: &str = "#005fb8";
    pub const HL_TYPE: &str = "#0b6870";
    pub const HL_ATTR: &str = "#8a5100";
    pub const HL_META: &str = "#5d5d5d";
    pub const HL_DELETION: &str = "#b4231a";
    pub const HL_ADDITION: &str = "#0f7b0f";
    pub const NOTE: &str = "#005fb8";
    pub const NOTE_BG: &str = "#ebf3fb";
    pub const TIP: &str = "#0f7b0f";
    pub const TIP_BG: &str = "#e9f6e8";
    pub const IMPORTANT: &str = "#7a3e9d";
    pub const IMPORTANT_BG: &str = "#f4ecfa";
    pub const WARNING: &str = "#8a5100";
    pub const WARNING_BG: &str = "#fff6d9";
    pub const CAUTION: &str = "#b4231a";
    pub const CAUTION_BG: &str = "#fdecee";
}

use colour::*;

const FONT: &str = "'Segoe UI', Aptos, Calibri, Arial, sans-serif";
const MONO: &str = "'Cascadia Mono', Consolas, 'Courier New', monospace";

/// Builds the clipboard's HTML and text from `html` (the view's HTML, a whole document or a
/// fragment). `images` reads a picture's bytes from its `src` as the view has it (a document
/// picture at [`crate::render::DOC_BASE`]); `None` when it can't. `data:` pictures are read
/// from their URI.
pub fn build(html: &str, images: &dyn Fn(&str) -> Option<Vec<u8>>, limits: &Limits) -> Copied {
    let tree = parse(html);
    let mut html_out = Html {
        out: String::new(),
        images,
        limits: *limits,
        used: 0,
    };
    html_out.out.push_str(&format!(
        "<div style=\"font-family:{FONT};font-size:11pt;line-height:1.5;color:{TEXT}\">"
    ));
    html_out.children(&tree, &Context::default());
    html_out.out.push_str("</div>");

    let mut text = Text::default();
    text.children(&tree, &TextContext::default());
    Copied {
        html: html_out.out,
        text: text.finish(),
    }
}

/// `fragment` framed as the clipboard's "HTML Format" (CF_HTML): the `Version:0.9` header with
/// the byte offsets of the HTML and of the fragment in it, then the HTML, all UTF-8.
pub fn cf_html(fragment: &str) -> Vec<u8> {
    const HEADER: &str = "Version:0.9\r\nStartHTML:{sh}\r\nEndHTML:{eh}\r\n\
        StartFragment:{sf}\r\nEndFragment:{ef}\r\n";
    const BEFORE: &str =
        "<html><head><meta charset=\"utf-8\"></head><body>\r\n<!--StartFragment-->";
    const AFTER: &str = "<!--EndFragment-->\r\n</body></html>";
    // Every offset is written as 10 digits, so the header's length is known before them.
    let header_len = HEADER.len() - 4 * "{sh}".len() + 4 * 10;
    let start_html = header_len;
    let start_fragment = start_html + BEFORE.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + AFTER.len();
    let header = HEADER
        .replace("{sh}", &format!("{start_html:010}"))
        .replace("{eh}", &format!("{end_html:010}"))
        .replace("{sf}", &format!("{start_fragment:010}"))
        .replace("{ef}", &format!("{end_fragment:010}"));
    debug_assert_eq!(header.len(), header_len);
    let mut bytes = Vec::with_capacity(end_html);
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(BEFORE.as_bytes());
    bytes.extend_from_slice(fragment.as_bytes());
    bytes.extend_from_slice(AFTER.as_bytes());
    bytes
}

// The tree

#[derive(Debug)]
enum Node {
    Element(Element),
    Text(String),
}

#[derive(Debug)]
struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Element {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn has_class(&self, class: &str) -> bool {
        self.attr("class")
            .is_some_and(|c| c.split_ascii_whitespace().any(|c| c == class))
    }

    fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// Every element inside, depth first.
    fn descendants(&self) -> Vec<&Element> {
        let mut found = Vec::new();
        for child in self.elements() {
            found.push(child);
            found.extend(child.descendants());
        }
        found
    }

    fn text(&self) -> String {
        let mut out = String::new();
        for child in &self.children {
            match child {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) => out.push_str(&e.text()),
            }
        }
        out
    }
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Removed with everything inside them.
const DROPPED: &[&str] = &[
    "script", "style", "template", "noscript", "svg", "math", "button", "textarea", "select",
    "iframe", "object", "embed", "canvas", "audio", "video", "head", "title",
];

/// The deepest the tree gets. Deeper elements give their content to the one at this depth, so
/// walking the tree (recursively) and dropping it stay well within the stack, however deeply
/// the HTML nests.
const MAX_DEPTH: usize = 64;

#[derive(Default)]
struct TreeSink {
    stack: RefCell<Vec<Element>>,
    /// The names of the elements open past [`MAX_DEPTH`], which aren't in the tree.
    flattened: RefCell<Vec<String>>,
    /// How many of those are dropped with their content (a script, an SVG).
    flattened_dropped: RefCell<usize>,
}

impl TokenSink for TreeSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut stack = self.stack.borrow_mut();
        let mut flattened = self.flattened.borrow_mut();
        let mut flattened_dropped = self.flattened_dropped.borrow_mut();
        // Inside something dropped (a script, an SVG) that's past the depth: its content goes.
        let dropping = *flattened_dropped > 0;
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.to_string();
                match tag.kind {
                    TagKind::StartTag => {
                        let element = Element {
                            attrs: tag
                                .attrs
                                .iter()
                                .map(|a| (a.name.local.to_string(), a.value.to_string()))
                                .collect(),
                            name: name.clone(),
                            children: Vec::new(),
                        };
                        if VOID.contains(&name.as_str()) || tag.self_closing {
                            if !dropping {
                                push_child(&mut stack, Node::Element(element));
                            }
                        } else if stack.len() > MAX_DEPTH {
                            if DROPPED.contains(&name.as_str()) {
                                *flattened_dropped += 1;
                            }
                            flattened.push(name.clone());
                        } else {
                            stack.push(element);
                        }
                        return match name.as_str() {
                            "script" => TokenSinkResult::RawData(RawKind::ScriptData),
                            "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
                                TokenSinkResult::RawData(RawKind::Rawtext)
                            }
                            "textarea" | "title" => TokenSinkResult::RawData(RawKind::Rcdata),
                            _ => TokenSinkResult::Continue,
                        };
                    }
                    TagKind::EndTag => {
                        if flattened.last() == Some(&name) {
                            flattened.pop();
                            if DROPPED.contains(&name.as_str()) {
                                *flattened_dropped -= 1;
                            }
                            return TokenSinkResult::Continue;
                        }
                        // Closes the nearest open element of that name, and any left open in
                        // it; an end tag with none open is ignored.
                        if let Some(at) = stack.iter().skip(1).rposition(|e| e.name == name) {
                            while stack.len() > at + 1 {
                                let done = stack.pop().unwrap();
                                push_child(&mut stack, Node::Element(done));
                            }
                        }
                    }
                }
            }
            Token::CharacterTokens(_) | Token::NullCharacterToken if dropping => {}
            Token::CharacterTokens(text) => push_text(&mut stack, &text),
            Token::NullCharacterToken => push_text(&mut stack, "\u{fffd}"),
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn push_child(stack: &mut [Element], node: Node) {
    stack.last_mut().unwrap().children.push(node);
}

fn push_text(stack: &mut [Element], text: &str) {
    let children = &mut stack.last_mut().unwrap().children;
    if let Some(Node::Text(last)) = children.last_mut() {
        last.push_str(text);
    } else {
        children.push(Node::Text(text.to_owned()));
    }
}

/// `html` as a tree, under a root element.
fn parse(html: &str) -> Element {
    let sink = TreeSink::default();
    sink.stack.borrow_mut().push(Element {
        name: String::new(),
        attrs: Vec::new(),
        children: Vec::new(),
    });
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from_slice(html));
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    let mut stack = tokenizer.sink.stack.into_inner();
    while stack.len() > 1 {
        let done = stack.pop().unwrap();
        push_child(&mut stack, Node::Element(done));
    }
    stack.pop().unwrap()
}

/// Whether the view adds `element` and it means nothing pasted: a Copy button, a placeholder
/// for a picture, a footnote's way back, a diagram's source toggle or error, or what can't be
/// pasted at all.
fn is_dropped(element: &Element) -> bool {
    DROPPED.contains(&element.name.as_str())
        || (element.name == "input" && element.attr("type") != Some("checkbox"))
        || element.has_class("remote-image")
        || element.has_class("missing-image")
        || element.has_class("copy")
        || element.has_class("footnote-backref")
}

// The HTML

#[derive(Default, Clone)]
struct Context {
    in_pre: bool,
    /// Inside a table's body: which row this is, for the zebra.
    row: usize,
    in_tbody: bool,
}

struct Html<'a> {
    out: String,
    images: &'a dyn Fn(&str) -> Option<Vec<u8>>,
    limits: Limits,
    /// Picture bytes embedded so far.
    used: usize,
}

impl Html<'_> {
    fn children(&mut self, element: &Element, cx: &Context) {
        let mut row = 0;
        for child in &element.children {
            match child {
                Node::Text(text) => self.out.push_str(&escape_text(text)),
                Node::Element(e) => {
                    let mut cx = cx.clone();
                    if e.name == "tr" && cx.in_tbody {
                        row += 1;
                        cx.row = row;
                    }
                    self.element(e, &cx);
                }
            }
        }
    }

    fn open(&mut self, tag: &str, attrs: &str, style: &str) {
        let _ = write!(self.out, "<{tag}{attrs}");
        if !style.is_empty() {
            let _ = write!(self.out, " style=\"{style}\"");
        }
        self.out.push('>');
    }

    fn wrap(&mut self, tag: &str, attrs: &str, style: &str, element: &Element, cx: &Context) {
        self.open(tag, attrs, style);
        self.children(element, cx);
        let _ = write!(self.out, "</{tag}>");
    }

    fn element(&mut self, e: &Element, cx: &Context) {
        if is_dropped(e) {
            return;
        }
        let name = e.name.as_str();
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let (size, rule, colour) = match name {
                    "h1" => ("20pt", true, TEXT),
                    "h2" => ("16pt", true, TEXT),
                    "h3" => ("13.5pt", false, TEXT),
                    "h4" => ("12pt", false, TEXT),
                    "h5" => ("11.5pt", false, TEXT),
                    _ => ("11pt", false, TEXT_MUTED),
                };
                let mut style = format!(
                    "font-family:{FONT};font-size:{size};font-weight:600;line-height:1.25;\
                     color:{colour};margin:18px 0 6px"
                );
                if rule {
                    let _ = write!(style, ";border-bottom:1px solid {RULE};padding-bottom:4px");
                }
                self.wrap(name, "", &style, e, cx);
            }
            "p" if e.has_class("alert-title") => self.wrap("p", "", "margin:0 0 4px", e, cx),
            "p" => self.wrap("p", "", "margin:0 0 10px", e, cx),
            "ul" | "ol" => {
                let mut attrs = String::new();
                for key in ["start", "type", "reversed"] {
                    if let Some(v) = e.attr(key) {
                        let _ = write!(attrs, " {key}=\"{}\"", escape_attr(v));
                    }
                }
                let task = e.elements().any(is_task_item);
                let style = if task {
                    "margin:0 0 10px;padding-left:4px;list-style:none"
                } else {
                    "margin:0 0 10px;padding-left:28px"
                };
                self.wrap(name, &attrs, style, e, cx);
            }
            "li" => {
                let attrs = e
                    .attr("value")
                    .map(|v| format!(" value=\"{}\"", escape_attr(v)))
                    .unwrap_or_default();
                self.wrap("li", &attrs, "margin:2px 0", e, cx);
            }
            "input" => {
                let ticked = e.attr("checked").is_some();
                self.out.push_str(if ticked { "☑ " } else { "☐ " });
            }
            "blockquote" => self.wrap(
                "blockquote",
                "",
                &format!(
                    "margin:0 0 10px;padding:0 0 0 12px;border-left:4px solid {QUOTE_BAR};\
                     color:{TEXT_MUTED}"
                ),
                e,
                cx,
            ),
            "aside" if e.has_class("alert") => self.alert(e, cx),
            "pre" => self.code_block(e, cx),
            "code" if cx.in_pre => self.children(e, cx),
            "code" | "kbd" | "samp" => self.wrap(
                "code",
                "",
                &format!(
                    "font-family:{MONO};font-size:0.9em;background:{CODE_BG};color:{CODE_TEXT};\
                     padding:1px 4px"
                ),
                e,
                cx,
            ),
            "span" if cx.in_pre => match highlight_style(e) {
                Some(style) => self.wrap("span", "", &style, e, cx),
                None => self.children(e, cx),
            },
            "table" => self.table(e, cx),
            "thead" | "tfoot" => self.wrap(
                name,
                "",
                "",
                e,
                &Context {
                    in_tbody: false,
                    ..cx.clone()
                },
            ),
            "tbody" => self.wrap(
                name,
                "",
                "",
                e,
                &Context {
                    in_tbody: true,
                    ..cx.clone()
                },
            ),
            "tr" => self.wrap("tr", "", "", e, cx),
            "th" | "td" => self.cell(e, cx),
            "caption" => self.wrap(
                "caption",
                "",
                &format!("text-align:left;color:{TEXT_MUTED};padding:0 0 4px"),
                e,
                cx,
            ),
            "colgroup" | "col" => {}
            "summary" => self.wrap("p", "", "margin:0 0 6px;font-weight:600", e, cx),
            "div" if e.has_class("mermaid") => self.diagram(e, cx),
            "img" => self.image(e),
            "a" => {
                let href = e.attr("href").filter(|h| is_web_link(h));
                match href {
                    Some(href) => self.wrap(
                        "a",
                        &format!(" href=\"{}\"", escape_attr(href)),
                        &format!("color:{ACCENT};text-decoration:underline"),
                        e,
                        cx,
                    ),
                    None => self.children(e, cx),
                }
            }
            "hr" => self.out.push_str(&format!(
                "<hr style=\"border:0;border-top:1px solid {RULE};margin:16px 0\">"
            )),
            "br" => self.out.push_str("<br>"),
            "section" if e.has_class("footnotes") => self.wrap(
                "div",
                "",
                &format!(
                    "border-top:1px solid {RULE};margin-top:20px;padding-top:6px;font-size:9.5pt;\
                     color:{TEXT_MUTED}"
                ),
                e,
                cx,
            ),
            "figcaption" => self.wrap(
                "p",
                "",
                &format!("margin:0 0 10px;color:{TEXT_MUTED}"),
                e,
                cx,
            ),
            "dl" => self.wrap("dl", "", "margin:0 0 10px", e, cx),
            "dt" => self.wrap("dt", "", "font-weight:600", e, cx),
            "dd" => self.wrap("dd", "", "margin:0 0 6px 24px", e, cx),
            "mark" => self.wrap("span", "", &format!("background:{WARNING_BG}"), e, cx),
            "strong" | "b" => self.wrap("strong", "", "", e, cx),
            "em" | "i" | "cite" | "dfn" | "var" => self.wrap("em", "", "", e, cx),
            "del" | "s" => self.wrap("del", "", "", e, cx),
            "u" | "ins" | "sub" | "sup" | "small" | "q" => self.wrap(name, "", "", e, cx),
            // Everything else (div, section, figure, span, abbr…) gives its content only.
            _ => self.children(e, cx),
        }
    }

    fn alert(&mut self, e: &Element, cx: &Context) {
        let (kind, bg, glyph) = if e.has_class("tip") {
            (TIP, TIP_BG, "💡")
        } else if e.has_class("important") {
            (IMPORTANT, IMPORTANT_BG, "❗")
        } else if e.has_class("warning") {
            (WARNING, WARNING_BG, "⚠")
        } else if e.has_class("caution") {
            (CAUTION, CAUTION_BG, "⛔")
        } else {
            (NOTE, NOTE_BG, "ℹ")
        };
        let _ = write!(
            self.out,
            "<table role=\"presentation\" width=\"100%\" cellpadding=\"10\" cellspacing=\"0\" \
             border=\"0\" style=\"border-collapse:collapse;width:100%;margin:0 0 10px\"><tr>\
             <td bgcolor=\"{bg}\" style=\"background:{bg};border-left:4px solid {kind};\
             padding:10px 14px\">"
        );
        for child in &e.children {
            match child {
                Node::Element(title) if title.name == "p" && title.has_class("alert-title") => {
                    let _ = write!(
                        self.out,
                        "<p style=\"margin:0 0 4px;font-weight:600;color:{kind}\">{glyph} "
                    );
                    self.children(title, cx);
                    self.out.push_str("</p>");
                }
                Node::Element(child) => self.element(child, cx),
                Node::Text(text) => self.out.push_str(&escape_text(text)),
            }
        }
        self.out.push_str("</td></tr></table>");
    }

    fn code_block(&mut self, e: &Element, cx: &Context) {
        self.code_open();
        self.children(
            e,
            &Context {
                in_pre: true,
                ..cx.clone()
            },
        );
        self.code_close();
    }

    fn code_text(&mut self, text: &str) {
        self.code_open();
        self.out.push_str(&escape_text(text));
        self.code_close();
    }

    fn code_open(&mut self) {
        let _ = write!(
            self.out,
            "<table role=\"presentation\" width=\"100%\" cellpadding=\"10\" cellspacing=\"0\" \
             border=\"0\" style=\"border-collapse:collapse;width:100%;margin:0 0 10px\"><tr>\
             <td bgcolor=\"{CODE_BG}\" style=\"background:{CODE_BG};padding:10px 14px\">\
             <pre style=\"margin:0;font-family:{MONO};font-size:9.5pt;line-height:1.45;\
             white-space:pre-wrap;color:{CODE_TEXT}\">"
        );
    }

    fn code_close(&mut self) {
        self.out.push_str("</pre></td></tr></table>");
    }

    fn table(&mut self, e: &Element, cx: &Context) {
        let _ = write!(
            self.out,
            "<table border=\"1\" cellpadding=\"8\" cellspacing=\"0\" \
             style=\"border-collapse:collapse;border:1px solid {RULE};margin:0 0 10px\">"
        );
        // A table without a <tbody> written out still has body rows.
        let bare_rows = e.elements().any(|c| c.name == "tr");
        let cx = Context {
            in_tbody: bare_rows,
            ..cx.clone()
        };
        self.children(e, &cx);
        self.out.push_str("</table>");
    }

    fn cell(&mut self, e: &Element, cx: &Context) {
        let header = e.name == "th";
        let row_header = header && e.attr("scope") == Some("row");
        let align = e
            .attr("align")
            .filter(|a| matches!(*a, "left" | "right" | "center"))
            .or(e.has_class("num").then_some("right"))
            .unwrap_or("left");
        let fill = if header && !row_header {
            Some(SURFACE)
        } else if cx.in_tbody && cx.row.is_multiple_of(2) {
            Some(STRIPE)
        } else {
            None
        };
        let mut attrs = format!(" align=\"{align}\" valign=\"top\"");
        for key in ["colspan", "rowspan"] {
            if let Some(v) = e.attr(key) {
                let _ = write!(attrs, " {key}=\"{}\"", escape_attr(v));
            }
        }
        let mut style = format!("border:1px solid {RULE};padding:8px 12px;text-align:{align}");
        if let Some(fill) = fill {
            let _ = write!(attrs, " bgcolor=\"{fill}\"");
            let _ = write!(style, ";background:{fill}");
        }
        if header {
            style.push_str(";font-weight:600");
        }
        if row_header {
            let _ = write!(style, ";color:{TEXT_MUTED}");
        }
        self.wrap(&e.name, &attrs, &style, e, cx);
    }

    /// A diagram: the picture the view drew of it, or else its source as code.
    fn diagram(&mut self, e: &Element, _cx: &Context) {
        let pictures: Vec<&Element> = e
            .descendants()
            .into_iter()
            .filter(|d| d.name == "img")
            .collect();
        if pictures.is_empty() {
            let source = e
                .attr("data-source")
                .map(str::to_owned)
                .or_else(|| {
                    e.descendants()
                        .into_iter()
                        .find(|d| d.name == "pre")
                        .map(Element::text)
                })
                .unwrap_or_default();
            self.code_text(&source);
            return;
        }
        self.out
            .push_str("<p style=\"margin:0 0 10px;text-align:center\">");
        for picture in pictures {
            self.image(picture);
        }
        self.out.push_str("</p>");
    }

    fn image(&mut self, e: &Element) {
        let alt = e.attr("alt").unwrap_or("").trim();
        let Some(src) = e.attr("src").filter(|s| !s.is_empty()) else {
            // A picture from the internet the user hasn't shown, or one outside the folder.
            if !alt.is_empty() {
                self.alt_text(alt);
            }
            return;
        };
        let bytes = match src.strip_prefix("data:") {
            Some(_) => decode_data_uri(src),
            None => (self.images)(src),
        };
        let name = if alt.is_empty() {
            tr!("clipboard-picture")
        } else {
            alt.to_owned()
        };
        let Some((bytes, mime)) = bytes.and_then(|b| picture_type(&b).map(|m| (b, m))) else {
            self.alt_text(&name);
            return;
        };
        if bytes.len() > self.limits.per_image || self.used + bytes.len() > self.limits.total {
            self.alt_text(&tr!("clipboard-picture-too-large", picture = name));
            return;
        }
        self.used += bytes.len();
        let size = e
            .attr("width")
            .and_then(|w| w.trim().parse::<u32>().ok())
            .map(|w| (w, None))
            .or_else(|| dimensions(&bytes).map(|(w, h)| (w, Some(h))));
        let mut attrs = String::new();
        // A picture whose size can't be read is still held to the reading width.
        let cap = if size.is_some() {
            "max-width:100%".to_owned()
        } else {
            format!("max-width:{MAX_WIDTH}px")
        };
        if let Some((width, height)) = size {
            let shown = width.min(MAX_WIDTH);
            let _ = write!(attrs, " width=\"{shown}\"");
            if let Some(h) = height {
                let shown_h = (u64::from(h) * u64::from(shown) / u64::from(width.max(1))) as u32;
                let _ = write!(attrs, " height=\"{shown_h}\"");
            }
        }
        let _ = write!(
            self.out,
            "<img src=\"data:{mime};base64,{}\" alt=\"{}\"{attrs} style=\"{cap};border:0\">",
            base64(&bytes),
            escape_attr(alt)
        );
    }

    fn alt_text(&mut self, text: &str) {
        let _ = write!(
            self.out,
            "<span style=\"color:{TEXT_MUTED};font-style:italic\">[{}]</span>",
            escape_text(text)
        );
    }
}

fn is_task_item(li: &Element) -> bool {
    li.name == "li"
        && li
            .elements()
            .next()
            .is_some_and(|first| first.name == "input" && first.attr("type") == Some("checkbox"))
}

/// Only links that work outside the document: to the web or to mail.
fn is_web_link(href: &str) -> bool {
    let lower = href.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
}

/// highlight.js's colour for a token, as an inline style.
fn highlight_style(span: &Element) -> Option<String> {
    let class = span.attr("class")?;
    let has = |c: &str| class.split_ascii_whitespace().any(|x| x == c);
    let colour = if has("hljs-comment") || has("hljs-quote") {
        return Some(format!("color:{HL_COMMENT};font-style:italic"));
    } else if has("hljs-keyword")
        || has("hljs-selector-tag")
        || has("hljs-literal")
        || has("hljs-built_in")
        || has("hljs-doctag")
    {
        HL_KEYWORD
    } else if has("hljs-string")
        || has("hljs-regexp")
        || has("hljs-symbol")
        || has("hljs-template-tag")
    {
        HL_STRING
    } else if has("hljs-number") || has("hljs-bullet") || has("hljs-link") {
        HL_NUMBER
    } else if has("hljs-section") {
        return Some(format!("color:{HL_TITLE};font-weight:600"));
    } else if has("hljs-type")
        || has("class_")
        || has("hljs-selector-class")
        || has("hljs-selector-id")
    {
        HL_TYPE
    } else if has("hljs-title") {
        HL_TITLE
    } else if has("hljs-attr")
        || has("hljs-attribute")
        || has("hljs-property")
        || has("hljs-params")
        || has("hljs-variable")
        || has("hljs-template-variable")
        || has("hljs-name")
    {
        HL_ATTR
    } else if has("hljs-meta") || has("hljs-tag") || has("hljs-punctuation") || has("hljs-subst") {
        HL_META
    } else if has("hljs-deletion") {
        HL_DELETION
    } else if has("hljs-addition") {
        HL_ADDITION
    } else if has("hljs-strong") {
        return Some("font-weight:600".to_owned());
    } else if has("hljs-emphasis") {
        return Some("font-style:italic".to_owned());
    } else {
        return None;
    };
    Some(format!("color:{colour}"))
}

// Pictures

/// The picture's media type, by its first bytes: only the types every paste target shows.
fn picture_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// A PNG's, GIF's or JPEG's width and height, from its header.
fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let be16 = |b: &[u8]| u32::from(u16::from_be_bytes([b[0], b[1]]));
    let le16 = |b: &[u8]| u32::from(u16::from_le_bytes([b[0], b[1]]));
    let le24 = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], 0]);
    match picture_type(bytes)? {
        "image/png" if bytes.len() >= 24 => Some((be32(&bytes[16..]), be32(&bytes[20..]))),
        "image/gif" if bytes.len() >= 10 => Some((le16(&bytes[6..]), le16(&bytes[8..]))),
        "image/jpeg" => {
            // Walk the segments to the first start-of-frame.
            let mut at = 2;
            while at + 9 < bytes.len() {
                if bytes[at] != 0xff {
                    return None;
                }
                let marker = bytes[at + 1];
                let length = be16(&bytes[at + 2..]) as usize;
                if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                    return Some((be16(&bytes[at + 7..]), be16(&bytes[at + 5..])));
                }
                at += 2 + length;
            }
            None
        }
        // The three kinds of WebP: lossy (VP8), lossless (VP8L) and extended (VP8X).
        "image/webp" if bytes.len() >= 30 => match &bytes[12..16] {
            b"VP8 " if bytes[23..26] == [0x9d, 0x01, 0x2a] => {
                Some((le16(&bytes[26..]) & 0x3fff, le16(&bytes[28..]) & 0x3fff))
            }
            b"VP8L" if bytes[20] == 0x2f => {
                let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            b"VP8X" => Some((le24(&bytes[24..]) + 1, le24(&bytes[27..]) + 1)),
            _ => None,
        },
        _ => None,
    }
}

/// The bytes of a base64 `data:` URI; `None` for any other.
fn decode_data_uri(uri: &str) -> Option<Vec<u8>> {
    let (meta, data) = uri.strip_prefix("data:")?.split_once(',')?;
    if !meta.to_ascii_lowercase().ends_with(";base64") {
        return None;
    }
    unbase64(data)
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut bits = 0u32;
    let mut count = 0;
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Some(out)
}

// The plain text

#[derive(Default, Clone)]
struct TextContext {
    in_pre: bool,
    /// Each open list: how it numbers its items, or `None` for bullets.
    lists: Vec<Option<Numbering>>,
    /// Directly in a list item, where paragraphs aren't spaced apart.
    in_item: bool,
}

/// An ordered list's numbering: the next item's number, and the step to the one after.
#[derive(Clone, Copy)]
struct Numbering {
    next: i64,
    step: i64,
}

impl Numbering {
    /// As HTML numbers `ol`: from `start`, or 1 (or, reversed, its number of items), counting up
    /// (or, reversed, down).
    fn of(ol: &Element) -> Self {
        let reversed = ol.attr("reversed").is_some();
        let items = ol.elements().filter(|c| c.name == "li").count();
        let start = ol.attr("start").and_then(|s| s.trim().parse().ok());
        Self {
            next: start.unwrap_or(if reversed {
                i64::try_from(items).unwrap_or(i64::MAX)
            } else {
                1
            }),
            step: if reversed { -1 } else { 1 },
        }
    }
}

#[derive(Default)]
struct Text {
    out: String,
    /// Line ends owed before the next text (2 for a blank line).
    pending: usize,
    /// What starts each new line (list indents, quote marks).
    prefix: String,
    /// What the blank lines owed carry: the prefix when they were owed.
    gap_prefix: String,
    /// Just after a list item's marker, where its first block starts on the same line.
    at_marker: bool,
    /// The last text was code's, whose spaces at a line's end are kept.
    verbatim: bool,
    /// Of the line ends owed, those code's own text ended with: kept at the end, where a
    /// block's are not.
    code_owed: usize,
}

impl Text {
    fn finish(mut self) -> String {
        self.end_line();
        if self.code_owed > 0 {
            self.pending = self.code_owed;
            self.line_ends();
        } else if !self.out.is_empty() {
            self.out.push('\n');
        }
        self.out
    }

    /// Before a line end: prose's spaces at the end of its line go, code's stay.
    fn end_line(&mut self) {
        if !self.verbatim {
            let kept = self.out.trim_end_matches([' ', '\t']).len();
            self.out.truncate(kept);
        }
    }

    /// Ends the current block: the next text starts after a blank line.
    fn block(&mut self) {
        self.owe(2);
    }

    fn line(&mut self) {
        self.owe(1);
    }

    /// Owes `lines` line ends before the next text. A blank line carries the marks both sides
    /// of it share: between a quote's paragraphs it's marked, before or after a quote it isn't.
    fn owe(&mut self, lines: usize) {
        if self.out.is_empty() || self.at_marker {
            return;
        }
        if self.pending == 0 {
            self.gap_prefix = self.prefix.clone();
        }
        self.pending = self.pending.max(lines);
    }

    fn put(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.pending > 0 {
            self.end_line();
            self.line_ends();
            self.out.push_str(&self.prefix);
        } else if self.out.is_empty() {
            self.out.push_str(&self.prefix);
        }
        self.out.push_str(text);
        self.at_marker = false;
        self.verbatim = false;
    }

    /// Writes the line ends owed, each blank line carrying the marks both sides of it share.
    fn line_ends(&mut self) {
        for i in 0..self.pending {
            if i > 0 {
                let shared: String = self
                    .gap_prefix
                    .chars()
                    .zip(self.prefix.chars())
                    .take_while(|(a, b)| a == b)
                    .map(|(a, _)| a)
                    .collect();
                self.out.push_str(shared.trim_end());
            }
            self.out.push('\n');
        }
        self.pending = 0;
        self.code_owed = 0;
    }

    /// Code's text, as it is.
    fn put_verbatim(&mut self, text: &str) {
        if !text.is_empty() {
            self.put(text);
            self.verbatim = true;
        }
    }

    /// Inline text, its spaces collapsed.
    fn words(&mut self, text: &str) {
        let collapsed: String = text.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
        let at_line_start = self.pending > 0 || self.out.is_empty() || self.out.ends_with('\n');
        let leading = text.starts_with(|c: char| c.is_ascii_whitespace())
            && !at_line_start
            && !self.out.ends_with(' ');
        let trailing = text.ends_with(|c: char| c.is_ascii_whitespace()) && !collapsed.is_empty();
        if collapsed.is_empty() {
            if !text.is_empty() && !at_line_start && !self.out.ends_with(' ') {
                self.out.push(' ');
            }
            return;
        }
        if leading {
            self.out.push(' ');
        }
        self.put(&collapsed);
        if trailing {
            self.out.push(' ');
        }
    }

    fn children(&mut self, element: &Element, cx: &TextContext) {
        let mut cx = cx.clone();
        for child in &element.children {
            match child {
                Node::Text(text) if cx.in_pre => {
                    let mut lines = text.split('\n').peekable();
                    while let Some(line) = lines.next() {
                        self.put_verbatim(line);
                        if lines.peek().is_some() {
                            self.pending += 1;
                            self.code_owed = self.pending;
                        }
                    }
                }
                Node::Text(text) => self.words(text),
                Node::Element(e) if e.name == "li" => {
                    // An item's own value sets its number, and the next ones count on from it.
                    if let Some(Some(numbering)) = cx.lists.last_mut()
                        && let Some(value) = e.attr("value").and_then(|v| v.trim().parse().ok())
                    {
                        numbering.next = value;
                    }
                    self.element(e, &cx);
                    if let Some(Some(numbering)) = cx.lists.last_mut() {
                        numbering.next = numbering.next.saturating_add(numbering.step);
                    }
                }
                Node::Element(e) => self.element(e, &cx),
            }
        }
    }

    fn element(&mut self, e: &Element, cx: &TextContext) {
        if is_dropped(e) {
            return;
        }
        match e.name.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "dl" | "figcaption" | "summary"
            | "caption"
                if cx.in_item =>
            {
                self.line();
                self.children(e, cx);
                self.line();
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "dl" | "figcaption" | "summary"
            | "caption" => {
                self.block();
                self.children(e, cx);
                self.block();
            }
            "dt" | "dd" | "tr" => {
                self.line();
                if e.name == "tr" {
                    let cells: Vec<String> = e
                        .elements()
                        .filter(|c| c.name == "th" || c.name == "td")
                        .map(inline_text)
                        .collect();
                    self.put(&cells.join("\t"));
                } else {
                    self.children(e, cx);
                }
                self.line();
            }
            "table" => {
                self.block();
                // Its own rows only: a table inside a cell is that cell's text.
                for child in e.elements() {
                    match child.name.as_str() {
                        "tr" => self.element(child, cx),
                        "thead" | "tbody" | "tfoot" => {
                            for row in child.elements().filter(|r| r.name == "tr") {
                                self.element(row, cx);
                            }
                        }
                        _ => {}
                    }
                }
                self.block();
            }
            "ul" | "ol" => {
                if cx.lists.is_empty() {
                    self.block();
                } else {
                    self.line();
                }
                let mut inner = cx.clone();
                inner.in_item = false;
                inner.lists.push((e.name == "ol").then(|| Numbering::of(e)));
                self.children(e, &inner);
                if cx.lists.is_empty() {
                    self.block();
                } else {
                    self.line();
                }
            }
            "li" => {
                self.line();
                let marker = match cx.lists.last() {
                    Some(Some(numbering)) => format!("{}. ", numbering.next),
                    _ if is_task_item(e) => String::new(),
                    _ => "• ".to_owned(),
                };
                let saved = self.prefix.clone();
                self.put_marker(&marker);
                // Its lines, and any list inside it, line up after the marker (a task's tick
                // box is its marker).
                let width = if marker.is_empty() {
                    2
                } else {
                    marker.chars().count()
                };
                self.prefix = format!("{saved}{}", " ".repeat(width));
                self.children(
                    e,
                    &TextContext {
                        in_item: true,
                        ..cx.clone()
                    },
                );
                self.prefix = saved;
                self.line();
            }
            "input" => {
                let ticked = e.attr("checked").is_some();
                self.put(if ticked { "☑ " } else { "☐ " });
            }
            "blockquote" => {
                self.block();
                let saved = self.prefix.clone();
                self.prefix.push_str("> ");
                self.children(e, cx);
                self.prefix = saved;
                self.block();
            }
            "aside" => {
                self.block();
                self.children(e, cx);
                self.block();
            }
            "pre" => {
                self.block();
                self.children(
                    e,
                    &TextContext {
                        in_pre: true,
                        ..cx.clone()
                    },
                );
                self.block();
            }
            "div" if e.has_class("mermaid") => {
                self.block();
                let pictures: Vec<_> = e
                    .descendants()
                    .into_iter()
                    .filter(|d| d.name == "img")
                    .collect();
                if pictures.is_empty() {
                    let source = e.attr("data-source").map(str::to_owned).unwrap_or_default();
                    self.children(
                        &Element {
                            name: "pre".into(),
                            attrs: Vec::new(),
                            children: vec![Node::Text(source)],
                        },
                        &TextContext {
                            in_pre: true,
                            ..cx.clone()
                        },
                    );
                } else {
                    for picture in pictures {
                        self.element(picture, cx);
                    }
                }
                self.block();
            }
            "details" | "div" | "section" | "figure" | "main" | "article" | "header" | "footer" => {
                self.block();
                self.children(e, cx);
                self.block();
            }
            "hr" => {
                self.block();
                self.put("———");
                self.block();
            }
            "br" => self.pending += 1,
            "img" => {
                let alt = e.attr("alt").unwrap_or("").trim();
                if !alt.is_empty() {
                    self.words(&format!("[{alt}]"));
                }
            }
            "a" => {
                let text = inline_text(e);
                self.children(e, cx);
                if let Some(href) = e.attr("href").filter(|h| is_web_link(h)) {
                    let plain = href.trim_start_matches("mailto:");
                    let bare = href
                        .trim_start_matches("https://")
                        .trim_start_matches("http://");
                    if text != href && text != plain && text != bare {
                        self.words(&format!(" ({plain})"));
                    }
                }
            }
            "sup" if e.has_class("footnote-ref") => {
                self.put(&format!("[{}]", inline_text(e)));
            }
            _ => self.children(e, cx),
        }
    }

    /// A list item's marker, which starts its line.
    fn put_marker(&mut self, marker: &str) {
        let saved = std::mem::take(&mut self.prefix);
        let owed = self.pending;
        if owed > 0 || self.out.is_empty() {
            if owed > 0 {
                self.end_line();
            }
            for _ in 0..owed {
                self.out.push('\n');
            }
            self.pending = 0;
            self.code_owed = 0;
        }
        self.out.push_str(&saved);
        self.out.push_str(marker);
        self.prefix = saved;
        self.at_marker = !marker.is_empty();
        self.verbatim = false;
    }
}

/// Elements that start a new line of their own.
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "li",
    "ul",
    "ol",
    "table",
    "tr",
    "td",
    "th",
    "blockquote",
    "pre",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "dt",
    "dd",
    "section",
    "aside",
    "figure",
    "figcaption",
    "details",
    "summary",
];

/// An element's text on one line, its spaces collapsed.
fn inline_text(e: &Element) -> String {
    let mut text = String::new();
    collect_inline(e, &mut text);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_inline(e: &Element, out: &mut String) {
    for child in &e.children {
        match child {
            Node::Text(t) => out.push_str(t),
            Node::Element(c) if is_dropped(c) => {}
            Node::Element(c) if c.name == "input" => {
                out.push_str(if c.attr("checked").is_some() {
                    "☑ "
                } else {
                    "☐ "
                });
            }
            Node::Element(c) if c.name == "img" => {
                if let Some(alt) = c.attr("alt").filter(|a| !a.trim().is_empty()) {
                    let _ = write!(out, "[{}]", alt.trim());
                }
            }
            Node::Element(c) if c.name == "br" => out.push(' '),
            // A block inside (a nested table's cells, paragraphs) is set apart by spaces.
            Node::Element(c) if BLOCKS.contains(&c.name.as_str()) => {
                out.push(' ');
                collect_inline(c, out);
                out.push(' ');
            }
            Node::Element(c) => collect_inline(c, out),
        }
    }
}

fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
    out
}

fn escape_attr(text: &str) -> String {
    escape_text(text).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_images(_: &str) -> Option<Vec<u8>> {
        None
    }

    fn copy(html: &str) -> Copied {
        build(html, &no_images, &Limits::default())
    }

    /// A PNG of `width` × `height` with `extra` bytes of padding after its header.
    pub(crate) fn png(width: u32, height: u32, extra: usize) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend(std::iter::repeat_n(0u8, extra));
        bytes
    }

    #[test]
    fn base64_round_trips() {
        for input in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            assert_eq!(unbase64(&base64(input)).unwrap(), input);
        }
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"fo"), "Zm8=");
    }

    #[test]
    fn cf_html_offsets_are_exact_bytes_with_non_ascii_text() {
        let fragment = "<p>Café – 日本語 🎉</p>";
        let bytes = cf_html(fragment);
        let text = std::str::from_utf8(&bytes).unwrap();
        let offset = |key: &str| -> usize {
            let at = text.find(key).unwrap() + key.len();
            text[at..at + 10].parse().unwrap()
        };
        let (sh, eh, sf, ef) = (
            offset("StartHTML:"),
            offset("EndHTML:"),
            offset("StartFragment:"),
            offset("EndFragment:"),
        );
        assert_eq!(&bytes[sf..ef], fragment.as_bytes());
        assert!(bytes[sh..].starts_with(b"<html>"));
        assert_eq!(eh, bytes.len());
        assert!(bytes[..sf].ends_with(b"<!--StartFragment-->"));
        assert!(bytes[ef..].starts_with(b"<!--EndFragment-->"));
    }

    #[test]
    fn nothing_is_left_to_a_stylesheet() {
        let copied = copy(
            r#"<style>p{color:red}</style><script>alert(1)</script>
            <p class="x" id="user-content-a" data-x="1">Text <svg><text>no</text></svg></p>
            <div class="code-block"><pre><code class="language-rs"><span class="hljs-keyword">fn</span> f</code></pre><button class="copy">Copy</button></div>"#,
        );
        let html = &copied.html;
        for banned in [
            "<style", "<script", "class=", " id=", "data-", "<svg", "<button", "alert(1)",
        ] {
            assert!(!html.contains(banned), "{banned} in {html}");
        }
        assert!(html.contains(&format!("<span style=\"color:{HL_KEYWORD}\">fn</span>")));
        assert!(!copied.text.contains("Copy"));
    }

    #[test]
    fn pictures_are_embedded_up_to_the_limits() {
        let small = png(1200, 600, 100);
        let images = |src: &str| match src {
            "https://legible-doc.localhost/a.png" | "https://legible-doc.localhost/b.png" => {
                Some(small.clone())
            }
            "https://legible-doc.localhost/notes.txt" => Some(b"text".to_vec()),
            _ => None,
        };
        let html = r#"<p><img src="https://legible-doc.localhost/a.png" alt="A"><img src="https://legible-doc.localhost/b.png" alt="B"><img src="https://legible-doc.localhost/gone.png" alt="Gone"><img src="https://legible-doc.localhost/notes.txt" alt="Text"></p>"#;

        let all = build(html, &images, &Limits::default());
        assert_eq!(
            all.html
                .matches("<img src=\"data:image/png;base64,")
                .count(),
            2
        );
        // Capped at the reading width, in proportion.
        assert!(all.html.contains("width=\"640\" height=\"320\""));
        assert!(all.html.contains("[Gone]") && all.html.contains("[Text]"));

        let per_image = Limits {
            per_image: small.len() - 1,
            total: usize::MAX,
        };
        let none = build(html, &images, &per_image);
        assert!(!none.html.contains("<img"));
        assert!(
            none.html.contains("[A (picture too large to copy)]"),
            "{}",
            none.html
        );

        let total = Limits {
            per_image: usize::MAX,
            total: small.len() * 3 / 2,
        };
        let one = build(html, &images, &total);
        assert_eq!(one.html.matches("<img").count(), 1);
        assert!(one.html.contains("[B (picture too large to copy)]"));
    }

    #[test]
    fn remote_pictures_are_their_alt_text_or_nothing() {
        let copied = copy(
            r#"<p><img data-remote-src="https://x.example/a.png" alt="Logo"><span class="remote-image">Picture from the internet, not shown</span><img data-remote-src="https://x.example/b.png"></p>"#,
        );
        assert!(copied.html.contains("[Logo]"));
        assert!(!copied.html.contains("internet"));
        assert_eq!(copied.text, "[Logo]\n");
    }

    #[test]
    fn data_uri_pictures_are_checked_and_kept() {
        let uri = format!("data:image/png;base64,{}", base64(&png(10, 20, 4)));
        let copied = copy(&format!(
            r#"<div class="mermaid diagram" data-source="graph LR"><img src="{uri}" alt="Diagram"></div>"#
        ));
        assert!(copied.html.contains(&format!(
            "<img src=\"{uri}\" alt=\"Diagram\" width=\"10\" height=\"20\""
        )));
        assert!(!copied.html.contains("graph LR"));
    }

    #[test]
    fn a_diagram_without_its_picture_is_its_source() {
        let copied = copy(r#"<div class="mermaid" data-source="graph LR&#10;  A --&gt; B"></div>"#);
        assert!(copied.html.contains("<pre"));
        assert!(copied.html.contains("A --&gt; B"));
        assert_eq!(copied.text, "graph LR\n  A --> B\n");
    }

    #[test]
    fn webp_sizes_are_read_and_capped() {
        // Lossy: the frame header after the VP8 chunk's header.
        let mut vp8 = b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0".to_vec();
        vp8.extend_from_slice(&[0, 0, 0, 0x9d, 0x01, 0x2a, 0x40, 0x06, 0x20, 0x03]);
        assert_eq!(dimensions(&vp8), Some((1600, 800)));
        // Lossless: 14 bits each of width - 1 and height - 1.
        let bits: u32 = (1600 - 1) | ((800 - 1) << 14);
        let mut vp8l = b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0\x2f".to_vec();
        vp8l.extend_from_slice(&bits.to_le_bytes());
        vp8l.extend_from_slice(&[0; 8]);
        assert_eq!(dimensions(&vp8l), Some((1600, 800)));
        // Extended: 24 bits each of canvas width - 1 and height - 1.
        let mut vp8x = b"RIFF\0\0\0\0WEBPVP8X\0\0\0\0\0\0\0\0".to_vec();
        vp8x.extend_from_slice(&[0x3f, 0x06, 0x00, 0x1f, 0x03, 0x00]);
        assert_eq!(dimensions(&vp8x), Some((1600, 800)));

        let images = |_: &str| Some(vp8.clone());
        let copied = build(
            r#"<img src="w.webp" alt="wide">"#,
            &images,
            &Limits::default(),
        );
        assert!(copied.html.contains("data:image/webp;base64,"));
        assert!(
            copied.html.contains("width=\"640\" height=\"320\""),
            "{}",
            copied.html
        );

        // A WebP whose size can't be read is still capped.
        let mut odd = b"RIFF\0\0\0\0WEBPVP8?".to_vec();
        odd.extend_from_slice(&[0; 20]);
        let images = |_: &str| Some(odd.clone());
        let copied = build(
            r#"<img src="w.webp" alt="odd">"#,
            &images,
            &Limits::default(),
        );
        assert!(
            copied.html.contains("style=\"max-width:640px;border:0\""),
            "{}",
            copied.html
        );
    }

    #[test]
    fn deep_nesting_is_flattened_not_followed() {
        let depth = 100_000;
        let html = format!(
            "{}deep text{}",
            "<div>".repeat(depth),
            "</div>".repeat(depth)
        );
        let tree = parse(&html);
        let mut levels = 0;
        let mut node = &tree;
        while let Some(child) = node.elements().next() {
            levels += 1;
            node = child;
        }
        assert!(levels <= MAX_DEPTH, "{levels}");
        let copied = copy(&format!("{html}<p>after</p>"));
        assert!(copied.text.contains("deep text") && copied.text.contains("after"));
        // A script past the depth still loses its content.
        let script = format!(
            "{}<script>alert(1)</script>x{}",
            "<div>".repeat(200),
            "</div>".repeat(200)
        );
        assert!(!copy(&script).text.contains("alert"));
    }

    #[test]
    fn nested_tables_give_each_cell_once() {
        let copied = copy(
            "<table><tr><td>outer<table><tr><td>inner</td></tr></table>end</td><td>2</td></tr></table>",
        );
        assert_eq!(copied.text, "outer inner end\t2\n");
    }

    #[test]
    fn ordered_lists_number_as_the_html_does() {
        assert_eq!(
            copy(r#"<ol reversed start="3"><li>A</li><li>B</li></ol>"#).text,
            "3. A\n2. B\n"
        );
        assert_eq!(
            copy("<ol reversed><li>A</li><li>B</li><li>C</li></ol>").text,
            "3. A\n2. B\n1. C\n"
        );
        assert_eq!(
            copy(r#"<ol start="-1"><li>A</li><li value="10">B</li><li>C</li></ol>"#).text,
            "-1. A\n10. B\n11. C\n"
        );
        let huge = format!(r#"<ol start="{}"><li>A</li><li>B</li></ol>"#, i64::MAX);
        assert_eq!(copy(&huge).text, format!("{0}. A\n{0}. B\n", i64::MAX));
    }

    #[test]
    fn code_keeps_its_whitespace() {
        let copied = copy("<p>Before  </p><pre><code>one  \n\n  two\t \n</code></pre><p>After</p>");
        assert_eq!(copied.text, "Before\n\none  \n\n  two\t \n\nAfter\n");
        assert_eq!(copy("<pre>end \n</pre>").text, "end \n");
    }

    #[test]
    fn code_at_the_end_keeps_its_blank_lines() {
        assert_eq!(copy("<pre><code>one\n\n\n</code></pre>").text, "one\n\n\n");
        assert_eq!(
            copy("<pre><code>one\n  \n\t\n\n</code></pre>").text,
            "one\n  \n\t\n\n"
        );
        assert_eq!(
            copy("<p>Before</p><pre><code>one\n\n</code></pre>").text,
            "Before\n\none\n\n"
        );
        assert_eq!(
            copy("<blockquote><pre><code>one\n\n</code></pre></blockquote>").text,
            "> one\n\n"
        );
        // Code followed by prose keeps them too, and at least the blank line a block is owed.
        assert_eq!(
            copy("<pre><code>one\n\n\n</code></pre><p>Two</p>").text,
            "one\n\n\nTwo\n"
        );
        assert_eq!(
            copy("<pre><code>one\n</code></pre><p>Two</p>").text,
            "one\n\nTwo\n"
        );
        // Prose's own line ends at the end still go, as do those of an empty block after code.
        assert_eq!(copy("<p>One<br><br></p>").text, "One\n");
        assert_eq!(
            copy("<pre><code>one\n</code></pre><p></p><div></div>").text,
            "one\n"
        );
        assert_eq!(
            copy("<pre><code>one\n\n</code></pre><p>Two<br><br></p>").text,
            "one\n\nTwo\n"
        );
    }

    #[test]
    fn jpeg_and_gif_sizes_are_read() {
        let gif = b"GIF89a\x20\x03\x58\x02rest".to_vec();
        assert_eq!(dimensions(&gif), Some((800, 600)));
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0];
        jpeg.extend_from_slice(&[0xff, 0xc0, 0, 17, 8, 0x01, 0x2c, 0x01, 0x90, 3, 0, 0]);
        assert_eq!(dimensions(&jpeg), Some((400, 300)));
    }

    #[test]
    fn plain_text_reads_on_its_own() {
        let copied = copy(
            "<h1>Title</h1><p>One <em>two</em>\nthree <a href=\"https://e.example/x\">link</a>.</p>\
             <ul><li>A</li><li>B<ul><li>B1</li></ul></li></ul>\
             <ol start=\"3\"><li><p>Three</p></li><li>Four</li></ol>\
             <ul><li><input type=\"checkbox\" checked=\"\" disabled=\"\"> Done</li><li><input type=\"checkbox\" disabled=\"\"> Not</li></ul>\
             <table><thead><tr><th>Name</th><th>Amount</th></tr></thead><tbody><tr><td>A</td><td>1,000</td></tr></tbody></table>\
             <blockquote><p>Quoted</p></blockquote>\
             <pre><code>let x = 1;\n  indented\n</code></pre><hr><p>End</p>",
        );
        assert_eq!(
            copied.text,
            "Title\n\nOne two three link (https://e.example/x).\n\n• A\n• B\n  • B1\n\n\
             3. Three\n4. Four\n\n☑ Done\n☐ Not\n\nName\tAmount\nA\t1,000\n\n> Quoted\n\n\
             let x = 1;\n  indented\n\n———\n\nEnd\n"
        );
    }

    #[test]
    fn a_selection_fragment_with_open_tags_still_copies() {
        let copied = copy("<td>loose cell</td><li>item</p></b>");
        assert!(copied.html.contains("loose cell"));
        assert!(copied.text.contains("item"));
    }

    #[test]
    fn the_colours_are_the_light_themes() {
        let css = include_str!("../../../web/doc/document.css");
        let light = |name: &str| -> String {
            let line = css
                .lines()
                .find(|l| l.trim_start().starts_with(&format!("--{name}:")))
                .unwrap_or_else(|| panic!("--{name}"));
            let value = line.split("light-dark(").nth(1).unwrap();
            value.split(',').next().unwrap().trim().to_owned()
        };
        let pairs = [
            ("text", TEXT),
            ("text-muted", TEXT_MUTED),
            ("accent", ACCENT),
            ("rule", RULE),
            ("surface", SURFACE),
            ("stripe", STRIPE),
            ("quote-bar", QUOTE_BAR),
            ("code-bg", CODE_BG),
            ("code-text", CODE_TEXT),
            ("hl-keyword", HL_KEYWORD),
            ("hl-string", HL_STRING),
            ("hl-number", HL_NUMBER),
            ("hl-comment", HL_COMMENT),
            ("hl-title", HL_TITLE),
            ("hl-type", HL_TYPE),
            ("hl-attr", HL_ATTR),
            ("hl-meta", HL_META),
            ("hl-deletion", HL_DELETION),
            ("hl-addition", HL_ADDITION),
            ("note", NOTE),
            ("note-bg", NOTE_BG),
            ("tip", TIP),
            ("tip-bg", TIP_BG),
            ("important", IMPORTANT),
            ("important-bg", IMPORTANT_BG),
            ("warning", WARNING),
            ("warning-bg", WARNING_BG),
            ("caution", CAUTION),
            ("caution-bg", CAUTION_BG),
        ];
        for (name, value) in pairs {
            assert_eq!(light(name), value, "--{name}");
        }
    }
}
