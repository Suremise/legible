"""Draws Legible's icons: the SVG masters, the .ico and .png files, and the preview sheet.

    python assets/export.py

Build-only (Python 3.10+ and Pillow; see assets/README.md). Every icon is described once, below,
as a list of shapes on a 16-unit grid; the same list is written as SVG and drawn as pixels. Each
pixel size is drawn on its own (supersampled, then reduced), not shrunk from a big image, and
32 px and below use the small drawing, flat and bolder, with its edges on whole pixels, so it
reads on a taskbar.

The icons' style: one bold circle with a vertical gradient, a soft gloss ring, a dark face and a
simple glyph in the brand's light tones. The glyph is a page with a heading and three lines of
text, "a readable document".
"""

from __future__ import annotations

import argparse
import math
import sys
from dataclasses import dataclass, field, replace
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ASSETS = Path(__file__).resolve().parent
GRID = 16.0

ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
PNG_SIZES = [64, 128, 256, 512, 1024]
# Up to this size, the small drawing.
SMALL_UP_TO = 32

# The palette (assets/README.md lists it as tokens). Fixed: the same in both themes.
BLUE_100 = "#E3EAFF"  # The large icon's page.
BLUE_300 = "#9DB4FA"  # Light: the gradient's top; the page's folded corner.
BLUE_400 = "#6F8EF0"  # Base: the gradient's middle.
BLUE_500 = "#4D6BDB"  # The small icon's flat disc; the lines cut through its page.
BLUE_600 = "#3A4BB8"  # Dark: the gradient's bottom. Legible's blue.
FACE = "#161B33"  # The face, top.
FACE_DEEP = "#0B0F22"  # The face, bottom, and its ring; the lines of text; the small icon's page.
WHITE = "#FFFFFF"
# The .md file icon's page, in the greys of Windows' own document icons.
PAPER = "#FFFFFF"
PAPER_FOLD = "#E4E7EC"
PAPER_EDGE = "#8A919C"
PAPER_LINE = "#C3C8D0"


# --- Shapes ------------------------------------------------------------------------------------


@dataclass
class Gradient:
    """A vertical gradient over the shape's box: (offset 0 to 1, colour, opacity) stops."""

    stops: list[tuple[float, str, float]]


Paint = str | Gradient | None


@dataclass
class RoundRect:
    x0: float
    y0: float
    x1: float
    y1: float
    radius: float
    fill: Paint = None
    stroke: Paint = None
    width: float = 0.0  # A stroke lies inside the rectangle.


@dataclass
class Circle:
    cx: float
    cy: float
    r: float
    fill: Paint = None
    stroke: Paint = None
    width: float = 0.0  # A stroke is centred on the circle, as in SVG.
    opacity: float = 1.0


@dataclass
class Polygon:
    points: list[tuple[float, float]]
    fill: Paint = None


Shape = RoundRect | Circle | Polygon


@dataclass
class Erase:
    """Cuts these shapes out of everything drawn before it (their colours are ignored)."""

    shapes: list[Shape]


@dataclass
class Group:
    """Steps drawn on a layer of their own, so an [`Erase`] in it cuts only what's in it."""

    steps: list["Step"]


Step = Shape | Erase | Group


@dataclass
class Icon:
    steps: list[Step] = field(default_factory=list)

    def add(self, *steps: Step) -> "Icon":
        self.steps.extend(steps)
        return self


def place(steps: list[Step], scale: float, dx: float, dy: float) -> list[Step]:
    """`steps` scaled by `scale` and then moved by (dx, dy): an icon drawn inside another."""

    def x(v: float) -> float:
        return v * scale + dx

    def y(v: float) -> float:
        return v * scale + dy

    out: list[Step] = []
    for s in steps:
        if isinstance(s, Group):
            out.append(Group(place(s.steps, scale, dx, dy)))
        elif isinstance(s, Erase):
            out.append(Erase(place(s.shapes, scale, dx, dy)))  # type: ignore[arg-type]
        elif isinstance(s, RoundRect):
            out.append(replace(s, x0=x(s.x0), y0=y(s.y0), x1=x(s.x1), y1=y(s.y1), radius=s.radius * scale, width=s.width * scale))
        elif isinstance(s, Circle):
            out.append(replace(s, cx=x(s.cx), cy=y(s.cy), r=s.r * scale, width=s.width * scale))
        elif isinstance(s, Polygon):
            out.append(replace(s, points=[(x(px), y(py)) for px, py in s.points]))
        else:
            raise TypeError(s)
    return out


