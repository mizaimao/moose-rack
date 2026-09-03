#!/usr/bin/env python3
"""Match the inventory against No-Intro and Redump, and record the verdict.

Reads `inventory.db`, which already holds every hash. Nothing on the SSD is
touched and nothing is renamed: this fills in `dat_source`, `dat_name`,
`verdict` and `proposed_name` so a rename can be reviewed per system later.

    tools/dat_match.py --fetch      download the DATs (cached in dats/)
    tools/dat_match.py              match everything hashed
    tools/dat_match.py --report     what matched, per system

Which hash wins, and why it is not just one:

* Archives are matched on the **inner** hash. The dump is the thing in the DAT;
  the zip around it is ours and No-Intro has never seen it.
* Loose files are matched on the **container** hash, which is the same bytes.
* NES falls back to the **stripped** hash. libretro's No-Intro NES set is
  headered -- entries are 16400 bytes where the ROM is 16384 -- so the unstripped
  hash is the one that usually hits, and stripped is the fallback for a dump
  stored without its header.

A file that matches nothing is `unknown`, never `bad`. Coverage is thin on
Europe-only releases and absence proves nothing.
"""

import argparse
import re
import sqlite3
import sys
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DB = ROOT / "inventory.db"
DATS = ROOT / "dats"
BASE = "https://raw.githubusercontent.com/libretro/libretro-database/master/metadat"

# Our system folder -> (database, DAT name). A system can have more than one.
# Arcade is deliberately absent: for MAME and FBNeo the filename *is* the set
# name and parent/clone loading depends on it, so it is verified elsewhere and
# never renamed.
MAP = {
    "nes":                    [("no-intro", "Nintendo - Nintendo Entertainment System")],
    "nes_unlicensed":         [("no-intro", "Nintendo - Nintendo Entertainment System")],
    "famicom":                [("no-intro", "Nintendo - Nintendo Entertainment System"),
                               ("no-intro", "Nintendo - Family Computer Disk System")],
    "snes":                   [("no-intro", "Nintendo - Super Nintendo Entertainment System")],
    "sfc":                    [("no-intro", "Nintendo - Super Nintendo Entertainment System")],
    "gb":                     [("no-intro", "Nintendo - Game Boy")],
    "gb_superset":            [("no-intro", "Nintendo - Game Boy")],
    "gbc":                    [("no-intro", "Nintendo - Game Boy Color")],
    "gbc_superset":           [("no-intro", "Nintendo - Game Boy Color")],
    "gba":                    [("no-intro", "Nintendo - Game Boy Advance")],
    "gba_superset":           [("no-intro", "Nintendo - Game Boy Advance")],
    "n64":                    [("no-intro", "Nintendo - Nintendo 64")],
    "nds":                    [("no-intro", "Nintendo - Nintendo DS")],
    "n3ds":                   [("no-intro", "Nintendo - Nintendo 3DS")],
    "genesis":                [("no-intro", "Sega - Mega Drive - Genesis")],
    "mastersystem_hidden":    [("no-intro", "Sega - Master System - Mark III")],
    "gamegear_hidden":        [("no-intro", "Sega - Game Gear")],
    "pcengine_hidden":        [("no-intro", "NEC - PC Engine - TurboGrafx 16")],
    "ngp_hidden":             [("no-intro", "SNK - Neo Geo Pocket"),
                               ("no-intro", "SNK - Neo Geo Pocket Color")],
    "wonderswan_hidden":      [("no-intro", "Bandai - WonderSwan")],
    "wonderswancolor_hidden": [("no-intro", "Bandai - WonderSwan Color")],
    "psx":                    [("redump",   "Sony - PlayStation")],
    "ps2":                    [("redump",   "Sony - PlayStation 2")],
    "ps3":                    [("redump",   "Sony - PlayStation 3")],
    "psp":                    [("redump",   "Sony - PlayStation Portable"),
                               ("no-intro", "Sony - PlayStation Portable")],
    "saturn":                 [("redump",   "Sega - Saturn")],
    "dreamcast":              [("redump",   "Sega - Dreamcast")],
    "3do":                    [("redump",   "The 3DO Company - 3DO")],
    "gc":                     [("redump",   "Nintendo - GameCube")],
    "wii":                    [("redump",   "Nintendo - Wii")],
    "xbox360":                [("redump",   "Microsoft - Xbox 360")],
    "psvita":                 [("no-intro", "Sony - PlayStation Vita")],
}

GAME = re.compile(r'game\s*\(\s*name\s+"([^"]+)"(.*?)\n\)', re.S)
ROM = re.compile(
    r'rom\s*\(\s*name\s+"([^"]+)"\s+size\s+(\d+)'
    r'(?:\s+crc\s+([0-9A-Fa-f]+))?'
    r'(?:\s+md5\s+([0-9A-Fa-f]+))?'
    r'(?:\s+sha1\s+([0-9A-Fa-f]+))?'
)


