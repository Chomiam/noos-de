#!/usr/bin/env python3
"""Génère les images du thème Plymouth NOOS à partir du logo (data/noos-logo.svg).

Le logo est redessiné avec cairo (mêmes tracés que le SVG) : le N, le S et le rail du
symbole infini forment l'image fixe ; le faisceau lumineux qui parcourt l'infini est
rendu image par image (beam-00.png … beam-NN.png), comme l'animation CSS du SVG.

    python3 noos-plymouth/generate.py            # écrit dans noos-plymouth/noos/
"""

import math
import os
import sys

import cairo

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "noos")

SCALE = 1.5           # logo 480×160 → 720×240
FRAMES = 60           # 3,6 s à ~16,7 images/s (Plymouth rafraîchit à 50 Hz)
W, H = int(480 * SCALE), int(160 * SCALE)

LETTER = (0xCD / 255, 0xD6 / 255, 0xF4 / 255)
TRACK = (0x26 / 255, 0x27 / 255, 0x38 / 255)
GRADIENT = [(0.0, "#89b4fa"), (0.5, "#cba6f7"), (1.0, "#f38ba8")]


def hex_rgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))


def letter_n(cr):
    pts = [(80, 40), (96, 40), (134, 98), (134, 40), (150, 40), (150, 120), (134, 120), (96, 62), (96, 120), (80, 120)]
    cr.move_to(*pts[0])
    for p in pts[1:]:
        cr.line_to(*p)
    cr.close_path()


def letter_s(cr):
    # Tracé du S du logo (commandes M/L/C absolues).
    cmds = [
        ("M", 403, 58.5), ("L", 386, 58.5), ("C", 386, 52, 380, 48, 373, 48),
        ("C", 363, 48, 357.5, 53.5, 357.5, 59.5), ("C", 357.5, 66.5, 363.5, 70.5, 376, 74.5),
        ("L", 381, 76), ("C", 397, 81, 405, 87.5, 405, 99.5), ("C", 405, 113.5, 393.5, 122, 373, 122),
        ("C", 354.5, 122, 341, 114, 340, 99.5), ("L", 357, 99.5), ("C", 358, 107, 364.5, 111.5, 373, 111.5),
        ("C", 383, 111.5, 388.5, 106.5, 388.5, 99.5), ("C", 388.5, 92.5, 382, 88.5, 369.5, 84.5),
        ("L", 364.5, 83), ("C", 348.5, 78, 341, 71.5, 341, 59.5), ("C", 341, 46.5, 353, 38, 373, 38),
        ("C", 390, 38, 402, 46.5, 403, 58.5),
    ]
    for c in cmds:
        if c[0] == "M":
            cr.move_to(*c[1:])
        elif c[0] == "L":
            cr.line_to(*c[1:])
        else:
            cr.curve_to(*c[1:])
    cr.close_path()


def infinity(cr):
    cr.save()
    cr.translate(140, 25)
    cr.move_to(105, 55)
    cr.curve_to(135, 15, 180, 15, 180, 55)
    cr.curve_to(180, 95, 135, 95, 105, 55)
    cr.curve_to(75, 15, 30, 15, 30, 55)
    cr.curve_to(30, 95, 75, 95, 105, 55)
    cr.close_path()
    cr.restore()


def path_length():
    surf = cairo.ImageSurface(cairo.FORMAT_ARGB32, 1, 1)
    cr = cairo.Context(surf)
    infinity(cr)
    flat = cr.copy_path_flat()
    length, last, first = 0.0, None, None
    for kind, pts in flat:
        if kind == cairo.PATH_MOVE_TO:
            last = first = pts
        elif kind == cairo.PATH_LINE_TO:
            length += math.dist(last, pts)
            last = pts
        elif kind == cairo.PATH_CLOSE_PATH and first:
            length += math.dist(last, first)
            last = first
    return length


