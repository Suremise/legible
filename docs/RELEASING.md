# Releasing Legible

A release is two files and their checksums, built by `release.yml` on a version tag and
published by hand after the manual checklist below:

| File | What |
|---|---|
| `legible-<version>-x64.msi` | Installs per user, with no admin rights, to `%LOCALAPPDATA%\Programs\Legible`, and registers Legible for `.md`, `.markdown` and `.mdown` |
| `legible-<version>-portable.zip` | The same files and an empty `portable` marker: unzip anywhere, and everything is kept beside the exe; it registers nothing unless asked |
| `SHA256SUMS.txt` | The SHA-256 of both, as `sha256sum -c` reads them; users compare their download with it |

Both hold `legible.exe`, `LICENSE.txt` and `THIRD-PARTY-NOTICES.txt`. Nothing is signed yet, so
Windows SmartScreen warns about both (see the checklist).

## Steps

1. **Update what changed.**
   - If a JavaScript pin moved (`third_party/js/VERSIONS`): `node scripts/audit-js-licences.mjs`,
     and review the diff of `third_party/js/LICENSES.md` and `NOTICES.txt`. A licence beyond MIT,
     BSD, ISC or Apache-2.0 (and the others `deny.toml` allows) needs the maintainer's decision.
   - Regenerate the notices: `scripts/third-party-notices.ps1` (needs cargo-about 0.9.2, below).
     Commit `THIRD-PARTY-NOTICES.txt` if it changed: About shows it, and the release fails if
     it's out of date.
   - Move `CHANGELOG.md`'s Unreleased notes under a dated section for the version
     (`## [<version>] - <yyyy-mm-dd>`), and leave an empty `## [Unreleased]` above it. The
     release's notes are that section, so write it for the people downloading Legible.
2. **Set the version** in `Cargo.toml` (`[workspace.package] version`), and run `cargo check` so
   `Cargo.lock` follows. Regenerate the notices again (they name the version).
