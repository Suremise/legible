# Legible architecture

How the code fits together, and the decisions behind it. The product brief (what Legible is for,
its features and principles) is the starting point; this file records how it's built.

## Layout

| Path | What |
|---|---|
| `crates/legible-core` | Platform-free: rendering Markdown to safe HTML, the clipboard's HTML, saving, the strings (`i18n`) and the embedded web assets (`assets`). Builds and tests on Linux too (CI checks it). |
| `crates/legible-win` | Windows only (`cfg(windows)`): where Legible keeps its files (`paths`), its logs (`logging`), the document windows and WebView2 (`shell`, `page`, `links`, `confine`, `placement`), one process per user (`instance`), the Win32 calls (`win32`, the only module allowed `unsafe`), the clipboard, and the file associations (`associations`). |
| `crates/legible` | The exe (`windows_subsystem = "windows"`): starts up and wires the two together. Its `build.rs` compiles in the manifest (`res/legible.exe.manifest`), the app icon and the version info. |
| `assets/` | The app icon and the `.md` file icon, drawn by `assets/export.py` (assets/README.md). |
| `web/doc/`, `web/chrome/`, `web/editor/` | Legible's own CSS, JavaScript and HTML: the document, the toolbar/outline/find, the editor. Compiled into the exe. |
| `third_party/js/` | The pinned, audited JavaScript bundles: highlight.js, Mermaid, CodeMirror. Compiled into the exe. |
| `locales/en/` | The strings, in Fluent files. Compiled into the exe. |
| `scripts/` | Build-only scripts: fetching and verifying the JavaScript, building CodeMirror, the licence audit. |

**The rules every change keeps:** platform-free code in `legible-core`, Windows code in
`legible-win`; user-visible strings through Fluent; everything shipped MIT or equally permissive
(below), with system fonts only; no network and no telemetry; tests use temp folders only.

## The parser

**Chosen: `comrak`** (0.59.0, BSD-2-Clause), with default features off (no command line, no
syntect: highlight.js highlights code in the view). Checked against `pulldown-cmark` (0.13.5, MIT)
on 27 Oct 2026, with GitHub-flavoured Markdown on in both:

| | comrak | pulldown-cmark |
|---|---|---|
| Tables, with column alignment | yes (`align` attributes) | yes (inline `style`) |
| Task lists | yes, disabled checkboxes | yes, disabled checkboxes |
| Strikethrough | yes | yes |
| Autolinks (bare `https://…` and `www.…`) | **yes** | **no**: left as text |
| Footnotes | yes, GitHub's markup: numbered, a back-link, collected at the end | yes, but its own markup: labelled by name, left where they're defined, no back-link |
| GitHub alerts (`> [!NOTE]`…) | yes: `markdown-alert-note` with its title ("Note") | yes, a class only: `<blockquote class="markdown-alert-note">`, no title |
| Front matter | yes: `---` (any delimiter), as a node of its own, out of the body | yes: YAML `---` and TOML `+++`, as metadata events |
| A 2 MB document (GFM sample repeated, release build, this PC) | about 70 ms | about 16 ms |
| Licence | BSD-2-Clause (and its tree passes `cargo deny`) | MIT |

pulldown-cmark is about four times faster, but both are far inside the budget (a 2 MB document
stays responsive; under 700 ms for a typical one, the whole view included). comrak matches
GitHub's output where pulldown-cmark doesn't: bare links become links, footnotes read as GitHub
shows them, and alerts carry their titles. Legible's readers get documents written for GitHub, so
fidelity wins. comrak also gives an AST, which suits the outline, heading ids, Mermaid blocks and
the image rewriting the renderer needs. Front matter, YAML or TOML, is taken off before comrak
parses (below).

## Rendering

**The API** (`legible_core::render`, for the window):

```rust
pub fn render(markdown: &str, base_dir: &Path) -> Rendered;
pub fn render_with(markdown: &str, base_dir: &Path, options: &RenderOptions) -> Rendered;

pub struct Rendered {
    pub html: String,               // sanitised HTML for the page's body
    pub outline: Vec<Heading>,      // { level, text, id }, in order
    pub properties: Vec<Property>,  // { key, value }, the front matter's, in order
    pub has_remote_images: bool,    // some <img data-remote-src="…"> wait for the user
    pub has_mermaid: bool,          // some <div class="mermaid" data-source="…"> to draw
}
pub struct RenderOptions {
    pub doc_base: String,  // default render::DOC_BASE
    pub sourcepos: bool,   // default false: mark blocks with their lines, for the editor
}
```

`base_dir` is the document's folder. It bounds what the document can reach, and is compared as
text, never read, so rendering does no I/O. `doc_base` is where the view serves that folder:
`https://legible-doc.localhost/` by default, which is how WebView2 (through wry, with its https
option) serves a custom scheme `legible-doc`. Another view passes its own.

`sourcepos` is for the editor's preview: each block (paragraphs, headings, lists and their
items, quotes, tables, code, and a Mermaid block's `div`) gets
`data-sourcepos="<line>:<column>-<line>:<column>"`, its lines counted from 1 in the whole file,
front matter included (each node's position is moved down in comrak's tree, before the HTML is
written), and the sanitiser keeps that one `data-` attribute. Only the renderer's marks count:
in the document's own raw HTML, an attribute named `data-sourcepos` (in any case) is renamed
`data-not-sourcepos`, and so dropped. Only attribute names are read for it, as the HTML
tokenizer reads tags (comments, quoted values and a `>` inside one are skipped), so the
document's text and its attribute values never change. It's off for View, so the golden
fixtures and what View shows don't change.

**The steps:**
1. **Front matter** (`frontmatter`): YAML between `---` lines or TOML between `+++` lines at the
   very top (after a BOM). It counts only if it parses as a mapping; anything else stays Markdown,
   so a document that opens with a rule isn't eaten. Values are shown as text: lists joined with
   commas, nested tables as `key: value`. The properties render as
   `<details class="properties">` with a table, first in `html`.
2. **comrak** parses the rest as GitHub-flavoured Markdown (tables, task lists, strikethrough,
   autolinks, footnotes, alerts), keeping the document's raw HTML for the sanitiser.
3. **The tree is rewritten:** a GitHub alert becomes `<aside class="alert note">` (`tip`,
   `important`, `warning`, `caution`) with `<p class="alert-title">`, the title from Fluent
   unless the alert names its own. A ```` ```mermaid ```` block becomes
   `<div class="mermaid" data-source="…">`, its source escaped, for the bundled Mermaid; nothing
   else of it is rendered. **Footnotes** are resolved here, not by comrak, which is told to
   leave the definitions where they are: of a label defined more than once comrak gives the
   references to the last and leaves the earlier ones where they were written, and its
   footnotes list, opened at the first of them, would take in every block after it. As GitHub
   does, the first definition is kept and the rest dropped, before any reference is counted,
   so a reference inside a dropped one counts for nothing. Labels match as comrak matches them
   (trimmed, spaces run together, Unicode case folded with the `caseless` crate comrak uses, so
   `Straße` is `STRASSE`); references are numbered in the document's order, and a definition
   referenced goes to the end with its count for the back-links, one never referenced nowhere.
4. **Headings** (`outline`) get GitHub's slugs as ids as they're written (lower case,
   punctuation dropped, spaces to hyphens, a repeat given `-1`, `-2`), each prefixed
   `user-content-`, and the same ids go in the outline. Code blocks keep `class="language-x"`.
5. **Sanitising** (`sanitize`, ammonia with an explicit allowlist):
   - **removed with their content:** `<script>`, `<style>`, `<iframe>`, `<object>`, `<embed>`,
     `<template>`, `<noscript>`, form controls (only a task list's tick stays, as a disabled
     checkbox), and **SVG and MathML, which are stripped, not sanitised to a subset**;
   - **removed, their text kept:** other tags not on the list (`<meta>`, `<form>`, `<link>`,
     `<base>`…);
   - **attributes:** only those on the list: no `on*`, no `style`. Classes are only Legible's own;
   - **ids:** every id gets the `user-content-` prefix, and so does every `#fragment` link, so a
     document can't clobber the page's own ids;
   - **links:** `http`, `https` and `mailto` stay, as do relative links and fragments; a `file:`
     link only when it points inside `base_dir`; anything else (`javascript:`, `data:`…) loses
     its `href`; every link gets `rel="noopener noreferrer"`;
   - **images:**
     - a relative `src`, or a `file:` one inside `base_dir`, is rewritten onto `doc_base`,
       percent-encoded; one that leaves the folder (`..` past it, or a rooted path) loses its
       `src`;
     - an `http`/`https` `src` becomes `data-remote-src` with no `src`, and sets
       `has_remote_images`;
     - `data:` is kept only for PNG, JPEG, GIF and WebP.

The view sets `src` from `data-remote-src` when the user allows remote images, and refuses a
path outside the folder itself too.

**Tests:** golden fixtures in `crates/legible-core/tests/fixtures/` (`<name>.md` renders to exactly
`<name>.html`, with the flags and outline as comments at the top; set
`LEGIBLE_REGENERATE_FIXTURES=1` to rewrite them after a change made on purpose, then review the
diff). The security fixture's tests check that nothing can run script or reach the network. Unit
tests cover the sanitiser's URL rules on both Windows and Linux paths.

**Speed:** a 2 MB document (the fixtures repeated) renders in about 0.4 s in a release build on
this PC (0.40 to 0.45 s over three runs, 27 Oct 2026). Most of that is the sanitiser; comrak alone
takes about 70 ms. A typical document under 200 KB renders in about 40 ms. A debug build takes
about 2.5 s.

## Web assets and the vendored JavaScript

**Everything the view loads is in the exe** (`legible_core::assets`, with `include_dir`), so the
portable zip is one file and nothing is fetched at run time. `assets::get(path)` serves
`doc/…`, `chrome/…` and `editor/…` from `web/`, and `js/…` from `third_party/js/`, only for
types a page loads (HTML, CSS, JavaScript, JSON, SVG, PNG, ICO); `..` is never followed.
`build.rs` makes cargo rebuild the crate when a file in either folder changes.

**The vendored bundles are pinned and verified:** `third_party/js/VERSIONS` pins each file's
SHA-256 and where it comes from.

- **highlight.js 11.12.0** (BSD-3-Clause): the prebuilt `highlight.min.js` (its common languages)
  from `@highlightjs/cdn-assets`.
- **CodeMirror 6** (MIT), which publishes no prebuilt browser bundle, and **Mermaid 12.2.0**
  (MIT) are built by Legible: `scripts/build-js.ps1 -Bundle codemirror|mermaid` bundles
  `scripts/js/<bundle>/entry.js`, at the versions its lockfile pins, with the pinned esbuild
  (`scripts/js/build.mjs`), into one minified script: CodeMirror's sets `window.CM`, Mermaid's
  `window.mermaid`. The builds are reproducible: each fails unless its output has the pinned
  SHA-256.
- **Mermaid without ELK (decided):** Mermaid's own prebuilt bundle includes elkjs, which is
  EPL-2.0, so Legible's build replaces it with a stub (`scripts/js/mermaid/elk-stub.js`) and
  nothing of it ships. Mermaid 12 lays graphs out with ELK by default, so the bundle makes
  dagre (MIT, the default before 12) the default instead; every diagram type renders
  (flowcharts, sequence, class, state, ER, Gantt, pie, mind maps, front matter, and
  `flowchart-elk` with dagre), and only a diagram that sets `layout: elk` in its own config
  shows Mermaid's error box. Mermaid's dependencies are held at the versions its own 12.2.0
  bundle has (`overrides` in `scripts/js/mermaid/package.json`). roughjs (the hand-drawn look),
  which Mermaid's release patches, comes unpatched from npm, and from its unbundled modules
  (below); fastdom, its other patched package, is inlined in Mermaid's own build with its patch.
- **No package hides another:** a package whose prebundled file folds in its own dependencies
  (roughjs's `bundled/rough.esm.js` holds hachure-fill, path-data-parser, points-on-curve and
  points-on-path) is built from its unbundled modules instead (`unbundled` in the bundle's
  `package.json`), so esbuild sees those dependencies and the audit lists them. To catch the
  next one, the build fails when a bundled package declares a dependency that has no code in
  the bundle, unless `notBundled` says why (types only, a command line's, tree-shaken out,
  never imported, stubbed).

