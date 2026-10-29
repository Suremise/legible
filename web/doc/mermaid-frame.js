// The diagram frame's side of the channel (document.js has the page's). It runs in a sandboxed
// frame with an opaque origin: Mermaid, and anything a diagram's text might make it do, can't
// reach the page, its document or the window's bridge to Legible.
//
// The page posts {id, source, theme, background, font}; the frame lays the diagram out and posts
// back {id, svg} or {id, error}, one at a time, in order. Once Mermaid has loaded it posts
// {ready: true}. Anything not from the page, or not shaped like a request, is ignored.

(() => {
  "use strict";

  // The page is the frame's parent, on Legible's origin (frame-ancestors makes sure of that):
  // replies go only there.
  const parentOrigin = location.ancestorOrigins?.[0] ?? "*";
  const reply = (message) => window.parent.postMessage(message, parentOrigin);

  const ID = /^legible-diagram-\d{1,9}$/;
  const COLOUR = /^rgba?\([\d.,\s/%]{1,60}\)$/;
  const FONT = /^[\w\s,'"-]{0,200}$/;

  function isRequest(data) {
    return (
      data !== null &&
      typeof data === "object" &&
      typeof data.id === "string" &&
      ID.test(data.id) &&
      typeof data.source === "string" &&
      (data.theme === "light" || data.theme === "dark") &&
      typeof data.background === "string" &&
      COLOUR.test(data.background) &&
      typeof data.font === "string" &&
      FONT.test(data.font)
    );
  }

  function config({ theme, background, font }) {
    return {
      startOnLoad: false,
      securityLevel: "strict",
      theme: theme === "dark" ? "dark" : "neutral",
      fontFamily: font,
      // Labels on the lines sit on the page, not in boxes of their own.
      themeVariables: { background, edgeLabelBackground: background },
    };
  }

  // The SVG as plain data, which the page builds its own elements from (it parses no markup):
  // an element is {ns, name, attrs: [[ns, name, value]], children}, a text node its text.
  function tree(node) {
    if (node.nodeType === Node.TEXT_NODE || node.nodeType === Node.CDATA_SECTION_NODE) {
      return node.data;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return null;
    return {
      ns: node.namespaceURI,
      name: node.localName,
      attrs: [...node.attributes].map((a) => [a.namespaceURI, a.name, a.value]),
      children: [...node.childNodes].map(tree).filter((child) => child !== null),
    };
  }

  // Requests are drawn one after another: Mermaid draws one diagram at a time.
  let queue = Promise.resolve();

  async function draw(request) {
    const { id } = request;
    try {
      window.mermaid.initialize(config(request));
      const { svg } = await window.mermaid.render(id, request.source);
      const parsed = new DOMParser().parseFromString(svg, "image/svg+xml");
      reply({ id, svg: tree(parsed.documentElement) });
    } catch (e) {
      reply({ id, error: String(e?.message ?? e).slice(0, 500) });
    } finally {
      // Mermaid leaves its scratch element behind when a diagram fails.
      document.getElementById(`d${id}`)?.remove();
    }
  }

  window.addEventListener("message", (event) => {
    if (event.source !== window.parent || !isRequest(event.data)) return;
    const request = event.data;
    queue = queue.then(() => draw(request));
  });

  reply({ ready: !!window.mermaid });
})();