3. **Build locally and check** (optional, but it's what the workflow does):
   `scripts/package.ps1` writes `target\dist\`. It checks, and fails on, each of: the notices
   up to date, the bundled JavaScript the pinned files, the exe's version resource and icons,
   its imports (no C runtime DLL), the MSI's text from Fluent, its upgrade settings, its
   associations (offered for each extension, never anyone's default), opening Legible after an
   interactive install only, and the zip's four files. **Don't install the MSI on a machine
   that matters:** look inside the zip, and decompile the MSI
   (`wix msi decompile target\dist\legible-<version>-x64.msi -o x.wxs`). Once the build is
   checked, run `pwsh scripts/clean-build.ps1`: it keeps `target\release` and `target\dist`, and
   removes the workspace crates' dev and test builds, so the next dist build stays quick and the
   checkout doesn't grow by gigabytes (docs/TESTING.md, Disk space).
   CI runs on GitHub on every push to main: fmt, clippy, the tests, and `legible-core` on Linux.
   A push of only docs and pictures (the list in `ci.yml`) doesn't run it, and has no run to wait
   for; tag a commit whose last code change has a green run.
4. **Commit, push and tag** (only with the maintainer's say-so): commit and push main, and wait
   for its CI run to pass; then `git tag v<version>` on that commit and push the tag.
   `release.yml` checks the tag matches the version, runs `cargo deny check`, the JavaScript
   check and the tests, builds the exe, the MSI, the zip and the checksums, and
   **drafts** a GitHub release with them, its notes the version's section of `CHANGELOG.md`
   (it fails if there isn't one).
5. **Run the manual checklist** below on the draft's files, on a test machine or account.
6. **Publish the draft** once the checklist passes.

### Build-only tools

| Tool | Install |
|---|---|
| Rust | `rust-toolchain.toml` (rustup) |
| cargo-deny | `cargo install cargo-deny --locked` |
| cargo-about 0.9.2 | `cargo install cargo-about --version 0.9.2 --locked --features cli` |
| cargo-nextest 0.9.149 | `cargo install cargo-nextest --version 0.9.149 --locked`: runs the tests (docs/TESTING.md) |
| WiX Toolset 5.0.2 | `dotnet tool install --global wix --version 5.0.2` (needs the .NET 8 SDK) |
| Node.js 22.4 or later | the JavaScript licence audit and the chrome's browser tests |
| MSVC build tools and Windows SDK | Visual Studio's C++ build tools: the linker, `rc.exe`, and the C runtime's static libraries |

**The C runtime is linked into the exe** (`.cargo/config.toml`, `+crt-static`), so it needs no
Visual C++ redistributable, which would need an admin install. It is the licence rule's one
native exception (docs/ARCHITECTURE.md, Licences): the parts of Microsoft's C runtime linked in
(the static `libcmt`, `libucrt` and `libvcruntime`, unmodified) aren't MIT or similar but
"Distributable Code" under the Visual Studio licence terms, which allow shipping them in an app.
`cargo deny` and cargo-about don't see them, so `scripts/third-party-notices.ps1` writes their
entry in THIRD-PARTY-NOTICES by hand. `package.ps1` checks the exe imports no C runtime DLL and
nothing that isn't part of Windows.

**WiX 5, not cargo-wix or WiX 6+:** `cargo-wix` drives WiX 3, which needs the .NET Framework 3.5
(an admin install); WiX 6 and later come with the Open Source Maintenance Fee terms. WiX 5 is
MS-RL and only builds the MSI: none of it ships in it. The MSI uses **no WiX extension and no
WixUI**, whose dialogs, bitmaps and custom-action DLLs (MS-RL) would be embedded in it;
Windows shows its own progress window instead.

## The MSI

`installer/legible.wxs`:

- **Per user, no admin** (`Scope="perUser"`), to `%LOCALAPPDATA%\Programs\Legible`; components
  keep their key paths under `HKCU\Software\Legible\Installer`. A Start menu shortcut, and the
  app icon in Apps & features.
- **Its own text** (the downgrade message, the shortcut's description, Markdown files' type name
  and Default apps' line about Legible) is the `installer-*` messages in
  `locales/en/window.ftl`; `package.ps1` passes it in and checks the built MSI carries it.
- **File associations, per user:** the ProgID `Legible.Markdown` ("Markdown document", the file
  icon in `legible.exe` as `"legible.exe",-2`, and the open command passing `"%1"`), listed under
  `OpenWithProgids` for `.md`, `.markdown` and `.mdown`; `Applications\legible.exe` with its
  `SupportedTypes`; and Legible's capabilities under `RegisteredApplications`, which Default
  apps lists it by. **It never takes over the default:** Windows keeps that the user's choice.
  These are the values `crates/legible-win/src/associations.rs` registers for a portable copy,
  and a test there keeps the two the same. Uninstalling removes exactly these values; another
  app's under the same extensions stay.
- **The default app:** while Legible is registered but isn't the `.md` default, each window
  offers "Make Legible the default for Markdown files", which opens Default apps at Legible
  (`ms-settings:defaultapps?registeredAppUser=Legible`: Windows 11 opens Legible's own page;
  Windows 10 opens Default apps) and says in one line what to choose. "Don't ask again" (or
  taking the offer) is remembered (`offer_default_app` in `settings.toml`); Close puts it off
  until Legible starts again; Settings keeps the button.
- **After an interactive install or upgrade Legible opens**, so the user sees it worked; a
  silent or basic-UI one (`/qn`, `/qb`: `UILevel` 3 or less) starts nothing.
- **Upgrades** are major upgrades (same `UpgradeCode`), **the same version included**
  (`AllowSameVersionUpgrades`): every build has a new `ProductCode`, so without it a rebuilt MSI
  of the same version installs beside the old one, and the installer keeps the old exe (equal
  file versions). The old installs are removed first (`Schedule="afterInstallInitialize"`), all
  of them. WiX's ICE61 warning about same-version upgrades is expected. `package.ps1` checks the
  built MSI has both settings, and that the exe carries a version resource of the crate's
  version (`crates/legible/build.rs`).
- **Uninstalling** leaves the settings, recent files and logs in `%APPDATA%\Legible`.

## The portable copy

The zip's `portable` marker keeps everything beside the exe (`paths.rs`). It registers no file
associations until the user asks in Settings ("Register Legible for .md files on this PC"),
which writes the same values as the MSI, for that copy's exe; "Remove Legible from this PC's
apps for .md files" removes them again, and only them.

## Manual checklist

Copy this for each release, fill it in and keep it with the release (in its notes, or attached
to them).

**Release:** v______ **Date:** ________ **Machine and Windows version:** ______________________
**Tester:** ________

| Done | Check | Result and notes |
|:---:|---|---|
| [ ] | **Install without admin:** double-click the MSI as a standard user; no UAC prompt; Legible opens; the Start menu shortcut works, with the app icon | |
| [ ] | **Install over an existing install:** install the previous version, then this one (and this one over itself); Apps & features shows **one** Legible, the exe's Properties > Details show this version, and the settings and recent files are kept | |
| [ ] | **Silent install:** `msiexec /i legible-<version>-x64.msi /qn` installs without a window and **doesn't open Legible** | |
| [ ] | **Double-click from Explorer:** a `.md`, a `.markdown` and a `.mdown` file; Open with lists Legible, with the Markdown file icon and "Markdown document" as the type | |
| [ ] | **Double-click from an email attachment:** a `.md` attached in Outlook opens in Legible (from Outlook's temporary folder) | |
| [ ] | **The default-app flow:** on first run the bar offers it; "Make Legible the default for Markdown files" opens Default apps at Legible and the window says what to choose; once chosen, double-clicking a `.md` opens Legible and the bar doesn't come back; Close hides it until Legible starts again; on another account, "Don't ask again" is remembered after a restart, and Settings still has the button | |
| [ ] | **Pastes:** Copy document and Copy selection, pasted into Outlook, Word and OneNote: headings, lists, tables, code, links and diagrams as pictures; plain text where only text is taken | |
| [ ] | **Print and PDF:** print a long document (page numbers in the footer); Save as PDF, then Open and Show in folder | |
| [ ] | **Live reload:** change the open file in another editor; the window updates, keeps its place and says "Updated"; move and delete the file and see the bar | |
| [ ] | **A large document:** a 2 MB Markdown file opens, scrolls and finds smoothly | |
| [ ] | **100% and 150%:** on a 100% and a 150% display (and moving a window between them): sharp text and icons, the bar and menus laid out | |
| [ ] | **Updates:** with Check for updates off (the default), Legible makes no connection at start (Resource Monitor or a firewall log); turned on, Settings checks and says what it found, and Help > Check for updates says so in the window; with a newer release out, the bar's See what's new opens its GitHub page | |
| [ ] | **No network while remote images are blocked:** with pictures from the internet set to Ask (and Never), open a document with remote pictures and watch with Resource Monitor (or a firewall log) that Legible makes no connection until Show once | |
| [ ] | **SmartScreen's unsigned warning:** on downloading and running both files; note what a user has to click | |
| [ ] | **Uninstall:** from Settings > Apps; Legible's associations go (Open with no longer lists it), another app's stay; settings and recent files in `%APPDATA%\Legible` stay | |
| [ ] | **Portable:** unzip and run; nothing is registered (Open with doesn't list it) until Settings' Register; then it's listed for `.md`; Remove takes it out again; settings are kept beside the exe | |
| [ ] | **A clean Windows VM** (no Visual C++ redistributable installed): the portable copy starts, and so does the MSI's | |
| [ ] | **Checksums:** `SHA256SUMS.txt` matches both downloaded files | |