`scripts/fetch-js.ps1 -VerifyOnly` (CI) checks every file against its pin, that no unpinned
bundle sits beside them, and that `LICENSES.md` audited each file at its pinned SHA-256. Without
`-VerifyOnly` it fetches a missing or changed file again: from its npm tarball, checked against
the registry's sha512 integrity, or by rebuilding it.

**The licence audit:** `node scripts/audit-js-licences.mjs` lists every package inside each
bundle at its exact version: for a bundle Legible builds, the packages with code in the output
(from esbuild's metafile, written to `scripts/js/<bundle>/bundled-packages.txt`) plus any that a
package's own build inlines (from its bundler's module comments: Mermaid's inlines js-yaml and
fastdom, its parser langium and the vscode-* libraries). It downloads each from npm, reads its
declared licence and every licence file in it, and writes `third_party/js/LICENSES.md` and
`NOTICES.txt` (the texts, for THIRD-PARTY-NOTICES). Anything not MIT or equally permissive is
listed under "Needs a decision" and the script fails: such a package ships only on the
maintainer's decision. Rerun it whenever a pin moves. Today: 96 packages across the three
bundles, all MIT, ISC, BSD, Apache-2.0, or DOMPurify's "MPL-2.0 OR Apache-2.0".

## The document's look

`web/doc/document.css` and `web/doc/document.js` make the rendered HTML read like a document.

**What the page provides** (the window's page, `web/chrome/window.html`):
- the document in `<main id="document">`, `/doc/document.css` linked, and `/doc/document.js`
  loaded (deferred);
- the script's strings in `<script type="application/json" id="legible-doc-strings">`, from
  `legible_core::render::script_strings()` (`locales/en/document.ftl`; escaped to be safe there);
- the view's choices as classes on `<html>`: `theme-light` or `theme-dark` over Windows' own
  (none: follow Windows), `width-narrow` or `width-wide` (the default; none: Comfortable), `font-serif`.

**The stylesheet:**
- **Colours** are variables on `:root`, each written once as `light-dark(light, dark)`, so the
  theme follows `color-scheme`: Windows' own by default, the class over it, and always light in
  print. They follow Windows 11's neutrals; the accent is Windows' default
  blue. `crates/legible-core/tests/contrast.rs` reads them from the file (and the window's from
  `web/chrome/window.css`) and checks every text colour against each background it's shown on,
  in both themes, for WCAG AA (4.5:1), and each bar's icon against its bar (3:1).
- **Properties** (front matter) are laid out as a grid, so a long key (at most 40% of the width)
  or value wraps rather than widening the page.
- **Measure:** 16 px text, line height 1.6, in a column of 60, 72 or 96 `ch` with at least 32 px
  each side. Sans is Segoe UI Variable Text (or Segoe UI), Serif is Georgia (or Cambria), code
  Cascadia Mono (or Consolas): system fonts only.
- **Print** (`@media print`): light, the document alone (everything else in `<body>` is hidden),
  18 mm margins, no break inside a table row, code block, alert, picture or diagram, headings
  kept with what follows, and each link's address after it (not a bare link's, nor one inside
  the document). The page's header and footer are the window's.

**The script** (`window.LegibleDoc.ready` settles when it's done; `LegibleDoc.prepare(root)` for
new content):
- **Tables** go in a frame that scrolls sideways, so the page never does; a table of more than
  15 rows scrolls down in its frame too, so its header sticks. A column whose cells all read as
  numbers is right-aligned, unless the Markdown aligned it.
- **Code** with a language is highlighted (highlight.js, loaded only then); every block gets a
  Copy button, shown on hover or focus.
- **Pictures:** a remote one (`data-remote-src`, no `src`) shows a placeholder until `src` is
  set; a missing one shows its alt text.
- **Mermaid** runs only when there's a diagram, and **in a sandboxed frame, never in the page**
  (below). Each diagram is drawn twice, in Mermaid's `neutral` and `dark` themes, and the
  stylesheet shows the one the page's theme calls for, and the light one in print: a change of
  theme needs no redrawing, and printing never waits on Mermaid. Its source is on a "Show
  source" toggle, and a diagram that fails shows that source with a short message.

**Mermaid's frame (decided, 27 Oct 2026):** Mermaid is a large library that runs on the opened
document's text, and has had script-injection bugs, so it runs where a bug can't reach the
document or the window's bridge to Legible:
- **The frame:** one hidden `<iframe sandbox="allow-scripts">` (no `allow-same-origin`: an
  opaque origin, so no access to the page's DOM, storage or script), opened when the first
  diagram needs it, outside `#document`. It loads `web/doc/mermaid-frame.html`, which loads the
  Mermaid bundle and `mermaid-frame.js`. The page's CSP allows that one URL in `frame-src` and
  no other frame. The frame's own CSP comes as its response header (legible-win's
  `MERMAID_FRAME_CSP`): Legible's scripts, inline styles (Mermaid lays text out with them, and
  measures it wrong without its `<style>`), nothing else (no images, fonts, connections or
  forms); `frame-ancestors` only Legible's page, and `sandbox allow-scripts`, so it's sandboxed
  even if opened by itself. The window's IPC handler takes messages only from the page at `/`,
  never from a frame of it (in WebView2, wry's `window.ipc` exists in frames too, though a
  frame's messages don't reach the handler: checked in the real window).
- **The channel:** the page posts `{id, source, theme, background, font}`; the frame posts back
  `{id, svg}` or `{id, error}`, and `{ready}` once Mermaid has loaded. Each side checks the
  sender (`event.source`, and the page also the origin `"null"`) and the message's shape, and
  ignores anything else; the page takes an answer only to a request it has sent. One diagram at
  a time, in order. A diagram not answered in 20 s (the frame's loading included) shows as
  failed, and the frame is closed; the next diagram opens a new one.
- **The SVG comes back as data, not markup:** a tree of `{ns, name, attrs, children}` (text as
  strings). The page builds its own elements from it, so it never parses markup (a parse in the
  page runs under its CSP and reports every `style` attribute), and it trusts the tree no more
  than the document: only the SVG elements Mermaid draws with and plain HTML inside a
  `<foreignObject>` (labels), no event handlers, `href`s only to `#…`, ids only the diagram's
  own (they start with its id), and at most 100,000 nodes, 100 deep. Its `style` attributes are
  set through the CSSOM, and its `<style>` becomes a constructed stylesheet holding only rules
  whose every selector starts at the diagram's id (no `~` or `+`), and its animations. Each
  figure has `contain: paint`, so nothing in a diagram draws outside it. The SVG is then inline
  in the document, so find, selection, print and copy work on it as on any other content.
- **The page's CSP stays strict:** no inline styles, and nothing refused while diagrams are
  drawn, in the page or the frame (`scripts/doc-tests.mjs` and `doc-shots.mjs` count them).

**Tests in Edge:** `node scripts/doc-tests.mjs` runs the document's script in a headless Edge
(the same engine as WebView2), with the documents in `scripts/doc-tests/`: diagrams drawn in the
frame in both themes, a broken one, a theme change, diagrams whose text tries to run script, the
frame ignoring messages that aren't requests, a frame taken over (`hostile-frame.js`: it tries
to reach the page and sends it junk and hostile SVG), a frame that stops answering, and one that
doesn't load. Each fails on an error the page logs or anything the CSP refuses (in the frame
too, except where a test's diagrams are meant to be refused there). About 5 seconds.

**Screenshots:** `node scripts/doc-shots.mjs --out target/doc-shots/<name>` renders the fixtures
and `scripts/doc-shots/showcase.md` (with legible-core's `doc-preview` example), serves them on
127.0.0.1 under the page's CSP, and takes them in a headless Edge of its own over the DevTools
protocol: the showcase in both themes, the three widths, Sans and Serif, 100% and 150% zoom; each
fixture in both themes; a document with very long properties; and the showcase printed to PDF
(with Windows dark) and its first pages as PNGs. It fails if a page scrolls sideways, logs an
error or has anything refused by the CSP (the page's or the frame's), or if a diagram showing
isn't the theme's (the light one in print). Both scripts share `scripts/doc-harness.mjs`.

## The clipboard

Outlook, Word and OneNote keep only inline styles and simple HTML from a paste: no `<style>`,
classes or CSS variables. `legible_core::clipboard_html` turns the view's HTML into that, always
in the light theme (the light values of `web/doc/document.css`; a test keeps the two equal).

**The API** (for the window's Copy document and Copy selection):
- `build(html, images, &Limits::default()) -> Copied { html, text }`: `html` is the document's
  rendered HTML or the view's current HTML (the whole `#document`, or a selection's fragment,
  serialised); `images` reads a picture's bytes from its `src` as the page has it (the document's
  pictures are at `https://legible-doc.localhost/…`: read them through the same confinement as
  the view), returning `None` when it can't. Before calling it, replace each Mermaid diagram's
  figure with an `<img src="data:image/png;base64,…">` of the diagram; without one, a diagram is
  copied as its source.
- `cf_html(&copied.html) -> Vec<u8>`: the bytes for the clipboard's "HTML Format" (CF_HTML),
  with the `Version:0.9` header's byte offsets. Put `copied.text` on the clipboard as Unicode
  text beside it.

**What it does:** every element gets inline styles in a font stack starting with Segoe UI
(Aptos, Calibri and Arial named as fallbacks, nothing shipped); tables get their borders, fill
and padding as attributes and styles both, and zebra rows; alerts and code blocks become
one-cell tables (an alert's icon as a character, code in `<pre>` that wraps, with highlight.js's
colours inline); tick boxes become ☑ and ☐; links to the web and mail stay, links inside the
document become their text. Pictures are embedded as `data:` URIs (PNG, JPEG, GIF or WebP,
checked by their bytes) at most 640 px wide; one that can't be read is its alt text, and one
past `Limits` (5 MB a picture, 20 MB in all) its alt text with a note. A picture from the
internet that isn't shown is its alt text. Classes, ids, `data-` attributes, scripts, styles,
SVG, buttons and form controls, the view's placeholders and Copy buttons all go. The HTML is
read with html5ever's tokenizer (already in the build through ammonia).

**The plain text** reads on its own: blank lines between blocks, `•` or `1.` before list items
(nested ones indented), `> ` before quoted lines, table cells separated by tabs, code as it is,
and a link's address after its text when they differ.

**Tests:** golden copies of every rendering fixture and of a document as the view prepares it
(`tests/fixtures/clipboard/`, regenerated like the rendering's), CF_HTML offsets with non-ASCII
text, the picture limits, and that no copy keeps a style, script, class or event attribute.

## The window

**Chosen: `wry` 0.57 + `tao` 0.37** (Apache-2.0 OR MIT), driving WebView2 directly, over Tauri
2 (2.12). Both were built as the same small window on 27 Oct 2026: a 900×1000 window loading a
205 KB GitHub-flavoured document (rendered by comrak, served by a custom protocol), timed from
the process's creation (Windows' own record of it) to the window being shown, and to the
document's second animation frame. Twelve runs each, alternating, in release builds, on this PC
(an AMD Ryzen 7 9800X3D desktop with WebView2 155, so faster than the mid-range laptop the
budgets are for):

| | wry + tao | Tauri 2 | Budget |
|---|---|---|---|
| The window shown (median, slowest) | **43 ms**, 56 ms | 380 ms, 529 ms | under 300 ms |
| The 205 KB document rendered (median, slowest) | **511 ms**, 699 ms | 532 ms, 653 ms | under 700 ms |
| The first run (nearest to a cold start) | 27 ms, 481 ms | 417 ms, 554 ms | |
| Crates in the Windows build | **92** | 218 | |
| The exe | **1.5 MB** | 8.9 MB | |
| `cargo deny check` | passes | **fails**: five MPL-2.0 crates (cssparser, cssparser-macros, selectors, dtoa-short, option-ext) | |

- **wry + tao meets both budgets; Tauri 2 meets neither the window's nor the licence rule.**
  Tauri builds the window and its WebView2 together, so nothing shows until WebView2 has
  started (about 350 ms); with tao the window shows at once and WebView2 starts inside it.
- **The document's time is mostly WebView2 starting** (its browser processes and the page's
  first paint), the same either way. On a slower laptop it is the number to watch: the shell
  logs both times on every start (below), so it can be checked there.
