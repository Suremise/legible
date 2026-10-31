// Tests of the window's chrome (web/chrome/chrome.js) in Edge in headless mode, the same engine
// as WebView2, on the window's real page and under its real CSP.
//
//   node scripts/chrome-tests.mjs
//
// The pages are written by legible-win's chrome-page example (legible_win::page::page, as a
// window gets it). They're served on 127.0.0.1, with the page's two origins
// (https://legible.localhost and https://legible-doc.localhost) rewritten to this server's own,
// so the CSP is the window's with only the origin changed. The server answers only GET
// requests for its own host and the paths it serves, and fails the run on anything else.
//
// A headless Edge of its own is driven over the DevTools protocol: no window, and no input to
// anything else (keys are dispatched into this headless page only). Every wait polls a
// condition; none sleeps for a time. It fails on any console error or CSP report the chrome
// causes. Mermaid's own inline styles while it lays a diagram out are refused by the CSP and
// counted, not failed (docs/ARCHITECTURE.md, "The window"). Nothing is fetched from the network.

import { spawn, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { FRAME_CSP, FRAME_PATH } from "./doc-harness.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const work = mkdtempSync(join(tmpdir(), "legible-chrome-tests-"));
const pages = join(work, "pages");
const docfiles = join(repo, "scripts", "doc-shots");
// A long document, written here: long enough that most of it waits off the screen to be laid
// out (web/chrome/window.css).
const longDoc = join(work, "long.md");
writeFileSync(
  longDoc,
  Array.from(
    { length: 80 },
    (_, i) =>
      `## Section ${i + 1}\n\n${"The quarter's figures, region by region, with the team's notes. ".repeat(6)}\n\n` +
      "- One point\n- Another point\n\n| Region | Sales |\n|---|---|\n| North | 1,200 |\n| South | 980 |\n",
  ).join("\n") + "\nThe last word: omega.\n",
);
// A document of thousands of blocks, which are laid out once it's drawn (web/chrome/chrome.js),
// except a list too big to lay out in one go.
const manyDoc = join(work, "many.md");
writeFileSync(
  manyDoc,
  "# Many blocks\n\n" +
    Array.from({ length: 2100 }, (_, i) => `Paragraph ${i + 1}.\n`).join("\n") +
    "\n" +
    Array.from({ length: 20100 }, (_, i) => `- Item ${i + 1}`).join("\n") +
    "\n\nThe last paragraph.\n",
);
const sources = [
  join(docfiles, "showcase.md"),
  join(repo, "scripts", "chrome-tests", "find.md"),
  join(repo, "scripts", "chrome-tests", "short.md"),
  join(repo, "scripts", "chrome-tests", "pictures.md"),
  join(repo, "scripts", "chrome-tests", "diagrams.md"),
  join(repo, "scripts", "chrome-tests", "notes-read-only.md"),
  longDoc,
  manyDoc,
];
// Each page's Markdown, for the stand-in window to give the editor.
const sourceOf = Object.fromEntries(sources.map((file) => [basename(file, ".md"), file]));

console.log("Writing the pages...");
execFileSync("cargo", ["run", "-q", "-p", "legible-win", "--example", "chrome-page", "--", pages, ...sources], {
  cwd: repo,
  stdio: "inherit",
});
// The editor's previews are rendered through core, as the window renders them. Built with the
// workspace's features (not -p legible-core's own), so it and the test suite's build don't each
// rebuild core for the other when they run side by side.
execFileSync("cargo", ["build", "-q", "--workspace", "--example", "editor-preview"], { cwd: repo, stdio: "inherit" });
const previewExe = join(repo, "target", "debug", "examples", "editor-preview.exe");

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
};

const problems = [];
let origin = "";
let host = "";

function serveFile(res, root, rel) {
  const file = resolve(root, decodeURIComponent(rel));
  if (!file.startsWith(root + "\\") || !existsSync(file) || !TYPES[extname(file)]) {
    res.writeHead(404).end();
    return;
  }
  res.writeHead(200, { "content-type": TYPES[extname(file)] }).end(readFileSync(file));
}

// Every request the server is sent, for the tests that prove one never was.
const requests = [];
const server = createServer((req, res) => {
  if (req.method !== "GET" || req.headers.host !== host) {
    problems.push(`unexpected request: ${req.method} ${req.headers.host} ${req.url}`);
    res.writeHead(403).end();
    return;
  }
  requests.push(req.url);
  const path = new URL(req.url, origin).pathname;
  if (path.startsWith("/page/")) {
    const name = path.slice(6);
    if (!/^[a-z-]+$/.test(name)) return res.writeHead(404).end();
    // The page's own origins, made this server's.
    const html = readFileSync(join(pages, `${name}.html`), "utf8")
      .replaceAll("https://legible-doc.localhost/", `${origin}/docfiles/`)
      .replaceAll("https://legible-doc.localhost", origin)
      .replaceAll("https://legible.localhost", origin);
    const csp = /http-equiv="Content-Security-Policy" content="([^"]+)"/.exec(html)[1].replaceAll("&#39;", "'");
    res.writeHead(200, { "content-type": "text/html; charset=utf-8", "content-security-policy": csp }).end(html);
  } else if (path === FRAME_PATH) {
    // Mermaid's frame, under its own CSP (as doc-tests serve it), so the diagrams are drawn.
    res.writeHead(200, { "content-type": "text/html; charset=utf-8", "content-security-policy": FRAME_CSP });
    res.end(readFileSync(join(repo, "web", "doc", "mermaid-frame.html")));
  } else if (path.startsWith("/doc/")) {
    serveFile(res, join(repo, "web", "doc"), path.slice(5));
  } else if (path.startsWith("/editor/")) {
    serveFile(res, join(repo, "web", "editor"), path.slice(8));
  } else if (path.startsWith("/chrome/")) {
    serveFile(res, join(repo, "web", "chrome"), path.slice(8));
  } else if (path.startsWith("/js/")) {
    serveFile(res, join(repo, "third_party", "js"), path.slice(4));
  } else if (path.startsWith("/docfiles/")) {
    serveFile(res, docfiles, path.slice(10));
  } else if (path === "/favicon.ico") {
    // Edge asks for one by itself.
    res.writeHead(204).end();
  } else {
    problems.push(`unexpected request: ${req.url}`);
    res.writeHead(404).end();
  }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
host = `127.0.0.1:${server.address().port}`;
origin = `http://${host}`;

// --- Edge --------------------------------------------------------------------------------------

function findEdge() {
  const candidates = [
    process.env["ProgramFiles(x86)"] && join(process.env["ProgramFiles(x86)"], "Microsoft", "Edge", "Application", "msedge.exe"),
    process.env.ProgramFiles && join(process.env.ProgramFiles, "Microsoft", "Edge", "Application", "msedge.exe"),
  ].filter(Boolean);
  const found = candidates.find((p) => existsSync(p));
  if (!found) throw new Error("Microsoft Edge isn't installed where expected");
  return found;
}

const profile = join(work, "profile");
const edge = spawn(
  findEdge(),
  [
    "--headless=new",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-sync",
    "--disable-background-networking",
    "--disable-component-update",
    "about:blank",
  ],
  { stdio: "ignore" },
);

/** Removes `dir`: true once it's gone, false while a file in it is still held open (or still
    being deleted: Windows can leave it listed for a moment). */
function removed(dir) {
  try {
    rmSync(dir, { recursive: true, force: true });
    return !existsSync(dir);
  } catch (e) {
    if (["EPERM", "EBUSY", "ENOTEMPTY"].includes(e.code)) return false;
    throw e;
  }
}

async function until(what, check, ms = 20000) {
  const end = Date.now() + ms;
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 25));
  }
}

const [port, browserPath] = await until("Edge's DevTools port", () => {
  try {
    const lines = readFileSync(join(profile, "DevToolsActivePort"), "utf8").trim().split("\n");
    return lines.length >= 2 ? lines : null;
  } catch {
    return null;
  }
});
const ws = new WebSocket(`ws://127.0.0.1:${port}${browserPath}`);
await new Promise((r, e) => {
  ws.onopen = r;
  ws.onerror = e;
});
let nextId = 0;
const pending = new Map();
const listeners = [];
ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id !== undefined && pending.has(msg.id)) {
    const { ok, reject } = pending.get(msg.id);
    pending.delete(msg.id);
    msg.error ? reject(new Error(msg.error.message)) : ok(msg.result);
  } else if (msg.method) {
    for (const l of listeners) l(msg);
  }
};
// Each call is bounded, so a page that hangs fails the run, naming the call, rather than stalling it.
const CALL_LIMIT_MS = 30000;
function send(method, params = {}, sessionId) {
  const id = ++nextId;
  ws.send(JSON.stringify({ id, method, params, sessionId }));
  return new Promise((ok, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`DevTools didn't answer ${method} ${JSON.stringify(params).slice(0, 120)}`));
    }, CALL_LIMIT_MS);
    pending.set(id, {
      ok: (value) => (clearTimeout(timer), ok(value)),
      reject: (error) => (clearTimeout(timer), reject(error)),
    });
  });
}

const { targetId } = await send("Target.createTarget", { url: "about:blank" });
const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
const cdp = (method, params) => send(method, params, sessionId);
await cdp("Page.enable");
await cdp("Runtime.enable");
await cdp("Log.enable");
await cdp("Emulation.setDeviceMetricsOverride", { width: 1100, height: 800, deviceScaleFactor: 1, mobile: false });
// The window's IPC, recorded: what the page would send the window.
await cdp("Page.addScriptToEvaluateOnNewDocument", {
  source: "window.sent = []; window.ipc = { postMessage: (m) => window.sent.push(m) };",
});

let errors = [];
let mermaidStyleReports = 0;
listeners.push((msg) => {
  if (msg.sessionId !== sessionId) return;
  if (msg.method === "Runtime.exceptionThrown") {
    errors.push(msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text);
  } else if (msg.method === "Log.entryAdded" && ["error", "warning"].includes(msg.params.entry.level)) {
    const { text, url = "" } = msg.params.entry;
    // Mermaid's layout pass, which document.js runs: an inline style refused. Counted. The
    // chrome's own files (/chrome/) must cause none.
    if (/Applying inline style violates/.test(text) && !url.includes("/chrome/")) mermaidStyleReports++;
    // The showcase's missing picture is missing on purpose.
    else if (/status of 404/.test(text) && url.includes("/docfiles/missing/")) return;
    else if (msg.params.entry.level === "error" || /Content Security Policy/.test(text)) errors.push(`${text} ${url}`);
  }
});

async function evaluate(expression) {
  const r = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
  return r.result.value;
}

// What the window gives each editor preview: its render's generation, and what its pictures
// may do.
let previewInfo = { generation: 0, remote: null };

let opened = null;
async function open(name, { dark = false } = {}) {
  opened = name;
  handled = 0;
  hold = false;
  held.length = 0;
  previewInfo = { generation: 0, remote: null };
  await cdp("Emulation.setEmulatedMedia", {
    media: "screen",
    features: [{ name: "prefers-color-scheme", value: dark ? "dark" : "light" }],
  });
  await cdp("Page.navigate", { url: `${origin}/page/${name}` });
  await until(`${name} to load`, () => evaluate("!!window.LegibleChrome?.ready").catch(() => false));
  await evaluate("LegibleChrome.ready");
}

