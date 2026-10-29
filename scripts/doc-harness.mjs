// What scripts/doc-shots.mjs and scripts/doc-tests.mjs share: the documents rendered with
// legible-core's doc-preview example, served on 127.0.0.1 round the window's own page and
// under its CSPs, and a headless Edge of their own (the same engine as WebView2), driven over
// the DevTools protocol: no window, no input to anything else, nothing from the network.

import { spawn, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, readFileSync, rmSync } from "node:fs";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// The window's CSPs (legible-win's page.rs), with 'self' for Legible's origin: the page's, and
// the one Mermaid's frame has.
export const CSP =
  "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; " +
  "frame-src 'self'; base-uri 'none'; form-action 'none'";
export const FRAME_CSP =
  "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; " +
  "base-uri 'none'; form-action 'none'; frame-ancestors 'self'; sandbox allow-scripts";
export const FRAME_PATH = "/doc/mermaid-frame.html";

// Renders Markdown files into `pages` (<name>.html each) and returns the script's strings.
export function renderDocuments(pages, sources) {
  execFileSync(
    "cargo",
    ["run", "-q", "-p", "legible-core", "--example", "doc-preview", "--", pages, "/docfiles/", ...sources],
    { cwd: repo, stdio: "inherit" },
  );
  return readFileSync(join(pages, "strings.json"), "utf8");
}

// The page round a document, as the window's own.
function page(pages, strings, name, classes) {
  const body = readFileSync(join(pages, `${name}.html`), "utf8");
  return `<!doctype html>
<html lang="en" class="${classes}">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${CSP}">
<title>${name}</title>
<link rel="stylesheet" href="/doc/document.css">
<script type="application/json" id="legible-doc-strings">${strings}</script>
<script src="/doc/document.js" defer></script>
</head>
<body>
<main id="document">${body}</main>
</body>
</html>`;
}

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
};

function serveFile(res, root, rel, headers = {}) {
  const file = resolve(root, decodeURIComponent(rel));
  if (!file.startsWith(root) || !existsSync(file) || !TYPES[extname(file)]) {
    res.writeHead(404).end();
    return;
  }
  res.writeHead(200, { "content-type": TYPES[extname(file)], ...headers }).end(readFileSync(file));
}

// Serves the documents rendered into `pages` at /page/<name>?classes=…, web/doc/ at /doc/,
// the bundles at /js/, `docfiles` at /docfiles/, and `test` (a folder of tests' own files) at
// /test/. Mermaid's frame is served under its CSP; `frame()`, when it returns a file, serves
// that in its place (the tests' stand-ins). Resolves to {origin, close}.
export async function serve({ pages, strings, docfiles, test, frame = () => null }) {
  const server = createServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    const path = url.pathname;
    if (path.startsWith("/page/")) {
      const html = page(pages, strings, path.slice(6), url.searchParams.get("classes") ?? "");
      res.writeHead(200, { "content-type": "text/html; charset=utf-8", "content-security-policy": CSP });
      res.end(html);
    } else if (path === FRAME_PATH) {
      const file = frame() ?? join(repo, "web", "doc", "mermaid-frame.html");
      if (!existsSync(file)) return res.writeHead(404).end();
      res.writeHead(200, { "content-type": TYPES[".html"], "content-security-policy": FRAME_CSP });
      res.end(readFileSync(file));
    } else if (path.startsWith("/doc/")) {
      serveFile(res, join(repo, "web", "doc"), path.slice(5));
    } else if (path.startsWith("/js/")) {
      serveFile(res, join(repo, "third_party", "js"), path.slice(4));
    } else if (docfiles && path.startsWith("/docfiles/")) {
      serveFile(res, docfiles, path.slice(10));
    } else if (test && path.startsWith("/test/")) {
      serveFile(res, test, path.slice(6));
    } else {
      res.writeHead(404).end();
    }
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    close: () => server.close(),
  };
}

