#!/usr/bin/env python3
"""Pull the hand-curated collections off RomM into text files.

These are the one thing on that server nobody can rebuild. The library itself is
on the SSD and hash-verified; the artwork is in ES-DE; but "which 86 of 882 NES
games are the good ones" is a judgement someone made once, over a long time, and
it exists only as rows in a database that is about to be deleted.

    tools/export-collections.py              write data/collections/
    tools/export-collections.py --check      compare files against the server

One file per collection, one game per line:

    # ★ Best of nes
    # 86 games, exported 2026-09-03 from RomM
    Castlevania (USA)
    Contra (USA)

Text because a list you can read, diff, edit and restore without a server
running is a list that survives the server. `library-service.md` chose this
shape; this is the export that makes it real.
"""

import argparse
import datetime
import pathlib
import sys
import tomllib
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "data" / "collections"


def api(base, token, path):
    req = urllib.request.Request(
        f"{base}{path}", headers={"Authorization": f"Bearer {token}"}
    )
    import json

    with urllib.request.urlopen(req, timeout=120) as r:
        return json.load(r)


def safe(name):
    """A file name that keeps the collection's name readable.

    Only the characters that cannot be in a path are replaced. The star in
    `★ Best of nes` stays: it sorts those nine to the top of a listing, which is
    presumably why it is there.
    """
    out = name
    for ch in '/\\:*?"<>|':
        out = out.replace(ch, "-")
    return out.strip() or "unnamed"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="compare, write nothing")
    ap.add_argument("--config", default=str(ROOT / "config.toml"))
    args = ap.parse_args()

    cfg = tomllib.load(open(args.config, "rb"))
    base = cfg["server"]["url"].rstrip("/")
    token = cfg["server"]["token"]

    cols = api(base, token, "/api/collections")
    cols = cols if isinstance(cols, list) else cols.get("items", [])
    print(f"  {len(cols)} collections on {base}")

    # One page of every rom, so ids can be resolved to names without a request
    # per game. 12k ids against 27 collections is one fetch, not 2,662.
    names = {}
    offset, limit = 0, 500
    while True:
        page = api(base, token, f"/api/roms?limit={limit}&offset={offset}")
        items = page.get("items", page if isinstance(page, list) else [])
        if not items:
            break
        for r in items:
            # The *filename* stem, not RomM's display title.
            #
            # RomM names a game from whatever metadata it matched -- "1943: The
            # Battle of Midway" -- and ES-DE names it from the file on disk --
            # "1943 (Japan) (FamicomBox)". A list keyed on the first matches
            # nothing anywhere else, which is what a first export produced: 58
            # of 86 for Best of nes, and those 28 were all present.
            #
            # The filename is the one identifier both sides already agree on,
            # and it is the No-Intro name, so it is the canonical one.
            fs = (r.get("fs_name") or "").strip()
            stem = fs.rsplit(".", 1)[0] if "." in fs else fs
            plat = (r.get("platform_fs_slug") or "").strip()
            names[r["id"]] = (plat, stem, (r.get("name") or "").strip())
        offset += limit
        if offset >= page.get("total", 0):
            break
    print(f"  {len(names)} rom names resolved")

    OUT.mkdir(parents=True, exist_ok=True)
    today = datetime.date.today().isoformat()
    total, missing, changed = 0, 0, 0

    for c in sorted(cols, key=lambda c: c.get("name", "")):
        ids = c.get("rom_ids") or []
        if not ids:
            full = api(base, token, f"/api/collections/{urllib.parse.quote(str(c['id']))}")
            ids = full.get("rom_ids") or []
        lines, gone = [], 0
        for i in ids:
            n = names.get(i)
            if n:
                plat, stem, title = n
                # `platform/name`, because a name alone is not unique: the
                # library holds arcade Contra and Famicom Contra, and an
                # un-prefixed list resolved Arcade Classics to the Famicom one.
                #
                # The title travels as a trailing comment so the list stays
                # readable, without being the primary key.
                entry = f"{plat}/{stem}" if plat else stem
                lines.append(f"{entry}    # {title}" if title and title != stem else entry)
            else:
                # Recorded rather than dropped: a membership pointing at a rom
                # the server no longer lists is exactly the rot worth seeing.
                gone += 1
                lines.append(f"# MISSING rom_id={i}")
        lines.sort(key=lambda s: (s.startswith("#"), s.lower()))
        body = (
            f"# {c.get('name')}\n"
            f"# {len(ids)} games, exported {today} from RomM\n"
            + "\n".join(lines)
            + "\n"
        )
        total += len(ids)
        missing += gone
        path = OUT / f"{safe(c.get('name', 'unnamed'))}.txt"
        if args.check:
            old = path.read_text() if path.exists() else ""
            same = old.split("\n")[2:] == body.split("\n")[2:]
            if not same:
                changed += 1
                print(f"    DIFFERS  {path.name}")
        else:
            path.write_text(body)
        print(f"    {c.get('name','?')[:42]:<42} {len(ids):>5}" + (f"  ({gone} missing)" if gone else ""))

    print(f"\n  {total} memberships, {missing} pointing at roms the server no longer lists")
    if args.check:
        print(f"  {changed} collections differ from what is on disk")
        return 1 if changed else 0
    print(f"  written to {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
