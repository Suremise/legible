// Bundles one of Legible's JavaScript bundles with the pinned esbuild. Build-only: run by
// scripts/build-js.ps1 in a temp copy of scripts/js/<bundle>/, after `npm ci` there.
//
//     node build.mjs <outfile> <metafile>
//
// The bundle's package.json may say, under "legible":
//   globalName: the global the bundle's exports are set on (CodeMirror: "CM");
//   stubs: packages replaced by a local file, so none of their code is bundled
//     (Mermaid: elkjs, whose EPL-2.0 licence Legible doesn't ship);
//   unbundled: packages imported from their own unbundled modules rather than the prebundled
//     file their package.json points to, so esbuild sees, and the licence audit lists, the
//     packages that prebundle folds in (Mermaid: roughjs, whose bundled/rough.esm.js holds
//     hachure-fill, path-data-parser, points-on-curve and points-on-path);
//   notBundled: each declared dependency of a bundled package that has no code in the bundle,
//     with why (types only, a command line's, tree-shaken out, stubbed). The build fails on any
//     other: a dependency missing from the bundle may be folded into a prebundled file, where
//     the audit can't see it.

import fs from "node:fs";
import path from "node:path";
import * as esbuild from "esbuild";

const [outfile, metafile] = process.argv.slice(2);
const config = JSON.parse(fs.readFileSync("package.json", "utf8")).legible ?? {};
const stubs = Object.entries(config.stubs ?? {});
const unbundled = Object.entries(config.unbundled ?? {});
const escape = (text) => text.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");

const result = await esbuild.build({
  entryPoints: ["entry.js"],
  bundle: true,
  minify: true,
  format: "iife",
  globalName: config.globalName,
  legalComments: "eof",
  target: "es2020",
  metafile: true,
  outfile,
  logLevel: "warning",
  plugins: [
    {
      name: "stubs-and-unbundled",
      setup(build) {
        for (const [pkg, file] of stubs) {
          build.onResolve({ filter: new RegExp(`^${escape(pkg)}(/|$)`) }, () => ({ path: path.resolve(file) }));
        }
        for (const [pkg, entry] of unbundled) {
          build.onResolve({ filter: new RegExp(`^${escape(pkg)}$`) }, () => ({
            path: path.resolve("node_modules", pkg, entry),
          }));
        }
      },
    },
  ],
});

const output = Object.values(result.metafile.outputs)[0];
const inBundle = Object.entries(output.inputs).filter(([, i]) => i.bytesInOutput > 0).map(([p]) => p);
const packageOf = (p) => p.match(/node_modules\/((?:@[^/]+\/)?[^/]+)/)?.[1];
const packages = new Set(inBundle.map(packageOf).filter(Boolean));

// A stubbed package must leave nothing of its own in the bundle, and an unbundled one nothing
// of its prebundled file.
for (const [pkg] of stubs) {
  if (packages.has(pkg)) throw new Error(`${pkg} is stubbed out, but the bundle has code from it`);
}
for (const [pkg, entry] of unbundled) {
  const prebundled = inBundle.filter((p) => packageOf(p) === pkg && !p.includes(`node_modules/${pkg}/${path.posix.dirname(entry)}/`));
  if (prebundled.length) throw new Error(`${pkg} should come from ${entry}, but the bundle has ${prebundled.join(", ")}`);
}

// Every declared dependency of a bundled package is in the bundle, or listed with why it isn't.
const notBundled = config.notBundled ?? {};
const unexplained = [];
for (const pkg of packages) {
  const manifest = JSON.parse(fs.readFileSync(path.join("node_modules", pkg, "package.json"), "utf8"));
  for (const dep of Object.keys(manifest.dependencies ?? {})) {
    if (!packages.has(dep) && !(dep in notBundled)) unexplained.push(`${dep} (a dependency of ${pkg})`);
  }
}
if (unexplained.length) {
  throw new Error(
    `Not in the bundle, and not in "notBundled" with a reason: ${unexplained.join(", ")}. ` +
      "If its code is folded into the package's prebundled file, add the package to \"unbundled\".",
  );
}
fs.writeFileSync(metafile, JSON.stringify(result.metafile));
