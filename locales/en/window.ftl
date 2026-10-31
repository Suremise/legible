# The document window. Conventions: docs/ARCHITECTURE.md, "Strings and languages".

# A document window's title. $file: the document's file name.
window-title = { $file } – Legible
# The same, while the document has changes that aren't saved. $title: the title above.
window-title-unsaved = ● { $title }

# WebView2 is missing: Legible can't show anything without it, and never downloads it itself.
webview2-missing-title = Legible needs Microsoft Edge WebView2
webview2-missing =
    Legible shows documents with Microsoft Edge WebView2, which is part of Windows 10 and 11 but isn't installed on this PC.

    You can get it free from Microsoft's page:
    { $url }

    Open that page in your browser now?

# A link to a web page that isn't secure (http, not https). $url: the link.
link-http-ask-title = Open a page that isn't secure?
link-http-ask = This link isn't secure (it starts with http, not https). Open it in your browser anyway?

    { $url }

# A document Legible couldn't open. $file: its path; $reason: what Windows said.
error-open-title = Legible couldn't open a document
error-open = Legible couldn't open { $file }: { $reason }

# The window started with no document.
window-no-document = Open a Markdown file (.md) to read it here: double-click it in File Explorer.

## The window's chrome: its toolbar, outline, find bar, Settings and About. A shortcut is shown
## in a button's tooltip after its name, as "Find (Ctrl+F)".

chrome-toolbar = Toolbar
# A button's tooltip with its shortcut. $label: the button's name; $keys: its keys, joined with
# "+" from the menu-key-* names, as "Ctrl+Shift+O".
chrome-tip-shortcut = { $label } ({ $keys })
# A button's tooltip with a note on why it can't be used. $label: its name; $note: the note.
chrome-tip-note = { $label }: { $note }
chrome-open = Open
# The Open dialog's file type.
chrome-open-filter = Markdown files
chrome-mode = View, Split or Edit
chrome-mode-view = View
chrome-mode-split = Split
chrome-mode-edit = Edit
# In each mode's tooltip after its name, as "Split (Ctrl+E cycles)": the shortcut goes round
# View, Split and Edit.
chrome-mode-cycles = Ctrl+E cycles
# A button whose feature isn't in this version yet.
chrome-coming-soon = Coming soon
chrome-outline = Outline
chrome-find = Find
chrome-zoom-out = Zoom out
chrome-zoom-in = Zoom in
chrome-zoom-reset = Reset zoom
# The zoom, on the Reset zoom button. $zoom: the percentage (100, 125â€¦), as the language writes
# numbers.
chrome-zoom-level = { $zoom }%
chrome-reading = Reading width and style
chrome-width-narrow = Narrow
chrome-width-comfortable = Comfortable
chrome-width-wide = Wide
chrome-style-sans = Sans
chrome-style-serif = Serif
chrome-print = Print
chrome-copy = Copy
chrome-more = More
chrome-save-pdf = Save as PDF
chrome-open-editor = Open in editor
chrome-show-folder = Show in folder
chrome-settings = Settings
chrome-about = About Legible
chrome-close = Close

# The outline panel, and the handle that changes its width.
chrome-outline-panel = Outline
chrome-outline-resize = Resize the outline
# The Outline button's tooltip in Split and Edit, where the editor takes the outline's place.
chrome-outline-in-view = shown in View

# The find bar. $current and $total: which match is selected, and how many there are.
chrome-find-field = Find in the document
chrome-find-previous = Previous match
chrome-find-next = Next match
chrome-find-count = { $current } of { $total }
chrome-find-none = No matches

