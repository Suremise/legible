# Legible: notes for agents

Legible is a Markdown reader for Windows, for business people rather than developers: Rust and
WebView2, one small exe. These notes are for every agent working on this repository, whichever
CLI runs it: Codex reads this file, and Claude Code reads it through `CLAUDE.md`.

- How the code fits together, the decisions behind it, and the licence rule:
  [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- **Testing: what to run when, flaky tests, and the rules every test keeps:
  [docs/TESTING.md](docs/TESTING.md).** In short: as you work, fmt, clippy and the tests of what
  you changed; the full suite (`cargo nextest run --workspace`) once before review; screenshots only
  for changes to the look; a flaky test's proof is its file 50 times under load plus one full run,
  never 50 full runs; tests wait for conditions, never for time; the full suite stays under 60 s.

**The rules every change keeps:** platform-free code in `legible-core`, Windows code in
`legible-win`; user-visible strings through Fluent (`locales/en/`); everything shipped MIT or
equally permissive, with system fonts only; no network and no telemetry; tests use temp folders
only. A JavaScript bundle changes only through `third_party/js/VERSIONS` and the licence audit
(`node scripts/audit-js-licences.mjs`); a licence beyond MIT, BSD, ISC or Apache-2.0 stops the
work for the maintainer's decision.

**Disk space** ([docs/TESTING.md](docs/TESTING.md#disk-space)): cargo never removes old test
exes, so a checkout's `target` grows by gigabytes over a few changes.
- **Once your changes are merged,** run `pwsh scripts/clean-build.ps1` in your checkout. It keeps
  the compiled dependencies, so the next change's first build stays quick.
- **Once you've reviewed a change,** delete your own probe folders under `%TEMP%`.

**No global keyboard or mouse input, ever** ([docs/TESTING.md](docs/TESTING.md#rules-every-test-keeps)),
in tests, probes or scripts: no SendKeys, SendInput, keybd_event or mouse_event, wscript or
AutoHotkey, and no clicking at screen coordinates. Synthetic input goes to whatever is in the
foreground, which is the user's work (on another project, a probe's typed path once arrived as a
message the user sent).
- **Drive only your own windows:** UI Automation patterns (ValuePattern, InvokePattern and the
  like) on elements found by your own process ID or window handle, messages posted to your own
  window's handle, or script run in your own WebView.
- **If you can't do it that way,** don't drive the real UI: prove it with tests, and say so in
  your summary of the change.

Probes, logs and scratch files go in your scratchpad or a temp folder, never in the repository or
beside a checkout. Personal, machine-specific notes belong in `CLAUDE.local.md` (Claude Code reads
it; keep it out of git), not here. The repository has no remote yet: never push.