/** A key pressed in this headless page (trusted, so Tab moves focus as it would). */
async function key(keyName, { code = keyName, keyCode = 0, ctrl = false, shift = false } = {}) {
  const modifiers = (ctrl ? 2 : 0) | (shift ? 8 : 0);
  const base = { key: keyName, code, modifiers, windowsVirtualKeyCode: keyCode };
  await cdp("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
  await cdp("Input.dispatchKeyEvent", { type: "keyUp", ...base });
}

// The stand-in window's state for the editor (pump, below).
let handled = 0;
let hold = false;
const held = [];

let failures = 0;
// A test this slow says how long it took, so a slow run can be accounted for.
const SLOW_MS = 5000;

async function test(name, body) {
  errors = [];
  const started = Date.now();
  const took = () => {
    const ms = Date.now() - started;
    return ms > SLOW_MS ? ` (${(ms / 1000).toFixed(1)} s)` : "";
  };
  try {
    await body();
    if (errors.length) throw new Error(`console: ${errors.join(" | ")}`);
    console.log(`  ok    ${name}${took()}`);
  } catch (error) {
    failures++;
    console.log(`  FAIL  ${name}${took()}: ${error.message}`);
  }
}
function expect(condition, what) {
  if (!condition) throw new Error(what);
}
const sent = () => evaluate("window.sent.slice()");

/** A page's document as rendered (inside <main id="document">), before the page prepared it. */
function renderedDocument(name) {
  const html = readFileSync(join(pages, `${name}.html`), "utf8");
  const open = /<main id="document"[^>]*>/.exec(html);
  const start = open.index + open[0].length;
  return html.slice(start, html.lastIndexOf("</main>")).replaceAll("https://legible-doc.localhost/", `${origin}/docfiles/`);
}
/** Live reload's replacement, as the window sends it; resolves once it has been applied. */
const replace = (generation, html) =>
  evaluate(`LegibleChrome.replaceDocument({ generation: ${generation}, html: ${JSON.stringify(html)} })`);
const headingTop = (text) =>
  evaluate(`[...document.querySelectorAll("#document h2")].find((h) => h.textContent === ${JSON.stringify(text)})?.getBoundingClientRect().top ?? null`);

// --- The tests ---------------------------------------------------------------------------------

console.log("Testing the chrome...");

await test("every toolbar button has a name and a tooltip, and the toolbar stays put", async () => {
  await open("showcase");
  const bad = await evaluate(`[...document.querySelectorAll("#toolbar button")]
    .filter((b) => !b.getAttribute("aria-label") || !b.title).map((b) => b.outerHTML)`);
  expect(bad.length === 0, `unnamed buttons: ${bad}`);
  const tips = await evaluate(`[...document.querySelectorAll("#toolbar button")].map((b) => b.title)`);
  expect(tips.includes("Find (Ctrl+F)") && tips.includes("Outline (Ctrl+Shift+O)"), `tooltips: ${tips}`);
  expect(tips.includes("Split (Ctrl+E cycles)") && tips.includes("Copy (Ctrl+Shift+C)"), `tooltips: ${tips}`);
  await evaluate("window.scrollTo(0, 3000)");
  const top = await evaluate("document.getElementById('toolbar').getBoundingClientRect().top");
  expect(top === 0, `the toolbar moved to ${top}`);
});

await test("the outline shows itself for a long document, not a short one, and Ctrl+Shift+O toggles it", async () => {
  await open("showcase");
  expect(await evaluate("!document.getElementById('outline').hidden"), "hidden for the long showcase");
  await open("short");
  expect(await evaluate("document.getElementById('outline').hidden"), "shown for a short note");
  await key("O", { code: "KeyO", keyCode: 79, ctrl: true, shift: true });
  expect(await evaluate("!document.getElementById('outline').hidden"), "Ctrl+Shift+O didn't open it");
  await key("O", { code: "KeyO", keyCode: 79, ctrl: true, shift: true });
  expect(await evaluate("document.getElementById('outline').hidden"), "Ctrl+Shift+O didn't shut it");
});

await test("an idle window asks for no more frames once it's ready", async () => {
  for (const name of ["short", "showcase"]) {
    await open(name);
    const asked = await evaluate(`new Promise((resolve) => {
      const original = window.requestAnimationFrame.bind(window);
      let count = 0;
      window.requestAnimationFrame = (f) => { count++; return original(f); };
      let frames = 0;
      const tick = () => (++frames < 12 ? original(tick) : resolve(count));
      original(tick);
    })`);
    expect(asked === 0, `${name}: ${asked} frames asked for while idle`);
  }
});

await test("a new outline setting overrides a hand toggle, which survives other changes", async () => {
  await open("short");
  const shown = () => evaluate("!document.getElementById('outline').hidden");
  await key("O", { code: "KeyO", keyCode: 79, ctrl: true, shift: true });
  await key("O", { code: "KeyO", keyCode: 79, ctrl: true, shift: true });
  await evaluate(`[...document.querySelectorAll('.menu-item')].find((m) => m.textContent.includes('Settings')).click()`);
  await evaluate(`document.querySelector('input[name=outline][value=always]').click()`);
  expect(await shown(), "Always from Settings didn't show it after a toggle");
  await key("Escape", { keyCode: 27 });
  await key("O", { code: "KeyO", keyCode: 79, ctrl: true, shift: true });
  const current = "JSON.parse(document.getElementById('legible-settings').textContent)";
  await evaluate(`LegibleChrome.apply({ ...${current}, outline: 'never' })`);
  expect(!(await shown()), "Never from another window didn't hide it");
  await key("O", { code: "KeyO", keyCode: 79, ctrl: true, shift: true });
  await evaluate(`LegibleChrome.apply({ ...${current}, outline: 'never', width: 'wide' })`);
  expect(await shown(), "an unrelated change undid the hand toggle");
});

await test("a failed save is shown wherever the change was made", async () => {
  await open("showcase");
  const noticeShows = (text) =>
    evaluate(`(() => { const n = document.getElementById('notice');
      return !n.hidden && n.getAttribute('role') === 'alert' && n.textContent.includes(${JSON.stringify(text)}); })()`);
  // From the toolbar, Settings never opened.
  await evaluate(`document.querySelector('[aria-label="Reading width and style"]').click()`);
  await evaluate(`[...document.querySelectorAll('.menu-item')].find((m) => m.textContent.includes('Wide')).click()`);
  await evaluate("LegibleChrome.saveFailed('Your settings could not be saved')");
  expect(await noticeShows("could not be saved"), "no notice with Settings never opened");
  // Settings opened, then closed.
  await evaluate(`[...document.querySelectorAll('.menu-item')].find((m) => m.textContent.includes('Settings')).click()`);
  await key("Escape", { keyCode: 27 });
  await evaluate("LegibleChrome.saveFailed('Saving failed again')");
  expect(await noticeShows("Saving failed again"), "no notice with Settings closed");
  // A save that works clears it.
  await evaluate("LegibleChrome.apply(JSON.parse(document.getElementById('legible-settings').textContent))");
  expect(await evaluate("document.getElementById('notice').hidden"), "the notice stayed after a good save");
});

await test("Enter on an outline entry scrolls to its heading, below the chrome, and focuses it", async () => {
  await open("showcase");
  await evaluate(`[...document.querySelectorAll("#outline a")].find((a) => a.textContent === "Diagrams").focus()`);
  await key("Enter", { keyCode: 13 });
  const settled = await until("the heading to come to rest below the toolbar", () =>
    evaluate(`(() => {
      const h = document.activeElement;
      const top = h.getBoundingClientRect().top;
      return h.tagName === "H2" && h.textContent === "Diagrams" && top >= 40 && top < 90 ? top : false;
    })()`),
  );
  expect(settled, "not settled");
  await until("the outline to mark it", () =>
    evaluate(`document.querySelector("#outline a[aria-current]")?.textContent === "Diagrams"`),
  );
});

await test("a long document: its end waits to be laid out, yet a jump, the place put back and find land", async () => {
  await open("long");
  // What's in a block is skipped until the block comes near: the last table's cells.
  const lastCell = `[...document.querySelectorAll("#document td")].at(-1)`;
  expect(!(await evaluate(`${lastCell}.checkVisibility({ contentVisibilityAuto: true })`)), "the end was laid out at once");
  // Far down, where the blocks between are laid out only as the page gets there: the heading
  // still comes to rest under the toolbar, and stays.
  const section = `[...document.querySelectorAll("#document h2")].find((h) => h.textContent === "Section 70")`;
  const at = (top) => `(() => { const t = ${section}.getBoundingClientRect().top; return ${top}; })()`;
  await evaluate(`[...document.querySelectorAll("#outline a")].find((a) => a.textContent === "Section 70").focus()`);
  await key("Enter", { keyCode: 13 });
  await until("the heading to come to rest below the toolbar", () => evaluate(at("t >= 40 && t < 90")));
  await evaluate("new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))");
  expect(await evaluate(at("t >= 40 && t < 90")), `it moved on: ${await evaluate(at("t"))}`);
  // From the top, back to it, as the window gives a place back.
  const id = await evaluate(`${section}.id`);
  await evaluate("window.scrollTo(0, 0)");
  await evaluate(`LegibleChrome.goTo({ id: ${JSON.stringify(id)}, offset: 0, fraction: 0 })`);
  await until("the reader back at the heading", () => evaluate(at("Math.abs(t - 40) < 2")));
  // Find, from the top, shows a match at the very end.
  await evaluate("window.scrollTo(0, 0)");
  await key("f", { code: "KeyF", keyCode: 70, ctrl: true });
  const count = await evaluate(`(() => { const f = document.querySelector('.find-field'); f.value = 'omega';
    f.dispatchEvent(new Event('input')); return document.querySelector('.find-count').textContent; })()`);
  expect(count === "1 of 1", `'omega': ${count}`);
  await until("the match to be shown below the chrome", () =>
    evaluate(`(() => { const r = [...CSS.highlights.get('legible-find-current')][0].getBoundingClientRect();
      return r.top >= document.getElementById('find').getBoundingClientRect().bottom && r.bottom <= innerHeight; })()`),
  );
  // What moved with the find bar isn't on <html>, where every element of the document would
  // inherit it: the page makes room by its own padding.
  expect((await evaluate(`document.documentElement.style.getPropertyValue("--chrome-top")`)) === "", "the layout is on <html>");
  expect((await evaluate("parseFloat(getComputedStyle(document.body).paddingTop)")) > 40, "the page made no room for the find bar");
  await key("Escape", { keyCode: 27 });
});

await test("a document of thousands of blocks: they're laid out once it's drawn, all but one too big", async () => {
  await open("many");
  const blocks = `[...document.querySelectorAll("#document > *")]`;
  const list = `document.querySelector("#document > ul")`;
  await until("every block but the list to be laid out, with no scrolling", () =>
    evaluate(`${blocks}.filter((b) => b.tagName !== "UL").every((b) => "laidOut" in b.dataset)`),
  );
  expect(await evaluate("window.scrollY === 0"), "the page scrolled");
  const last = `[...document.querySelectorAll("#document > p")].at(-1)`;
  expect(await evaluate(`${last}.checkVisibility({ contentVisibilityAuto: true })`), "the last paragraph waits");
  // The list waits still: its last item isn't laid out until the page gets near it.
  expect(!(await evaluate(`"laidOut" in ${list}.dataset`)), "the list was marked");
  expect(!(await evaluate(`${list}.lastElementChild.checkVisibility({ contentVisibilityAuto: true })`)), "the list was laid out");
});

await test("find: Ctrl+F, a match across inline elements, none across blocks, F3 and Esc", async () => {
  await open("find");
  await key("f", { code: "KeyF", keyCode: 70, ctrl: true });
  expect(await evaluate("document.activeElement.classList.contains('find-field')"), "the field isn't focused");
  const type = (text) => evaluate(`(() => { const f = document.querySelector('.find-field'); f.value = ${JSON.stringify(text)};
    f.dispatchEvent(new Event('input')); return document.querySelector('.find-count').textContent; })()`);
  expect((await type("bold")) === "1 of 1", "'bold' isn't one match");
  const across = await evaluate(`(() => { const r = [...CSS.highlights.get('legible-find')][0];
    return [r.toString(), r.startContainer !== r.endContainer]; })()`);
  expect(across[0] === "bold" && across[1], `the match: ${across}`);
  expect((await type("paragraph start")) === "No matches", "matched across two paragraphs");
  expect((await type("order received")) === "1 of 1", "the diagram's hidden copy or its SVG was searched");
  expect((await type("apple")) === "1 of 3", "three apples");
  await key("F3", { keyCode: 114 });
  expect((await evaluate("document.querySelector('.find-count').textContent")) === "2 of 3", "F3");
  await key("Enter", { keyCode: 13, shift: true });
  await key("Enter", { keyCode: 13, shift: true });
  expect((await evaluate("document.querySelector('.find-count').textContent")) === "3 of 3", "Shift+Enter wraps");
  await key("Escape", { keyCode: 27 });
  expect(await evaluate("document.getElementById('find').hidden && CSS.highlights.size === 0"), "Esc didn't close and clear");
  expect(await evaluate("document.activeElement.getAttribute('aria-label') === 'Find'"), "focus didn't go back");
});

await test("find opens a closed <details> round a match and shows it below the chrome", async () => {
  await open("find");
  await key("f", { code: "KeyF", keyCode: 70, ctrl: true });
  await evaluate(`(() => { window.scrollTo(0, 0); const f = document.querySelector('.find-field'); f.value = 'treasure';
    f.dispatchEvent(new Event('input')); })()`);
  expect(await evaluate("document.querySelector('details.properties').open"), "the properties didn't open");
  const top = await evaluate("[...CSS.highlights.get('legible-find-current')][0].getBoundingClientRect().top");
  const chrome = await evaluate("document.getElementById('find').getBoundingClientRect().bottom");
  expect(top >= chrome && top < 800, `the match is at ${top}, the chrome ends at ${chrome}`);
});

await test("find follows the document as it changes", async () => {
  await open("find");
  await key("f", { code: "KeyF", keyCode: 70, ctrl: true });
  await evaluate(`(() => { const f = document.querySelector('.find-field'); f.value = 'apple'; f.dispatchEvent(new Event('input')); })()`);
  await evaluate(`document.getElementById('document').append(Object.assign(document.createElement('p'), { textContent: 'apple' }))`);
  await until("the count to take in the new paragraph", () =>
    evaluate("document.querySelector('.find-count').textContent === '1 of 4'"),
  );
});

await test("zoom: 50% to 300%, Ctrl+0, Ctrl+wheel, and a new default zoom leaves this window's be", async () => {
  await open("showcase");
  const level = () => evaluate("document.querySelector('.zoom-level').textContent");
  for (let i = 0; i < 20; i++) await key("=", { code: "Equal", keyCode: 187, ctrl: true });
  expect((await level()) === "300%", `up to ${await level()}`);
  for (let i = 0; i < 20; i++) await key("-", { code: "Minus", keyCode: 189, ctrl: true });
  expect((await level()) === "50%", `down to ${await level()}`);
  await key("0", { code: "Digit0", keyCode: 48, ctrl: true });
  expect((await level()) === "100%", "Ctrl+0");
  await cdp("Input.dispatchMouseEvent", { type: "mouseWheel", x: 500, y: 400, deltaX: 0, deltaY: -120, modifiers: 2 });
  await until("Ctrl+wheel to zoom in", async () => (await level()) === "110%");
  expect((await evaluate("getComputedStyle(document.getElementById('toolbar')).zoom")) === "1", "the toolbar zoomed");
  await evaluate("LegibleChrome.apply({ ...JSON.parse(document.getElementById('legible-settings').textContent), zoom: 150 })");
  expect((await level()) === "110%", "a new default zoom changed this window's");
});

await test("Ctrl+wheel: a step a notch, a pinch's small movements a notch's worth, and nothing when turned off", async () => {
  await open("showcase");
  const level = () => evaluate("document.querySelector('.zoom-level').textContent");
  // The page's own wheel events (as a touchpad's pinch comes, Ctrl + small movements): whether
  // the browser's own zoom was stopped.
  const wheel = (deltaY) =>
    evaluate(`(() => {
      const event = new WheelEvent("wheel", { deltaY: ${deltaY}, ctrlKey: true, bubbles: true, cancelable: true });
      document.getElementById("document").dispatchEvent(event);
      return event.defaultPrevented;
    })()`);
  expect(await wheel(-100), "the browser's own zoom wasn't stopped");
  expect((await level()) === "110%", `a notch: ${await level()}`);
  for (let i = 0; i < 9; i++) await wheel(-10);
  expect((await level()) === "110%", `less than a notch's worth zoomed: ${await level()}`);
  await wheel(-10);
  expect((await level()) === "125%", `a notch's worth of pinch: ${await level()}`);
  await wheel(100);
  expect((await level()) === "110%", `a notch out: ${await level()}`);
  // Turned off in Settings: nothing, and still no browser zoom.
  await evaluate(`LegibleChrome.apply({ ...JSON.parse(document.getElementById('legible-settings').textContent), "wheel-zoom": false })`);
  expect(await wheel(-100), "the browser's own zoom wasn't stopped with Ctrl+wheel off");
  expect((await level()) === "110%", `zoomed with Ctrl+wheel off: ${await level()}`);
});

await test("the reading place: sent in View as the reader scrolls, and put back by its heading", async () => {
  await open("showcase");
  await evaluate(`[...document.querySelectorAll("#document h2")].find((h) => h.textContent === "Code").scrollIntoView()`);
  const places = async () => (await sent()).filter((m) => m.startsWith("place\n"));
  await until("the place to be sent", async () => (await places()).length > 0);
  const [, fraction, offset, id] = (await places()).at(-1).split("\n");
  const code = await evaluate(`[...document.querySelectorAll("#document h2")].find((h) => h.textContent === "Code").id`);
  expect(id === code, `the place's heading: ${id}, not ${code}`);
  expect(Number(fraction) > 0 && Number(fraction) <= 1, `fraction ${fraction}`);
  // Asked as the window closes, it answers where the reader is that moment, before any scroll
  // event has come.
  const now = await evaluate(`(() => {
    [...document.querySelectorAll("#document h2")].find((h) => h.textContent === "Figures").scrollIntoView();
    return LegibleChrome.place();
  })()`);
  const figures = await evaluate(`[...document.querySelectorAll("#document h2")].find((h) => h.textContent === "Figures").id`);
  expect(now?.split("\n")[2] === figures, `place() said ${JSON.stringify(now)}`);
  await evaluate(`[...document.querySelectorAll("#document h2")].find((h) => h.textContent === "Code").scrollIntoView()`);
  // From the top, back to that heading, under the chrome, as the window gives it back.
  const at = await headingTop("Code");
  await evaluate("window.scrollTo(0, 0)");
  await evaluate(`LegibleChrome.goTo({ id: ${JSON.stringify(id)}, offset: ${Number(offset)}, fraction: ${Number(fraction)} })`);
  await until("the reader back at the heading", async () => Math.abs((await headingTop("Code")) - at) < 2);
});

const COMFORT_SETTINGS = ["wheel-zoom", "reopen-windows", "remember-place"];

await test("Settings: the Comfort group sends each toggle and shows the saved settings", async () => {
  await open("showcase");
  await evaluate(`[...document.querySelectorAll('.menu-item')].find((m) => m.textContent.includes('Settings')).click()`);
  const legend = await evaluate("document.querySelector('[aria-labelledby=settings-comfort-name]').textContent");
  expect(legend.startsWith("Comfort"), `the group: ${legend}`);
  expect(await evaluate(`document.querySelector('input[name="width"][value="wide"]').checked`), "Wide isn't the default width");
  for (const name of ["wheel-zoom", "reopen-windows", "remember-place"]) {
    expect(await evaluate(`document.getElementById("settings-${name}").checked`), `${name} isn't on by default`);
    await evaluate(`document.getElementById("settings-${name}").click()`);
    expect((await sent()).includes(`setting\n${name}\nfalse`), `${name} wasn't sent`);
  }
  await evaluate(`LegibleChrome.apply({ ...JSON.parse(document.getElementById('legible-settings').textContent), "remember-place": false })`);
  expect(!(await evaluate(`document.getElementById("settings-remember-place").checked`)), "the saved setting wasn't shown");
  await key("Escape", { keyCode: 27 });
});

await test("the reading menu works from the keyboard and saves the width", async () => {
  await open("showcase");
  // Wide is the default: start from Comfortable so choosing Wide changes something.
  await evaluate(`LegibleChrome.apply({ ...JSON.parse(document.getElementById('legible-settings').textContent), width: 'comfortable' })`);
  expect(!(await evaluate("document.documentElement.classList.contains('width-wide')")), "Comfortable wasn't applied");
  await evaluate(`document.querySelector('[aria-label="Reading width and style"]').focus()`);
  await key("ArrowDown", { keyCode: 40 });
  expect(await evaluate("document.activeElement.getAttribute('role') === 'menuitemradio'"), "the menu didn't take focus");
  await key("ArrowDown", { keyCode: 40 });
  await key("ArrowDown", { keyCode: 40 });
  await key("Enter", { keyCode: 13 });
  expect(await evaluate("document.documentElement.classList.contains('width-wide')"), "Wide wasn't applied");
  expect((await sent()).includes("setting\nwidth\nwide"), `sent: ${await sent()}`);
  expect(await evaluate("document.activeElement.getAttribute('aria-label') === 'Reading width and style'"), "focus");
  await key("ArrowDown", { keyCode: 40 });
  await key("Escape", { keyCode: 27 });
  expect(await evaluate("document.querySelector('.menu:not([hidden])') === null"), "Esc didn't close the menu");
});

await test("Settings: a modal that keeps focus, sends each change and shows a failed save", async () => {
  await open("showcase");
  await evaluate(`[...document.querySelectorAll('.menu-item')].find((m) => m.textContent.includes('Settings')).click()`);
  expect(await evaluate("document.querySelector('dialog[open]')?.getAttribute('aria-label') === 'Settings'"), "not open");
  for (let i = 0; i < 25; i++) await key("Tab", { keyCode: 9 });
  expect(await evaluate("document.querySelector('dialog[open]').contains(document.activeElement)"), "focus left the dialog");
  await evaluate(`document.querySelector('input[name=theme][value=dark]').click()`);
  expect(await evaluate("document.documentElement.classList.contains('theme-dark')"), "the theme");
  expect((await sent()).includes("setting\ntheme\ndark"), "the theme wasn't sent");
  await evaluate(`document.querySelector('#settings-status-bar').click()`);
  expect((await sent()).includes("setting\nstatus-bar\ntrue"), "the status bar wasn't sent");
  // Legible has no update check, so nothing offers one.
  expect(await evaluate("!document.querySelector('#settings-updates, input[name=check-for-updates]')"), "Check for updates is offered");
  await evaluate("LegibleChrome.saveFailed('Your settings couldn\\'t be saved')");
  expect(await evaluate("document.querySelector('.save-status').textContent.startsWith('Your settings')"), "no failure shown");
  await key("Escape", { keyCode: 27 });
  expect(await evaluate("!document.querySelector('dialog[open]')"), "Esc didn't close it");
});

// Settings' tabs, as the dialog has them: each tab's settings, by the name each is saved under.
const SETTINGS_TABS = {
  appearance: ["theme", "style", "width", "zoom", "editor-font"],
  reading: ["outline", "status-bar", "remote-images", "wheel-zoom", "reopen-windows", "remember-place"],
  general: ["default-app"],
};
const openSettings = () =>
  evaluate(`[...document.querySelectorAll('.menu-item')].find((m) => m.textContent.includes('Settings')).click()`);
/** Which of Settings' tabs is selected, and the settings that can be seen. */
const settingsShown = () =>
  evaluate(`(() => {
    const d = document.querySelector("dialog[open]");
    const seen = (e) => e.checkVisibility({ visibilityProperty: true });
    const selected = [...d.querySelectorAll("[role=tab]")].filter((t) => t.getAttribute("aria-selected") === "true");
    const names = [...d.querySelectorAll("input, select")].filter(seen).map((e) => e.name);
    if (d.querySelector(".default-app") && seen(d.querySelector(".default-app"))) names.push("default-app");
    return { selected: selected.map((t) => t.dataset.tab), names: [...new Set(names)], focus: document.activeElement.dataset?.tab ?? null };
  })()`);

await test("Settings: three tabs, each showing its own settings and hiding the others'", async () => {
  await open("short");
  await evaluate("sessionStorage.clear()");
  await openSettings();
  const tabs = await evaluate(`[...document.querySelectorAll("dialog[open] [role=tablist] [role=tab]")].map((t) => [t.textContent, t.getAttribute("aria-controls")])`);
  expect(JSON.stringify(tabs) === JSON.stringify([["Appearance", "settings-panel-appearance"], ["Reading", "settings-panel-reading"], ["General", "settings-panel-general"]]), JSON.stringify(tabs));
  for (const [tab, names] of Object.entries(SETTINGS_TABS)) {
    await evaluate(`document.getElementById("settings-tab-${tab}").click()`);
    const shown = await settingsShown();
    expect(shown.selected.join() === tab && shown.names.join() === names.join(), `${tab}: ${JSON.stringify(shown)}`);
    // A hidden panel is out of the accessibility tree too.
    const hidden = await evaluate(`[...document.querySelectorAll("[role=tabpanel]")].filter((p) => p.hidden).map((p) => getComputedStyle(p).visibility)`);
    expect(hidden.length === 2 && hidden.every((v) => v === "hidden"), `the others: ${hidden}`);
  }
  await key("Escape", { keyCode: 27 });
});

await test("Settings' tabs from the keyboard: focus on the tab as it opens, arrows, Home and End, Tab into the panel, and the last tab kept", async () => {
  await open("short");
  await evaluate("sessionStorage.clear()");
  await openSettings();
  let shown = await settingsShown();
  expect(shown.selected.join() === "appearance" && shown.focus === "appearance", `opened: ${JSON.stringify(shown)}`);
  const step = async (name, code, expected) => {
    await key(name, { keyCode: code });
    shown = await settingsShown();
    expect(shown.selected.join() === expected && shown.focus === expected, `${name}: ${JSON.stringify(shown)}`);
  };
  await step("ArrowRight", 39, "reading");
  await step("ArrowRight", 39, "general");
  await step("ArrowRight", 39, "appearance");
  await step("ArrowLeft", 37, "general");
  await step("Home", 36, "appearance");
  await step("End", 35, "general");
  await step("ArrowLeft", 37, "reading");
  // One tab stop for the list: Tab goes on into the panel, to its first choice.
  await key("Tab", { keyCode: 9 });
  expect(await evaluate(`document.activeElement.name === "outline" && document.getElementById("settings-panel-reading").contains(document.activeElement)`), "Tab didn't go into the panel");
  await key("Escape", { keyCode: 27 });
  expect(await evaluate("!document.querySelector('dialog[open]')"), "Esc didn't close it");
  // Opened again, and after a reload: the tab last shown, focused.
  await openSettings();
  shown = await settingsShown();
  expect(shown.selected.join() === "reading" && shown.focus === "reading", `reopened: ${JSON.stringify(shown)}`);
  await key("Escape", { keyCode: 27 });
  await open("short");
  await openSettings();
  shown = await settingsShown();
  expect(shown.selected.join() === "reading", `after a reload: ${JSON.stringify(shown)}`);
  await key("Escape", { keyCode: 27 });
  await evaluate("sessionStorage.clear()");
});

await test("Settings: every setting on every tab is sent and applied as before", async () => {
  await open("short");
  await openSettings();
  for (const tab of Object.keys(SETTINGS_TABS)) {
    await evaluate(`document.getElementById("settings-tab-${tab}").click()`);
    const choices = await evaluate(`[...document.querySelectorAll("#settings-panel-${tab} input[type=radio]")].map((i) => [i.name, i.value])`);
    for (const [name, value] of choices) {
      await evaluate(`window.sent.length = 0; document.querySelector('#settings-panel-${tab} input[name="${name}"][value="${value}"]').click()`);
      const message = `setting\n${name}\n${value}`;
      // A choice already made sends nothing; every other is sent.
      const was = await evaluate(`JSON.parse(document.getElementById('legible-settings').textContent)[${JSON.stringify(name)}]`);
      expect(was === value || (await sent()).includes(message), `${tab}: ${name} ${value} wasn't sent: ${JSON.stringify(await sent())}`);
    }
  }
  // What they apply to the page: the last of each.
  const root = await evaluate("document.documentElement.className");
  expect(/theme-dark/.test(root) && /width-wide/.test(root) && /font-serif/.test(root), `the view: ${root}`);
  await evaluate(`document.getElementById("settings-tab-appearance").click()`);
  await evaluate(`window.sent.length = 0; (() => { const z = document.getElementById("settings-zoom"); z.value = "150"; z.dispatchEvent(new Event("change")); })()`);
  expect((await sent()).includes("setting\nzoom\n150"), `zoom: ${JSON.stringify(await sent())}`);
  await evaluate(`document.getElementById("settings-tab-reading").click()`);
  for (const name of ["status-bar", ...COMFORT_SETTINGS]) {
    await evaluate(`window.sent.length = 0; document.getElementById("settings-${name}").click()`);
    expect((await sent()).some((m) => m.startsWith(`setting\n${name}\n`)), `${name}: ${JSON.stringify(await sent())}`);
  }
  await key("Escape", { keyCode: 27 });
});

await test("Settings has no scrollbar on any tab, in the first window at 100% and at 150%", async () => {
  // A 1080p screen's first window (85% of its work area's height), at 100% and at 150%.
  const sizes = [
    { width: 900, height: 820, deviceScaleFactor: 1 },
    { width: 900, height: 530, deviceScaleFactor: 1.5 },
  ];
  try {
    for (const size of sizes) {
      await cdp("Emulation.setDeviceMetricsOverride", { ...size, mobile: false });
      await open("short");
      await openSettings();
      for (const tab of Object.keys(SETTINGS_TABS)) {
        await evaluate(`document.getElementById("settings-tab-${tab}").click()`);
        const fit = await evaluate(`(() => { const d = document.querySelector('dialog[open]');
          return { scroll: d.scrollHeight - d.clientHeight, sideways: d.scrollWidth - d.clientWidth }; })()`);
        expect(fit.scroll <= 0 && fit.sideways <= 0, `${size.height} at ${size.deviceScaleFactor}, ${tab}: ${JSON.stringify(fit)}`);
      }
      await key("Escape", { keyCode: 27 });
    }
  } finally {
    await cdp("Emulation.setDeviceMetricsOverride", { width: 1100, height: 800, deviceScaleFactor: 1, mobile: false });
  }
});

await test("Settings: the first tab's text starts where the heading and the setting names do, at 100% and at 150%", async () => {
  try {
    for (const deviceScaleFactor of [1, 1.5]) {
      await cdp("Emulation.setDeviceMetricsOverride", { width: 1100, height: 800, deviceScaleFactor, mobile: false });
      await open("short");
      await openSettings();
      for (const tab of Object.keys(SETTINGS_TABS)) {
        await evaluate(`document.getElementById("settings-tab-${tab}").click()`);
        // The text's own box, not the tab's: its padding is outside the text.
        const left = await evaluate(`(() => {
          const d = document.querySelector("dialog[open]");
          const text = document.createRange();
          text.selectNodeContents(d.querySelector("[role=tab]"));
          const name = [...d.querySelectorAll(".setting-name")].find((n) => n.checkVisibility({ visibilityProperty: true }));
          return { tab: text.getBoundingClientRect().left, heading: d.querySelector("h2").getBoundingClientRect().left, name: name.getBoundingClientRect().left };
        })()`);
        expect(Math.abs(left.tab - left.name) <= 1 && Math.abs(left.tab - left.heading) <= 1, `${tab} at ${deviceScaleFactor}: ${JSON.stringify(left)}`);
      }
      await key("Escape", { keyCode: 27 });
    }
  } finally {
    await cdp("Emulation.setDeviceMetricsOverride", { width: 1100, height: 800, deviceScaleFactor: 1, mobile: false });
  }
});

await test("Settings scrolls inside itself in a small window", async () => {
  await cdp("Emulation.setDeviceMetricsOverride", { width: 700, height: 260, deviceScaleFactor: 1, mobile: false });
  try {
    await open("showcase");
    await evaluate(`[...document.querySelectorAll('.menu-item')].find((m) => m.textContent.includes('Settings')).click()`);
    const fit = await evaluate(`(() => { const d = document.querySelector('dialog[open]'); const r = d.getBoundingClientRect();
      return { top: r.top, bottom: r.bottom, height: innerHeight, scrolls: d.scrollHeight > d.clientHeight }; })()`);
    expect(fit.top >= 0 && fit.bottom <= fit.height && fit.scrolls, `the dialog: ${JSON.stringify(fit)}`);
  } finally {
    await cdp("Emulation.setDeviceMetricsOverride", { width: 1100, height: 800, deviceScaleFactor: 1, mobile: false });
  }
});

await test("About: its layout, the window's facts, Copy, and the licence, notices and release notes", async () => {
  await open("showcase");
  // The ⋯ menu opens it, as Help → About Legible does (LegibleChrome.command('about')).
  await evaluate(`window.sent.length = 0; [...document.querySelectorAll('.menu-item')].find((m) => m.textContent.includes('About')).click()`);
  const about = "document.querySelector('dialog.about')";
  expect(await evaluate(`${about}.open && ${about}.getAttribute('aria-label') === 'About Legible'`), "About isn't open");
  expect((await sent()).includes("about"), "the window wasn't asked for its facts");
  expect(await evaluate("document.activeElement.classList.contains('default') && document.activeElement.textContent === 'OK'"), "OK isn't focused");
  // The app's name is Fluent's, from the page's strings, as every string the chrome shows.
  const name = await evaluate(
    "[document.querySelector('dialog.about .app-name').textContent, JSON.parse(document.getElementById('legible-chrome-strings').textContent)['app-name']]",
  );
  expect(name[0] === name[1] && name[1] === "Legible", `the app's name: ${JSON.stringify(name)}`);
  const text = await evaluate(`${about}.textContent`);
  for (const words of ["A reader that makes Markdown legible", "Open source under the MIT License. Includes third-party software.", "No telemetry. Legible goes online only for pictures you allow.", "isn't affiliated with, endorsed or sponsored by Microsoft"]) {
    expect(text.includes(words), `About doesn't say ${JSON.stringify(words)}: ${text}`);
  }
  // The icon, Legible's own, loads under the page's CSP.
  await until("the icon to load", () => evaluate("(() => { const i = document.querySelector('.about-icon'); return i.complete && i.naturalWidth > 0 && i.src.startsWith('data:image/svg+xml,'); })()"));
  // The table is the window's facts.
  await evaluate(`LegibleChrome.about({ rows: [["Version", "0.1.0"], ["Build", "Portable"], ["WebView2", "156.0.4314.50"], ["Windows", "Windows 11 24H2 (build 26200.6899)"]] })`);
  const rows = await evaluate(`[...document.querySelectorAll(".about-table tr")].map((r) => r.textContent)`);
  expect(rows.join("|") === "Version0.1.0|BuildPortable|WebView2156.0.4314.50|WindowsWindows 11 24H2 (build 26200.6899)", `rows: ${rows}`);
  // Copy asks the window, which copies its own facts; its answer says so.
  await evaluate("window.sent.length = 0; [...document.querySelectorAll('.about-button')].find((b) => b.textContent === 'Copy').click()");
  expect((await sent()).at(-1) === "about-copy", `Copy sent ${JSON.stringify(await sent())}`);
  await evaluate("LegibleChrome.aboutCopied()");
  expect(await evaluate("document.getElementById('toast').textContent.includes('Version information copied')"), "no toast");
  // The licence link: the reading view over About, the licence as plain text.
  // A real click (trusted, so the reading view is a dialog of its own for Esc: Chromium closes
  // dialogs opened with no user activation between them together).
  await evaluate("window.sent.length = 0");
  const [lx, ly] = await evaluate("(() => { const r = document.querySelectorAll('.about-licence .link')[0].getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; })()");
  for (const type of ["mousePressed", "mouseReleased"]) await cdp("Input.dispatchMouseEvent", { type, x: lx, y: ly, button: "left", clickCount: 1 });
  expect((await sent()).at(-1) === "reading\nlicence", `the licence link sent ${JSON.stringify(await sent())}`);
  expect(await evaluate("document.querySelector('dialog.reading').open && document.getElementById('reading-title').textContent === 'MIT License'"), "the licence's view");
  // An answer for another text than the one shown is let go; the one asked for shows, as text.
  await evaluate("LegibleChrome.reading('notices', 'not this'); LegibleChrome.reading('licence', '<b>MIT</b> License & more')");
  expect(
    await evaluate("(() => { const p = document.querySelector('.reading-text'); return p.textContent === '<b>MIT</b> License & more' && !p.querySelector('b'); })()"),
    "the licence wasn't shown as plain text",
  );
  // Esc goes back to About.
  await key("Escape", { keyCode: 27 });
  expect(await evaluate(`!document.querySelector('dialog.reading').open && ${about}.open`), "Esc didn't go back to About");
  await evaluate("window.sent.length = 0; [...document.querySelectorAll('.about-licence .link')][1].click()");
  expect((await sent()).at(-1) === "reading\nnotices" && (await evaluate("document.getElementById('reading-title').textContent")) === "Third-party notices", "the notices' link");
  await evaluate("document.querySelector('dialog.reading .default').click()");
  await evaluate("window.sent.length = 0; [...document.querySelectorAll('.about-button')].find((b) => b.textContent === 'Release notes').click()");
  expect((await sent()).at(-1) === "reading\nchangelog" && (await evaluate("document.getElementById('reading-title').textContent")) === "Release notes", "Release notes");
  // The release notes come rendered: shown as a document, its links as any link.
  await evaluate(`LegibleChrome.reading("changelog", "# Changelog", '<h1>Changelog</h1><p>See <a href="https://example.com/">it</a>.</p>')`);
  expect(
    await evaluate("document.querySelector('.reading-doc h1')?.textContent === 'Changelog' && document.querySelector('.reading-text').hidden && !document.querySelector('.reading-doc').hidden"),
    "the release notes weren't shown as a document",
  );
  // Its links are ordinary links: links.js (Legible's origin only, so not in these tests) sends
  // each to the window, as a document's.
  expect((await evaluate("document.querySelector('.reading-doc a')?.getAttribute('href')")) === "https://example.com/", "the link");
  await key("Escape", { keyCode: 27 });
  // OK closes it.
  await evaluate(`${about}.querySelector('.default').click()`);
  expect(!(await evaluate(`${about}.open`)), "OK didn't close About");
  // The privacy line is the same whatever the settings, an old copy's check-for-updates among them.
  await evaluate(`LegibleChrome.apply({ ...JSON.parse(document.getElementById("legible-settings").textContent), "check-for-updates": true })`);
  await evaluate("LegibleChrome.command('about')");
  expect(
    (await evaluate("document.querySelector('.about-privacy').textContent")) === "No telemetry. Legible goes online only for pictures you allow.",
    "the privacy line",
  );
  await key("Escape", { keyCode: 27 });
});

await test("live reload keeps the reader's place, rebuilds the outline and find, and says so", async () => {
  await open("showcase");
  const html = renderedDocument("showcase");
  await evaluate(`[...document.querySelectorAll("#outline a")].find((a) => a.textContent === "Code").click()`);
  await until("the jump to settle", async () => {
    const top = await headingTop("Code");
    return top !== null && top > 40 && top < 90;
  });
  await key("f", { code: "KeyF", keyCode: 70, ctrl: true });
  await evaluate(`(() => { const f = document.querySelector('.find-field'); f.value = 'brand new'; f.dispatchEvent(new Event('input')); })()`);
  // The find bar moves the document down, and the jump scrolls smoothly: measure once the
  // heading has come to rest below the bar (the same place twice running).
  let last = null;
  const before = await until("the heading to come to rest", async () => {
    const padded = await evaluate("parseFloat(getComputedStyle(document.body).paddingTop) > 60");
    const top = await headingTop("Code");
    const rested = padded && top === last;
    last = top;
    return rested ? top : false;
  });
  // A paragraph added above, and a new section at the end.
  const changed = html.replace("<h2", "<p>A brand new paragraph, near the top.</p><h2") + '<h2 id="user-content-added">Added</h2><p>brand new</p>';
  await replace(1, changed);
  const after = await headingTop("Code");
  expect(Math.abs(after - before) <= 3, `the place moved from ${before} to ${after}`);
  expect(await evaluate("[...document.querySelectorAll('#outline a')].some((a) => a.textContent === 'Added')"), "the outline wasn't rebuilt");
  expect((await evaluate("document.querySelector('.find-count').textContent")) === "1 of 2", "find wasn't rebuilt");
  expect(await evaluate("!document.getElementById('updated').hidden && document.getElementById('updated').getAttribute('role') === 'status'"), "no note");
  expect((await evaluate("getComputedStyle(document.getElementById('document')).zoom")) === "1", "the zoom changed");
});

await test("live reload falls back to the same share of the page when the heading has gone", async () => {
  await open("showcase");
  const html = renderedDocument("showcase");
  await evaluate("window.scrollTo(0, (document.documentElement.scrollHeight - innerHeight) / 2)");
  const before = await evaluate("scrollY / (document.documentElement.scrollHeight - innerHeight)");
  const anchored = await evaluate(`(() => { let id = null; for (const h of document.querySelectorAll('#document h2[id]'))
    if (h.getBoundingClientRect().top <= 50) id = h.id; return id; })()`);
  expect(anchored, "no heading above the middle");
  const without = html.replace(new RegExp(`<h2 id="${anchored}"[^>]*>`), "<h2>");
  expect(without !== html, "the heading wasn't taken out");
  await replace(1, without);
  const after = await evaluate("scrollY / (document.documentElement.scrollHeight - innerHeight)");
  expect(Math.abs(after - before) < 0.05, `the share moved from ${before} to ${after}`);
});

await test("an older replacement never overwrites a newer one, even when it finishes last", async () => {
  await open("short");
  const html = renderedDocument("short");
  await replace(3, html.replace("A short note", "Generation three"));
  await replace(2, html.replace("A short note", "Generation two"));
  expect(await evaluate("document.querySelector('#document h1').textContent === 'Generation three'"), "2 overwrote 3");
  // 4 has a diagram, so it prepares slowly; 5 starts while it does and finishes first.
  const diagram = renderedDocument("find");
  await evaluate(`(() => { window.slow = LegibleChrome.replaceDocument({ generation: 4, html: ${JSON.stringify(diagram)} }); })()`);
  await replace(5, html.replace("A short note", "Generation five"));
  await evaluate("window.slow");
  expect(await evaluate("document.querySelector('#document h1').textContent === 'Generation five'"), "4 overwrote 5");
  expect(await evaluate("[...document.querySelectorAll('#outline a')].map((a) => a.textContent).join() === 'Generation five,The second'"), "the outline is 4's");
});

await test("the moved-or-deleted bar: Keep showing it, and Close", async () => {
  await open("short");
  await evaluate("LegibleChrome.fileGone(true)");
  const bar = () => evaluate("!document.getElementById('file-bar').hidden");
  expect(await bar(), "no bar");
  expect(await evaluate("document.getElementById('file-bar').textContent.includes('moved or deleted')"), "its text");
  await until("the document to move below the bar", () =>
    evaluate("parseFloat(getComputedStyle(document.body).paddingTop) > 60"),
  );
  await evaluate(`[...document.querySelectorAll('#file-bar button')].find((b) => b.textContent === 'Close').click()`);
  expect((await sent()).includes("close"), "Close didn't ask the window");
  await evaluate(`[...document.querySelectorAll('#file-bar button')].find((b) => b.textContent === 'Keep showing it').click()`);
  expect(!(await bar()), "Keep showing it didn't hide it");
  // The window remembers both, for a page it serves again: a page starting asks for the bar.
  expect((await sent()).at(-1) === "file-keep", `Keep showing it sent ${JSON.stringify((await sent()).at(-1))}`);
  expect((await sent()).includes("file-status"), "the page didn't ask whether to show the bar");
  // A new version shown hides it too.
  await evaluate("LegibleChrome.fileGone(true)");
  await replace(1, renderedDocument("short"));
  expect(!(await bar()), "a reload didn't hide it");
});

await test("the answers to a held change are sent to the window", async () => {
  await open("short");
  await evaluate("LegibleChrome.reloadFromDisk(3); LegibleChrome.keepMine(4)");
  const messages = await sent();
  for (const m of ["reload\n3", "keep\n4"]) {
    expect(messages.includes(m), `${JSON.stringify(m)} not sent: ${JSON.stringify(messages)}`);
  }
});

await test("recent files: the menu shows the newest list, and a pick names it by its token", async () => {
  await open("short");
  expect((await sent()).includes("recent"), "the page didn't ask for the recent files");
  const more = `document.querySelector('[aria-label="More"]')`;
  // Empty: a line saying so, which does nothing.
  await evaluate(`${more}.click()`);
  const empty = await evaluate(`[...document.querySelectorAll('.menu:not([hidden]) .menu-item')].map((m) => [m.textContent, m.getAttribute('aria-disabled')])`);
  expect(empty[0][0] === "No recent files" && empty[0][1] === "true", `empty: ${JSON.stringify(empty)}`);
  await key("Escape", { keyCode: 27 });
  // A list arrives; the menu opened from the keyboard starts at its first file.
  const setRecent = (list) => evaluate(`LegibleChrome.setRecent(${JSON.stringify(list)})`);
  await setRecent({
    token: 7,
    items: [
      { name: "Plan.md", path: "C:\\Notes\\Plan.md" },
      { name: "Notes ü.md", path: "C:\\Notes\\Notes ü.md" },
    ],
  });
  await evaluate(`${more}.focus()`);
  await key("ArrowDown", { keyCode: 40 });
  const focused = await evaluate("[document.activeElement.textContent, document.activeElement.title]");
  expect(focused[0] === "Plan.md" && focused[1] === "C:\\Notes\\Plan.md", `focused: ${focused}`);
  expect(await evaluate("[...document.querySelectorAll('.menu:not([hidden]) .menu-heading')].some((h) => h.textContent === 'Recent files')"), "no heading");
  await key("ArrowDown", { keyCode: 40 });
  await key("Enter", { keyCode: 13 });
  expect((await sent()).includes("open-recent\n7\n1"), `sent: ${JSON.stringify(await sent())}`);
  // A newer list replaces it, and picks name the new token.
  await setRecent({ token: 8, items: [{ name: "Other.md", path: "C:\\Other.md" }] });
  await evaluate(`${more}.click()`);
  await evaluate(`[...document.querySelectorAll('.menu-item')].find((m) => m.textContent === 'Other.md').click()`);
  expect((await sent()).includes("open-recent\n8\n0"), "the new list's token");
  // A list that comes while the menu is open: a row still showing names the list it came from,
  // so the window refuses it rather than opening whatever is at that place now.
  await setRecent({ token: 20, items: [{ name: "Old.md", path: "C:\\Old.md" }] });
  await evaluate(`${more}.click()`);
  await setRecent({ token: 21, items: [{ name: "Different.md", path: "C:\\Different.md" }] });
  await evaluate(`[...document.querySelectorAll('.menu:not([hidden]) .menu-item')].find((m) => m.textContent === 'Old.md').click()`);
  const picks = (await sent()).filter((m) => m.startsWith("open-recent"));
  expect(picks.at(-1) === "open-recent\n20\n0", `the open menu's pick: ${JSON.stringify(picks.at(-1))}`);
  // Files of the same name show as many of their folders as tell them apart; others show none.
  await setRecent({
    token: 9,
    items: [
      { name: "README.md", path: "C:\\Work\\Handbook\\README.md" },
      { name: "Plan.md", path: "C:\\Notes\\Plan.md" },
      { name: "readme.md", path: "D:\\Home\\Handbook\\readme.md" },
      { name: "README.md", path: "C:\\Old\\README.md" },
    ],
  });
  await evaluate(`${more}.click()`);
  const hints = await evaluate(
    `[...document.querySelectorAll('.menu:not([hidden]) .menu-item')].slice(0, 4).map((m) => m.querySelector('.menu-hint')?.textContent ?? "")`,
  );
  expect(JSON.stringify(hints) === JSON.stringify(["Work\\Handbook", "", "Home\\Handbook", "Old"]), `hints: ${JSON.stringify(hints)}`);
  await key("Escape", { keyCode: 27 });
});

await test("pictures from the internet: placeholders until the window lists them, and the bar asks", async () => {
  await open("pictures");
  expect((await sent()).includes("remote-images"), "the page didn't ask what its pictures may do");
  const pictures = () =>
    evaluate(`[...document.querySelectorAll("#document img[data-remote-src]")].map((i) => i.getAttribute("src"))`);
  expect(JSON.stringify(await pictures()) === "[null,null]", `before anything: ${JSON.stringify(await pictures())}`);
  const raw = await evaluate(`[...document.querySelectorAll("#document img[data-remote-src]")].map((i) => i.getAttribute("data-remote-src"))`);
  const bar = `document.getElementById("pictures-bar")`;
  // An answer for another render changes nothing.
  await evaluate(`LegibleChrome.remoteImages({ generation: 4, show: ${JSON.stringify(raw)}, ask: true })`);
  expect(JSON.stringify(await pictures()) === "[null,null]", "a stale answer showed pictures");
  expect(await evaluate(`${bar}.hidden`), "a stale answer showed the bar");
  // The first may load, and the bar asks about the other.
  await evaluate(`LegibleChrome.remoteImages({ generation: 0, show: [${JSON.stringify(raw[0])}], ask: true })`);
  const shown = await pictures();
  expect(shown[0] === raw[0] && shown[1] === null, `listed: ${JSON.stringify(shown)}`);
  expect(
    await evaluate(`!${bar}.hidden && ${bar}.getAttribute("role") === "region" && ${bar}.textContent.includes("pictures from the internet")`),
    "the bar doesn't ask",
  );
  await until("the first picture to load", () => evaluate(`document.querySelector("#document img[src]").naturalWidth > 0`));
  // Its buttons answer for this render; Close hides it for this render.
  for (const [label, answer] of [
    ["Show once", "once"],
    ["Always for this file", "always"],
  ]) {
    await evaluate(`[...${bar}.querySelectorAll("button")].find((b) => b.textContent === ${JSON.stringify(label)}).click()`);
    expect((await sent()).at(-1) === `remote-images\n${answer}\n0`, `${label} sent ${JSON.stringify((await sent()).at(-1))}`);
  }
  await evaluate(`[...${bar}.querySelectorAll("button")].find((b) => b.textContent === "Close").click()`);
  expect(await evaluate(`${bar}.hidden`), "Close didn't hide the bar");
  // Why none can show: said, with nothing to answer.
  await open("pictures");
  await evaluate(`LegibleChrome.remoteImages({ generation: 0, show: [], ask: false, unavailable: "failed" })`);
  expect(
    await evaluate(`!${bar}.hidden && ${bar}.textContent.includes("stopped showing") && ${bar}.querySelector(".pictures-actions").hidden`),
    "the unavailable bar",
  );
  expect(JSON.stringify(await pictures()) === "[null,null]", "pictures shown while unavailable");
  // Once they can't be checked, none shows on this page again, whatever it's told after.
  const html = renderedDocument("pictures");
  await evaluate(`LegibleChrome.replaceDocument({ generation: 1, html: ${JSON.stringify(html)}, remote: { show: [${JSON.stringify(raw[1])}], ask: false } })`);
  await until("the replacement", () => evaluate(`document.getElementById("document").dataset.generation === "1"`));
  await evaluate(`LegibleChrome.remoteImages({ generation: 1, show: ${JSON.stringify(raw)}, ask: false })`);
  expect(JSON.stringify(await pictures()) === "[null,null]", "a picture shown after they couldn't be checked");
  // A live replacement carries what its render's pictures may do.
  await open("pictures");
  await evaluate(`LegibleChrome.replaceDocument({ generation: 1, html: ${JSON.stringify(html)}, remote: { show: [${JSON.stringify(raw[1])}], ask: false } })`);
  await until("the replacement's picture", async () => (await pictures())[1] === raw[1]);
  expect((await pictures())[0] === null && (await evaluate(`${bar}.hidden`)), "the replacement's pictures");
});

await test("a page served again keeps the reader's place, the zoom and the outline; a new one doesn't", async () => {
  await open("showcase");
  await key("=", { code: "Equal", keyCode: 187, ctrl: true });
  await key("O", { code: "KeyO", keyCode: 79, ctrl: true, shift: true });
  const outlineOpen = await evaluate(`!document.getElementById("outline").hidden`);
  await evaluate(`document.querySelector("#document h2:nth-of-type(3)").scrollIntoView()`);
  const before = await evaluate(`document.querySelector("#document h2:nth-of-type(3)").getBoundingClientRect().top`);
  // Navigating away is what serving the page again does: the next page is generation 1.
  await open("again");
  const state = await evaluate(`({
    zoom: document.querySelector(".zoom-level").textContent,
    outline: !document.getElementById("outline").hidden,
    top: document.querySelector("#document h2:nth-of-type(3)").getBoundingClientRect().top,
  })`);
  expect(state.zoom === "110%", `zoom: ${state.zoom}`);
  expect(state.outline === outlineOpen, `outline: ${state.outline}`);
  expect(Math.abs(state.top - before) < 2, `place: ${state.top} against ${before}`);
  // A first page (generation 0) starts afresh.
  await open("showcase");
  expect((await evaluate(`document.querySelector(".zoom-level").textContent`)) === "100%", "a new page took the zoom");
  expect((await evaluate("window.scrollY")) === 0, "a new page took the place");
});

/** The last `copy` message the page sent: its id, generation and HTML. */
async function lastCopy() {
  const message = (await sent()).filter((m) => m.startsWith("copy\n")).at(-1);
  if (!message) return null;
  const [, id, generation, ...html] = message.split("\n");
  return { id: Number(id), generation: Number(generation), html: html.join("\n") };
}

await test("Copy document sends the document, each diagram as its light picture, labels drawn", async () => {
  await open("showcase", { dark: true });
  await key("C", { code: "KeyC", keyCode: 67, ctrl: true, shift: true });
  await until("the copy to be sent", () => lastCopy());
  const copy = await lastCopy();
  expect(copy.generation === 0, `generation ${copy.generation}`);
  expect(copy.html.includes("Quarterly operations review") && !copy.html.includes("<svg"), "the document, no SVG");
  const png = /<img src="(data:image\/png;base64,[^"]+)" alt="Diagram">/.exec(copy.html)?.[1];
  expect(png, "no diagram picture");
  // Light (white corners), with ink in it (the shapes and labels drew).
  const look = await evaluate(`(async () => {
    const image = new Image();
    await new Promise((ok) => { image.onload = ok; image.src = ${JSON.stringify(png)}; });
    const canvas = document.createElement("canvas");
    canvas.width = image.width; canvas.height = image.height;
    const context = canvas.getContext("2d");
    context.drawImage(image, 0, 0);
    const data = context.getImageData(0, 0, image.width, image.height).data;
    let dark = 0;
    for (let i = 0; i < data.length; i += 4) if (data[i] < 128 && data[i + 1] < 128 && data[i + 2] < 128) dark++;
    const corner = [...context.getImageData(1, 1, 1, 1).data];
    return { width: image.width, height: image.height, dark, corner };
  })()`);
  expect(look.width > 400 && look.corner.join() === "255,255,255,255" && look.dark > 500, `the picture: ${JSON.stringify(look)}`);
  // The window's answer, for this copy only.
  await evaluate(`LegibleChrome.copied({ id: ${copy.id - 1}, generation: 0, outcome: "copied" })`);
  expect(await evaluate("document.getElementById('toast').hidden"), "an old copy's answer showed the toast");
  await evaluate(`LegibleChrome.copied({ id: ${copy.id}, generation: 0, outcome: "copied" })`);
  expect(
    await evaluate("!document.getElementById('toast').hidden && document.getElementById('toast').textContent.includes('ready to paste into Outlook, Word or OneNote')"),
    "the toast",
  );
  // The toolbar's Copy does the same.
  await evaluate(`document.querySelector('[aria-label="Copy"]').click()`);
  await until("a second copy", async () => (await lastCopy()).id === copy.id + 1);
  await evaluate(`LegibleChrome.copied({ id: ${copy.id + 1}, generation: 0, outcome: "failed", text: "Couldn't copy: nope" })`);
  expect(await evaluate("document.getElementById('notice').textContent.includes(\"Couldn't copy: nope\")"), "the failure");
});

