# Contributing to Legible

Thanks for helping. Bug reports, fixes, translations and ideas are all welcome. Please read the
[Code of Conduct](CODE_OF_CONDUCT.md) first, and report security problems privately, as
[SECURITY.md](SECURITY.md) says, not in a public issue.

For anything bigger than a small fix, open an issue first so we can agree on the shape before you
spend time on it. Legible is a reader first, for people who aren't developers: a change should
keep it simple to use, quick to open and safe with any document.

## Building and checking

You need Windows 10 or 11, [Rust](https://rustup.rs), Visual Studio's C++ build tools with the
Windows SDK, and PowerShell 7. The toolchain is pinned in `rust-toolchain.toml`, and rustup
installs it on first use. [Node.js](https://nodejs.org) 22.4 or later runs the browser tests
(they use its built-in WebSocket), and Microsoft Edge (already on Windows) runs them headless.

```
cargo build
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace        # the tests, in parallel (cargo install cargo-nextest)
cargo test --workspace --doc         # the doc tests, which nextest doesn't run
cargo deny check                     # licences, advisories and sources (cargo install cargo-deny)
pwsh scripts/fetch-js.ps1 -VerifyOnly  # the bundled JavaScript is the pinned, audited files
node scripts/doc-tests.mjs           # the document's script, in headless Edge
node scripts/chrome-tests.mjs        # the toolbar, outline, find, menus and editor, in headless Edge
```

`cargo test --workspace` runs the same Rust tests without nextest, more slowly.
[docs/TESTING.md](docs/TESTING.md) says what to run when (screenshots, the checks that open a
real window, the real clipboard by hand), how to run only the tests you changed, and the rules
every test keeps.

**Disk space.** Cargo never removes old test exes, so `pwsh scripts/clean-build.ps1` now and then
frees what piles up in `target\` while keeping the compiled dependencies (`-All` removes
everything cargo built). docs/TESTING.md, Disk space, has the numbers.

A pull request must pass all of these, so run them before you open one: nothing runs on GitHub
for a push or a pull request. The checks run there on a version tag, before the release is built,
and by hand from the Actions tab, where CI runs the Rust checks and the JavaScript check on
Windows, and `legible-core`'s tests on Linux too. Run the two browser tests yourself whenever you
change `web/` (docs/TESTING.md says which).

## How the code is laid out

```
crates/
  legible-core/   platform-free: Markdown to safe HTML (front matter, the parser, the
                  sanitiser), the clipboard's HTML, saving, the strings, the embedded web assets
  legible-win/    Windows only: the document windows and WebView2, links, pictures, live
                  reload, the editor's window side, saving to disk, the clipboard, Print and
                  PDF, the menus, file associations, settings, paths and logs
  legible/        the exe: starts up and wires the two together
web/              the page's own HTML, CSS and JavaScript: doc/ (the document's look), chrome/
                  (the toolbar, outline, find, Settings and About), editor/ (Split and Edit)
third_party/js/   the pinned JavaScript bundles (highlight.js, Mermaid, CodeMirror) and their audit
locales/en/       the strings, in Fluent
docs/             ARCHITECTURE.md, TESTING.md, RELEASING.md and the user guide
installer/        the MSI's source
assets/           the icons, drawn by assets/export.py
scripts/          build-only scripts and the browser tests
```

**`legible-core` must build and test on any OS**: it does no I/O to render, and nothing in it
calls Windows. Windows code goes in `legible-win`, and `unsafe` only in its `win32` module.
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) explains each part and the decisions behind it.

## Strings and languages

Every word the user sees comes from the [Fluent](https://projectfluent.org) files in
`locales/<language>/`, the page's included. Logs stay in English.

- **Adding a string:** add `key = Text` to the right file in `locales/en/` (`window.ftl`,
  `document.ftl`, `errors.ftl`, ...). Keys are lowercase and hyphenated, prefixed by where they
  show (`chrome-`, `menu-`, `error-`, ...). Add a comment for any argument whose meaning isn't
  obvious. In Rust, use `tr("key")`, or `tr!("key", name = value)` with arguments, and pass the key
  as a literal: a test checks that every key the code uses exists in English. For the page, add a
  `chrome-` key's name (without `chrome-`) to the list in `crates/legible-win/src/page.rs`, and
  the scripts read it with `s("name")`.
- **Adding a language:** create `locales/<tag>/` (such as `locales/fr/`) with any of English's
  files, translating the keys you can. Missing keys fall back to English one by one. Then add it to
  `LOCALES` in `crates/legible-core/src/i18n/embedded.rs`: the tests fail until every file on disk
  is listed there.

## Dependencies: the licence rule

Everything Legible ships must be **MIT or equally permissive**: MIT, Apache-2.0, BSD-2-Clause,
BSD-3-Clause, ISC, Zlib, 0BSD, BSL-1.0, CC0-1.0 or Unicode-3.0 / Unicode-DFS-2016, or a dual
licence that includes one of them. **No copyleft and no extra conditions**: no GPL, LGPL, MPL,
EPL, OFL and the like, and **system fonts only**. The decided exceptions (two MPL-2.0 crates the
sanitiser needs, shipped unmodified) are recorded in docs/ARCHITECTURE.md, Licences.

- **Rust crates:** `cargo deny check` enforces the rule for every crate in the tree (the allowed
  list is in `deny.toml`). Before adding one, check its licence and its dependencies'; prefer a
  well-used, maintained crate, with only the features you need; and say in the pull request what
  it's for and why it's worth it.
- **JavaScript:** the bundles change only through `third_party/js/VERSIONS` and the licence audit,
  `node scripts/audit-js-licences.mjs`, which lists every package inside each bundle and fails on
  a licence beyond the rule. Such a package ships only on the maintainer's decision.
- After either, regenerate THIRD-PARTY-NOTICES.txt with `pwsh scripts/third-party-notices.ps1`
  (docs/RELEASING.md).

Build-only tools (the toolchain, CI actions, scripts) don't ship, so the rule doesn't apply to
them, but list a new one in ARCHITECTURE.md's "Build-only tools".

## Tests

- **Tests use temporary folders only.** A test never reads or writes your real settings, file
  associations or clipboard, and cleans up after itself even when it fails.
- **Tests wait for conditions, never for time,** and pass under load, beside every other test.
- **No network:** Legible makes no network calls of its own, and nor do its tests.
- **No global keyboard or mouse input,** ever: a test drives only its own window, through its own
  page's script or its own window handle.
- A fix comes with a test that fails without it.

## Commits and pull requests

- **A commit message's first line says what changed, in plain English** ("Find opens a folded
  section round its match"), under about 72 characters, with the issue or pull request number at
  the end when there is one ("... (#12)"). Add a body when the why isn't obvious.
- Keep a pull request to one change. Update the docs it affects ([the user guide](docs/user-guide.md)
  for anything a user does, [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how it works), and add
  an `Unreleased` entry in [CHANGELOG.md](CHANGELOG.md) for anything a user would notice.
- A change to what's on screen comes with before and after screenshots (docs/TESTING.md).
- By contributing, you agree that your contribution is under Legible's [MIT licence](LICENSE).
