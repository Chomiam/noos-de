#!/usr/bin/env python3
"""Fond d'écran « Aurore » de NOOS (œuvre originale, utilisée pour les captures du README).

    python3 docs/captures/fond-aurore.py sortie.png [largeur hauteur]

Dégradé nuit saphir + voiles de lumière (accent bleu, mauve, turquoise) + grain léger.
"""
import math
import random
import sys

import cairo

out = sys.argv[1] if len(sys.argv) > 1 else "aurore.png"
W = int(sys.argv[2]) if len(sys.argv) > 2 else 2560
H = int(sys.argv[3]) if len(sys.argv) > 3 else 1440
random.seed(7)

surf = cairo.ImageSurface(cairo.FORMAT_RGB24, W, H)
cr = cairo.Context(surf)


def hexc(h, a=1.0):
    h = h.lstrip("#")
    return tuple(int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)) + (a,)


# Fond : nuit bleutée, plus claire en bas à droite.
g = cairo.LinearGradient(0, 0, W, H)
g.add_color_stop_rgba(0.0, *hexc("#0d0e1a"))
g.add_color_stop_rgba(0.55, *hexc("#161829"))
g.add_color_stop_rgba(1.0, *hexc("#1f2340"))
cr.set_source(g)
cr.paint()

# Halos (taches de lumière très diffuses).
for x, y, r, col, a in [
    (0.18, 0.25, 0.55, "#89b4fa", 0.30),
    (0.78, 0.18, 0.45, "#cba6f7", 0.24),
    (0.62, 0.85, 0.60, "#74c7ec", 0.18),
    (0.05, 0.95, 0.40, "#b4befe", 0.14),
]:
    rg = cairo.RadialGradient(x * W, y * H, 0, x * W, y * H, r * W)
    rg.add_color_stop_rgba(0, *hexc(col, a))
    rg.add_color_stop_rgba(1, *hexc(col, 0))
    cr.set_source(rg)
    cr.paint()

# Rideaux d'aurore : traits verticaux qui s'estompent vers le haut, le long de courbes
# ondulées, ajoutés à la lumière existante.
cr.set_operator(cairo.OPERATOR_ADD)
for band, (col, base, amp, freq, phase, height, alpha) in enumerate([
    ("#5fe0c0", 0.62, 0.08, 1.2, 0.4, 0.40, 0.30),
    ("#7aa2ff", 0.52, 0.07, 1.6, 2.2, 0.34, 0.26),
    ("#c58cff", 0.44, 0.06, 2.3, 3.6, 0.26, 0.20),
]):
    step = 3
    for xi in range(0, W, step):
        u = xi / W
        y = (base + amp * math.sin(freq * u * math.pi * 2 + phase) + 0.025 * math.sin(7.1 * u * math.pi + band * 1.7)) * H
        # Intensité qui varie le long du rideau (plis).
        k = 0.5 + 0.5 * math.sin(u * 23.0 + band) * math.sin(u * 9.0 + phase)
        k = max(0.0, k) ** 1.5 * (0.35 + 0.65 * math.sin(u * math.pi))
        k *= 0.75 + 0.25 * random.random()  # stries fines
        top = y - height * H
        lg = cairo.LinearGradient(0, top, 0, y + 0.02 * H)
        lg.add_color_stop_rgba(0.0, *hexc(col, 0))
        lg.add_color_stop_rgba(0.6, *hexc(col, alpha * k * 0.35))
        lg.add_color_stop_rgba(0.93, *hexc(col, alpha * k))
        lg.add_color_stop_rgba(1.0, *hexc(col, 0))
        cr.rectangle(xi, top, step, y + 0.02 * H - top)
        cr.set_source(lg)
        cr.fill()
cr.set_operator(cairo.OPERATOR_OVER)

# Horizon sombre en bas (là où se pose la barre).
hg = cairo.LinearGradient(0, H * 0.70, 0, H)
hg.add_color_stop_rgba(0, *hexc("#0b0c16", 0))
hg.add_color_stop_rgba(1, *hexc("#0b0c16", 0.75))
cr.set_source(hg)
cr.paint()

# Étoiles discrètes dans le haut.
for _ in range(220):
    x, y = random.random() * W, random.random() ** 1.8 * H * 0.6
    r = random.random() * 1.2 + 0.3
    cr.arc(x, y, r, 0, 2 * math.pi)
    cr.set_source_rgba(1, 1, 1, random.random() * 0.35 + 0.05)
    cr.fill()

# Vignette.
vg = cairo.RadialGradient(W / 2, H / 2, min(W, H) * 0.35, W / 2, H / 2, max(W, H) * 0.8)
vg.add_color_stop_rgba(0, 0, 0, 0, 0)
vg.add_color_stop_rgba(1, 0, 0, 0, 0.45)
cr.set_source(vg)
cr.paint()

surf.write_to_png(out)
print(out)
