"""Draws the Multiplayer glyph for the top bar of the game's main menu.

    python tools/art/icons/menu_icon.py

The main menu's top-bar icons (settings_50, achievements_50, credits_50,
quit_50 in the game's gui/menu/icons) are 100 by 100 greyscale TGAs: a
solid, rounded white glyph on black, which the game tints. This draws the
same three people as the Multiplayer button of icons.html in that form, as
menu_multiplayer_50@2x.tga (100 px) and menu_multiplayer_50.tga (50 px),
into the TPF3-MP mod. Needs Pillow. The TGAs are committed; run this again
only after changing the drawing.
"""

import pathlib

from PIL import Image, ImageDraw

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
OUT = ROOT / "mod" / "tpf3mp_1" / "content" / "gui" / "tpf3mp" / "icons"

# Drawn on a 24-unit grid, as icons.html's symbols are, at SCALE px a unit,
# then scaled down: smooth edges without anti-aliasing primitives.
SCALE = 64
# The game's glyphs span about 72% of their square (settings_50 and
# profile_50: 72 of 100), centred.
GLYPH = 0.72


def people(size):
    """The three people, white on black, `size` px square."""
    units = 24
    big = Image.new("L", (units * SCALE, units * SCALE), 0)
    draw = ImageDraw.Draw(big)

    def u(value):
        return round(value * SCALE)

    def circle(cx, cy, r, fill=255):
        draw.ellipse((u(cx - r), u(cy - r), u(cx + r), u(cy + r)), fill=fill)

    def body(cx, top, half, bottom, fill=255):
        # A rounded shoulder line down to a flat base.
        draw.ellipse((u(cx - half), u(top), u(cx + half), u(top + 2 * half)), fill=fill)
        draw.rectangle((u(cx - half), u(top + half), u(cx + half), u(bottom)), fill=fill)

    # The two behind, left and right.
    for cx in (5.0, 19.0):
        circle(cx, 9.0, 2.6)
        body(cx, 13.0, 4.2, 19.6)
    # A gap around the one in front, so the three read apart.
    gap = 1.1
    circle(12.0, 7.4, 3.5 + gap, fill=0)
    body(12.0, 12.6 - gap, 6.4 + gap, 21.0, fill=0)
    circle(12.0, 7.4, 3.5)
    body(12.0, 12.6, 6.4, 21.0)

    big = big.crop(big.getbbox())
    fit = size * GLYPH / max(big.size)
    glyph = big.resize((round(big.width * fit), round(big.height * fit)), Image.LANCZOS)
    out = Image.new("L", (size, size), 0)
    out.paste(glyph, ((size - glyph.width) // 2, (size - glyph.height) // 2))
    return out


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name, size in (("menu_multiplayer_50@2x.tga", 100), ("menu_multiplayer_50.tga", 50)):
        people(size).save(OUT / name)
        print(f"wrote {(OUT / name).relative_to(ROOT)}")


if __name__ == "__main__":
    main()
