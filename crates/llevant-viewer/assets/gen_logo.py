"""Draws the llevant logo: a golden wind rose (llevant is the east wind).

Writes, next to this script:

- logo.png (512 px): the whole rose;
- logo-ring.png (256 px): the ring and its ticks, which stay still in the spinner;
- logo-star.png (256 px): the eight-pointed star, which turns.

The ring and the star leave a gap between them, so that the star can turn
inside the ring without touching it. Drawn four times larger and reduced,
for smooth edges. Needs Pillow:

    python crates/llevant-viewer/assets/gen_logo.py
"""

import math
from pathlib import Path

from PIL import Image, ImageDraw

SS = 4  # supersampling
SIZE = 512 * SS
C = SIZE / 2

GOLD = (253, 198, 0, 255)  # the yellow of the earlier logo
DEEP = (196, 140, 0, 255)  # its shaded half, a deeper gold

RING = (212, 238)  # inner and outer radius of the ring
TICK_MAJOR = 190  # inner end of the ticks at the eight winds
TICK_MINOR = 202  # and of the others
STAR_CARDINAL = 182  # tip of the four long points
STAR_ORDINAL = 124  # tip of the four short ones
STAR_WAIST = 0.21  # half-width of a point at its base, over its length
HOLE = 13  # the hole at the centre


def s(v):
    return v * SS


def polar(r, a):
    # a in turns from north, clockwise; y grows downwards
    t = a * math.tau
    return (C + s(r) * math.sin(t), C - s(r) * math.cos(t))


def ring(draw):
    ro, ri = s(RING[1]), s(RING[0])
    draw.ellipse([C - ro, C - ro, C + ro, C + ro], fill=GOLD)
    draw.ellipse([C - ri, C - ri, C + ri, C + ri], fill=(0, 0, 0, 0))
    for k in range(32):
        a = k / 32
        major = k % 4 == 0
        inner = TICK_MAJOR if major else TICK_MINOR
        half = (5.5 if major else 2.5) / RING[0] / math.tau  # half-width in turns
        tip = 0.6 / inner / math.tau if major else half
        draw.polygon(
            [
                polar(RING[0] + 2, a - half),
                polar(RING[0] + 2, a + half),
                polar(inner, a + tip),
                polar(inner, a - tip),
            ],
            fill=GOLD,
        )


def star(draw):
    # the short points first, under the long ones
    for length, offset in ((STAR_ORDINAL, 1 / 8), (STAR_CARDINAL, 0.0)):
        waist = length * STAR_WAIST
        for k in range(4):
            a = offset + k / 4
            tip = polar(length, a)
            # each point in two halves, lit on its clockwise side
            draw.polygon([(C, C), tip, polar(waist, a + 1 / 8)], fill=GOLD)
            draw.polygon([(C, C), tip, polar(waist, a - 1 / 8)], fill=DEEP)
    draw.ellipse([C - s(HOLE), C - s(HOLE), C + s(HOLE), C + s(HOLE)], fill=(0, 0, 0, 0))


def image(*parts):
    im = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(im)
    for part in parts:
        part(draw)
    return im


def reduce(im, side):
    # premultiply, so that transparent pixels carry no colour into the edges
    return im.convert("RGBa").resize((side, side), Image.Resampling.BOX).convert("RGBA")


def main():
    here = Path(__file__).parent
    reduce(image(ring, star), 512).save(here / "logo.png", optimize=True)
    reduce(image(ring), 256).save(here / "logo-ring.png", optimize=True)
    reduce(image(star), 256).save(here / "logo-star.png", optimize=True)


if __name__ == "__main__":
    main()
