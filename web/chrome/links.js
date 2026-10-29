// Run in every page of a Legible window, before the page's own scripts: links never navigate
// the page. Each click on a link sends its href, as the document wrote it, to the window,
// which decides what it does (legible_win::links). Links within the page (#heading) scroll
// as usual. The page also says when it has first been drawn, for the start-up timing.
(() => {
  if (location.origin !== "https://legible.localhost") return;
  const send = (message) => window.ipc.postMessage(message);

  const onClick = (event) => {
    if (event.button > 1) return;
    const link = event.target instanceof Element && event.target.closest("a[href]");
    if (!link) return;
    const href = link.getAttribute("href");
    if (href.trim().startsWith("#") && event.type === "click" && event.button === 0) return;
    event.preventDefault();
    send("link\n" + href);
  };
  document.addEventListener("click", onClick, true);
  document.addEventListener("auxclick", onClick, true);

  document.addEventListener("DOMContentLoaded", () => {
    requestAnimationFrame(() => requestAnimationFrame(() => send("drawn")));
  });
})();