# --- The drawings ------------------------------------------------------------------------------


def dog_ear(x0: float, y0: float, x1: float, y1: float, fold: float) -> list[tuple[float, float]]:
    """A page's outline with its top right corner folded over by `fold`."""
    return [(x0, y0), (x1 - fold, y0), (x1, y0 + fold), (x1, y1), (x0, y1)]


def page(box: tuple[float, float, float, float], fold: float, ink: str, fold_ink: str | None) -> list[Step]:
    """A page with a folded corner: the page cut at the corner, then the fold drawn on it, with a
    hairline gap so the face shows between them (`fold_ink` None leaves the corner cut away)."""
    x0, y0, x1, y1 = box
    steps: list[Step] = [Polygon(dog_ear(*box, fold), fill=ink)]
    if fold_ink:
        g = fold * 0.16
        steps.append(Polygon([(x1 - fold, y0 + g * 1.4), (x1 - fold, y0 + fold), (x1 - g * 1.4, y0 + fold)], fill=fold_ink))
    return steps


def text_lines(x: float, top: float, heading: tuple[float, float], lines: list[tuple[float, float]], gap: float, ink: str) -> list[Shape]:
    """A heading (width, thickness), then lines of text (width, thickness) under it, `gap` apart,
    from (x, top); round-ended, so they read as text."""
    out: list[Shape] = []
    w, t = heading
    out.append(RoundRect(x, top, x + w, top + t, t / 2, fill=ink))
    y = top + t + gap * 1.35
    for w, t in lines:
        out.append(RoundRect(x, y, x + w, y + t, t / 2, fill=ink))
        y += t + gap
    return out


# The large drawing (40 px and up), on the 16-unit grid.
PAGE_LARGE = (5.0, 3.85, 11.0, 12.15)
FOLD_LARGE = 1.7


def app_glyph_large() -> list[Step]:
    x0, y0, _, _ = PAGE_LARGE
    return [
        *page(PAGE_LARGE, FOLD_LARGE, BLUE_100, BLUE_300),
        *text_lines(x0 + 1.0, y0 + 1.25, (2.6, 1.0), [(4.0, 0.62), (4.0, 0.62), (2.7, 0.62)], 0.78, FACE_DEEP),
    ]


def app_icon_large() -> Icon:
    return Icon().add(
        Circle(8, 8, 7.375, fill=Gradient([(0, BLUE_300, 1), (0.55, BLUE_400, 1), (1, BLUE_600, 1)])),
        Circle(8, 8, 6.6875, stroke=Gradient([(0, WHITE, 0.35), (1, WHITE, 0)]), width=0.1875, opacity=0.6),
        Circle(8, 8, 5.625, fill=Gradient([(0, FACE, 1), (1, FACE_DEEP, 1)]), stroke=FACE_DEEP, width=0.5),
        *app_glyph_large(),
    )


# The small drawings (16 to 32 px): no gradient, gloss or face. A
# flat blue disc, the page in the deep face colour, and the lines cut through it in the disc's
# blue. Hand-tuned per size, in whole pixels, so the edges stay sharp: the page's box and fold;
# where the text starts, as an inset from the page's top left; the heading's width and thickness
# and the gap under it; then the lines' widths, thickness and gap. 16 and 24 px have room for a
# heading and two lines, 20 and 32 px for three.
SMALL_GLYPHS = {
    16: ((4, 3, 12, 13), 2, (2, 2), (3, 2, 1), ([4, 3], 1, 1)),
    20: ((5, 4, 15, 16), 3, (2, 2), (4, 2, 1), ([6, 6, 4], 1, 1)),
    24: ((6, 4, 18, 20), 4, (2, 3), (5, 2, 2), ([8, 6], 2, 2)),
    32: ((8, 4, 24, 28), 5, (3, 4), (7, 3, 3), ([10, 10, 7], 2, 2)),
}


def small_glyph(size: int) -> list[Step]:
    (px0, py0, px1, py1), fold, (ix, iy), (hw, ht, hgap), (widths, lt, gap) = SMALL_GLYPHS[size]
    u = GRID / size  # one pixel, in grid units
    x, y = (px0 + ix) * u, (py0 + iy) * u
    steps: list[Step] = list(page((px0 * u, py0 * u, px1 * u, py1 * u), fold * u, FACE_DEEP, None))
    # The lines, square-ended at these sizes: round ends blur into the page.
    steps.append(RoundRect(x, y, x + hw * u, y + ht * u, 0, fill=BLUE_500))
    y += (ht + hgap) * u
    for w in widths:
        steps.append(RoundRect(x, y, x + w * u, y + lt * u, 0, fill=BLUE_500))
        y += (lt + gap) * u
    return steps


