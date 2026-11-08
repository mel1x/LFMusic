# Draws the README banner: the logo, the name and what it does on the left, the catalog on the right,
# running off the edge.
#   docs/banner.png
# python docs/tools/make-banner.py  (needs Pillow; run docs/tools/screens.mjs first)
import os

from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..", "..")
DOCS = os.path.join(ROOT, "docs")
FONTS = os.path.join(ROOT, "src", "assets", "fonts")

W, H = 1800, 760
SS = 2   # drawn at twice the size, then scaled down for smooth edges

BG = (16, 11, 14)
ACCENT = (0xE0, 0x6A, 0x93)
TEXT = (245, 245, 247)
DIM = (168, 160, 165)


def font(name, size, weight):
    f = ImageFont.truetype(os.path.join(FONTS, name), size * SS)
    axes = f.get_variation_axes()
    f.set_variation_by_axes([weight if a["name"] == b"Weight" else a["default"] for a in axes])
    return f


def s(v):
    return int(round(v * SS))


def glow(size, box, color, alpha, blur):
    # a soft light: an ellipse blurred far past its edge
    layer = Image.new("RGBA", size, color + (0,))
    ImageDraw.Draw(layer).ellipse(box, fill=color + (alpha,))
    return layer.filter(ImageFilter.GaussianBlur(blur))


def rounded_mask(size, radius):
    m = Image.new("L", size, 0)
    ImageDraw.Draw(m).rounded_rectangle((0, 0, size[0] - 1, size[1] - 1), radius, fill=255)
    return m


def main():
    size = (s(W), s(H))
    img = Image.new("RGBA", size, BG + (255,))

    # two soft lights in the accent color: behind the name and behind the window
    img.alpha_composite(glow(size, (s(-200), s(-260), s(700), s(520)), ACCENT, 30, s(140)))
    img.alpha_composite(glow(size, (s(900), s(120), s(1900), s(900)), ACCENT, 44, s(170)))

    # the app, scaled down, with rounded corners and a shadow, cut off at the right and bottom
    shot = Image.open(os.path.join(DOCS, "screens", "catalog.png")).convert("RGBA")
    sw = s(1100)
    sh = int(shot.height * sw / shot.width)
    shot = shot.resize((sw, sh), Image.LANCZOS)
    rad = s(18)
    sx, sy = s(820), s(110)
    shadow = Image.new("RGBA", size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle((sx, sy + s(24), sx + sw, sy + sh + s(24)), rad, fill=(0, 0, 0, 180))
    img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(s(36))))
    framed = Image.new("RGBA", (sw, sh), (0, 0, 0, 0))
    framed.paste(shot, (0, 0), rounded_mask((sw, sh), rad))
    img.alpha_composite(framed, (sx, sy))

    # left: the logo, the name, the line under it and the main features
    d = ImageDraw.Draw(img)
    lx, ly = s(110), s(178)
    logo = Image.open(os.path.join(ROOT, "src-tauri", "icons", "icon.png")).convert("RGBA").resize((s(104), s(104)), Image.LANCZOS)
    img.alpha_composite(logo, (lx, ly))
    d.text((lx - s(4), ly + s(132)), "LFMusic", font=font("montserrat-latin-wght-normal.woff2", 96, 800), fill=TEXT)
    d.text((lx, ly + s(268)), "A fast player for your local music", font=font("InterVariable.woff2", 30, 400), fill=DIM)

    chips = ["Offline", "My mix", "Lossless", "Android"]
    cf = font("InterVariable.woff2", 20, 600)
    cx, cy, ch = lx, ly + s(330), s(44)
    for label in chips:
        tw = d.textlength(label, font=cf)
        w = int(tw) + s(48)
        pill = Image.new("RGBA", size, (0, 0, 0, 0))
        ImageDraw.Draw(pill).rounded_rectangle((cx, cy, cx + w, cy + ch), ch // 2, fill=(255, 255, 255, 16))
        img.alpha_composite(pill)
        d = ImageDraw.Draw(img)
        dot = s(8)
        d.ellipse((cx + s(18), cy + (ch - dot) // 2, cx + s(18) + dot, cy + (ch + dot) // 2), fill=ACCENT)
        d.text((cx + s(34), cy + ch // 2), label, font=cf, fill=TEXT, anchor="lm")
        cx += w + s(12)

    img = img.resize((W, H), Image.LANCZOS)
    out = os.path.join(DOCS, "banner.png")
    img.convert("RGB").save(out, optimize=True)
    print(out)


if __name__ == "__main__":
    main()
