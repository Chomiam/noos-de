#!/usr/bin/env python3
"""Pilotage de la VM de test NOOS par QMP (à lancer dans le toolbox noos-dev).

    qmp.py shot SORTIE.png        capture d'écran (PPM → PNG, nécessite pycairo)
    qmp.py click X Y [L H]        clic gauche absolu (tablette USB), écran 1280×800 par défaut
    qmp.py scroll X Y N           molette : N crans vers le bas (négatif : vers le haut)
    qmp.py key TOUCHE…            touches QEMU (ret, esc, ctrl-alt-f2, down…), « - » = combinaison
    qmp.py type "texte\\n"         saisie de texte, clavier AZERTY côté invité
    qmp.py cmd COMMANDE           commande QMP brute (system_powerdown, system_reset…)

Socket : $NOOS_VM_QMP, sinon $XDG_RUNTIME_DIR/noosvm.sock (chemin court : limite de 108 octets).
"""
import json
import os
import socket
import sys
import time

SOCK = os.environ.get("NOOS_VM_QMP") or os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/tmp"), "noosvm.sock")

# Touches physiques à envoyer pour obtenir un caractère avec une disposition AZERTY (fr).
AZERTY = {
    'a': 'q', 'q': 'a', 'z': 'w', 'w': 'z', 'm': 'semicolon', ',': 'm', ';': 'comma', ':': 'dot',
    '.': 'shift-comma', '/': 'shift-dot', '-': '6', '_': '8', ' ': 'spc', '\n': 'ret', '|': 'alt_r-6',
    '>': 'shift-less', '<': 'less', '=': 'equal', '"': '3', "'": '4', '@': 'alt_r-0', '*': 'backslash',
    '&': '1', '$': 'bracket_right', '!': 'slash', '~': 'alt_r-2', '#': 'alt_r-3',
    '(': '5', ')': 'minus', '[': 'alt_r-5', ']': 'alt_r-minus', '{': 'alt_r-4', '}': 'alt_r-equal',
    '\\': 'alt_r-8', '`': 'alt_r-7', '+': 'shift-equal', '?': 'shift-m', '%': 'shift-apostrophe',
    '1': 'shift-1', '2': 'shift-2', '3': 'shift-3', '4': 'shift-4', '5': 'shift-5',
    '6': 'shift-6', '7': 'shift-7', '8': 'shift-8', '9': 'shift-9', '0': 'shift-0',
}


class Qmp:
    def __init__(self):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(SOCK)
        self.f = self.s.makefile("rw")
        self.f.readline()
        self.call("qmp_capabilities")

    def call(self, cmd, **args):
        msg = {"execute": cmd}
        if args:
            msg["arguments"] = args
        self.f.write(json.dumps(msg) + "\n")
        self.f.flush()
        while True:
            r = json.loads(self.f.readline())
            if "return" in r or "error" in r:
                return r

    def keys(self, combo):
        self.call("send-key", keys=[{"type": "qcode", "data": k} for k in combo.split("-")])
        time.sleep(0.05)


def shot(q, out):
    import cairo
    ppm = out + ".ppm"
    r = q.call("screendump", filename=ppm)
    if "error" in r:
        sys.exit(r)
    data = open(ppm, "rb").read()
    parts = data.split(b"\n", 3)
    w, h = map(int, parts[1].split())
    px = parts[3]
    surf = cairo.ImageSurface(cairo.FORMAT_RGB24, w, h)
    buf, stride = surf.get_data(), surf.get_stride()
    for y in range(h):
        row = px[y * w * 3:(y + 1) * w * 3]
        o = bytearray(w * 4)
        o[0::4], o[1::4], o[2::4] = row[2::3], row[1::3], row[0::3]
        buf[y * stride:y * stride + w * 4] = o
    surf.mark_dirty()
    surf.write_to_png(out)
    os.remove(ppm)
    print(out, w, h)


def click(q, x, y, w=1280.0, h=800.0):
    ax, ay = int(x / w * 32767), int(y / h * 32767)
    q.call("input-send-event", events=[
        {"type": "abs", "data": {"axis": "x", "value": ax}},
        {"type": "abs", "data": {"axis": "y", "value": ay}},
    ])
    time.sleep(0.15)
    for down in (True, False):
        q.call("input-send-event", events=[{"type": "btn", "data": {"down": down, "button": "left"}}])
        time.sleep(0.08)


def scroll(q, x, y, n, w=1280.0, h=800.0):
    """Molette : n crans vers le bas (négatif = vers le haut) au point (x, y)."""
    q.call("input-send-event", events=[
        {"type": "abs", "data": {"axis": "x", "value": int(x / w * 32767)}},
        {"type": "abs", "data": {"axis": "y", "value": int(y / h * 32767)}},
    ])
    time.sleep(0.1)
    button = "wheel-down" if n > 0 else "wheel-up"
    for _ in range(abs(n)):
        for down in (True, False):
            q.call("input-send-event", events=[{"type": "btn", "data": {"down": down, "button": button}}])
            time.sleep(0.04)


def type_text(q, text):
    for ch in text:
        k = AZERTY.get(ch.lower() if ch.isalpha() else ch, ch.lower())
        if ch.isalpha() and ch.isupper():
            k = "shift-" + k
        q.keys(k)


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    q = Qmp()
    cmd, args = sys.argv[1], sys.argv[2:]
    if cmd == "shot":
        shot(q, args[0])
    elif cmd == "click":
        click(q, *map(float, args))
    elif cmd == "scroll":
        scroll(q, float(sys.argv[2]), float(sys.argv[3]), int(sys.argv[4]))
    elif cmd == "key":
        for k in args:
            q.keys(k)
            time.sleep(0.1)
    elif cmd == "type":
        type_text(q, args[0].encode().decode("unicode_escape"))
    elif cmd == "cmd":
        print(q.call(args[0]))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