def app_icon_small(size: int) -> Icon:
    return Icon().add(Circle(8, 8, 7.5, fill=BLUE_500), *small_glyph(size))


def app_icon(size: int) -> Icon:
    return app_icon_small(size) if size <= SMALL_UP_TO else app_icon_large()


# The .md file icon: Windows' document shape (a white page with a folded corner and a grey edge)
# with a small Legible disc at the bottom right, where Explorer's own overlays don't go (the
# shortcut arrow is at the bottom left). Per size, in pixels: the page's box, fold and edge; the
# disc's size; and below 32 px, the plain page the disc holds there (its box, in the icon's
# pixels). From 32 px up the disc is the app icon at a size of its own, on whole pixels, and the
# page has grey lines of text.
FILE_PAGES = {
    16: ((2, 0, 14, 16), 4, 1, 10, (9, 8, 13, 14)),
    20: ((3, 0, 17, 20), 5, 1, 12, (12, 11, 16, 17)),
    24: ((3, 0, 20, 24), 6, 1, 13, (15, 14, 20, 21)),
    32: ((5, 1, 27, 31), 7, 1, 16, None),
    40: ((6, 1, 34, 39), 9, 1, 20, None),
    48: ((7, 1, 41, 47), 11, 1, 24, None),
    64: ((10, 2, 54, 62), 14, 1.5, 32, None),
    256: ((40, 8, 216, 248), 56, 5, 128, None),
}


def file_icon(size: int) -> Icon:
    (px0, py0, px1, py1), fold, edge, n, tiny = FILE_PAGES[size]
    u = GRID / size  # one pixel, in grid units
    x0, y0, x1, y1, f, e = px0 * u, py0 * u, px1 * u, py1 * u, fold * u, edge * u
    r2 = math.sqrt(2)
    inner = [(x0 + e, y0 + e), (x1 - f + e - e * r2, y0 + e), (x1 - e, y0 + f - e + e * r2), (x1 - e, y1 - e), (x0 + e, y1 - e)]
    fold_inner = [(x1 - f + e, y0 + e + e * r2), (x1 - f + e, y0 + f - e), (x1 - e - e * r2, y0 + f - e)]
    sheet: list[Step] = [
        Polygon(dog_ear(x0, y0, x1, y1, f), fill=PAPER_EDGE),
        Polygon(inner, fill=PAPER),
        Polygon(fold_inner, fill=PAPER_FOLD),
    ]
    if size >= 32:
        lx, t = x0 + 3 * f / 7 + e, max(round(size / 24), 1) * u
        ly = y0 + f + 2 * t
        for w in [0.62, 0.45, 0.62, 0.3]:
            sheet.append(RoundRect(lx, ly, lx + (x1 - x0) * w, ly + t, t / 2, fill=PAPER_LINE))
            ly += 2.5 * t
    # The disc fills the bottom right corner, the app icon at n px, in a cut-out ring so it
    # stands apart from the page.
    scale = n / size
    dx = dy = (size - n) * u
    cx, cy = dx + 8 * scale, dy + 8 * scale
    if tiny:
        bx0, by0, bx1, by1 = tiny
        badge = [Circle(cx, cy, 7.5 * scale, fill=BLUE_500), RoundRect(bx0 * u, by0 * u, bx1 * u, by1 * u, 0, fill=FACE_DEEP)]
    else:
        badge = place(app_icon(n).steps, scale, dx, dy)
    ring = max(1, round(size / 20)) * u
    sheet.append(Erase([Circle(cx, cy, (7.5 if n <= SMALL_UP_TO else 7.375) * scale + ring)]))
    return Icon().add(Group(sheet), *badge)


# --- SVG ---------------------------------------------------------------------------------------


def fmt(v: float) -> str:
    return f"{v:.3f}".rstrip("0").rstrip(".")


