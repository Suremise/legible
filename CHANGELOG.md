# Changelog

All notable changes to Legible are recorded here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Legible follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-10-29

The first release.

**Not signed yet:** Windows SmartScreen may warn the first time you run the installer or the exe
("Windows protected your PC"). Check the download against the release's `SHA256SUMS.txt` first
(`sha256sum -c SHA256SUMS.txt --ignore-missing`, or compare `Get-FileHash <file>` with it), and
if it matches, choose **More info**, then **Run anyway**.

### Added

- **Markdown that reads like a document,** in a light, dark or Windows theme: clear type at a
  comfortable width, everything GitHub shows (tables, task lists, footnotes, alerts and the
  rest), code coloured by its language, and Mermaid diagrams.
- **Outline and find:** an outline of the headings to move round a long document, find in the
  document (Ctrl+F), and zoom from 50% to 300%.
- **Copy into Outlook, Word and OneNote:** Copy document (Ctrl+Shift+C) and Copy selection keep
  headings, lists, tables, code, links and pictures, with diagrams as pictures.
- **Print and Save as PDF:** the document alone, in the light theme, with its title and page
  numbers.
- **View, Split and Edit** (Ctrl+E): the Markdown text beside the document or alone, the document
  updated as you type, with find and replace and Windows' spell-check.
- **Saving that can't lose work:** a file keeps its own encoding and line endings, and is saved
  only over the file as it was read. A file changed on disk while you edit asks which to keep, and
  closing with unsaved changes asks first.
- **Live reload:** a file changed on disk shows its new version and keeps your place.
- **Pictures from the internet only with your consent:** they're blocked until you allow them,
  once or always for a file, and then only over HTTPS.
- **An installer and a portable zip:** a per-user installer (no admin rights) that registers
  Legible for Markdown files, and a portable zip that changes nothing on the PC unless asked.
- **Privacy:** no telemetry and no network. A document's scripts never run, and links open in
  your browser.
- MIT licensed, with every dependency MIT or equally permissive and system fonts only; the
  licences are in THIRD-PARTY-NOTICES.txt.
