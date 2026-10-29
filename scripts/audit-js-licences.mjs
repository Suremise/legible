// Audits the licences of every package bundled into the pinned JavaScript in third_party/js/, and
// writes third_party/js/LICENSES.md (the record) and third_party/js/NOTICES.txt (the licence
// texts, for THIRD-PARTY-NOTICES).
//
// Build-only, run by hand when a pin in third_party/js/VERSIONS moves:
//
//     node scripts/audit-js-licences.mjs [--cache DIR]
//
// For each bundle it lists the packages inside it (VERSIONS' `inventory` lines), downloads each
// package's npm tarball at exactly that version (`npm pack`, which checks the registry's
// integrity), and reads its package.json licence and every licence-like file in it, vendored
// ones in subfolders included. A package passes when its declared licence is satisfiable from
// the allowed list (an OR passes when one side is allowed) and every licence file found names an
// allowed licence. Anything else is listed under "Needs a decision", and the script exits 1:
// Legible ships nothing else without the maintainer's decision (docs/ARCHITECTURE.md, Web assets).

import { execFileSync, execSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const JS = path.join(ROOT, "third_party", "js");

// The brief's list (§3.2), as SPDX identifiers.
const ALLOWED = new Set([
  "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD", "BSL-1.0",
  "CC0-1.0", "Unicode-3.0",
]);
// Code named like a licence file (@iconify/utils' lib/misc/licenses.js) isn't one.
const CODE = /\.(m?js|cjs|ts|map|json)$/i;
const COPYLEFT = new Set(["GPL", "LGPL", "AGPL", "EPL-2.0"]);
const LICENCE_FILE =
  /(^|\/)(LICEN[CS]E|COPYING|NOTICE|PATENTS|UNLICENSE|COPYRIGHT)[^/]*$|(^|\/)[^/]*\.LICEN[CS]E$/i;

const sha256 = (data) => crypto.createHash("sha256").update(data).digest("hex");

function readPins() {
  const pins = [];
  let current = null;
  for (const line of fs.readFileSync(path.join(JS, "VERSIONS"), "utf8").split(/\r?\n/)) {
    if (/^\s*(#|$)/.test(line)) continue;
    const section = line.match(/^\[(.+)\]$/);
    const pair = line.match(/^\s*([a-z0-9_]+)\s*=\s*(.+?)\s*$/);
    if (section) pins.push((current = { file: section[1], inventory: [] }));
    else if (pair && current) {
      if (pair[1] === "inventory") current.inventory.push(pair[2]);
      else current[pair[1]] = pair[2];
    } else throw new Error(`VERSIONS: can't read '${line}'`);
  }
  return pins;
}

function tar() {
  return process.platform === "win32"
    ? path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe")
    : "tar";
}

/** Downloads and unpacks name@version into the cache; returns its folder. */
function unpack(cache, name, version) {
  const dir = path.join(cache, `${name.replace("/", "+")}@${version}`);
  if (fs.existsSync(path.join(dir, "package", "package.json"))) return path.join(dir, "package");
  fs.mkdirSync(dir, { recursive: true });
  // Through the shell, as npm is a .cmd script on Windows; the spec and folder are ours.
  const out = execSync(`npm pack "${name}@${version}" --json --pack-destination "${dir}"`, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  });
  const [packed] = JSON.parse(out);
  execFileSync(tar(), ["-xzf", path.join(dir, packed.filename), "-C", dir]);
  return path.join(dir, "package");
}

/** name@version (a scoped name keeps its leading @). */
function splitSpec(spec) {
  const at = spec.lastIndexOf("@");
  if (at <= 0) throw new Error(`not name@version: ${spec}`);
  return [spec.slice(0, at), spec.slice(at + 1)];
}

/** The packages a bundle holds, as a map of "name@version" to a note ("" or "patched by …"). */
function inventory(pin, cache) {
  const found = new Map();
  for (const entry of pin.inventory) {
    if (entry === "self") {
      found.set(`${pin.package}@${pin.version}`, "");
    } else if (entry.startsWith("list:")) {
      const file = path.join(ROOT, entry.slice(5));
      for (const line of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
        if (line.trim() && !line.startsWith("#")) found.set(line.trim(), "");
      }
    } else if (entry.startsWith("comments:")) {
      const rest = entry.slice(9);
      const colon = rest.indexOf(":", 1);
      const spec = rest.slice(0, colon);
      found.set(spec, "");
      const [name, version] = splitSpec(spec);
      // A file, or a folder of a build's chunks (every .js and .mjs file in it).
      const target = path.join(unpack(cache, name, version), "..", rest.slice(colon + 1));
      const source = fs.statSync(target).isDirectory()
        ? fs.readdirSync(target, { recursive: true })
            .filter((f) => /\.m?js$/.test(f))
            .map((f) => fs.readFileSync(path.join(target, f), "utf8"))
            .join("\n")
        : fs.readFileSync(target, "utf8");
      // pnpm's layout: node_modules/.pnpm/<name, '/' as '+'>@<version>[_<peers or patch>]/node_modules/…
      for (const m of source.matchAll(/^\s*\/\/ \S*node_modules\/\.pnpm\/([^/\s]+)\/node_modules\//gm)) {
        const parsed = m[1].match(/^(@?[^@]+)@([^_]+)(_.*)?$/);
        if (!parsed) throw new Error(`can't read the package in ${m[0]}`);
        const key = `${parsed[1].replace("+", "/")}@${parsed[2]}`;
        const patched = parsed[3]?.includes("patch_hash") ? `patched by ${name}` : "";
        found.set(key, found.get(key) || patched);
      }
      const flat = source.match(/^\s*\/\/ (?!\S*\.pnpm\/)\S*node_modules\/\S+/m);
      if (flat) throw new Error(`a bundled path with no version: ${flat[0].trim()}`);
    } else {
      throw new Error(`${pin.file}: unknown inventory '${entry}'`);
    }
  }
  return found;
}

/** The licences a licence file's text contains. */
function classify(text) {
  const found = [];
  if (/Permission is hereby granted, free of charge/i.test(text)) found.push("MIT");
  if (/Apache License/i.test(text) && /Version 2\.0/i.test(text)) found.push("Apache-2.0");
  if (/Redistribution and use in source and binary forms/i.test(text)) {
    const third = /Neither the name|names of (its|their)\s+contributors|name of the copyright holder/i.test(text);
    found.push(third ? "BSD-3-Clause" : "BSD-2-Clause");
  }
  if (/Permission to use, copy, modify, and\/?or distribute this software for any/i.test(text)) {
    // 0BSD is ISC without the condition to keep the notice.
    found.push(/above copyright notice/i.test(text) ? "ISC" : "0BSD");
  }
  if (/Mozilla Public License/i.test(text) && /2\.0/.test(text)) found.push("MPL-2.0");
  if (/Eclipse Public License/i.test(text)) found.push("EPL-2.0");
  if (/CC0 1\.0 Universal/i.test(text)) found.push("CC0-1.0");
  if (/This is free and unencumbered software released into the public domain/i.test(text)) found.push("Unlicense");
  // MPL-2.0's and EPL-2.0's own texts name the GPL family (as "Secondary Licenses"), so it's
  // only reported when neither is found.
  if (!found.includes("MPL-2.0") && !found.includes("EPL-2.0")) {
    if (/Lesser General Public License/i.test(text)) found.push("LGPL");
    else if (/Affero General Public License/i.test(text)) found.push("AGPL");
    else if (/GNU General Public License/i.test(text)) found.push("GPL");
  }
  return found;
}

/** Whether an SPDX expression can be met from the allowed list. */
function satisfiable(expression) {
  const text = expression.replace(/[()]/g, " ").trim();
  return text.split(/\s+OR\s+/).some((side) => side.split(/\s+AND\s+/).every((id) => ALLOWED.has(id.trim())));
}

function declared(pkg) {
  if (typeof pkg.license === "string") return pkg.license;
  if (pkg.license?.type) return pkg.license.type;
  if (Array.isArray(pkg.licenses)) return pkg.licenses.map((l) => l.type ?? l).join(" OR ");
  return "";
}

function licenceFiles(dir) {
  const files = [];
  const walk = (sub) => {
    for (const entry of fs.readdirSync(path.join(dir, sub), { withFileTypes: true })) {
      const rel = sub ? `${sub}/${entry.name}` : entry.name;
      if (entry.isDirectory()) walk(rel);
      else if (LICENCE_FILE.test(rel) && !CODE.test(rel)) files.push(rel);
    }
  };
  walk("");
  return files.sort();
}

function audit(cache) {
  const pins = readPins();
  const results = [];
  const texts = new Map(); // sha256 of a licence text -> { text, packages: Set }
  for (const pin of pins) {
    const bundle = { pin, packages: [] };
    for (const [spec, note] of [...inventory(pin, cache)].sort(([a], [b]) => a.localeCompare(b))) {
      const [name, version] = splitSpec(spec);
      const dir = unpack(cache, name, version);
      const pkg = JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8"));
      const field = declared(pkg);
      const files = licenceFiles(dir).map((rel) => {
        const text = fs.readFileSync(path.join(dir, rel), "utf8");
        const hash = sha256(text.replace(/\r\n/g, "\n"));
        if (!texts.has(hash)) texts.set(hash, { text: text.replace(/\r\n/g, "\n").trim(), packages: new Set() });
        texts.get(hash).packages.add(spec);
        return { rel, licences: classify(text), hash };
      });
      const problems = [];
      if (field && !satisfiable(field)) problems.push(`declares ${field}`);
      if (!field && files.length === 0) problems.push("declares no licence and has no licence file");
      for (const f of files) {
        if (f.licences.length === 0) problems.push(`\`${f.rel}\` names no licence the audit recognises`);
        else if (!f.licences.some((l) => ALLOWED.has(l))) {
          // A file under a licence the package also offers as an alternative (MPL beside a
          // declared "MPL-2.0 OR Apache-2.0") is fine; copyleft never is.
          if (f.licences.some((l) => COPYLEFT.has(l)) || !(field && satisfiable(field)))
            problems.push(`\`${f.rel}\` is ${f.licences.join(", ")}`);
        }
      }
      if (!field && files.length > 0 && !files.some((f) => f.licences.some((l) => ALLOWED.has(l))))
        problems.push("declares no licence, and its licence files name none allowed");
      bundle.packages.push({ spec, name, version, note, field, files, problems });
    }
    results.push(bundle);
  }
  return { results, texts };
}

function write({ results, texts }) {
  const failing = results.flatMap((b) => b.packages.filter((p) => p.problems.length).map((p) => ({ ...p, file: b.pin.file })));
  const total = new Set(results.flatMap((b) => b.packages.map((p) => p.spec))).size;
  const md = [];
  md.push("# The bundled JavaScript's licences", "");
  md.push(
    "Legible embeds the pinned JavaScript bundles in this folder (`VERSIONS` pins each file's SHA-256 and",
    "source). This file records the licence of every package inside each bundle, at the exact version",
    "bundled, for the licence rule (MIT or equally permissive only) and THIRD-PARTY-NOTICES.",
    "",
    "**Generated by `scripts/audit-js-licences.mjs`.** Don't edit it by hand: rerun the script when a pin",
    "moves, and review the diff. `scripts/fetch-js.ps1 -VerifyOnly` (CI) fails unless this file records",
    "each bundle at its pinned SHA-256.",
    "",
    "## Result",
    "",
  );
  if (failing.length === 0) {
    md.push(`All ${total} bundled packages are ${[...ALLOWED].join(", ")}, or dual-licensed with one of those.`, "");
  } else {
    md.push(`**Needs a decision:** ${failing.length} of ${total} packages ${failing.length === 1 ? "isn't" : "aren't"} plainly allowed:`, "");
    for (const p of failing) md.push(`- \`${p.spec}\` in ${p.file}: ${p.problems.join("; ")}.`);
    md.push("");
  }
  md.push(
    "## How this was checked",
    "",
    "1. Each bundle's packages come from its `inventory` in `VERSIONS`: the package itself for a single-package",
    "   file; for a bundle Legible builds, the packages with code in the output, from esbuild's metafile",
    "   (`scripts/js/<bundle>/bundled-packages.txt`, written by the build); and the packages a package's",
    "   own build inlines, from that build's `// node_modules/.pnpm/<package>@<version>/…` comments, which",
    "   name every module its bundler put in it, at its exact version.",
    "2. Each package's tarball was downloaded from the npm registry at that version (`npm pack`, which",
    "   checks the registry's integrity).",
    "3. The script read the package's declared licence and every file in it named like LICENSE, COPYING,",
    "   NOTICE, PATENTS or COPYRIGHT, in subfolders too, which catches vendored code under other terms.",
    "",
  );
  for (const { pin, packages } of results) {
    const source = pin.build
      ? `built from ${pin.package}@${pin.version} by \`scripts/build-js.ps1 -Bundle ${pin.build}\``
      : `\`${pin.path}\` from [${pin.package}@${pin.version}](${pin.tarball})`;
    md.push(`## ${pin.file}`, "", `SHA-256 \`${pin.sha256}\`, ${source}. ${packages.length} package${packages.length === 1 ? "" : "s"}.`, "");
    md.push("| Package | Version | Declared | Licence files |", "|---|---|---|---|");
    for (const p of packages) {
      const files = p.files.map((f) => `\`${f.rel}\` ${f.licences.join(", ") || "?"} \`${f.hash.slice(0, 12)}\``).join("<br>") || "-";
      const note = p.note ? ` (${p.note})` : "";
      md.push(`| \`${p.name}\`${note} | ${p.version} | ${p.field || "-"} | ${files} |`);
    }
    md.push("");
  }
  fs.writeFileSync(path.join(JS, "LICENSES.md"), md.join("\n"));

  const notices = [
    "The bundled JavaScript's licence texts: each distinct text once, with the packages it covers.",
    "Generated by scripts/audit-js-licences.mjs.",
    "",
  ];
  for (const [, { text, packages }] of [...texts].sort(([, a], [, b]) => [...a.packages][0].localeCompare([...b.packages][0]))) {
    notices.push("=".repeat(78), [...packages].sort().join(", "), "-".repeat(78), text, "");
  }
  fs.writeFileSync(path.join(JS, "NOTICES.txt"), notices.join("\n"));
  return failing;
}

const args = process.argv.slice(2);
const cacheArg = args.indexOf("--cache");
const cache = cacheArg >= 0 ? path.resolve(args[cacheArg + 1]) : fs.mkdtempSync(path.join(os.tmpdir(), "legible-js-audit-"));
try {
  const failing = write(audit(cache));
  console.log(failing.length ? `Needs a decision: ${failing.map((p) => p.spec).join(", ")}` : "All bundled packages are allowed.");
  process.exitCode = failing.length ? 1 : 0;
} finally {
  if (cacheArg < 0) fs.rmSync(cache, { recursive: true, force: true });
}
