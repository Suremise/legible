// The document, made ready to read: run once the page has loaded, on <main id="document">.
//
// - Tables: each goes in a frame that scrolls sideways when it's wide, a long one's header
//   sticks, and a column of numbers is right-aligned (unless the Markdown aligned it).
// - Code: highlighted (highlight.js, loaded only when a block names its language), with a Copy
//   button.
// - Images: a picture from the internet gets a placeholder until the user shows it; a missing
//   one shows its alt text.
// - Mermaid: drawn in a sandboxed frame, only when the page has a diagram, themed light or dark
//   like the page, with its source on a toggle; a diagram that fails shows its source and a
//   short message. Each is drawn light and dark, and the stylesheet shows the one the theme (or
//   print) needs.
//
// The page gives the strings, translated, as JSON in <script type="application/json"
// id="legible-doc-strings"> (legible_core::render::script_strings). `window.LegibleDoc.ready`
// settles once everything above is done; `LegibleDoc.prepare(root, {current})` does it again for
// new content (the editor's preview), writing nothing once `current()` says that content has
// been replaced. Each drawn diagram's stylesheet belongs to its figure:
// `LegibleDoc.releaseSheets()` drops those whose figures have left the page, and
// `LegibleDoc.isDrawn(div)` says whether a diagram is drawn in every theme.

