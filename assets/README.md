# Legible's icons

Original art for Legible, under Legible's MIT licence. No third-party icon sets, fonts or images
are used in any of the icons.

**Every file here except this README is generated** by `export.py`. To change an icon, edit the
shapes in `export.py` and run it again; don't edit the SVG, ICO or PNG files by hand.

```
python assets/export.py
```

It needs Python 3.10 or later and [Pillow](https://python-pillow.org/) (`pip install pillow`;
made with Pillow 12.3.0). Both are build-only tools and don't ship in Legible. With the same
Pillow version, it writes the same bytes every time.

## The files

| File | What |
|---|---|
| `icon/legible.svg` | The app icon's master, used for 40 px and up |
| `icon/legible-16.svg`, `legible-32.svg` | The small variant at 16 and 32 px (20 and 24 px are drawn the same way, tuned to their own pixels) |
| `icon/legible.ico` | The exe's and the window's icon: 16, 20, 24, 32, 40, 48, 64 and 256 px |
| `icon/legible-64.png` … `legible-1024.png` | 64, 128, 256, 512 and 1024 px, for the README, the docs and surmise.it/tools |
| `file/markdown.svg` | The `.md` file icon at 256 px, as a master to look at |
| `file/markdown-16.svg` | The `.md` file icon at 16 px |
| `file/markdown.ico` | The icon Explorer shows for `.md` files: 16, 20, 24, 32, 40, 48, 64 and 256 px |
| `preview.png` | Both icons at every size on light and dark, with 16 and 32 px enlarged, for review |

## The icon

**A readable document.** A page with a folded corner, a heading (a bolder, shorter line) and
lines of text under it: what Legible shows you. No "#" or "M↓" mark: it doesn't survive 16 px,
and the icon is about reading, not about Markdown. It's a page in a circle, in blue clear of
Word's, so it doesn't look like Notepad, Word, OneNote or any other Microsoft icon.

Its style: one bold circle with a vertical gradient, a soft gloss ring, a dark face, and the
glyph in the brand's light tones. The shapes are on a 16-unit grid (the circle 7.375, the gloss
ring 6.6875, the face 5.625 with a 0.5 ring).

- **The large icon** (40 px and up): the page solid in blue 100 with its fold in blue 300, and the
  heading and three lines in the deep face colour, round-ended.
- **The small icon** (16 to 32 px): no gradient, gloss or face.
  A flat blue 500 disc, the page in the deep face colour and the lines cut through it in the
  disc's blue, square-ended. Each size is hand-tuned in whole pixels so its edges stay sharp: a
  heading and two lines at 16 and 24 px, a heading and three at 20 and 32 px.
- **The `.md` file icon:** Windows' document shape, a white page with a folded corner, a grey
  edge and, from 32 px, grey lines of text, with the Legible disc at the bottom right in a
  cut-out ring. The bottom right keeps clear of Explorer's shortcut arrow at the bottom left.
  From 32 px the disc is the app icon at a size of its own (16 px on the 32 px icon, 128 px on
  the 256 px one); below that it's the disc with a plain page.
- Each pixel size is drawn on its own, supersampled and then reduced, rather than shrunk from the
  biggest image.

**Contrast** (WCAG 2, printed by `export.py`):

| Pair | Ratio |
|---|---|
| The small icon's disc on a light taskbar (`#F3F3F3`) | 4.26:1 |
| The disc on a dark taskbar (`#202020`) | 3.45:1 |
| The small icon's page (`--brand-face-deep`) on the disc | 4.02:1 |
| The large icon's page (`--brand-blue-100`) on the face | 14.11:1 |
| The lines (`--brand-face-deep`) on the large icon's page | 15.81:1 |

## Palette

The icon's colours, fixed and the same in both themes. A cornflower-to-indigo blue, clear of
Word's blue. Legible's own blue is blue 600.

| Token | Hex | Used for |
|---|---|---|
| `--brand-blue-100` | `#E3EAFF` | The large icon's page |
| `--brand-blue-300` | `#9DB4FA` | Light: the gradient's top; the page's folded corner |
| `--brand-blue-400` | `#6F8EF0` | Base: the gradient's middle |
| `--brand-blue-500` | `#4D6BDB` | The small icon's flat disc and the lines cut through its page; the file icon's disc |
| `--brand-blue-600` | `#3A4BB8` | Dark: the gradient's bottom. Legible's blue |
| `--brand-face` | `#161B33` | The face, top |
| `--brand-face-deep` | `#0B0F22` | The face, bottom, and its ring; the lines of text; the small icon's page |

The `.md` file icon's page uses greys close to Windows' own document icons: `#FFFFFF` paper,
`#E4E7EC` fold, `#8A919C` edge and `#C3C8D0` lines.

The labels in `preview.png` use the Windows system font (Segoe UI) when it's there. The preview
is only for review and doesn't ship.
