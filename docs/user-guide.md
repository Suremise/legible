# Legible user guide

Legible shows Markdown files (`.md`, `.markdown` and `.mdown`) as clear, readable documents, and
lets you copy them into Outlook, Word or OneNote, print them, save them as PDF and, when they need
a change, edit them. This guide covers every feature, then what to do when something doesn't work
as you expect.

- [Opening a document](#opening-a-document)
- [Making Legible the default](#making-legible-the-default)
- [Reading](#reading)
- [Pictures from the internet](#pictures-from-the-internet)
- [Copy, print and PDF](#copy-print-and-pdf)
- [Editing: View, Split and Edit](#editing-view-split-and-edit)
- [Saving, and when the file changes](#saving-and-when-the-file-changes)
- [Settings](#settings)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Privacy](#privacy)
- [Where Legible keeps things](#where-legible-keeps-things)
- [Troubleshooting](#troubleshooting)

## Opening a document

- **Double-click** a Markdown file in File Explorer, or in an email (an attachment).
- **File > Open** (Ctrl+O), or the toolbar's **Open**.
- **Drag** one or more Markdown files onto a Legible window.
- **A link** in one Markdown file to another opens it in Legible.
- **Recently opened files:** **File > Open recent**, the **⋯** menu's Recent files, or right-click
  Legible's icon on the taskbar. They list the ten files you opened last; hover over one to see
  where it is.

**File > Open**, **Open recent** and dragging a file onto a window show the file in that window, in
place of the one it had. If that file has unsaved changes, Legible asks first, as it does when you
close the window: **Cancel** leaves everything as it was. Drag several files at once and the first
takes the window's place, and each of the others opens in a window of its own.

Starting Legible, or opening a Markdown file from Explorer or the taskbar, opens a new window. A
file that's already open in another window comes to the front instead. Legible remembers where you
were in each file, and opens it there next time. A new window opens at the size and place of the
last one you closed.

**File > Close** (Ctrl+W) closes the window; **File > Exit** closes every Legible window, asking
first about any unsaved changes.

## Making Legible the default

The installer adds Legible to the apps that can open Markdown files, but Windows doesn't let an
app make itself the default: you choose. While Legible isn't what opens `.md` files, a bar at the
top of its window offers **Make Legible the default for Markdown files**:

1. Click it. Windows' **Default apps** opens at Legible's entry.
2. Choose Legible for `.md` (and for `.markdown` and `.mdown`, if you have them).

From then on, double-clicking a Markdown file opens Legible, and the bar doesn't come back.
**Don't ask again** stops it asking; the bar's close button puts it off until Legible starts
again. The same button is always in **Settings > General**.

A **portable copy** changes nothing on the PC until you ask: **Settings > General > Register
Legible for .md files on this PC** adds it to the apps for Markdown files, and **Remove Legible
from this PC's apps for .md files** takes it out again.

## Reading

The toolbar at the top has, from the left: **Open**; **View, Split and Edit**; **Outline**;
**Find**; the **zoom**; **Reading width and style**; **Print**; **Copy**; and **⋯** (More), with
Save as PDF, Open in editor, Show in folder, Recent files, Settings and About. Every button says
what it does, and its shortcut, when you hover over it. The menu bar above it (File, Edit, View,
Tools and Help) has the same commands, and works from the keyboard with Alt.

### What a document shows

- **Headings, lists, links, quotes, footnotes** and the rest of Markdown, laid out like a
  document. Task lists show their ticks. Footnotes are numbered and collected at the end, each
  with a link back; a footnote defined twice uses its first definition, as GitHub does.
- **Properties:** the block of details some notes start with (between `---` lines: a title, an
  author, a date) shows as a table at the top, folded when there are more than six. Click its
  heading to fold or unfold it.
- **Tables** fit the page: a wide one scrolls sideways in its own frame, and a long one keeps its
  header in view. Columns of numbers line up on the right.
- **Alerts** (`> [!NOTE]`, `> [!TIP]`, `> [!IMPORTANT]`, `> [!WARNING]`, `> [!CAUTION]`) show as
  coloured boxes with their title.
- **Code** is coloured by its language. Hover over a block for its **Copy** button.
- **Diagrams** written in Mermaid (flowcharts, sequence diagrams, Gantt charts, pie charts, mind
  maps and the rest) are drawn, in the theme's colours. **Show source** shows the text a diagram
  was drawn from. A diagram Legible can't draw shows its text, with a short note.
- **Pictures** in the document's folder (or a folder inside it) show. Pictures from the internet
  wait until you allow them ([below](#pictures-from-the-internet)). A picture that can't be shown
  shows its description instead.
- **A document's own scripts never run,** and anything in its HTML that could run one is removed.

### Outline

The **Outline** (Ctrl+Shift+O) lists the document's headings down the left. Click one, or move to
it with the arrow keys and press Enter, to go there; the section you're reading is marked as you
scroll. Drag its edge to make it wider or narrower. It shows by itself for long documents; Settings
can make it always or never show. In Split and Edit the text takes its place.

### Find

**Find** (Ctrl+F) opens a bar under the toolbar. Type, and every match is marked, with "3 of 12"
beside the box. **Enter** or **F3** goes to the next match, **Shift+Enter** or **Shift+F3** to the
one before, and **Esc** closes the bar. A match inside a folded section (such as the properties)
opens it. In Split and Edit, Ctrl+F finds in the text instead (below).

### Zoom, width, style and theme

- **Zoom** (Ctrl+Plus, Ctrl+Minus, Ctrl+0 for 100%, or Ctrl and the mouse wheel, or a touchpad
  pinch) changes the document's size from 50% to 300%, in that window. The toolbar shows it; click
  the number to go back to 100%. Settings has the zoom every window starts at.
- **Reading width and style** (the toolbar, or **View > Reading width**): Narrow, Comfortable or
  Wide (the default), and Sans or Serif type.
- **Theme** (**View > Theme**, or Settings): Follow Windows, Light or Dark. Printing and copying
  always use the light theme.

### The status bar

**View > Status bar** (or Settings) shows a strip at the bottom of the window with the file's
path; click the path to copy it. If the file opened for reading only because of where it is (an
email attachment, a download, a temporary folder), it says so. In Split and Edit it also shows
the number of words, the file's encoding and its line endings.

### When the file changes

If the open file changes on disk (you save it in another app, a sync updates it, a colleague
changes it on a shared drive), Legible shows the new version within a second, keeps your place,
and says "Updated just now". Pictures it shows are watched too. If the file is moved or deleted, a
bar says so: **Keep showing it**, or **Close**. If it comes back, the bar goes.

If you're editing it when that happens, Legible doesn't replace your changes:
see [Saving, and when the file changes](#saving-and-when-the-file-changes).

### Links

- A link to a web page opens in your browser. A link that isn't secure (it starts with `http:`,
  not `https:`) asks first.
- An email address opens your mail app.
- A link to another Markdown file opens it in Legible. A link to a heading in the same document
  scrolls to it.
- Links to anything else (programs, other kinds of file) do nothing.

### Open in editor and Show in folder

In the **⋯** menu: **Open in editor** opens the file in the app Windows uses to edit Markdown
files (or the one that opens them, or Notepad); **Show in folder** opens its folder in File
Explorer with the file selected.

## Pictures from the internet

A document can show pictures from a web address. Opening one tells that address you opened the
document, and when, so (as in Outlook) **Legible doesn't load them until you allow it.** Until
then each shows as a placeholder, and a bar asks: "This document shows pictures from the internet.
Show them?"

- **Show once** shows them in this window, until you close it.
- **Always for this file** shows them now and every time you open this file.
- The bar's close button leaves them hidden.

**Settings > Reading > Pictures from the internet** can change that for every document: **Ask** (the
default), **Never show them** or **Always show them**.

Even when you allow them, Legible loads only the pictures the document names, from the sites it
names them on, and only over a secure connection (`https:`). A picture whose address isn't secure
stays a placeholder. While you have unsaved changes the bar asks you to save first, because
showing the pictures reloads the page.

## Copy, print and PDF

### Copy into Outlook, Word or OneNote

- **Copy document** (Ctrl+Shift+C, the toolbar's **Copy**, or **Edit > Copy document**) copies the
  whole document.
- **Ctrl+C** with text selected in the document copies the selection.

Paste into an email, a Word document or a OneNote page and it keeps its headings, lists, tables,
code, links and pictures, in the light theme. Diagrams are pasted as pictures, and alerts and code
as boxes. An app that only takes text gets tidy plain text: list bullets, quoted lines and table
columns kept. "Copied, ready to paste into Outlook, Word or OneNote" says it worked.

A picture from the internet is copied as its description unless it's shown. Very large pictures
(over 5 MB, or 20 MB in all) are copied as their description too, with a note.

### Print

**Print** (Ctrl+P) prints the document alone, in the light theme, without the toolbar. Each page
has the document's title at the top and "Page 1 of 3" at the bottom. Tables, code blocks, alerts,
pictures and diagrams aren't split across pages where they fit on one, and a link's web address
is printed after it.

### Save as PDF

**File > Save as PDF…** (or the **⋯** menu) saves the same pages as a PDF: on A4, or on Letter if
your Windows region measures in inches. It suggests the document's name, in its folder (or in
Documents). When it's done, **Open** opens the PDF and **Show in folder** shows it. An existing
PDF is only replaced once the new one is complete.

## Editing: View, Split and Edit

Every file opens in **View**, for reading. To change it, use the toolbar's **View, Split and
Edit**, the **View** menu, or **Ctrl+E**, which goes View, Split, Edit and back.

- **Split** shows the Markdown text on the left and the document on the right, updated a moment
  after you stop typing. The two scroll together. Drag the line between them to give either more
  room (or use the arrow keys when it's selected); double-click it to share the window evenly.
- **Edit** shows the text alone, in the reading column.

Changing between them keeps your place: the text opens where you were reading, and back in View
the document shows where you were in the text, or in Split's document.

The text is in the reading style's font, with Markdown's marks faded and headings larger.
**Settings > Appearance > Editor font** can make it Monospace instead.

**Shortcuts while editing:**

| Keys | What they do |
|---|---|
| Ctrl+B, Ctrl+I | Bold, italic (or take it off again) |
| Ctrl+K | Make the selection a link |
| Ctrl+1, Ctrl+2, Ctrl+3 | A heading, level 1, 2 or 3 |
| Ctrl+0 | Back to plain text |
| Ctrl+Z, Ctrl+Y | Undo, redo |
| Ctrl+F, Ctrl+H | Find, and find and replace |
| Ctrl+Alt+G | Go to a line |
| Ctrl+S | Save |

**Spell-check** is Windows' own: misspelt words are underlined, and right-click one for
suggestions.

## Saving, and when the file changes

**Save** (Ctrl+S, or **File > Save**) saves the file in its own format: its encoding (UTF-8 or
UTF-16) and its line endings, so an unchanged file is saved exactly as it was. While there are
unsaved changes, a dot shows beside View, Split and Edit and in the window's title ("● notes.md –
Legible"). Undoing back to the saved text clears it.

**Closing** a window with unsaved changes asks: **Save**, **Don't save** or **Cancel**. Don't save
is the only way changes are dropped.

**Windows signing out, restarting or shutting down** with unsaved changes: Legible keeps a copy of
them, and Windows shows that Legible is stopping it, so you can go back and choose. If Windows
goes ahead anyway, the next time Legible starts it opens the file with a bar: "Restore unsaved
changes to notes.md?" **Restore** puts them back into the text, unsaved, in Split (Undo takes
them out again); **Discard** drops them. If the file has gone, or can't be changed where it is,
Legible asks before opening anything whether to save them as a new file.

### Save a copy

**File > Save a copy…** (Ctrl+Shift+S) saves the text as a new file (in Documents, named
"notes (copy).md" to start with), and opens the copy in a window of its own. It never replaces
the open file. Use it for a file you can't change where it is.

### Files you can't change where they are

Some files open for reading only. Split and Edit are greyed out, and their tooltips say how to
edit a copy:

- **An email attachment, a download or a file in a temporary folder:** opened from there, it's a
  temporary copy, and your changes would be lost.
- **A read-only file.**
- **A file that isn't saved as UTF-8 or UTF-16:** Legible shows it but doesn't edit it, so it
  never changes its characters. **Save a copy…** keeps it exactly as it is.

Save a copy where you keep your documents, and edit that.

### "notes.md changed while you were editing it"

If the file changes on disk while you have unsaved changes (another app, a sync, a colleague on a
shared drive), Legible keeps your changes on screen and shows a bar:

- **Keep my changes** keeps your text. Save, and it replaces the file.
- **Use the file** shows the file as it is now, after asking: your changes are discarded.
- **Compare** shows the file in the document pane and your changes in the text, side by side,
  until you choose (or **Stop comparing**).

If the file was **moved or deleted**, the bar offers **Keep my changes** (save to put it back) or
**Close**.

If the file was **saved by something else at the same moment you saved**, both are kept: your
changes in the file, and the other version in a copy beside it ("notes (other version).md").
**Keep my changes** leaves that copy for you to keep or delete; **Use the other version** puts it
into the file and the text (Undo brings yours back). Legible never deletes that copy.

## Settings

**Tools > Settings…** (or the **⋯** menu). Changes apply to every window at once, and are saved.
Settings has three tabs: **Appearance**, **Reading** and **General**. It opens at the tab you
used last in that window. From the keyboard, the arrow keys move between the tabs and Tab goes
into one; Esc closes Settings.

**Appearance**

| Setting | What it does |
|---|---|
| Theme | Follow Windows (the default), Light or Dark |
| Reading style | Sans (the default) or Serif type |
| Reading width | Narrow, Comfortable or Wide (the default) |
| Default zoom | The zoom each new window starts at |
| Editor font | The reading style (Sans or Serif), or Monospace |

**Reading**

| Setting | What it does |
|---|---|
| Outline | Automatic, for long documents (the default), Always or Never |
| Status bar | Shows the strip with the file's path at the bottom (off by default) |
| Pictures from the internet | Ask (the default), Never show them, or Always show them |
| **Comfort:** Zoom with Ctrl + mouse wheel | On by default; off, the wheel only scrolls |
| **Comfort:** Open windows at their last size and position | On by default; off, each window opens at the standard size, where Windows puts it |
| **Comfort:** Remember where I was in each file | On by default; turning it off forgets every place |

**General**

| Setting | What it does |
|---|---|
| Default app | Make Legible the default for Markdown files; for a portable copy, Register or Remove |

## Keyboard shortcuts

| Keys | What they do |
|---|---|
| Ctrl+O | Open |
| Ctrl+S | Save |
| Ctrl+Shift+S | Save a copy |
| Ctrl+P | Print |
| Ctrl+W | Close the window |
| Ctrl+E | View, Split, Edit |
| Ctrl+Shift+O | Outline |
| Ctrl+F, F3, Shift+F3, Esc | Find, next, previous, close |
| Ctrl+H | Find and replace (in Split and Edit) |
| Ctrl+Shift+C | Copy document |
| Ctrl+C | Copy the selection |
| Ctrl+A | Select the whole document (or all the text, in the editor) |
| Ctrl+Plus, Ctrl+Minus, Ctrl+0 | Zoom in, zoom out, 100% |
| Alt | The menu bar |

## Privacy

- **No telemetry.** Legible sends nothing about you or your documents anywhere, and has no
  account.
- **No network**, except pictures from the internet that you allow
  ([above](#pictures-from-the-internet)). Legible itself contains no code that connects to the
  internet; the pictures you allow are fetched by WebView2, Microsoft's component that shows the
  page.
- **Everything stays on your PC:** settings, recent files and logs are in Legible's folder
  ([below](#where-legible-keeps-things)).

## Where Legible keeps things

Installed with the MSI, Legible keeps its files in `%APPDATA%\Legible\`. A portable copy keeps
them beside `legible.exe` (it's portable because of the `portable` file there).

| File or folder | What |
|---|---|
| `settings.toml` | Your settings |
| `recent.txt` | Recent files |
| `places.txt` | Where you were in each of the last 100 files you closed |
| `window.toml` | The size and place of the last window closed |
| `remote-images.txt` | Files you chose "Always for this file" for |
| `Recovery\` | Unsaved changes kept as Windows signed out, until you choose |
| `logs\` | What Legible did, for when something goes wrong: the last 20 runs, at most 14 days |
| `WebView2\` | WebView2's own data |

Uninstalling keeps these; delete the folder to remove them.

## Troubleshooting

### "Legible needs Microsoft Edge WebView2"

Legible shows documents with Microsoft Edge WebView2, which comes with Windows 10 and 11 but can
be missing (on some managed PCs, or an older Windows 10). Choose **Yes** to open Microsoft's
WebView2 page in your browser, download the **Evergreen** runtime and install it, then open
Legible again. Legible never downloads anything itself. On a work PC, your IT team may need to
install it.

### Double-clicking a Markdown file doesn't open Legible

Windows opens a file type with the app chosen for it. Choose Legible: use the bar in Legible's
window, or **Settings > General > Make Legible the default for Markdown files**, then in
Default apps choose Legible for `.md` (and `.markdown`, `.mdown`). Or right-click the file, choose
**Open with > Choose another app**, pick Legible and tick **Always**.

If Legible isn't in the list at all: a portable copy needs **Register Legible for .md files on
this PC** first; the MSI does it when it installs.

### Pictures don't show

- **A placeholder, and a bar asking:** they're pictures from the internet, waiting for you to
  allow them ([above](#pictures-from-the-internet)). If there's no bar, check **Settings >
  Reading > Pictures from the internet** isn't **Never show them**.
- **"Save your changes to show pictures from the internet":** you have unsaved changes; save, and
  they show.
- **A placeholder that never loads:** its address isn't secure (`http:`), so Legible won't load it.
- **"Legible can't show pictures from the internet on this computer":** the WebView2 on this PC is
  too old to let Legible check each address, so it shows none. Updating WebView2 (or Windows)
  fixes it.
- **"Legible stopped showing pictures from the internet in this document…":** Legible couldn't
  check one of their addresses, so it stopped, to be safe. Close and open the document again.
- **A picture's description instead of the picture:** the file isn't there, or isn't in the
  document's folder or a folder inside it. Legible shows only pictures beside the document, so a
  document can't reach other files on your PC.

### A file opens for reading only

Split and Edit are greyed out, and their tooltips say why: the file is an email attachment, a
download or in a temporary folder, is read-only, or isn't UTF-8 or UTF-16. The status bar (View >
Status bar) says which, for the first three. Use **File > Save a copy…** to save it where you keep
your documents, and edit the copy ([above](#files-you-cant-change-where-they-are)).

### "This file changed on disk" (or "changed while you were editing it")

Something else changed the file: another app, a sync (OneDrive, SharePoint), or a colleague on a
shared drive. If you had no unsaved changes, Legible has already shown the new version. If you
had, it kept your changes and asks what to do: **Keep my changes**, **Use the file** or
**Compare** ([above](#notesmd-changed-while-you-were-editing-it)). Nothing is lost until you
choose. "Saved at the same moment" means both versions are kept, one in a copy beside the file.

### "This file was moved or deleted"

The file isn't where it was. **Keep showing it** keeps reading it; if it comes back (a sync put it
back, say), Legible shows it again. With unsaved changes, **Keep my changes** and Save put it
back.

### Windows protected your PC

Legible isn't code-signed yet, so SmartScreen warns about the download. Check it against the
release's `SHA256SUMS.txt` (the README's
[SmartScreen and checksums](../README.md#smartscreen-and-checksums) says how), then choose **More
info**, then **Run anyway**.

### "Your settings couldn't be saved"

Legible's folder can't be written to: usually a portable copy on a read-only drive or folder.
Your settings apply until you close Legible. Move the portable copy somewhere you can write to.

### A diagram shows an error

Its Mermaid text has a mistake, or uses something Legible's Mermaid doesn't: a diagram that asks
for the `elk` layout can't be drawn, as that part of Mermaid isn't included. The error box shows
the diagram's text; correct it in Split to see it drawn.

### Copy says "Couldn't copy" or "Copied as plain text"

"Couldn't copy: another app is using the clipboard" means another app held on to the clipboard
for too long: wait a moment and copy again. "Copied as plain text" means the text was copied but
its formatting couldn't be; copying again usually brings it.

### A very large document is slow

A document of a few hundred KB opens in about a second. A very large one (2 MB, hundreds of pages)
takes a few seconds to appear, and a search in it answers in under a second. For a few seconds
after it appears, Legible lays out the rest of it in the background: scrolling or jumping far
down may stutter until it has, and after that it scrolls smoothly.

### Something else

Open an issue on [GitHub](https://github.com/Suremise/legible/issues), with what you did, what
happened, and **Help > About Legible > Copy** pasted in. If Legible closed, the newest file in
`logs\` helps (check it first: it has file paths).
