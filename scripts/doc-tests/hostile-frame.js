// A frame taken over (a bug in Mermaid, a diagram that tricks it): it tries to reach the page,
// sends the page junk, and answers each diagram with something hostile. scripts/doc-tests.mjs
// checks the page comes through it. What it learns it reports as {probe} messages.

(() => {
  const SVG = "http://www.w3.org/2000/svg";
  const HTML = "http://www.w3.org/1999/xhtml";
  const XLINK = "http://www.w3.org/1999/xlink";
  const send = (message) => window.parent.postMessage(message, "*");

  const attempt = (name, f) => {
    try {
      f();
      send({ probe: name, reached: true });
    } catch (e) {
      send({ probe: name, reached: false, error: e.name });
    }
  };
  attempt("parent's document", () => window.parent.document.body.textContent);
  attempt("top's document", () => window.top.document.title);
  attempt("parent's script", () => window.parent.LegibleDoc.prepare);
  attempt("the window's bridge through the parent", () => window.parent.ipc.postMessage("drawn"));
  attempt("cookies", () => document.cookie);
  attempt("local storage", () => window.localStorage.length);
  attempt("navigating the page", () => {
    window.top.location.href = "about:blank#pwned";
  });

  // Junk, and answers to diagrams nobody asked for.
  for (const junk of [
    null,
    "legible-diagram-1",
    { ready: "yes" },
    { id: "legible-diagram-999", svg: { ns: SVG, name: "svg", attrs: [], children: [] } },
    { id: 1, svg: "<svg/>" },
    { id: "legible-diagram-1", svg: "<svg id='legible-diagram-1'><script>top.__legiblePwned=1</script></svg>" },
  ]) {
    send(junk);
  }
  send({ ready: true });

  const el = (name, attrs = [], children = [], ns = SVG) => ({
    ns,
    name,
    attrs: attrs.map((a) => (a.length === 3 ? a : [null, ...a])),
    children,
  });

  // Diagram 1: a diagram with everything a page mustn't take in it, beside what it may keep.
  const malicious = (id) =>
    el("svg", [["id", id], ["viewBox", "0 0 100 100"], ["onload", "top.__legiblePwned=10"], ["style", "position:fixed;inset:0;z-index:9999"]], [
      el("style", [], [
        `#${id} .ok { fill: red; }`,
        `body { display: none !important; }`,
        `#${id} ~ * { display: none; }`,
        `#${id}, main { visibility: hidden; }`,
        `@import url("https://example.invalid/x.css");`,
        `@font-face { font-family: x; src: url("https://example.invalid/x.woff"); }`,
      ]),
      el("script", [], ["top.__legiblePwned = 11"]),
      el("rect", [["class", "ok"], ["width", "10"], ["height", "10"], ["onclick", "top.__legiblePwned=12"]]),
      el("a", [["href", "javascript:top.__legiblePwned=13"]], [el("text", [], ["a link"])]),
      el("a", [[XLINK, "xlink:href", "javascript:top.__legiblePwned=14"]], [el("text", [], ["an old link"])]),
      el("use", [["href", "https://example.invalid/sprite.svg#x"]]),
      el("use", [["href", `#${id}-ok`]]),
      el("g", [["id", "document"]], [el("rect", [["id", "LegibleDoc"]])]),
      el("g", [["id", `${id}-ok`]]),
      el("animate", [["attributeName", "href"], ["to", "javascript:top.__legiblePwned=15"]]),
      el("image", [["href", "https://example.invalid/track.png"]]),
      el("foreignObject", [["width", "100"], ["height", "20"]], [
        el("div", [["class", "label"]], ["kept ", el("b", [], ["label"], HTML)], HTML),
        el("iframe", [["srcdoc", "<script>top.__legiblePwned=16</script>"]], [], HTML),
        el("img", [["src", "x"], ["onerror", "top.__legiblePwned=17"]], [], HTML),
        el("form", [["action", "https://example.invalid/"]], [el("input", [["name", "q"]], [], HTML)], HTML),
      ]),
      el("div", [], ["HTML outside a foreignObject"], HTML),
      el("x-unknown", [], [], "urn:x-unknown"),
      el("text", [["x", "1"], ["{bad", "1"], ["xml:space", "preserve"]], ["text"]),
    ]);

  // Diagram 3: nested far deeper than any diagram.
  const deep = (id) => {
    let node = el("g");
    for (let i = 0; i < 500; i++) node = el("g", [], [node]);
    return el("svg", [["id", id]], [node]);
  };

  let requests = 0;
  window.addEventListener("message", (event) => {
    const { id } = event.data ?? {};
    if (typeof id !== "string") return;
    switch (++requests) {
      case 1:
        return send({ id, svg: malicious(id) });
      case 2:
        // Another diagram's id, so its styles would reach that one.
        return send({ id, svg: el("svg", [["id", "legible-diagram-1"]]) });
      case 3:
        return send({ id, svg: deep(id) });
      case 4:
        return send({ id, error: "<img src=x onerror=top.__legiblePwned=18>" });
      default:
        return send({ id, svg: el("svg", [["id", id]], [el("rect", [["width", "1"], ["height", "1"]])]) });
    }
  });
})();
