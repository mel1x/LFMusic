# Draws the README banner: the logo, the name and one line on the left, the desktop album page on the right,
# and the phone in front of the window's lower left corner.
#   docs/banner.png
# python docs/tools/make-banner.py  (needs Pillow; run docs/tools/screens.mjs first)
import os

from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..", "..")
DOCS = os.path.join(ROOT, "docs")
SCREENS = os.path.join(DOCS, "screens")
FONTS = os.path.join(ROOT, "src", "assets", "fonts")

W, H = 1800, 760
SS = 2   # drawn at twice the size, then scaled down for smooth edges
SIZE = (W * SS, H * SS)

BG = (12, 9, 11)
ACCENT = (0xBD, 0x38, 0x71)
TEXT = (247, 247, 249)
DIM = (176, 172, 180)


def s(v):
    return int(round(v * SS))


def font(name, size, weight):
    f = ImageFont.truetype(os.path.join(FONTS, name), size * SS)
    f.set_variation_by_axes([weight if a["name"] == b"Weight" else a["default"] for a in f.get_variation_axes()])
    return f


def rounded(img, radius):
    m = Image.new("L", img.size, 0)
    ImageDraw.Draw(m).rounded_rectangle((0, 0, img.size[0] - 1, img.size[1] - 1), radius, fill=255)
    out = Image.new("RGBA", img.size, (0, 0, 0, 0))
    out.paste(img, (0, 0), m)
    return out


def screen(name, width, radius):
    im = Image.open(os.path.join(SCREENS, name)).convert("RGBA")
    im = im.resize((width, int(im.height * width / im.width)), Image.LANCZOS)
    return rounded(im, radius)


def glow(box, alpha, blur):
    layer = Image.new("RGBA", SIZE, ACCENT + (0,))
    ImageDraw.Draw(layer).ellipse(box, fill=ACCENT + (alpha,))
    return layer.filter(ImageFilter.GaussianBlur(blur))


def place(img, im, x, y, radius, alpha=190, blur=34, drop=22):
    # a soft shadow under the image, then the image
    sh = Image.new("RGBA", SIZE, (0, 0, 0, 0))
    ImageDraw.Draw(sh).rounded_rectangle((x, y + s(drop), x + im.width, y + im.height + s(drop)), radius, fill=(0, 0, 0, alpha))
    img.alpha_composite(sh.filter(ImageFilter.GaussianBlur(s(blur))))
    img.alpha_composite(im, (x, y))


def main():
    img = Image.new("RGBA", SIZE, BG + (255,))
    img.alpha_composite(glow((s(800), s(0), s(1900), s(900)), 60, s(170)))

    # left: the logo, the name and what it is
    d = ImageDraw.Draw(img)
    lx, ly = s(110), s(180)
    logo = Image.open(os.path.join(ROOT, "src-tauri", "icons", "icon.png")).convert("RGBA").resize((s(84), s(84)), Image.LANCZOS)
    img.alpha_composite(logo, (lx, ly))
    d.text((lx - s(4), ly + s(120)), "LFMusic", font=font("montserrat-latin-wght-normal.woff2", 96, 800), fill=TEXT)
    body = font("InterVariable.woff2", 30, 400)
    d.text((lx, ly + s(260)), "A fast, offline player for", font=body, fill=DIM)
    d.text((lx, ly + s(300)), "your own music collection", font=body, fill=DIM)

    # right: the desktop window running off the edges, the phone in front of it
    window = screen("album.png", s(1060), s(18))
    place(img, window, s(820), s(120), s(18))
    phone = screen("phone-album.png", s(232), s(16))
    place(img, phone, s(700), s(240), s(16), 220, 34, 22)

    out = os.path.join(DOCS, "banner.png")
    img.resize((W, H), Image.LANCZOS).convert("RGB").save(out, optimize=True)
    print(out)


if __name__ == "__main__":
    main()