class Svg:
    def __init__(self) -> None:
        self.defs: list[str] = []

    def paint(self, p: Paint) -> str:
        if p is None:
            return "none"
        if isinstance(p, str):
            return p
        n = len(self.defs) + 1
        stops = "".join(
            f'<stop offset="{fmt(o)}" stop-color="{c}"' + (f' stop-opacity="{fmt(a)}"' if a != 1 else "") + "/>"
            for o, c, a in p.stops
        )
        self.defs.append(f'<linearGradient id="g{n}" x1="0" y1="0" x2="0" y2="1">{stops}</linearGradient>')
        return f"url(#g{n})"

    def attrs(self, fill: Paint, stroke: Paint = None, width: float = 0.0, erase: bool = False) -> str:
        if erase:
            fill = "#000" if fill or not stroke else None
            stroke = "#000" if stroke else None
        out = f'fill="{self.paint(fill)}"'
        if stroke:
            out += f' stroke="{self.paint(stroke)}" stroke-width="{fmt(width)}"'
        return out

    def shape(self, shape: Shape, erase: bool = False) -> str:
        if isinstance(shape, RoundRect):
            x0, y0, x1, y1, r = shape.x0, shape.y0, shape.x1, shape.y1, shape.radius
            parts = []
            if shape.fill or erase and not shape.stroke:
                parts.append(
                    f'<rect x="{fmt(x0)}" y="{fmt(y0)}" width="{fmt(x1 - x0)}" height="{fmt(y1 - y0)}"'
                    + (f' rx="{fmt(r)}"' if r else "")
                    + f" {self.attrs(shape.fill, erase=erase)}/>"
                )
            if shape.stroke:
                h = shape.width / 2
                parts.append(
                    f'<rect x="{fmt(x0 + h)}" y="{fmt(y0 + h)}" width="{fmt(x1 - x0 - shape.width)}"'
                    f' height="{fmt(y1 - y0 - shape.width)}" rx="{fmt(max(r - h, 0))}"'
                    f" {self.attrs(None, shape.stroke, shape.width, erase)}/>"
                )
            return "".join(parts)
        if isinstance(shape, Circle):
            opacity = f' opacity="{fmt(shape.opacity)}"' if shape.opacity != 1 else ""
            return (
                f'<circle cx="{fmt(shape.cx)}" cy="{fmt(shape.cy)}" r="{fmt(shape.r)}"'
                f" {self.attrs(shape.fill, shape.stroke, shape.width, erase)}{opacity}/>"
            )
        if isinstance(shape, Polygon):
            points = " ".join(f"{fmt(x)},{fmt(y)}" for x, y in shape.points)
            return f'<polygon points="{points}" {self.attrs(shape.fill, erase=erase)}/>'
        raise TypeError(shape)


def svg_body(svg: Svg, steps: list[Step], masks: list[str]) -> str:
    body = ""
    for step in steps:
        if isinstance(step, Group):
            body += f"<g>{svg_body(svg, step.steps, masks)}</g>"
        elif isinstance(step, Erase):
            n = len(masks) + 1
            cut = "".join(svg.shape(s, erase=True) for s in step.shapes)
            masks.append(
                f'<mask id="cut{n}" maskUnits="userSpaceOnUse" x="0" y="0" width="16" height="16">'
                f'<rect width="16" height="16" fill="#fff"/>{cut}</mask>'
            )
            body = f'<g mask="url(#cut{n})">{body}</g>'
        else:
            body += svg.shape(step)
    return body


def to_svg(icon: Icon, what: str) -> str:
    svg = Svg()
    masks: list[str] = []
    body = svg_body(svg, icon.steps, masks)
    defs = svg.defs + masks
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 16 16">\n'
        f"<!-- Legible {what} (MIT). Generated by assets/export.py: edit that, not this. -->\n"
        f"{'<defs>' + ''.join(defs) + '</defs>' if defs else ''}{body}\n</svg>\n"
    )


# --- Pixels ------------------------------------------------------------------------------------


def rgb(colour: str) -> tuple[int, int, int]:
    c = colour.lstrip("#")
    return int(c[0:2], 16), int(c[2:4], 16), int(c[4:6], 16)


def supersample(size: int) -> int:
    """How many times bigger a size is drawn before it's reduced: less for the big ones, which
    have pixels enough."""
    return 8 if size <= 64 else 4 if size <= 256 else 2


def fill_image(paint: str | Gradient, top: float, bottom: float, big: int) -> Image.Image:
    """`paint` over the whole canvas; a gradient runs from `top` to `bottom` (pixels)."""
    if isinstance(paint, str):
        return Image.new("RGBA", (big, big), (*rgb(paint), 255))
    column = Image.new("RGBA", (1, big))
    stops = paint.stops
    for y in range(big):
        t = min(max((y + 0.5 - top) / max(bottom - top, 1e-6), 0.0), 1.0)
        for (o0, c0, a0), (o1, c1, a1) in zip(stops, stops[1:]):
            if t <= o1 or (o1, c1, a1) == stops[-1]:
                f = 0.0 if o1 == o0 else min(max((t - o0) / (o1 - o0), 0.0), 1.0)
                r0, r1 = rgb(c0), rgb(c1)
                colour = tuple(round(r0[i] + (r1[i] - r0[i]) * f) for i in range(3))
                column.putpixel((0, y), (*colour, round(255 * (a0 + (a1 - a0) * f))))
                break
    return column.resize((big, big), Image.Resampling.NEAREST)


