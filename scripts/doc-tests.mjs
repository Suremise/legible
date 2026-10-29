// Tests of the document's script in a real engine: Mermaid drawing in its sandboxed frame, and
// the page coming through a broken diagram, a hung or missing frame, diagrams that try to get
// out, and a frame taken over. Headless Edge, as doc-shots (scripts/doc-harness.mjs).
//
//   node scripts/doc-tests.mjs
//
// Each test opens its own tab; it fails on what it checks, on any error the page logs, and on
// anything the CSP refuses in the page or the frame. Exits 1 if any test failed.

import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { FRAME_PATH, launchEdge, renderDocuments, removeWhenFree, repo, serve, until } from "./doc-harness.mjs";

const here = join(repo, "scripts", "doc-tests");
const work = mkdtempSync(join(tmpdir(), "legible-doc-tests-"));
const pages = join(work, "pages");

console.log("Rendering the documents...");
const strings = renderDocuments(
  pages,
  ["diagrams", "injection", "hostile", "print"].map((name) => join(here, `${name}.md`)),
);

// The frame each test is served: Mermaid's own unless a test stands another in.
let frame = null;
const server = await serve({ pages, strings, test: here, frame: () => frame });
const edge = await launchEdge(join(work, "profile"));

// Collects, in the page only, what frames post it, for the tests to read.
const LISTEN = `if (window === window.top) {
  window.__messages = [];
  window.addEventListener("message", (e) => window.__messages.push(e.data));
}`;

// Opens a document in a tab of its own, with `setup` run in the page before its scripts, and
// waits for the document's script to finish.
async function open(name, { setup = "", classes = "" } = {}) {
  const tab = await edge.tab();
  await tab.s("Page.addScriptToEvaluateOnNewDocument", { source: `${LISTEN}\n${setup}` });
  await tab.s("Page.navigate", { url: `${server.origin}/page/${name}?classes=${classes}` });
  await until(`${name} to load`, () =>
    tab.evaluate("document.readyState === 'complete' && !!window.LegibleDoc").catch(() => false),
  );
  await tab.evaluate("LegibleDoc.ready");
  return tab;
}

async function close(tab) {
  await edge.send("Target.closeTarget", { targetId: tab.targetId });
}

function check(condition, what) {
  if (!condition) throw new Error(what);
}

// What a page shows of its diagrams.
const DIAGRAMS = `[...document.querySelectorAll(".diagram")].map((d) => ({
  failed: d.classList.contains("failed"),
  error: d.querySelector(".diagram-error")?.textContent ?? null,
  open: d.querySelector(":scope > details").open,
  light: !!d.querySelector(".figure.light > svg"),
  dark: !!d.querySelector(".figure.dark > svg"),
}))`;

// Nothing in the document that runs script or leaves it, wherever it came from.
const NOTHING_RUNS = `(() => {
  const found = [];
  for (const node of document.querySelectorAll("#document *")) {
    const inDiagram = !!node.closest(".figure");
    if (inDiagram && /^(script|iframe|object|embed|form|input|img|image|animate|set)$/i.test(node.localName)) {
      found.push(node.localName);
    }
    for (const a of node.attributes) {
      if (/^on/i.test(a.localName)) found.push(node.localName + " " + a.name);
      if (inDiagram && /href$/i.test(a.localName) && !a.value.trim().startsWith("#")) {
        found.push(node.localName + " " + a.name + "=" + a.value);
      }
    }
  }
  if (window.__legiblePwned !== undefined) found.push("ran: " + window.__legiblePwned);
  if (document.title !== document.querySelector("title").textContent) found.push("title");
  return found;
})()`;

