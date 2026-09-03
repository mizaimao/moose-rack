#!/usr/bin/env python3
"""What each voted collection would actually contain, and where the games are.

`voted.json` says which titles a platform's sources agreed on. This says what
that means in practice, which is the only question left before building
collections: of the games the sources named, how many are already in the
library, how many are sitting on the external drive waiting to be copied, and
how many exist nowhere and have to be sourced.

Arcade is read from its own files rather than `voted.json` — it was curated
separately and is keyed by MAME romset, which needs no title matching and is
more reliable than any of this.

    python3 tools/collection_status.py
    python3 tools/collection_status.py --tier agreed --platform megadrive --list
"""

import argparse
import collections
import json
import pathlib
import sqlite3
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from community_favorites import norm  # noqa: E402
from drive_vs_library import PLATFORM_MAP, open_manifest  # noqa: E402


def load(db, manifest):
    lib, drive = collections.defaultdict(dict), collections.defaultdict(dict)
    con = sqlite3.connect(db)
    for slug, name in con.execute(
        "select platform_slug, COALESCE(NULLIF(name,''), fs_name) from roms"
    ):
        if norm(name):
            lib[slug][norm(name)] = name
    con.close()
    with open_manifest(manifest) as fh:
        for line in fh:
            r = json.loads(line)
            slug = PLATFORM_MAP.get(r["platform"])
            if slug and norm(r["name"]):
                drive[slug][norm(r["name"])] = r["name"]
    return lib, drive


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--voted", default="data/community/voted.json")
    ap.add_argument("--db", default="cache.sqlite3")
    ap.add_argument("--manifest", default="drive-manifest/manifest.jsonl")
    ap.add_argument("--tier", default="most", choices=("agreed", "most"))
    ap.add_argument("--platform")
    ap.add_argument("--list", action="store_true", help="print the titles, grouped by where they are")
    args = ap.parse_args()

    voted = json.load(open(args.voted))
    lib, drive = load(args.db, args.manifest)

    rows = []
    for p, v in voted.items():
        if args.platform and p != args.platform:
            continue
        have, copy, gone = [], [], []
        for t in v[args.tier]:
            k = norm(t)
            (have if k in lib.get(p, {}) else copy if k in drive.get(p, {}) else gone).append(t)
        rows.append((p, v["source_count"], len(v[args.tier]), have, copy, gone))
        if args.list and args.platform:
            for label, items in (("in your library", have), ("on the drive", copy), ("nowhere", gone)):
                print(f"\n{label} ({len(items)}):")
                for t in items:
                    print(f"   {t}")

    # Arcade was curated separately, keyed by romset — no title matching needed.
    con = sqlite3.connect(args.db)
    romsets = {fs.rsplit(".", 1)[0].lower()
               for (fs,) in con.execute("select fs_name from roms where platform_slug='arcade'")}
    con.close()
    for name, tier in (("arcade", "short"), ("arcade", "long")):
        f = pathlib.Path(f"data/community/arcade-bestof-{tier}.json")
        if f.exists() and not args.platform:
            g = json.load(open(f))["games"]
            have = [v for k, v in g.items() if k.lower() in romsets]
            rows.append((f"arcade ({tier})", 3 if tier == "long" else 2, len(g), have, [], 
                         [v for k, v in g.items() if k.lower() not in romsets]))

    print(f"\n{'platform':20}{'src':>4}{'titles':>8}{'have':>7}{'on drive':>10}{'missing':>9}  coverage")
    print("-" * 76)
    for p, n, tot, have, copy, gone in sorted(rows, key=lambda r: -len(r[3])):
        pct = len(have) / tot if tot else 0
        bar = "#" * round(pct * 12)
        print(f"{p:20}{n:4}{tot:8}{len(have):7}{len(copy):10}{len(gone):9}  {bar:<12} {pct:4.0%}")
    t = [sum(len(r[i]) if i > 2 else r[i] for r in rows) for i in (2, 3, 4, 5)]
    print("-" * 76)
    print(f"{'TOTAL':20}{'':4}{t[0]:8}{t[1]:7}{t[2]:10}{t[3]:9}")


if __name__ == "__main__":
    main()