# Settings. Its tabs, each a group of settings: how documents look; reading them; and Legible on
# this PC (the default app).
chrome-settings-tab-appearance = Appearance
chrome-settings-tab-reading = Reading
chrome-settings-tab-general = General
chrome-settings-theme = Theme
chrome-theme-system = Follow Windows
chrome-theme-light = Light
chrome-theme-dark = Dark
chrome-settings-style = Reading style
chrome-settings-width = Reading width
chrome-settings-zoom = Default zoom
chrome-settings-remote = Pictures from the internet
chrome-remote-ask = Ask
chrome-remote-never = Never show them
chrome-remote-always = Always show them
chrome-settings-outline = Outline
chrome-outline-automatic = Automatic, for long documents
chrome-outline-always = Always
chrome-outline-never = Never
# Small comforts, each on by default: Ctrl + mouse wheel zooms the document; a new window opens
# at the size and place of the last one closed; a file opens where the reader was in it.
chrome-settings-comfort = Comfort
chrome-settings-wheel-zoom = Zoom with Ctrl + mouse wheel
chrome-settings-reopen-windows = Open windows at their last size and position
chrome-settings-remember-place = Remember where I was in each file
chrome-settings-default-app = Make Legible the default for Markdown files
# Settings' Default app section: its heading, what it says once Legible opens Markdown files, and
# a portable copy's buttons (it changes nothing on the PC until the user registers it).
chrome-default-heading = Default app
chrome-default-is = Legible opens your Markdown files.
chrome-default-register = Register Legible for .md files on this PC
chrome-default-unregister = Remove Legible from this PC's apps for .md files
chrome-default-portable = This copy of Legible is portable: it adds nothing to this PC until you register it.
# Settings couldn't be saved. They still apply until Legible closes. $reason: what Windows said.
chrome-save-read-only = Your settings couldn't be saved: Legible's folder can't be written to. They apply until you close Legible.
chrome-save-failed = Your settings couldn't be saved ({ $reason }). They apply until you close Legible.


# Live reload: the note when the document changed on disk and was shown again, and the bar
# when it was moved or deleted.
chrome-updated = Updated just now
chrome-file-gone = This file was moved or deleted.
chrome-file-keep = Keep showing it

# Recent files: the heading in the More menu and the Jump List's category, the empty menu's line,
# and what a window says when what's dropped on it isn't Markdown.
chrome-recent = Recent files
chrome-recent-none = No recent files
chrome-drop-not-markdown = Legible opens Markdown files (.md, .markdown and .mdown).

# Open in editor and Show in folder, in the More menu: what a window says when one didn't work.
chrome-editor-failed = Couldn't open an editor for this file.
chrome-folder-failed = Couldn't show this file's folder.

# Pictures from the internet: the bar that asks before any loads, its buttons and its label for
# screen readers, and what it says when this document can't show any.
chrome-pictures-ask = This document shows pictures from the internet. Show them?
chrome-pictures-once = Show once
chrome-pictures-always = Always for this file
chrome-pictures-label = Pictures from the internet
chrome-pictures-off-runtime = Legible can't show pictures from the internet on this computer.
chrome-pictures-off-failed = Legible stopped showing pictures from the internet in this document because it couldn't check one of their addresses.
# Instead of asking, while the editor has unsaved changes: showing pictures from the internet
# reloads the page, which waits until the changes are saved.
chrome-pictures-save-first = Save your changes to show pictures from the internet.
# A file in an encoding Legible doesn't edit (not UTF-8 or UTF-16): shown, but not editable.
chrome-encoding-unsupported = This file isn't saved as UTF-8 or UTF-16, so Legible shows it but can't edit it. Save as… keeps it exactly as it is, in a new file.
# The file mixes line endings; saving writes every line with the one it uses most.
chrome-line-endings-mixed = This file mixes line endings. Saving it uses { $ending ->
        [crlf] Windows line endings (CRLF)
        [cr] old Mac line endings (CR)
       *[lf] Unix line endings (LF)
    } for every line.

# Saving: the conflict bar, a save that failed, Save as and closing.

