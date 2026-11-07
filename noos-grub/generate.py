#!/usr/bin/env python3
"""Génère le thème GRUB NOOS (palette Nuit saphir) dans noos-grub/noos/.

- background.png : dégradé sombre et halo aux couleurs du logo ;
- logo.png       : logo NOOS (repris du thème Plymouth) ;
- menu_*.png     : panneau du menu (9 morceaux, coins arrondis, translucide) ;
- select_*.png   : pastille de l'entrée sélectionnée (accent #89b4fa) ;
- *.pf2          : polices Noto Sans converties par grub2-mkfont (« NOOS Regular 20 »…).

Les fichiers produits sont versionnés : seul ce script a besoin de cairo et grub2-mkfont.

    python3 noos-grub/generate.py
"""

import math
import os
import shutil
import subprocess

import importlib.util
import sys

import cairo

sys.dont_write_bytecode = True  # pas de __pycache__ dans noos-plymouth/

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "noos")

# Tracés du logo : ceux du thème Plymouth (même dessin que data/noos-logo.svg).
_spec = importlib.util.spec_from_file_location("plymouth", os.path.join(HERE, "..", "noos-plymouth", "generate.py"))
plymouth = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(plymouth)
FONTS = "/usr/share/fonts/google-noto"

W, H = 1920, 1080
BG_TOP = (0x11, 0x11, 0x1B)
BG_BOTTOM = (0x1E, 0x1E, 0x2E)
PANEL = (0x1E, 0x1E, 0x2E, 0.78)
PANEL_BORDER = (0xCD, 0xD6, 0xF4, 0.08)
ACCENT = (0x89, 0xB4, 0xFA, 1.0)
HALO = [(0x89, 0xB4, 0xFA), (0xCB, 0xA6, 0xF7), (0xF3, 0x8B, 0xA8)]

# Plages de caractères des polices : latin (accents français), ponctuation, flèches.
RANGES = "0x20-0x7E,0xA0-0x17F,0x2010-0x2027"


def rgb(c):
    return tuple(x / 255 for x in c[:3])


def background():
    s = cairo.ImageSurface(cairo.FORMAT_RGB24, W, H)
    cr = cairo.Context(s)
    g = cairo.LinearGradient(0, 0, 0, H)
    g.add_color_stop_rgb(0, *rgb(BG_TOP))
    g.add_color_stop_rgb(1, *rgb(BG_BOTTOM))
    cr.set_source(g)
    cr.paint()
    # Halos diffus (bleu, violet, rose) derrière le logo et le menu.
    for (x, y, r), c in zip([(0.30, 0.30, 700), (0.70, 0.36, 620), (0.50, 0.78, 760)], HALO):
        rg = cairo.RadialGradient(W * x, H * y, 0, W * x, H * y, r)
        rg.add_color_stop_rgba(0, *rgb(c), 0.10)
        rg.add_color_stop_rgba(1, *rgb(c), 0.0)
        cr.set_source(rg)
        cr.paint()
    s.write_to_png(os.path.join(OUT, "background.png"))


def logo():
    """Logo NOOS : lettres claires et symbole infini en dégradé (bleu → violet → rose)."""
    scale = 1.0
    # Le dessin occupe x ∈ [80, 405], y ∈ [38, 122] dans le repère 480×160 du SVG.
    x0, y0, w0, h0 = 70, 28, 345, 104
    w, h = int(w0 * scale), int(h0 * scale)
    s = cairo.ImageSurface(cairo.FORMAT_ARGB32, w, h)
    cr = cairo.Context(s)
    cr.scale(scale, scale)
    cr.translate(-x0, -y0)
    cr.set_line_cap(cairo.LINE_CAP_ROUND)
    cr.set_line_join(cairo.LINE_JOIN_ROUND)
    cr.set_source_rgb(*plymouth.LETTER)
    plymouth.letter_n(cr)
    cr.fill()
    plymouth.letter_s(cr)
    cr.fill()
    cr.set_source(plymouth.gradient())
    cr.set_line_width(13)
    plymouth.infinity(cr)
    cr.stroke()
    s.write_to_png(os.path.join(OUT, "logo.png"))
    return w, h


def rounded(cr, x, y, w, h, r):
    cr.new_sub_path()
    cr.arc(x + w - r, y + r, r, -math.pi / 2, 0)
    cr.arc(x + w - r, y + h - r, r, 0, math.pi / 2)
    cr.arc(x + r, y + h - r, r, math.pi / 2, math.pi)
    cr.arc(x + r, y + r, r, math.pi, 3 * math.pi / 2)
    cr.close_path()


def nine_slice(prefix, fill, radius, border=None):
    """Découpe un rectangle arrondi en 9 morceaux (style « *_nw.png »… de GRUB)."""
    size = radius * 2 + 4
    full = cairo.ImageSurface(cairo.FORMAT_ARGB32, size, size)
    cr = cairo.Context(full)
    rounded(cr, 0.5, 0.5, size - 1, size - 1, radius)
    cr.set_source_rgba(*rgb(fill), fill[3])
    cr.fill_preserve()
    if border:
        cr.set_source_rgba(*rgb(border), border[3])
        cr.set_line_width(1)
        cr.stroke()
    r, m = radius, size - 2 * radius
    parts = {
        "nw": (0, 0, r, r), "n": (r, 0, m, r), "ne": (r + m, 0, r, r),
        "w": (0, r, r, m), "c": (r, r, m, m), "e": (r + m, r, r, m),
        "sw": (0, r + m, r, r), "s": (r, r + m, m, r), "se": (r + m, r + m, r, r),
    }
    for name, (x, y, w, h) in parts.items():
        p = cairo.ImageSurface(cairo.FORMAT_ARGB32, w, h)
        pc = cairo.Context(p)
        pc.set_source_surface(full, -x, -y)
        pc.paint()
        p.write_to_png(os.path.join(OUT, f"{prefix}_{name}.png"))


def fonts():
    mkfont = shutil.which("grub2-mkfont") or shutil.which("grub-mkfont")
    if not mkfont:
        raise SystemExit("grub2-mkfont introuvable (paquet grub2-tools-extra)")
    # grub2-mkfont ajoute le style du fichier (« Regular ») au nom : « NOOS Regular 20 »,
    # « NOOS SemiBold Regular 20 » (noms utilisés dans theme.txt).
    for family, file, sizes in [("NOOS", "NotoSans-Regular.ttf", [16, 20]), ("NOOS SemiBold", "NotoSans-SemiBold.ttf", [20, 28])]:
        for size in sizes:
            out = os.path.join(OUT, f"{family.lower().replace(' ', '-')}-{size}.pf2")
            subprocess.run(
                [mkfont, "-n", family, "-s", str(size), "-r", RANGES, "-o", out, os.path.join(FONTS, file)],
                check=True,
            )


def main():
    os.makedirs(OUT, exist_ok=True)
    background()
    logo()
    nine_slice("menu", PANEL, 18, PANEL_BORDER)
    nine_slice("select", ACCENT, 8)
    fonts()
    print(f"Thème GRUB NOOS généré dans {OUT}")


if __name__ == "__main__":
    main()
