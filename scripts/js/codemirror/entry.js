// The CodeMirror 6 modules Legible's editor (web/editor/) uses, bundled as one script that sets
// `window.CM`. Build-only: `scripts/build-js.ps1 -Bundle codemirror` bundles it with the pinned
// esbuild into third_party/js/codemirror.min.js. Add an export here only when the editor needs it.
export * as state from "@codemirror/state";
export * as view from "@codemirror/view";
export * as commands from "@codemirror/commands";
export * as language from "@codemirror/language";
export * as search from "@codemirror/search";
export * as markdown from "@codemirror/lang-markdown";
export * as highlight from "@lezer/highlight";