await test("an answer for a page served before is never taken for this page's copy of the same number", async () => {
  // The page before (generation 0) made copy 1, and is served again (generation 1)...
  await open("showcase");
  await key("C", { code: "KeyC", keyCode: 67, ctrl: true, shift: true });
  await until("the first page's copy", () => lastCopy());
  expect((await lastCopy()).id === 1 && (await lastCopy()).generation === 0, "the first page's copy 1");
  await open("again");
  await key("C", { code: "KeyC", keyCode: 67, ctrl: true, shift: true });
  await until("the new page's copy", () => lastCopy());
  const copy = await lastCopy();
  expect(copy.id === 1 && copy.generation === 1, `the new page's copy: ${JSON.stringify([copy.id, copy.generation])}`);
  // ...and the first page's answers arrive late, in any order: superseded, failed.
  const toast = "!document.getElementById('toast').hidden";
  const notice = "!document.getElementById('notice').hidden";
  await evaluate(`LegibleChrome.copied({ id: 1, generation: 0, outcome: "superseded" })`);
  await evaluate(`LegibleChrome.copied({ id: 1, generation: 0, outcome: "failed", text: "Couldn't copy: old" })`);
  expect(!(await evaluate(toast)) && !(await evaluate(notice)), "an old page's answer showed something");
  // The new page's own answer still counts.
  await evaluate(`LegibleChrome.copied({ id: 1, generation: 1, outcome: "copied" })`);
  expect(await evaluate(toast), "the new page's copy wasn't answered");
});

