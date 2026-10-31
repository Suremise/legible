// The window's chrome round the document: the toolbar, the outline, find, zoom, the reading
// width and style, Settings and About. Run deferred, after document.js.
//
// Strings come from <script type="application/json" id="legible-chrome-strings"> and the
// settings from id="legible-settings" (legible_win::page). The page talks to the window with
// window.ipc.postMessage, one message a line or two:
//   open                       the Open dialog
//   mode\n<view|split|edit>    the mode (web/editor/editor.js changes it and says so)
//   setting\n<key>\n<value>    a setting changed (the window checks, saves and sends it to
//                              every window)
//   about                      About opened: its table's facts → about({rows})
//   about-copy                 About's Copy: the window copies its own facts → aboutCopied()
//   reading\n<licence|notices|changelog>  About's reading view → reading(name, text, html)
//   dirty\n<true|false>        the editor has (or no longer has) unsaved edits (editor.js
//                              sends it)
//   reload\n<id> / keep\n<id>  the answer to held change <id>: take it from disk, or keep
//                              the edits (live reload's hook for safe saving)
//   close                      the moved-or-deleted bar's Close, and Ctrl+W
//   outline\n<true|false>      the Outline button is (or no longer is) pressed: the menu bar
//                              ticks Outline as it is
//   status                     the status bar wants what it shows of the file → status(info)
//   copy-path                  the status bar's path was clicked: copy it → pathCopied()
//   file-keep                  its Keep showing it (remembered for a page served again)
//   file-status                the page wants to know whether to show that bar
//   copy\n<id>\n<generation>\n<html>  Copy document or Copy selection: the HTML to copy
//   save-pdf                   Save as PDF; pdf-open, pdf-show  the PDF just saved
//   recent                     the page wants the recent files
//   open-recent\n<token>\n<i>  open recent file <i> of the list sent as <token>
//   open-editor, show-folder   Open in editor and Show in folder, for this window's file
//   remote-images              the page wants what its pictures from the internet may do
//   remote-images\n<once|always>\n<generation>  the bar's answer, for that generation
//   default-app                whether Legible is registered for .md and is its default
//   default-app-open           open Windows' Default apps at Legible
//   default-app-close          the bar's Close: not offered again until Legible starts again
//   default-app-register, default-app-unregister  a portable copy's file associations
// and the window answers through window.LegibleChrome: apply(settings), saveFailed(text),
// about({rows}), aboutCopied(), reading(name, text, html), replaceDocument({generation, html}) (live reload), updated() (the editor showed
// the file as it now is on disk), fileGone(bool), setRecent({token, items: [{name, path}]}),
// notice(text), remoteImages({generation, show, ask, unavailable}), adopt({generation, remote})
// (the editor's preview shows that render), reloadWaits(bool) (a reload held until the edits are
// saved), defaultApp({offer, registered, isDefault, portable}) and layout() (the editor's
// conflict bar, #conflict-bar, came or went: it stacks under the others), status({path, note,
// encoding, endings}) and pathCopied() (the status bar), and command(name) (an
// item of the window's menu bar, legible_win::menu: what its button or shortcut here does);
// replaceDocument may carry
// `remote`, the same for its generation. A change
// held while there are unsaved edits is announced as a `legible:external-change` event on
// window, with detail {kind: "changed" | "deleted", id}.
//
// The page's CSP allows no inline styles: everything here is built with createElement and
// textContent, and what changes with the layout (the zoom, the outline's width, the chrome's
// height) is set through the CSSOM, as custom properties on <html>, which window.css reads.
(() => {
  "use strict";

  const root = document.documentElement;
  const doc = document.getElementById("document");
  const data = (id) => JSON.parse(document.getElementById(id).textContent);
  const strings = data("legible-chrome-strings");
  let settings = data("legible-settings");
  const { editable } = data("legible-editor");
  const s = (key) => strings[key] ?? key;
  const send = (message) => window.ipc?.postMessage(message);
  /** A shortcut as a tooltip names it: "Ctrl", "Shift", "Enter" and "Esc" in Fluent's words. */
  const keys = (...parts) => parts.map((k) => (["Ctrl", "Shift", "Enter", "Esc"].includes(k) ? s(`key-${k.toLowerCase()}`) : k)).join("+");
  /** A tooltip: `label` with its shortcut's `keys`, or with a `note`. */
  const tipWithKeys = (label, shortcut) => s("tip-shortcut").replace("{label}", () => label).replace("{keys}", () => shortcut);
  const tipWithNote = (label, note) => s("tip-note").replace("{label}", () => label).replace("{note}", () => note);

  const ZOOM_STEPS = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];
  const TOOLBAR_HEIGHT = 40;
  const OUTLINE_MIN = 160;
  const OUTLINE_MAX = 480;
  // Never part of a match: a separator between blocks, so a match can't run from one into the
  // next.
  const BLOCK_BREAK = String.fromCharCode(0);

  // --- Building ------------------------------------------------------------------------------

  /** An element: `props` are its attributes (className, text and on* aside), then children. */
  function el(tag, props = {}, ...children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
      if (value === undefined || value === null || value === false) continue;
      if (key === "className") node.className = value;
      else if (key === "text") node.textContent = value;
      else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
      else node.setAttribute(key, value === true ? "" : value);
    }
    node.append(...children.filter(Boolean));
    return node;
  }

  const SVG = "http://www.w3.org/2000/svg";
  /** A 16-pixel line icon from path data, drawn in the text colour. */
  function icon(...paths) {
    const svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("viewBox", "0 0 16 16");
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("class", "icon");
    for (const d of paths) {
      const path = document.createElementNS(SVG, "path");
      path.setAttribute("d", d);
      svg.append(path);
    }
    return svg;
  }

  const ICONS = {
    open: ["M1.5 4.5v8a1 1 0 0 0 1 1h11a1 1 0 0 0 1-1v-6a1 1 0 0 0-1-1h-6l-1.5-2h-3.5a1 1 0 0 0-1 1z"],
    outline: ["M2 3.5h1.5M5.5 3.5h8.5M4 8h1.5M7.5 8h6.5M4 12.5h1.5M7.5 12.5h6.5"],
    find: ["M6.75 2a4.75 4.75 0 1 1 0 9.5a4.75 4.75 0 0 1 0-9.5zM10.2 10.2l4.3 4.3"],
    reading: ["M1.5 8h13M4 5.5l-2.5 2.5l2.5 2.5M12 5.5l2.5 2.5l-2.5 2.5"],
    print: ["M4 6V1.5h8V6M4 11.5H2.5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h11a1 1 0 0 1 1 1v3.5a1 1 0 0 1-1 1H12M4 9.5h8v5H4z"],
    copy: ["M5.5 5.5h8v9h-8zM10.5 5.5V1.5h-8v9h3"],
    more: ["M3 8h.01M8 8h.01M13 8h.01"],
    up: ["M3.5 10l4.5-4.5l4.5 4.5"],
    down: ["M3.5 6l4.5 4.5l4.5-4.5"],
    close: ["M3.5 3.5l9 9M12.5 3.5l-9 9"],
    // The modes: an open book (reading), a window in two panes, a pencil.
    view: ["M8 4.5C6.5 3.3 4.2 2.9 1.5 3v9.5c2.7-.1 5 .3 6.5 1.5c1.5-1.2 3.8-1.6 6.5-1.5V3c-2.7-.1-5 .3-6.5 1.5zM8 4.5v9"],
    split: ["M2.5 2.5h11a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1zM8 2.5v11"],
    edit: ["M2 14l.75-3.25L10.5 3a1.06 1.06 0 0 1 1.5 0l1 1a1.06 1.06 0 0 1 0 1.5L5.25 13.25zM9.5 4l2.5 2.5"],
  };

  /** A toolbar button: `label` is its accessible name, and its tooltip with `shortcut`. */
  function button(label, { shortcut, iconName, text, onclick, soon, className, attrs = {} } = {}) {
    const tip = soon ? tipWithNote(label, s("coming-soon")) : shortcut ? tipWithKeys(label, shortcut) : label;
    const node = el(
      "button",
      {
        type: "button",
        className: ["tool", className].filter(Boolean).join(" "),
        "aria-label": label,
        title: tip,
        "aria-disabled": soon ? "true" : undefined,
        onclick: soon ? undefined : onclick,
        ...attrs,
      },
      iconName ? icon(...ICONS[iconName]) : null,
      text ? el("span", { text }) : null,
    );
    return node;
  }

  // --- Menus (the WAI-ARIA menu button pattern) ----------------------------------------------

  let openMenu = null;

  /** A button that opens a menu of `items` (or a function giving them, called each time it
      opens): { label, onselect, soon, checked(), title, hint } or "-", or { heading }. */
  function menuButton(label, iconName, items, shortcut) {
    const id = `menu-${Math.random().toString(36).slice(2)}`;
    const trigger = button(label, {
      iconName,
      shortcut,
      attrs: { "aria-haspopup": "menu", "aria-expanded": "false", "aria-controls": id },
    });
    const menu = el("div", { role: "menu", id, className: "menu", "aria-label": label, hidden: true });
    let entries = [];
    function build() {
      entries = [];
      menu.replaceChildren();
      for (const item of typeof items === "function" ? items() : items) {
        if (item === "-") {
          menu.append(el("div", { role: "separator", className: "menu-separator" }));
          continue;
        }
        if (item.heading) {
          menu.append(el("div", { role: "presentation", className: "menu-heading", text: item.heading }));
          continue;
        }
        const radio = typeof item.checked === "function";
        const entry = el(
          "div",
          {
            role: radio ? "menuitemradio" : "menuitem",
            tabindex: "-1",
            className: "menu-item",
            "aria-disabled": item.soon || item.disabled ? "true" : undefined,
            title: item.soon ? tipWithNote(item.label, s("coming-soon")) : item.title,
            onclick: () => {
              if (item.soon || item.disabled) return;
              close();
              item.onselect();
            },
          },
          el("span", { className: "check", "aria-hidden": "true" }),
          el("span", { text: item.label }),
          item.hint ? el("span", { className: "menu-hint", text: item.hint }) : "",
        );
        entry.legibleItem = item;
        entries.push(entry);
        menu.append(entry);
      }
    }
    build();

    const refresh = () => {
      for (const entry of entries) {
        const { checked } = entry.legibleItem;
        if (typeof checked === "function") entry.setAttribute("aria-checked", String(checked()));
      }
    };
    const focusAt = (i) => entries[(i + entries.length) % entries.length].focus();
    function open(focusLast = false) {
      if (openMenu && openMenu !== api) openMenu.close(false);
      if (typeof items === "function") build();
      refresh();
      menu.hidden = false;
      trigger.setAttribute("aria-expanded", "true");
      openMenu = api;
      focusAt(focusLast ? entries.length - 1 : 0);
    }
    function close(returnFocus = true) {
      if (menu.hidden) return;
      menu.hidden = true;
      trigger.setAttribute("aria-expanded", "false");
      if (openMenu === api) openMenu = null;
      if (returnFocus) trigger.focus();
    }
    const api = { close, menu, trigger };

    trigger.addEventListener("click", () => (menu.hidden ? open() : close()));
    trigger.addEventListener("keydown", (event) => {
      if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        open();
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        open(true);
      }
    });
    menu.addEventListener("keydown", (event) => {
      const i = entries.indexOf(document.activeElement);
      switch (event.key) {
        case "ArrowDown":
          focusAt(i + 1);
          break;
        case "ArrowUp":
          focusAt(i - 1);
          break;
        case "Home":
          focusAt(0);
          break;
        case "End":
          focusAt(entries.length - 1);
          break;
        case "Escape":
          close();
          break;
        case "Tab":
          close(false);
          return;
        case "Enter":
        case " ":
          document.activeElement?.click();
          break;
        default:
          return;
      }
      event.preventDefault();
      event.stopPropagation();
    });
    const wrap = el("div", { className: "menu-wrap" }, trigger, menu);
    return { wrap, ...api };
  }

  document.addEventListener("pointerdown", (event) => {
    if (openMenu && !openMenu.menu.parentElement.contains(event.target)) openMenu.close(false);
  });

  // --- The view: theme, width and style, as classes on <html> ---------------------------------

  function applyView() {
    root.classList.toggle("theme-light", settings.theme === "light");
    root.classList.toggle("theme-dark", settings.theme === "dark");
    root.classList.toggle("width-narrow", settings.width === "narrow");
    root.classList.toggle("width-wide", settings.width === "wide");
    root.classList.toggle("font-serif", settings.style === "serif");
    showStatusBar(settings["status-bar"] === true);
  }

  /** Tells the editor the settings (its font, its share in Split). */
  const shareSettings = () => document.dispatchEvent(new CustomEvent("legible-settings", { detail: settings }));

  /** Changes a setting here at once, and asks the window to save it and tell the others. */
  function change(key, value) {
    if (key === "outline" && value !== settings.outline) outlineByHand = false;
    settings = { ...settings, [key]: value };
    applyView();
    shareSettings();
    updateOutlineVisibility();
    send(`setting\n${key}\n${value}`);
  }

  // --- Zoom: the document only, for this window ----------------------------------------------

  let zoom = ZOOM_STEPS.includes(settings.zoom) ? settings.zoom : 100;
  let zoomLabel;
  // The zoom --zoom gives: window.css's own default until it's first set. A custom property on
  // <html> restyles the whole document, so it's set only when it changes.
  let zoomShown = 100;

  function setZoom(percent) {
    zoom = Math.min(300, Math.max(50, percent));
    if (zoom !== zoomShown) {
      zoomShown = zoom;
      root.style.setProperty("--zoom", String(zoom / 100));
    }
    if (zoomLabel) zoomLabel.textContent = zoomText();
    scheduleLayout();
  }
  /** The zoom as the language writes it (window.ftl's chrome-zoom-level). */
  function zoomText() {
    return s("zoom-level").replaceAll("{zoom}", () => zoom.toLocaleString(root.lang || undefined));
  }
  const zoomBy = (direction) => {
    const next =
      direction > 0 ? ZOOM_STEPS.find((z) => z > zoom) : [...ZOOM_STEPS].reverse().find((z) => z < zoom);
    if (next) setZoom(next);
  };

  // --- The toolbar ---------------------------------------------------------------------------

  // View, Split and Edit (Ctrl+E goes round them), as icons named by their modes: only modes,
  // always. On a file that can't be edited where it is, Split and Edit are there but unusable,
  // their tooltips saying how to edit a copy (File's Save a copy…, Ctrl+Shift+S).
  const modeButtons = ["view", "split", "edit"].map((mode) => {
    const usable = editable || mode === "view";
    const node = button(s(`mode-${mode}`), {
      iconName: mode,
      className: "segment",
      shortcut: s("mode-cycles"),
      attrs: { "aria-pressed": String(mode === "view"), "data-mode": mode, "aria-disabled": usable ? undefined : "true" },
      onclick: usable ? () => window.LegibleEditor?.setMode(mode) : undefined,
    });
    if (!usable) node.title = tipWithNote(s(`mode-${mode}`), s("mode-not-editable"));
    return node;
  });
  const unsaved = el("span", { className: "unsaved", role: "img", "aria-label": s("unsaved"), title: s("unsaved"), hidden: true });
  document.addEventListener("legible-mode", (event) => {
    for (const b of modeButtons) {
      if (b.dataset.mode) b.setAttribute("aria-pressed", String(b.dataset.mode === event.detail));
    }
  });
  document.addEventListener("legible-dirty", (event) => (unsaved.hidden = !event.detail));
  // The editor's preview replaced the document: the outline follows.
  document.addEventListener("legible-document-changed", () => documentChanged());

  const outlineButton = button(s("outline"), {
    iconName: "outline",
    shortcut: keys("Ctrl", "Shift", "O"),
    attrs: { "aria-pressed": "false", "aria-controls": "outline" },
    onclick: () => toggleOutline(),
  });
  const findButton = button(s("find"), {
    iconName: "find",
    shortcut: keys("Ctrl", "F"),
    attrs: { "aria-pressed": "false", "aria-controls": "find" },
    onclick: () => (findBar.hidden ? openFind() : closeFind()),
  });
  zoomLabel = button(s("zoom-reset"), { text: zoomText(), shortcut: keys("Ctrl", "0"), className: "zoom-level", onclick: () => setZoom(100) });

  const width = (value) => ({
    label: s(`width-${value}`),
    checked: () => settings.width === value,
    onselect: () => change("width", value),
  });
  const style = (value) => ({
    label: s(`style-${value}`),
    checked: () => settings.style === value,
    onselect: () => change("style", value),
  });
  const reading = menuButton(s("reading"), "reading", [
    width("narrow"),
    width("comfortable"),
    width("wide"),
    "-",
    style("sans"),
    style("serif"),
  ]);
  const noDocument = document.documentElement.classList.contains("no-document");
  // The recent files, as the window last sent them; a pick names the list by its token.
  let recent = { token: 0, items: [] };
  // Files of the same name are told apart by their folders: as many of the last ones as it
  // takes, shown faintly after the name (the full path is the tooltip).
  function folderHints(items) {
    const folders = items.map((item) => item.path.split(/[\\/]/).slice(0, -1).filter(Boolean));
    // Windows compares names without case.
    const key = (name) => name.toLocaleLowerCase();
    const hint = (j, n) => folders[j].slice(-n).join("\\");
    return items.map((item, i) => {
      const twins = items.flatMap((other, j) => (j !== i && key(other.name) === key(item.name) ? [j] : []));
      if (twins.length === 0) return "";
      let n = 1;
      while (n < folders[i].length && twins.some((j) => key(hint(j, n)) === key(hint(i, n)))) n++;
      return hint(i, n);
    });
  }
  // Each row names the list it was built from, so a row still on screen after a newer list
  // has come is a pick from an outdated list, which the window refuses.
  const recentItems = () => {
    const { token, items } = recent;
    const hints = folderHints(items);
    return [
      { heading: s("recent") },
      ...(items.length
        ? items.map((item, i) => ({
            label: item.name,
            hint: hints[i],
            title: item.path,
            onselect: () => send(`open-recent\n${token}\n${i}`),
          }))
        : [{ label: s("recent-none"), disabled: true }]),
      "-",
    ];
  };
  const more = menuButton(s("more"), "more", () => [
    ...recentItems(),
    { label: s("save-pdf"), disabled: noDocument, onselect: () => send("save-pdf") },
    // A window with no document has no file to open elsewhere or show (page::empty_page).
    { label: s("open-editor"), disabled: noDocument, onselect: () => send("open-editor") },
    { label: s("show-folder"), disabled: noDocument, onselect: () => send("show-folder") },
    "-",
    { label: s("settings"), onselect: () => showSettings() },
    { label: s("about"), onselect: () => showAbout() },
  ]);

  const toolbar = el(
    "header",
    { id: "toolbar", role: "toolbar", "aria-label": s("toolbar") },
    el(
      "div",
      { className: "group" },
      button(s("open"), { iconName: "open", shortcut: keys("Ctrl", "O"), onclick: () => send("open") }),
      el("div", { className: "segmented", role: "group", "aria-label": s("mode") }, ...modeButtons),
      unsaved,
    ),
    el(
      "div",
      { className: "group" },
      outlineButton,
      findButton,
      el(
        "div",
        { className: "zoom", role: "group", "aria-label": s("zoom-reset") },
        button(s("zoom-out"), { text: "−", shortcut: keys("Ctrl", "−"), onclick: () => zoomBy(-1) }),
        zoomLabel,
        button(s("zoom-in"), { text: "+", shortcut: keys("Ctrl", "+"), onclick: () => zoomBy(1) }),
      ),
      reading.wrap,
      button(s("print"), { iconName: "print", shortcut: keys("Ctrl", "P"), onclick: () => window.print() }),
      button(s("copy"), { iconName: "copy", shortcut: keys("Ctrl", "Shift", "C"), onclick: () => copyDocument() }),
      more.wrap,
    ),
  );

  // The toolbar's arrow keys move between its buttons (one tab stop each is fine too: every
  // button stays in the tab order, so the keyboard reaches everything either way).
  toolbar.addEventListener("keydown", (event) => {
    if (event.target.closest(".menu")) return;
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    const tools = [...toolbar.querySelectorAll("button.tool")];
    const i = tools.indexOf(document.activeElement);
    if (i < 0) return;
    event.preventDefault();
    tools[(i + (event.key === "ArrowRight" ? 1 : -1) + tools.length) % tools.length].focus();
  });

  // --- The outline ---------------------------------------------------------------------------

  const outlineList = el("div", { className: "outline-list" });
  const splitter = el("div", {
    className: "splitter",
    role: "separator",
    tabindex: "0",
    "aria-orientation": "vertical",
    "aria-label": s("outline-resize"),
    "aria-controls": "outline",
    "aria-valuemin": String(OUTLINE_MIN),
    "aria-valuemax": String(OUTLINE_MAX),
  });
  const outline = el("nav", { id: "outline", "aria-label": s("outline-panel"), hidden: true }, outlineList, splitter);
  let outlineWidth = 240;
  // Set when this window's outline was opened or shut by hand: Automatic leaves it be then.
  let outlineByHand = false;
  let headings = [];
  let outlineLinks = new Map();

  function setOutlineWidth(px) {
    const width = Math.round(Math.min(OUTLINE_MAX, Math.max(OUTLINE_MIN, px)));
    // Set only when it changes, as --zoom is (setZoom). It changes only as the handle moves,
    // and the document is laid out again then anyway.
    if (width !== outlineWidth) root.style.setProperty("--outline-width", `${width}px`);
    outlineWidth = width;
    splitter.setAttribute("aria-valuenow", String(outlineWidth));
  }

  function buildOutline() {
    headings = [...doc.querySelectorAll("h1[id], h2[id], h3[id], h4[id], h5[id], h6[id]")];
    outlineLinks = new Map();
    const top = el("ul");
    const stack = [{ level: 0, list: top }];
    for (const heading of headings) {
      const level = Number(heading.tagName[1]);
      while (stack.length > 1 && stack[stack.length - 1].level >= level) stack.pop();
      let { list } = stack[stack.length - 1];
      const link = el("a", { href: `#${heading.id}`, text: heading.textContent.trim() });
      link.addEventListener("click", (event) => {
        event.preventDefault();
        jumpTo(heading);
      });
      const item = el("li", {}, link);
      list.append(item);
      outlineLinks.set(heading, link);
      const sub = el("ul");
      item.append(sub);
      stack.push({ level, list: sub });
    }
    for (const empty of top.querySelectorAll("ul:empty")) empty.remove();
    outlineList.replaceChildren(top);
    current = null;
    updateCurrentSection();
  }

  function jumpTo(heading) {
    heading.setAttribute("tabindex", "-1");
    // Where it comes to rest: its scroll margin, 16 px under the chrome.
    const line = chromeTop() + 16;
    if (Math.abs(heading.getBoundingClientRect().top - line) >= 1) {
      const jump = ++holding;
      document.addEventListener(
        "scrollend",
        () => jump === holding && holdAt(() => heading.getBoundingClientRect().top, line),
        { once: true },
      );
    }
    heading.scrollIntoView({ behavior: "smooth", block: "start" });
    heading.focus({ preventScroll: true });
  }

  // A block off the screen has the height the browser guesses for it until it's been shown
  // (window.css), so a jump far into the document lands near its target, which then moves as
  // the blocks round it are shown. holdAt keeps it where it was put: each frame it's put back,
  // until it stays (a second at most). The reader's own scrolling, keys or pointer end it, as
  // does the next jump.
  let holding = 0;
  for (const type of ["wheel", "keydown", "pointerdown", "touchstart"]) {
    window.addEventListener(type, () => holding++, { capture: true, passive: true });
  }

  /** Keeps what `top()` measures (a top, from the viewport's) at `line`. */
  function holdAt(top, line) {
    const hold = ++holding;
    let still = 0;
    let frames = 0;
    const step = () => {
      if (hold !== holding || ++frames > 60) return;
      const off = top() - line;
      if (Math.abs(off) < 1) {
        if (++still >= 3) return;
      } else {
        still = 0;
        const before = window.scrollY;
        window.scrollBy({ top: off, behavior: "instant" });
        if (window.scrollY === before) return; // as far as the page goes
      }
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }

  const isLong = () => headings.length > 3 && root.scrollHeight > 2 * window.innerHeight;

  function showOutline(show) {
    if (outline.hidden === !show) return;
    outline.hidden = !show;
    root.classList.toggle("outline-open", show);
    syncOutlineButton();
    scheduleLayout();
  }

  function toggleOutline() {
    if (!outlineInMode()) return;
    outlineByHand = true;
    showOutline(outline.hidden);
  }

  // In Split and Edit the editor's text takes the outline's side (web/editor/editor.css hides
  // #outline), so the Outline button shows as off and can't be used there, its tooltip saying
  // why, and Ctrl+Shift+O does nothing. The mode never touches the outline's own state (the
  // setting, the hand toggle, whether it's open): back in View, it's as it was. The editor says
  // which mode it's in with a `legible-mode` event, its detail the mode.
  let mode = "view";
  const outlineInMode = () => mode === "view";
  const outlineTip = outlineButton.title;
  let outlinePressed = false;
  function syncOutlineButton() {
    const available = outlineInMode();
    const pressed = available && !outline.hidden;
    outlineButton.setAttribute("aria-pressed", String(pressed));
    if (pressed !== outlinePressed) {
      outlinePressed = pressed;
      send(`outline\n${pressed}`);
    }
    if (available) outlineButton.removeAttribute("aria-disabled");
    else outlineButton.setAttribute("aria-disabled", "true");
    outlineButton.title = available ? outlineTip : tipWithNote(s("outline"), s("outline-in-view"));
  }
  document.addEventListener("legible-mode", (event) => {
    mode = ["view", "split", "edit"].includes(event.detail) ? event.detail : "view";
    syncOutlineButton();
  });

  function updateOutlineVisibility() {
    if (outlineByHand) return;
    const mode = settings.outline;
    showOutline(mode === "always" || (mode === "automatic" && isLong()));
  }

  let current = null;
  /** A heading's top, from the viewport's. One in a block that isn't laid out yet (window.css)
      is taken as where that block starts: measuring the heading would lay all of the block
      out, most of a second for a long one. */
  function headingTop(heading) {
    const shown = heading.checkVisibility({ contentVisibilityAuto: true });
    return (shown ? heading : (heading.closest("#document > *") ?? heading)).getBoundingClientRect().top;
  }

  /** In headings in the document's order, whose tops only go down the page: the index of the
      first whose top passes `past` (`list.length` if none does). A binary search, so a document
      with thousands of headings measures a dozen or so, not each one above the screen. */
  function firstWhere(list, past) {
    let low = 0;
    let high = list.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (past(headingTop(list[mid]))) high = mid;
      else low = mid + 1;
    }
    return low;
  }

  function updateCurrentSection() {
    // Below where a heading comes to rest after a jump (its scroll margin is 16 px).
    const line = chromeTop() + 32;
    const found = headings[firstWhere(headings, (top) => top > line) - 1] ?? headings[0] ?? null;
    if (found === current) return;
    outlineLinks.get(current)?.removeAttribute("aria-current");
    current = found;
    const link = outlineLinks.get(current);
    if (link) {
      link.setAttribute("aria-current", "location");
      if (!outline.hidden) link.scrollIntoView({ block: "nearest" });
    }
  }

  splitter.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    splitter.setPointerCapture(event.pointerId);
    const move = (e) => setOutlineWidth(e.clientX);
    const up = () => {
      splitter.removeEventListener("pointermove", move);
      splitter.removeEventListener("pointerup", up);
      scheduleLayout();
    };
    splitter.addEventListener("pointermove", move);
    splitter.addEventListener("pointerup", up);
  });
  splitter.addEventListener("keydown", (event) => {
    const step = event.shiftKey ? 64 : 16;
    if (event.key === "ArrowLeft") setOutlineWidth(outlineWidth - step);
    else if (event.key === "ArrowRight") setOutlineWidth(outlineWidth + step);
    else if (event.key === "Home") setOutlineWidth(OUTLINE_MIN);
    else if (event.key === "End") setOutlineWidth(OUTLINE_MAX);
    else return;
    event.preventDefault();
    scheduleLayout();
  });

  // --- Find ----------------------------------------------------------------------------------

  const findField = el("input", {
    type: "search",
    className: "find-field",
    "aria-label": s("find-field"),
    placeholder: s("find-field"),
    spellcheck: "false",
  });
  const findCount = el("span", { className: "find-count", role: "status", "aria-live": "polite" });
  const findBar = el(
    "div",
    { id: "find", role: "search", "aria-label": s("find"), hidden: true },
    findField,
    findCount,
    button(s("find-previous"), { iconName: "up", shortcut: keys("Shift", "Enter"), onclick: () => step(-1) }),
    button(s("find-next"), { iconName: "down", shortcut: keys("Enter"), onclick: () => step(1) }),
    button(s("close"), { iconName: "close", shortcut: keys("Esc"), onclick: () => closeFind() }),
  );

  const highlights = typeof Highlight === "function" && window.CSS?.highlights;
  let index = null; // { text, nodes: [{ node, start }] }
  let matches = []; // Ranges
  let selected = -1;

  // Find goes by the document's markup, never its computed style: asking each element's style
  // would lay out every block the page hasn't shown yet (window.css), most of a second in a long
  // document. What it leaves out: what's never read, and what document.css hides (a picture's
  // placeholder once the picture shows; a diagram's other theme is SVG).
  const UNSHOWN = "script, style, template, button, svg, rp, [hidden], [aria-hidden='true'], img[src] + .remote-image";
  // The elements the sanitiser allows that lay out as blocks, by the browser's own stylesheet
  // and document.css; the others are inline, except in document.css's flex and grid boxes,
  // whose children are blocks.
  const BLOCKS = new Set(
    "ASIDE BLOCKQUOTE CAPTION COL COLGROUP DD DETAILS DIV DL DT FIGCAPTION FIGURE H1 H2 H3 H4 H5 H6 HR LI OL P PRE SECTION SUMMARY TABLE TBODY TD TFOOT TH THEAD TR UL".split(" "),
  );
  const BOXES = ".alert-title, .remote-image, .missing-image, details.properties tbody";

  /** Is this element shown, as far as find is concerned? A closed <details>' content counts:
      opening it shows the match. */
  function shown(element, cache) {
    if (element === doc) return true;
    let result = cache.get(element);
    if (result === undefined) {
      result = !element.matches(UNSHOWN) && shown(element.parentElement, cache);
      cache.set(element, result);
    }
    return result;
  }

  /** The nearest ancestor that lays out as a block: text in different ones never joins. */
  function blockOf(element) {
    let node = element;
    while (node !== doc && !BLOCKS.has(node.tagName) && !node.parentElement.matches(BOXES)) node = node.parentElement;
    return node;
  }

  /** A text node's text in lower case and plain spaces, one character for one, so offsets map
      straight back. */
  function plain(data) {
    if (/^[\x00-\x7f]*$/.test(data)) return data.toLowerCase().replace(/\s/g, " ");
    let text = "";
    for (const ch of data) {
      const lower = ch.toLowerCase();
      const one = lower.length === ch.length ? lower : ch;
      text += /\s/.test(one) ? " ".repeat(one.length) : one;
    }
    return text;
  }

  function buildIndex() {
    const shownCache = new Map();
    const nodes = [];
    const parts = [];
    let length = 0;
    let lastBlock = null;
    const walker = document.createTreeWalker(doc, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const parent = node.parentElement;
      if (!parent || !node.data || !shown(parent, shownCache)) continue;
      const block = blockOf(parent);
      if (lastBlock && block !== lastBlock) {
        parts.push(BLOCK_BREAK);
        length += 1;
      }
      lastBlock = block;
      nodes.push({ node, start: length });
      parts.push(plain(node.data));
      length += node.data.length;
    }
    index = { text: parts.join(""), nodes };
  }

  /** The text node and offset at `at` in the index. */
  function locate(at, end) {
    const { nodes } = index;
    let lo = 0;
    let hi = nodes.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (nodes[mid].start < at || (!end && nodes[mid].start === at)) lo = mid;
      else hi = mid - 1;
    }
    const { node, start } = nodes[lo];
    return [node, Math.min(at - start, node.data.length)];
  }

  function search({ keep = false } = {}) {
    const query = findField.value.toLowerCase().replace(/\s/g, " ");
    const previous = keep && selected >= 0 ? matches[selected] : null;
    matches = [];
    if (query && !query.includes(BLOCK_BREAK)) {
      if (!index) buildIndex();
      for (let at = index.text.indexOf(query); at >= 0; at = index.text.indexOf(query, at + query.length)) {
        const range = document.createRange();
        range.setStart(...locate(at, false));
        range.setEnd(...locate(at + query.length, true));
        matches.push(range);
      }
    }
    if (previous) {
      selected = matches.findIndex((r) => r.compareBoundaryPoints(Range.START_TO_START, previous) >= 0);
      if (selected < 0) selected = matches.length ? 0 : -1;
    } else {
      selected = matches.length ? 0 : -1;
    }
    paint(!keep);
  }

  function paint(reveal) {
    if (highlights) {
      highlights.set("legible-find", new Highlight(...matches));
      if (selected >= 0) highlights.set("legible-find-current", new Highlight(matches[selected]));
      else highlights.delete("legible-find-current");
    }
    findCount.textContent = !findField.value
      ? ""
      : matches.length
        ? s("find-count")
            .replaceAll("{current}", () => (selected + 1).toLocaleString(root.lang || undefined))
            .replaceAll("{total}", () => matches.length.toLocaleString(root.lang || undefined))
        : s("find-none");
    findField.classList.toggle("no-match", Boolean(findField.value) && !matches.length);
    if (reveal && selected >= 0) revealMatch(matches[selected]);
  }

  /** Opens the <details> round the match, then scrolls it into view below the chrome. */
  function revealMatch(range) {
    let node = range.startContainer.parentElement;
    let opened = false;
    while (node && node !== doc) {
      if (node.tagName === "DETAILS" && !node.open) {
        node.open = true;
        opened = true;
      }
      node = node.parentElement;
    }
    if (opened) {
      buildIndex();
      search({ keep: true });
    }
    const rect = range.getBoundingClientRect();
    const top = chromeTop();
    if (rect.top < top + 8 || rect.bottom > window.innerHeight - 8) {
      const line = top + (window.innerHeight - top) / 3;
      window.scrollBy({ top: rect.top - line, behavior: "instant" });
      holdAt(() => range.getBoundingClientRect().top, line);
    }
  }

  function step(direction) {
    if (findBar.hidden) return openFind();
    if (!matches.length) return;
    selected = (selected + direction + matches.length) % matches.length;
    paint(true);
  }

  function openFind() {
    const wasHidden = findBar.hidden;
    findBar.hidden = false;
    findButton.setAttribute("aria-pressed", "true");
    root.classList.add("find-open");
    const picked = window.getSelection()?.toString().trim();
    if (picked && !picked.includes("\n") && doc.contains(window.getSelection().anchorNode)) findField.value = picked;
    if (wasHidden) {
      index = null;
      search();
    }
    scheduleLayout();
    findField.focus();
    findField.select();
  }

  function closeFind() {
    if (findBar.hidden) return;
    findBar.hidden = true;
    findButton.setAttribute("aria-pressed", "false");
    root.classList.remove("find-open");
    highlights?.delete("legible-find");
    highlights?.delete("legible-find-current");
    matches = [];
    selected = -1;
    index = null;
    scheduleLayout();
    findButton.focus();
  }

  findField.addEventListener("input", () => search());
  findField.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      step(event.shiftKey ? -1 : 1);
    } else if (event.key === "Escape") {
      event.preventDefault();
      closeFind();
    }
  });

  // The document changes after load (code highlighted, diagrams drawn) and may be replaced:
  // while find is open, its index follows.
  let reindexTimer = 0;
  new MutationObserver(() => {
    if (findBar.hidden) return;
    clearTimeout(reindexTimer);
    reindexTimer = setTimeout(() => {
      index = null;
      search({ keep: true });
    }, 150);
  }).observe(doc, { childList: true, subtree: true, characterData: true });

  // --- Dialogs: Settings and About -----------------------------------------------------------

  function dialog(title, ...content) {
    const heading = el("h2", { text: title });
    const node = el(
      "dialog",
      { className: "panel", "aria-label": title },
      el(
        "div",
        { className: "panel-head" },
        heading,
        button(s("close"), { iconName: "close", shortcut: keys("Esc"), onclick: () => node.close() }),
      ),
      ...content,
    );
    document.body.append(node);
    return node;
  }

  // Settings' rows: a setting's name in the first column, its choices in the second. A row of
  // choices is a group (a radiogroup, or a group of checkboxes) named by its row's name.
  function row(name, label, choices, role = "group") {
    const id = `settings-${name}-name`;
    return el(
      "div",
      { className: "setting", role, "aria-labelledby": id },
      el("span", { className: "setting-name", id, text: label }),
      el("div", { className: "choices" }, ...choices),
    );
  }

  function radios(name, label, options) {
    const choices = options.map(([value, text]) => {
      const input = el("input", { type: "radio", name, value });
      input.addEventListener("change", () => input.checked && change(name, value));
      return el("label", {}, input, el("span", { text }));
    });
    return row(name, label, choices, "radiogroup");
  }

  let settingsDialog = null;
  let saveStatus = null;

  function syncSettingsDialog() {
    if (!settingsDialog) return;
    for (const input of settingsDialog.querySelectorAll("input[type=radio]")) {
      input.checked = String(settings[input.name]) === input.value;
    }
    settingsDialog.querySelector("select[name=zoom]").value = String(settings.zoom);
    for (const name of COMFORTS) settingsDialog.querySelector(`input[name=${name}]`).checked = settings[name];
    settingsDialog.querySelector("input[name=status-bar]").checked = settings["status-bar"] === true;
    syncDefaultSection();
  }

  // The comforts, each on by default and turned off here.
  const COMFORTS = ["wheel-zoom", "reopen-windows", "remember-place"];

  function checkbox(name, label) {
    const input = el("input", { type: "checkbox", name, id: `settings-${name}` });
    input.addEventListener("change", () => change(name, input.checked));
    return el("label", {}, input, el("span", { text: label }));
  }

  // Settings' tabs, each a panel of its settings: the arrow keys, Home and End move between
  // them, and Tab goes on into the panel. The one shown last is kept for the window's session
  // (a reload too), so Settings opens where it was left.
  const SETTINGS_TAB = "legible-settings-tab";
  let settingsTabs = null;

  function tabs(panels) {
    settingsTabs = {};
    const list = el("div", { className: "tabs", role: "tablist", "aria-label": s("settings") });
    const body = el("div", { className: "tab-panels" });
    for (const [name, ...content] of panels) {
      const tab = el("button", {
        type: "button",
        className: "tab",
        role: "tab",
        id: `settings-tab-${name}`,
        "aria-controls": `settings-panel-${name}`,
        "data-tab": name,
        text: s(`settings-tab-${name}`),
        onclick: () => selectTab(name),
      });
      const panel = el(
        "div",
        { className: "panel-body", role: "tabpanel", id: `settings-panel-${name}`, "aria-labelledby": tab.id },
        ...content,
      );
      list.append(tab);
      body.append(panel);
      settingsTabs[name] = { tab, panel };
    }
    list.addEventListener("keydown", (event) => {
      const names = Object.keys(settingsTabs);
      const i = names.indexOf(event.target.dataset.tab);
      const next = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: names.length - 1 }[event.key];
      if (i < 0 || next === undefined) return;
      event.preventDefault();
      settingsTabs[selectTab(names[(next + names.length) % names.length])].tab.focus();
    });
    return [list, body];
  }

  /** Shows tab `name` (or the first, if it isn't one), keeps it for next time, and returns it. */
  function selectTab(name) {
    if (!(name in settingsTabs)) name = Object.keys(settingsTabs)[0];
    for (const [key, { tab, panel }] of Object.entries(settingsTabs)) {
      tab.setAttribute("aria-selected", String(key === name));
      tab.tabIndex = key === name ? 0 : -1;
      panel.hidden = key !== name;
    }
    try {
      sessionStorage.setItem(SETTINGS_TAB, name);
    } catch {
      // No storage: Settings opens at its first tab next time.
    }
    return name;
  }

  function lastTab() {
    try {
      return sessionStorage.getItem(SETTINGS_TAB);
    } catch {
      return null;
    }
  }

  function showSettings() {
    if (!settingsDialog) {
      const zoomSelect = el("select", { name: "zoom", id: "settings-zoom", "aria-labelledby": "settings-zoom-name" });
      for (const z of ZOOM_STEPS) zoomSelect.append(el("option", { value: String(z), text: `${z}%` }));
      zoomSelect.addEventListener("change", () => change("zoom", Number(zoomSelect.value)));
      saveStatus = el("p", { className: "save-status", role: "status", "aria-live": "polite" });
      settingsDialog = dialog(
        s("settings"),
        ...tabs([
          [
            "appearance",
            radios("theme", s("settings-theme"), [
              ["system", s("theme-system")],
              ["light", s("theme-light")],
              ["dark", s("theme-dark")],
            ]),
            radios("style", s("settings-style"), [
              ["sans", s("style-sans")],
              ["serif", s("style-serif")],
            ]),
            radios("width", s("settings-width"), [
              ["narrow", s("width-narrow")],
              ["comfortable", s("width-comfortable")],
              ["wide", s("width-wide")],
            ]),
            row("zoom", s("settings-zoom"), [zoomSelect]),
            radios("editor-font", s("settings-editor-font"), [
              ["reading", s("editor-font-reading")],
              ["monospace", s("editor-font-monospace")],
            ]),
          ],
          [
            "reading",
            radios("outline", s("settings-outline"), [
              ["automatic", s("outline-automatic")],
              ["always", s("outline-always")],
              ["never", s("outline-never")],
            ]),
            row("status", s("settings-status"), [checkbox("status-bar", s("settings-status-bar"))]),
            radios("remote-images", s("settings-remote"), [
              ["ask", s("remote-ask")],
              ["never", s("remote-never")],
              ["always", s("remote-always")],
            ]),
            row("comfort", s("settings-comfort"), COMFORTS.map((name) => checkbox(name, s(`settings-${name}`)))),
          ],
          ["general", defaultSection()],
        ]),
        saveStatus,
      );
    }
    syncSettingsDialog();
    send("default-app");
    const tab = selectTab(lastTab());
    settingsDialog.showModal();
    settingsTabs[tab].tab.focus();
  }

  // --- About Legible (Help → About Legible, or the ⋯ menu) -----------------------------------
  // The icon and the name, the tagline, the table (the window's facts: LegibleChrome.about), the
  // licence line (its links open the licence and the notices in the reading view), privacy, the
  // copyright and independence notice; Copy (the window copies its own facts), Release notes
  // (the reading view again), and OK, focused as it opens. Esc closes it, as any dialog.

  let aboutDialog = null;
  let aboutRows = null;
  let aboutPrivacy = null;
  let readingDialog = null;
  let readingTitle = null;
  let readingText = null;
  let readingDoc = null;

  const READINGS = { licence: "about-licence-title", notices: "about-notices-title", changelog: "about-release-notes-title" };

  function linkButton(text, onclick) {
    return el("button", { type: "button", className: "link", text, onclick });
  }

  /** The licence line, its two links where the string puts them. */
  function licenceLine() {
    const links = {
      "{mit}": linkButton(s("about-mit"), () => showReading("licence")),
      "{notices}": linkButton(s("about-third-party"), () => showReading("notices")),
    };
    const line = el("p", { className: "about-licence" });
    for (const part of s("about-licence-line").split(/(\{mit\}|\{notices\})/)) line.append(links[part] ?? part);
    return line;
  }

  function showAbout() {
    if (!aboutDialog) {
      aboutRows = el("tbody");
      aboutPrivacy = el("p", { className: "about-privacy" });
      const ok = el("button", { type: "button", className: "about-button default", text: s("about-ok"), onclick: () => aboutDialog.close() });
      aboutDialog = el(
        "dialog",
        { className: "panel about", "aria-label": s("about") },
        el("img", { className: "about-icon", src: strings["about-icon"], alt: "", width: "56", height: "56" }),
        el("h2", { className: "app-name", text: s("app-name") }),
        el("p", { className: "about-tagline", text: s("about-tagline") }),
        el("table", { className: "about-table" }, aboutRows),
        licenceLine(),
        aboutPrivacy,
        el("p", { className: "about-notice", text: s("about-notice") }),
        el(
          "div",
          { className: "about-buttons" },
          el("button", { type: "button", className: "about-button", text: s("about-copy"), onclick: () => send("about-copy") }),
          el("button", { type: "button", className: "about-button", text: s("about-release-notes"), onclick: () => showReading("changelog") }),
          ok,
        ),
      );
      document.body.append(aboutDialog);
    }
    aboutPrivacy.textContent = s("about-privacy");
    send("about");
    aboutDialog.showModal();
    aboutDialog.querySelector(".default").focus();
  }

  /** The licence, the notices or the release notes, as plain text over About. */
  function showReading(name) {
    if (!readingDialog) {
      readingTitle = el("h2", { id: "reading-title" });
      readingText = el("pre", { className: "reading-text", tabindex: "0" });
      readingDoc = el("div", { className: "reading-doc", tabindex: "0", hidden: true });
      readingDialog = el(
        "dialog",
        { className: "panel reading", "aria-labelledby": "reading-title" },
        el("div", { className: "panel-head" }, readingTitle),
        readingText,
        readingDoc,
        el(
          "div",
          { className: "about-buttons" },
          el("button", { type: "button", className: "about-button default", text: s("about-back"), onclick: () => readingDialog.close() }),
        ),
      );
      document.body.append(readingDialog);
    }
    readingTitle.textContent = s(READINGS[name]);
    readingText.textContent = "";
    readingDoc.replaceChildren();
    readingDialog.dataset.reading = name;
    send(`reading\n${name}`);
    readingDialog.showModal();
    readingDialog.querySelector(".default").focus();
  }

  // --- Live reload ---------------------------------------------------------------------------

  const updatedNote = el("div", { id: "updated", role: "status", hidden: true });
  let updatedTimer = 0;
  function showUpdated() {
    updatedNote.textContent = s("updated");
    updatedNote.hidden = false;
    clearTimeout(updatedTimer);
    updatedTimer = setTimeout(() => (updatedNote.hidden = true), 4000);
  }

  const fileBar = el(
    "div",
    { id: "file-bar", role: "alert", hidden: true },
    el("span", { className: "file-bar-text", text: s("file-gone") }),
    button(s("file-keep"), {
      text: s("file-keep"),
      className: "wide-button",
      onclick: () => {
        fileGone(false);
        send("file-keep");
      },
    }),
    button(s("close"), { text: s("close"), className: "wide-button", onclick: () => send("close") }),
  );

  function fileGone(gone) {
    if (fileBar.hidden === !gone) return;
    fileBar.hidden = !gone;
    root.classList.toggle("file-gone", gone);
    scheduleLayout();
  }

  /** Where the reader is: the heading nearest the top of the page under the chrome (above it
      or below), how far it is from there, and how far down the page as a fraction. */
  function takeAnchor() {
    const line = chromeTop();
    // The outline's list, unless the document was just replaced and it's not rebuilt yet.
    const list = headings[0]?.isConnected
      ? headings
      : [...doc.querySelectorAll("h1[id], h2[id], h3[id], h4[id], h5[id], h6[id]")];
    // The nearest is the last at or above the line or the first below it; of several at the
    // same place (in one block not laid out yet), the first.
    let heading = null;
    let top = 0;
    const below = firstWhere(list, (at) => at > line);
    if (below > 0) {
      top = headingTop(list[below - 1]);
      heading = list[firstWhere(list, (at) => at >= top)];
    }
    if (below < list.length) {
      const at = headingTop(list[below]);
      if (!heading || at - line < line - top) {
        heading = list[below];
        top = at;
      }
    }
    const scrollable = Math.max(1, root.scrollHeight - window.innerHeight);
    return {
      id: heading?.id ?? null,
      offset: heading ? line - top : 0,
      fraction: window.scrollY / scrollable,
    };
  }

  function restoreAnchor(anchor) {
    const heading = anchor.id && document.getElementById(anchor.id);
    if (heading && doc.contains(heading)) {
      const line = chromeTop() - anchor.offset;
      window.scrollTo({ top: heading.getBoundingClientRect().top + window.scrollY - line, behavior: "instant" });
      holdAt(() => heading.getBoundingClientRect().top, line);
    } else {
      const scrollable = Math.max(0, root.scrollHeight - window.innerHeight);
      window.scrollTo({ top: anchor.fraction * scrollable, behavior: "instant" });
    }
  }

  // The newest generation started: an older replacement still preparing is left to finish
  // and ignored, so it can't overwrite a newer document. A page the window reloaded starts
  // at the generation it was made for.
  let newestGeneration = Number(doc.dataset.generation) || 0;

  async function replaceDocument({ generation, html, remote }) {
    if (!(generation > newestGeneration)) return;
    newestGeneration = generation;
    doc.dataset.generation = String(generation);
    const anchor = takeAnchor();
    doc.innerHTML = html;
    restoreAnchor(anchor);
    fileGone(false);
    try {
      await (window.LegibleDoc?.prepare(doc) ?? Promise.resolve());
    } catch (error) {
      console.error(error);
    }
    if (generation !== newestGeneration) return;
    documentChanged();
    restoreAnchor(anchor);
    showUpdated();
    if (remote) remoteImages({ generation, ...remote });
  }

  /** The document's content was replaced (by live reload or the editor's preview): what's
      built from it follows. */
  function documentChanged() {
    buildOutline();
    if (!findBar.hidden) {
      index = null;
      search({ keep: true });
    }
    scheduleLayout();
    layOutWaiting();
  }

  // Blocks off the screen wait to be laid out (window.css), so a long document is drawn at
  // once. With thousands of them, though, the browser's care for each (is it near the screen
  // yet?) costs every frame: in a 2 MB document, a step of the scroll waited 0.2 s. So once it's
  // drawn, a document of LAY_OUT_FROM blocks or more has them laid out in the browser's idle
  // time, from the top, a batch at a time, each block marked data-laid-out (window.css lets it
  // be). Batches of about 0.3 s: smaller ones took four times as long in all, the page as slow
  // to scroll meanwhile; all at once held it for most of a second. A block with a great deal
  // in it stays waiting, as laying it out would hold the page longer than a batch.
  const LAY_OUT_FROM = 2000;
  const LAY_OUT_MOST = 20000; // elements in one block
  const LAY_OUT_BATCH_MS = 300;
  let layOutRun = 0;
  function layOutWaiting() {
    const run = ++layOutRun;
    const blocks = [...doc.children];
    if (blocks.length < LAY_OUT_FROM) return;
    let next = 0;
    let batch = 100;
    const step = () => {
      if (run !== layOutRun) return; // the document was replaced: its own run goes on
      const started = performance.now();
      for (const end = Math.min(blocks.length, next + batch); next < end; next++) {
        const block = blocks[next];
        if (block.getElementsByTagName("*").length <= LAY_OUT_MOST) block.dataset.laidOut = "";
      }
      void doc.offsetHeight; // laid out here, so the batch's time is known
      // The next batch sized for LAY_OUT_BATCH_MS, as this one went (at most twice as many).
      const took = Math.max(1, performance.now() - started);
      batch = Math.max(10, Math.round(batch * Math.min(2, LAY_OUT_BATCH_MS / took)));
      if (next < blocks.length) requestIdleCallback(step, { timeout: 1000 });
    };
    requestIdleCallback(step, { timeout: 1000 });
  }

  // --- Pictures from the internet ------------------------------------------------------------

  // The window decides what may load (Settings, Show once, Always for this file), and the
  // page's CSP and its request filter hold it to that: this page only shows what it's told to,
  // and asks. A consent reloads the page with a CSP that lets those pictures in.
  const picturesText = el("span", { className: "file-bar-text" });
  const picturesAnswer = (answer) => () => send(`remote-images\n${answer}\n${newestGeneration}`);
  const picturesActions = el(
    "span",
    { className: "pictures-actions" },
    button(s("pictures-once"), { text: s("pictures-once"), className: "wide-button", onclick: picturesAnswer("once") }),
    button(s("pictures-always"), {
      text: s("pictures-always"),
      className: "wide-button",
      onclick: picturesAnswer("always"),
    }),
  );
  let picturesClosed = -1;
  const picturesBar = el(
    "div",
    { id: "pictures-bar", role: "region", "aria-label": s("pictures-label"), hidden: true },
    picturesText,
    picturesActions,
    button(s("close"), {
      text: s("close"),
      className: "wide-button",
      onclick: () => {
        picturesClosed = newestGeneration;
        showPicturesBar("", false);
      },
    }),
  );

  // While the editor has unsaved changes the page isn't reloaded (a reload would lose them), so
  // pictures from the internet can't be let in: instead of asking, the bar says to save first,
  // with nothing to answer, as it does for a reload the window holds (`reloadWaits`). Saved, it
  // asks again, or the held reload goes ahead.
  let editorDirty = false;
  let reloadWaiting = false;
  let picturesShown = { text: "", asking: false };

  function showPicturesBar(text, asking) {
    picturesShown = { text, asking };
    paintPicturesBar();
  }

  function paintPicturesBar() {
    const { text: said, asking } = picturesShown;
    const saveFirst = editorDirty && ((asking && Boolean(said)) || reloadWaiting);
    const text = saveFirst ? s("pictures-save-first") : said;
    picturesText.textContent = text;
    picturesActions.hidden = saveFirst || !asking;
    const show = Boolean(text) && picturesClosed !== newestGeneration;
    if (picturesBar.hidden === !show) return;
    picturesBar.hidden = !show;
    scheduleLayout();
  }

  document.addEventListener("legible-dirty", (event) => {
    editorDirty = Boolean(event.detail);
    if (!editorDirty) reloadWaiting = false;
    paintPicturesBar();
  });

  /** The window holds a reload (to let pictures from the internet in) until the edits are
      saved. */
  function reloadWaits(waiting) {
    reloadWaiting = Boolean(waiting);
    paintPicturesBar();
  }

  // The pictures from the internet the window lets show now, by their exact addresses: only
  // these get a `src`, and one no longer among them loses its own. Once the window has said they
  // can't be checked ("failed"), none gets one again on this page, whatever it's told after.
  let approved = new Set();
  let picturesFailed = false;

  function applyApproved() {
    for (const img of doc.querySelectorAll("img[data-remote-src]")) {
      const address = img.getAttribute("data-remote-src");
      if (!picturesFailed && approved.has(address)) {
        if (img.getAttribute("src") !== address) img.src = address;
      } else if (img.hasAttribute("src")) {
        img.removeAttribute("src");
      }
    }
  }

  /** What the window says of this document's pictures from the internet: `show`, the ones that
      may load (as the document has them); `ask`, whether to ask about the others;
      `unavailable`, why none can ("runtime" or "failed"). Only for the newest generation, but
      "failed" takes them away whichever generation it's for. */
  async function remoteImages({ generation, show = [], ask = false, unavailable = "" }) {
    if (unavailable === "failed") {
      picturesFailed = true;
      approved = new Set();
      applyApproved();
    }
    await window.LegibleChrome.ready;
    if (Number(generation) !== newestGeneration) return;
    if (!picturesFailed) approved = new Set(Array.isArray(show) ? show : []);
    applyApproved();
    if (unavailable === "runtime" || unavailable === "failed") showPicturesBar(s(`pictures-off-${unavailable}`), false);
    else showPicturesBar(ask ? s("pictures-ask") : "", true);
  }

  // --- The default app -----------------------------------------------------------------------

  // Windows keeps the default app the user's choice: while Legible is registered for .md (so
  // Default apps lists it) but isn't the default, a bar offers to open Default apps at Legible,
  // until the user takes the offer or says not to ask again (the offer-default-app setting, for
  // every window); Close puts it off until Legible starts again. Settings has the same button, and a portable copy's Register and Remove.
  let defaultState = { offer: false, registered: false, isDefault: false, portable: false };
  const stopOffering = () => {
    if (settings["offer-default-app"] !== false) change("offer-default-app", false);
    updateDefaultApp();
  };
  const makeDefault = () => {
    send("default-app-open");
    stopOffering();
  };
  const defaultBar = el(
    "div",
    { id: "default-bar", role: "region", "aria-label": s("default-heading"), hidden: true },
    el("span", { className: "file-bar-text", text: s("default-offer") }),
    button(s("settings-default-app"), { text: s("settings-default-app"), className: "wide-button", onclick: makeDefault }),
    button(s("default-dismiss"), { text: s("default-dismiss"), className: "wide-button", onclick: stopOffering }),
    button(s("close"), {
      text: s("close"),
      className: "wide-button",
      onclick: () => {
        defaultState = { ...defaultState, offer: false };
        send("default-app-close");
        updateDefaultApp();
      },
    }),
  );

  let defaultParts = null;
  function defaultSection() {
    defaultParts = {
      status: el("p", { className: "default-status" }),
      make: button(s("settings-default-app"), { text: s("settings-default-app"), className: "wide-button", onclick: makeDefault }),
      register: button(s("default-register"), {
        text: s("default-register"),
        className: "wide-button",
        onclick: () => send("default-app-register"),
      }),
      unregister: button(s("default-unregister"), {
        text: s("default-unregister"),
        className: "wide-button",
        onclick: () => send("default-app-unregister"),
      }),
    };
    const { status, make, register, unregister } = defaultParts;
    const section = row("default", s("default-heading"), [status, make, register, unregister]);
    section.classList.add("default-app");
    return section;
  }

  function syncDefaultSection() {
    if (!defaultParts) return;
    const { registered, isDefault, portable } = defaultState;
    const { status, make, register, unregister } = defaultParts;
    status.textContent = isDefault ? s("default-is") : portable && !registered ? s("default-portable") : "";
    status.hidden = !status.textContent;
    // The portable note is a hint, muted; that Legible is the default is the answer itself.
    status.classList.toggle("hint", !isDefault);
    make.hidden = !registered || isDefault;
    register.hidden = registered;
    unregister.hidden = !(portable && registered);
  }

  function updateDefaultApp() {
    const show = Boolean(defaultState.offer) && settings["offer-default-app"] !== false;
    if (defaultBar.hidden === show) {
      defaultBar.hidden = !show;
      scheduleLayout();
    }
    syncDefaultSection();
  }

  /** What the window says of Legible and .md files: `registered` (Default apps lists it),
      `isDefault`, `portable`, and `offer` (registered, not the default). */
  function defaultApp(state) {
    defaultState = {
      offer: Boolean(state?.offer),
      registered: Boolean(state?.registered),
      isDefault: Boolean(state?.isDefault),
      portable: Boolean(state?.portable),
    };
    updateDefaultApp();
  }

  // Back from Default apps (or anywhere else): the window says again what's so now.
  window.addEventListener("focus", () => send("default-app"));

  /** The editor's preview (or its text as now on disk) shows render `generation` now: the
      page's render is that one, and its pictures are as `remote` says (or as approved till now).
      */
  function adopt({ generation, remote } = {}) {
    if (Number(generation) > newestGeneration) {
      newestGeneration = Number(generation);
      doc.dataset.generation = String(newestGeneration);
    }
    if (remote) remoteImages(remote);
    else applyApproved();
  }

  // A reload (to let consented pictures in, or to take them away) keeps the reader's place,
  // the zoom and an outline opened or shut by hand, as live reload does.
  const PLACE = "legible-place";
  window.addEventListener("pagehide", () => {
    try {
      const place = { generation: newestGeneration, anchor: takeAnchor(), zoom, outlineByHand, outlineOpen: !outline.hidden };
      sessionStorage.setItem(PLACE, JSON.stringify(place));
    } catch {
      // No storage: the page starts at the top, as a new window does.
    }
  });

  function restorePlace() {
    let place = null;
    try {
      place = JSON.parse(sessionStorage.getItem(PLACE) ?? "null");
      sessionStorage.removeItem(PLACE);
    } catch {
      return;
    }
    // Only a newer render of this window's document (the window serves it again as one): a
    // first page is generation 0, and starts at the top.
    if (!place || typeof place !== "object" || !(newestGeneration > Number(place.generation))) return;
    if (ZOOM_STEPS.includes(place.zoom)) setZoom(place.zoom);
    if (place.outlineByHand === true) {
      outlineByHand = true;
      showOutline(place.outlineOpen === true);
    }
    if (place.anchor && typeof place.anchor === "object") restoreAnchor(place.anchor);
  }

  // --- Getting it out: Copy, Print and Save as PDF ------------------------------------------

  // A toast, for a moment: what was done ("Copied, ready to paste…", "Saved…"), and what can be
  // done next with it.
  const toastText = el("span");
  const toastActions = el("span", { className: "toast-actions" });
  const toast = el("div", { id: "toast", role: "status", hidden: true }, toastText, toastActions);
  let toastTimer = 0;
  function showToast(text, actions = []) {
    toastText.textContent = text;
    toastActions.replaceChildren(
      ...actions.map(([label, onclick]) => button(label, { text: label, className: "wide-button", onclick })),
    );
    toast.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast.hidden = true), actions.length ? 10000 : 4000);
  }

  // The selection, if it's a real one in the document (not in a control, a dialog or the find
  // bar): its ranges, each inside #document at both ends.
  function documentSelection() {
    const active = document.activeElement;
    if (active?.closest?.("input, textarea, select, [contenteditable], dialog, #find, #toolbar")) return null;
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed || selection.rangeCount === 0) return null;
    const ranges = [];
    for (let i = 0; i < selection.rangeCount; i++) {
      const range = selection.getRangeAt(i);
      if (!doc.contains(range.startContainer) || !doc.contains(range.endContainer)) return null;
      ranges.push(range);
    }
    return ranges;
  }

  // A diagram's light drawing (it's in the page in both themes; the light one is what pastes and
  // prints), with Mermaid's own rules for it and its font, ready to draw as an image: taken at
  // once, so nothing that happens to the page meanwhile changes what's copied.
  function lightDrawing(diagram) {
    const svg = diagram.querySelector(":scope > .figure.light svg");
    if (!svg) return null;
    const box = svg.viewBox.baseVal;
    const width = box?.width || svg.width.baseVal.value;
    const height = box?.height || svg.height.baseVal.value;
    if (!(width > 0 && height > 0)) return null;
    const clone = svg.cloneNode(true);
    clone.setAttribute("width", String(width));
    clone.setAttribute("height", String(height));
    const rules = [];
    for (const sheet of document.adoptedStyleSheets) {
      for (const rule of sheet.cssRules) if (svg.id && rule.cssText.includes(`#${svg.id}`)) rules.push(rule.cssText);
    }
    const style = document.createElementNS(SVG, "style");
    style.textContent = rules.join(" ");
    clone.prepend(style);
    const look = getComputedStyle(svg);
    for (const property of ["font-family", "font-size", "line-height", "color"]) {
      clone.style.setProperty(property, look.getPropertyValue(property));
    }
    return { text: new XMLSerializer().serializeToString(clone), width, height };
  }

  /** A drawing as a PNG `data:` URI, at twice its size, on white. */
  async function drawingPng({ text, width, height }) {
    const image = new Image();
    await new Promise((loaded, failed) => {
      image.onload = loaded;
      image.onerror = () => failed(new Error("the diagram didn't draw"));
      image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(text)}`;
    });
    const canvas = document.createElement("canvas");
    canvas.width = Math.ceil(width * 2);
    canvas.height = Math.ceil(height * 2);
    const context = canvas.getContext("2d");
    context.fillStyle = "#ffffff";
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    return canvas.toDataURL("image/png");
  }

  // Each diagram in `copy` (a copy of `originals`, in the same order) becomes the image the
  // clipboard's builder takes: its light drawing as a PNG, its source kept for when it can't.
  async function diagramsAsImages(copy, originals) {
    const drawings = originals.map(lightDrawing);
    const copies = [...copy.querySelectorAll(".diagram")];
    await Promise.all(
      copies.map(async (diagram, i) => {
        const drawing = drawings[i];
        diagram.replaceChildren();
        if (!drawing) return;
        try {
          const png = await drawingPng(drawing);
          diagram.append(el("img", { src: png, alt: s("copy-diagram") }));
        } catch (error) {
          console.warn(error);
        }
      }),
    );
  }

  // One copy waits for the window's answer at a time; a newer one takes its place, and the
  // older's answer is let go. A copy is named by its number and the render it was made from,
  // so an answer meant for a page served before this one is never taken for this page's.
  let copyNext = 0;
  let copyWaiting = null;

  async function sendCopy(container, originals) {
    const id = ++copyNext;
    const generation = newestGeneration;
    const ticket = { id, generation };
    copyWaiting = ticket;
    await diagramsAsImages(container, originals);
    if (copyWaiting !== ticket) return;
    send(`copy\n${id}\n${generation}\n${container.innerHTML}`);
  }

  function copyDocument() {
    if (noDocument) return;
    const container = el("div");
    container.append(...[...doc.childNodes].map((node) => node.cloneNode(true)));
    sendCopy(container, [...doc.querySelectorAll(".diagram")]);
  }

  function copySelection() {
    const ranges = documentSelection();
    if (!ranges) return;
    const container = el("div");
    const originals = [];
    for (const range of ranges) {
      const touched = [...doc.querySelectorAll(".diagram")].filter((d) => range.intersectsNode(d));
      const inside = range.commonAncestorContainer;
      const within = (inside.nodeType === 1 ? inside : inside.parentElement)?.closest(".diagram");
      if (within) {
        // A selection inside one diagram (its labels, say) copies that diagram.
        container.append(within.cloneNode(true));
        originals.push(within);
        continue;
      }
      const part = range.cloneContents();
      // A diagram the selection touches comes whole, whichever part of it was selected.
      const pieces = [...part.querySelectorAll(".diagram")];
      pieces.forEach((piece, i) => {
        if (touched[i]) piece.replaceWith(touched[i].cloneNode(true));
      });
      originals.push(...touched.slice(0, pieces.length));
      container.append(part);
    }
    sendCopy(container, originals);
  }

  /** The window's answer to copy `id` of render `generation`: copied, as text only, failed (with
      why), superseded. Only the copy this page is waiting for counts. */
  function copied({ id, generation, outcome, text }) {
    if (!copyWaiting || Number(id) !== copyWaiting.id || Number(generation) !== copyWaiting.generation) return;
    copyWaiting = null;
    if (outcome === "copied") showToast(s("copied"));
    else if (outcome === "text-only") showToast(s("copied-text-only"));
    else if (outcome === "failed") showSaveFailure(String(text ?? ""));
  }

  /** A string as CSS writes one: in quotes, its quotes and backslashes escaped. */
  function cssString(text) {
    return `"${String(text).replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/[\r\n]+/g, "\\a ")}"`;
  }

  // Printing (and Save as PDF): the document's title at the top of each page, and its number at
  // the bottom ("Page 2 of 5", in the reader's language: the translation's own order kept).
  const printSheet = new CSSStyleSheet();
  document.adoptedStyleSheets = [...document.adoptedStyleSheets, printSheet];
  function updatePrintSheet() {
    const footer = s("print-page")
      .split(/(\{page\}|\{pages\})/)
      .filter((part) => part !== "")
      .map((part) => (part === "{page}" ? "counter(page)" : part === "{pages}" ? "counter(pages)" : cssString(part)))
      .join(" ");
    const box = 'font: 9pt "Segoe UI", system-ui, sans-serif; color: #5d5d5d;';
    printSheet.replaceSync(
      `@page { @top-center { content: ${cssString(document.title)}; ${box} } ` +
        `@bottom-center { content: ${footer || '""'}; ${box} } }`,
    );
  }
  updatePrintSheet();
  new MutationObserver(updatePrintSheet).observe(document.querySelector("title") ?? document.head, {
    childList: true,
    characterData: true,
    subtree: true,
  });

  /** A PDF was saved as `name`: say so, with Open and Show in folder (the window knows where). */
  function pdfSaved({ name }) {
    showToast(s("saved").replace("{name}", String(name)), [
      [s("open-file"), () => send("pdf-open")],
      [s("show-folder"), () => send("pdf-show")],
    ]);
  }

  // --- The save-failure notice ---------------------------------------------------------------

  const noticeText = el("span");
  const notice = el(
    "div",
    { id: "notice", role: "alert", hidden: true },
    noticeText,
    button(s("close"), { iconName: "close", onclick: () => showSaveFailure("") }),
  );

  function showSaveFailure(text) {
    if (saveStatus) saveStatus.textContent = text;
    noticeText.textContent = text;
    notice.hidden = !text;
  }

  // --- Layout --------------------------------------------------------------------------------

  const findHeight = () => (findBar.hidden ? 0 : findBar.offsetHeight);
  const fileHeight = () => (fileBar.hidden ? 0 : fileBar.offsetHeight);
  const picturesHeight = () => (picturesBar.hidden ? 0 : picturesBar.offsetHeight);
  // The editor's conflict bar (web/editor/editor.js) goes under the others.
  const conflictHeight = () => {
    const bar = document.getElementById("conflict-bar");
    return bar && !bar.hidden ? bar.offsetHeight : 0;
  };
  const barsTop = () =>
    TOOLBAR_HEIGHT + findHeight() + fileHeight() + picturesHeight() + (defaultBar.hidden ? 0 : defaultBar.offsetHeight);
  const chromeTop = () => barsTop() + conflictHeight();

  // What moves with the bars (their heights, where the document starts under them, the status
  // bar's height) is set on the elements that follow them, never as custom properties on
  // <html>: one there is inherited by every element of the document, so each change restyled
  // all of it (most of a second in a 2 MB document), and the same goes for any element with
  // thousands inside it. So the page, the document and the outline (its list can be as long as
  // the document's headings) get plain values: the body's padding, the page's scroll padding
  // (where a heading comes to rest), the document's top and bottom in Split, the outline's; the
  // chrome's other elements (the children of <body>) get the custom properties window.css and
  // editor.css read. A value set again as it was costs nothing.
  function writeLayout({ chromeTop, conflictTop, findHeight, fileHeight, picturesHeight, statusHeight }) {
    root.style.scrollPaddingTop = `${chromeTop}px`;
    document.body.style.paddingTop = `${chromeTop}px`;
    document.body.style.paddingBottom = `${statusHeight}px`;
    for (const element of [doc, outline]) {
      element.style.top = `${chromeTop}px`;
      element.style.bottom = `${statusHeight}px`;
    }
    const properties = {
      "--chrome-top": chromeTop,
      "--conflict-top": conflictTop,
      "--find-height": findHeight,
      "--file-height": fileHeight,
      "--pictures-height": picturesHeight,
      "--status-height": statusHeight,
    };
    for (const element of document.body.children) {
      if (element === doc || element === outline) continue;
      for (const [name, px] of Object.entries(properties)) element.style.setProperty(name, `${px}px`);
    }
  }

  let layoutFrame = 0;
  function scheduleLayout() {
    if (layoutFrame) return;
    layoutFrame = requestAnimationFrame(() => {
      layoutFrame = 0;
      // Everything measured, then written: a write between two measurements would lay the page
      // out again for the second.
      const conflictTop = barsTop();
      writeLayout({
        chromeTop: conflictTop + conflictHeight(),
        conflictTop,
        findHeight: findHeight(),
        fileHeight: fileHeight(),
        picturesHeight: picturesHeight(),
        statusHeight: statusBar.hidden ? 0 : statusBar.offsetHeight,
      });
      fitPath();
      updateOutlineVisibility();
      updateCurrentSection();
    });
  }

  // --- The status bar (View → Status bar, or Settings; off by default) --------------------------

  // Left, the file's path, shortened in the middle to fit (its folders give way first, never
  // its name), the whole of it in the tooltip; a click copies it (the window copies its own
  // file's path, never this text). After it, why the file opened for reading only, if it came
  // from an attachment, a download or the temp folder. Right, in Split and Edit only: the text's
  // words (editor.js counts them as it changes), and the file's encoding and line endings.
  const statusPath = el("button", { type: "button", className: "status-path", hidden: true, onclick: () => send("copy-path") });
  const statusNote = el("span", { className: "status-note", hidden: true });
  const statusWords = el("span", { className: "status-words" });
  const statusEncoding = el("span", { className: "status-encoding" });
  const statusEndings = el("span", { className: "status-endings" });
  const statusEdit = el("div", { className: "status-edit", hidden: true }, statusWords, statusEncoding, statusEndings);
  const statusBar = el(
    "footer",
    { id: "status-bar", "aria-label": s("status-label"), hidden: true },
    el("div", { className: "status-file" }, statusPath, statusNote),
    statusEdit,
  );
  let statusPathFull = "";
  let statusMode = "view";

  function showStatusBar(show) {
    if (statusBar.hidden === !show) return;
    statusBar.hidden = !show;
    scheduleLayout();
  }

  function showStatus({ path, note, encoding, endings } = {}) {
    statusPathFull = typeof path === "string" ? path : "";
    statusPath.hidden = !statusPathFull;
    // A function for the path, so a "$&" in it is the path's own.
    statusPath.title = statusPathFull ? s("status-path-tooltip").replaceAll("{path}", () => statusPathFull) : "";
    statusPath.setAttribute("aria-label", s("status-path-label").replaceAll("{path}", () => statusPathFull));
    statusNote.textContent = note ?? "";
    statusNote.hidden = !note;
    statusEncoding.textContent = encoding ?? "";
    statusEndings.textContent = endings ?? "";
    syncStatusEdit();
    fitPath();
  }

  // The right side came, went or changed: the path has another width to fit in.
  function syncStatusEdit() {
    statusEdit.hidden = statusMode === "view" || !statusEncoding.textContent;
    scheduleLayout();
  }

  /** The path in the room it has: whole if it fits, else its drive (or share) and "…" for as
      many of the folders after it as it takes, the name always whole. */
  function fitPath() {
    if (statusBar.hidden || !statusPathFull) return;
    const parts = statusPathFull.split("\\");
    const share = statusPathFull.startsWith("\\\\");
    const head = parts.slice(0, share ? 4 : 1);
    const tail = parts.slice(head.length);
    statusPath.textContent = statusPathFull;
    while (statusPath.scrollWidth > statusPath.clientWidth && tail.length > 1) {
      tail.shift();
      statusPath.textContent = [...head, "…", ...tail].join("\\");
    }
  }

  document.addEventListener("legible-mode", (event) => {
    statusMode = event.detail;
    syncStatusEdit();
  });
  document.addEventListener("legible-words", (event) => {
    const count = Number(event.detail);
    // The form for the count in the strings' own language (window.ftl's chrome-status-words):
    // a variant for exactly this number first, as Fluent picks, else its plural category's.
    const { lang, forms, exact } = strings["status-words"];
    const form = exact[String(count)] ?? forms[new Intl.PluralRules(lang || undefined).select(count)] ?? forms.other;
    statusWords.textContent = form.replaceAll("{count}", () => count.toLocaleString(root.lang || undefined));
    scheduleLayout();
  });

  // --- Keys ----------------------------------------------------------------------------------

  document.addEventListener(
    "keydown",
    (event) => {
      const ctrl = event.ctrlKey && !event.altKey && !event.metaKey;
      const key = event.key;
      // In the editor, find (and replace) and Ctrl+0 (plain text, not a heading) are its own.
      const inEditor = event.composedPath().some((node) => node.id === "editor");
      if (inEditor && ctrl && /^[fFhH0]$/.test(key)) return;
      let handled = true;
      if (ctrl && !event.shiftKey && (key === "f" || key === "F")) openFind();
      else if (key === "F3") step(event.shiftKey ? -1 : 1);
      else if (ctrl && event.shiftKey && (key === "o" || key === "O")) toggleOutline();
      else if (ctrl && !event.shiftKey && (key === "o" || key === "O")) send("open");
      else if (ctrl && (key === "+" || key === "=" || event.code === "NumpadAdd")) zoomBy(1);
      else if (ctrl && (key === "-" || key === "_" || event.code === "NumpadSubtract")) zoomBy(-1);
      else if (ctrl && (key === "0" || event.code === "Numpad0")) setZoom(100);
      else if (ctrl && !event.shiftKey && (key === "p" || key === "P")) window.print();
      else if (ctrl && !event.shiftKey && (key === "w" || key === "W")) send("close");
      else if (ctrl && event.shiftKey && (key === "c" || key === "C")) copyDocument();
      // Ctrl+C copies the document's way only for a selection in the document; anywhere else
      // (the find field, Settings, a dialog) it's the ordinary copy.
      else if (ctrl && !event.shiftKey && (key === "c" || key === "C") && documentSelection()) copySelection();
      else if (key === "Escape" && !findBar.hidden && !document.querySelector("dialog[open]")) closeFind();
      else handled = false;
      if (handled) {
        event.preventDefault();
        event.stopPropagation();
      }
    },
    true,
  );

  // The window's menu bar (legible_win::menu): each item does what its button or shortcut here
  // does. Undo, Redo and the clipboard's come as their own keys instead, so they're the keys'.
  const COMMANDS = {
    print: () => window.print(),
    // As Ctrl+F: the editor's find where the text is (in Edit, or Split with it focused).
    find: () => {
      const mode = window.LegibleEditor?.mode?.() ?? "view";
      const inEditor = document.activeElement?.id === "editor";
      if (mode === "edit" || (mode === "split" && inEditor)) window.LegibleEditor.find();
      else openFind();
    },
    replace: () => window.LegibleEditor?.replace(),
    "copy-document": () => copyDocument(),
    "mode-view": () => window.LegibleEditor?.setMode("view"),
    "mode-split": () => window.LegibleEditor?.setMode("split"),
    "mode-edit": () => window.LegibleEditor?.setMode("edit"),
    outline: () => toggleOutline(),
    "status-bar": () => change("status-bar", settings["status-bar"] !== true),
    "zoom-in": () => zoomBy(1),
    "zoom-out": () => zoomBy(-1),
    "zoom-reset": () => setZoom(100),
    "width-narrow": () => change("width", "narrow"),
    "width-comfortable": () => change("width", "comfortable"),
    "width-wide": () => change("width", "wide"),
    "theme-system": () => change("theme", "system"),
    "theme-light": () => change("theme", "light"),
    "theme-dark": () => change("theme", "dark"),
    settings: () => showSettings(),
    about: () => showAbout(),
  };

  // Ctrl + mouse wheel zooms in the toolbar's steps (Settings' "Zoom with Ctrl + mouse wheel"):
  // a step a notch, and a touchpad's pinch (which WebView2 sends as Ctrl + wheel, in small
  // movements) a step for each notch's worth. Turned off, it does nothing: WebView2's own zoom
  // is off either way, so there's only ever this one.
  const WHEEL_NOTCH = 100;
  let wheelTravel = 0;
  window.addEventListener(
    "wheel",
    (event) => {
      if (!event.ctrlKey) return;
      event.preventDefault();
      if (!settings["wheel-zoom"]) return;
      const delta = event.deltaMode === 1 ? event.deltaY * 33 : event.deltaMode === 2 ? event.deltaY * 800 : event.deltaY;
      if (Math.abs(delta) >= WHEEL_NOTCH / 2) {
        wheelTravel = 0;
        zoomBy(delta < 0 ? 1 : -1);
        return;
      }
      if (Math.sign(delta) !== Math.sign(wheelTravel)) wheelTravel = 0;
      wheelTravel += delta;
      if (Math.abs(wheelTravel) >= WHEEL_NOTCH) {
        zoomBy(wheelTravel < 0 ? 1 : -1);
        wheelTravel = 0;
      }
    },
    { passive: false },
  );

  // Where the reader is, for the window to keep for the file (Settings' "Remember where I was in
  // each file"): the anchor live reload keeps, in View, as each scroll lands (the browser sends
  // one a frame at most), so a window closed straight after scrolling has the newest. The same
  // place again isn't sent.
  // The window also asks for it as the window closes (placeNow), so a close before the scroll's
  // event has come still keeps the newest.
  function placeNow() {
    const editing = root.classList.contains("mode-split") || root.classList.contains("mode-edit");
    if (noDocument || editing || !settings["remember-place"]) return null;
    const { id, offset, fraction } = takeAnchor();
    return `${fraction}\n${offset}\n${id ?? ""}`;
  }

  let placeSent = "";
  function sendPlace() {
    const place = placeNow();
    if (place === null || place === placeSent) return;
    placeSent = place;
    send(`place\n${place}`);
  }

  window.addEventListener(
    "scroll",
    () => {
      requestAnimationFrame(updateCurrentSection);
      sendPlace();
    },
    { passive: true },
  );
  window.addEventListener("resize", scheduleLayout);

  // --- From the window -----------------------------------------------------------------------

  window.LegibleChrome = {
    /** The saved settings changed (in this window or another): take them up. A window's zoom
        is its own: a new default zoom is for new windows. */
    apply(next) {
      if (next.outline !== settings.outline) outlineByHand = false;
      settings = next;
      applyView();
      shareSettings();
      syncSettingsDialog();
      showSaveFailure("");
      updateOutlineVisibility();
      updateDefaultApp();
    },
    /** A setting changed here couldn't be saved: said in Settings, and in a notice over the
        document, so it's seen wherever the change was made (the toolbar menu too). */
    saveFailed(text) {
      showSaveFailure(text);
    },
    /** Something the window says (as when what was dropped isn't Markdown). */
    notice(text) {
      showSaveFailure(text);
    },
    /** The recent files changed: the menu shows these from its next opening. */
    setRecent(list) {
      recent = { token: Number(list.token), items: Array.isArray(list.items) ? list.items : [] };
    },
    /** About's table: the window's facts, as rows of [label, value]. */
    about({ rows } = {}) {
      if (!aboutRows || !Array.isArray(rows)) return;
      aboutRows.replaceChildren(
        ...rows.map(([label, value]) => el("tr", {}, el("th", { scope: "row", text: String(label) }), el("td", { text: String(value) }))),
      );
    },
    /** About's Copy is on the clipboard. */
    aboutCopied() {
      showToast(s("about-copied"));
    },
    /** A text the reading view asked for, by name: shown only if it's still the one asked for.
        The release notes come rendered too (`html`, core's render of Markdown, as a document
        is): shown as a document; the others as plain text. */
    reading(name, text, html) {
      if (readingDialog?.dataset.reading !== name) return;
      const rendered = typeof html === "string";
      readingText.hidden = rendered;
      readingDoc.hidden = !rendered;
      if (rendered) readingDoc.innerHTML = html;
      else readingText.textContent = String(text);
    },
    replaceDocument,
    fileGone,
    /** Where the reader was in this file last time, once the document is ready: by its
        heading, as live reload keeps the place. */
    goTo(anchor) {
      if (!anchor || typeof anchor !== "object") return;
      window.LegibleChrome.ready.then(() => restoreAnchor(anchor));
    },
    /** Where the reader is this moment, as the place message has it (null in Split or Edit, or
        with the setting off): the window asks as it closes. */
    place: placeNow,
    copied,
    pdfSaved,
    remoteImages,
    defaultApp,
    adopt,
    reloadWaits,
    layout: scheduleLayout,
    /** Where the page's view starts under the chrome, from the viewport's top. */
    top: () => chromeTop(),
    /** Keeps what `top()` measures at `line` while the blocks round it are laid out (holdAt):
        the editor's place, back in View. */
    hold: (top, line) => holdAt(top, line),
    status: showStatus,
    /** The status bar's path is on the clipboard. */
    pathCopied() {
      showToast(s("status-path-copied"));
    },
    /** An item of the window's menu bar. */
    command(name) {
      if (Object.hasOwn(COMMANDS, name)) COMMANDS[name]();
    },
    /** The editor's preview shows the file as it now is on disk. */
    updated() {
      fileGone(false);
      showUpdated();
    },
    /** The answer to held change `id`: take it from disk (the edits are discarded)... */
    reloadFromDisk(id) {
      send(`reload\n${Number(id)}`);
    },
    /** ...or keep the edits. */
    keepMine(id) {
      send(`keep\n${Number(id)}`);
    },
    ready: null,
  };

  // --- Start ---------------------------------------------------------------------------------

  document.body.prepend(toolbar, findBar, fileBar, picturesBar, defaultBar, outline, updatedNote);
  document.body.append(notice, toast, statusBar);
  send("recent");
  send("status");
  send("remote-images");
  send("file-status");
  send("default-app");
  applyView();
  setOutlineWidth(outlineWidth);
  setZoom(zoom);
  buildOutline();
  updateOutlineVisibility();
  window.LegibleChrome.ready = (window.LegibleDoc?.ready ?? Promise.resolve()).then(() => {
    buildOutline();
    restorePlace();
    scheduleLayout();
    return new Promise((resolve) =>
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          layOutWaiting();
          resolve();
        }),
      ),
    );
  });
})();
