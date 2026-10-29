# Legible's web assets

Compiled into the exe (`legible_core::assets`), so the portable zip needs no extra files.

- `doc/`: the document's CSS and render-time JavaScript.
- `chrome/`: the toolbar, outline and find.
- `editor/`: the editor, built on CodeMirror.

The vendored bundles (highlight.js, Mermaid, CodeMirror) are in `third_party/js/`, served as `js/…`.
Only Legible's own files go here: no fonts (system fonts only) and nothing fetched at run time.