# The conflict bar: the file changed on disk while there are unsaved changes. There's one file,
# and two versions of its text: your changes (in Legible, unsaved) and the file (as it is on
# disk now). Legible can't know who changed it (another app, a sync, a colleague), so these never
# say "they", "theirs" or "another program". { $file } is the open file's name.
chrome-conflict-changed = { $file } changed while you were editing it.
chrome-conflict-deleted = { $file } was moved or deleted. Keep your changes to save it again, or close it.
# The file was saved by something else at the same moment as Legible saved it. { $name } is the
# copy beside it that holds the other version.
chrome-conflict-raced = { $file } was saved at the same moment you saved. Both are kept: your changes are in { $file }, and the other version is in the copy "{ $name }".
# The name the other version of a save made at the same moment is kept under, beside the file;
# { $name } is the file's, without its extension (which the copy has too), and { $n } a number
# when that's taken.
chrome-raced-name = { $name } (other version)
chrome-raced-name-numbered = { $name } (other version { $n })
chrome-conflict-keep = Keep my changes
chrome-conflict-take = Use the file
# For a save made at the same moment: the other version, from its copy.
chrome-conflict-take-other = Use the other version
chrome-conflict-compare = Compare
chrome-conflict-stop-comparing = Stop comparing
chrome-conflict-cancel = Cancel
chrome-conflict-take-confirm = Use the file as it is now? Your changes will be discarded.
chrome-conflict-take-raced = Use the other version, in the file and the editor, instead of your changes? Undo in the editor brings your changes back.
chrome-conflict-comparing = Comparing: the preview shows the file, the editor your changes.
chrome-conflict-comparing-raced = Comparing: the preview shows the other version, the editor your changes.
# The editor bar's text while comparing: $text, what the bar says of the change, then $comparing
# (chrome-conflict-comparing or chrome-conflict-comparing-raced).
chrome-conflict-with-comparing = { $text } { $comparing }
# Use the file, done: the file is shown as it is on disk now.
chrome-conflict-taken = Your changes were discarded. { $file } shows the file as it is now.
# Keep my changes for a save made at the same moment: the other version stays in its copy, for the
# reader to keep or delete.
chrome-conflict-raced-kept = Your changes stay in { $file }. The other version is still in the copy "{ $name }" if you want it.
# Use the other version, done: it's the file now; its copy is left for the reader.
chrome-conflict-raced-taken = The other version is in { $file } now. The copy "{ $name }" is still there, if you want to delete it.
# Either answer for a save made at the same moment, when the file has changed again since.
chrome-conflict-raced-again = { $file } changed again, so nothing was replaced or removed. The other version is still in the copy "{ $name }".
chrome-conflict-raced-take-failed = The other version couldn't be put back ({ $reason }). It's still in the copy "{ $name }".
chrome-save-file-failed = { $name } couldn't be saved ({ $reason }). Your changes are still here.
chrome-save-partial = Saving { $name } didn't finish, and nothing is lost: the file as it was is at { $original }, and your version at { $saved }.
# Save as: the text saved as a new file, which the window then shows in place of the old one.
chrome-save-as-title = Save as
chrome-save-as-filter = Markdown
# Save as chose the file itself, which can't be saved where it is (an attachment, say).
chrome-save-as-same = This file can't be saved where it is. Choose another folder or name.
# Save as chose a file that's open in another Legible window. { $name } is its name.
chrome-save-as-open = { $name } is open in another window. Close it there first, or choose another name.
chrome-save-as-changed = That file changed while it was being saved, so it wasn't replaced. Try again.
chrome-save-as-failed = The file couldn't be saved ({ $reason }). Your changes are still here.
# Closing a window with unsaved changes.
save-close-question = Save your changes to { $name }?
save-close-detail = If you don't save, your changes will be lost.
save-close-save = Save
save-close-dont-save = Don't save
# Windows signing out, restarting or shutting down while a window has unsaved changes: the reason
# Windows shows for that window on its screen of apps that are stopping it.
session-unsaved = You have unsaved changes in { $name }
# The next start: unsaved changes kept as Windows ended the session, offered back on a bar over
# the file (Restore puts them into the editor, unsaved; Discard drops them). If the file has
# gone or can't be edited where it is, a question instead: save them as a new file, or discard.
restore-question = Restore unsaved changes to { $name }?
restore-changed = { $name } has changed since then.
chrome-restore = Restore
chrome-discard = Discard
restore-elsewhere = { $name } has been moved or deleted, or can't be changed where it is. Save the changes as a new file, or discard them.
restore-save-as = Save as…
restore-discard = Discard
restore-failed = Your changes couldn't be saved ({ $reason }). Legible keeps them, and offers them again the next time it starts.

# Getting it out: what a window says after Copy (Copy document or Copy selection), the alt text
# a copied diagram's picture has, the footer printed on each page (Print and Save as PDF), and
# what it says after Save as PDF, with its buttons.
chrome-copied = Copied, ready to paste into Outlook, Word or OneNote
chrome-copied-text-only = Copied as plain text: the formatting couldn't be copied
chrome-copy-failed = Couldn't copy: { $reason }
chrome-copy-busy = another app is using the clipboard
chrome-copy-diagram = Diagram
chrome-print-page = Page { $page } of { $pages }
chrome-saved = Saved { $name }
chrome-open-file = Open
chrome-pdf-busy = A PDF of this document is already being saved.
chrome-pdf-failed = Couldn't save the PDF: { $reason }
chrome-pdf-timed-out = it took too long
chrome-pdf-print-failed = WebView2 couldn't print the PDF

