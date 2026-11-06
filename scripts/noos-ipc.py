#!/usr/bin/env python3
"""Envoie une requête IPC au compositeur NOOS.

Exemples :
  scripts/noos-ipc.py get-windows
  scripts/noos-ipc.py toggle-start-menu
  scripts/noos-ipc.py screenshot path=/tmp/noos.png
"""
import glob, json, os, socket, sys

def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 1
    req = {"type": sys.argv[1]}
    for arg in sys.argv[2:]:
        k, _, v = arg.partition("=")
        if k in ("output", "transform", "path", "command"):
            req[k] = v
        else:
            try:
                req[k] = json.loads(v)
            except ValueError:
                req[k] = v
    path = os.environ.get("NOOS_IPC_SOCKET")
    if not path:
        socks = sorted(glob.glob(os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/tmp"), "noos-ipc-*.sock")), key=os.path.getmtime)
        if not socks:
            print("aucun compositeur NOOS trouvé", file=sys.stderr)
            return 1
        path = socks[-1]
    s = socket.socket(socket.AF_UNIX)
    s.connect(path)
    s.sendall((json.dumps(req) + "\n").encode())
    print(s.makefile().readline().strip())
    return 0

if __name__ == "__main__":
    sys.exit(main())
