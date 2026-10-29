# Strings used in more than one place. Conventions: docs/ARCHITECTURE.md, "Strings and languages".

app-name = Legible
# A build for another system: Legible v1 runs on Windows only.
app-windows-only = Legible runs on Windows only in this version.

## The rendered document

# The heading of the table of a document's front matter (its title, author, date and so on).
properties-title = Properties
# A front matter value that's true or false (YAML's and TOML's true and false).
properties-true = true
properties-false = false
# Between the items of a list in a front matter value.
properties-list-separator = {", "}
# One entry of a table in a front matter value. $key: its name; $value: its value.
properties-entry = { $key }: { $value }
# GitHub alerts' titles, as GitHub shows them.
alert-note = Note
alert-tip = Tip
alert-important = Important
alert-warning = Warning
alert-caution = Caution