# The Save as PDF dialog: its title, and its file type.
window-save-pdf-title = Save as PDF
window-save-pdf-filter = PDF document (*.pdf)

# The default app: the bar a window shows while Legible isn't the default for Markdown files
# (its button is chrome-settings-default-app), its close button, the line shown once Default apps
# opens, and what it says when the file associations couldn't be changed. $reason: what Windows
# said.
chrome-default-offer = Open Markdown files in Legible when you double-click them.
chrome-default-dismiss = Don't ask again
chrome-default-how = In Default apps, choose Legible for .md (and for .markdown and .mdown, if you have them).
chrome-default-failed = Legible couldn't change its file associations ({ $reason }).

# The installer (installer/legible.wxs; one line each, no placeables: the installer can't format
# Fluent): when a newer Legible is installed, and the Start menu shortcut's description. Then the
# file associations: the type File Explorer shows for Markdown files, and Default apps' line about
# Legible.
installer-downgrade = A newer version of Legible is already installed.
installer-shortcut-description = Read Markdown files
installer-file-type = Markdown document
installer-app-description = Reads Markdown files: notes, reports and documentation, clearly laid out.

## The editor (Split and Edit).

# The dot beside View, Split and Edit while there are changes that aren't saved.
chrome-unsaved = Unsaved changes
# After Split's or Edit's name, on a file that can't be changed where it is (read-only, or an
# email's attachment): saving it elsewhere is how to edit it.
chrome-mode-not-editable = this file can't be edited where it is. To edit it, save it somewhere else with Save as (Ctrl+Shift+S).
# The toolbar's Save and Save as, beside the modes. Save is greyed out, its tooltip saying why
# after its name, while there's nothing unsaved, or on a file that can't be saved where it is.
chrome-save = Save
chrome-save-as = Save as
chrome-save-nothing = nothing to save
chrome-save-not-editable = this file can't be saved where it is. Use Save as to save it somewhere else.
chrome-editor = Markdown text
chrome-editor-resize = Resize the text and the document
chrome-settings-editor-font = Editor font
chrome-editor-font-reading = The reading style (Sans or Serif)
chrome-editor-font-monospace = Monospace
# Find and replace in the editor (Ctrl+F, Ctrl+H): the fields, buttons and options.
chrome-editor-find = Find
chrome-editor-replace-with = Replace with
chrome-editor-next = Next
chrome-editor-previous = Previous
chrome-editor-select-all = Select all
chrome-editor-match-case = Match case
chrome-editor-regexp = Regular expression
chrome-editor-by-word = Whole words
chrome-editor-replace = Replace
chrome-editor-replace-all = Replace all
chrome-editor-close = Close
# What a screen reader hears as the matches are stepped through: "current match", then
# "on line" and the line's number.
chrome-editor-current-match = current match
chrome-editor-on-line = on line
# After a replace, for a screen reader. CodeMirror puts the number where the $ is: keep it.
chrome-editor-replaced-one = Replaced the match on line $
chrome-editor-replaced-many = Replaced $ matches
# Go to line (Ctrl+Alt+G in the editor).
chrome-editor-go-to-line = Go to line
chrome-editor-go = Go
# Read out for an invisible character in the text, and a deleted selection.
chrome-editor-control-character = Control character
chrome-editor-selection-deleted = Selection deleted

## The menu bar above the toolbar: Windows' own menus. An & marks the access key, the letter
## underlined when Alt is held: within each menu, give every item a different one ("&&" shows
## an "&"). A shortcut is shown after its item, as the menu-key strings build it ("Ctrl+O").

