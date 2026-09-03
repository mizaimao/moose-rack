#!/usr/bin/env python3
"""Build `data/arcade-names.json` — romset short name → real title.

RomM names a ROM from whatever metadata it matched, and falls back to the file
name when nothing matched. On arcade platforms that fallback is the romset short
name, so the library ends up showing `kof98`, `samsho4`, `tophuntr`.

The DATs already fetched for core-coverage analysis carry a `<description>` per
romset, which is the actual title. This flattens FBNeo's and MAME's into one
map. FBNeo is read first and wins on conflict: its descriptions are shorter and
closer to the name on the cabinet.

Two bits of cleanup, both from looking at the output rather than guessed at:

* XML entities are unescaped — the raw DAT has `Roddy &amp; Cathy`.
* Trailing Neo Geo board codes are dropped. `(NGM-2420)` is a part number, not
  part of the title, and it is pure noise in a grid. Region and revision
  parentheses are kept, because those distinguish genuinely different sets.
"""

import argparse
import html
import json
import pathlib
import re

BOARD_CODE = re.compile(r"\s*\((?:NGM|NGH|ALM|ALH|MVS)[-–][0-9]+(?:\s*~\s*(?:NGM|NGH)[-–][0-9]+)?\)\s*$")
ENTRY = re.compile(
    r'<(?:game|machine) name="([^"]+)"[^>]*>\s*(?:<[^>]+>\s*)*?<description>([^<]+)</description>'
)


def clean(desc):
    desc = html.unescape(desc).strip()
    desc = BOARD_CODE.sub("", desc)
    return desc.strip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dats", nargs="+", help="DAT/XML files, highest priority first")
    ap.add_argument("--out", default="data/arcade-names.json")
    args = ap.parse_args()

    names = {}
    for f in args.dats:
        p = pathlib.Path(f)
        if not p.exists():
            print(f"  skip {p}: not found")
            continue
        n = 0
        for m in ENTRY.finditer(p.read_text(errors="replace")):
            title = clean(m.group(2))
            if title and names.setdefault(m.group(1), title) == title:
                n += 1
        print(f"  {p.name}: {n} new")

    out = pathlib.Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(names, indent=0, sort_keys=True, ensure_ascii=False))
    print(f"{len(names)} romsets -> {out} ({out.stat().st_size/1e6:.1f} MB)")


if __name__ == "__main__":
    main()
