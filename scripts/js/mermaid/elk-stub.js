// Stands in for elkjs (EPL-2.0), which Legible doesn't ship. Mermaid uses ELK only for the ELK
// layout, its default from Mermaid 12 (entry.js makes dagre the default instead). A diagram that
// names the ELK layout itself (`layout: elk` in its config) fails to lay out, and Mermaid shows
// its error in the diagram's place; `flowchart-elk` is laid out with dagre.
export default class ELK {
  layout() {
    return Promise.reject(new Error("The ELK layout isn't available in Legible."));
  }
}