menu-file = &File
menu-edit = &Edit
menu-view = &View
menu-tools = &Tools
menu-help = &Help
menu-open = &Open…
menu-recent = Open &recent
menu-save = &Save
menu-save-as = Save &as…
menu-save-pdf = Save as P&DF…
menu-print = &Print…
menu-close = C&lose
# Closes every Legible window, each asking about unsaved changes first.
menu-exit = E&xit
menu-undo = &Undo
menu-redo = &Redo
menu-cut = Cu&t
menu-copy = &Copy
menu-paste = &Paste
menu-select-all = Select &all
menu-find = &Find…
menu-replace = R&eplace…
menu-copy-document = Copy &document
menu-mode-view = &View
menu-mode-split = &Split
menu-mode-edit = &Edit
menu-outline = &Outline
menu-status-bar = Status &bar
menu-zoom-in = Zoom &in
menu-zoom-out = Zoom o&ut
menu-zoom-reset = &Actual size
menu-width = Reading &width
menu-width-narrow = &Narrow
menu-width-comfortable = &Comfortable
menu-width-wide = &Wide
menu-theme = &Theme
menu-theme-system = &Follow Windows
menu-theme-light = &Light
menu-theme-dark = &Dark
menu-settings = &Settings…
menu-about = &About Legible
# The keys of a shortcut, joined with "+": "Ctrl+Shift+S", "Ctrl+Plus".
menu-key-ctrl = Ctrl
menu-key-shift = Shift
menu-key-plus = Plus
menu-key-minus = Minus
# Named in the toolbar's tooltips only (Enter and Esc: the find bar's next match and close).
menu-key-enter = Enter
menu-key-esc = Esc

## The status bar (View > Status bar, or Settings): the file's path; while editing, its words,
## encoding and line endings.

# Why a file opened for reading only, after its path.
status-from-attachment = Opened from an email attachment
status-from-download = Opened from a download
status-from-temp = Opened from a temporary folder
status-utf8 = UTF-8
status-utf8-bom = UTF-8 with BOM
status-utf16le = UTF-16 LE
status-utf16be = UTF-16 BE
# The file's line endings: Windows', Unix's and old Macs'.
status-crlf = CRLF
status-lf = LF
status-cr = CR
# A file whose lines end in more than one way: saving makes them all $kept (CRLF, LF or CR).
status-endings-mixed = Mixed ({ $kept } kept)
# The text's words, not counting the Markdown marks round them. $count: how many, as the
# language writes numbers.
chrome-status-words =
    { $count ->
        [one] { $count } word
       *[other] { $count } words
    }
chrome-status-copy-path = Click to copy the path
# The path's tooltip and what a screen reader says of it: $path, the file's path, and $action,
# chrome-status-copy-path.
chrome-status-path-tooltip =
    { $path }
    { $action }
chrome-status-path-label = { $path }, { $action }
chrome-status-path-copied = Path copied
chrome-status-label = Status bar
# Settings' row for the status bar, and its checkbox.
chrome-settings-status = Status bar
chrome-settings-status-bar = Show the status bar

## About Legible: Help > About Legible, or the ⋯ menu.

chrome-about-tagline = A reader that makes Markdown legible: read it, edit it, copy it into Outlook, Word or OneNote, print it or save it as PDF.
# The licence line: { $mit } and { $notices } are links (chrome-about-mit, chrome-about-third-party).
chrome-about-licence-line = Open source under the { $mit }. Includes { $notices }.
chrome-about-mit = MIT License
chrome-about-third-party = third-party software
chrome-about-privacy = No telemetry. Legible goes online only for pictures you allow.
# Word for word as in README.md: change both together.
chrome-about-notice = © 2026 Darren Marshall. Legible is independently developed and isn't affiliated with, endorsed or sponsored by Microsoft. Microsoft, Windows, Outlook, Word and OneNote are trademarks of the Microsoft group of companies.
chrome-about-copy = Copy
chrome-about-release-notes = Release notes
chrome-about-ok = OK
chrome-about-copied = Version information copied
chrome-about-licence-title = MIT License
chrome-about-notices-title = Third-party notices
chrome-about-release-notes-title = Release notes
chrome-about-back = Back
# About's table, and what Copy puts on the clipboard.
# Copy's first line: $app, Legible's name, and $version, its version (as "0.1.0").
about-copy-title = { $app } { $version }
# Each of Copy's other lines: one of the table's rows, $label and its $value.
about-copy-row = { $label }: { $value }
about-version = Version
about-build = Build
about-build-installed = Installed
about-build-portable = Portable
about-webview2 = WebView2
about-windows-label = Windows
# Windows' version: $major is 10 or 11, $release its release (as "24H2"), $build its build.
about-windows-release = Windows { $major } { $release } (build { $build })
about-windows = Windows { $major } (build { $build })
about-unknown = Unknown