(() => {
  "use strict";

  const strings = readStrings();

  function readStrings() {
    const el = document.getElementById("legible-doc-strings");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch {
      return {};
    }
  }

  function t(key) {
    return strings[key] ?? key;
  }

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  // Loads one of the vendored bundles (served as /js/<name>) once.
  const scripts = new Map();
  function loadScript(name) {
    if (!scripts.has(name)) {
      scripts.set(
        name,
        new Promise((resolve, reject) => {
          const script = el("script");
          script.src = `/js/${name}`;
          script.onload = resolve;
          script.onerror = () => reject(new Error(`${name} didn't load`));
          document.head.append(script);
        }),
      );
    }
    return scripts.get(name);
  }

  // Tables

  // A cell that reads as a number: 1,000, -2.5, (300), 45%, £1.2m, 3 kg, €12, +7.
  const NUMBER = /^[(+\-−]?\s*[$£€¥]?\s*\d[\d,.\s]*(?:\s*(?:%|[kmb]n?|[a-zA-Z]{1,3}))?\)?$/;
  // A table this long scrolls in its own frame, so its header can stick.
  const LONG_ROWS = 15;

  function prepareTables(root) {
    for (const table of root.querySelectorAll("table")) {
      if (table.closest("details.properties, .table-frame")) continue;
      const frame = el("div", "table-frame");
      table.replaceWith(frame);
      frame.append(table);
      if (table.tBodies[0]?.rows.length > LONG_ROWS) frame.classList.add("long");
      alignNumbers(table);
    }
  }

  function alignNumbers(table) {
    const rows = [...table.querySelectorAll(":scope > tbody > tr")];
    const head = table.tHead?.rows[0];
    const columns = Math.max(0, ...rows.map((r) => r.cells.length));
    for (let c = 0; c < columns; c++) {
      const cells = rows.map((r) => r.cells[c]).filter(Boolean);
      const filled = cells.filter((cell) => cell.textContent.trim() !== "");
      if (filled.length === 0) continue;
      if (!filled.every((cell) => NUMBER.test(cell.textContent.trim()))) continue;
      for (const cell of cells) cell.classList.add("num");
      head?.cells[c]?.classList.add("num");
    }
  }

  // Code

  async function prepareCode(root, current) {
    const blocks = [...root.querySelectorAll("pre > code")].filter(
      (code) => !code.closest(".diagram"),
    );
    for (const code of blocks) addCopyButton(code.parentElement);
    const named = blocks.filter((code) => /\blanguage-/.test(code.className));
    if (named.length === 0) return;
    try {
      await loadScript("highlight.min.js");
    } catch {
      return; // Plain code still reads.
    }
    if (!current()) return;
    for (const code of named) {
      const lang = code.className.match(/\blanguage-(\S+)/)[1];
      if (window.hljs.getLanguage(lang)) window.hljs.highlightElement(code);
    }
  }

  function addCopyButton(pre) {
    if (pre.parentElement.classList.contains("code-block")) return;
    const block = el("div", "code-block");
    pre.replaceWith(block);
    const button = el("button", "copy", t("copy"));
    button.type = "button";
    button.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(pre.textContent);
        button.textContent = t("copied");
      } catch {
        button.textContent = t("copy-failed");
      }
      button.classList.add("done");
      setTimeout(() => {
        button.textContent = t("copy");
        button.classList.remove("done");
      }, 1500);
    });
    block.append(pre, button);
  }

  // Links

  // A bare link (its text is its address) doesn't print its address twice.
  function markBareLinks(root) {
    for (const a of root.querySelectorAll("a[href]")) {
      const text = a.textContent.trim();
      const href = a.getAttribute("href");
      if (text === href || href.replace(/^(https?:\/\/|mailto:)/, "") === text) {
        a.classList.add("bare-link");
      }
    }
  }

  // Images

  function prepareImages(root) {
    for (const img of root.querySelectorAll("img")) {
      if (img.hasAttribute("data-remote-src")) {
        if (!img.nextElementSibling?.classList.contains("remote-image")) {
          const box = el("span", "remote-image", t("remote-image"));
          box.setAttribute("role", "img");
          box.setAttribute("aria-label", t("remote-image"));
          img.after(box);
        }
        // Shown later (src set from data-remote-src): watch for it failing then.
        img.addEventListener("error", () => markMissing(img));
      } else if (!img.hasAttribute("src")) {
        markMissing(img);
      } else if (img.complete && img.naturalWidth === 0) {
        markMissing(img);
      } else {
        img.addEventListener("error", () => markMissing(img));
      }
    }
  }

  function markMissing(img) {
    if (img.classList.contains("missing")) return;
    img.classList.add("missing");
    img.nextElementSibling?.classList.contains("remote-image") &&
      img.nextElementSibling.remove();
    const box = el("span", "missing-image");
    const alt = img.getAttribute("alt")?.trim();
    box.append(el("span", "alt", alt || t("missing-image")));
    box.setAttribute("role", "img");
    box.setAttribute("aria-label", alt || t("missing-image"));
    img.after(box);
  }

  // Mermaid

  // Each diagram is drawn in both themes up front, and the stylesheet shows the one the page's
  // theme calls for: so a change of theme needs no redrawing, and print (always light) never
  // waits on Mermaid.
  const THEMES = ["light", "dark"];

  // Mermaid runs in a frame of its own (mermaid-frame.html and .js), sandboxed with an opaque
  // origin: a bug in it, or a diagram's text that tricks it, can't reach this page or the
  // window's bridge to Legible. The page sends it one diagram at a time and takes back the SVG,
  // which it trusts no more than the document: diagramElement keeps only what a diagram needs.
  const FRAME_URL = "/doc/mermaid-frame.html";
  // How long a diagram may take, the frame's loading included, before it shows as failed
  // (scripts/doc-tests.mjs sets window.legibleDiagramTimeout to make it short).
  const TIMEOUT_MS = Number(window.legibleDiagramTimeout) || 20000;

  let diagramCount = 0;
  let frame = null;
  // The diagrams waiting for the frame, sent one after another.
  let line = Promise.resolve();

  function openFrame() {
    const iframe = el("iframe", "diagram-frame");
    iframe.setAttribute("sandbox", "allow-scripts");
    iframe.setAttribute("aria-hidden", "true");
    iframe.tabIndex = -1;
    iframe.src = FRAME_URL;
    const opened = { iframe, waiting: new Map() };
    opened.ready = new Promise((resolve) => (opened.loaded = resolve));
    document.body.append(iframe);
    return opened;
  }

  // A frame that stopped answering is closed, and the next diagram opens a new one.
  function closeFrame(closing, error) {
    for (const { reject } of closing.waiting.values()) reject(error);
    closing.waiting.clear();
    closing.iframe.remove();
    if (frame === closing) frame = null;
  }

  window.addEventListener("message", (event) => {
    if (!frame || event.source !== frame.iframe.contentWindow || event.origin !== "null") return;
    const data = event.data;
    if (data === null || typeof data !== "object") return;
    if (typeof data.ready === "boolean") {
      frame.loaded(data.ready);
      return;
    }
    // Only an answer to a request sent: never one the frame made up, or sent before it.
    const waiting = typeof data.id === "string" && frame.waiting.get(data.id);
    if (!waiting?.sent) return;
    if (data.svg !== undefined) waiting.resolve(data.svg);
    else if (typeof data.error === "string") waiting.reject(new Error(data.error));
  });

  // The SVG of one diagram, drawn by the frame.
  // Content replaced before its turn came: nothing is sent, and nothing shows.
  const STALE = new Error("replaced");

  function render(id, source, theme, current) {
    const turn = line.then(() => {
      if (!current()) throw STALE;
      return send({ id, source, theme, ...look(theme) });
    });
    line = turn.catch(() => {});
    return turn;
  }

  function send(request) {
    frame ??= openFrame();
    const to = frame;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => closeFrame(to, new Error("Mermaid took too long")),
        TIMEOUT_MS,
      );
      const done = (settle) => (value) => {
        clearTimeout(timer);
        to.waiting.delete(request.id);
        settle(value);
      };
      const waiting = { sent: false, resolve: done(resolve), reject: done(reject) };
      to.waiting.set(request.id, waiting);
      to.ready.then((ready) => {
        if (!ready) return closeFrame(to, new Error("Mermaid didn't load"));
        if (!to.waiting.has(request.id)) return; // closed meanwhile
        waiting.sent = true;
        // "*": an opaque origin has no name to send to. The page's frame-src keeps the frame
        // on Mermaid's page, whatever it navigates to.
        to.iframe.contentWindow?.postMessage(request, "*");
      });
    });
  }

  // What Mermaid needs of the page's look in a theme: its background, computed (Mermaid knows
  // nothing of light-dark()), and its font.
  function look(theme) {
    const probe = el("span");
    probe.style.colorScheme = theme;
    probe.style.backgroundColor = "var(--bg)";
    document.body.append(probe);
    const background = getComputedStyle(probe).backgroundColor;
    probe.remove();
    const font = getComputedStyle(document.documentElement).getPropertyValue("--sans").trim();
    return { background, font };
  }

  async function prepareDiagrams(root, current) {
    const diagrams = [...root.querySelectorAll("div.mermaid[data-source]")].filter(
      (div) => !div.classList.contains("diagram"),
    );
    if (diagrams.length === 0) return;
    for (const div of diagrams) buildDiagram(div);
    for (const theme of THEMES) {
      for (const div of diagrams) {
        if (!current()) return;
        if (div.classList.contains("failed")) continue;
        await drawDiagram(div, theme, current);
      }
    }
  }

  function buildDiagram(div) {
    div.classList.add("diagram");
    const source = div.dataset.source;
    const figures = THEMES.map((theme) => {
      const figure = el("div", `figure ${theme}`);
      figure.setAttribute("role", "img");
      return figure;
    });
    const details = el("details");
    details.append(el("summary", null, t("show-source")));
    const pre = el("pre");
    pre.append(el("code", "language-mermaid", source));
    details.append(pre);
    div.append(...figures, details);
  }

  // Each drawn figure's stylesheet, by sheet: dropped once its figure has left the page.
  const sheetFigures = new Map();

  async function drawDiagram(div, theme, current) {
    const id = `legible-diagram-${++diagramCount}`;
    try {
      const tree = await render(id, div.dataset.source, theme, current);
      if (!current()) return;
      const { svg, sheet } = diagramElement(tree, id);
      const figure = div.querySelector(`:scope > .figure.${theme}`);
      figure.replaceChildren(svg);
      if (sheet) {
        sheetFigures.set(sheet, figure);
        document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
      }
    } catch (e) {
      if (e !== STALE && current()) showFailure(div, e);
    }
  }

  /** Whether a diagram is drawn in every theme, and didn't fail: whole, so it can be moved into
      new content as it is (prepare leaves a built diagram be). */
  function isDrawn(div) {
    return (
      div.classList.contains("diagram") &&
      !div.classList.contains("failed") &&
      THEMES.every((theme) => div.querySelector(`:scope > .figure.${theme} > svg`))
    );
  }

  function releaseSheets() {
    const gone = new Set();
    for (const [sheet, figure] of sheetFigures) {
      if (!figure.isConnected) {
        gone.add(sheet);
        sheetFigures.delete(sheet);
      }
    }
    if (gone.size) document.adoptedStyleSheets = document.adoptedStyleSheets.filter((sheet) => !gone.has(sheet));
  }

  // What a diagram may hold: Mermaid's SVG, and the plain HTML of its labels.
  const SVG_NS = "http://www.w3.org/2000/svg";
  const HTML_NS = "http://www.w3.org/1999/xhtml";
  const XLINK_NS = "http://www.w3.org/1999/xlink";
  const SVG_ELEMENTS = new Set(
    (
      "svg g a defs symbol use marker path rect circle ellipse line polyline polygon text tspan " +
      "textPath title desc foreignObject linearGradient radialGradient stop pattern clipPath " +
      "mask filter feBlend feColorMatrix feComponentTransfer feComposite feDropShadow feFlood " +
      "feFuncA feFuncB feFuncG feFuncR feGaussianBlur feMerge feMergeNode feMorphology " +
      "feOffset style"
    ).split(" "),
  );
  const HTML_ELEMENTS = new Set(
    "div span p br b strong i em u s del code sub sup small mark".split(" "),
  );

  // Most nodes, and the deepest nesting, a diagram may have.
  const MAX_NODES = 100_000;
  const MAX_DEPTH = 100;

  // The diagram the frame sent for `id`, as the page's own SVG: only the elements above (HTML
  // only inside a <foreignObject>), no event handlers, links only within the diagram, ids only
  // the diagram's own, and styles that reach nothing outside it.
  //
  // The frame sends the SVG as a tree of plain data ({ns, name, attrs, children}, text as
  // strings), never as markup: the page builds each element itself, so nothing is parsed here.
  // Its styles go in through the CSSOM, which the page's CSP allows (it allows no <style>
  // elements or style attributes from markup): its <style> as a constructed sheet scoped by
  // the SVG's id, its style attributes property by property.
  function diagramElement(tree, id) {
    if (!isElement(tree) || tree.ns !== SVG_NS || tree.name !== "svg") {
      throw new Error("the frame sent something that isn't a diagram");
    }
    const ids = tree.attrs.filter(([ns, name]) => ns === null && name === "id");
    if (ids.length !== 1 || ids[0][2] !== id) {
      throw new Error("the frame sent another diagram");
    }
    const sheets = [];
    let count = 0;
    const build = (node, depth, inForeign) => {
      if (++count > MAX_NODES || depth > MAX_DEPTH) throw new Error("the diagram is too big");
      if (typeof node === "string") return document.createTextNode(node);
      if (!isElement(node) || !allowedElement(node, inForeign)) return null;
      if (node.name === "style") {
        sheets.push(node.children.filter((c) => typeof c === "string").join(""));
        return null;
      }
      const element = document.createElementNS(node.ns, node.name);
      for (const [ns, name, value] of node.attrs) {
        if (name === "style" && ns === null) element.style.cssText = value;
        else if (allowedAttribute(ns, name, value, id)) element.setAttributeNS(ns, name, value);
      }
      const foreign = inForeign || node.name === "foreignObject";
      for (const child of node.children) {
        const built = build(child, depth + 1, foreign);
        if (built) element.append(built);
      }
      return element;
    };
    const svg = build(tree, 0, false);
    const css = scopedRules(sheets.join("\n"), id);
    let sheet = null;
    if (css) {
      sheet = new CSSStyleSheet();
      sheet.replaceSync(css);
    }
    svg.removeAttribute("height");
    return { svg, sheet };
  }

  function isElement(node) {
    return (
      node !== null &&
      typeof node === "object" &&
      typeof node.ns === "string" &&
      typeof node.name === "string" &&
      Array.isArray(node.attrs) &&
      node.attrs.every(
        (a) =>
          Array.isArray(a) &&
          a.length === 3 &&
          (a[0] === null || typeof a[0] === "string") &&
          typeof a[1] === "string" &&
          typeof a[2] === "string",
      ) &&
      Array.isArray(node.children)
    );
  }

  function allowedElement(node, inForeign) {
    if (node.ns === SVG_NS) return SVG_ELEMENTS.has(node.name);
    return node.ns === HTML_NS && inForeign && HTML_ELEMENTS.has(node.name);
  }

  // An attribute's name as SVG and HTML write it (xlink:href, stroke-width, data-id).
  const NAME = /^(?:xlink:)?[a-zA-Z][\w.-]*$/;

  function allowedAttribute(ns, name, value, id) {
    if (!NAME.test(name)) return false;
    if (ns !== null && !(ns === XLINK_NS && name === "xlink:href")) return false;
    const local = name.toLowerCase().replace(/^xlink:/, "");
    if (local.startsWith("on")) return false;
    if (local === "href") return value.trim().startsWith("#");
    if (local === "id") return ownId(value, id);
    return true;
  }

  // The diagram's own ids start with its id, so they never take one of the page's.
  function ownId(value, id) {
    return value === id || (value.startsWith(id) && /^[-_]/.test(value.slice(id.length)));
  }

  // The diagram's style rules that only reach inside it (each selector starts at its id), and
  // its animations; nothing else.
  function scopedRules(css, id) {
    if (!css.trim()) return "";
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(css);
    const scope = new RegExp(`^#${id}(?![\\w-])`);
    const scoped = (selector) => scope.test(selector.trim()) && !/[~+]/.test(selector);
    return [...sheet.cssRules]
      .filter(
        (rule) =>
          rule instanceof CSSKeyframesRule ||
          (rule instanceof CSSStyleRule && rule.selectorText.split(",").every(scoped)),
      )
      .map((rule) => rule.cssText)
      .join("\n");
  }

  function showFailure(div, error) {
    div.classList.add("failed");
    for (const figure of div.querySelectorAll(":scope > .figure")) figure.replaceChildren();
    if (!div.querySelector(":scope > .diagram-error")) {
      div.prepend(el("p", "diagram-error", t("diagram-failed")));
    }
    const details = div.querySelector(":scope > details");
    if (details) details.open = true;
    console.warn("Legible: a diagram didn't render:", error);
  }

  async function prepare(root, { current = () => true } = {}) {
    prepareTables(root);
    prepareImages(root);
    markBareLinks(root);
    await Promise.all([prepareCode(root, current), prepareDiagrams(root, current)]);
  }

  function start() {
    const root = document.getElementById("document");
    if (!root) return Promise.resolve();
    return prepare(root);
  }

  const ready =
    document.readyState === "loading"
      ? new Promise((resolve) =>
          document.addEventListener("DOMContentLoaded", () => resolve(start()), { once: true }),
        )
      : start();

  window.LegibleDoc = { ready, prepare, releaseSheets, isDrawn };
})();
