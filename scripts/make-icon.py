#!/usr/bin/env python3
"""Generate the placeholder Margin app icon as a macOS .iconset.

    python3 scripts/make-icon.py            # writes packaging/macos/AppIcon.iconset/

The design (a commit graph with a note card in its margin) is a stand-in;
replace the PNGs with a designer's artwork any time — the packaging script
only needs the iconset. Requires Pillow (pip install pillow).
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

S = 1024
BG_TOP, BG_BOTTOM = (0x2A, 0x2E, 0x37), (0x17, 0x19, 0x1E)
ACCENT = (0xE5, 0xB5, 0x67)
LANES = [(0x5A, 0xB0, 0xF0), (0x9A, 0xD0, 0x6C), (0xD0, 0x7C, 0xE0)]
PAPER, INK = (0xF4, 0xEF, 0xE4), (0x8A, 0x84, 0x78)


def rounded_mask(size, box, radius):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle(box, radius, fill=255)
    return m


def draw() -> Image.Image:
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    # macOS (Big Sur+) icon grid: 824px rounded square centred, soft shadow.
    box = (100, 100, 924, 924)
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle((100, 118, 924, 942), 185, fill=(0, 0, 0, 150))
    img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(18)))

    grad = Image.new("RGBA", (S, S))
    gd = ImageDraw.Draw(grad)
    for y in range(S):
        t = y / (S - 1)
        gd.line([(0, y), (S, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(BG_TOP, BG_BOTTOM)) + (255,))
    img.paste(grad, (0, 0), rounded_mask(S, box, 185))

    d = ImageDraw.Draw(img)
    w = 30  # line width
    # Commit graph: a main lane with a branch that merges back.
    x0, x1 = 300, 430
    d.line([(x0, 230), (x0, 800)], fill=LANES[0], width=w)
    d.line([(x0, 330), (x1, 440), (x1, 600), (x0, 700)], fill=LANES[1], width=w, joint="curve")
    for (x, y, c) in [(x0, 250, LANES[0]), (x0, 330, LANES[0]), (x1, 470, LANES[1]), (x1, 580, LANES[1]), (x0, 700, LANES[0]), (x0, 790, LANES[0])]:
        d.ellipse((x - 34, y - 34, x + 34, y + 34), fill=c, outline=BG_BOTTOM, width=10)

    # Note card in the margin, tied to a commit by a dashed leader.
    card = (540, 380, 840, 690)
    cs = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(cs).rounded_rectangle((card[0] + 6, card[1] + 14, card[2] + 6, card[3] + 14), 34, fill=(0, 0, 0, 120))
    img.alpha_composite(cs.filter(ImageFilter.GaussianBlur(10)))
    d.rounded_rectangle(card, 34, fill=PAPER)
    d.rectangle((card[0], card[1] + 40, card[0] + 22, card[3] - 40), fill=ACCENT)  # accent spine
    for i, frac in enumerate([0.78, 0.62, 0.70, 0.45]):
        y = card[1] + 75 + i * 58
        d.rounded_rectangle((card[0] + 60, y, card[0] + 60 + int(200 * frac), y + 18), 9, fill=INK)
    for x in range(x1 + 50, card[0] - 10, 34):
        d.line([(x, 525), (x + 16, 525)], fill=ACCENT, width=10)
    return img


def main():
    out = Path(__file__).resolve().parent.parent / "packaging/macos/AppIcon.iconset"
    out.mkdir(parents=True, exist_ok=True)
    base = draw()
    for size in (16, 32, 128, 256, 512):
        base.resize((size, size), Image.LANCZOS).save(out / f"icon_{size}x{size}.png")
        base.resize((size * 2, size * 2), Image.LANCZOS).save(out / f"icon_{size}x{size}@2x.png")
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
