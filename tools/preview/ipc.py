#!/usr/bin/env python3
"""Client IPC du compositeur imbriqué : ipc.py <socket IPC> '<requête JSON>' [...]

Ex. : ipc.py /run/user/1000/noos-ipc-1234.sock '{"type":"add-desktop","name":null,"switch":true}'
Affiche la réponse de chaque requête (une ligne JSON).
"""
import json, os, socket, sys

path = sys.argv[1]
for req in sys.argv[2:]:
    s = socket.socket(socket.AF_UNIX)
    s.connect(path)
    s.sendall(req.encode() + b"\n")
    f = s.makefile()
    print(f.readline().strip())
    s.close()
