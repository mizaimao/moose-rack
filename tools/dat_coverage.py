#!/usr/bin/env python3
"""Decide which arcade core can run each ROM, without launching anything.

Launching RetroArch opens a window per game, so a 3,315-game sweep is not an
option. This reaches the same verdict from file contents alone.

The method matters. Matching romset *names* against a DAT is not enough: MAME
0.283 knows Street Fighter III perfectly well and still refused our copy,
because a CPS3 set needs files our zip does not contain. Absent drivers are
rare; incomplete or wrong-version romsets are the real failure mode. So this
compares the CRC32 of every file the DAT says a driver requires against the
CRC32s actually present in our zip — read from the zip's central directory,
which needs no decompression.

Parent sets and BIOS matter too: a clone's DAT entry lists only its own files
and inherits the rest via `cloneof`/`romof`. A file missing from the clone but
present in the parent is not missing, provided we also have the parent.

Usage:
    dat_coverage.py --roms library/roms/arcade --dats <dir> [--json out.json]
"""

import argparse
import json
import pathlib
import re
import sys
import zipfile
from xml.etree import ElementTree as ET


def parse_dat(path):
    """romset -> {"roms": {name: crc}, "parents": [...], "status": ...}"""
    games = {}
    # iterparse: these files are up to 71 MB and a full DOM is wasteful.
    for _, el in ET.iterparse(str(path), events=("end",)):
        if el.tag not in ("game", "machine"):
            continue
        name = el.get("name")
        if name:
            roms = {}
            for r in el.findall("rom"):
                crc = (r.get("crc") or "").lower().lstrip("0") or None
                # status="nodump" means no correct dump exists; requiring it
                # would mark working sets as broken.
                if crc and r.get("status") != "nodump":
                    roms[r.get("name", "")] = crc
            parents = [p for p in (el.get("cloneof"), el.get("romof")) if p]
            games[name] = {"roms": roms, "parents": parents,
                           "chd": bool(el.findall("disk")),
                           "isbios": el.get("isbios") == "yes"}
        el.clear()
    return games


def bios_crcs(games):
    """Every CRC that belongs to a BIOS set, across the whole DAT.

    Per-set flagging is not enough. MAME's DAT repeats the BIOS ROMs inside
    each Neo Geo game entry as well as in the `neogeo` BIOS set, so a file
    looked like a game file purely because the game's own entry was read first.
    A CRC that appears in any `isbios` machine is a BIOS file wherever else it
    turns up."""
    out = set()
    for g in games.values():
        if g["isbios"]:
            out |= set(g["roms"].values())
    return out


def zip_crcs(path):
    """CRC32s inside a zip, as lowercase hex without leading zeros."""
    try:
        with zipfile.ZipFile(path) as z:
            return {format(i.CRC, "x").lstrip("0") or "0" for i in z.infolist()}
    except Exception:
        return None


def required(games, name, seen=None):
    """Every CRC a set needs, following parents, as {file: (crc, from_bios)}.

    A BIOS set is flagged rather than merged silently. Its 30-odd files are
    mostly *alternative* regional BIOSes — a Neo Geo cabinet needs one of them,
    not all — so demanding the full set marks every working Neo Geo game as
    broken. Verified against two real neogeo.zip BIOS sets here: both have the
    essential files and differ only in which regional variants they carry.

    Missing parents are ignored: their files are then simply absent from the
    requirement set, which the availability check in `verdict` handles."""
    seen = seen or set()
    if name in seen or name not in games:
        return {}
    seen.add(name)
    is_bios = games[name]["isbios"]
    out = {n: (crc, is_bios) for n, crc in games[name]["roms"].items()}
    for p in games[name]["parents"]:
        for k, v in required(games, p, seen).items():
            out.setdefault(k, v)
    return out


def verdict(games, name, have, library, bios_pool):
    """Can this core run this set, given the CRCs we have?

    `library` maps romset -> set of CRCs, so parent and BIOS files stored in
    their own zips count as present."""
    if name not in games:
        return "unsupported", "no such driver in this DAT"
    if games[name]["chd"]:
        return "needs-chd", "set requires a CHD, which is a separate download"

    want = required(games, name)
    if not want:
        return "unknown", "DAT lists no dumpable files"

    pool = set(have)
    for p in games[name]["parents"]:
        pool |= library.get(p, set())
        for gp in games.get(p, {}).get("parents", []):
            pool |= library.get(gp, set())

    missing_game = [
        n for n, (crc, bios) in want.items()
        if crc not in pool and not bios and crc not in bios_pool
    ]
    missing_bios = [
        n for n, (crc, bios) in want.items()
        if crc not in pool and (bios or crc in bios_pool)
    ]

    if missing_game:
        return "missing", (
            f"{len(missing_game)}/{len(want)} game files missing, e.g. {missing_game[0]}"
        )
    if missing_bios:
        # Not fatal: these are alternative regional BIOSes, and one is enough.
        return "ok", f"ok; {len(missing_bios)} unused BIOS variant(s) absent"
    return "ok", f"all {len(want)} files present"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--roms", required=True)
    ap.add_argument("--dats", required=True)
    ap.add_argument("--json")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()

    datdir = pathlib.Path(args.dats)
    sources = {
        "fbneo": datdir / "fbneo.dat",
        "mame": datdir / "mame_arcade.dat",
        "mame2003_plus": datdir / "mame2003plus.xml",
    }
    dats = {}
    bios = {}
    for core, path in sources.items():
        if not path.exists():
            print(f"skip {core}: {path} not found", file=sys.stderr)
            continue
        dats[core] = parse_dat(path)
        bios[core] = bios_crcs(dats[core])
        if not args.quiet:
            print(
                f"{core:16} {len(dats[core]):>6} sets, {len(bios[core]):>4} BIOS files",
                file=sys.stderr,
            )

    romdir = pathlib.Path(args.roms)
    library = {}
    for f in sorted(romdir.glob("*.zip")):
        crcs = zip_crcs(f)
        if crcs is not None:
            library[f.stem] = crcs
    if not args.quiet:
        print(f"{'local':16} {len(library):>6} zips read", file=sys.stderr)

    results = {}
    for name, have in sorted(library.items()):
        results[name] = {
            core: verdict(games, name, have, library, bios[core])
            for core, games in dats.items()
        }

    if args.json:
        pathlib.Path(args.json).write_text(json.dumps(results, indent=1))

    cores = list(dats)
    print(f"\n{'romset':<18}" + "".join(f"{c:>16}" for c in cores))
    for name, per in sorted(results.items()):
        print(f"{name:<18}" + "".join(f"{per[c][0]:>16}" for c in cores))

    print(f"\n{'core':<18}{'runnable':>10}{'share':>8}")
    for c in cores:
        ok = sum(1 for r in results.values() if r[c][0] == "ok")
        print(f"{c:<18}{f'{ok}/{len(results)}':>10}{ok / max(len(results), 1) * 100:>7.0f}%")


if __name__ == "__main__":
    main()
