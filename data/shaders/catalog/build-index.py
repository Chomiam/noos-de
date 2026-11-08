#!/usr/bin/env python3
"""Construit index.json (catalogue d'animations NOOS) à partir des fichiers .frag du dossier.

En-tête de chaque fichier : lignes « // clé: valeur » (name, description, tags, homage,
author, license). Lancer après chaque ajout : ./build-index.py
"""
import json, pathlib

here = pathlib.Path(__file__).resolve().parent
out = []
for f in sorted(here.glob("*.frag")):
    meta, code = {}, []
    for line in f.read_text(encoding="utf-8").splitlines():
        if not code and line.startswith("// ") and ":" in line:
            k, v = line[3:].split(":", 1)
            meta[k.strip()] = v.strip()
        else:
            code.append(line)
    out.append({
        "id": f.stem,
        "name": meta.get("name", f.stem),
        "author": meta.get("author", ""),
        "description": meta.get("description", ""),
        "tags": [t.strip() for t in meta.get("tags", "").split(",") if t.strip()],
        "license": meta.get("license", ""),
        "homage": meta.get("homage", ""),
        "code": "\n".join(code).strip() + "\n",
    })
(here / "index.json").write_text(json.dumps({"version": 1, "shaders": out}, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
print(f"{len(out)} animations → index.json")
