# Testing Legible

How the tests are run, what to run when, and the rules every test keeps. For contributors,
people and coding agents alike. The aim is what's necessary without going overboard.

## Running the tests

The suite runs under [cargo-nextest](https://nexte.st), a build-only tool (never shipped):

```powershell
cargo install cargo-nextest --version 0.9.149 --locked   # once

cargo nextest run --workspace                            # every test but the ignored
cargo test --workspace --doc                             # the doc tests (nextest doesn't run them)
```

nextest runs every test as its own process, all the test binaries side by side. Its settings are
in `.config/nextest.toml`. **`cargo test --workspace` still works**, for anyone without nextest:
the same tests, slower.

**Just the tests you changed:** name the package and the binary (a file under `tests/`) or the
unit tests' module:

```powershell
cargo nextest run -p legible-core -E 'test(/i18n::/)'
cargo nextest run -p legible-win -E 'binary(logging)'
```

**On a busy PC** (several builds at once), cap the threads: `-j 4`, and
`CARGO_BUILD_JOBS=4` for the build.

**The full suite should stay under 60 seconds** (the build aside). A test that needs longer, or
repeats something many times to catch a rare race, is marked `#[ignore = "stress: …"]` with a
quick version in the default run, and run by hand with `--run-ignored only`.

**The vendored JavaScript:** `pwsh scripts/fetch-js.ps1 -VerifyOnly` checks the bundles against
their pins and their licence audit (docs/ARCHITECTURE.md, Web assets). It runs on GitHub in CI, on
every push to main, and again on a version tag.

## Tests that take turns

Tests in different binaries run at the same moment. A test that touches something the OS shares
between processes goes in a **test group** with `max-threads = 1` in `.config/nextest.toml`,
never in a lock of its own (nextest runs each test as its own process, so an in-process `Mutex`
doesn't make tests take turns). The groups:

- `clipboard`: the real clipboard;
- `file-associations`: the current user's file associations;
- `credential-manager`: the real Credential Manager.

Put such a test's binary in its group with an override (the file shows how). Everything else
shares what it uses safely: temp folders, `APPDATA` per test, names of its own.

## What to run when

- **As you work:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, and the tests of the crates and files you changed (as above). Not the full suite.
- **Before review, once:** the full suite (`cargo nextest run --workspace`, and the doc tests if
  you changed a doc comment's example).
- **Before merging:** the full suite again only if main moved since that run.
- **The document's script** (`web/doc/`): `node scripts/doc-tests.mjs` (headless Edge, about 5
  s) on any change there, and in the Mermaid frame's CSP (legible-win's `page.rs`).
- **Screenshots:** only on changes to what's on screen. The README's pictures
  (`cargo run -p legible-win --example readme-shots`, a few seconds; the real window, not
  focused, with Legible's folders in a temp folder of its run's own) when the window's look
  changes: they're written to `target/ui-shots/readme` and copied into `docs/screenshots/`
  once approved. Each is taken once the page has painted the theme and mode asked for, and
  checked (the toolbar's colour, the pressed mode button): a picture a step behind is taken
  again, and the run's output says so.
- **The chrome's and the editor's browser tests** (`node scripts/chrome-tests.mjs`, about 20
  s): on changes to `web/chrome/`, `web/editor/` or the page.
- **The navigation check** (`cargo run -p legible-win --example nav-guard`, a few seconds; a
  quiet window of its own, and a temp folder of its run's own): on changes to the
  window's navigation policy, the page's `dirty` message or how a window serves its page. In a
  real WebView2 it proves a dirty editor refuses every reload and navigation, the text kept.
- **The save check** (`cargo run -p legible-win --example save-check`, a few seconds; a quiet
  window of its own, and a temp folder of its run's own): on changes to saving, closing,
  the conflict bar or the editor's save messages. In a real WebView2 it saves through the page's
  Ctrl+S, `saving::Session` and Windows' replace: the file byte for byte in its own format, a
  reload that waited going ahead once saved, a conflict found and kept over, and a close's save
  whose frozen editor keeps a late edit out.
- **The real window** (`cargo nextest run -p legible-win -E 'binary(shell)'`, about 4 s, and
  in the full suite): the shell itself, driven in its test's process by `shell::harness`
  (docs/ARCHITECTURE.md, The window), on files in a temp folder of its run's own, its windows
  quiet and checked after every case to have left the focus alone. Run it on changes to `shell`, opening, saving, closing, the conflict bar,
  pictures consent or how a window serves its page. A case that needs the real window goes there, not
  in a check of its own.
- **The PDF check** (`cargo run -p legible-win --example pdf-check`, a few seconds; a quiet
  window of its own, and a temp folder of its run's own): on changes to Save as PDF or the
  print sheet. It writes `target/ui-shots/export/showcase.pdf`.
- **The real clipboard,** by hand only (`cargo nextest run -p legible-win --run-ignored only -E
  'binary(clipboard)'`): it overwrites what's on the clipboard, so it's never in the suite; the
  suite covers the writer with a recording clipboard.
- **The check of pictures from the internet** (`cargo run --manifest-path
  crates/legible-win/checks/remote-images/Cargo.toml`, a few seconds once built; it opens a
  quiet window of its own, and keeps WebView2's data in a temp folder of its run's own,
  removed afterwards whether it passed, failed or overran): on changes to `consent`, the
  request filter, the page's CSP or how a window serves its page.
- **Reviewers** rely on the author's run record (the commit, each command, its result and where
  the log is), and rerun only what they doubt, plus the tests of anything they probe.

## A flaky test

Fix it at its cause: a test that waits for what it expects, or a race in the product. **The proof
is the affected test file 50 times under parallel load plus one full run**, never 50 full runs:

```powershell
# 50 runs of one binary while another binary's run gives the load; then the full suite once.
cargo nextest run -p legible-win -E 'binary(logging)' --stress-count 50
cargo nextest run --workspace
```

Run another binary beside it, or the full suite in a second terminal, for the load. Log both runs
where a reviewer can read them.

## Rules every test keeps

- **Wait for conditions, never for time.** Poll what you expect to see, with a generous bound for
  a hung run, rather than sleep and then check. A timeout bounds a hung run and is never the fix.
- **Clean up, even on failure.** Temp folders and anything else a test makes are removed in
  `Drop`, so a failed assertion still cleans up.
- **Pass under load,** not only alone: every test runs beside all the others.
- **Nothing outside temp folders:** no test writes to the real `%APPDATA%\Legible`, the real file
  associations or the real clipboard outside its test group.
- **No network.** No test goes online. The update check, Legible's one network call (only when
  the reader turns it on), is answered by the harness's stand-in in the window's tests, never by
  GitHub, and its request is tested against a server of the test's own on the loopback address.
- **No global keyboard or mouse input, ever,** in tests, probes or scripts: no SendKeys,
  SendInput, keybd_event or mouse_event, wscript or AutoHotkey, and no clicking at screen
  coordinates. It goes to whatever is in the foreground, which is the user's work. Drive
  only your own windows: UI Automation patterns (ValuePattern, InvokePattern and the like) on
  elements found by your own process ID or window handle, messages posted to your own window's
  handle, or script run in your own WebView. If you can't, don't drive the real UI: prove it
  with tests, and say so in the pull request.
- **Windows stay quiet.** Every window a test, example or check opens is *quiet*
  (`legible_win::quiet`), so the tests can run while someone types in another app: it opens
  off every monitor at its normal size (WebView2 still lays it out and paints it, and
  `CapturePreview` shots are the same as on screen), without activating and unable to be
  activated (`WS_EX_NOACTIVATE`), with no taskbar button, and its WebView2 never asks for the
  keyboard focus; nothing brings it forward, restores it or maximises it, a single-instance
  hand-off passes no right to the foreground on, nothing launches another app (a browser,
  Explorer, an editor), and no dialog or message box is shown (each is taken as cancelled). A
  window a test places itself (the placement case) goes where it's put, still never active.
  Quiet exists only in builds with the test-only `harness` feature, never the exe's: there,
  `harness::run` and each window example and check turn it on as they start
  (`quiet::enable()`), and a process a test starts gets it through the environment
  (`LEGIBLE_TEST_QUIET=1`, read once). A new example or check that opens a window does the
  same, builds its window with `quiet::window` and its WebView with `with_focused(false)`, and
  watches the focus: `quiet::watch_focus()` as it starts, and `quiet::focus_taken()` failing
  it at the end. The watch hears every moment a window of the process comes to the
  foreground, takes the keyboard focus (WebView2's windows included) or is shown able to be
  activated, so focus taken and given back between checks is caught too. The real-window suite
  checks it after every case (`Driver::focus_kept`), with every window still open being one
  that can't be activated.

## Disk space

Cargo never removes an old test exe: each round of edits builds new ones, and the incremental
caches grow beside them, so a checkout's `target\debug` reaches tens of GB within days. Two
things keep it down:

- **Less debug info in dev and test builds** (`Cargo.toml`): `debug = "line-tables-only"` for
  the workspace's crates, so panics, test failures and backtraces still name the file and line,
  and none for the dependencies. Release builds are unchanged. A debugger shows no local
  variables in a dev build; to get them back for a session, set `CARGO_PROFILE_DEV_DEBUG=full`
  (and rebuild).
- **`pwsh scripts/clean-build.ps1`** removes the workspace crates' dev and test builds (their
  exes, rlibs, every stale test exe and the incremental caches) and keeps the compiled
  dependencies and `target\release`, so the next build stays quick. `-All` removes everything
  cargo built (a full rebuild follows). Neither ever touches `target\ui-shots` (screenshots
  awaiting approval), `target\dist`, or anything outside the checkout's `target` (a cargo build
  folder set elsewhere with `build.build-dir` is left alone), and both stop
  before cleaning anything if `target` is, or holds, a junction or symbolic link (cargo never
  makes one). `pwsh scripts/test-clean-build.ps1` checks all of it in a throwaway workspace, and
  CI's Windows job runs that check. Contributors run the script once their changes are merged
  (AGENTS.md); the release steps run it once a dist build is checked, keeping `target\release`.

The JavaScript bundles leave no cache: `scripts/build-js.ps1` builds in a temp folder and
removes it.

Measured on a 16-thread PC, 27 Oct 2026, with `CARGO_BUILD_JOBS=4` (`cargo nextest run --workspace
--no-run` from an empty target; a "one-line change" is a comment added to `legible-core`'s
`lib.rs`):

| | Full debug info | Line tables only (now) |
|---|---|---|
| `target\debug` after the test build | 530 MB | **332 MB** |
| The test build from empty | 33 s | 20 s |
| A one-line change in `legible-core` | 3 s, +38 MB | 2 s, +34 MB |

`clean-build.ps1` on this checkout's `target` (release build included): 1.2 GB to 0.8 GB, and the
test build after it took 3 s.