- **Tauri's plumbing** (its IPC, its command system, its config and bundler) isn't needed: the
  shell talks to its page through wry's one message channel and its custom protocols.

**The shell** (`legible_win::shell`, on 28 Oct 2026 in a release build: the window in 40–70 ms
whatever the document's size, 2 MB included, and a small document drawn in 470–650 ms, both
logged at every start as "the first window is shown" and "the first document is drawn", in ms
since the process was created; "Performance", below, has the full record):

- **One window per file, one process** (`instance`): the first Legible to start claims a named
  pipe, `\\.\pipe\Legible-<user's SID>-<session>`, whose security descriptor lets only that
  user open it. A later launch (a double-click in Explorer) checks the pipe's server is the same
  user's process, lets it take the foreground, sends its files (UTF-16, so any path survives)
  and exits once they're taken. The first opens each in a new window on its one event loop, or
  brings the file's window forward if it's open (files are compared by canonical path). If the
  first has just exited or doesn't answer in 5 seconds, the launch opens its own windows, so a
  file always opens. The command line is `legible [FILE]...`; relative paths are made absolute
  before they're handed over.
- **A window first, then its file:** a window is made, and WebView2 started in it, before its
  file is read. A thread of its own reads and renders the file meanwhile (a 2 MB document's
  render takes about 0.4 s, mostly the sanitiser, about as long as WebView2 takes to start), and
  the page is served once it's done. Read first, as it once was, a 2 MB document's window took
  about 0.5 s to appear; now it's 47 ms (42–71 over six runs), and a 205 KB document's 43–46 ms
  rather than 50–82. Live reload's snapshot of the file is taken before the read, so a change in
  between is its first report; the file joins the recent files once it's read; a window closed
  before then just closes; and a file that can't be read (another program holding it) closes
  its window and says why. Till then its menu bar offers what needs no document (Open, Close,
  Exit, the settings), as the empty window's does, and Save a copy and Save as PDF do nothing.