await test("Ctrl+C copies a selection in the document, and leaves everything else to the browser", async () => {
  await open("short");
  const copies = async () => (await sent()).filter((m) => m.startsWith("copy\n")).length;
  // Nothing selected: nothing sent.
  await key("c", { code: "KeyC", keyCode: 67, ctrl: true });
  expect((await copies()) === 0, "a copy with nothing selected");
  // A paragraph's words selected: just those.
  await evaluate(`(() => {
    const p = document.querySelector("#document p");
    const range = document.createRange();
    range.selectNodeContents(p);
    getSelection().removeAllRanges();
    getSelection().addRange(range);
  })()`);
  const paragraph = await evaluate(`document.querySelector("#document p").textContent`);
  await key("c", { code: "KeyC", keyCode: 67, ctrl: true });
  await until("the selection's copy", async () => (await copies()) === 1);
  const copy = await lastCopy();
  expect(copy.html.includes(paragraph) && !copy.html.includes("<h1"), `the selection: ${copy.html}`);
  // In the find field: the browser's own copy, not the document's.
  await key("f", { code: "KeyF", keyCode: 70, ctrl: true });
  await key("c", { code: "KeyC", keyCode: 67, ctrl: true });
  expect((await copies()) === 1, "Ctrl+C in the find field copied the document's way");
});

