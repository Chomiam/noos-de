#!/usr/bin/env python3
"""Console série de la VM de test (Windows : 127.0.0.1:4445, voir tools/vm/vm.sh).

    serial.py login UTILISATEUR MOT_DE_PASSE   se connecte (getty série : console=ttyS0)
    serial.py run "commande"                    lance une commande, affiche sa sortie
    serial.py read [SECONDES]                   affiche ce qui arrive pendant N secondes

Le système invité doit avoir « console=ttyS0,115200 » sur la ligne du noyau (systemd lance
alors serial-getty@ttyS0). Sortie de « run » : entre deux marqueurs, sans les couleurs.
"""
import os
import re
import socket
import sys
import time

ADDR = os.environ.get("NOOS_VM_SERIAL", "127.0.0.1:4445")


def connect():
    host, _, port = ADDR.rpartition(":")
    s = socket.create_connection((host, int(port)))
    s.settimeout(0.3)
    return s


def drain(s, seconds):
    out = b""
    end = time.time() + seconds
    while time.time() < end:
        try:
            chunk = s.recv(65536)
            if not chunk:
                break
            out += chunk
        except socket.timeout:
            pass
    return out.decode("utf-8", "replace")


def until(s, pattern, timeout):
    out = ""
    end = time.time() + timeout
    while time.time() < end:
        out += drain(s, 0.5)
        if re.search(pattern, out, re.M):
            break
    return out


def clean(text):
    return re.sub(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07|\r", "", text)


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    s = connect()
    cmd = sys.argv[1]
    if cmd == "login":
        s.sendall(b"\n")
        out = until(s, r"login:\s*$", 15)
        s.sendall(sys.argv[2].encode() + b"\n")
        out += until(s, r"[Pp]assword|[Mm]ot de passe", 10)
        s.sendall(sys.argv[3].encode() + b"\n")
        out += until(s, r"[$#>]\s*$", 20)
        # Pas de couleurs ni de pagination ; bash plutôt que fish pour la suite.
        s.sendall(b"exec bash --norc\n")
        out += drain(s, 1.5)
        s.sendall(b"export TERM=dumb PAGER=cat SYSTEMD_PAGER= SYSTEMD_COLORS=0; stty cols 200\n")
        out += drain(s, 1)
        print(clean(out)[-1500:])
    elif cmd == "run":
        mark = f"FIN{int(time.time())}"
        s.sendall(f"{sys.argv[2]}; echo {mark}\n".encode())
        out = until(s, rf"^{mark}", float(os.environ.get("NOOS_VM_TIMEOUT", "60")))
        text = clean(out)
        text = text.split("\n", 1)[1] if "\n" in text else text   # écho de la commande
        print(re.split(rf"^{mark}", text, flags=re.M)[0].rstrip())
    elif cmd == "read":
        print(clean(drain(s, float(sys.argv[2]) if len(sys.argv) > 2 else 3)))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
