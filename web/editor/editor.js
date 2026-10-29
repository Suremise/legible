// The editor: View, Split and Edit (docs/ARCHITECTURE.md, "The editor"). Run deferred, after
// chrome.js.
//
// Every file opens in View. Split puts the Markdown text on the left and the live document on
// the right, their scrolling kept in step; Edit shows the text alone, in the reading column.
// Ctrl+E goes View → Split → Edit. The editor is CodeMirror 6 (window.CM, loaded the first time
// it's needed), mounted in a shadow root: there CodeMirror's styles go in as a constructed
// stylesheet, which the page's CSP allows, where in the page itself they'd be refused.
//
// With the window (window.ipc.postMessage, answered through window.LegibleEditor):
//   source                       the editor opened: the file's text, please → load({text, editable})
//   render\n<version>\n<text>    the text to render for the preview → preview(version, html,
//                                {generation, remote}): its render, and what its pictures may do
//   dirty\n<true|false>          the text differs from what was loaded or saved (or no longer
//                                does): while it does, live reload holds changes from disk
//   text\n<version>\n<text>      after every edit while it does, and before dirty\ntrue: the
//                                text, which the window keeps as it comes for Windows ending
//                                the session
//   contents\n<request>\n<version>\n<text>   the answer to contents(request)
//   save                         Ctrl+S
//   save-copy                    "Save a copy…" (Ctrl+Shift+S, or the button on a file that
//                                can't be edited where it is)
//   keep\n<id>, reload\n<id>, compare\n<id>   the conflict bar's Keep my changes, Use the
//                                file and Compare, for held change <id>
//   raced\n<keep|take|compare>\n<id>   the same (Use the other version for take), for save <id>
//                                made at the same moment as something else saved the file
//   clean\n<true|false>          the answer to closeCheck(): nothing is unsaved
//   restore\n<yes|no>            the bar's Restore or Discard, for unsaved changes kept as
//                                Windows ended the session (offerRestore(question)); Restore's
//                                text then comes as restore(text)
//   restored                     restore(text)'s text is in the editor
// and once a save has written a version, markSaved(version), or saveAbandoned(version) if it
// didn't (the version asked for is kept until then). The conflict bar asks about a
// `legible:external-change` event on window ({kind: "changed" | "deleted" | "raced", id, text}:
// the window's own words, which name the file),
// and Compare's answer is compare(kind, id, html, info): the version on disk in the preview.
// For a close's save the window calls freeze() (the text takes no changes), then closeCheck()
// (which freezes it too, before it answers), and unfreeze() if the window stays open. A
// confirmed Use the file (or Use the other version) freezes the text before its answer goes;
// the window unfreezes it once the take is over. Once the editor has opened, a
// change on disk comes to it (fromDisk(text)) rather than to the page, as the text that's saved:
// the window holds such a change while the text is dirty, so fromDisk never discards an edit
// unasked (Use the file is asked).
//
// Every change to the text moves `version` on. A preview is shown only if it's of the current
// version, and whatever it starts (diagrams, highlighting, the scroll map) lands only while it
// still is: so nothing older than the text ever shows. A drawn diagram whose source hasn't
// changed moves into the new preview instead of being drawn again.
(() => {
  "use strict";

  const root = document.documentElement;
  const doc = document.getElementById("document");
  const data = (id) => JSON.parse(document.getElementById(id).textContent);
  const strings = data("legible-chrome-strings");
  const { editable } = data("legible-editor");
  const s = (key) => strings[key] ?? key;
  const send = (message) => window.ipc?.postMessage(message);

  const MODES = ["view", "split", "edit"];
  const RENDER_DELAY_MS = 150;
  const SPLIT_MIN = 20;
  const SPLIT_MAX = 80;
  // Saves that can be in flight at once (contents asked for, markSaved or saveAbandoned not yet
  // told): beyond it, the oldest is let go, and its markSaved leaves the text unsaved.
  const SAVES_IN_FLIGHT = 4;

  let settings = data("legible-settings");
  let mode = "view";

  // --- The panes -----------------------------------------------------------------------------

  const host = document.createElement("section");
  host.id = "editor";
  host.setAttribute("aria-label", s("editor"));
  const shadow = host.attachShadow({ mode: "open" });
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = "/editor/codemirror.css";
  const mount = document.createElement("div");
  mount.className = "editor";
  shadow.append(link, mount);

  const divider = document.createElement("div");
  divider.className = "editor-divider";
  divider.setAttribute("role", "separator");
  divider.setAttribute("tabindex", "0");
  divider.setAttribute("aria-orientation", "vertical");
  divider.setAttribute("aria-label", s("editor-resize"));
  divider.setAttribute("aria-controls", "editor");
  divider.setAttribute("aria-valuemin", String(SPLIT_MIN));
  divider.setAttribute("aria-valuemax", String(SPLIT_MAX));
  document.body.append(host, divider);

  let split = settings.split ?? 50;
  // The share --split gives: editor.css's own default until it's first set. A custom property
  // on <html> restyles the whole document, so it's set only when it changes.
  let splitShown = 50;
  function setSplit(percent, save) {
    split = Math.round(Math.min(SPLIT_MAX, Math.max(SPLIT_MIN, percent)));
    if (split !== splitShown) {
      splitShown = split;
      root.style.setProperty("--split", String(split));
    }
    divider.setAttribute("aria-valuenow", String(split));
    if (save && split !== settings.split) {
      settings = { ...settings, split };
      send(`setting\nsplit\n${split}`);
    }
  }

  // Dragged, the divider keeps its highlight (`dragging`) until it's let go, wherever the pointer
  // is; a double-click puts it back in the middle, saved as a drag is.
  divider.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    divider.setPointerCapture(event.pointerId);
    divider.classList.add("dragging");
    const move = (e) => setSplit((e.clientX / window.innerWidth) * 100, false);
    const up = () => {
      divider.removeEventListener("pointermove", move);
      divider.removeEventListener("pointerup", up);
      divider.removeEventListener("lostpointercapture", up);
      divider.classList.remove("dragging");
      setSplit(split, true);
    };
    divider.addEventListener("pointermove", move);
    divider.addEventListener("pointerup", up);
    divider.addEventListener("lostpointercapture", up);
  });
  divider.addEventListener("dblclick", () => setSplit(50, true));
  divider.addEventListener("keydown", (event) => {
    const step = event.shiftKey ? 10 : 2;
    if (event.key === "ArrowLeft") setSplit(split - step, true);
    else if (event.key === "ArrowRight") setSplit(split + step, true);
    else if (event.key === "Home") setSplit(SPLIT_MIN, true);
    else if (event.key === "End") setSplit(SPLIT_MAX, true);
    else return;
    event.preventDefault();
  });

  function applySettings() {
    host.classList.toggle("serif", settings.style === "serif");
    host.classList.toggle("monospace", settings["editor-font"] === "monospace");
    setSplit(settings.split ?? split, false);
  }
  document.addEventListener("legible-settings", (event) => {
    settings = event.detail;
    applySettings();
  });

  // --- Loading -------------------------------------------------------------------------------

  let editor = null; // the EditorView, once loaded
  let loading = null; // the promise of it
  let gotSource = null;
  let diskWhileLoading = null; // the text from disk, if it changed while the editor loaded

  function loadScript(src) {
    return new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = src;
      script.onload = resolve;
      script.onerror = () => reject(new Error(`${src} didn't load`));
      document.head.append(script);
    });
  }

  function ensureEditor() {
    loading ??= (async () => {
      const source = new Promise((resolve) => (gotSource = resolve));
      await loadScript("/js/codemirror.min.js");
      send("source");
      const text = await source;
      editor = build(diskWhileLoading ?? text);
      diskWhileLoading = null;
      return editor;
    })();
    return loading;
  }

  // --- The editor ----------------------------------------------------------------------------

  let version = 0;
  let baseline = null; // the text last loaded or saved (a CodeMirror Text)
  // The text of each version handed to a save, until that save is over: version → Text, oldest
  // first. Only these can be marked saved; edits keep nothing.
  const saving = new Map();
  let dirty = false;
  let applyingDisk = false; // while fromDisk's change goes in: it's saved text, not an edit
  let frozen = false; // for a close's save: the text takes no edits until the close is decided
  let lock = null; // the compartment that makes the text read-only while frozen
  let diskVersion = -1; // the version fromDisk made, whose preview says "Updated"

  function build(text) {
    const { state, view, commands, language, search, markdown, highlight } = window.CM;
    const { tags } = highlight;
    lock = new state.Compartment();
    const markdownStyle = language.HighlightStyle.define([
      { tag: tags.heading1, class: "md-heading md-heading-1" },
      { tag: tags.heading2, class: "md-heading md-heading-2" },
      { tag: tags.heading3, class: "md-heading md-heading-3" },
      { tag: [tags.heading4, tags.heading5, tags.heading6], class: "md-heading" },
      { tag: tags.processingInstruction, class: "md-mark" },
      { tag: tags.strong, class: "md-strong" },
      { tag: tags.emphasis, class: "md-emphasis" },
      { tag: tags.strikethrough, class: "md-strike" },
      { tag: tags.link, class: "md-link" },
      { tag: tags.url, class: "md-url" },
      { tag: tags.monospace, class: "md-code" },
      { tag: tags.quote, class: "md-quote" },
      { tag: [tags.meta, tags.comment, tags.contentSeparator], class: "md-meta" },
    ]);
    const keys = view.keymap.of([
      { key: "Mod-b", run: wrapWith("**") },
      { key: "Mod-i", run: wrapWith("*") },
      { key: "Mod-k", run: makeLink },
      { key: "Mod-1", run: heading(1) },
      { key: "Mod-2", run: heading(2) },
      { key: "Mod-3", run: heading(3) },
      { key: "Mod-0", run: heading(0) },
      { key: "Mod-h", run: openReplace },
      ...commands.defaultKeymap,
      ...commands.historyKeymap,
      ...search.searchKeymap,
    ]);
    const editorView = new view.EditorView({
      root: shadow,
      parent: mount,
      state: state.EditorState.create({
        doc: text,
        extensions: [
          commands.history(),
          view.drawSelection(),
          state.EditorState.phrases.of(phrases()),
          markdown.markdown({ extensions: [frontMatter(tags)] }),
          language.syntaxHighlighting(markdownStyle),
          view.EditorView.lineWrapping,
          search.search({ top: true }),
          keys,
          view.EditorView.contentAttributes.of({
            spellcheck: "true",
            lang: root.lang || "en",
            "aria-label": s("editor"),
          }),
          lock.of([]),
          // Frozen, nothing changes the text but the file from disk (a change saved, not an edit).
          state.EditorState.transactionFilter.of((tr) => (frozen && tr.docChanged && !applyingDisk ? [] : tr)),
          view.EditorView.updateListener.of((update) => {
            if (update.docChanged) changed(update.state.doc);
          }),
        ],
      }),
    });
    baseline = editorView.state.doc;
    setTimeout(() => document.dispatchEvent(new CustomEvent("legible-words", { detail: countWords(text) })));
    editorView.scrollDOM.addEventListener("scroll", () => editorScrolled(), { passive: true });
    return editorView;
  }

  // CodeMirror's own words (its find and replace panel, go to line, what a screen reader is
  // told), each as Legible's string (locales/en/window.ftl, chrome-editor-*).
  const PHRASES = {
    Find: "editor-find",
    Replace: "editor-replace-with",
    next: "editor-next",
    previous: "editor-previous",
    all: "editor-select-all",
    "match case": "editor-match-case",
    regexp: "editor-regexp",
    "by word": "editor-by-word",
    replace: "editor-replace",
    "replace all": "editor-replace-all",
    close: "editor-close",
    "current match": "editor-current-match",
    "on line": "editor-on-line",
    "replaced match on line $": "editor-replaced-one",
    "replaced $ matches": "editor-replaced-many",
    "Go to line": "editor-go-to-line",
    go: "editor-go",
    "Control character": "editor-control-character",
    "Selection deleted": "editor-selection-deleted",
  };

  function phrases() {
    return Object.fromEntries(Object.entries(PHRASES).map(([phrase, key]) => [phrase, s(key)]));
  }

  /** Front matter as core reads it (legible_core's frontmatter): YAML between `---` lines or
      TOML between `+++` lines at the very top, after a BOM, its first line a key. Coloured as
      metadata, where Markdown alone would take `key: value` above the closing `---` for a
      heading. (Core also asks that it parse as a mapping; colouring doesn't need to.) */
  function frontMatter(tags) {
    const FENCE = /^﻿?(---|\+\+\+)[ \t]*$/;
    const KEY = /^[\p{L}\p{N}_"'-][^:=]*[:=]/u;
    return {
      defineNodes: [{ name: "FrontMatter", block: true, style: tags.meta }],
      parseBlock: [
        {
          name: "FrontMatter",
          before: "HorizontalRule",
          parse(cx, line) {
            const open = cx.lineStart === 0 && FENCE.exec(line.text);
            if (!open) return false;
            const head = cx.input.read(0, Math.min(cx.input.length, 65536)).split("\n");
            const close = head.findIndex((text, i) => i > 0 && text.replace(/\r$/, "").trimEnd() === open[1]);
            if (close < 2 || !KEY.test(head[1])) return false;
            let end = 0;
            for (let i = 0; i <= close; i++) end += head[i].length + 1;
            for (let i = 0; i <= close; i++) cx.nextLine();
            cx.addElement(cx.elt("FrontMatter", 0, Math.min(end - 1, cx.input.length)));
            return true;
          },
        },
      ],
    };
  }

  // The text's words for the status bar (chrome.js): Markdown's marks aren't words, and a link
  // counts as its text. Counted as the text changes, a moment after typing stops.
  const WORDS_DELAY_MS = 300;
  const segmenter = new Intl.Segmenter(root.lang || undefined, { granularity: "word" });
  let wordsTimer = 0;

  function countWords(text) {
    const plain = text
      .replace(/^(---|\+\+\+)\r?\n[\s\S]*?\r?\n\1[ \t]*(\r?\n|$)/, "")
      .replace(/<!--[\s\S]*?-->/g, " ")
      .replace(/<[^>]+>/g, " ")
      .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/https?:\/\/\S+/g, "address");
    let count = 0;
    for (const segment of segmenter.segment(plain)) if (segment.isWordLike) count++;
    return count;
  }

  function sendWords() {
    clearTimeout(wordsTimer);
    wordsTimer = setTimeout(() => {
      if (editor) document.dispatchEvent(new CustomEvent("legible-words", { detail: countWords(editor.state.doc.toString()) }));
    }, WORDS_DELAY_MS);
  }

  function changed(text) {
    sendWords();
    version++;
    if (applyingDisk) return;
    const unsaved = !text.eq(baseline);
    if (unsaved) sendText(text);
    setDirty(unsaved);
    // Typing ends Compare: the preview is the text's again, and the bar still asks.
    if (comparing) {
      endCompare();
      if (conflict) askConflict();
    }
    scheduleRender();
  }

  /** The file as it now is on disk becomes the text and what's saved. Only what differs is
      replaced, so the cursor and the scroll stay where they are; undo can take it back. The
      same text still renders again: a picture it shows may have changed. */
  function fromDisk(text) {
    const current = editor.state.doc.toString();
    const shortest = Math.min(current.length, text.length);
    let start = 0;
    while (start < shortest && current.charCodeAt(start) === text.charCodeAt(start)) start++;
    let end = 0;
    while (
      end < shortest - start &&
      current.charCodeAt(current.length - 1 - end) === text.charCodeAt(text.length - 1 - end)
    ) end++;
    applyingDisk = true;
    try {
      if (current !== text) {
        editor.dispatch({
          changes: { from: start, to: current.length - end, insert: text.slice(start, text.length - end) },
        });
      } else {
        version++;
      }
    } finally {
      applyingDisk = false;
    }
    baseline = editor.state.doc;
    setDirty(false);
    diskVersion = version;
    // The text is the file's now: a held change's question is answered (a raced save's isn't:
    // both its versions are still kept).
    if (conflict && conflict.kind !== "raced") {
      endCompare();
      hideBar();
    }
    sendRender();
  }

  /** The window keeps the newest unsaved text as it comes, for Windows ending the session: never
      behind the preview's delay, and sent before the word that it's unsaved, so the window never
      stands unsaved without the text (a session's end asked between the two kept no copy). */
  function sendText(text) {
    send(`text\n${version}\n${text.toString()}`);
  }

  function setDirty(next) {
    if (next === dirty) return;
    dirty = next;
    send(`dirty\n${dirty}`);
    document.dispatchEvent(new CustomEvent("legible-dirty", { detail: dirty }));
  }

  // --- Shortcuts -----------------------------------------------------------------------------

  /** Ctrl+B, Ctrl+I: wrap each selection in `mark`, or take it off if it's there. */
  function wrapWith(mark) {
    return (target) => {
      const { state } = target;
      const n = mark.length;
      const changes = state.changeByRange((range) => {
        const before = state.sliceDoc(range.from - n, range.from);
        const after = state.sliceDoc(range.to, range.to + n);
        const inner = state.sliceDoc(range.from, range.to);
        // Already wrapped, outside the selection or inside it: unwrap.
        if (before === mark && after === mark && !(mark === "*" && isBold(state, range))) {
          return {
            changes: [
              { from: range.from - n, to: range.from },
              { from: range.to, to: range.to + n },
            ],
            range: window.CM.state.EditorSelection.range(range.from - n, range.to - n),
          };
        }
        if (inner.length >= 2 * n && inner.startsWith(mark) && inner.endsWith(mark)) {
          return {
            changes: { from: range.from, to: range.to, insert: inner.slice(n, -n) },
            range: window.CM.state.EditorSelection.range(range.from, range.to - 2 * n),
          };
        }
        return {
          changes: { from: range.from, to: range.to, insert: mark + inner + mark },
          range: window.CM.state.EditorSelection.range(range.from + n, range.to + n),
        };
      });
      target.dispatch(state.update(changes, { scrollIntoView: true, userEvent: "input" }));
      return true;
    };
  }

  // `**bold**` round a selection isn't `*` round it.
  function isBold(state, range) {
    return state.sliceDoc(range.from - 2, range.from) === "**" && state.sliceDoc(range.to, range.to + 2) === "**";
  }

  /** Ctrl+K: the selection becomes a link's text, `url` selected to type the address over. */
  function makeLink(target) {
    const { state } = target;
    const { EditorSelection } = window.CM.state;
    const changes = state.changeByRange((range) => {
      const text = state.sliceDoc(range.from, range.to);
      const insert = `[${text}](url)`;
      const urlFrom = range.from + text.length + 3;
      return {
        changes: { from: range.from, to: range.to, insert },
        range: text ? EditorSelection.range(urlFrom, urlFrom + 3) : EditorSelection.cursor(range.from + 1),
      };
    });
    target.dispatch(state.update(changes, { scrollIntoView: true, userEvent: "input" }));
    return true;
  }

  /** Ctrl+1, 2, 3: the selection's lines become headings of that level; Ctrl+0 plain text. */
  function heading(level) {
    return (target) => {
      const { state } = target;
      const changes = [];
      let last = -1;
      for (const range of state.selection.ranges) {
        for (let pos = range.from; pos <= range.to; ) {
          const line = state.doc.lineAt(pos);
          if (line.number !== last) {
            last = line.number;
            const marks = /^#{1,6}[ \t]+|^#{1,6}$/.exec(line.text)?.[0] ?? "";
            const insert = level ? `${"#".repeat(level)} ` : "";
            if (marks !== insert) changes.push({ from: line.from, to: line.from + marks.length, insert });
          }
          pos = line.to + 1;
        }
      }
      if (changes.length) target.dispatch({ changes, userEvent: "input" });
      return true;
    };
  }

  /** Ctrl+H: find and replace, with the replace field ready. */
  function openReplace(target) {
    window.CM.search.openSearchPanel(target);
    requestAnimationFrame(() => {
      const field = shadow.querySelector(".cm-search input[name=replace]");
      field?.focus();
      field?.select();
    });
    return true;
  }

  // --- The preview ---------------------------------------------------------------------------

  let renderTimer = 0;
  let sentVersion = -1; // the last version sent to be rendered
  // The version #document shows: -1 is the file as it opened, rendered without its lines.
  let shownVersion = -1;
  // The version whose preview is ready: its diagrams drawn, its code highlighted.
  let preparedVersion = -1;

  function scheduleRender() {
    clearTimeout(renderTimer);
    renderTimer = setTimeout(sendRender, RENDER_DELAY_MS);
  }

  /** Sends the text to be rendered now, if the preview isn't of it already. */
  function sendRender() {
    clearTimeout(renderTimer);
    renderTimer = 0;
    if (!editor || version === shownVersion || version === sentVersion) return;
    sentVersion = version;
    send(`render\n${version}\n${editor.state.doc.toString()}`);
  }

  /** The window's rendering of `rendered` (a version): shown only if it's still the text's. */
  function preview(rendered, html, info) {
    // While comparing, the preview is the version on disk: the text's render waits for the end.
    if (rendered !== version || comparing) return;
    // The drawn diagrams, by source, to move into the new preview rather than draw again.
    const drawn = new Map();
    // Only one drawn in every theme: one whose drawing was cut short would never be finished.
    for (const diagram of doc.querySelectorAll(".diagram")) {
      if (!window.LegibleDoc?.isDrawn(diagram)) continue;
      const list = drawn.get(diagram.dataset.source) ?? [];
      list.push(diagram);
      drawn.set(diagram.dataset.source, list);
    }
    const scroller = previewScroller();
    const top = scroller.scrollTop;
    doc.innerHTML = html;
    for (const fresh of doc.querySelectorAll("div.mermaid[data-source]")) {
      const old = drawn.get(fresh.dataset.source)?.shift();
      if (!old) continue;
      old.dataset.sourcepos = fresh.dataset.sourcepos ?? "";
      fresh.replaceWith(old);
    }
    shownVersion = rendered;
    scroller.scrollTop = top;
    // The page's render is this one now, and its approved pictures show in it.
    window.LegibleChrome?.adopt?.(info);
    const current = () => shownVersion === rendered && version === rendered;
    window.LegibleDoc?.releaseSheets?.();
    document.dispatchEvent(new CustomEvent("legible-document-changed"));
    const prepared = window.LegibleDoc?.prepare(doc, { current }) ?? Promise.resolve();
    prepared.then(() => {
      window.LegibleDoc?.releaseSheets?.();
      if (!current()) return;
      preparedVersion = rendered;
      blocks = null;
      if (mode === "split") syncPreview();
      if (rendered === diskVersion) window.LegibleChrome?.updated?.();
    });
  }

  // --- Scroll sync ---------------------------------------------------------------------------

  // The rendered blocks with the lines they came from, in document order.
  let blocks = null;

  function previewScroller() {
    return mode === "split" ? doc : document.scrollingElement;
  }

  function sourceBlocks() {
    if (blocks) return blocks;
    blocks = [];
    for (const el of doc.querySelectorAll("[data-sourcepos]")) {
      const m = /^(\d+):\d+-(\d+):\d+$/.exec(el.dataset.sourcepos);
      if (!m) continue;
      const start = Number(m[1]);
      const end = Math.max(start, Number(m[2]));
      blocks.push({ el, start, end });
    }
    return blocks;
  }

  /** For line `line`: the smallest block that holds it; or, for a line between blocks (a blank
      one), the blocks either side of it. */
  function blocksForLine(line) {
    let best = null;
    let before = null;
    let after = null;
    for (const block of sourceBlocks()) {
      if (block.start <= line && line <= block.end) {
        if (!best || block.end - block.start <= best.end - best.start) best = block;
      } else if (block.end < line) {
        if (!before || block.end >= before.end) before = block;
      } else if (!after || block.start < after.start) {
        after = block;
      }
    }
    return { best, before, after };
  }

  function topIn(scroller, el) {
    return el.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop;
  }

  // The document's pane, scrolled to follow the text: its own scroll event, when it comes, is
  // that and nothing more. The text's pane is scrolled by CodeMirror (which measures first), so
  // its scroll events are let be for a few frames instead.
  let previewEcho = null;
  let quietFrames = 0;

  function editorScrolled() {
    if (mode !== "split" || !editor || quietFrames > 0) return;
    syncPreview();
  }

  /** The preview to the text's scroll: the same line at the top of each. */
  function syncPreview() {
    if (!editor || shownVersion !== version) return;
    const { line, within } = editorTop();
    const target = editor.scrollDOM.scrollTop <= 0 ? 0 : previewTopFor(line + within);
    if (target === null) return;
    const scroller = previewScroller();
    const max = scroller.scrollHeight - scroller.clientHeight;
    const next = Math.round(Math.min(max, Math.max(0, target)));
    if (Math.abs(scroller.scrollTop - next) < 2) return;
    previewEcho = next;
    scroller.scrollTop = next;
  }

  // CodeMirror measures heights from the top of the text; the scroller's own scrollTop counts
  // from above its padding.
  const textOffset = () =>
    editor.documentTop - editor.scrollDOM.getBoundingClientRect().top + editor.scrollDOM.scrollTop;

  /** The line at the top of the text's pane, and how far into it (0 to 1). */
  function editorTop() {
    const height = Math.max(0, editor.scrollDOM.scrollTop - textOffset());
    const block = editor.lineBlockAtHeight(height);
    const within = block.height ? Math.min(1, Math.max(0, (height - block.top) / block.height)) : 0;
    return { line: editor.state.doc.lineAt(block.from).number, within };
  }

  /** Puts line `line` at the top of the text's pane, by CodeMirror, which measures the lines on
      the way (their heights until then are estimates). `quiet`: the scroll follows the
      document, and mustn't be followed back. */
  function scrollEditorToLine(line, quiet = false) {
    const whole = Math.min(editor.state.doc.lines, Math.max(1, Math.floor(line)));
    const { EditorView } = window.CM.view;
    if (quiet) {
      quietFrames = 3;
      const count = () => {
        if (--quietFrames > 0) requestAnimationFrame(count);
      };
      requestAnimationFrame(count);
    }
    const effect = EditorView.scrollIntoView(editor.state.doc.line(whole).from, { y: "start", yMargin: 0 });
    editor.dispatch({ effects: effect });
  }

  /** Where the preview shows (fractional) line `line` at its top. */
  function previewTopFor(line) {
    const { best, before, after } = blocksForLine(Math.floor(line));
    const scroller = previewScroller();
    const bottomOf = (block) => topIn(scroller, block.el) + block.el.getBoundingClientRect().height;
    let top;
    if (best) {
      const span = best.end - best.start + 1;
      const fraction = Math.min(1, Math.max(0, (line - best.start) / span));
      top = topIn(scroller, best.el) + fraction * best.el.getBoundingClientRect().height;
    } else if (before && after) {
      // Between two blocks: as far between the one's bottom and the other's top.
      const fraction = Math.min(1, Math.max(0, (line - before.end) / (after.start - before.end)));
      top = bottomOf(before) + fraction * (topIn(scroller, after.el) - bottomOf(before));
    } else if (after) {
      top = topIn(scroller, after.el);
    } else if (before) {
      top = bottomOf(before);
    } else {
      return null;
    }
    return top - 8;
  }

  function previewScrolled() {
    if (mode !== "split" || !editor || shownVersion !== version) return;
    const scroller = previewScroller();
    if (previewEcho !== null && Math.abs(scroller.scrollTop - previewEcho) < 2) {
      previewEcho = null;
      return;
    }
    previewEcho = null;
    if (scroller.scrollTop <= 0) return scrollEditorToLine(1, true);
    // The block at the preview's top (the smallest there), or the first below it.
    const edge = scroller.getBoundingClientRect().top + 8;
    let found = null;
    for (const block of sourceBlocks()) {
      const rect = block.el.getBoundingClientRect();
      if (rect.height === 0) continue;
      if (rect.top <= edge && rect.bottom > edge) {
        if (!found || block.end - block.start <= found.block.end - found.block.start) found = { block, rect };
      } else if (rect.top > edge && !found) {
        found = { block, rect };
        break;
      }
    }
    if (!found) return;
    const { block, rect } = found;
    const fraction = rect.height ? Math.min(1, Math.max(0, (edge - rect.top) / rect.height)) : 0;
    scrollEditorToLine(block.start + fraction * (block.end - block.start + 1), true);
  }

  doc.addEventListener("scroll", () => previewScrolled(), { passive: true });

  // --- Modes ---------------------------------------------------------------------------------

  // The last mode asked for wins, however long the editor takes to load for an earlier one.
  let modeRequest = 0;

  async function setMode(next) {
    if (!MODES.includes(next)) return;
    if (next !== "view" && !editable) return;
    const request = ++modeRequest;
    if (next !== "view") {
      try {
        await ensureEditor();
      } catch (error) {
        console.warn("Legible: the editor didn't load:", error);
        return;
      }
    }
    if (request !== modeRequest || next === mode) return;
    const previous = mode;
    // The line at the top, kept across the change.
    const line = previous === "view" ? null : topLine();
    mode = next;
    root.classList.toggle("mode-split", mode === "split");
    root.classList.toggle("mode-edit", mode === "edit");
    send(`mode\n${mode}`);
    document.dispatchEvent(new CustomEvent("legible-mode", { detail: mode }));
    if (mode === "view") {
      // The document shows the latest text as soon as it's rendered.
      sendRender();
      return;
    }
    editor.requestMeasure();
    // Once the new layout is measured. The promise settles after this, so a caller that waits
    // for it finds the change done, the editor's focus included.
    await new Promise((resolve) =>
      requestAnimationFrame(() => {
        if (line) scrollEditorToLine(line);
        if (mode === "split") {
          sendRender();
          syncPreview();
        }
        if (previous === "view") editor.focus();
        resolve();
      }),
    );
  }

  function topLine() {
    return editor ? editorTop().line : null;
  }

  function cycle() {
    setMode(MODES[(MODES.indexOf(mode) + 1) % MODES.length]);
  }

  document.addEventListener(
    "keydown",
    (event) => {
      const ctrl = event.ctrlKey && !event.altKey && !event.metaKey;
      if (ctrl && !event.shiftKey && (event.key === "e" || event.key === "E")) {
        event.preventDefault();
        event.stopPropagation();
        if (editable) cycle();
      } else if (ctrl && (event.key === "s" || event.key === "S")) {
        // Save, or Save a copy; on a file that can't be saved where it is, the window offers a
        // copy for either.
        event.preventDefault();
        event.stopPropagation();
        send(event.shiftKey ? "save-copy" : "save");
      }
    },
    true,
  );

  // --- The conflict bar ----------------------------------------------------------------------

  // The file changed on disk while the text has unsaved edits (held by the window, or found by
  // a save), or went; or it was saved at the same moment as a save, and both versions are kept. The reader chooses, for the newest: the window ignores an answer for an
  // older one. The bar goes under the chrome's own (chrome.js stacks it: LegibleChrome.layout).
  const bar = document.createElement("div");
  bar.id = "conflict-bar";
  bar.setAttribute("role", "alert");
  bar.hidden = true;
  document.body.append(bar);
  let conflict = null; // {kind, id, text}, the newest
  let comparing = false; // the preview shows the version on disk

  function barButton(key, onclick) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "tool wide-button";
    const label = document.createElement("span");
    label.textContent = s(key);
    button.append(label);
    button.addEventListener("click", onclick);
    return button;
  }

  function showBar(text, buttons) {
    const message = document.createElement("span");
    message.className = "conflict-text";
    message.textContent = text;
    bar.replaceChildren(message, ...buttons);
    bar.hidden = false;
    layoutBar();
  }

  function hideBar() {
    conflict = null;
    bar.hidden = true;
    bar.replaceChildren();
    layoutBar();
  }

  function layoutBar() {
    window.LegibleChrome?.layout?.();
  }

  /** The window's message for answer `name` (keep, take or compare) to the current conflict. */
  function answer(name) {
    const { kind, id } = conflict;
    if (kind === "raced") return `raced\n${name}\n${id}`;
    return `${name === "take" ? "reload" : name}\n${id}`;
  }

  function askConflict() {
    const { kind, text } = conflict;
    const keep = barButton("conflict-keep", () => {
      // Frozen for a close or a take, the text's question waits: no answer goes.
      if (frozen) return;
      send(answer("keep"));
      endCompare();
      hideBar();
    });
    // A file that went has nothing to take or compare: keep the text (saving puts the file
    // back), or close (which asks about the edits).
    if (kind === "deleted") return showBar(text, [keep, barButton("close", () => send("close"))]);
    // Comparing, the bar says so, and offers the way back to the text's own preview.
    const compareOrBack = comparing
      ? barButton("conflict-stop-comparing", () => {
          endCompare();
          askConflict();
        })
      : barButton("conflict-compare", () => send(answer("compare")));
    const raced = kind === "raced";
    const comparingText = s(raced ? "conflict-comparing-raced" : "conflict-comparing");
    // In one pass, so a "{comparing}" in the text (a file's name) is the text's own.
    const parts = { text, comparing: comparingText };
    const shown = comparing
      ? s("conflict-with-comparing").replace(/\{(text|comparing)\}/g, (_, part) => parts[part])
      : text;
    showBar(shown, [
      keep,
      barButton(raced ? "conflict-take-other" : "conflict-take", confirmTake),
      compareOrBack,
    ]);
  }

  /** Use the file discards the text (or, for a raced save, Use the other version puts the other
      version over it): asked first. */
  function confirmTake() {
    const raced = conflict.kind === "raced";
    showBar(s(raced ? "conflict-take-raced" : "conflict-take-confirm"), [
      barButton(raced ? "conflict-take-other" : "conflict-take", () => {
        if (frozen) return;
        // The take replaces the text as it is now: frozen before the answer goes, so nothing
        // typed after the confirmation is lost. The window unfreezes it once done.
        setFrozen(true);
        send(answer("take"));
        endCompare();
        hideBar();
      }),
      barButton("conflict-cancel", askConflict),
    ]);
  }

  /** The preview shows the text's own render again. */
  function endCompare() {
    if (!comparing) return;
    comparing = false;
    shownVersion = -2;
    sentVersion = -2;
    sendRender();
  }

  window.addEventListener("legible:external-change", (event) => {
    const { kind, id, text } = event.detail ?? {};
    endCompare();
    conflict = { kind: String(kind), id: Number(id), text: String(text ?? "") };
    askConflict();
  });

  /** Compare's answer: `html`, the version on disk, in the preview beside the text (Split). */
  async function compare(kind, id, html, info) {
    const current = () => conflict?.kind === kind && conflict.id === id;
    if (!current() || !editor) return false;
    comparing = true;
    if (mode !== "split") await setMode("split");
    if (!comparing || !current()) return false;
    doc.innerHTML = html;
    shownVersion = -2;
    blocks = null;
    window.LegibleChrome?.adopt?.(info);
    window.LegibleDoc?.releaseSheets?.();
    document.dispatchEvent(new CustomEvent("legible-document-changed"));
    const showing = () => comparing && current();
    await (window.LegibleDoc?.prepare(doc, { current: showing }) ?? Promise.resolve());
    window.LegibleDoc?.releaseSheets?.();
    if (showing()) askConflict();
    return showing();
  }

  /** Frozen for a close's save, or not. */
  function setFrozen(next) {
    frozen = next;
    if (!editor) return;
    const { state, view } = window.CM;
    const locked = next ? [state.EditorState.readOnly.of(true), view.EditorView.editable.of(false)] : [];
    editor.dispatch({ effects: lock.reconfigure(locked) });
  }

  // --- From the window, and for saving --------------------------------------------------------

  window.LegibleEditor = {
    /** The file's text, once, when the editor first opens. */
    load({ text }) {
      if (!gotSource) return;
      const resolve = gotSource;
      gotSource = null;
      resolve(String(text ?? ""));
    },
    preview(rendered, html, info) {
      preview(rendered, html, info);
    },
    /** The file as it now is on disk (live reload, or Use the file). */
    fromDisk(text) {
      text = String(text ?? "");
      if (editor) fromDisk(text);
      else diskWhileLoading = text;
    },
    setMode,
    mode: () => mode,
    /** The menu bar's Find and Replace: what Ctrl+F and Ctrl+H do in the text. */
    find() {
      if (!editor || mode === "view") return;
      editor.focus();
      window.CM.search.openSearchPanel(editor);
    },
    replace() {
      if (!editor || mode === "view" || !editable) return;
      editor.focus();
      openReplace(editor);
    },
    editable,
    /** Whether the text differs from what was loaded or last saved. */
    isDirty: () => dirty,
    /** The text and its version, for a save; `request` is sent back with them. The version is
        kept until the save says how it went: markSaved or saveAbandoned. */
    contents(request) {
      const text = editor ? editor.state.doc : null;
      if (text) {
        saving.delete(version);
        saving.set(version, text);
        while (saving.size > SAVES_IN_FLIGHT) saving.delete(saving.keys().next().value);
      }
      const result = { version, text: text ? text.toString() : null };
      if (request !== undefined && text) send(`contents\n${request}\n${version}\n${result.text}`);
      return result;
    },
    /** `saved` (a version) is what's now on disk: the text is unsaved only if it differs. Saves
        of it and of older versions are over (an older one finishing later can't undo it). */
    markSaved(saved) {
      const text = saving.get(saved);
      if (!text || !editor) return false;
      for (const kept of [...saving.keys()]) if (kept <= saved) saving.delete(kept);
      baseline = text;
      const unsaved = !editor.state.doc.eq(baseline);
      // Unsaved again (undone, during the save, back past what it wrote): its text wasn't sent
      // while it matched what was saved before.
      if (unsaved && !dirty) sendText(editor.state.doc);
      setDirty(unsaved);
      return true;
    },
    /** The save of `version` failed or was cancelled: nothing changes but what's kept. */
    saveAbandoned(abandoned) {
      return saving.delete(abandoned);
    },
    compare(kind, id, html, info) {
      return compare(String(kind), Number(id), String(html ?? ""), info);
    },
    /** For a close's save: the text takes no edits from now on... */
    freeze() {
      setFrozen(true);
    },
    /** ...says, once saved, whether anything is unsaved, frozen first (so nothing typed
        after the answer can be lost)... */
    closeCheck() {
      setFrozen(true);
      send(`clean\n${!dirty}`);
    },
    /** ...and takes them again if the window stays open. */
    unfreeze() {
      setFrozen(false);
    },
    isFrozen: () => frozen,
    /** Unsaved changes kept as Windows ended the session, offered back: `question` on the bar,
        with Restore and Discard. */
    offerRestore(question) {
      endCompare();
      conflict = null;
      showBar(String(question ?? ""), [
        barButton("restore", () => {
          hideBar();
          send("restore\nyes");
        }),
        barButton("discard", () => {
          hideBar();
          send("restore\nno");
        }),
      ]);
    },
    /** Restore's text: the editor opens (Split, from View) and it goes in as an edit of the
        file's, so it's unsaved, and Undo brings the file's back. */
    async restore(text) {
      if (mode === "view") await setMode("split");
      else await ensureEditor();
      if (!editor || frozen) return false;
      editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: String(text ?? "") } });
      send("restored");
      return true;
    },
    /** The conflict the bar asks about, and whether Compare shows (for the tests). */
    conflictState: () => ({ conflict, comparing, bar: !bar.hidden }),
    /** The CodeMirror view, for the tests. */
    view: () => editor,
    /** The words the status bar counts in `text` (for the tests). */
    countWords,
    /** Whether the preview is of the text as it is, and whether it's ready (for the tests). */
    previewState: () => ({ current: shownVersion === version, prepared: preparedVersion === version }),
  };

  applySettings();
})();