- **Open replaces the window's document; a launch opens a window** (`Shell::open_here`, and
  `Shell::open` for the rest). Open (Ctrl+O), Open recent and a drop show the file in the
  window they came from; a launch (a double-click, the Jump List, a later launch's hand-off), a
  link and a copy just saved open a window of their own. A file open in another window brings
  that one forward instead, and the window asked stays as it is. The replace goes through the
  window's own closing (`saving::Session`), so unsaved changes ask the close question, by the
  same path: a close that would close the window (`Shell::closed`) replaces its document
  instead, and Cancel, or a save that fails or conflicts, keeps it; a close asked for meanwhile
  wins. The old document goes as a closed window's does (`Shell::retire`): its copy kept for the
  session's end is removed, live reload stops watching it, Save as PDF's work for it stops, and
  its reading place is kept. The window then opens on the new file as a new window does (read
  on a thread of its own, its page served once that's done, at generation 0, so nothing of the
  old page's place or zoom is carried over), with everything per document new: the bars, the
  outline, the status bar, the pictures from the internet (asked about again, the filter letting
  nothing through meanwhile), the document protocol's folder, the title, and recent files and
  the Jump List. The reader's mode stays (Split or Edit is asked of the new page once it's
  drawn), unless the new file can't be edited. A new file that can't be read leaves the window
  open with no document, and says why. Each page a window loads is numbered in its URL
  (`/?page=<n>`), and the window keeps the number of its document's first page. The old
  document's pages are left behind the moment the replace starts, though the last shows, frozen,
  till the new one is served: nothing they say reaches the new document (a pictures answer, an
  edit, a link), whether the message handler hears it then or the event loop takes it up after
  (each event carries its page's number). Only the page loaded last says it's drawn or where the
  reader is, so a page served again's late word doesn't count either. Each read of a file is
  numbered too, so a late read for an earlier opening, of the same file or another, is out of
  date. A later launch with no file (the shortcut, with Legible running) opens an empty window
  of its own.
- **WebView2:** the Evergreen runtime is checked for at start (before the pipe, so a Legible
  that can't show anything never holds it). If it's missing, a message says so and offers to
  open Microsoft's page in the browser; Legible never downloads anything. WebView2 keeps its
  data in `WebView2\` in Legible's own folder (`paths`), never beside the exe; a folder that
  can't be written to (a read-only portable copy) sends it to `%TEMP%\Legible-WebView2`, logged.
- **Release builds** turn off the developer tools (which takes "Inspect" off the context menu)
  and WebView2's own shortcut keys (reload, devtools, find, print, and the rest); debug builds
  keep them.
- **Two origins, by custom protocol** (`page`): wry serves a protocol `name` at
  `https://name.localhost/` on Windows (the https form, so the page is a secure context).
  `https://legible.localhost/` is Legible's own: the page at `/`, the embedded assets at
  `/doc/…`, `/chrome/…`, `/js/…`. `https://legible-doc.localhost/` serves the document's images,
  and nothing else.
- **The page's CSP**, in a `<meta>` before anything loads and in the response header:
  `default-src 'none'`, scripts and styles only from `https://legible.localhost`, images only
  from `https://legible-doc.localhost` and `data:`, one frame (Mermaid's,
  `https://legible.localhost/doc/mermaid-frame.html`), no `<base>`, no forms, and no page may
  frame it. **Decided (27 Oct 2026): it stays strict**, with no inline styles: Mermaid draws in
  that sandboxed frame, under a CSP of its own (`page::MERMAID_FRAME_CSP`, sent as the frame
  page's header), and the page builds the diagrams from what it sends ("The document's look").
  Its one widening: `img-src` gains the `https://host[:port]` origins of the pictures from the
  internet the reader has let in (`page::csp`; "Pictures from the internet", below). The page
  is served by a window's own protocol handler, with the CSP it was made with as its header.
- **The document's images** (`confine`): a request's path is percent-decoded and refused before
  the disk is touched if it has `..`, `.`, empty parts (`//server`), `\`, `:` or a null. Then the
  file is canonicalised, which follows symlinks and junctions to where it really is, and served
  only if that's still inside the document's folder (canonicalised the same way, so a document
  on a share is held to its folder there too), and only for image types.
- **Links never navigate the page** (`links`, and `web/chrome/links.js`, run before the page's
  own scripts): every click on a link sends its `href` as the document wrote it, and the shell
  decides against the document's real path: `#…` scrolls; `https:` opens in the default browser;
  `http:` asks first; `mailto:` opens the mail app; a `.md`, `.markdown` or `.mdown` file that's
  there (relative, by its drive path, or `file:`) opens in a Legible window; anything else does
  nothing (logged). As a second line, every navigation but Legible's own page is refused, and so
  is every new window (`window.open`, `target=_blank`).
- **Window placement** (`placement`): the first window is 900×1000
  (logical pixels), within 85% of the primary monitor's work area, and Windows places it. After
  that, each new window opens at the size, position and maximised state of the last one closed,
  kept in `window.toml` in Legible's folder (Settings' "Open windows at their last size and
  position", on by default; off, each opens at the first size, placed by Windows). Its position
  goes back on the monitor it was on (the one it overlaps most, by `GetMonitorInfoW`'s work
  areas), moved fully onto that monitor's work area; if that monitor has gone, or the position
  is off every screen, the window is centred on the primary. That's worked out from the
  remembered inner size; once the window is made, its whole outer rectangle (frame, title and
  menu bar) is fitted again (`placement::onto_monitor`): made smaller by what's too big for the
  work area, and moved fully onto it. The title is "<file name> –
  Legible".
- **The reading place** (`places`): a file opens where the reader was in it (Settings'
  "Remember where I was in each file", on by default). As a window closes, its page is asked, in one
  script, to take no more edits (frozen, as a close's save freezes it), whether anything is
  unsaved, and where the reader is that moment (`LegibleChrome.place()`); it closes with the
  answer, or after a second without one (then by its own account of the edits). So a close
  straight after scrolling keeps the newest place, and an edit that came after the close was
  decided asks the close question rather than being lost (Don't save, chosen, still closes).
  In View the page also sends its anchor as each scroll lands, the same one only once, for an
  end that never closes the window (`place\n<fraction>\n<offset>\n<heading>`: the heading nearest
  the top, as live reload keeps the place, never a pixel offset alone); the window keeps it,
  and as it closes the file's place is kept in `places.txt` in Legible's folder, for the 100
  files closed most recently, by full path. Files that have gone leave the list as Legible
  starts (found off the UI thread). Once the file's page is first drawn, the window sends it
  back (`LegibleChrome.goTo`). Turning the setting off forgets every place, the file too.
- **The manifest and the icon** (`crates/legible/build.rs`): the exe's manifest asks for common
  controls 6 (themed message boxes), `asInvoker`, Windows 10 and 11, per-monitor v2 DPI
  awareness (tao asks for the same at run time; its call then fails harmlessly) and UTF-8 as the
  ANSI code page. The app icon is resource 1, so Explorer shows it, and each window loads it from
  there for its title bar (16 px) and the taskbar and Alt+Tab (32 px), scaled by its own
  monitor's DPI, and loads them again when it moves to a monitor with another scaling
  (`ScaleFactorChanged`), so Windows never resamples the wrong size.
  `crates/legible/tests/resources.rs` checks they're all in the exe; `cargo run -p legible
  --example icon-dpi` (the exe's resources are linked into the examples too) tells a window of
  its own its DPI changed and checks both icons follow.
- **The real window in tests** (`shell::harness`: a build-time Cargo
  feature, `harness`, never an option in the shipped product's `Options`): legible-win's tests
  turn it on through the crate's dev-dependency on itself, so only test builds have it, and
  `shell::run`, which the exe runs, never takes a harness. `harness::run` runs the shell's own
  event loop in the test's process (on any thread, and returning once done) with a stand-in
  (`StandIn`) for what a test mustn't touch or can't answer: no app identity and no Jump List
  (both are the user's, by the app's ID), no single-instance pipe, quiet windows (`quiet`:
  off every monitor and never activated, so a test run leaves the user's focus alone;
  docs/TESTING.md, Rules every test keeps), a clipboard of its own, the close question's and Save a copy's answers given, message
  boxes recorded. A driver on a thread of its own runs script in each window's own page (never
  input), reads the window's state, holds a page's message back, and acts in the moments of a
  write (`Write` and `Moment`: another app writing the file between a save's check and its replace,
  say). `crates/legible-win/build.rs` compiles legible.exe's manifest into the crate's test and example exes,
  as the close question's task dialog needs common controls 6. `tests/shell.rs` runs the shell
  in a copy of itself whose temp folder is one of the run's own: its documents are in the temp
  folder, where Legible won't edit a file.
- **The page** (`page`, `web/chrome/window.html`): the document rendered by
  `legible_core::render` (images served from its folder), with the document's look loaded as
  "The document's look" says, and the view's choices as classes on `<html>` (`page::View`;
  the defaults until Settings set them). `web/chrome/window.css` holds only what the window
  adds.
- **Live reload** (`watch`, `reload`): **one** service thread for the whole process polls
  every open document, and the pictures it uses, every 250 ms (creation time, last write and
  size, so a file replaced by a rename is seen as new). A change is acted on once two polls
  agree (a debounce of about half a second); the document missing for a second is deleted, and
  missing for less (a replace by rename) is a change. The changed document is read and rendered
  on that thread, never the UI's. Pictures are watched by where they'd be
  (`DocRoot::candidate`, the protocol's own path rules), present or not, and only while they
  resolve inside the document's folder; one appearing, changing or going re-renders the
  document, and is never a deletion. **Bounded:** never more than one watcher thread; opening
  and closing windows only take a lock that is never held during file I/O, so a call stuck on a
  network share stalls live reload until it answers but never blocks closing or piles up
  threads (a cycle over 5 s is logged). A read that fails (half-written, locked) is tried again
  on the next agreeing polls, and nothing on screen is wiped.
  - **In the page:** `LegibleChrome.replaceDocument({generation, html})`. An older generation
    than the newest started is ignored, and one still preparing when a newer starts is left to
    finish and dropped. It awaits `LegibleDoc.prepare(doc)`, then rebuilds the outline and
    Find, and keeps the reader's place: the heading nearest the top of the page under the
    chrome, or the same share of the page if that heading has gone. "Updated just now" shows
    for 4 seconds. Pictures on the document's origin are rewritten to
    `/~<generation>/<path>` (the protocol takes that part off, once, before its checks), so a
    changed picture is fetched afresh. A deleted document shows a bar, "This file was moved or
    deleted", with "Keep showing it" and Close; if the file comes back, the bar goes.
  - **Unsaved edits** (the hook for safe saving, `reload::State`): the editor sends
    `dirty\n<true|false>` (which also puts the dot in the title). While dirty, a change
    **and** a deletion are both held: the edits stay on screen, the newest is kept as pending
    with a rising id, and `legible:external-change` is fired on `window` with `detail {kind:
    "changed" | "deleted", id}`. Clearing dirty applies nothing, and later changes stay
    pending, until the prompt answers for the current id: `LegibleChrome.reloadFromDisk(id)`
    (Use the file, which discards the edits) or `LegibleChrome.keepMine(id)`. An answer for
    an older id is ignored.
  - **With the editor open,** what changes on disk (applied, or taken with Use the file) goes
    to the editor rather than the page: `LegibleEditor.fromDisk(text)` makes it the text and
    what's saved, and the editor renders it for its own preview, which then shows "Updated just
    now". So the document has one writer at a time, and its source lines stay right for scroll
    sync. Until the editor opens, the window keeps the file's newest text for it.
- **The chrome** (`web/chrome/chrome.js`, `window.css`): a fixed 40 px toolbar (Open; View,
  Split and Edit; Outline, Find, Zoom, Reading width and style, Print, Copy, and a More menu with
  Settings and About), a find bar under it, and the outline panel on the left. The document
  stays a direct child of `<body>` and the page itself scrolls, so the print stylesheet's rule
  (`body > :not(#document)` hidden) hides all the chrome, and `window.css` resets the zoom and
  the chrome's padding in print. Everything is built with `createElement` and `textContent`,
  and what moves with the layout is set through the CSSOM, so the CSP needs nothing new. Every
  button has a name and a tooltip with its shortcut; the menus follow the WAI-ARIA menu-button
  pattern, and Settings and About are modal `<dialog>`s. Settings follows the WAI-ARIA tabs
  pattern: Appearance, Reading and General, one tab stop with the arrow keys, Home and End
  between them, the tab last shown kept in `sessionStorage` for the window's session. Its
  panels share one grid cell (a hidden one keeps its place, unseen), so the dialog keeps one
  height and one column of names whichever tab is shown.
  - **What moves with the layout** never goes on `<html>` as it changes: a custom property
    there is inherited by every element of the document, so each write restyled all of it (a
    second for 2 MB). The bars' heights go on the elements that follow them: plain values on
    the body (its padding), the page (`scroll-padding-top`, so a heading jumped to rests under
    the chrome by its own 16 px margin), the document in Split and the outline (its list is as
    long as the document's headings); `--chrome-top`, `--status-height` and the bars' heights
    on the chrome's other elements, the children of `<body>`. `--zoom`, `--outline-width` and
    the editor's `--split` stay on `<html>`, written only when they change. A layout pass
    measures everything, then writes.
  - **A long document is drawn once its first screen is:** the blocks Legible renders
    (paragraphs, lists, quotes, code, tables, alerts, diagrams, the properties and the
    footnotes) have `content-visibility: auto` on screen, so a block off the screen isn't laid
    out or painted until it comes near, and keeps its height once shown. Not the headings (the
    outline, its jumps and the reader's place are measured by them), nor a document's own HTML
    boxes, whose margins could reach out of them. The look is unchanged: the margins that used
    to reach out of a contained block (a loose list's last paragraph, a diagram's source) are
    none inside it, and `overflow-clip-margin` keeps a focused link's ring whole. Off the
    screen, a block's height is a guess until it's shown, so a jump far down (the outline, a
    place put back, find's match) is held on its target each frame until it stays (a second at
    most; the reader's own scrolling, keys or pointer end it). The outline's current section
    and the reader's place find their heading by a binary search (the headings' tops only go
    down the page), taking a heading inside a waiting block as where that block starts. The
    outline's entries wait the same way, for a document with thousands of headings.
  - **A document of thousands of blocks is laid out once it's drawn:** the browser looks at
    every waiting block on every frame (is it near the screen yet?), so with 50,000 of them (a
    2 MB document) a step of the scroll waited 0.2 s. A document of 2,000 blocks or more has
    them laid out in the browser's idle time once it's drawn, from the top, in batches of about
    0.3 s (smaller batches took four times as long in all; all at once held the page for most
    of a second), each block marked `data-laid-out`, which `window.css` lets be (copying drops
    `data-` attributes). A block of more than 20,000 elements stays waiting, as it would hold
    the page longer than a batch. A smaller document's blocks stay waiting: there the cost is a
    few milliseconds a frame.
  - **Long documents, measured** (release build, portable copy, this PC, drawn as the shell
    logs it), with two kinds of test document. *Nested*: the showcase, then the GFM, alerts,
    headings and front matter fixtures repeated under "Part" headings; the fixtures define the
    same footnote label in every part, and until the renderer kept a label's first definition
    only (Rendering, step 3), a label defined twice pulled what followed it into the footnotes,
    so nearly all of it was one block; the figures for it below are from then. *Flat*: the same
    with each part's labels its own, the shape of a real long document: about 5,000 blocks in
    205 KB and 50,000 in 2 MB.
    - On 29 Oct 2026, with the machine quiet (six runs, medians): the nested 205 KB document is
      drawn in 525 ms (910 ms before blocks waited), its first find in 25 ms (307); the nested
      2 MB one in 1.05 s (5.2 s), its first find in 0.17 s (3 s); a 200 KB report of prose in
      470 ms (510).
    - **Measured under load** later that night (another workload using most of the CPU, so to
      be measured again on a quiet machine): the same session, builds interleaved, medians of
      four runs, against the build before the background layout. The flat 2 MB document is
      drawn in 3.3 s either way (8.0 s before blocks waited). Its first jump to the end
      straight after opening takes 0.8 s (1.05 s); once laid out, about 4 s after it's drawn,
      0.17 s (0.47 s); a small step of the scroll 6–7 ms (0.2 s). The flat 205 KB document is
      laid out within 0.4 s of being drawn, and a step of its scroll takes 6 ms (20 ms). The
      nested 2 MB document's first jump to the end takes 1.6 s either way, laying out its one
      block of 174,000 elements; the nested 205 KB one is drawn in 0.74 s (0.77 s).
  - **Layers:** the toolbar strip (and the find bar on it) is a layer of its own under the menu
    bar: a step lighter than the bar in dark, a touch whiter than the page's surfaces in light,
    with a hairline under it (`--toolbar-bg`, `--toolbar-rule`). The
    bars over the document look like Windows' InfoBar: information and choices (pictures from
    the internet, the default app) on a tint of the accent (`--info-bg`), warnings about the
    reader's work (the moved-or-deleted bar, the editor's conflict bar) in amber only, each with
    a 5 px stripe and its icon leading in the stripe's colour (an "i", a triangle, and an
    octagon for the red save-failure notice). Their colours are tokens in `window.css` that the
    contrast test reads.
  - **The status bar** (`status`, `shell::status_bar`): off by default,
    the `status-bar` setting, from View → Status bar (ticked as it is) or Settings. A thin strip
    at the bottom on the toolbar's layer; the page, the outline, Split's panes and the toasts end
    above it (`--status-height`). Left: the file's path as Windows shows it, its folders giving
    way in the middle when it doesn't fit (never its name), the whole of it in the tooltip; a
    click asks the window to copy its own file's path (`copy-path`, as text, the newest copy of
    the process's) and "Path copied" shows. After it, why a file opened for reading only, if it
    came from Outlook's attachments, the browser's cache or the temp folder (`status::Origin`).
    Right, in Split and Edit only: the text's words (counted in the page a moment after typing
    stops, by `Intl.Segmenter`: front matter, comments, tags and addresses aside, a link as its
    text, Markdown's marks never words), the encoding and the line endings ("Mixed (CRLF kept)"
    for a file that mixed them: what saving writes). The page asks for the facts as it starts
    (`status`, answered `LegibleChrome.status`), and is told again when the file changes on disk.
  - **Select all takes the document alone** (Ctrl+A, Edit → Select all): the toolbar, the find
    bar, the outline, the bars, the toast and the notices are `user-select: none`; the find field,
    Settings and About (its reading view) stay selectable. In the editor, Ctrl+A is CodeMirror's own.
  - **Outline:** the document's headings, nested; the current section marked as it scrolls;
    Enter or a click scrolls smoothly to the heading and focuses it. Shown by itself (Settings:
    Automatic) for more than 3 headings and more than two screens; Ctrl+Shift+O toggles it;
    160–480 px wide by its handle (pointer or arrow keys).
  - **Find** (Ctrl+F, Enter and Shift+Enter, F3, Esc): in the page, not WebView2's own. The
    document's shown text (no script, buttons, SVG or hidden copy of a diagram) is joined into
    one index that maps back to its text nodes, with a break between blocks, so a match can run
    across inline elements but never from one paragraph into the next. The index goes by the
    markup, never the computed style (which would lay out every block still waiting): what's
    hidden by its tag or attributes, and the one thing `document.css` hides (a picture's
    placeholder once the picture shows); blocks by their tag, and the children of
    `document.css`'s flex and grid boxes. It's built on the first search, 10 ms for a heavy
    205 KB document and 0.2 s for 2 MB. Matches are CSS Custom Highlights (no change to the
    DOM); the current one opens any `<details>` round it and is scrolled clear of the chrome.
    The index follows the document as it changes.
  - **Zoom** (Ctrl +/−/0, Ctrl+wheel, the toolbar): the document only, 50–300% in steps, per
    window for the session, starting from Settings' default. WebView2's own zoom is off. Ctrl+wheel
    is a step a notch, and a touchpad's pinch (which WebView2 sends as Ctrl+wheel, in small
    movements) a step for each notch's worth; Settings' "Zoom with Ctrl + mouse wheel" (on by
    default) turns it off, leaving no wheel zoom at all.
  - **Messages** to the window, one or two lines each: `open` (Windows' Open dialog), `mode`
    (the editor does the work; the menu bar shows it), `outline` (the Outline button pressed or
    not, for the menu bar), `close` (Ctrl+W, and the moved-or-deleted bar's Close),
    `setting <key> <value>` (checked against the known
    keys and values, then saved and sent to every window as `LegibleChrome.apply(...)`; a new
    default zoom changes no open window's zoom), `about`, `about-copy` and `reading` (About,
    below), `recent` and `open-recent` (Recent files, below), and live reload's and the
    editor's (above and below). A save that fails, or a folder that can't be written to, is
    said in Settings (`LegibleChrome.saveFailed`).
  - **About Legible** (`about`, `shell::about`; Help → About Legible or
    the ⋯ menu): the icon (Legible's own SVG, as a
    `data:` picture the CSP lets in) over the name, the tagline, a table, the licence line, the
    privacy line, the copyright and independence notice (one Fluent message, word for word as
    the README's), and Copy, Release notes and OK (focused as it opens; Esc closes).
    - **The table is the window's facts** (`about::Facts`), asked for as About opens (`about`,
      answered `LegibleChrome.about({rows})`): the version (Cargo's), the build (Installed, or
      Portable as `AppPaths` says), the WebView2 runtime's version, and Windows (its release,
      build and revision from the registry, `win32::registry::windows_version`).
    - **Copy** asks the window (`about-copy`), which puts "Legible <version>" and the other
      rows on the clipboard as text, its own facts never the page's, as the newest copy of the
      process's, and About says "Version information copied".
    - **The reading view** (a dialog over About; Esc or Back goes back to it): the licence
      line's links open the MIT License (`LICENSE`) and the third-party notices
      (`THIRD-PARTY-NOTICES.txt`), and Release notes `CHANGELOG.md`, each bundled in the exe
      (`about::Reading`, `reading <name>`, answered `LegibleChrome.reading(name, text, html)`,
      shown only if it's still the one asked for). The licence and the notices read as plain
      text; the release notes are rendered by `legible_core::render`, as a document is, and
      shown in a document's look (`.reading-doc`), their links going to the window as any
      link's: in a window with no document too, where `links::classify_without_document`
      opens a web or mail link as anywhere and a path or `file:` link, with nothing to place
      it by, does nothing. The dialog fits the window, and only its text scrolls.
    - **Privacy:** "No telemetry", and that Legible goes online only for pictures the reader
      allows.
  - **Recent files** (`recent`): the ten files opened last, newest first, by canonical path;
    every open that works adds one (a double-click, a later launch, Ctrl+O, a link, a drop, a
    recent file). Kept in `recent.txt` in Legible's folder (written by temp-and-rename, a path
    that isn't valid Unicode kept exactly), or for the session only if the folder can't be
    written to. Files gone are found off the UI thread by one worker that takes only the newest
    list (never a queue, so a share that hangs can't pile them up); only a file certainly not
    found is dropped, never one that's only unreachable for now, and an answer about an older
    list is ignored. The More menu shows them by name, rebuilt each time it opens, from
    `LegibleChrome.setRecent({token, items})`; files of the same name show, faintly, as many of
    their folders as tell them apart, and the full path is each one's tooltip. A pick sends
    `open-recent` with the token and the index, and one from a list that has changed since
    opens nothing.
  - **The Jump List** (`jumplist`): **Legible's own list, not Windows' automatic
    Recent**, so it works the same installed or portable. Each process takes the AppUserModelID `Surmise.Legible` before any window, and the recent files are published
    as its custom "Recent files" category (`ICustomDestinationList`), links to Legible's exe
    with the file as its quoted argument, each time the list changes. A file the user removes
    from the Jump List leaves the recent files too, matched by its own argument. A list that
    can't be built is given up (the old one stays), and the failure logged once. Windows'
    own limits on what it shows still apply.
  - **Drag and drop:** a file dropped on a window is handled by Legible, never shown by
    WebView2. The first Markdown file of the drop replaces the window's document, as Open does,
    and the others open in windows of their own; a drop with nothing Markdown says what Legible
    opens.
  - **Open in editor and Show in folder** (`actions`, More menu): for the window's own file, as
    the shell has it (never a path from the page); a window with no document (`no-document` on
    `<html>`, `page::empty_page`) shows both disabled. **The editor** is `.md`'s association's
    `edit` handler, else its `open` handler (the user's default), else Notepad from System32;
    a handler that is Legible (this exe, or any `legible.exe`) is skipped. Both questions go to
    one `IQueryAssociations` for `.md`, and the launch (`ShellExecuteExW` with
    `SEE_MASK_CLASSKEY`) uses that association's class key, so a `.markdown` or `.mdown` file
    opens with the same `.md` editor, never with Legible through its own association. The path
    is only ever `lpFile`, or Notepad's one argument: Legible builds no command line. Both
    actions give the shell the path without the `\\?\` prefix of Legible's canonical paths
    (`actions::shell_path`), which the shell's parsing refuses; a path past 260 characters may
    still be refused by the shell or the editor. **Show in
    folder** gives Explorer the file's full ID list with no children, which opens its folder
    with it selected; a file that has gone opens its folder itself. Both run on one `actions`
    thread for the process (COM single-threaded) with a queue of four, so a share that hangs
    stalls only them, and a click past a full queue is dropped (logged). A failure is said in
    the window, through Fluent. The tests run all of it against a recording fake of the shell
    (`actions::Platform`): nothing is launched by a test.
  - **Getting it out: Copy, Print and Save as PDF** (`clipboard`, `export`,
    `win32::clipboard`, `win32::pdf`).
    - **Copy document** (Ctrl+Shift+C, the toolbar's Copy) and **Copy selection** (Ctrl+C, only
      for a selection in the document; anywhere else it's the ordinary copy). The page sends the
      HTML it shows (a selection touching a diagram brings the whole diagram), each diagram
      turned into its **light** drawing as a PNG at twice its size: the drawing is in the page
      in both themes, so the copy takes the light one, with Mermaid's own rules for it (scoped
      to its id) inside it, and the visible page never changes.
    - The shell numbers each copy in **one order for the process** (the clipboard is everyone's:
      a later copy in any window wins), with the window's document folder as it was then, and
      builds it on one thread with `legible_core::clipboard_html` (inline styles, pictures embedded):
      a document picture is read through that folder as the window serves it (confined, size
      capped); a picture from the internet is never fetched, and pastes as its alt text. A copy
      waiting is replaced by a newer one; every copy gets one answer (copied, as text only,
      failed, superseded), and only the newest is ever written. A copy is named by its number
      and the render it was made from (`clipboard::Ticket`), so an answer for a page served
      before is never taken for a newer page's copy of the same number.
    - **Writing**, on the UI thread, owned by the window: both formats ("HTML Format", CF_HTML
      with its offsets, and CF_UNICODETEXT with CRLF) prepared before the clipboard is emptied,
      blocks it doesn't take freed, closed whatever happens; a copy with nothing in it never
      empties it. Another app holding the clipboard is tried again 20 times, 50 ms apart,
      without holding the window up. A toast says "Copied, ready to paste into Outlook, Word or
      OneNote".
    - **Print** (Ctrl+P): WebView2's print UI with the document's print stylesheet, and `@page` margin
      boxes from the chrome (a constructed stylesheet; the CSP is as strict as ever): the page's
      title at the top of each page, Fluent's "Page { $page } of { $pages }" at the bottom (the
      translation's order kept, its text CSS-escaped).
    - **Save as PDF** (the More menu): one job at a time per window, each numbered. Its folder
      is found off the UI thread (the document's, if a file can be made there, else Documents);
      the Save dialog suggests `<name>.pdf`; WebView2's `PrintToPdf` (A4, or Letter where the
      locale measures in inches; backgrounds on; WebView2's own header and footer off, as the
      margin boxes do it) prints to a staging file beside the one chosen, which replaces it
      only once the print worked. One that fails, or takes over two minutes, leaves an existing
      PDF as it was and removes only its own staging file. The jobs (`export::Jobs`) belong to
      the process, not the window: a print that ran out of time never commits when it ends
      late; once the staging file is replacing the chosen one, nothing calls it off and no other
      save of that window starts until it's done; a window closed mid-print (or after its print
      ran out of time) ends the job and has its staging file removed, retried while it's held.
      With the last window closed, Legible exits only once that work is settled (a commit
      landed, each removal reported done), waiting off the UI thread for a minute at most.
      "Saved <name>" offers Open and Show in folder, for the path the shell kept (never one
      from the page).
    - **Checks:** the writer's every step and failure against a recording clipboard; the real
      clipboard only by hand (`tests/clipboard.rs`, ignored: it overwrites what's on it);
      `cargo run -p legible-win --example pdf-check` prints the showcase through the production
      `PrintToPdf` and checks the PDF.
  - **Pictures from the internet** (`consent`, `win32::request_filter`): **only with the
    reader's say-so, and Legible has no network code of its own:** WebView2
    fetches them, held to it two ways.
    - **What's asked for** is each `<img data-remote-src>` in Rust's own render (never an
      address from the page). Only `https:` pictures on a host a CSP can name are offered;
      `http:` ones, and IPv6 hosts, stay placeholders.
    - **What may load** (`consent::Pictures`, per window): Settings' Never, Always or Ask;
      under Ask, "Always for this file" (canonical paths in `remote-images.txt` in Legible's
      folder, the format of `recent.txt`, at most 500) or Show once, for that window. **Consent
      is per host:** Show once lets in the hosts of that render's
      pictures, so a live reload's new picture on one of them shows, and one on a new host asks
      again, about it alone. The
      bar ("This document shows pictures from the internet. Show them?", Show once, Always for
      this file, Close) answers with its render's generation; an answer for an older one does
      nothing.
    - **The first guard, the page's CSP:** on consent the window serves its page again (the
      reader's place, zoom and outline kept, as live reload keeps them) with `img-src` naming
      the consented hosts' `https://host[:port]` origins and no others. WebView2 then refuses a
      picture, or a redirect, to any other host, port or scheme (HTTPS to HTTP included) before
      a request exists.
    - **The second, WebView2's request filter:** on every request of every source (frames and
      workers too, `ICoreWebView2_22`), put on each window's WebView2 before its page loads.
      Legible's own two origins pass (matched exactly: scheme, host, port); anything else goes
      ahead only if it's an image, by GET, over HTTPS, to a consented host, and is otherwise
      answered with the window's 403 (made once, at install). A redirect within a consented
      host is within the consent (WebView2 can't tell a redirect's next request from a first
      one in any case).
    - **The page asks only for the document's pictures:** it sets an `<img>`'s `src` only to an
      address the window lists from its render, for the newest generation; so the first
      requests are exactly the document's (the native check proves it, below). The list is the
      page's exact approved set: a picture no longer on it loses its `src`, and one typed new
      in the editor shows nothing (and asks for nothing) until it's on it, whatever its host.
    - **Taking it back:** Never (or a grant gone) empties the filter's set at once, then serves
      the page again with the strict `img-src`. A request already sent can't be recalled; what
      it brings lands on a page that's gone. A request the filter couldn't refuse (`failed`)
      takes every picture's `src` away in the page at once, whatever generation it's said for,
      and on that page none gets one again: the reload that tightens the CSP comes after.
    - **Never while there are unsaved edits:** a page served again would
      lose the editor's unsaved text, so every serve waits while the editor is dirty
      (`editor::ReloadGate`, the newest origins winning) and goes ahead once the edits are
      saved or undone, as live reload waits for them. Meanwhile the bar says "Save your changes
      to show pictures from the internet", with nothing to answer, instead of asking.
    - **And never any navigation while there are:** the WebView's own navigation handler
      (`shell::navigation_policy`, the one boundary every load of the page crosses: the window
      serving it again, a reload key, the page itself) refuses Legible's page too while the
      editor is dirty (`editor::NavigationGuard`). The guard is set from the page's `dirty`
      message the moment it arrives, before the window's event loop takes it, so an edit just
      before a navigation still stops it. A serve takes effect (its pictures' CSP, the editor
      closed) only once its navigation is allowed; one refused because the editor became dirty
      meanwhile is put back as it was, and waits for a save like any other.
    - **A page served again** is the same document as before: the window keeps the
      moved-or-deleted bar's state (`reload::FileBar`: shown, or Keep showing it chosen), and
      a new page asks for it as it starts, so the bar shows as it did until the file is back.
    - **Failing small, never closing:** no filter (an older WebView2, or installing it failed)
      leaves the strict `img-src` and says so in the bar; a request the filter couldn't refuse
      turns this document's pictures off for good (until it's opened again), serves the page
      again strict, and says why. Nothing closes and nothing exits. The request that couldn't
      be refused may have gone ahead: it could only be on an origin the reader let in.
    - **The native check** (`crates/legible-win/checks/remote-images`, a workspace of its own,
      so its test-only TLS (rustls, rcgen) stays out of Legible's build and lock): the page,
      its filter and `consent::Pictures` in a real WebView2, against HTTPS and HTTP servers on
      127.0.0.1 that log every request; its WebView2 accepts that run's certificate alone. It
      checks: nothing loads before consent; after it, every picture's first request is exactly
      a listed address (an encoded query included), and every `src` the page sets is one; a
      redirect within the host loads, and one to another host, port or to HTTP never reaches a
      server; a fetch, frame or stylesheet is refused; live reload (a new picture on a host let
      in shows, one on a new host asks); stale answers and messages do nothing; Always for this
      file; Never with a picture on its way; Always; and failing small, the window alive. The legible-win `checks` feature, which
      only it turns on, lets a refusal be made to fail.
  - **Tests:** `node scripts/chrome-tests.mjs` runs the window's real page
    (`examples/chrome-page.rs`) in a headless Edge under its own CSP: names and tooltips, the
    outline, find, zoom, menus and dialogs from the keyboard, and print. `cargo run -p
    legible-win --example ui-shots` takes screenshots in a real WebView2 (CapturePreview) into
    `target/ui-shots/reading/`, at 100% and 150% scaling (other sets, each named on the
    command line: `reload` live reload, `recent` recent files,
    `editor` the editor, `pictures` the pictures bar, `divider` Split's divider idle, hovered
    and dragged, the pointer sent to its own WebView through DevTools, `status-bar` the status
    bar off, on in View and on in Edit, and `export`, `saving`, `restore`, `comfort`,
    `default-app` and `about`; the example's doc comment lists them all). `cargo run -p
    legible-win --example readme-shots` takes the README's pictures of the real window, its title
    bar and menu bar included, in View and Split, light and dark: the shell run through
    `shell::harness` on `docs/screenshots/project-update.md`, each window copied with
    `PrintWindow` from its own process.
- **The menu bar** (`menu`, `shell::menus`, `win32::menu`): File, Edit,
  View, Tools and Help above the toolbar, which stays as it is (the ⋯ menu too). **Windows' own
  menus, through `muda` 0.20** (MIT or Apache-2.0), not an imitation in the page, so Alt, the
  access keys and screen readers work as in any Windows app. Its strings are Fluent's
  (`menu-*`); within each menu every item has its own access key, which a test checks.
  - **One code path:** every item does what its toolbar button or shortcut does
    (`menu::Command::route`). Open, Save, Save a copy…, Save as PDF… and Close are the page's
    own messages to the window (`open`, `save`, `save-copy`, `save-pdf`, `close`), which the
    shell takes as if the page had sent them; a file in Open recent (the ⋯ menu's list: its
    name, then its folder, shortened in the middle past 40 characters) is `open-recent` with
    the list's token. Print, Find, Replace, Copy document,
    the modes, Outline, the zoom, the reading width, the theme, Settings and About call the
    page's own function for each (`LegibleChrome.command`). Find, as Ctrl+F, is the editor's in
    Edit, or in Split with the text focused, and the page's otherwise. Undo, Redo, Cut, Copy,
    Paste and Select all are their shortcuts' keys, sent to the window's own WebView through
    its DevTools protocol (`Input.dispatchKeyEvent`): the page, CodeMirror and the browser's
    editing take them as they take the keyboard, so Copy in the document is Legible's copy and
    Paste is allowed. Nothing goes through Windows' input.
  - **Shortcuts:** each item shows its keys (Fluent's names for them: "Ctrl+Shift+S",
    "Ctrl+Plus"), but the menus register none, so every key is the page's, as it was without
    the bar. While a menu is open it has the keyboard, as in every app with Windows' own menus:
    the arrows, Enter, the access keys and Esc are the menu's, and the shortcuts work again once
    it closes. Ctrl+W, new with it, closes the window. Ctrl+E goes round the modes, so it's on
    none of the three mode items.
  - **States** (`menu::State`, set after every event, and only where they changed): an item
    that doesn't apply is disabled, never hidden. Undo, Redo, Cut, Paste and Replace need Split
    or Edit; View's Status bar is ticked as the setting is; Save needs unsaved edits (in any
    mode, as Ctrl+S: edits made in Split are still
    unsaved back in View) and no save under way (nor a close's own); a file that
    can't be edited where it is offers View and Save a copy…; the empty window offers Open,
    Close, Exit, the width, the theme, Settings and About. The mode, Outline (only in View, as
    its button), the reading width and the theme are ticked as they are: the page says the
    mode (`mode`) and when the Outline button's pressed state changes (`outline
    <true|false>`), and a page served again starts in View.
  - **Exit** (`menu::Exit`): every window in turn, the one it was picked in first, each brought
    forward and closed as its own Close closes it (asking about unsaved edits, waiting for a
    save); the first that stays open (Cancel, or a save that didn't work) ends it, and the
    windows after it aren't asked.
  - **The theme:** the bar follows Legible's theme (muda draws it dark), and so do the menus
    that open from it (uxtheme's `SetPreferredAppMode`, for the process: dark or light, or
    Windows' own for Follow Windows) and the window's title bar.
  - **Sizing:** a window is created with a placeholder menu, so the size it's given (the
    remembered one) is its page's, with the bar outside it; the real menus then take its place.
  - **Tests:** `menu`'s check the command table (each item's route and shortcut, every command
    on the bar once, the labels and their access keys, the IDs), the states in View, Split and
    Edit, with unsaved edits, while saving, for a file that can't be edited and in the empty
    window, and Exit against real `saving::Session`s (two windows with unsaved edits are asked
    in turn, and Cancel on the second stops it; a save is waited for; a window closed
    meanwhile is skipped). `scripts/chrome-tests.mjs` runs each page command, and Ctrl+W.
- **The editor** (`web/editor/`, and `editor` for the window's side): View, Split and Edit.
  - **The modes:** every file opens in View. Split puts the Markdown text on the left and the
    live document on the right, with a divider between them (pointer, or the arrow keys and
    Home/End when it has focus) from 20% to 80%, remembered as the `split` setting. The divider
    is a 1 px line at the left of a 10 px grab area that lies over the document's
    margin, clear of the editor's scrollbar just left of it (it never moves either pane):
    hovered, focused from the keyboard or dragged (`dragging`, kept until it's let go), the line
    takes the accent and a grip shows at its middle (the contrast test checks the accent on the
    panes at 3:1); a double-click puts it back at 50%, saved as a drag is. Edit shows
    the text alone, in the reading column. Ctrl+E goes View → Split → Edit → View, and the
    toolbar's buttons follow; the last mode asked for wins, however long the editor takes to
    load. **The place is kept across a change of mode,** by the lines the blocks came from
    (`data-sourcepos`, as scroll sync maps them): the page itself doesn't scroll in Split and
    Edit, so View's place would otherwise be lost on the way. Going to View, the line at the top
    of the text (Edit) or of the document's pane (Split) is put at the top of the page under the
    chrome, at once if the document shows the text already, else once its render is shown, and
    again once its diagrams are drawn, held there by the chrome's hold as the blocks round it
    are laid out (Long documents); the reader's own scrolling, keys or pointer let it go. Going
    from View, the line at the top of the page is put at the top of the text, and Split's pane
    follows the text. The file as it opened is rendered without its lines, so there the way to
    the element at the top (each child's index from `#document` down) is kept instead, and
    followed in the first render that has them. The page's layout for each is `mode-split` or `mode-edit` on `<html>`
    (`editor.css`): in Split the document's pane scrolls by itself. The editor's pane is as tall
    as its top and bottom leave (`codemirror.css`'s `height: 100%` on its host is overruled, so
    its last lines are never below the window).
  - **The editor is CodeMirror 6** (`window.CM`, from `third_party/js/`), loaded the first time
    Split or Edit is chosen, and only then is the file's text asked for. It's mounted in a
    **shadow root**: there CodeMirror's styles go in as constructed stylesheets, which the
    page's CSP allows where it refuses them in the page itself, and `codemirror.css` (linked in
    the shadow root) gives it the document theme's colours, light and dark. The text is in the
    reading font (Sans or Serif) by default, or Monospace (the `editor-font` setting); it wraps,
    with no line numbers or gutter. Markdown is coloured lightly (headings larger and bold,
    marks faded, code in monospace), not as a code editor's palette.
  - **Spell-check** is WebView2's own: the editable content has `spellcheck="true"` and the
    page's language, so Windows' suggestions are on its right-click menu.
  - **Shortcuts, and no toolbar:** Ctrl+B bold and Ctrl+I italic (wrapping the selection, or
    unwrapping it), Ctrl+K a link round it, Ctrl+1/2/3 a heading, Ctrl+0 plain text, Ctrl+Z
    and Ctrl+Y, Ctrl+F find and Ctrl+H find and replace (CodeMirror's own panel). In the editor
    Ctrl+F, Ctrl+H and Ctrl+0 are the editor's; elsewhere they're the page's find and zoom.
    CodeMirror's own words (the panel's fields, buttons and options, go to line, what a screen
    reader is told) are Legible's strings, `chrome-editor-*` in `window.ftl`, through
    CodeMirror's `EditorState.phrases`.
  - **The preview in Split:** 150 ms after typing stops, the text is sent (`render`) with its
    `version`, which goes up on **every** change; the window renders it with `sourcepos` on and
    answers `LegibleEditor.preview(version, html)`. An answer is shown only if its version is
    still the text's, and `LegibleDoc.prepare(doc, {current})` checks the same before each step
    it takes (a diagram queued or inserted, its sheet adopted, code highlighted, the scroll map
    built), so nothing older than the text ever lands. A drawn diagram whose source hasn't
    changed is **moved** into the new preview (never cloned, each at most once) rather than
    drawn again, taking the new placeholder's lines, but only one drawn in every theme
    (`LegibleDoc.isDrawn`): one whose drawing was cut short is drawn afresh.
    `LegibleDoc.releaseSheets()` then drops the adopted sheets of diagrams no longer in the
    document.
  - **Scroll sync, both ways:** the rendered blocks' `data-sourcepos` map lines to places in the
    document. Scrolling the text puts the same line (and the same share of it) at the top of
    the document; scrolling the document puts the block at its top at the top of the text,
    through CodeMirror's `scrollIntoView` (which measures the lines on the way). Each side
    ignores the scroll it caused, so the panes don't chase each other.
  - **Unsaved changes:** the text is compared with what was last loaded or saved, so undoing
    back to it clears them. While there are some, a dot shows beside the modes and in the
    window's title ("● notes.md – Legible"), live reload holds changes from disk (above), and
    the page is never served again (pictures from the internet wait for a save, above). The
    window keeps the file's text as last **applied** (shown, or given to the editor), for the
    editor's `source`; a change held for unsaved edits becomes it only with Use the file.
    For saving: `LegibleEditor.isDirty()`, `contents(request)` (the text and its `version`,
    also sent back as `contents`), then `markSaved(version)` (only what that version held
    counts as saved, so typing during a save stays unsaved) or `saveAbandoned(version)`. A
    version handed to a save is kept until one of those says how it went, however much is
    typed meanwhile, for up to four saves at once (the oldest beyond that is let go, and its
    markSaved leaves the text unsaved); a save done ends the older ones, so one finishing
    late can't take the text back. Edits themselves keep nothing. The window's title is shown
    from live reload's state after every change to it, from the editor or from Use the
    file.
  - **A file that can't be edited where it is** (read-only, in the temp folder, or in Outlook's
    or the browser's cache, `Content.Outlook`, `INetCache` or `Temporary Internet Files`, where
    an email's attachment or a download opens from: `editor::file_is_editable`), or in an
    encoding Legible doesn't save (below), shows Split and Edit unusable, their tooltips saying
    how to edit a copy, and Ctrl+E does nothing. **The switch is only ever the modes:** Save a
    copy… is File's (`save-copy`, Ctrl+Shift+S), and Ctrl+S offers the copy too.
  - **Generations:** the editor's previews are renders of the window's generation, and carry it
    with what their pictures may do (`preview(version, html, {generation, remote})`); the
    preview tells the chrome (`LegibleChrome.adopt`), so the bar's answers are for the render
    shown, and the approved pictures show in it. A change from disk with the editor open goes
    through the same pictures, bar and CSP steps as one shown in View.
  - **Messages** (`editor::Message`): `source`, `render`, `dirty`, `contents`, `save`,
    `save-copy`, `compare`, `raced` and `clean` from the page (and the bar's `keep` and
    `reload`, which chrome.js's live reload answers use too); `load({text, editable})`,
    `preview(version, html, info)`, `fromDisk(text)`, `markSaved(version)`,
    `saveAbandoned(version)`, `compare(kind, id, html, info)`, `freeze()`, `closeCheck()` and
    `unfreeze()` from the window, and the `legible:external-change` event for the bar.
  - **Tests:** `scripts/chrome-tests.mjs` stands in for the window, rendering each preview
    through core (`legible-core`'s `editor-preview` example), and can hold an answer back to
    test a stale one: the modes and Ctrl+E, the shadow root under the CSP, the shortcuts,
    unsaved and saving, a file that can't be edited, the divider, stale previews, diagrams
    moved across previews, a change from disk, scroll sync both ways, pictures in the preview
    (only the approved, none typed new, taken away at once), the pictures bar while dirty,
    Ctrl+S and Ctrl+Shift+S, the conflict bar (Compare included) and the close's freeze.
    `editor::ReloadGate`'s test covers every reload waiting while dirty.

## Saving

Saving can't lose or mangle work: what's written is the file's own format, only over the file as
it was read, and nothing that might be the only copy of someone's text is ever removed unasked.

- **Formats** (`legible_core::save::{decode, encode}`): a file opens as UTF-8 (with or without a
  BOM) or UTF-16 LE or BE (with its BOM); its line endings are normalised to `\n` for the editor
  and written back as the file's, so an unedited text saves to exactly its own bytes. A file that
  mixes them saves with the most frequent (a tie goes to the first), and says so when the
  editor opens (`chrome-line-endings-mixed`); a final newline, or none, is part of the text. Any
  other encoding opens for reading only, with a note, and a copy of it is its bytes as they are
  (`disk::Content::Unsupported`).
- **Stamps:** the window keeps the file as last applied (`disk::Disk`: its content and the
  stamp of exactly those bytes, read from one handle: length, FNV-1a hash, last write and the
  file's identity). A save checks the file against `DocWindow::expected`: that stamp, or after
  Keep my changes the stamp of the change kept over (or none, to put back a file that went).
- **The write** (`save::write`, Windows' half in `win32::replace`): the bytes go to a staging
  file beside the document (`.{name}.legible-{pid}-{n}.tmp`), flushed, and the backup name is
  reserved beside it (`.bak`, empty until the replace fills it); both are created new, so a
  file already under such a name (a crashed run's, or any other) is never touched, and another
  pair is tried. The document is checked
  against the stamp and only then replaced, by `ReplaceFileW` with a backup name, which keeps
  its attributes, creation time and DACL (a merge or security error fails the save rather than
  weakening it: there's no fallback). A sharing violation is tried again five times, each after
  checking again. The half-done result (`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`) puts the document
  back, and if even that fails says where both files are (`Partial`). A new file (a copy, or one
  put back) is moved into place without ever replacing one that's appeared.
- **A raced save: the file saved at the same moment**: a write that
  lands in the moment between the check and the replace is in the backup, which `write` then
  finds isn't the bytes that were checked (`Raced`). The save stands; the other version's bytes
  are kept, moved (never over a file) to a copy beside the file whose name says what it is
  ("notes (other version).md", or numbered), never removed silently, and the bar says plainly
  that the file was saved at the same moment and both are kept, naming the copy. Legible never
  removes that copy: Keep my changes leaves it, and says so; Use the other version writes the
  copy's bytes as they are when taken back over the file (checked against the save's stamp, so
  atomically), only while the file is still the save's own, and leaves the copy too, so whatever
  is written meanwhile is kept somewhere (`recovery`, proved on temp files). The page freezes
  its text when Use the other version is confirmed, so nothing typed after that is replaced; the
  window unfreezes it once the take is over, and a close asked for meanwhile waits for it.
  Compare shows the copy as it is. A page served again shows the bar again.
- **The wording**: there's one file and two versions of its text,
  **your changes** (in Legible, unsaved) and **the file** (as it is on disk now). Legible can't
  know who changed a file (another app, a sync, a colleague on a share), so its messages name
  the open file, quote the copies' names, and never say "they", "theirs" or "another program".
  The window sends the bar its words (`legible:external-change`'s `text`), so they name the
  file.
- **The window's side** (`shell::save`, with the decisions in `saving::Session`): Ctrl+S asks
  the editor for its contents (`contents(request)`), and the write runs on a thread of its own,
  one at a time (a save asked for meanwhile waits, the newest winning). Once it's over the file
  is read back: only if it's exactly what was written does that version become saved
  (`markSaved`), the window's file, stamp, render and pictures the saved text's, and live reload
  start a new epoch from it (`Watch::register`), so the window's own write never comes back as a
  change. A dirty `false` then lets a reload that waited go ahead, as any save does, showing the
  saved text. Anything else leaves the
  text unsaved (`saveAbandoned`): a change found is held as live reload holds one, and asked
  about; a failure is said, with the text still in the editor.
- **The conflict bar** (`web/editor/editor.js`, under the toolbar and the chrome's other bars,
  like them: chrome.js counts it in `--chrome-top` when `LegibleChrome.layout()` is called):
  "notes.md changed while you were editing it.", with Keep my changes, Use the file (after a
  confirmation: "Use the file as it is now? Your changes will be discarded.") and Compare; a
  raced save offers Use the other version in its place, and a file that went offers Keep my
  changes and Close. Use the file, done, says so ("Your changes were discarded. notes.md shows
  the file as it is now."). Compare goes to Split with the file in the preview
  and the text in the editor, until the text changes, Stop comparing is chosen (the bar says
  it's comparing meanwhile) or the question is answered. Only the
  newest question is asked, and the window acts only on an answer for the newest; Keep my
  changes for a change a save found saves again. Use the file and Compare never navigate the
  page.
- **Save a copy…** (Ctrl+Shift+S): the Save dialog, in Documents, named "<name> (copy)"; never
  the open file itself, by any name (its path or its identity, checked again just before the
  write). The destination is approved as it is when the dialog closes (absent, or a file with
  its stamp: `recovery::approve`), and the editor's text (or the file's bytes, if it isn't
  editable) is written only over exactly that: a file that appears or changes after the
  approval is left alone, and said. The copy opens in a window of its own, and the window's
  own text stays as saved or unsaved as it was.
- **Closing** (every way a window closes goes through `Shell::ask_close`): with nothing unsaved
  and nothing being written it closes; with a save under way it waits for it, then checks again;
  with unsaved edits it asks (a task dialog: Save, Don't save, Cancel). Save freezes the editor
  (`freeze()`: read-only, and a transaction filter that keeps out every edit) before asking for
  its text, so nothing typed afterwards can be lost; once saved, the page is asked whether it's
  clean (`closeCheck()`, which freezes it before it answers, on every path, a close during an
  ordinary save's included), and only then does the window close; asked again, it takes edits
  again first. A conflict or a
  failure unfreezes it, and the window stays. Don't save is the only way edits are dropped.
- **Windows ending the session** (signing out, restarting, shutting down, or an installer's
  Restart Manager; `session`, `win32::session`). tao ignores `WM_QUERYENDSESSION`, and on
  `WM_ENDSESSION` its hidden window exits the process at once, so each document window hears
  both through a subclass of its own procedure, and does what matters at the question, which
  comes first even for a forced end. Windows asks no other app's windows, and maybe none of
  Legible's others, once one has stopped the end, so whichever window it asks answers for all of
  them. Every window with anything unsaved (edits, a save under way, a close's save:
  `session::unsaved`) first keeps a copy of the editor's newest text in `Recovery\` in Legible's
  folder (`legible_core::unsaved`: never beside or over the document, written whole and renamed
  into place). The page sends that text after every edit while it's unsaved
  (`text\n<version>\n<text>`), and the window's message handler keeps it as it comes, never
  behind the preview's debounce or the event loop. Each such window then gives its reason ("You
  have unsaved changes in notes.md", `ShutdownBlockReasonCreate`), the end is stopped, and the
  unsaved windows ask their close question in turn, oldest first (`session::Round`). Cancel ends
  the round. A critical end can't be stopped: the copies are kept, and it goes. With nothing
  unsaved, the session ends at once. A copy goes once its text is saved or dropped, or the
  session goes on after all. Each run names its copies for itself (`unsaved::session`), so a
  copy from an earlier run, still waiting for the reader's choice, is never written over. The
  listener reads state the shell brings up to date after every event (`Shell::share_ending`), as
  it runs outside the shell's own work.
- **Unsaved edits offered back** (`shell::kept`): at the start, each copy
  in `Recovery\` is offered back (`session::offer`). Its file opens with a bar over the document
  (the conflict bar's place): "Restore unsaved changes to notes.md?", adding "notes.md has
  changed since then." when the file isn't the one the edits were made to (the copy keeps its
  hash). Restore
  puts the text into the editor as an edit (Split, unsaved, undone by Undo), and the copy goes
  once the page says it's in (`restored`); Discard drops it; a window closed without a choice
  keeps it for next time. Copies of one file from more than one run are offered in turn, oldest
  first. A copy its file already holds goes
  unasked. One whose file has gone, or can't be edited where it is, is asked about before any
  window opens: Save as… (Save a copy's dialog, never over the file, the text in the file's
  own format if it's known) or Discard, and Cancel keeps it.
- **Screenshots** (`ui-shots` set `saving`): the bar in Split, its confirmation, Compare, a raced
  save and a file that went, in light and dark.
- **Tests:** `legible-core`'s `save` tests (byte-exact round trips in every format, a crash
  between the write and the replace, a conflict found, a raced write kept, busy retries, a
  half-done replace recovered or reported, a new file never over one that appeared),
  `win32::replace`'s (the attributes and DACL kept, ReplaceFileW's results), `recovery`'s (Use
  the other version while either file changes, a copy only over what the dialog approved),
  `saving`'s, the
  chrome tests (above), `examples/save-check`, which saves through the real page and
  Windows' replace in a real WebView2 (TESTING.md), and `tests/shell.rs`, which drives the real
  window (The window, above): a save then Copy or a pictures answer straight away, a reload that
  waited for a save, Use the other version (completed, raced by a third writer, and written
  over before it's taken up), Keep my changes over a change a save found, a close during a save,
  Save a copy over
  a destination that changed, Windows ending the session (stopped and asked, nothing unsaved,
  cancelled, two windows in turn, a critical end, an end before the preview caught up, every
  window kept at the first question, an earlier run's unchosen copy outliving a later end) and
  the copies offered back (Restore, Discard, offered again, a file that has gone, one already
  saved).

## Performance

**The budgets** (for a mid-range laptop): the window shown in under 300 ms; a typical 200 KB
document readable in under 700 ms; a 2 MB document responsive; idle memory recorded.

**Measured under load** on 29 Oct 2026: another workload was using 83–93% of the CPU throughout,
so these are a record of the design, not of the budgets, which are to be measured again on a
quiet machine before a release. A release build (`legible.exe` 10.8 MB, the JavaScript bundles
inside it), run as a portable copy from a temp folder, on a desktop with an 8-core AMD Ryzen 7
9800X3D and WebView2 156, so faster than the laptop the budgets are for. Each run is a new
process opening one document; the times are the shell's own log lines (The window, above), from
the process's creation to the window shown, and to the page's first frames after its scripts ran
(`DOMContentLoaded` and two animation frames, before code is highlighted or diagrams are drawn).
Six runs each (four for 2 MB), as median and range:

| Document | Window shown | Document drawn |
|---|---|---|
| The showcase, 5 KB (front matter, tables, code, three diagrams) | **143 ms** (110–229) | **855 ms** (794–943) |
| A prose report, 205 KB (71 sections of paragraphs, a list and a table each) | **106 ms** (100–229) | **768 ms** (678–943) |
| A heavy 205 KB document (the showcase, then the GFM, alerts, headings and front matter fixtures repeated: 5,123 blocks, about 2,000 headings, 165 tables and 170 code blocks) | **106 ms** (94–119) | **991 ms** (857–1,395) |
| 2 MB (the heavy document repeated: 50,600 blocks) | **100 ms** (92–114) | **3.5 s** (3.3–3.9) |

**Responsiveness**, by script in the window's own page over its DevTools port: the first find for
a word; the first jump to the end, straight after the document is drawn; how long the
background layout of a document of thousands of blocks took (The window, above); then, once it
had, a jump to the end, and 40 small steps of the scroll at the top and at the end (median and
worst wait for a frame):

| Document | First find | First jump to the end | Laid out | Then to the end | Scroll steps |
|---|---|---|---|---|---|
| Showcase | 7 ms | 21 ms | — | 12–120 ms | 6 ms, at most 8 |
| Prose report, 205 KB | 15 ms | 9 ms | — | 11 ms | 6 ms, at most 10 |
| Heavy, 205 KB | 39 ms | 70 ms | 0.23–0.34 s after it's drawn | 11–21 ms | 6–7 ms, at most 76 |
| 2 MB | 427 ms | 1.0 s | about 5 s after it's drawn | 0.23 s | 6–8 ms, at most 187 |

**Idle memory:** two windows on the showcase, left alone for 20 s: eight or nine processes
(Legible and WebView2's), **311–335 MB private memory** and 515–570 MB working set over two runs.

**Against the budgets** (to be confirmed on a quiet machine):
- **The window:** met at every size, 2 MB included (100–143 ms): a window is made, and WebView2
  started in it, before its file is read.
- **A typical 200 KB document:** over the budget under this load (768 ms for the prose report,
  991 ms for the heavy one). On a quiet machine the same night, before the background layout,
  the prose report was drawn in 470 ms.
- **A 2 MB document:** responsive: it appears in about 3.5 s under load, its first find answers
  in 0.4 s, and once it's laid out, about 5 s after it's drawn, a step of the scroll takes 7 ms.
- **Idle memory:** recorded above.

To measure again: the shell's log has both start-up times on every run, in `logs\` in Legible's
folder.

## Strings and languages

Every user-visible string comes from Fluent (`.ftl`) files in `locales/<language>/`, through
`legible_core::i18n`. English is the only language in v1, and the
fallback for every key.

- **The files are compiled in** (`include_str!` in `crates/legible-core/src/i18n/embedded.rs`):
  `common.ftl` (used in more than one place), `errors.ftl`, `window.ftl`, `document.ftl` and
  `clipboard.ftl` so far; each part of the app adds
  its own file.
- **The API:** `i18n::init(tag)` once at start-up; then `tr("app-name")`, or
  `tr!("error-file-not-found", file = name)` with arguments (text or numbers).
- **Choosing the language:** an exact match for the tag, then the same language without a region,
  then any region of it, then English, key by key.
- **Nothing fails silently:** a key missing in every language shows as the key itself and is
  logged, as is a message that doesn't format.
- **Tests** (`i18n::embedded`): every file parses with no duplicates; every key has a value; every
  `.ftl` file on disk is embedded; other languages only have keys English has; and every literal
  key the code passes to `tr(`, `tr_args(`, `tr!(` or `tr_plurals(` exists in English. So pass
  keys as literals.
- **Logs stay in English**: they're diagnostics.
- **The page's own numbers:** the page counts words and matches and zooms for itself, so its
  strings keep a number's place (`{count}`, `{current}`, `{zoom}`), and the script fills it with
  the number written the language's way (`toLocaleString` with `<html lang>`). A count that
  changes the words round it is sent in each plural form (`i18n::tr_plurals`, the status bar's
  word count): the message is resolved from its Fluent syntax once for each of CLDR's six
  categories, a select on the count taking the variant Fluent would and each `{ $count }` kept
  as the script's `{count}`, so no digit in the text is taken for the number. A variant for an
  exact number (`[0]`) is a form of its own, resolved as Fluent would for that number (by
  Fluent's own plural rules for the bundle's language, a region falling back to its language).
  The script takes the exact number's form first, else its category's by `Intl.PluralRules`.
  The count is supported as a selector and a placeable only: anywhere else (in `NUMBER()`) it's
  missing, which is logged.
- **Nothing is joined in code:** text made of parts (the status bar path's tooltip, the editor
  bar's text while comparing, About's Copy lines, a front matter list or table) is a Fluent
  pattern, so a language can order and punctuate it its own way. A pattern the page fills keeps
  its parts' places for the script, which fills them in one pass, so a part's own text (a file
  name with `{text}` in it) is never taken for another's place.
- **Why something failed** (`{ $reason }`): `legible_win::reason::of` says the common failures
  in Legible's words (`reason-*` in `errors.ftl`: not there, not allowed, in use, read-only, disk
  full), another of Windows' in Windows' own words (in Windows' language, which may not be
  Legible's) without Rust's " (os error N)", and one of Legible's own as "something unexpected",
  which the log explains. The log keeps every error as it was.
- **Deliberately left in English or as they are:**
  - **The document's own words:** front matter keys and text values, headings and so on are
    shown as written; only true and false and the joins are Legible's.
  - **Mermaid's frame** (`web/doc/mermaid-frame.html`): its `lang="en"` and `<title>` are never
    seen or read out (the frame is hidden and `aria-hidden`), and the diagrams' text is the
    document's.
  - **A WebView2 that won't start** in a window (`error-open`'s reason): wry's error, with
    Windows' message in it. It's rare, and the log has the detail.

**Adding a string:** add `key = Text` to the right file in `locales/en/` (lowercase, hyphenated,
prefixed by where it shows: `error-`, `toolbar-`…), with a comment for any argument whose
meaning isn't obvious, then use `tr("key")` or `tr!("key", arg = value)`.

**Adding a language:** create `locales/<tag>/` with any of English's files and add a `Locale` for
it to `LOCALES` in `embedded.rs`; the tests fail until every file on disk is listed there.

## Installing, and the default app

The MSI (`installer/legible.wxs`) and the portable zip are built by `scripts/package.ps1`;
docs/RELEASING.md has how, and what they hold. **File associations** are per user
(`associations`): a ProgID, `Legible.Markdown`, under each extension's `OpenWithProgids`
(`.md`, `.markdown`, `.mdown`), `Applications\legible.exe` with its `SupportedTypes`, and
Legible's capabilities under `RegisteredApplications`. The MSI writes them; a portable copy
writes the same values for its own exe only when the user asks in Settings, and can remove them
again (only its own values: another app's under the same extensions stay). A test reads the
`.wxs` and checks it registers exactly `associations::values`. The registry calls
(`win32::registry`) take their root, so their tests run in a hive of their own, loaded from a
temp folder (`RegLoadAppKeyW`), never the user's registry.

**The default app:** Windows doesn't let an app make itself the default, so Legible never
writes a default or `UserChoice`. While it's registered but isn't what opens `.md`
(`AssocQueryStringW`), each window shows a bar offering "Make Legible the default for Markdown
files": it opens Default apps at Legible's entry
(`ms-settings:defaultapps?registeredAppUser=Legible`) and says in one line what to choose.
Taking the offer or "Don't ask again" turns the `offer_default_app` setting off, for every
window; Close puts it off, in every window, until Legible starts again; Settings' General tab keeps the button, and a portable copy's Register and
Remove. A window asks again when it's focused, so it catches up once the user comes back from
Default apps.

## Settings, data and logs

**Where things live** (`legible_win::paths`): `AppPaths::detect("Legible")` at start-up.
**Portable** when a file named `portable` (empty, no extension) or a `settings.toml` sits beside
the exe, and everything then lives beside it; **installed** otherwise, in `%APPDATA%\Legible\`.
A portable folder that can't be written to stays the app's folder (`is_writable()` is false):
the app keeps its settings in memory and says so, and never writes elsewhere.

**Settings** (`legible_win::settings`): `settings.toml` in that folder, flat `key = value` lines,
read and written without a TOML library. Theme (follow Windows, Light, Dark), reading style and
width, default zoom, pictures from the internet (Ask, Never, Always), the outline (Automatic,
Always, Never), the editor's font (the reading style, or Monospace), the editor's share in
Split (20–80%, 50 by default), and the Comfort group's three, each on by default: Zoom with
Ctrl + mouse wheel, Open windows at their last size and position, and Remember where I was in
each file. A missing or nonsensical value is its default; lines it doesn't know are kept (an
earlier copy's `check_for_updates` among them: Legible has no update check, so it offers no such
setting). Saved through a temporary file renamed over the old one.

**Unsaved edits** kept as Windows ended the session: `Recovery\` in that folder, one copy per
document until the reader chooses (Saving, above). **Where windows go and where the reader was**
in each file: `window.toml` and `places.txt` (The window, above).

**Logs** (`legible_win::logging`): one file per process, `logs\legible-<UTC start>-<pid>.log`,
never renamed, so windows open at once never touch each other's logs; the oldest beyond 20 files
or 14 days are deleted at start-up, skipping any still open. Panics on any thread are logged.

## Licences

Everything Legible ships is MIT or equally permissive: MIT, Apache-2.0, BSD, ISC, Zlib, 0BSD,
BSL-1.0, CC0 or Unicode. System fonts only. Anything else ships only on the maintainer's
decision, recorded here.

- **Rust crates:** `cargo deny check` (`deny.toml`) gates every crate in the exe, in CI too.
- **JavaScript:** the audit in "Web assets and the vendored JavaScript" gates the bundles.
- **THIRD-PARTY-NOTICES.txt** (`scripts/third-party-notices.ps1`, committed): part 1 is the
  crates in the Windows build (cargo-about, `about.toml` and `about.hbs`), then the copyright
  notices of every crate whose licence text there is a template (MIT's `<year> <copyright
  holders>`, BSD's `<year> <owner>`, Apache's `[yyyy]`), collected from its own licence and
  notice files (the script fails if any such crate has none), and where the MPL-2.0 crates'
  source is;
  part 2 is each bundled JavaScript file with its packages and their licence texts
  (`third_party/js/LICENSES.md` and `NOTICES.txt`). About opens it (its third-party software
  link), both packages ship it, and
  `scripts/package.ps1` fails if it's out of date (docs/RELEASING.md).
- **Decided exceptions:**
  - **Mermaid without ELK** (23 Oct 2026 brief; decided 27 Oct 2026): elkjs is EPL-2.0, so
    Legible's Mermaid bundle leaves it out rather than ship it.
  - **cssparser 0.38.0 and dtoa-short 0.3.5, MPL-2.0** (decided 27 Oct 2026): ammonia 4.1 and
    later needs them, for CSS filtering Legible doesn't use. The alternative, ammonia 4.0.x, has
    open advisories. They ship unmodified, allowed by name and version in `deny.toml` (no other
    MPL crate passes), and are credited with where their source is in
    `third_party/rust/LICENSES.md`, for THIRD-PARTY-NOTICES.

## Build-only tools

Used to build Legible but never shipped, so the licence rule doesn't apply to them.

| Tool | Use |
|---|---|
| Rust toolchain 1.98.1 (rustc, cargo, rustfmt, clippy) | build, format, lint |
| cargo-deny | the licence and advisory gate (`deny.toml`) |
| `embed-resource` 3 (MIT), with the Windows SDK's `rc.exe` | compiles the manifest, icon and version info (`crates/legible/build.rs`), and the manifest into legible-win's test exes (`crates/legible-win/build.rs`) |
| Python 3.10 or later, with Pillow | draws the icons (`assets/export.py`) |
| cargo-nextest 0.9.149 (Apache-2.0 OR MIT) | runs the tests in parallel (`.config/nextest.toml`, docs/TESTING.md); `cargo test` works without it |
| PowerShell 7 | `scripts/fetch-js.ps1`, `scripts/build-js.ps1`, and `scripts/clean-build.ps1`, which frees `target\` (the workspace crates' dev and test builds, or with `-All` everything cargo built, never `target\ui-shots` or `target\dist`; docs/TESTING.md, Disk space), checked by `scripts/test-clean-build.ps1` |
| Node.js 22.4 or later, with npm | `scripts/build-js.ps1`, `scripts/audit-js-licences.mjs` and the browser tests (`scripts/chrome-tests.mjs`, `scripts/doc-tests.mjs`; standard library only, its built-in WebSocket included) |
| esbuild 0.28.2 (MIT) | bundles CodeMirror and Mermaid; pinned in each `scripts/js/<bundle>/package-lock.json` |
| cargo-about 0.9.2 | the crates part of THIRD-PARTY-NOTICES.txt (`about.toml`, `about.hbs`) |
| WiX Toolset 5.0.2 (MS-RL; no extension or WixUI, so none of it is in the MSI) | builds the MSI (`installer/legible.wxs`, `scripts/package.ps1`; docs/RELEASING.md) |
| GitHub Actions: `actions/checkout@v5`, `Swatinem/rust-cache@v2`, `taiki-e/install-action@v2` | CI, and the draft release (`release.yml`) |

## Generic parts

These parts know nothing of Markdown or of Legible's window: they're kept generic, with only
names to change, so they could move into a crate of their own.

| Part | Where | What's Legible's own |
|---|---|---|
| The licence gate | `deny.toml` | nothing |
| The Fluent strings module | `crates/legible-core/src/i18n/mod.rs` | the doc example and one test use Legible's keys |
| Its embedded files | `crates/legible-core/src/i18n/embedded.rs` | the file list and core keys |
| Portable/installed paths | `crates/legible-win/src/paths.rs`, tests in `tests/paths.rs` | names only |
| Logs (one file per process, with cleanup, panics logged) | `crates/legible-win/src/logging.rs`, tests in `tests/logging.rs` | names only |
| The current user's SID, the UI language and Win32 errors as `io::Error`s | `crates/legible-win/src/win32/mod.rs` (`current_user_sid`, `user_sid_of_process`, `ui_language`, `io_error`) | nothing |
| The static C runtime | `.cargo/config.toml` | nothing |