def fetch(db, name):
    out = DATS / db / f"{name}.dat"
    if out.exists():
        return out
    out.parent.mkdir(parents=True, exist_ok=True)
    url = f"{BASE}/{db}/{urllib.parse.quote(name)}.dat"
    try:
        with urllib.request.urlopen(url, timeout=60) as r:
            out.write_bytes(r.read())
        print(f"  fetched {db}/{name}")
        return out
    except Exception as e:
        print(f"  MISSING {db}/{name}: {e}")
        return None


def load(path):
    """(md5, sha1, crc) -> (dat game name, rom file name), lowercased keys."""
    by_md5, by_sha1, by_crc = {}, {}, {}
    text = path.read_text(errors="replace")
    for gname, body in GAME.findall(text):
        for rname, _size, crc, md5, sha1 in ROM.findall(body):
            val = (gname, rname)
            if md5:
                by_md5[md5.lower()] = val
            if sha1:
                by_sha1[sha1.lower()] = val
            if crc:
                by_crc[crc.lower().zfill(8)] = val
    return by_md5, by_sha1, by_crc


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--fetch", action="store_true", help="download DATs and stop")
    ap.add_argument("--report", action="store_true")
    ap.add_argument("--systems")
    args = ap.parse_args()

    db = sqlite3.connect(DB)
    db.execute("PRAGMA journal_mode=WAL")

    if args.report:
        print(f"  {'system':<24}{'good':>7}{'unknown':>9}{'n/a':>7}")
        for s, g, u, n in db.execute(
            "SELECT system, SUM(verdict='good'), SUM(verdict='unknown'), SUM(verdict IS NULL) "
            "FROM files GROUP BY system ORDER BY 2 DESC"
        ):
            if g or u:
                print(f"  {s:<24}{g or 0:>7}{u or 0:>9}{n or 0:>7}")
        tot = db.execute(
            "SELECT SUM(verdict='good'), SUM(verdict='unknown') FROM files"
        ).fetchone()
        print(f"\n  matched {tot[0] or 0:,}   unknown {tot[1] or 0:,}")
        return

    systems = args.systems.split(",") if args.systems else list(MAP)

    if args.fetch:
        for s in systems:
            for dbname, dat in MAP.get(s, []):
                fetch(dbname, dat)
        return

    for s in systems:
        specs = MAP.get(s)
        if not specs:
            continue
        rows = db.execute(
            "SELECT id, container_md5, container_sha1, container_crc32, "
            "       inner_md5, inner_sha1, inner_crc32, "
            "       stripped_md5, stripped_sha1, stripped_crc32, "
            "       COALESCE(inner_name, path), chd_datasha1 "
            "FROM files WHERE system=? AND status='ok' AND container_format<>'non-game'",
            (s,),
        ).fetchall()
        if not rows:
            continue

        indexes = []
        for dbname, dat in specs:
            p = fetch(dbname, dat)
            if p:
                indexes.append((dbname, *load(p)))
        if not indexes:
            continue

        hits = 0
        updates = []
        for (rid, cm, cs, cc, im, isha, ic, sm, ss, sc, curname, chdraw) in rows:
            found = None
            # Inner first: the dump is what the DAT knows, not our zip. Then the
            # CHD's raw SHA-1 -- for a DVD-type CHD the decompressed image *is*
            # the ISO, so that hash is exactly what Redump lists, and without it
            # every disc system matches nothing at all.
            for md5, sha1, crc in ((im, isha, ic), (None, chdraw, None), (cm, cs, cc), (sm, ss, sc)):
                for dbname, by_md5, by_sha1, by_crc in indexes:
                    v = (
                        (sha1 and by_sha1.get(sha1))
                        or (md5 and by_md5.get(md5))
                        or (crc and by_crc.get(crc))
                    )
                    if v:
                        found = (dbname, *v)
                        break
                if found:
                    break
            if found:
                dbname, gname, rname = found
                hits += 1
                proposed = None if Path(curname).stem == Path(rname).stem else rname
                updates.append(("good", dbname, gname, proposed, rid))
            else:
                updates.append(("unknown", None, None, None, rid))

        db.executemany(
            "UPDATE files SET verdict=?, dat_source=?, dat_name=?, proposed_name=? WHERE id=?",
            updates,
        )
        db.commit()
        pct = 100 * hits / len(rows) if rows else 0
        print(f"  {s:<24} {hits:>5}/{len(rows):<5} {pct:5.1f}%")

    return 0


if __name__ == "__main__":
    sys.exit(main())
