"""Prints the Rust crate tables for THIRD-PARTY-NOTICES.md from Cargo.lock.

    cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc > meta.json
    python scripts/third-party.py meta.json

Two tables: the crates compiled into vale.exe (normal dependencies, reached
from the package) and the ones used only to build it (build scripts and
their dependencies). Paste them over the tables in THIRD-PARTY-NOTICES.md
whenever Cargo.lock changes.
"""

import json
import sys


def main(path):
    meta = json.load(open(path, encoding="utf-8"))
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    packages = {p["id"]: p for p in meta["packages"]}
    root = meta["resolve"]["root"]

    shipped, stack = set(), [root]
    while stack:
        for dep in nodes[stack.pop()]["deps"]:
            normal = any(k["kind"] is None for k in dep["dep_kinds"])
            if normal and dep["pkg"] not in shipped:
                shipped.add(dep["pkg"])
                stack.append(dep["pkg"])
    build = set(nodes) - shipped - {root}

    def table(ids):
        rows = ["| Crate | Version | Licence | Source |", "|---|---|---|---|"]
        for i in sorted(ids, key=lambda i: (packages[i]["name"], packages[i]["version"])):
            p = packages[i]
            repo = p.get("repository") or f"https://crates.io/crates/{p['name']}"
            rows.append(f"| {p['name']} | {p['version']} | {p.get('license') or '?'} | {repo} |")
        return "\n".join(rows)

    print("### Dependencies of vale.exe (with the macros that generate its code)\n")
    print(table(shipped))
    print("\n### Used only to build it (resources, the Windows SDK lookup)\n")
    print(table(build))


if __name__ == "__main__":
    main(sys.argv[1])
