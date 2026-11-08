#!/usr/bin/env python3
"""Sons de NOOS (créés ici, libres) : branchement et retrait d'un périphérique.

Deux notes douces (sinus + harmoniques, attaque courte, décroissance exponentielle) avec un
léger écho : montante au branchement (mi → si), descendante au retrait (si → mi).
Lancer : ./generate.py (produit les .wav à côté).
"""
import math, struct, wave, pathlib

RATE = 44100

def note(freq, start, dur, out, gain=0.5):
    n0 = int(start * RATE)
    for i in range(int(dur * RATE)):
        t = i / RATE
        env = min(1.0, t / 0.008) * math.exp(-t * 7.0)
        v = (math.sin(2 * math.pi * freq * t)
             + 0.35 * math.sin(2 * math.pi * freq * 2 * t) * math.exp(-t * 4)
             + 0.12 * math.sin(2 * math.pi * freq * 3 * t) * math.exp(-t * 10))
        if n0 + i < len(out):
            out[n0 + i] += gain * env * v

def render(name, notes, length=0.75):
    out = [0.0] * int(length * RATE)
    for f, s in notes:
        note(f, s, length - s, out)
    # Écho discret (90 ms, -14 dB).
    d = int(0.09 * RATE)
    for i in range(len(out) - 1, d, -1):
        out[i] += 0.2 * out[i - d]
    peak = max(abs(x) for x in out) or 1.0
    # Fondu de sortie, normalisation à -6 dB.
    for i in range(len(out)):
        fade = min(1.0, (len(out) - i) / (0.08 * RATE))
        out[i] = out[i] / peak * 0.5 * fade
    with wave.open(str(pathlib.Path(__file__).with_name(name)), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(x * 32767)) for x in out))

E5, B5, E6 = 659.25, 987.77, 1318.51
render("noos-device-added.wav", [(E5, 0.0), (B5, 0.085)])
render("noos-device-removed.wav", [(B5, 0.0), (E5, 0.085)])
print("ok")