def gradient():
    g = cairo.LinearGradient(140, 25, 140 + 210, 25 + 110)
    for off, col in GRADIENT:
        g.add_color_stop_rgb(off, *hex_rgb(col))
    return g


def surface():
    s = cairo.ImageSurface(cairo.FORMAT_ARGB32, W, H)
    cr = cairo.Context(s)
    cr.scale(SCALE, SCALE)
    cr.set_line_cap(cairo.LINE_CAP_ROUND)
    cr.set_line_join(cairo.LINE_JOIN_ROUND)
    return s, cr


def base():
    s, cr = surface()
    cr.set_source_rgb(*LETTER)
    letter_n(cr)
    cr.fill()
    letter_s(cr)
    cr.fill()
    cr.set_source_rgb(*TRACK)
    cr.set_line_width(13)
    infinity(cr)
    cr.stroke()
    return s


def ease(t):
    # cubic-bezier(0.4, 0, 0.2, 1) (« ease » de Material) : approximation par recherche.
    lo, hi = 0.0, 1.0
    for _ in range(40):
        mid = (lo + hi) / 2
        x = 3 * (1 - mid) ** 2 * mid * 0.4 + 3 * (1 - mid) * mid ** 2 * 0.2 + mid ** 3
        if x < t:
            lo = mid
        else:
            hi = mid
    u = (lo + hi) / 2
    return 3 * (1 - u) * u ** 2 * 1.0 + u ** 3


def beam(frame, length):
    s, cr = surface()
    progress = ease(frame / FRAMES)
    offset = -progress * length
    for dash, extra, alpha in [((110, length - 110), 0.0, 1.0), ((55, length - 55), -length / 2, 0.85)]:
        # Halo (flou gaussien du SVG simulé par des traits de plus en plus larges et pâles).
        for width, a in [(26, 0.10), (20, 0.16), (16, 0.25), (13, 1.0)]:
            cr.save()
            cr.push_group()
            cr.set_source(gradient())
            cr.set_line_width(width)
            cr.set_dash(dash, offset + extra)
            infinity(cr)
            cr.stroke()
            cr.pop_group_to_source()
            cr.paint_with_alpha(a * alpha)
            cr.restore()
    return s


def main():
    os.makedirs(OUT, exist_ok=True)
    base().write_to_png(os.path.join(OUT, "logo.png"))
    length = path_length()
    for f in range(FRAMES):
        beam(f, length).write_to_png(os.path.join(OUT, f"beam-{f:02d}.png"))
    # Petit point du champ de mot de passe (disques chiffrés).
    s = cairo.ImageSurface(cairo.FORMAT_ARGB32, 14, 14)
    cr = cairo.Context(s)
    cr.set_source_rgb(*LETTER)
    cr.arc(7, 7, 5, 0, 2 * math.pi)
    cr.fill()
    s.write_to_png(os.path.join(OUT, "bullet.png"))
    # Cadre du champ de saisie.
    s = cairo.ImageSurface(cairo.FORMAT_ARGB32, 320, 44)
    cr = cairo.Context(s)
    r = 10
    cr.new_sub_path()
    cr.arc(320 - r - 1, r + 1, r, -math.pi / 2, 0)
    cr.arc(320 - r - 1, 44 - r - 1, r, 0, math.pi / 2)
    cr.arc(r + 1, 44 - r - 1, r, math.pi / 2, math.pi)
    cr.arc(r + 1, r + 1, r, math.pi, 3 * math.pi / 2)
    cr.close_path()
    cr.set_source_rgba(1, 1, 1, 0.06)
    cr.fill_preserve()
    cr.set_source_rgb(*hex_rgb("#89b4fa"))
    cr.set_line_width(1.5)
    cr.stroke()
    s.write_to_png(os.path.join(OUT, "entry.png"))
    print(f"{FRAMES} images d'animation écrites dans {OUT}")


if __name__ == "__main__":
    sys.exit(main())
