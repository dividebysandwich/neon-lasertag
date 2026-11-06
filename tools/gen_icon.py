#!/usr/bin/env python3
"""Draws the application icon (a neon crosshair on black) for the Linux packages.

    python3 tools/gen_icon.py [packaging/linux/lasertag.png]

Needs Pillow. The glow is the crosshair blurred at a few radii and added under the sharp lines.
"""

import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter

SIZE = 256
SS = 4  # supersampling
CYAN = (40, 240, 255)
RED = (255, 40, 90)


def crosshair(scale):
    s = SIZE * scale
    img = Image.new("RGB", (s, s), (0, 0, 0))
    d = ImageDraw.Draw(img)
    c = s / 2
    w = 7 * scale
    r = 74 * scale
    d.ellipse([c - r, c - r, c + r, c + r], outline=CYAN, width=w)
    gap, end = 26 * scale, 112 * scale
    for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
        d.line([c + dx * gap, c + dy * gap, c + dx * end, c + dy * end], fill=CYAN, width=w)
    dot = 9 * scale
    d.ellipse([c - dot, c - dot, c + dot, c + dot], fill=RED)
    return img


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "packaging/linux/lasertag.png"
    sharp = crosshair(SS).resize((SIZE, SIZE), Image.LANCZOS)
    img = Image.new("RGB", (SIZE, SIZE), (0, 0, 0))
    for radius in (14, 6, 2):
        img = ImageChops.add(img, sharp.filter(ImageFilter.GaussianBlur(radius)))
    img = ImageChops.add(img, sharp)
    img.save(out)
    print(out)


if __name__ == "__main__":
    main()