def parts(shape: Shape, k: float, big: int) -> list[tuple[Image.Image, Paint, float, float]]:
    """Each painted part of `shape` as (its coverage mask, its paint, and the top and bottom of
    the box a gradient spans), scaled by `k` pixels per grid unit."""

    def mask() -> tuple[Image.Image, ImageDraw.ImageDraw]:
        m = Image.new("L", (big, big), 0)
        return m, ImageDraw.Draw(m)

    out = []
    if isinstance(shape, RoundRect):
        box = [round(shape.x0 * k), round(shape.y0 * k), round(shape.x1 * k) - 1, round(shape.y1 * k) - 1]
        if shape.fill:
            m, d = mask()
            d.rounded_rectangle(box, radius=shape.radius * k, fill=255)
            out.append((m, shape.fill, box[1], box[3]))
        if shape.stroke:
            m, d = mask()
            d.rounded_rectangle(box, radius=shape.radius * k, outline=255, width=round(shape.width * k))
            out.append((m, shape.stroke, box[1], box[3]))
    elif isinstance(shape, Circle):
        box = [(shape.cx - shape.r) * k, (shape.cy - shape.r) * k, (shape.cx + shape.r) * k, (shape.cy + shape.r) * k]
        if shape.fill:
            m, d = mask()
            d.ellipse([box[0], box[1], box[2] - 1, box[3] - 1], fill=255)
            out.append((m, shape.fill, box[1], box[3]))
        if shape.stroke:
            m, d = mask()
            h = shape.width * k / 2
            d.ellipse([box[0] - h, box[1] - h, box[2] + h - 1, box[3] + h - 1], fill=255)
            d.ellipse([box[0] + h, box[1] + h, box[2] - h - 1, box[3] - h - 1], fill=0)
            out.append((m, shape.stroke, box[1], box[3]))
    elif isinstance(shape, Polygon):
        m, d = mask()
        d.polygon([(x * k, y * k) for x, y in shape.points], fill=255)
        ys = [y * k for _, y in shape.points]
        out.append((m, shape.fill, min(ys), max(ys)))
    else:
        raise TypeError(shape)
    return out


def draw_steps(steps: list[Step], big: int, k: float) -> Image.Image:
    canvas = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    for step in steps:
        if isinstance(step, Group):
            canvas = Image.alpha_composite(canvas, draw_steps(step.steps, big, k))
            continue
        if isinstance(step, Erase):
            cut = Image.new("L", (big, big), 0)
            for shape in step.shapes:
                for m, *_ in parts(shape, k, big):
                    cut = Image.composite(Image.new("L", (big, big), 255), cut, m)
            alpha = Image.composite(Image.new("L", (big, big), 0), canvas.getchannel("A"), cut)
            canvas.putalpha(alpha)
            continue
        opacity = step.opacity if isinstance(step, Circle) else 1.0
        for m, paint, top, bottom in parts(step, k, big):
            layer = fill_image(paint, top, bottom, big)
            a = Image.composite(layer.getchannel("A"), Image.new("L", (big, big), 0), m)
            if opacity != 1.0:
                a = a.point(lambda v: round(v * opacity))
            layer.putalpha(a)
            canvas = Image.alpha_composite(canvas, layer)
    return canvas


def render(icon: Icon, size: int) -> Image.Image:
    big = size * supersample(size)
    return draw_steps(icon.steps, big, big / GRID).resize((size, size), Image.Resampling.LANCZOS)


def save_ico(path: Path, draw, sizes: list[int]) -> dict[int, Image.Image]:
    images = {s: draw(s) for s in sizes}
    largest = images[sizes[-1]]
    largest.save(path, format="ICO", sizes=[(s, s) for s in sizes], append_images=[images[s] for s in sizes[:-1]])
    return images


# --- Contrast ----------------------------------------------------------------------------------


def contrast(a: str, b: str) -> float:
    """WCAG 2's contrast ratio between two opaque colours."""

    def lum(c: str) -> float:
        v = [x / 255 for x in rgb(c)]
        v = [x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4 for x in v]
        return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]

    la, lb = lum(a), lum(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)


