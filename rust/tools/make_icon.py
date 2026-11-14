"""Draws Adventure Pad's icon: a pad (a D-pad and two buttons) in the app's yellow on its dark purple.

    uv run --with pillow python rust/tools/make_icon.py

Writes rust/crates/adventurepad/assets/icon.png (256 px, the window's icon) and icon.ico (16 to 256 px, the exe's).
Drawn 4x larger and scaled down, for smooth edges.
"""

from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parents[1] / "crates" / "adventurepad" / "assets"

# The app's style (adventurepad-sdk's style module): ACCENT on CARD, a little lighter at the top.
ACCENT = (0xFF, 0xD8, 0x4A, 255)
ACCENT_DARK = (0xC9, 0x9E, 0x1E, 255)
BG_TOP = (0x3A, 0x33, 0x4C, 255)
BG_BOTTOM = (0x22, 0x1F, 0x2B, 255)
RIM = (0x55, 0x4C, 0x6E, 255)

S = 1024  # drawn at, then scaled to 256


def draw() -> Image.Image:
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    # The rounded square, with a top-to-bottom shade.
    shade = Image.new("RGBA", (S, S))
    sd = ImageDraw.Draw(shade)
    for y in range(S):
        t = y / (S - 1)
        sd.line([(0, y), (S, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(BG_TOP, BG_BOTTOM)))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([24, 24, S - 24, S - 24], radius=210, fill=255)
    img.paste(shade, (0, 0), mask)
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([24, 24, S - 24, S - 24], radius=210, outline=RIM, width=18)

    # The D-pad, left of centre: a plus with rounded arms, a darker edge under it.
    cx, cy, arm, w = 330, 540, 175, 118
    for dx, dy, col in ((0, 14, ACCENT_DARK), (0, 0, ACCENT)):
        d.rounded_rectangle([cx - w / 2 + dx, cy - arm + dy, cx + w / 2 + dx, cy + arm + dy], radius=28, fill=col)
        d.rounded_rectangle([cx - arm + dx, cy - w / 2 + dy, cx + arm + dx, cy + w / 2 + dy], radius=28, fill=col)
    d.ellipse([cx - 26, cy - 26, cx + 26, cy + 26], fill=ACCENT_DARK)

    # Two buttons on the right, on a slant (A higher, B lower), as on a pad.
    for bx, by in ((735, 450), (610, 610)):
        r = 82
        d.ellipse([bx - r, by - r + 14, bx + r, by + r + 14], fill=ACCENT_DARK)
        d.ellipse([bx - r, by - r, bx + r, by + r], fill=ACCENT)
    return img.resize((256, 256), Image.LANCZOS)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    icon = draw()
    icon.save(OUT / "icon.png")
    icon.save(OUT / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    print(f"wrote {OUT / 'icon.png'} and icon.ico")


if __name__ == "__main__":
    main()
