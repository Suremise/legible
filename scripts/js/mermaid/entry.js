// Mermaid, bundled as one script that sets `window.mermaid`, as Mermaid's own prebuilt bundle
// does, but without the ELK layout: elkjs is EPL-2.0, which Legible doesn't ship, so build.mjs
// replaces it with elk-stub.js. Build-only: `scripts/build-js.ps1 -Bundle mermaid` bundles it
// with the pinned esbuild into third_party/js/mermaid.min.js.
import mermaid from "mermaid";

// Mermaid 12's default layout is ELK, so without it every flowchart would fail. Legible's
// default is dagre (MIT), Mermaid's default before 12, whatever the page passes to initialize;
// a page or a diagram can still name a layout itself.
const initialize = mermaid.initialize.bind(mermaid);
mermaid.initialize = (config = {}) => initialize({ layout: "dagre", ...config });
mermaid.initialize();

globalThis.mermaid = mermaid;
