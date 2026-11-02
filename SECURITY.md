# Security

Legible opens files people are sent: email attachments, downloads, notes from colleagues and AI
tools. A Markdown file can carry HTML, scripts, links and pictures from anywhere, so security
reports matter a lot. Thank you for taking the time.

## Reporting a vulnerability

**Please report it privately, not in a public issue.**

Use GitHub's private vulnerability reporting: on
[github.com/Suremise/legible](https://github.com/Suremise/legible), open the **Security** tab and
choose **Report a vulnerability**, or go straight to
[the report form](https://github.com/Suremise/legible/security/advisories/new). Only the
maintainers can see it.

Please include:

- what the problem is, and what someone could do with it;
- a document (or the steps) that shows it, the Legible version (Help > About Legible, then Copy)
  and Windows version;
- anything you've found about a fix.

We aim to answer within a week, and to keep you told as we work on it. We'll credit you in the
advisory and the changelog unless you'd rather we didn't. Please give us a reasonable time to fix
it before you make it public.

## Supported versions

Fixes go into the latest release only.

## What's most sensitive

Reports in these areas matter most. Each is a promise Legible makes about any document, however
it was written:

- **The sanitiser** (`crates/legible-core/src/sanitize.rs`): a document's HTML is cleaned against
  an allowlist before anything shows it. Nothing in a document may run script: no `<script>`,
  event handler, `javascript:` link, SVG or MathML, style or form gets through, and a document's
  ids can't clash with the page's own. A document that runs script, or that loads anything from
  the network without the reader allowing it, is a vulnerability.
- **The navigation policy** (`crates/legible-win/src/links.rs`, `web/chrome/links.js` and the
  window's navigation handler): a link never navigates Legible's window. Web links open in the
  browser (an `http:` one only after asking), mail links in the mail app, and Markdown files in a
  Legible window; anything else does nothing. Every navigation away from Legible's own page, and
  every new window, is refused. A document that takes over the window, opens a page in it, or
  launches a program or a file that isn't Markdown, is a vulnerability.
- **`legible-doc:` confinement** (`crates/legible-win/src/confine.rs`): the document's pictures
  are served from its own folder only, by path after following links and junctions, and only
  pictures. A document that reads any other file through it, or anything outside its folder, is a
  vulnerability.
- **The clipboard** (`crates/legible-core/src/clipboard_html.rs`, `crates/legible-win/src/clipboard.rs`):
  Copy embeds the document's pictures, read through the same confinement, and never fetches one
  from the internet. What's copied keeps no script, style, event handler or class, so pasting it
  into Outlook, Word or OneNote is as safe as the text. A copy that carries active content, or a
  file from outside the document's folder, is a vulnerability.
- **Pictures from the internet** (`crates/legible-win/src/consent.rs` and the request filter):
  they load only after the reader allows them, only the document's own, and only over HTTPS to the
  hosts allowed. Any request to the network before that, or to another host, is a vulnerability.
- **Diagrams** (`web/doc/mermaid-frame.*`): Mermaid runs in a sandboxed frame that can't reach the
  page, and what it draws is rebuilt by the page from data. A diagram that reaches the page, the
  window or the network is a vulnerability.
- **Saving**: Legible writes only the open file (or a copy the reader chose), only over the file
  as it was read, and never removes text that might be the only copy. A document or another
  process that gets Legible to write elsewhere, or to lose someone's changes, counts.
- **One Legible per user**: a later launch hands its files to the running Legible through a named
  pipe only that user can open. Another user or process reaching it counts.

Legible makes one network connection of its own, the update check, and only while the reader has
it turned on: a `GET` of `https://github.com/Suremise/legible/releases/latest` whose redirect it
reads without following (`crates/legible-win/src/update.rs`, `win32/http.rs`). A request with it
off, to another address, or carrying anything about the reader or their documents, is a
vulnerability, and so is a newer release's link that opens anything but that release's page on
GitHub. The only other requests it allows are for pictures the reader has let in, and WebView2
makes them.
