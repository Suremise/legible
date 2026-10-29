// Screenshots of the document's look, taken with Edge in headless mode (the same engine as
// WebView2), for checking web/doc/ by eye.
//
//   node scripts/doc-shots.mjs [--out <folder>]      (default: target/doc-shots)
//
// It renders the golden fixtures (crates/legible-core/tests/fixtures) and a showcase
// (scripts/doc-shots/showcase.md) with legible-core's doc-preview example, serves them on
// 127.0.0.1 under the page's own CSP with web/doc/ and the vendored bundles, and drives a
// headless Edge of its own over the DevTools protocol (no window, no input to anything else;
// scripts/doc-harness.mjs):
//
// - the showcase in light and dark, Narrow, Comfortable and Wide, Sans and Serif, at 100% and
//   150% zoom, and with the theme set over Windows' own both ways;
// - each fixture in light and dark;
// - a document with very long properties, Narrow, at 100% and 150%;
// - the showcase printed to PDF with Windows in dark mode (print is always light), and its
//   pages as PNGs.
//
// It fails if any page scrolls sideways, logs an error, or has anything refused by the CSP (the
// page's or Mermaid's frame's). Nothing is fetched from the network.

import { mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

import { launchEdge, renderDocuments, removeWhenFree, repo, serve, until } from "./doc-harness.mjs";

const outArg = process.argv.indexOf("--out");
const out = resolve(outArg > 0 ? process.argv[outArg + 1] : join(repo, "target", "doc-shots"));
const work = mkdtempSync(join(tmpdir(), "legible-doc-shots-"));
const pages = join(work, "pages");

// The window: 1100 px wide; at 150% zoom the page sees 1100 / 1.5 px.
const WINDOW_WIDTH = 1100;
const WINDOW_HEIGHT = 900;

const fixtures = join(repo, "crates", "legible-core", "tests", "fixtures");
const showcaseDir = join(repo, "scripts", "doc-shots");
const sources = [
  join(showcaseDir, "showcase.md"),
  join(showcaseDir, "long-properties.md"),
  ...readdirSync(fixtures)
    .filter((f) => f.endsWith(".md") && f !== "security.md")
    .map((f) => join(fixtures, f)),
];

console.log("Rendering the documents...");
const strings = renderDocuments(pages, sources);
const server = await serve({ pages, strings, docfiles: showcaseDir });
const origin = server.origin;

const edge = await launchEdge(join(work, "profile"));
const send = edge.send;
const tab = await edge.tab();
const s = tab.s;
const evaluate = tab.evaluate;

async function open(name, { classes = "", dark = false, zoom = 1, media = "screen" } = {}) {
  await s("Emulation.setDeviceMetricsOverride", {
    width: Math.round(WINDOW_WIDTH / zoom),
    height: Math.round(WINDOW_HEIGHT / zoom),
    deviceScaleFactor: zoom,
    mobile: false,
  });
  await s("Emulation.setEmulatedMedia", {
    media,
    features: [{ name: "prefers-color-scheme", value: dark ? "dark" : "light" }],
  });
  tab.reset();
  await s("Page.navigate", { url: `${origin}/page/${name}?classes=${encodeURIComponent(classes)}` });
  await until(`${name} to load`, () =>
    evaluate("document.readyState === 'complete' && !!window.LegibleDoc").catch(() => false),
  );
  return evaluate(`LegibleDoc.ready.then(() => document.fonts.ready).then(() => ({
    width: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
    height: document.documentElement.scrollHeight,
  }))`);
}

// Each diagram is drawn light and dark: the one showing must be the theme's (light in print),
// and drawn.
async function checkDiagrams(file, expected) {
  const shown = await evaluate(`[...document.querySelectorAll(".diagram:not(.failed)")].map((d) =>
    [...d.querySelectorAll(":scope > .figure")]
      .filter((f) => getComputedStyle(f).display !== "none")
      .map((f) => f.classList.contains("dark") ? "dark" : "light") + (d.querySelector(".figure svg") ? "" : " (not drawn)"))`);
  for (const [i, theme] of shown.entries()) {
    if (theme !== expected) problems.push(`${file}: diagram ${i + 1} shows ${theme || "nothing"}, not ${expected}`);
  }
}

const problems = [];
async function shot(file, name, options) {
  const size = await open(name, options);
  const classes = options.classes ?? "";
  const dark = classes.includes("theme-dark") || (options.dark && !classes.includes("theme-light"));
  await checkDiagrams(file, dark ? "dark" : "light");
  if (size.scrollWidth > size.width) problems.push(`${file}: the page scrolls sideways (${size.scrollWidth} > ${size.width})`);
  for (const e of tab.errors) problems.push(`${file}: ${e}`);
  // Mermaid draws in its own frame, so nothing should be refused, in the page or the frame.
  for (const [where, count] of Object.entries(tab.refusals)) {
    if (count) problems.push(`${file}: the CSP refused ${count} thing(s) in the ${where}`);
  }
  const { data } = await s("Page.captureScreenshot", {
    format: "png",
    captureBeyondViewport: true,
    clip: { x: 0, y: 0, width: size.width, height: size.height, scale: 1 },
  });
  writeFileSync(join(out, file), Buffer.from(data, "base64"));
  console.log(`  ${file}`);
}

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

try {
  console.log(`Screenshots in ${out}:`);
  for (const dark of [false, true]) {
    for (const width of ["narrow", "comfortable", "wide"]) {
      for (const font of ["sans", "serif"]) {
        for (const zoom of [1, 1.5]) {
          const classes = [width !== "comfortable" && `width-${width}`, font === "serif" && "font-serif"]
            .filter(Boolean)
            .join(" ");
          const file = `showcase-${dark ? "dark" : "light"}-${width}-${font}-${zoom * 100}.png`;
          await shot(file, "showcase", { classes, dark, zoom });
        }
      }
    }
  }
  await shot("showcase-windows-light-set-dark.png", "showcase", { classes: "theme-dark" });
  await shot("showcase-windows-dark-set-light.png", "showcase", { classes: "theme-light", dark: true });

  for (const zoom of [1, 1.5]) {
    await shot(`long-properties-narrow-${zoom * 100}.png`, "long-properties", { classes: "width-narrow", zoom });
  }

  for (const fixture of sources.slice(2).map((f) => basename(f, ".md"))) {
    for (const dark of [false, true]) {
      await shot(`fixture-${fixture}-${dark ? "dark" : "light"}.png`, fixture, { dark });
    }
  }

  // Print: Windows in dark mode, which the print must ignore.
  await open("showcase", { dark: true, classes: "theme-dark" });
  await s("Emulation.setEmulatedMedia", {
    media: "print",
    features: [{ name: "prefers-color-scheme", value: "dark" }],
  });
  await checkDiagrams("showcase-print.pdf", "light");
  await s("Emulation.setEmulatedMedia", {
    media: "",
    features: [{ name: "prefers-color-scheme", value: "dark" }],
  });
  const { data } = await s("Page.printToPDF", { printBackground: true, preferCSSPageSize: true });
  const pdf = join(out, "showcase-print.pdf");
  writeFileSync(pdf, Buffer.from(data, "base64"));
  console.log("  showcase-print.pdf");
  await printPages(pdf);
} finally {
  await edge.close();
  server.close();
  await removeWhenFree(work);
}

// The PDF's pages as PNGs, as Edge's own PDF viewer draws them, in a tab of their own (the
// viewer takes over the tab it opens in).
async function printPages(pdf) {
  for (const pageNo of [1, 2, 3, 4, 5]) {
    const url = `${pathToFileURL(pdf).href}#page=${pageNo}&toolbar=0&navpanes=0&view=FitH`;
    const tab = await send("Target.createTarget", { url, newWindow: true, width: 900, height: 1270 });
    const session = (await send("Target.attachToTarget", { targetId: tab.targetId, flatten: true })).sessionId;
    // The viewer draws on its own and offers nothing to wait on from here: wait until two
    // frames in a row are the same and not blank.
    let last = "";
    await until(`page ${pageNo} of the PDF to draw`, async () => {
      const { data } = await send("Page.captureScreenshot", { format: "png" }, session);
      const same = data === last && data.length > 30000;
      last = data;
      return same;
    });
    const file = `showcase-print-page-${pageNo}.png`;
    writeFileSync(join(out, file), Buffer.from(last, "base64"));
    console.log(`  ${file}`);
    await send("Target.closeTarget", { targetId: tab.targetId });
  }
}

if (problems.length) {
  console.error(`\n${problems.length} problem(s):\n${problems.join("\n")}`);
  process.exitCode = 1;
} else {
  console.log("\nNo page scrolled sideways, and none logged an error or a CSP refusal.");
}