const tests = {
  async "a diagram is drawn in the frame, light and dark, with nothing refused"() {
    const tab = await open("diagrams");
    const diagrams = await tab.evaluate(DIAGRAMS);
    check(diagrams.length === 3, `3 diagrams, not ${diagrams.length}`);
    for (const d of diagrams.slice(0, 2)) check(!d.failed && d.light && d.dark, `drawn: ${JSON.stringify(d)}`);
    const frame = await tab.evaluate(`(() => {
      const f = document.querySelector("iframe.diagram-frame");
      return f && { sandbox: f.getAttribute("sandbox"), inDocument: !!f.closest("#document") };
    })()`);
    check(frame?.sandbox === "allow-scripts" && !frame.inDocument, `the frame: ${JSON.stringify(frame)}`);
    // The SVG is the page's own: its styles came in through the CSSOM, not as markup.
    const markup = await tab.evaluate(`[...document.querySelectorAll(".figure svg")].every((svg) =>
      !svg.querySelector("style") && svg.id.startsWith("legible-diagram-"))`);
    check(markup, "a <style> element in a diagram, or one without its id");
    return tab;
  },

  async "a broken diagram shows its source and a short message"() {
    const tab = await open("diagrams");
    const [, , broken] = await tab.evaluate(DIAGRAMS);
    check(broken.failed && broken.error && broken.open && !broken.light && !broken.dark, JSON.stringify(broken));
    return tab;
  },

  async "changing the theme shows the diagram drawn for it"() {
    const tab = await open("diagrams");
    for (const theme of ["dark", "light", "dark"]) {
      const shown = await tab.evaluate(`(() => {
        document.documentElement.className = "theme-${theme}";
        const figures = [...document.querySelectorAll(".diagram:not(.failed) > .figure")];
        return figures.filter((f) => getComputedStyle(f).display !== "none")
          .map((f) => (f.classList.contains("dark") ? "dark" : "light") + (f.querySelector("svg") ? "" : " empty"));
      })()`);
      check(shown.length === 2 && shown.every((s) => s === theme), `${theme}: ${shown}`);
    }
    return tab;
  },

  async "diagrams that try to run script get nothing"() {
    const tab = await open("injection");
    // Mermaid lets some of it into the frame while it lays out, and the frame's CSP stops it
    // there: refused in the frame, as it should be.
    tab.frameMayRefuse = true;
    const found = await tab.evaluate(NOTHING_RUNS);
    check(found.length === 0, `got through: ${found.join(", ")}`);
    const diagrams = await tab.evaluate(DIAGRAMS);
    check(diagrams.length === 4, `4 diagrams, not ${diagrams.length}`);
    // Each is drawn, or fails as a diagram does.
    for (const d of diagrams) check(d.failed ? d.error && d.open : d.light && d.dark, JSON.stringify(d));
    return tab;
  },

  async "the frame ignores messages that aren't requests"() {
    const tab = await open("diagrams");
    const replies = await tab.evaluate(`(async () => {
      const frame = document.querySelector("iframe.diagram-frame").contentWindow;
      const ok = { id: "legible-diagram-9001", source: "flowchart LR\\n A --> B", theme: "light",
        background: "rgb(255, 255, 255)", font: "Segoe UI" };
      window.__messages = [];
      for (const junk of [
        null, "flowchart LR\\n A --> B", { ...ok, id: "x" }, { ...ok, id: "legible-diagram-1;x" },
        { ...ok, theme: "loose" }, { ...ok, background: "url(https://example.invalid/)" },
        { ...ok, font: "x; } body { display: none" }, { ...ok, source: 42 },
      ]) frame.postMessage(junk, "*");
      // Answered in order: once the good one is, every message before it has been dealt with.
      frame.postMessage({ ...ok, id: "legible-diagram-9002" }, "*");
      const start = Date.now();
      while (!window.__messages.some((m) => m?.id === "legible-diagram-9002")) {
        if (Date.now() - start > 30000) throw new Error("no answer from the frame");
        await new Promise((r) => setTimeout(r, 20));
      }
      return window.__messages.map((m) => m?.id ?? JSON.stringify(m));
    })()`);
    check(replies.length === 1 && replies[0] === "legible-diagram-9002", `answered: ${replies}`);
    return tab;
  },

  async "a frame taken over can't reach the page, and what it sends is made safe"() {
    frame = join(here, "hostile-frame.html");
    const tab = await open("hostile");
    // The browser reports the navigation it refused the frame.
    tab.expected = /^frame: Unsafe attempt to initiate navigation/;
    const probes = await tab.evaluate("window.__messages.filter((m) => m?.probe)");
    check(probes.length === 7, `${probes.length} probes`);
    for (const p of probes) check(!p.reached, `the frame reached ${p.probe}`);
    const found = await tab.evaluate(NOTHING_RUNS);
    check(found.length === 0, `got through: ${found.join(", ")}`);
    const diagrams = await tab.evaluate(DIAGRAMS);
    // 1 is taken in, made safe; 2 (another's id), 3 (too deep) and 4 (an error) fail.
    check(!diagrams[0].failed && diagrams[0].light && diagrams[0].dark, JSON.stringify(diagrams[0]));
    for (const d of diagrams.slice(1)) check(d.failed && d.error, JSON.stringify(d));
    const page = await tab.evaluate(`(() => {
      const svg = document.querySelector(".figure.light > svg");
      const h1 = document.querySelector("h1").getBoundingClientRect();
      return {
        url: location.href,
        shown: getComputedStyle(document.body).display !== "none" &&
          getComputedStyle(document.getElementById("document")).visibility === "visible",
        // The diagram's own styles don't let it cover the page.
        covered: document.elementFromPoint(h1.left + 5, h1.top + h1.height / 2)?.localName !== "h1",
        ids: [...svg.querySelectorAll("[id]")].map((n) => n.id),
        kept: [...svg.querySelectorAll("*")].map((n) => n.localName),
        label: svg.querySelector("foreignObject")?.textContent,
        rules: document.adoptedStyleSheets.flatMap((s) => [...s.cssRules].map((r) => r.cssText)),
        documents: document.querySelectorAll("#document").length,
      };
    })()`);
    check(page.url.includes("/page/hostile") && page.shown && !page.covered, `the page: ${JSON.stringify(page)}`);
    check(page.documents === 1 && page.ids.every((id) => id.startsWith("legible-diagram-1-")), `ids: ${page.ids}`);
    const expected = ["rect", "a", "text", "a", "text", "use", "use", "g", "rect", "g", "foreignObject", "div", "b", "text"];
    check(JSON.stringify(page.kept) === JSON.stringify(expected), `kept: ${page.kept}`);
    check(page.label === "kept label", `the label: ${page.label}`);
    check(
      page.rules.length > 0 && page.rules.every((r) => /^#legible-diagram-\d+ \.ok/.test(r)),
      `rules: ${page.rules.join(" | ")}`,
    );
    return tab;
  },

  async "in print, a tick and every icon is a picture in its colour, not a mask"() {
    // Masks print as PDF soft masks, which Windows' own PDF renderer ignores.
    for (const classes of ["", "theme-light", "theme-dark"]) {
      const tab = await open("print", { classes });
      await tab.s("Emulation.setEmulatedMedia", { media: "print" });
      const drawn = await tab.evaluate(`(() => {
        const hex = (rgb) => "#" + rgb.match(/\\d+/g).slice(0, 3).map((n) => (+n).toString(16).padStart(2, "0")).join("");
        // The picture's own colour, and the colour it stands for in print.
        const picture = (el, colour) => {
          const style = getComputedStyle(el, "::before");
          const svg = decodeURIComponent(style.backgroundImage);
          return {
            mask: style.maskImage,
            colour: svg.match(/(?:fill|stroke)='(#[0-9a-f]{6})'/)?.[1] ?? svg.slice(0, 80),
            expected: hex(colour),
            size: style.width + " " + style.height,
          };
        };
        const onAccent = document.createElement("span");
        onAccent.style.color = "var(--on-accent)";
        document.body.append(onAccent);
        const boxes = document.querySelectorAll("#document li > input[type=checkbox]");
        const out = {
          boxes: boxes.length,
          tick: picture(boxes[0], getComputedStyle(onAccent).color),
          unticked: getComputedStyle(boxes[1], "::before").content,
          alerts: [...document.querySelectorAll("#document .alert-title")].map((t) => picture(t, getComputedStyle(t).color)),
          missing: [...document.querySelectorAll("#document .missing-image")].map((m) => picture(m, getComputedStyle(m).color)),
        };
        onAccent.remove();
        return out;
      })()`);
      const say = `${classes || "Windows' theme"}: ${JSON.stringify(drawn)}`;
      const pictures = [drawn.tick, ...drawn.alerts, ...drawn.missing];
      check(drawn.boxes === 2 && drawn.alerts.length === 5 && drawn.missing.length === 1, say);
      check(drawn.unticked === "none" || drawn.unticked === "normal", `an unticked box has a tick: ${say}`);
      for (const p of pictures) {
        check(p.mask === "none" && p.colour === p.expected && !p.size.startsWith("0px"), say);
      }
      // Each kind its own colour, so none fell back to the note's.
      check(new Set(drawn.alerts.map((a) => a.colour)).size === 5, `the kinds' colours: ${say}`);
      // The last theme's tab goes back to be checked for errors, as every test's does.
      if (classes === "theme-dark") return tab;
      check(tab.errors.length === 0 && tab.refusals.page === 0, `${say}: errors ${tab.errors.join(" | ")}`);
      await close(tab);
    }
  },

  async "a frame that stops answering fails its diagrams, and is closed"() {
    frame = join(here, "silent-frame.html");
    const tab = await open("diagrams", { setup: "window.legibleDiagramTimeout = 300;" });
    const diagrams = await tab.evaluate(DIAGRAMS);
    for (const d of diagrams) check(d.failed && d.error && d.open, JSON.stringify(d));
    check(!(await tab.evaluate("!!document.querySelector('iframe.diagram-frame')")), "the frame is still there");
    return tab;
  },

  async "a frame that doesn't load fails its diagrams"() {
    frame = join(here, "no-such-frame.html");
    const tab = await open("diagrams", { setup: "window.legibleDiagramTimeout = 300;" });
    for (const d of await tab.evaluate(DIAGRAMS)) check(d.failed && d.error, JSON.stringify(d));
    return tab;
  },
};

let failed = 0;
try {
  for (const [name, test] of Object.entries(tests)) {
    frame = null;
    let tab;
    try {
      tab = await test();
      const errors = tab.errors.filter((e) => !tab.expected?.test(e));
      check(errors.length === 0, `errors: ${errors.join(" | ")}`);
      check(tab.refusals.page === 0, `the CSP refused ${tab.refusals.page} thing(s) in the page`);
      check(tab.frameMayRefuse || tab.refusals.frame === 0, `the CSP refused ${tab.refusals.frame} thing(s) in the frame`);
      console.log(`  ok    ${name}`);
      await close(tab);
    } catch (e) {
      failed++;
      console.log(`  FAIL  ${name}\n        ${e.message}`);
    }
  }
} finally {
  await edge.close();
  server.close();
  await removeWhenFree(work);
}

if (failed) {
  console.error(`\n${failed} test(s) failed.`);
  process.exitCode = 1;
} else {
  console.log(`\nAll ${Object.keys(tests).length} passed.`);
}