await test("a selection touching a diagram copies the whole diagram", async () => {
  await open("showcase");
  await evaluate(`(() => {
    const diagram = document.querySelector("#document .diagram");
    let before = diagram.previousElementSibling;
    while (before && !before.textContent.trim()) before = before.previousElementSibling;
    const label = diagram.querySelector(".figure.light foreignObject");
    const range = document.createRange();
    range.setStart(before.firstChild, 0);
    range.setEnd(label, 0);
    getSelection().removeAllRanges();
    getSelection().addRange(range);
  })()`);
  await key("c", { code: "KeyC", keyCode: 67, ctrl: true });
  await until("the selection's copy", () => lastCopy());
  const copy = await lastCopy();
  const diagrams = (copy.html.match(/class="mermaid diagram"/g) || []).length;
  expect(diagrams === 1 && /<img src="data:image\/png/.test(copy.html), `one whole diagram: ${copy.html.slice(0, 300)}`);
});

await test("printing puts the title at the top of each page and its number at the bottom", async () => {
  await open("showcase");
  const rule = await evaluate(`[...document.adoptedStyleSheets].flatMap((s) => [...s.cssRules]).map((r) => r.cssText).find((t) => t.startsWith("@page"))`);
  expect(rule && rule.includes("@top-center") && rule.includes("showcase"), `the header: ${rule}`);
  expect(/@bottom-center \{[^}]*content: "Page " counter\(page\) " of " counter\(pages\)/.test(rule), `the footer: ${rule}`);
  // An awkward title: still one CSS string.
  await evaluate(`document.title = 'Q3 "review" \\\\ back\\\\slash — Ünïcödé'`);
  await until("the header to follow the title", () =>
    evaluate(`[...document.adoptedStyleSheets].flatMap((s) => [...s.cssRules]).some((r) => r.cssText.includes('content: "Q3 \\\\"review\\\\" \\\\\\\\ back\\\\\\\\slash'))`),
  );
});

await test("Save as PDF asks the window; a saved PDF offers Open and Show in folder", async () => {
  await open("short");
  await evaluate(`document.querySelector('[aria-label="More"]').click()`);
  await evaluate(`[...document.querySelectorAll('.menu-item')].find((m) => m.textContent === 'Save as PDF').click()`);
  expect((await sent()).at(-1) === "save-pdf", `sent ${JSON.stringify((await sent()).at(-1))}`);
  await evaluate(`LegibleChrome.pdfSaved({ name: "Plan.pdf" })`);
  const toast = `document.getElementById('toast')`;
  expect(await evaluate(`!${toast}.hidden && ${toast}.textContent.includes('Saved Plan.pdf')`), "the toast");
  for (const [label, message] of [
    ["Open", "pdf-open"],
    ["Show in folder", "pdf-show"],
  ]) {
    await evaluate(`[...${toast}.querySelectorAll('button')].find((b) => b.textContent === ${JSON.stringify(label)}).click()`);
    expect((await sent()).at(-1) === message, `${label} sent ${JSON.stringify((await sent()).at(-1))}`);
  }
  await open("empty");
  await evaluate(`document.querySelector('[aria-label="More"]').click()`);
  expect(
    (await evaluate(`[...document.querySelectorAll('.menu-item')].find((m) => m.textContent === 'Save as PDF').getAttribute('aria-disabled')`)) === "true",
    "Save as PDF without a document",
  );
});

await test("Open in editor and Show in folder ask the window, and do nothing without a document", async () => {
  const more = `document.querySelector('[aria-label="More"]')`;
  const item = (label) =>
    `[...document.querySelectorAll('.menu:not([hidden]) .menu-item')].find((m) => m.textContent === ${JSON.stringify(label)})`;
  await open("short");
  for (const [label, message] of [
    ["Open in editor", "open-editor"],
    ["Show in folder", "show-folder"],
  ]) {
    await evaluate(`${more}.click()`);
    expect((await evaluate(`${item(label)}.getAttribute('aria-disabled')`)) === null, `${label} is disabled`);
    await evaluate(`${item(label)}.click()`);
    expect((await sent()).at(-1) === message, `${label} sent ${JSON.stringify((await sent()).at(-1))}`);
  }
  await evaluate("LegibleChrome.notice('Couldn\\'t open an editor for this file.')");
  expect(await evaluate("!document.getElementById('notice').hidden"), "no notice of a failure");

  await open("empty");
  for (const label of ["Open in editor", "Show in folder"]) {
    await evaluate(`${more}.click()`);
    expect((await evaluate(`${item(label)}.getAttribute('aria-disabled')`)) === "true", `${label} enabled`);
    await evaluate(`${item(label)}.click()`);
    await key("Escape", { keyCode: 27 });
  }
  expect(!(await sent()).some((m) => m === "open-editor" || m === "show-folder"), "sent without a document");
});

await test("a window says what it opens when what's dropped isn't Markdown", async () => {
  await open("short");
  await evaluate("LegibleChrome.notice('Legible opens Markdown files (.md, .markdown and .mdown).')");
  expect(
    await evaluate("(() => { const n = document.getElementById('notice'); return !n.hidden && n.getAttribute('role') === 'alert' && n.textContent.includes('Markdown files'); })()"),
    "no notice",
  );
});

await test("print: the document alone, unzoomed, over several pages", async () => {
  await open("showcase");
  await key("=", { code: "Equal", keyCode: 187, ctrl: true });
  await cdp("Emulation.setEmulatedMedia", { media: "print" });
  const state = await evaluate(`({
    chrome: ["toolbar", "outline", "find"].map((id) => getComputedStyle(document.getElementById(id)).display),
    zoom: getComputedStyle(document.getElementById('document')).zoom,
    padding: getComputedStyle(document.body).paddingLeft + " " + getComputedStyle(document.body).paddingTop,
  })`);
  expect(state.chrome.every((d) => d === "none"), `chrome in print: ${state.chrome}`);
  expect(state.zoom === "1" && state.padding === "0px 0px", `print layout: ${JSON.stringify(state)}`);
  await cdp("Emulation.setEmulatedMedia", { media: "screen" });
  const { data } = await cdp("Page.printToPDF", { printBackground: true });
  const pdfPages = (Buffer.from(data, "base64").toString("latin1").match(/\/Type\s*\/Page[^s]/g) || []).length;
  expect(pdfPages > 1, `${pdfPages} page(s)`);
});

await test("the default app: the bar offers it until it's taken or dismissed, and Settings follows", async () => {
  await open("short");
  expect((await sent()).includes("default-app"), "the page didn't ask about the default app");
  const bar = `document.getElementById("default-bar")`;
  // Where the document starts: under the toolbar and the bars.
  const chromeTop = () => evaluate(`parseFloat(getComputedStyle(document.body).paddingTop)`);
  expect(await evaluate(`${bar}.hidden`), "the bar showed before the window said anything");
  const state = (offer, registered, isDefault, portable) =>
    evaluate(`LegibleChrome.defaultApp(${JSON.stringify({ offer, registered, isDefault, portable })})`);
  // Registered and not the default: offered, under the toolbar, with the document below it.
  await state(true, true, false, false);
  expect(await evaluate(`!${bar}.hidden && ${bar}.getAttribute("role") === "region"`), "the bar didn't offer");
  expect((await evaluate(`${bar}.getBoundingClientRect().top`)) === 40, "the bar isn't under the toolbar");
  await until("the document to make room", async () => (await chromeTop()) > 40);
  // Taken: Default apps opens, and the offer stops (the setting, for every window).
  await evaluate(`window.sent.length = 0; ${bar}.querySelectorAll("button")[0].click()`);
  const taken = await sent();
  expect(taken.includes("default-app-open") && taken.includes("setting\noffer-default-app\nfalse"), JSON.stringify(taken));
  expect(await evaluate(`${bar}.hidden`), "the bar stayed once taken");
  await until("the document to move back up", async () => (await chromeTop()) === 40);
  // Another window's state arriving doesn't bring it back.
  await state(true, true, false, false);
  expect(await evaluate(`${bar}.hidden`), "the bar came back after it was taken");

  // A fresh window: Close puts it off (the window tells every window) without a setting.
  await open("short");
  await state(true, true, false, false);
  await evaluate(`window.sent.length = 0; ${bar}.querySelectorAll("button")[2].click()`);
  const closed = await sent();
  expect(closed.join() === "default-app-close", JSON.stringify(closed));
  expect(await evaluate(`${bar}.hidden`), "the bar stayed once closed");

  // A fresh window: Don't ask again stops it without opening anything.
  await open("short");
  await state(true, true, false, false);
  await evaluate(`window.sent.length = 0; ${bar}.querySelectorAll("button")[1].click()`);
  const dismissed = await sent();
  expect(!dismissed.includes("default-app-open") && dismissed.includes("setting\noffer-default-app\nfalse"), JSON.stringify(dismissed));
  expect(await evaluate(`${bar}.hidden`), "the bar stayed once dismissed");
  // Already the default, or not registered (Default apps wouldn't list it): never offered.
  await open("short");
  await state(false, true, true, false);
  expect(await evaluate(`${bar}.hidden`), "offered while already the default");

  // Settings: which buttons show, for each state.
  await evaluate(`[...document.querySelectorAll('.menu-item')].find((m) => m.textContent.includes('Settings')).click()`);
  expect((await sent()).filter((m) => m === "default-app").length >= 2, "Settings didn't ask again");
  const shown = () =>
    evaluate(`(() => {
      const section = document.querySelector(".default-app");
      return [...section.querySelectorAll("p, button")].filter((e) => !e.hidden).map((e) => e.textContent).join(" | ");
    })()`);
  await state(false, true, true, false);
  expect((await shown()) === "Legible opens your Markdown files.", await shown());
  await state(true, true, false, false);
  expect((await shown()) === "Make Legible the default for Markdown files", await shown());
  await state(false, false, false, true);
  const portable = await shown();
  expect(portable.includes("portable") && portable.endsWith("Register Legible for .md files on this PC"), portable);
  await state(true, true, false, true);
  expect((await shown()).includes("Make Legible the default") && (await shown()).includes("Remove Legible"), await shown());
  await evaluate(`window.sent.length = 0; [...document.querySelectorAll(".default-app button")].find((b) => b.textContent.startsWith("Remove")).click()`);
  expect((await sent()).includes("default-app-unregister"), JSON.stringify(await sent()));
  await state(false, false, false, true);
  await evaluate(`window.sent.length = 0; [...document.querySelectorAll(".default-app button")].find((b) => !b.hidden && b.textContent.startsWith("Register")).click()`);
  expect((await sent()).includes("default-app-register"), JSON.stringify(await sent()));
  await key("Escape", { keyCode: 27 });
});

await test("the Outline button is off and unusable in Split and Edit, and the outline comes back in View", async () => {
  await open("showcase");
  const button = `document.querySelector('[aria-controls="outline"]')`;
  const outlineOpen = () => evaluate("!document.getElementById('outline').hidden");
  const buttonState = () =>
    evaluate(`({ pressed: ${button}.getAttribute("aria-pressed"), disabled: ${button}.getAttribute("aria-disabled") === "true", title: ${button}.title })`);
  const settingsNow = "JSON.parse(document.getElementById('legible-settings').textContent)";
  const toMode = (mode) => evaluate(`document.dispatchEvent(new CustomEvent("legible-mode", { detail: "${mode}" }))`);
  const ctrlShiftO = () => key("O", { code: "KeyO", keyCode: 79, ctrl: true, shift: true });
  // View, with the outline open (the showcase is long).
  expect(await outlineOpen(), "the showcase's outline isn't open in View");
  let state = await buttonState();
  expect(state.pressed === "true" && !state.disabled && state.title === "Outline (Ctrl+Shift+O)", JSON.stringify(state));
  await evaluate("window.sent.length = 0");
  for (const mode of ["split", "edit"]) {
    await toMode(mode);
    state = await buttonState();
    expect(state.pressed === "false" && state.disabled, `${mode}: ${JSON.stringify(state)}`);
    expect(state.title === "Outline: shown in View", `${mode}: ${state.title}`);
    // Neither the button nor Ctrl+Shift+O does anything there.
    await evaluate(`${button}.click()`);
    await ctrlShiftO();
    expect(await outlineOpen(), `${mode}: the outline's own state changed`);
    expect((await buttonState()).pressed === "false", `${mode}: shown as pressed`);
  }
  expect(!(await sent()).some((m) => m.startsWith("setting")), `a setting was sent: ${await sent()}`);
  // Back in View: as it was, and usable again.
  await toMode("view");
  state = await buttonState();
  expect(state.pressed === "true" && !state.disabled && state.title === "Outline (Ctrl+Shift+O)", JSON.stringify(state));
  expect(await outlineOpen(), "the outline didn't come back in View");
  await ctrlShiftO();
  expect(!(await outlineOpen()) && (await buttonState()).pressed === "false", "Ctrl+Shift+O doesn't close it in View");
  // Closed by hand in View, it stays closed through Edit and back.
  await toMode("edit");
  await toMode("view");
  expect(!(await outlineOpen()) && (await buttonState()).pressed === "false", "the hand toggle was lost");
  expect((await evaluate(`${settingsNow}.outline`)) === "automatic", "the outline setting changed");
});

await test("the menu bar's items do what their buttons and keys do, and the window hears the outline", async () => {
  await open("showcase");
  const command = (name) => evaluate(`LegibleChrome.command(${JSON.stringify(name)})`);
  const outlineOpen = () => evaluate("!document.getElementById('outline').hidden");
  const level = () => evaluate("document.querySelector('.zoom-level').textContent");
  // The long showcase opens with its outline: the window is told, so its menu ticks Outline.
  expect((await sent()).includes("outline\ntrue"), `sent: ${JSON.stringify(await sent())}`);
  await evaluate("window.sent.length = 0");
  await command("outline");
  expect(!(await outlineOpen()) && (await sent()).at(-1) === "outline\nfalse", `outline: ${JSON.stringify(await sent())}`);
  await command("outline");
  expect((await outlineOpen()) && (await sent()).at(-1) === "outline\ntrue", "outline again");
  await command("zoom-in");
  expect((await level()) === "110%", `zoom in: ${await level()}`);
  await command("zoom-out");
  await command("zoom-out");
  expect((await level()) === "90%", `zoom out: ${await level()}`);
  await command("zoom-reset");
  expect((await level()) === "100%", `actual size: ${await level()}`);
  await command("width-narrow");
  expect(await evaluate("document.documentElement.classList.contains('width-narrow')"), "Narrow");
  await command("theme-dark");
  expect(await evaluate("document.documentElement.classList.contains('theme-dark')"), "Dark");
  expect((await sent()).includes("setting\nwidth\nnarrow") && (await sent()).includes("setting\ntheme\ndark"), "the settings weren't sent");
  await command("find");
  expect(await evaluate("!document.getElementById('find').hidden && document.activeElement.closest('#find') !== null"), "Find");
  await key("Escape", { keyCode: 27 });
  await command("settings");
  expect(await evaluate("document.querySelector('dialog[open]')?.getAttribute('aria-label') === 'Settings'"), "Settings");
  await key("Escape", { keyCode: 27 });
  await command("about");
  const dialogs = await evaluate("[...document.querySelectorAll('dialog[open]')].map((d) => d.getAttribute('aria-label'))");
  expect(dialogs.join() === "About Legible", `About: open ${JSON.stringify(dialogs)}`);
  await key("Escape", { keyCode: 27 });
  await evaluate("window.printed = 0; window.print = () => window.printed++");
  await command("print");
  expect((await evaluate("window.printed")) === 1, "Print");
  await command("copy-document");
  await until("Copy document to be sent", () => lastCopy());
  // One it doesn't know does nothing.
  await command("nothing");
  await command("__proto__");
  // Ctrl+W asks the window to close, as File's Close does.
  await evaluate("window.sent.length = 0");
  await key("w", { code: "KeyW", keyCode: 87, ctrl: true });
  expect((await sent()).at(-1) === "close", `Ctrl+W sent ${JSON.stringify(await sent())}`);
});

await test("Ctrl+A in View selects the document alone; the find field and the dialogs still take a selection", async () => {
  await open("showcase");
  // The chrome's text is all there: the toolbar, the outline, and a bar (the moved-or-deleted one).
  await evaluate("LegibleChrome.fileGone(true)");
  expect(await evaluate("!document.getElementById('outline').hidden && !document.getElementById('file-bar').hidden"), "chrome");
  await evaluate("document.activeElement?.blur(); getSelection().removeAllRanges()");
  await key("a", { code: "KeyA", keyCode: 65, ctrl: true });
  // What's selected reads exactly as the document does: nothing of the toolbar, outline or bars.
  const texts = await evaluate(`(() => {
    const words = (t) => t.replace(/\\s+/g, " ").trim();
    const all = words(getSelection().toString());
    const range = document.createRange();
    range.selectNodeContents(document.getElementById("document"));
    getSelection().removeAllRanges();
    getSelection().addRange(range);
    return { all, doc: words(getSelection().toString()) };
  })()`);
  expect(texts.all.length > 1000 && texts.all === texts.doc, `selected beyond the document: ${JSON.stringify(texts.all.slice(0, 200))}`);
  // The find field takes a selection.
  await key("f", { code: "KeyF", keyCode: 70, ctrl: true });
  await cdp("Input.insertText", { text: "quarter" });
  await key("a", { code: "KeyA", keyCode: 65, ctrl: true });
  const field = await evaluate("(() => { const f = document.activeElement; return [f.className, f.selectionStart, f.selectionEnd]; })()");
  expect(field[0].includes("find-field") && field[1] === 0 && field[2] === 7, `the find field: ${JSON.stringify(field)}`);
  await key("Escape", { keyCode: 27 });
  // Settings and About's notices can still be selected.
  await evaluate("LegibleChrome.command('settings')");
  expect(await evaluate("getComputedStyle(document.querySelector('dialog[open]')).userSelect !== 'none'"), "Settings isn't selectable");
  await key("Escape", { keyCode: 27 });
  await evaluate("LegibleChrome.command('about')");
  await evaluate("[...document.querySelectorAll('.about-licence .link')][1].click(); LegibleChrome.reading('notices', 'MIT notices')");
  const notices = await evaluate("(() => { getSelection().selectAllChildren(document.querySelector('.reading-text')); return getSelection().toString(); })()");
  expect(notices === "MIT notices", `the notices: ${JSON.stringify(notices)}`);
  await key("Escape", { keyCode: 27 });
  await key("Escape", { keyCode: 27 });
});

// --- The editor (web/editor/editor.js) -----------------------------------------------------------

console.log("Testing the editor...");

// The window's side, stood in for: `source` is answered with the file's text, and each `render`
// is rendered through core (legible-core's editor-preview example) and given back. While `hold`
// is set, renders wait in `held` until a test releases them (their state is with open()'s).

async function pump() {
  const all = await sent();
  for (const message of all.slice(handled)) {
    handled++;
    const [name, ...rest] = message.split("\n");
    if (message === "source") {
      const text = readFileSync(sourceOf[opened], "utf8");
      await evaluate(`LegibleEditor.load(${JSON.stringify({ text, editable: true })})`);
    } else if (name === "render") {
      const [version, ...text] = rest;
      const html = execFileSync(previewExe, ["/docfiles/"], { input: text.join("\n") })
        .toString()
        .replaceAll("https://legible-doc.localhost/", `${origin}/docfiles/`);
      const answer = `LegibleEditor.preview(${version}, ${JSON.stringify(html)}, ${JSON.stringify(previewInfo)})`;
      if (hold) held.push(answer);
      else await evaluate(answer);
    }
  }
}

const fileText = (name) => readFileSync(sourceOf[name], "utf8").replace(/\r\n?/g, "\n");
const pumped = (what, check) =>
  until(what, async () => {
    await pump();
    return check();
  });

/** Switches the mode as a toolbar button does, the window answering meanwhile. */
async function toMode(mode) {
  // Done when setMode's promise settles (the editor focused, the panes laid out), not merely
  // when the mode is set; the window's answers are pumped meanwhile.
  await evaluate(`window.modeSettled = null;
    void LegibleEditor.setMode("${mode}").then(() => (window.modeSettled = "${mode}"))`);
  await pumped(`${mode} mode`, () => evaluate(`window.modeSettled === "${mode}"`));
}
const text = () => evaluate("LegibleEditor.view().state.doc.toString()");
/** Selects from `anchor` to `head` in the text, focused. */
const select = (anchor, head = anchor) =>
  evaluate(`(() => { const v = LegibleEditor.view(); v.dispatch({ selection: { anchor: ${anchor}, head: ${head} } }); v.focus(); })()`);
const selected = () =>
  evaluate("(() => { const s = LegibleEditor.view().state; return s.sliceDoc(s.selection.main.from, s.selection.main.to); })()");
const type = (chars) => cdp("Input.insertText", { text: chars });
const ctrlKey = (k, code, keyCode) => key(k, { code, keyCode, ctrl: true });
/** The preview has caught up with the text, and its diagrams are drawn. */
const caughtUp = () => pumped("the preview", () => evaluate("LegibleEditor.previewState().prepared"));

await test("Split and Edit: Ctrl+E goes View → Split → Edit → View, and the buttons follow", async () => {
  await open("showcase");
  const pressed = () =>
    evaluate(`[...document.querySelectorAll(".segmented button[data-mode]")]
      .filter((b) => b.getAttribute("aria-pressed") === "true").map((b) => b.dataset.mode).join()`);
  expect((await pressed()) === "view", "View pressed at first");
  await ctrlKey("e", "KeyE", 69);
  await pumped("Split", () => evaluate(`LegibleEditor.mode() === "split"`));
  expect((await pressed()) === "split", `pressed: ${await pressed()}`);
  // Its lines, whatever the checkout's line endings: CodeMirror's text is always LF (saving
  // keeps the file's own).
  const lines = readFileSync(sourceOf.showcase, "utf8").replace(/\r\n?/g, "\n");
  expect((await text()) === lines, "the editor has the file's text");
  await caughtUp();
  const split = await evaluate(`({
    editor: document.getElementById("editor").getBoundingClientRect().width,
    doc: getComputedStyle(document.getElementById("document")).overflowY,
    outline: getComputedStyle(document.getElementById("outline")).display,
    // The document's pane scrolls down only: a wide table scrolls in its own frame.
    sideways: (() => {
      const d = document.getElementById("document");
      return d.scrollWidth - d.clientWidth;
    })(),
  })`);
  expect(Math.abs(split.editor - 550) < 2 && split.doc === "auto" && split.outline === "none", JSON.stringify(split));
  expect(split.sideways === 0, `the document's pane scrolls sideways by ${split.sideways}px`);
  await ctrlKey("e", "KeyE", 69);
  await pumped("Edit", () => evaluate(`LegibleEditor.mode() === "edit"`));
  const edit = await evaluate(`({
    editor: document.getElementById("editor").getBoundingClientRect().width,
    doc: getComputedStyle(document.getElementById("document")).display,
    column: document.getElementById("editor").shadowRoot.querySelector(".cm-content").getBoundingClientRect().width,
  })`);
  // The text keeps to the reading column (Wide by default), not the window's whole width.
  expect(edit.editor === 1100 && edit.doc === "none" && edit.column < 1000, JSON.stringify(edit));
  await ctrlKey("e", "KeyE", 69);
  await pumped("View", () => evaluate(`LegibleEditor.mode() === "view"`));
  expect((await pressed()) === "view", "View pressed again");
  expect(await evaluate(`!document.documentElement.classList.contains("mode-split") &&
    getComputedStyle(document.getElementById("editor")).display === "none"`), "back in View");
  const modes = (await sent()).filter((m) => m.startsWith("mode\n"));
  expect(modes.join() === "mode\nsplit,mode\nedit,mode\nview", `sent: ${modes}`);
  // The buttons, found by name, switch to their modes. Each is an icon alone, as wide as the
  // toolbar's other buttons, its name and tooltip the mode's.
  const segments = await evaluate(`[...document.querySelectorAll(".segmented button")].map((b) => ({
    name: b.getAttribute("aria-label"), tip: b.title, text: b.textContent, icon: !!b.querySelector("svg.icon path"),
    width: b.getBoundingClientRect().width }))`);
  const openWidth = await evaluate(`document.querySelector('#toolbar [aria-label="Open"]').getBoundingClientRect().width`);
  expect(
    segments.map((b) => `${b.name}|${b.tip}`).join() === "View|View (Ctrl+E cycles),Split|Split (Ctrl+E cycles),Edit|Edit (Ctrl+E cycles)",
    JSON.stringify(segments),
  );
  expect(segments.every((b) => b.icon && b.text === "" && b.width === openWidth), `segments ${JSON.stringify(segments)}, Open ${openWidth}`);
  for (const mode of ["Edit", "Split", "View"]) {
    await evaluate(`document.querySelector('.segmented [aria-label="${mode}"]').click()`);
    await pumped(mode, () => evaluate(`LegibleEditor.mode() === "${mode.toLowerCase()}"`));
    expect((await pressed()) === mode.toLowerCase(), `${mode} clicked, pressed: ${await pressed()}`);
  }
});

await test("the editor: styled inside its shadow root, spell-checked, wrapped, with nothing refused", async () => {
  await open("short");
  await toMode("edit");
  const look = await evaluate(`(() => {
    const shadow = document.getElementById("editor").shadowRoot;
    const content = shadow.querySelector(".cm-content");
    return {
      sheets: shadow.adoptedStyleSheets.length,
      styleElements: document.querySelectorAll("style").length + shadow.querySelectorAll("style").length,
      spellcheck: content.getAttribute("spellcheck"),
      wrap: getComputedStyle(content).whiteSpace,
      gutters: !!shadow.querySelector(".cm-gutters"),
      font: getComputedStyle(content).fontFamily,
      heading: getComputedStyle(shadow.querySelector(".md-heading")).fontWeight,
    };
  })()`);
  expect(look.sheets > 0 && look.styleElements === 0, `styles: ${JSON.stringify(look)}`);
  expect(look.spellcheck === "true" && look.wrap === "break-spaces" && !look.gutters, JSON.stringify(look));
  expect(look.font.includes("Segoe UI") && look.heading === "600", JSON.stringify(look));
  // Monospace, then a theme change, then search: still nothing refused.
  await evaluate(`LegibleChrome.apply({ ...JSON.parse(document.getElementById("legible-settings").textContent),
    "editor-font": "monospace", theme: "dark" })`);
  const after = await evaluate(`(() => {
    const content = document.getElementById("editor").shadowRoot.querySelector(".cm-content");
    return { font: getComputedStyle(content).fontFamily, colour: getComputedStyle(content).color };
  })()`);
  expect(after.font.includes("Cascadia") && after.colour === "rgb(232, 232, 232)", JSON.stringify(after));
  await select(0);
  await ctrlKey("h", "KeyH", 72);
  await until("the replace field", () =>
    evaluate(`document.getElementById("editor").shadowRoot.activeElement?.name === "replace"`));
  expect(await evaluate(`document.getElementById("find").hidden`), "the page's find bar opened too");
  // Every word of the panel is Legible's (Fluent's), none CodeMirror's own.
  const panel = await evaluate(`(() => {
    const strings = JSON.parse(document.getElementById("legible-chrome-strings").textContent);
    const search = document.getElementById("editor").shadowRoot.querySelector(".cm-search");
    const words = [
      // A button named by aria-label (Close, its text a ×) is checked by that.
      ...[...search.querySelectorAll("button:not([aria-label]), label")].map((e) => e.textContent.trim()),
      ...[...search.querySelectorAll("input[placeholder]")].map((e) => e.placeholder),
      ...[...search.querySelectorAll("[aria-label]")].map((e) => e.getAttribute("aria-label")),
    ].filter(Boolean);
    const ours = new Set(Object.entries(strings).filter(([k]) => k.startsWith("editor-")).map(([, v]) => v));
    return { words, foreign: words.filter((w) => !ours.has(w)) };
  })()`);
  expect(panel.words.length >= 10 && panel.foreign.length === 0, `the panel's words: ${JSON.stringify(panel)}`);
});

await test("Edit: the text in the reading column, centred, and front matter as metadata, not a heading", async () => {
  await open("showcase");
  await toMode("edit");
  const look = await evaluate(`(() => {
    const shadow = document.getElementById("editor").shadowRoot;
    const content = shadow.querySelector(".cm-content").getBoundingClientRect();
    const scroller = shadow.querySelector(".cm-scroller").getBoundingClientRect();
    const style = getComputedStyle(shadow.querySelector(".cm-content"));
    const lines = [...shadow.querySelectorAll(".cm-line")].slice(0, 10);
    const close = lines.findIndex((l, i) => i > 0 && l.textContent === "---");
    return {
      left: content.left - scroller.left,
      right: scroller.right - content.right,
      padding: style.paddingLeft,
      lineHeight: parseFloat(style.lineHeight) / parseFloat(style.fontSize),
      linePadding: getComputedStyle(lines[1]).paddingLeft,
      meta: lines.slice(0, close + 1).every((l) => l.querySelector(".md-meta") && !l.querySelector(".md-heading")),
      close,
    };
  })()`);
  expect(look.padding === "32px" && look.linePadding === "0px", `padding: ${JSON.stringify(look)}`);
  expect(look.left > 40 && Math.abs(look.left - look.right) < 24, `not centred: ${JSON.stringify(look)}`);
  expect(Math.abs(look.lineHeight - 1.6) < 0.05, `line height: ${JSON.stringify(look)}`);
  expect(look.close > 1 && look.meta, `front matter: ${JSON.stringify(look)}`);
});

await test("shortcuts: Ctrl+B, Ctrl+I, Ctrl+K, Ctrl+1/2/3/0, Ctrl+F in the editor, undo", async () => {
  await open("short");
  await toMode("edit");
  const original = await text();
  const at = original.indexOf("Two");
  await select(at, at + 3);
  await ctrlKey("b", "KeyB", 66);
  expect((await text()).includes("**Two** headings") && (await selected()) === "Two", "bold");
  await ctrlKey("b", "KeyB", 66);
  expect((await text()) === original && (await selected()) === "Two", "bold off");
  await ctrlKey("i", "KeyI", 73);
  expect((await text()).includes("*Two* headings"), "italic");
  await ctrlKey("i", "KeyI", 73);
  await ctrlKey("k", "KeyK", 75);
  expect((await text()).includes("[Two](url) headings") && (await selected()) === "url", `link: ${await text()}`);
  await ctrlKey("z", "KeyZ", 90);
  expect((await text()) === original, "undo");
  await select(at);
  await ctrlKey("2", "Digit2", 50);
  expect((await text()).includes("\n## Two headings only."), "heading 2");
  await ctrlKey("1", "Digit1", 49);
  expect((await text()).includes("\n# Two headings only."), "heading 1");
  await ctrlKey("3", "Digit3", 51);
  expect((await text()).includes("\n### Two headings only."), "heading 3");
  await ctrlKey("0", "Digit0", 48);
  expect((await text()) === original, "Ctrl+0 makes it plain text");
  expect(await evaluate(`document.querySelector(".zoom-level").textContent === "100%"`), "Ctrl+0 reset the zoom");
  // Ctrl+F in the editor is the editor's find, not the page's.
  await ctrlKey("f", "KeyF", 70);
  await until("the editor's search panel", () => evaluate(`!!document.getElementById("editor").shadowRoot.querySelector(".cm-search")`));
  expect(await evaluate(`document.getElementById("find").hidden`), "the page's find bar opened");
});

await test("the menu bar: View, Split and Edit, and Find and Replace in the text", async () => {
  await open("short");
  const command = (name) => evaluate(`LegibleChrome.command(${JSON.stringify(name)})`);
  const shadow = `document.getElementById("editor").shadowRoot`;
  await command("mode-split");
  await pumped("Split", () => evaluate(`LegibleEditor.mode() === "split"`));
  await until("the editor focused", () => evaluate(`document.activeElement?.id === "editor"`));
  // In Split with the text focused, Find is the text's, as Ctrl+F there is.
  await command("find");
  await until("the editor's search panel", () => evaluate(`!!${shadow}.querySelector(".cm-search")`));
  expect(await evaluate(`document.getElementById("find").hidden`), "the page's find bar opened");
  await command("replace");
  await until("the replace field focused", () => evaluate(`${shadow}.activeElement?.name === "replace"`));
  await command("mode-edit");
  await pumped("Edit", () => evaluate(`LegibleEditor.mode() === "edit"`));
  await command("mode-view");
  await pumped("View", () => evaluate(`LegibleEditor.mode() === "view"`));
  // In View, Find is the page's, and Replace does nothing.
  await command("replace");
  await command("find");
  expect(await evaluate(`!document.getElementById("find").hidden`), "the page's find bar didn't open");
  // A file that can't be edited stays in View.
  await open("notes-read-only");
  await command("mode-split");
  await command("replace");
  await pump();
  expect(await evaluate(`LegibleEditor.mode() === "view"`), "a read-only file left View");
});

await test("Ctrl+A in the text selects the text alone: CodeMirror's select all", async () => {
  await open("short");
  await toMode("split");
  await select(0);
  await ctrlKey("a", "KeyA", 65);
  const sel = await evaluate("(() => { const s = LegibleEditor.view().state; return [s.selection.main.from, s.selection.main.to, s.doc.length]; })()");
  expect(sel[0] === 0 && sel[1] === sel[2], `the text's selection: ${sel}`);
  expect((await evaluate("getSelection().toString()")) === "" || (await evaluate("document.activeElement.id")) === "editor", "the page took a selection");
});

await test("the status bar: off at first, the menu and Settings toggle it, the path fitted and copied, the editing facts in Split and Edit", async () => {
  await open("short");
  const bar = () =>
    evaluate(`(() => {
      const b = document.getElementById("status-bar");
      const q = (c) => b.querySelector(c);
      return {
        shown: !b.hidden,
        height: b.offsetHeight,
        path: q(".status-path").textContent,
        title: q(".status-path").title,
        note: q(".status-note").hidden ? null : q(".status-note").textContent,
        edit: !q(".status-edit").hidden,
        words: q(".status-words").textContent,
        encoding: q(".status-encoding").textContent,
        endings: q(".status-endings").textContent,
        padding: getComputedStyle(document.body).paddingBottom,
      };
    })()`);
  // Off at first; the page asks what it would show.
  expect(!(await bar()).shown, "the status bar is on at first");
  expect((await sent()).includes("status"), "the page didn't ask for the status");
  // The View menu's item toggles the setting, which the window saves (and ticks the item by).
  await evaluate("window.sent.length = 0; LegibleChrome.command('status-bar')");
  expect((await bar()).shown && (await sent()).includes("setting\nstatus-bar\ntrue"), `on: ${JSON.stringify(await sent())}`);
  const full = String.raw`C:\Users\Someone\AppData\Local\Microsoft\Windows\INetCache\Content.Outlook\AB12CD34\Quarterly operations review.md`;
  await evaluate(`LegibleChrome.status(${JSON.stringify({ path: full, note: "Opened from an email attachment", encoding: "UTF-8 with BOM", endings: "Mixed (CRLF kept)" })})`);
  await until("the page to make room for the bar", async () => (await bar()).padding === `${(await bar()).height}px`);
  let shown = await bar();
  expect(shown.path === full && shown.title.startsWith(full) && shown.note === "Opened from an email attachment", `the file: ${JSON.stringify(shown)}`);
  // In View, nothing of the editor's.
  expect(!shown.edit, "the editing facts show in View");
  // In a narrow window the folders give way in the middle; the name stays whole.
  await cdp("Emulation.setDeviceMetricsOverride", { width: 520, height: 800, deviceScaleFactor: 1, mobile: false });
  try {
    await until("the path to fit", async () => (await bar()).path.includes("…"));
    shown = await bar();
    expect(shown.path.startsWith("C:\\…\\") && shown.path.endsWith(String.raw`\Quarterly operations review.md`), `fitted: ${shown.path}`);
    expect(shown.title.startsWith(full), "the tooltip lost the full path");
    expect(await evaluate(`(() => { const p = document.querySelector(".status-path"); return p.scrollWidth <= p.clientWidth; })()`), "it still overflows");
  } finally {
    await cdp("Emulation.setDeviceMetricsOverride", { width: 1100, height: 800, deviceScaleFactor: 1, mobile: false });
  }
  await until("the path whole again", async () => (await bar()).path === full);
  // A click asks the window to copy its own file's path; its answer shows a toast.
  await evaluate("window.sent.length = 0; document.querySelector('.status-path').click()");
  expect((await sent()).at(-1) === "copy-path", `the click sent ${JSON.stringify(await sent())}`);
  await evaluate("LegibleChrome.pathCopied()");
  expect(await evaluate("!document.getElementById('toast').hidden && document.getElementById('toast').textContent.includes('Path copied')"), "no toast");
  // In Split: the words, the encoding and the line endings; the panes end above the bar.
  await toMode("split");
  await until("the words", async () => (await bar()).words === "11 words");
  shown = await bar();
  expect(shown.edit && shown.encoding === "UTF-8 with BOM" && shown.endings === "Mixed (CRLF kept)", `in Split: ${JSON.stringify(shown)}`);
  const bottoms = await evaluate(`[document.getElementById("editor").getBoundingClientRect().bottom, document.getElementById("document").getBoundingClientRect().bottom, innerHeight]`);
  expect(bottoms[0] === bottoms[2] - shown.height && bottoms[1] === bottoms[0], `the panes' bottoms: ${bottoms}`);
  // The count follows the text: a link counts as its text, marks and addresses aren't words.
  await evaluate(`(() => { const v = LegibleEditor.view(); v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: "# Title\\n\\nOne **bold** [link](https://example.com/x)." } }); })()`);
  await until("the new count", async () => (await bar()).words === "4 words");
  const counts = await evaluate(`[
    LegibleEditor.countWords("---\\ntitle: Not counted\\n---\\nTwo words"),
    LegibleEditor.countWords("Visit https://example.com/a/b now"),
    LegibleEditor.countWords("<!-- a note -->Just *this*"),
    LegibleEditor.countWords("One"),
  ]`);
  expect(counts.join() === "2,3,2,1", `counts: ${counts}`);
  await evaluate(`(() => { const v = LegibleEditor.view(); v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: "Alone" } }); })()`);
  await until("one word", async () => (await bar()).words === "1 word");
  await toMode("view");
  expect(!(await bar()).edit, "the editing facts stay in View");
  // Settings: the same setting, both ways.
  await evaluate("LegibleChrome.command('settings')");
  expect(await evaluate("document.querySelector('input[name=status-bar]').checked"), "Settings doesn't show it on");
  await evaluate("window.sent.length = 0; document.querySelector('input[name=status-bar]').click()");
  expect(!(await bar()).shown && (await sent()).includes("setting\nstatus-bar\nfalse"), "Settings didn't turn it off");
  await key("Escape", { keyCode: 27 });
  // Another window turned it on: this one follows.
  await evaluate(`LegibleChrome.apply({ ...JSON.parse(document.getElementById("legible-settings").textContent), "status-bar": true })`);
  expect((await bar()).shown, "a setting from another window");
});

await test("the word count: an exact number's form first, else its category's, with every place of the count filled", async () => {
  // A page whose word count is as a translation might have it (i18n::tr_plurals' forms): a
  // form for exactly 0, a digit of its own, and the count twice over.
  const page = readFileSync(join(pages, "short.html"), "utf8").replace(
    /(<script type="application\/json" id="legible-chrome-strings">)(.*?)(<\/script>)/s,
    (_, start, json, end) => {
      const strings = JSON.parse(json);
      strings["status-words"] = {
        lang: "en-GB",
        forms: { one: "{count} word ({count})", other: "Section 0: {count} words ({count})" },
        exact: { 0: "No words" },
      };
      return start + JSON.stringify(strings).replaceAll("<", "\\u003c") + end;
    },
  );
  writeFileSync(join(pages, "plurals.html"), page);
  await open("plurals");
  const words = (count) =>
    evaluate(`(() => {
      document.dispatchEvent(new CustomEvent("legible-words", { detail: ${count} }));
      return document.querySelector(".status-words").textContent;
    })()`);
  const shown = [await words(0), await words(1), await words(2), await words(1234)];
  const want = ["No words", "1 word (1)", "Section 0: 2 words (2)", "Section 0: 1,234 words (1,234)"];
  expect(shown.join("|") === want.join("|"), `the counts: ${JSON.stringify(shown)}`);
});

await test("the status bar's path refits as the editing facts come, change and go", async () => {
  await open("short");
  await evaluate(`LegibleChrome.apply({ ...JSON.parse(document.getElementById("legible-settings").textContent), "status-bar": true })`);
  const path = String.raw`C:\Users\Someone\Documents\Reports\Quarterly operations review.md`;
  await evaluate(`LegibleChrome.status(${JSON.stringify({ path, note: null, encoding: "UTF-8 with BOM", endings: "Mixed (CRLF kept)" })})`);
  const shown = () =>
    evaluate(`(() => { const p = document.querySelector(".status-path"); return { text: p.textContent, fits: p.scrollWidth <= p.clientWidth }; })()`);
  // Wide enough for the whole path in View, but not beside the editing facts.
  await cdp("Emulation.setDeviceMetricsOverride", { width: 600, height: 800, deviceScaleFactor: 1, mobile: false });
  try {
    await until("the path whole in View", async () => (await shown()).text === path);
    await toMode("split");
    await until("the path shortened beside the facts", async () => {
      const now = await shown();
      return now.text.includes("…") && now.text.endsWith("Quarterly operations review.md") && now.fits;
    });
    // A longer count (as typing makes it) takes more room: the path gives way again.
    await evaluate(`document.dispatchEvent(new CustomEvent("legible-words", { detail: 123456789 }))`);
    await until("the path refitted for the longer count", async () => (await shown()).fits);
    await toMode("view");
    await until("the path whole again in View", async () => (await shown()).text === path);
  } finally {
    await cdp("Emulation.setDeviceMetricsOverride", { width: 1100, height: 800, deviceScaleFactor: 1, mobile: false });
  }
});

await test("unsaved: the dot follows the text, undo clears it, and a save marks only what it wrote", async () => {
  await open("short");
  await toMode("edit");
  const dot = () => evaluate(`!document.querySelector(".unsaved").hidden`);
  expect(!(await dot()), "no dot at first");
  await select(0);
  await type("x");
  await until("the dot", dot);
  expect((await sent()).includes("dirty\ntrue"), "dirty sent");
  await ctrlKey("z", "KeyZ", 90);
  await until("no dot", async () => !(await dot()));
  expect((await sent()).filter((m) => m.startsWith("dirty")).join() === "dirty\ntrue,dirty\nfalse", "dirty, then clean");
  await type("y");
  const asked = await evaluate("LegibleEditor.contents(7)");
  const reply = (await sent()).find((m) => m.startsWith("contents\n7\n"));
  expect(reply === `contents\n7\n${asked.version}\n${asked.text}` && asked.text.startsWith("y# A"), `contents: ${reply}`);
  // Typed while the save was writing, a change at a time and more of them than any history
  // would keep: still unsaved after it, and the text it wrote is what's saved.
  for (const c of "zzzzzzzzzzzz") await type(c);
  expect(await evaluate(`LegibleEditor.markSaved(${asked.version})`), "markSaved knows the version");
  expect(await dot(), "the newer edits aren't saved");
  await evaluate(`(() => { const v = LegibleEditor.view();
    v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: ${JSON.stringify(asked.text)} } }); })()`);
  await until("no dot back at the saved text", async () => !(await dot()));
  expect(!(await evaluate(`LegibleEditor.markSaved(${asked.version})`)), "a finished save is still kept");
  // An abandoned save is let go; so is the oldest of more than four in flight.
  await type("a");
  const abandoned = await evaluate("LegibleEditor.contents().version");
  expect(await evaluate(`LegibleEditor.saveAbandoned(${abandoned})`), "saveAbandoned");
  expect(!(await evaluate(`LegibleEditor.markSaved(${abandoned})`)), "an abandoned save marked saved");
  const versions = [];
  for (const c of "bcdef") {
    await type(c);
    versions.push(await evaluate("LegibleEditor.contents().version"));
  }
  expect(!(await evaluate(`LegibleEditor.markSaved(${versions[0]})`)), "more than four saves kept");
  // A newer save done ends the older ones: one finishing late can't take the text back.
  expect(await evaluate(`LegibleEditor.markSaved(${versions[4]})`), "the newest save");
  expect(!(await evaluate(`LegibleEditor.markSaved(${versions[3]})`)), "an older save after a newer one");
  await until("no dot after saving all", async () => !(await dot()));
  expect(await evaluate("LegibleEditor.isDirty() === false"), "isDirty");
});

await test("unsaved: the window has the text before it hears the text is unsaved", async () => {
  await open("short");
  await toMode("edit");
  const text = () => evaluate("LegibleEditor.view().state.doc.toString()");
  // What the window has heard, in order, from the editor's text and dirty messages.
  const heard = async () =>
    (await sent()).filter((m) => m.startsWith("text\n") || m.startsWith("dirty\n")).map((m) => m.split("\n")[0] === "text" ? `text:${m.slice(m.indexOf("\n", 5) + 1)}` : m);
  await select(0);
  await evaluate("window.sent.length = 0");
  await type("x");
  const typed = await text();
  expect((await heard()).join("|") === `text:${typed}|dirty\ntrue`, `an edit: ${JSON.stringify(await heard())}`);
  // Saved, and undone to the text before while the save was writing: once it's done, unsaved
  // again, its text first (it wasn't sent while it matched what was saved before).
  const asked = await evaluate("LegibleEditor.contents()");
  await ctrlKey("z", "KeyZ", 90);
  await until("the edit undone", async () => !(await evaluate("LegibleEditor.isDirty()")));
  const undone = await text();
  await evaluate("window.sent.length = 0");
  expect(await evaluate(`LegibleEditor.markSaved(${asked.version})`), "markSaved");
  expect((await heard()).join("|") === `text:${undone}|dirty\ntrue`, `a save over an undo: ${JSON.stringify(await heard())}`);
});

await test("Ctrl+S saves and Ctrl+Shift+S saves a copy, in any mode", async () => {
  await open("short");
  await key("s", { code: "KeyS", keyCode: 83, ctrl: true });
  await toMode("edit");
  await select(0);
  await type("x");
  await key("s", { code: "KeyS", keyCode: 83, ctrl: true });
  await key("S", { code: "KeyS", keyCode: 83, ctrl: true, shift: true });
  const asked = (await sent()).filter((m) => m === "save" || m === "save-copy");
  expect(asked.join() === "save,save,save-copy", `sent: ${asked}`);
  expect((await text()).startsWith("x# A"), "the keys typed into the text");
});

// The conflict bar's buttons, by their text.
const barButtons = () => evaluate(`[...document.querySelectorAll("#conflict-bar button")].map((b) => b.textContent)`);
const barText = () => evaluate(`document.querySelector("#conflict-bar .conflict-text")?.textContent ?? ""`);
const press = (label) =>
  evaluate(`[...document.querySelectorAll("#conflict-bar button")].find((b) => b.textContent === ${JSON.stringify(label)}).click()`);
// The bar's words, as the window sends them: they name the file.
const conflictText = {
  changed: "short.md changed while you were editing it.",
  deleted: "short.md was moved or deleted. Keep your changes to save it again, or close it.",
};
const change = (detail) =>
  evaluate(
    `window.dispatchEvent(new CustomEvent("legible:external-change", { detail: ${JSON.stringify({ text: conflictText[detail.kind], ...detail })} }))`,
  );
const conflictState = () => evaluate("LegibleEditor.conflictState()");

await test("the conflict bar: Keep my changes, Use the file once confirmed, and Compare until typing", async () => {
  await open("short");
  await toMode("edit");
  await select(0);
  await type("mine ");
  await caughtUp();
  await change({ kind: "changed", id: 3 });
  expect((await barText()) === conflictText.changed, `text: ${await barText()}`);
  expect((await barButtons()).join() === "Keep my changes,Use the file,Compare", `buttons: ${await barButtons()}`);
  // Under the toolbar, like the chrome's other bars, and the panes start below it.
  await until("the bar stacked under the toolbar", () =>
    evaluate(`(() => {
      const bar = document.getElementById("conflict-bar").getBoundingClientRect();
      const toolbar = document.querySelector("[role=toolbar]")?.getBoundingClientRect() ?? { bottom: 40 };
      const pane = document.getElementById("editor").getBoundingClientRect();
      return Math.abs(bar.top - toolbar.bottom) < 1 && Math.abs(pane.top - bar.bottom) < 1;
    })()`),
  );

  // Compare: theirs in the preview, beside the text, until the text changes.
  await press("Compare");
  expect((await sent()).includes("compare\n3"), "compare sent");
  const theirs = `<h1 id="theirs">Theirs</h1>`;
  expect(await evaluate(`LegibleEditor.compare("changed", 3, ${JSON.stringify(theirs)}, ${JSON.stringify(previewInfo)})`), "compared");
  expect(await evaluate(`LegibleEditor.mode() === "split"`), "Compare is Split");
  expect(await evaluate(`!!document.getElementById("theirs")`), "theirs isn't in the preview");
  expect((await barText()).includes("Comparing: the preview shows the file, the editor your changes."), `text: ${await barText()}`);
  expect((await barButtons()).join() === "Keep my changes,Use the file,Stop comparing", `comparing: ${await barButtons()}`);
  // A render of the text that comes meanwhile doesn't replace theirs.
  await pump();
  await evaluate(`LegibleEditor.preview(LegibleEditor.contents().version, "<p>mine</p>", ${JSON.stringify(previewInfo)})`);
  expect(await evaluate(`!!document.getElementById("theirs")`), "a render replaced theirs while comparing");
  // An answer for another change shows nothing.
  expect(!(await evaluate(`LegibleEditor.compare("changed", 2, "<p id=old></p>", null)`)), "an older change compared");
  await type("!");
  await caughtUp();
  expect(await evaluate(`!document.getElementById("theirs")`), "typing didn't end Compare");
  expect((await conflictState()).bar && !(await conflictState()).comparing, "the bar went with Compare");
  expect((await barText()) === conflictText.changed, `text after Compare: ${await barText()}`);
  // Stop comparing: the text's own preview again, and the bar asks as before.
  await press("Compare");
  expect(await evaluate(`LegibleEditor.compare("changed", 3, ${JSON.stringify(theirs)}, ${JSON.stringify(previewInfo)})`), "compared again");
  await press("Stop comparing");
  await caughtUp();
  expect(await evaluate(`!document.getElementById("theirs")`), "Stop comparing left theirs");
  expect((await barButtons()).join() === "Keep my changes,Use the file,Compare", `after Stop comparing: ${await barButtons()}`);

  // Use the file asks first; Cancel goes back.
  await press("Use the file");
  expect((await barButtons()).join() === "Use the file,Cancel", `confirm: ${await barButtons()}`);
  expect(!(await sent()).includes("reload\n3"), "taken before it was confirmed");
  await press("Cancel");
  expect((await barButtons()).join() === "Keep my changes,Use the file,Compare", `after Cancel: ${await barButtons()}`);
  await press("Use the file");
  await press("Use the file");
  expect((await sent()).includes("reload\n3"), "reload sent");
  expect(!(await conflictState()).bar, "the bar stayed");
  // Confirmed, the text is frozen before the answer goes: nothing typed until the window's
  // take is over can be replaced.
  expect(await evaluate("LegibleEditor.isFrozen()"), "not frozen once Use the file was confirmed");
  const beforeTake = await text();
  await type("after the confirmation");
  expect((await text()) === beforeTake, "an edit landed after Use the file was confirmed");
  await evaluate("LegibleEditor.unfreeze()");

  // A newer change replaces the question; Keep my changes answers for it.
  await change({ kind: "changed", id: 4 });
  await change({ kind: "changed", id: 5 });
  await press("Keep my changes");
  expect((await sent()).includes("keep\n5") && !(await sent()).includes("keep\n4"), "keep for the newest");
  expect(!(await conflictState()).bar, "the bar stayed after Keep my changes");

  // A file that went: Keep my changes, or Close (which asks about the edits).
  await change({ kind: "deleted", id: 6 });
  expect((await barButtons()).join() === "Keep my changes,Close", `deleted: ${await barButtons()}`);
  await press("Close");
  expect((await sent()).includes("close"), "close sent");

  // The file from disk answers a held change's question...
  await change({ kind: "changed", id: 7 });
  await evaluate(`LegibleEditor.fromDisk("# From disk\\n")`);
  expect(!(await conflictState()).bar, "the bar stayed after the file came from disk");
});

await test("a raced save: the bar says both are kept, and answers as raced", async () => {
  await open("short");
  await toMode("split");
  await caughtUp();
  const text = "short.md was saved at the same moment you saved. Both are kept: your changes are in short.md, and the other version is in the copy \"short (other version).md\".";
  await change({ kind: "raced", id: 2, text });
  expect((await barText()) === text, `text: ${await barText()}`);
  expect((await barButtons()).join() === "Keep my changes,Use the other version,Compare", `buttons: ${await barButtons()}`);
  // Live reload's change from disk doesn't answer it: both versions are still kept.
  await evaluate(`LegibleEditor.fromDisk("# Again\\n")`);
  expect((await conflictState()).bar, "the raced bar went with a change from disk");
  await press("Compare");
  await press("Use the other version");
  expect((await barButtons()).join() === "Use the other version,Cancel", `confirm: ${await barButtons()}`);
  await press("Use the other version");
  const raced = (await sent()).filter((m) => m.startsWith("raced\n"));
  expect(raced.join() === "raced\ncompare\n2,raced\ntake\n2", `sent: ${raced}`);
  expect(await evaluate("LegibleEditor.isFrozen()"), "a raced Use the other version didn't freeze the text");
  // Frozen, the bar answers nothing.
  await change({ kind: "raced", id: 3, text });
  await press("Keep my changes");
  expect(!(await sent()).includes("raced\nkeep\n3"), "Keep my changes answered while frozen");
  await evaluate("LegibleEditor.unfreeze()");
  await change({ kind: "raced", id: 3, text });
  await press("Keep my changes");
  expect((await sent()).includes("raced\nkeep\n3"), "raced keep");
});

await test("unsaved changes kept as Windows ended the session: offered, Restore puts them in unsaved, Discard drops them", async () => {
  await open("short");
  const question = "Restore unsaved changes to short.md? short.md has changed since then.";
  await evaluate(`LegibleEditor.offerRestore(${JSON.stringify(question)})`);
  expect((await barText()) === question, `text: ${await barText()}`);
  expect((await barButtons()).join() === "Restore,Discard", `buttons: ${await barButtons()}`);
  await press("Restore");
  expect((await sent()).at(-1) === "restore\nyes", `Restore sent ${JSON.stringify((await sent()).at(-1))}`);
  expect(!(await conflictState()).bar, "the bar stayed");
  // The window's answer: from View, the editor opens in Split with the text as an edit.
  await evaluate(`window.restored = null;
    void LegibleEditor.restore("# Restored\\n").then((done) => (window.restored = done))`);
  await pumped("the restore", () => evaluate("window.restored === true"));
  expect((await text()) === "# Restored\n", `text: ${await text()}`);
  expect(await evaluate("LegibleEditor.isDirty() && LegibleEditor.mode() === 'split'"), "not unsaved in Split");
  expect((await sent()).includes("dirty\ntrue"), "the window wasn't told it's unsaved");
  expect((await sent()).at(-1) === "restored", `after the text: ${JSON.stringify((await sent()).at(-1))}`);
  // Each edit while unsaved sends the text at once, for Windows ending the session.
  const texts = (await sent()).filter((m) => m.startsWith("text\n"));
  expect(texts.at(-1)?.endsWith("\n# Restored\n"), `text sent: ${JSON.stringify(texts.at(-1))}`);
  await select(0);
  await type("x");
  expect((await sent()).at(-1).endsWith("\nx# Restored\n"), `typed: ${JSON.stringify((await sent()).at(-1))}`);
  // Offered again, Discard answers and nothing changes.
  await evaluate(`LegibleEditor.offerRestore(${JSON.stringify(question)})`);
  await press("Discard");
  expect((await sent()).at(-1) === "restore\nno", `Discard sent ${JSON.stringify((await sent()).at(-1))}`);
  expect(!(await conflictState()).bar && (await text()) === "x# Restored\n", "Discard changed the text, or the bar stayed");
});

await test("a close's save freezes the text: no edit lands between saving and the answer", async () => {
  await open("short");
  await toMode("edit");
  await select(0);
  await type("a");
  await evaluate("LegibleEditor.freeze()");
  // Typed, or put in by script: neither changes the frozen text.
  await type("b");
  await evaluate(`LegibleEditor.view().dispatch({ changes: { from: 0, insert: "c" } })`);
  expect((await text()).startsWith("a# A"), `frozen text changed: ${(await text()).slice(0, 8)}`);
  const asked = await evaluate("LegibleEditor.contents(1)");
  expect(await evaluate(`LegibleEditor.markSaved(${asked.version})`), "markSaved");
  // Between the window's question and the page's answer, an edit still can't land.
  await type("d");
  await evaluate("LegibleEditor.closeCheck()");
  await type("e");
  expect((await sent()).filter((m) => m.startsWith("clean")).join() === "clean\ntrue", `clean: ${await sent()}`);
  expect((await text()).startsWith("a# A"), "an edit landed while frozen");
  expect(await evaluate("LegibleEditor.isDirty() === false"), "dirty while frozen");
  // The file from disk still goes in, as saved text.
  await evaluate(`LegibleEditor.fromDisk("# Disk\\n")`);
  expect((await text()) === "# Disk\n", `fromDisk while frozen: ${await text()}`);
  // The window stays open: the text takes edits again, and asked again it isn't clean.
  await evaluate("LegibleEditor.unfreeze()");
  await select(0);
  await type("f");
  expect((await text()).startsWith("f# Disk"), "unfrozen, the text took no edit");
  await evaluate("LegibleEditor.freeze()");
  await evaluate("LegibleEditor.closeCheck()");
  expect((await sent()).filter((m) => m.startsWith("clean")).join() === "clean\ntrue,clean\nfalse", "clean false");

  // Asked with no save of the close's own (a close while an ordinary save was written): the
  // page freezes as it answers, so an edit after a clean answer can't land.
  await evaluate("LegibleEditor.unfreeze()");
  const saved = await evaluate("LegibleEditor.contents()");
  await evaluate(`LegibleEditor.markSaved(${saved.version})`);
  await evaluate("LegibleEditor.closeCheck()");
  await type("after clean");
  await evaluate(`LegibleEditor.view().dispatch({ changes: { from: 0, insert: "after clean" } })`);
  expect(!(await text()).includes("after clean"), "an edit landed after a clean answer");
  expect(await evaluate("LegibleEditor.isFrozen() && !LegibleEditor.isDirty()"), "not frozen and clean");
});

await test("a file that can't be edited where it is: Split and Edit unusable, Ctrl+Shift+S saves a copy, Ctrl+E nothing", async () => {
  await open("notes-read-only");
  const buttons = await evaluate(`[...document.querySelectorAll(".segmented button")]
    .filter((b) => !b.hidden).map((b) => [b.getAttribute("aria-label"), b.getAttribute("aria-disabled") === "true", b.title])`);
  // Only the modes: Save a copy… is File's (and Ctrl+Shift+S).
  expect(buttons.map(([name]) => name).join() === "View,Split,Edit", `buttons: ${JSON.stringify(buttons)}`);
  expect(!buttons[0][1] && buttons[1][1] && buttons[2][1], `disabled: ${JSON.stringify(buttons)}`);
  expect(buttons[0][2] === "View (Ctrl+E cycles)", buttons[0][2]);
  expect(buttons[1][2].startsWith("Split: this file can't be edited") && buttons[1][2].includes("Ctrl+Shift+S"), buttons[1][2]);
  expect(buttons[2][2].startsWith("Edit: this file can't be edited"), buttons[2][2]);
  await evaluate(`document.querySelector('.segmented [aria-label="Split"]').click()`);
  await ctrlKey("e", "KeyE", 69);
  await key("S", { code: "KeyS", keyCode: 83, ctrl: true, shift: true });
  await pump();
  const messages = await sent();
  expect(messages.includes("save-copy") && !messages.includes("source"), `sent: ${messages}`);
  expect(await evaluate(`LegibleEditor.mode() === "view"`), "left View");
});

await test("the divider: arrows and Home/End resize Split, saved; a new setting moves it", async () => {
  await open("short");
  await toMode("split");
  await evaluate(`document.querySelector(".editor-divider").focus()`);
  expect(await evaluate(`document.activeElement?.classList.contains("editor-divider")`), "the divider has no focus");
  await key("ArrowRight", { keyCode: 39 });
  const share = () => evaluate(`document.documentElement.style.getPropertyValue("--split")`);
  expect((await share()) === "52", `after → ${await share()}`);
  await key("End", { keyCode: 35 });
  expect((await share()) === "80", `after End ${await share()}`);
  const saved = (await sent()).filter((m) => m.startsWith("setting\nsplit\n"));
  expect(saved.join() === "setting\nsplit\n52,setting\nsplit\n80", `saved: ${saved}`);
  await evaluate(`LegibleChrome.apply({ ...JSON.parse(document.getElementById("legible-settings").textContent), split: 35 })`);
  expect((await share()) === "35", "a new setting");
  const width = await evaluate(`document.getElementById("editor").getBoundingClientRect().width`);
  expect(Math.abs(width - 385) < 2, `the editor is ${width}px`);
});

await test("the divider: a 10 px grab area by a thin line, clear of the scrollbar, a handle on hover, focus and drag, double-click to 50/50", async () => {
  await open("short");
  await toMode("split");
  await evaluate(`LegibleChrome.apply({ ...JSON.parse(document.getElementById("legible-settings").textContent), split: 35 })`);
  const look = () =>
    evaluate(`(() => {
      const d = document.querySelector(".editor-divider");
      const r = d.getBoundingClientRect();
      return {
        width: r.width, line: r.left + 1, x: r.left + r.width / 2, y: r.top + r.height / 2,
        lineWidth: parseFloat(getComputedStyle(d, "::before").width),
        // Just left of the line, the editor's scrollbar: never the divider's.
        left: document.elementFromPoint(document.getElementById("editor").getBoundingClientRect().right - 2, r.top + r.height / 2)?.id,
        grip: getComputedStyle(d, "::after").opacity,
        dragging: d.classList.contains("dragging"),
        editor: document.getElementById("editor").getBoundingClientRect().width,
      };
    })()`);
  const idle = await look();
  // The grab area is wide, the line thin, and the panes where the share puts them.
  expect(idle.width >= 8 && idle.lineWidth >= 1 && idle.lineWidth <= 2 && idle.grip === "0", `idle: ${JSON.stringify(idle)}`);
  expect(Math.abs(idle.editor - 385) < 2 && Math.abs(idle.line - 385) < 1, `the panes moved: ${JSON.stringify(idle)}`);
  expect(idle.left === "editor", `the divider takes the editor's scrollbar: ${JSON.stringify(idle)}`);
  const mouse = (type, x, y, extra = {}) => cdp("Input.dispatchMouseEvent", { type, x, y, button: "left", ...extra });
  // Hovered: the handle shows; away again, it goes.
  await mouse("mouseMoved", idle.x + 4, idle.y);
  const hovered = await look();
  expect(hovered.grip === "1" && hovered.lineWidth === 2, `hover: ${JSON.stringify(hovered)}`);
  await mouse("mouseMoved", 800, 300);
  expect((await look()).grip === "0", "the handle stayed after the pointer left");
  // Dragged: the handle stays while the pointer is away from it, until it's let go.
  await evaluate("window.sent.length = 0");
  await mouse("mousePressed", idle.x, idle.y, { clickCount: 1 });
  await mouse("mouseMoved", 550, 300, { buttons: 1 });
  const dragging = await look();
  expect(dragging.dragging && dragging.grip === "1" && Math.abs(dragging.line - 550) < 6, `dragging: ${JSON.stringify(dragging)}`);
  await mouse("mouseReleased", 550, 300, { clickCount: 1 });
  await mouse("mouseMoved", 800, 300);
  const dropped = await look();
  expect(!dropped.dragging && dropped.grip === "0", `let go: ${JSON.stringify(dropped)}`);
  expect((await sent()).includes("setting\nsplit\n50"), `the drag wasn't saved: ${JSON.stringify(await sent())}`);
  // A double-click puts it back in the middle, saved as a drag is.
  await evaluate(`LegibleChrome.apply({ ...JSON.parse(document.getElementById("legible-settings").textContent), split: 30 })`);
  await evaluate("window.sent.length = 0");
  const at = await look();
  for (const clickCount of [1, 2]) {
    await mouse("mousePressed", at.x, at.y, { clickCount });
    await mouse("mouseReleased", at.x, at.y, { clickCount });
  }
  const share = await evaluate(`document.documentElement.style.getPropertyValue("--split")`);
  expect(share === "50" && (await sent()).includes("setting\nsplit\n50"), `double-click: ${share}, ${JSON.stringify(await sent())}`);
  await mouse("mouseMoved", 800, 300);
  // Focused from the keyboard: the same highlight.
  await evaluate(`document.querySelector(".editor-divider").focus()`);
  await key("ArrowLeft", { keyCode: 37 });
  const focused = await look();
  expect(focused.grip === "1" && focused.lineWidth === 2, `focus: ${JSON.stringify(focused)}`);
});

await test("the preview: typing shows through core, and an answer older than the text is never shown", async () => {
  await open("short");
  await toMode("split");
  await caughtUp();
  // Held: rendered, answered after another keystroke but before that one's render is sent.
  hold = true;
  await select(await evaluate("LegibleEditor.view().state.doc.length"));
  await type("\nFirst addition.\n");
  await pumped("a render", () => held.length === 1);
  await type("Second addition.\n");
  const before = (await sent()).length;
  await evaluate(held.shift());
  expect((await sent()).length === before, "the next render went out before the old answer");
  expect(!(await evaluate(`document.getElementById("document").textContent.includes("First addition.")`)), "the old answer was shown");
  hold = false;
  await pumped("the latest", () => evaluate(`document.getElementById("document").textContent.includes("Second addition.")`));
  // Modes changed quickly while a render is on its way, then View: the latest text shows.
  hold = true;
  await type("Third addition.\n");
  await evaluate(`LegibleEditor.setMode("edit"); LegibleEditor.setMode("split"); LegibleEditor.setMode("view")`);
  await pumped("the render View asked for", () => held.length >= 1);
  hold = false;
  for (const answer of held.splice(0)) await evaluate(answer);
  await pumped("View shows the latest", () => evaluate(`document.getElementById("document").textContent.includes("Third addition.")`));
  expect(await evaluate(`LegibleEditor.mode() === "view"`), "in View");
});

await test("a change on disk goes to the open editor: its text, saved, the cursor kept, and Updated", async () => {
  await open("short");
  // Changed while the editor loads: the editor opens on the newer text.
  await evaluate(`void LegibleEditor.setMode("split"); LegibleEditor.fromDisk("# Newer\\n\\nText.\\n")`);
  await pumped("split mode", () => evaluate(`LegibleEditor.mode() === "split"`));
  expect((await text()) === "# Newer\n\nText.\n", `opened on: ${JSON.stringify(await text())}`);
  await caughtUp();
  await select(3);
  const renders = () => sent().then((all) => all.filter((m) => m.startsWith("render\n")).length);
  const before = await renders();
  await evaluate(`LegibleEditor.fromDisk("# Newer\\n\\nText.\\n\\nFrom disk.\\n")`);
  await pumped("the preview of it", () => evaluate(`document.getElementById("document").textContent.includes("From disk.")`));
  await caughtUp();
  expect((await text()).endsWith("From disk.\n"), "the text");
  expect(await evaluate("LegibleEditor.isDirty() === false"), "it's what's saved");
  expect(!(await sent()).includes("dirty\ntrue"), "dirty was sent");
  expect(await evaluate("LegibleEditor.view().state.selection.main.head === 3"), "the cursor moved");
  expect(await evaluate(`!document.getElementById("updated").hidden`), "Updated isn't shown");
  // The same text (a picture changed): rendered again all the same.
  const after = await renders();
  expect(after === before + 1, `${after - before} renders`);
  await evaluate(`LegibleEditor.fromDisk("# Newer\\n\\nText.\\n\\nFrom disk.\\n")`);
  await pumped("another render", async () => (await renders()) === after + 1);
});

await test("diagrams: unchanged ones move into the new preview; ids stay unique and sheets don't pile up", async () => {
  await open("diagrams");
  await toMode("split");
  const drawn = () => evaluate(`document.querySelectorAll("#document .figure svg").length`);
  await pumped("the diagrams drawn", async () => (await drawn()) === 6);
  await evaluate(`document.querySelectorAll("#document .figure svg").forEach((svg) => (svg.legibleKept = true))`);
  // The page's own sheets (the print sheet, say) stay as they are; each drawn figure adds one.
  const others = await evaluate(`document.adoptedStyleSheets.length - 6`);
  const state = () =>
    evaluate(`(() => {
      const ids = [...document.querySelectorAll("[id]")].map((e) => e.id);
      const refs = [...document.querySelectorAll("#document .figure svg [marker-end]")]
        .map((e) => /url\\(#([^)]+)\\)/.exec(e.getAttribute("marker-end"))?.[1]).filter(Boolean);
      return {
        unique: new Set(ids).size === ids.length,
        refs: refs.length > 0 && refs.every((id) => document.getElementById(id)),
        sheets: document.adoptedStyleSheets.length,
        figures: document.querySelectorAll("#document .figure svg").length,
        kept: [...document.querySelectorAll("#document .figure svg")].filter((s) => s.legibleKept).length,
      };
    })()`);
  // Text between the diagrams changes: every diagram is the same one, moved.
  const between = (await text()).indexOf("Between them.");
  await select(between + "Between them".length);
  await type(" More text");
  await pumped("the preview", () => evaluate(`document.getElementById("document").textContent.includes("Between them More text.")`));
  let now = await state();
  expect(now.kept === 6 && now.unique && now.refs && now.sheets === now.figures + others, `after text: ${JSON.stringify(now)}`);
  // The last diagram changes 35 times: the others stay, and the sheets never pile up.
  for (let i = 0; i < 35; i++) {
    const t = await text();
    const at = t.lastIndexOf("--> D");
    await select(at + 5);
    await type(String(i % 10));
    await caughtUp();
  }
  now = await state();
  expect(now.kept === 4 && now.unique && now.refs && now.sheets === 6 + others, `after churn: ${JSON.stringify(now)}`);
});

await test("diagrams: one whose drawing was cut short isn't moved into a new preview half drawn", async () => {
  // Before the page's own scripts, so it hears the frame's answers first: while
  // window.holdAnswers is set, the frame's next answer (a changed diagram's light figure) goes
  // through and the one after it (its dark figure) is held back, as a slow frame would.
  const { identifier } = await cdp("Page.addScriptToEvaluateOnNewDocument", {
    source: `window.heldAnswers = [];
      window.addEventListener("message", (event) => {
        if (!window.holdAnswers || event.data?.svg === undefined) return;
        if (window.holdAnswers-- > 1) return;
        event.stopImmediatePropagation();
        window.heldAnswers.push({ data: event.data, source: event.source });
      });`,
  });
  try {
    await open("diagrams");
  } finally {
    await cdp("Page.removeScriptToEvaluateOnNewDocument", { identifier });
  }
  await toMode("split");
  await caughtUp();
  await pumped("the diagrams drawn", () => evaluate(`document.querySelectorAll("#document .figure svg").length === 6`));
  await evaluate("window.holdAnswers = 2");
  const t = await text();
  await select(t.lastIndexOf("--> D") + 5);
  await type("2");
  await pumped("the dark figure held", () => evaluate(`window.heldAnswers.length === 1`));
  expect(await evaluate(`document.querySelectorAll("#document .figure.light svg").length === 3`), "the light figure drawn");
  // Text elsewhere changes before the held answer comes: a new preview.
  await select((await text()).indexOf("Between them.") + "Between them".length);
  await type(" More");
  await pumped("the new preview shown", () => evaluate("LegibleEditor.previewState().current"));
  // The old answer at last, once the new preview is shown: too late to land anywhere. (The
  // frame answers in turn, so the new preview's drawing waits for it.)
  await evaluate(`window.heldAnswers.forEach(({ data, source }) =>
    window.dispatchEvent(new MessageEvent("message", { data, source, origin: "null" })))`);
  await caughtUp();
  const drawn = await evaluate(`[...document.querySelectorAll("#document .diagram")].map((d) => LegibleDoc.isDrawn(d))`);
  expect(drawn.length === 3 && drawn.every(Boolean), `drawn in both themes: ${JSON.stringify(drawn)}`);
  expect(await evaluate(`document.querySelectorAll("#document .figure.dark svg").length === 3`), "a dark figure missing");
});

await test("pictures in the preview: only the approved ones, the render's generation for the bar, and taken away at once", async () => {
  await open("pictures");
  const raw = await evaluate(`[...document.querySelectorAll("#document img[data-remote-src]")].map((i) => i.getAttribute("data-remote-src"))`);
  const remote = (generation, show) => ({ generation, show, ask: true });
  previewInfo = { generation: 0, remote: remote(0, [raw[0]]) };
  await evaluate(`LegibleChrome.remoteImages(${JSON.stringify(previewInfo.remote)})`);
  await toMode("split");
  await caughtUp();
  const pictures = () => evaluate(`[...document.querySelectorAll("#document img[data-remote-src]")].map((i) => i.getAttribute("src"))`);
  expect(JSON.stringify(await pictures()) === JSON.stringify([raw[0], null]), `the preview's: ${JSON.stringify(await pictures())}`);
  // A picture typed new, on an approved origin: no request until it's approved itself.
  requests.length = 0;
  await select(await evaluate("LegibleEditor.view().state.doc.length"));
  await type("\n\n![New](https://legible-doc.localhost/typed-new.svg)\n");
  await caughtUp();
  expect((await pictures()).length === 3 && (await pictures())[2] === null, `the new one: ${JSON.stringify(await pictures())}`);
  expect(!requests.some((r) => r.includes("typed-new")), `requested: ${requests}`);
  // The text as it now is on disk, in render 3: the bar's answers are for render 3.
  previewInfo = { generation: 3, remote: remote(3, [raw[0]]) };
  await evaluate(`LegibleEditor.fromDisk(${JSON.stringify(fileText("pictures"))})`);
  await pumped("render 3", () => evaluate(`document.getElementById("document").dataset.generation === "3"`));
  await caughtUp();
  await evaluate(`[...document.querySelectorAll("#pictures-bar button")].find((b) => b.textContent === "Show once").click()`);
  expect((await sent()).at(-1) === "remote-images\nonce\n3", `answered: ${JSON.stringify((await sent()).at(-1))}`);
  // They can't be checked: taken away at once, whichever render it's said for.
  await evaluate(`LegibleChrome.remoteImages({ generation: 0, show: [], ask: false, unavailable: "failed" })`);
  await until("the pictures taken away", async () => (await pictures()).every((src) => src === null));
  // No preview after brings the old list back.
  await select(0);
  await type("Still editable. ");
  await caughtUp();
  expect((await pictures()).every((src) => src === null), `brought back: ${JSON.stringify(await pictures())}`);
});

await test("a dirty editor holds every reload: the pictures bar says to save first, and asks again once saved", async () => {
  await open("pictures");
  const bar = `document.getElementById("pictures-bar")`;
  const state = () =>
    evaluate(`({ shown: !${bar}.hidden, text: ${bar}.querySelector(".file-bar-text").textContent,
      answers: !${bar}.querySelector(".pictures-actions").hidden })`);
  await evaluate(`LegibleChrome.remoteImages({ generation: 0, show: [], ask: true })`);
  let now = await state();
  expect(now.shown && now.answers && now.text.includes("Show them?"), `asking: ${JSON.stringify(now)}`);
  // Unsaved changes: nothing to answer, since the reload that would show them waits.
  await toMode("edit");
  await select(0);
  await type("x");
  await until("the bar to say save first", async () => (await state()).text.startsWith("Save your changes"));
  now = await state();
  expect(now.shown && !now.answers, `while dirty: ${JSON.stringify(now)}`);
  // A reload the window holds (a setting, say) says so too, and goes once saved.
  await evaluate(`LegibleChrome.remoteImages({ generation: 0, show: [], ask: false })`);
  expect(!(await state()).shown, "no bar with nothing to ask");
  await evaluate("LegibleChrome.reloadWaits(true)");
  expect((await state()).text.startsWith("Save your changes"), "a held reload unsaid");
  // Undone (as saved): the held reload is let go, and the bar asks again if there's anything
  // to ask.
  await ctrlKey("z", "KeyZ", 90);
  await until("clean", () => evaluate("!LegibleEditor.isDirty()"));
  expect(!(await state()).shown, "a bar left after the edits were saved");
  await evaluate(`LegibleChrome.remoteImages({ generation: 0, show: [], ask: true })`);
  now = await state();
  expect(now.shown && now.answers && now.text.includes("Show them?"), `asking again: ${JSON.stringify(now)}`);
  expect((await sent()).includes("dirty\nfalse"), "the window wasn't told the edits are gone");
});

await test("a save that brings a new picture: the window's message after markSaved makes the bar ask", async () => {
  await open("pictures");
  const bar = `document.getElementById("pictures-bar")`;
  const state = () =>
    evaluate(`({ shown: !${bar}.hidden, text: ${bar}.querySelector(".file-bar-text").textContent,
      answers: !${bar}.querySelector(".pictures-actions").hidden })`);
  await toMode("split");
  await caughtUp();
  await evaluate("window.marked = true");
  // A picture from a new host typed in, its preview shown, then saved: in the window's order,
  // markSaved and then what the saved text's pictures may do, for the same generation.
  await select(0);
  await type("![New](https://new.example/picture.png)\n\n");
  await caughtUp();
  const saving = await evaluate("LegibleEditor.contents()");
  await evaluate(`LegibleEditor.markSaved(${saving.version})`);
  await evaluate(`LegibleChrome.remoteImages({ generation: ${previewInfo.generation}, show: [], ask: true })`);
  const now = await state();
  expect(now.shown && now.answers && now.text.includes("Show them?"), `after the save: ${JSON.stringify(now)}`);
  expect(await evaluate("window.marked === true && !LegibleEditor.isDirty()"), "the page was reloaded, or not clean");
  expect((await text()).startsWith("![New](https://new.example/picture.png)"), "the text changed");
});

await test("scroll sync: the text and the document keep the same place, both ways", async () => {
  await open("showcase");
  await toMode("split");
  await caughtUp();
  const lines = (await text()).split("\n");
  const lineOf = (heading) => lines.findIndex((l) => l === heading) + 1;
  // The text scrolled to a heading: the document shows it at the top.
  const figures = lineOf("## Figures");
  await evaluate(`(() => {
    const v = LegibleEditor.view();
    const block = v.lineBlockAt(v.state.doc.line(${figures}).from);
    v.scrollDOM.scrollTop = v.documentTop - v.scrollDOM.getBoundingClientRect().top + v.scrollDOM.scrollTop + block.top;
  })()`);
  await until("the document to follow", () =>
    evaluate(`(() => {
      const doc = document.getElementById("document");
      const h = [...doc.querySelectorAll("h2")].find((e) => e.textContent === "Figures");
      return Math.abs(h.getBoundingClientRect().top - doc.getBoundingClientRect().top) < 24;
    })()`),
  );
  // The document scrolled to another: the text follows.
  const notes = lineOf("## Notes and warnings");
  await evaluate(`(() => {
    const doc = document.getElementById("document");
    const h = [...doc.querySelectorAll("h2")].find((e) => e.textContent === "Notes and warnings");
    doc.scrollTop += h.getBoundingClientRect().top - doc.getBoundingClientRect().top;
  })()`);
  await until("the text to follow", () =>
    evaluate(`(() => {
      const v = LegibleEditor.view();
      const top = v.scrollDOM.getBoundingClientRect().top - v.documentTop;
      const line = v.state.doc.lineAt(v.lineBlockAtHeight(Math.max(0, top)).from).number;
      return Math.abs(line - ${notes}) <= 1;
    })()`),
  );
});

await test("the place kept between the modes: View → Edit → type → View, and View → Split → scroll → View", async () => {
  await open("long");
  const lines = fileText("long").split("\n");
  const lineOf = (n) => lines.indexOf(`## Section ${n}`) + 1;
  const section = (n) => `[...document.querySelectorAll("#document h2")].find((h) => h.textContent === "Section ${n}")`;
  // The heading's top, from the page's top under the chrome (View), or from the document's pane (Split).
  const fromTop = (n) =>
    evaluate(`(() => {
      const doc = document.getElementById("document");
      const split = document.documentElement.classList.contains("mode-split");
      const top = split ? doc.getBoundingClientRect().top : parseFloat(getComputedStyle(document.body).paddingTop);
      return ${section(n)}.getBoundingClientRect().top - top;
    })()`);
  const textTop = () =>
    evaluate(`(() => {
      const v = LegibleEditor.view();
      const top = v.scrollDOM.getBoundingClientRect().top - v.documentTop;
      return v.state.doc.lineAt(v.lineBlockAtHeight(Math.max(0, top)).from).number;
    })()`);
  /** The text scrolled to put line `line` at its top. */
  const scrollText = (line) =>
    evaluate(`(() => {
      const v = LegibleEditor.view();
      v.dispatch({ effects: CM.view.EditorView.scrollIntoView(v.state.doc.line(${line}).from, { y: "start", yMargin: 0 }) });
    })()`);
  // Held there (the blocks round it are laid out as they come), not merely passed on the way.
  const restsAt = async (n, what) => {
    await pumped(what, async () => Math.abs((await fromTop(n)) - 8) < 20);
    await evaluate("new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))");
    const at = await fromTop(n);
    expect(Math.abs(at - 8) < 20, `${what}: it moved on, to ${at}`);
  };

  // View, at Section 40: Edit shows the text there.
  const id = await evaluate(`${section(40)}.id`);
  await evaluate(`LegibleChrome.goTo({ id: ${JSON.stringify(id)}, offset: 0, fraction: 0 })`);
  await until("the reader at Section 40", async () => Math.abs(await fromTop(40)) < 2);
  await toMode("edit");
  await pumped("the text at Section 40", async () => Math.abs((await textTop()) - lineOf(40)) <= 1);
  // A change typed there, and the text scrolled on to Section 55: back in View, the document
  // shows Section 55 at the top, its new render and all.
  await evaluate(`(() => { const v = LegibleEditor.view(); v.dispatch({ selection: { anchor: v.state.doc.line(${lineOf(40) + 2}).from } }); v.focus(); })()`);
  await type("Edited. ");
  await scrollText(lineOf(55));
  await until("the text at Section 55", async () => Math.abs((await textTop()) - lineOf(55)) <= 1);
  await toMode("view");
  await restsAt(55, "View at Section 55 after Edit");
  expect(await evaluate(`document.getElementById("document").textContent.includes("Edited.")`), "the change isn't shown");

  // Split from there: both panes at Section 55.
  await toMode("split");
  await pumped("the text at Section 55 in Split", async () => Math.abs((await textTop()) - lineOf(55)) <= 1);
  await pumped("the document's pane at Section 55", async () => Math.abs((await fromTop(55)) - 8) < 24);
  // The document's pane scrolled back to Section 25: View shows it at the top.
  await evaluate(`(() => {
    const doc = document.getElementById("document");
    doc.scrollTop += ${section(25)}.getBoundingClientRect().top - doc.getBoundingClientRect().top;
  })()`);
  await until("the text to follow to Section 25", async () => Math.abs((await textTop()) - lineOf(25)) <= 1);
  await toMode("view");
  await restsAt(25, "View at Section 25 after Split");
  // And with nothing changed, Edit and back keeps it too.
  await toMode("edit");
  await pumped("the text at Section 25", async () => Math.abs((await textTop()) - lineOf(25)) <= 1);
  await toMode("view");
  await restsAt(25, "View at Section 25 after Edit, unchanged");
});

console.log(`Mermaid's inline styles refused by the CSP while laying out: ${mermaidStyleReports}.`);
for (const p of problems) {
  failures++;
  console.log(`  FAIL  ${p}`);
}

// Edge is asked to close, which closes its helper processes too (they hold the profile's
// files): not answered, as it closes the connection on the way. Killed only if it doesn't.
ws.send(JSON.stringify({ id: ++nextId, method: "Browser.close" }));
const exited = () => edge.exitCode !== null || edge.signalCode !== null;
await until("Edge to exit", exited, 15000).catch(() => edge.kill());
ws.close();
server.close();
// The temp folder goes once the last of Edge's processes has let go of it.
await until("the temp folder to be removed", () => removed(work), 30000).catch((e) => {
  failures++;
  console.log(`  FAIL  ${e.message}: ${work}`);
});

if (failures) {
  console.log(`${failures} failed.`);
  process.exit(1);
}
console.log("All passed.");