// Polls `check` until it returns something truthy, failing after `ms`.
export async function until(what, check, ms = 30000) {
  const end = Date.now() + ms;
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

function findEdge() {
  const candidates = [
    process.env["ProgramFiles(x86)"] && join(process.env["ProgramFiles(x86)"], "Microsoft", "Edge", "Application", "msedge.exe"),
    process.env.ProgramFiles && join(process.env.ProgramFiles, "Microsoft", "Edge", "Application", "msedge.exe"),
  ].filter(Boolean);
  const found = candidates.find((p) => existsSync(p));
  if (!found) throw new Error("Microsoft Edge isn't installed where expected");
  return found;
}

// A headless Edge with its profile in `profile`. Resolves to {send, tab, close}: send(method,
// params, sessionId) is a DevTools call; tab() opens a tab (below).
/** Removes `dir` once the last of Edge's processes has let go of it (a file still held open
    keeps it a while after Edge itself exits). */
export async function removeWhenFree(dir) {
  await until("the temp folder to be removed", () => {
    try {
      rmSync(dir, { recursive: true, force: true });
      // Windows can leave it listed for a moment while it's deleted.
      return !existsSync(dir);
    } catch (e) {
      if (["EPERM", "EBUSY", "ENOTEMPTY"].includes(e.code)) return false;
      throw e;
    }
  });
}

export async function launchEdge(profile) {
  const edge = spawn(
    findEdge(),
    [
      "--headless=new",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--hide-scrollbars",
      "--disable-sync",
      "--disable-background-networking",
      "--disable-component-update",
      "about:blank",
    ],
    { stdio: "ignore" },
  );

  const portFile = join(profile, "DevToolsActivePort");
  const [port, browserPath] = await until("Edge's DevTools port", () => {
    try {
      const lines = readFileSync(portFile, "utf8").trim().split("\n");
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
      const { resolve: ok, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      msg.error ? reject(new Error(`${msg.error.message}`)) : ok(msg.result);
    } else if (msg.method) {
      for (const l of listeners) l(msg);
    }
  };
  function send(method, params = {}, sessionId) {
    const id = ++nextId;
    ws.send(JSON.stringify({ id, method, params, sessionId }));
    return new Promise((ok, reject) => pending.set(id, { resolve: ok, reject }));
  }

  // A tab, with what its page and its frames log: `errors` (exceptions and errors), and the
  // CSP's refusals, `refusals.page` and `refusals.frame` (Mermaid's, which runs in a process
  // of its own: its sandbox gives it an opaque origin).
  async function tab() {
    const { targetId } = await send("Target.createTarget", { url: "about:blank" });
    const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
    const frames = new Set();
    const t = {
      targetId,
      sessionId,
      errors: [],
      refusals: { page: 0, frame: 0 },
      s: (method, params) => send(method, params, sessionId),
      reset() {
        t.errors = [];
        t.refusals = { page: 0, frame: 0 };
      },
      async evaluate(expression) {
        const r = await t.s("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
        if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
        return r.result.value;
      },
    };
    const ours = (msg) => msg.sessionId === sessionId || frames.has(msg.sessionId);
    listeners.push((msg) => {
      if (msg.method === "Target.attachedToTarget" && msg.sessionId === sessionId) {
        const child = msg.params.sessionId;
        frames.add(child);
        const auto = { autoAttach: true, waitForDebuggerOnStart: true, flatten: true };
        Promise.all([
          send("Runtime.enable", {}, child),
          send("Log.enable", {}, child),
          send("Target.setAutoAttach", auto, child),
        ])
          .then(() => send("Runtime.runIfWaitingForDebugger", {}, child))
          .catch(() => {});
        return;
      }
      if (!ours(msg)) return;
      const where = msg.sessionId === sessionId ? "page" : "frame";
      if (msg.method === "Runtime.exceptionThrown") {
        const d = msg.params.exceptionDetails;
        t.errors.push(`${where}: ${d.exception?.description ?? d.text}`);
      } else if (msg.method === "Log.entryAdded" && msg.params.entry.level === "error") {
        const text = msg.params.entry.text;
        if (/Content Security Policy/.test(text)) t.refusals[where]++;
        // A missing picture is shown on purpose (the showcase's and the images fixture's).
        else if (!/404|ERR_NAME_NOT_RESOLVED|Failed to load resource/.test(text)) t.errors.push(`${where}: ${text}`);
      }
    });
    await t.s("Page.enable");
    await t.s("Runtime.enable");
    await t.s("Log.enable");
    await t.s("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: true, flatten: true });
    return t;
  }

  // Edge is asked to close, which closes its helper processes too (they hold the profile's
  // files): not answered, as it closes the connection on the way. Killed only if it doesn't.
  async function close() {
    ws.send(JSON.stringify({ id: ++nextId, method: "Browser.close" }));
    const exited = () => edge.exitCode !== null || edge.signalCode !== null;
    await until("Edge to exit", exited, 15000).catch(() => edge.kill());
    ws.close();
  }

  return { send, tab, close };
}