LIGHT_TASKBAR = "#F3F3F3"
DARK_TASKBAR = "#202020"


# --- The preview sheet -------------------------------------------------------------------------


def font(size: int) -> ImageFont.ImageFont:
    for name in ["segoeui.ttf", "arial.ttf", "DejaVuSans.ttf"]:
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default()


def preview(rows: list[tuple[str, dict[int, Image.Image]]]) -> Image.Image:
    """Each icon at every .ico size, actual pixels, on a light and a dark background, then its
    16 and 32 px enlarged four times so their pixels can be checked."""
    row_h = 300
    sheet = Image.new("RGB", (1240, 90 + len(rows) * 2 * row_h), "#FAFAFA")
    d = ImageDraw.Draw(sheet)
    title, label, small = font(26), font(16), font(13)

    d.text((30, 20), "Legible's icons", fill="#111", font=title)
    d.text((30, 58), "Actual pixels at 16, 20, 24, 32, 40, 48, 64 and 256 px; then 16 and 32 px at 400%", fill="#444", font=label)

    y = 90
    for name, images in rows:
        for bg, fg in [(LIGHT_TASKBAR, "#333"), (DARK_TASKBAR, "#DDD")]:
            d.rectangle([30, y, 1210, y + row_h - 10], fill=bg)
            d.text((40, y + 8), f"{name}, on {'light' if bg == LIGHT_TASKBAR else 'dark'}", fill=fg, font=small)
            x = 50
            base = y + 40 + 256
            for s in ICO_SIZES:
                sheet.paste(images[s], (x, base - s), images[s])
                d.text((x, base + 6), f"{s}", fill=fg, font=small)
                x += s + 26
            for s in [16, 32]:
                zoomed = images[s].resize((s * 4, s * 4), Image.Resampling.NEAREST)
                sheet.paste(zoomed, (x, base - s * 4), zoomed)
                d.text((x, base + 6), f"{s} ×4", fill=fg, font=small)
                x += s * 4 + 26
            y += row_h
    return sheet


# --- Main --------------------------------------------------------------------------------------


def main() -> int:
    argparse.ArgumentParser(description=__doc__.splitlines()[0]).parse_args()

    icon_dir = ASSETS / "icon"
    file_dir = ASSETS / "file"
    icon_dir.mkdir(exist_ok=True)
    file_dir.mkdir(exist_ok=True)

    (icon_dir / "legible.svg").write_text(to_svg(app_icon_large(), "icon"), encoding="utf-8", newline="\n")
    for s in [16, 32]:
        (icon_dir / f"legible-{s}.svg").write_text(to_svg(app_icon_small(s), f"icon at {s} px"), encoding="utf-8", newline="\n")
    app = save_ico(icon_dir / "legible.ico", lambda s: render(app_icon(s), s), ICO_SIZES)
    for s in PNG_SIZES:
        image = app.get(s) or render(app_icon(s), s)
        image.save(icon_dir / f"legible-{s}.png", optimize=True)

    (file_dir / "markdown.svg").write_text(to_svg(file_icon(256), "Markdown file icon"), encoding="utf-8", newline="\n")
    (file_dir / "markdown-16.svg").write_text(to_svg(file_icon(16), "Markdown file icon at 16 px"), encoding="utf-8", newline="\n")
    md = save_ico(file_dir / "markdown.ico", lambda s: render(file_icon(s), s), ICO_SIZES)

    preview([("The app icon", app), ("The .md file icon", md)]).save(ASSETS / "preview.png", optimize=True)

    print(f"Wrote the icons to {icon_dir}, {file_dir} and {ASSETS / 'preview.png'}")
    print("Contrast (WCAG 2):")
    print(f"  the small icon's disc {BLUE_500} on a light taskbar: {contrast(BLUE_500, LIGHT_TASKBAR):.2f}:1")
    print(f"  on a dark taskbar: {contrast(BLUE_500, DARK_TASKBAR):.2f}:1")
    print(f"  the small icon's page {FACE_DEEP} on the disc: {contrast(FACE_DEEP, BLUE_500):.2f}:1")
    print(f"  the large icon's page {BLUE_100} on the face {FACE}: {contrast(BLUE_100, FACE):.2f}:1")
    print(f"  the lines {FACE_DEEP} on the large icon's page: {contrast(FACE_DEEP, BLUE_100):.2f}:1")
    return 0


if __name__ == "__main__":
    sys.exit(main())
