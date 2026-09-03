#!/usr/bin/env python3
"""Match the drive manifest against the library, per platform.

Answers the question the manifest cannot on its own: of the 30,802 games on the
external drive, which are already in the library, which are on the drive and
missing from the library, and which are in the library and not on the drive.

Titles are matched, not filenames. `norm` is imported from
`community_favorites` rather than copied, so a game matched here and a game
matched against a published ranking there are matched the same way — two
normalisers that drift apart would put a game in a "best of" collection and
then fail to find the very same game on the drive.

The platform map is the judgement call in here and it is written down rather
than inferred. The drive splits some systems the library keeps together — a
Mega Drive game may sit under `megadrive` or `genesis`, a Neo Geo Pocket game
under `ngp` or `ngpc` — so several drive directories can feed one library slug.
Anything unmapped is reported, never silently dropped.

    python3 tools/drive_vs_library.py
    python3 tools/drive_vs_library.py --platform snes --show-missing 40
"""

import argparse
import collections
import gzip
import json
import pathlib
import sqlite3
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from community_favorites import norm  # noqa: E402

# drive directory -> library platform slug. Several to one where the drive
# separates a system the library does not: `genesis` and `megadrive` are the
# same console under two names, and the library calls it `megadrive`.
PLATFORM_MAP = {
    "3do": "3do",
    "dreamcast": "dc",
    "famicom": "famicom",
    "fds": "famicom",
    "gamecube": "ngc",
    "gamegear": "gamegear",
    "gb": "gb",
    "gba": "gba",
    "gbc": "gbc",
    "genesis": "megadrive",
    "megadrive": "megadrive",
    "mastersystem": "mastersystem",
    "n64": "n64",
    "nds": "nds",
    "nes": "nes",
    "ngp": "neo-geo-pocket",
    "ngpc": "neo-geo-pocket",
    "pcengine": "pcengine",
    "pcenginecd": "pcengine",
    "turbografx16": "pcengine",
    "turbografxcd": "pcengine",
    "psp": "psp",
    "pspminis": "psp",
    "psx": "psx",
    "sfc": "sfc",
    "satellaview": "sfc",
    "snes": "snes",
    "sgb": "gb",
    "wswanc": "wonderswancolor",
    # Consoles the drive carries that the library has no games for at all.
    # Mapping them is what lets a "best of" list say "all 16 of these are on
    # the drive" rather than "all 16 are missing" — the difference between a
    # copy job and a shopping list.
    "saturn": "saturn",
}

# Folders that hold *versions* of games rather than the games. Mapping them in
# made the planner prefer an 981 MB MSU-1 audio hack of Super Mario RPG over
# the 4 MB cartridge, because it picks the largest file per title and an MSU
# build is two hundred times the size. Four of them reached the server before
# the sizes looked wrong. The `-h` folders are hack collections; `snes-msu`
# needs core support and extra PCM tracks and is not what a "best of" list
# means when it says Kirby Super Star.
VARIANT_FOLDERS = {
    "snes-msu", "nesh", "gbah", "genesish", "megadriveh", "mastersystemh",
}



def open_manifest(path):
    """Open the manifest, preferring the committed `.gz` when the plain file is absent.

    The uncompressed manifest is 8.8 MB and regenerating it needs the drive
    plugged in; gzipped it is 1.0 MB and lives in the repo. Everything
    downstream — deciding what to fetch, what to copy, what is missing — is
    answerable without the drive, so the drive should not be a prerequisite for
    running any of it.
    """
    p = pathlib.Path(path)
    if p.exists():
        return p.open(encoding="utf-8")
    gz = p.with_suffix(p.suffix + ".gz")
    if gz.exists():
        return gzip.open(gz, "rt", encoding="utf-8")
    raise SystemExit(f"no manifest at {p} or {gz} — run tools/drive_manifest.py with the drive mounted")


def load_manifest(path):
    by_platform = collections.defaultdict(dict)
    with open_manifest(path) as fh:
        for line in fh:
            r = json.loads(line)
            slug = PLATFORM_MAP.get(r["platform"])
            if not slug:
                continue
            # Keep the largest file for a title: where a drive holds several
            # regions of one game, one entry per title is what a comparison
            # against the library wants.
            key = norm(r["name"])
            if not key:
                continue
            prev = by_platform[slug].get(key)
            if prev is None or (r["size"] or 0) > (prev["size"] or 0):
                by_platform[slug][key] = r
    return by_platform


def load_library(db):
    con = sqlite3.connect(db)
    lib = collections.defaultdict(dict)
    for slug, name in con.execute(
        "select platform_slug, COALESCE(NULLIF(name,''), fs_name) from roms"
    ):
        lib[slug][norm(name)] = name
    con.close()
    return lib


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--manifest", default="drive-manifest/manifest.jsonl")
    ap.add_argument("--db", default="cache.sqlite3")
    ap.add_argument("--out", default="drive-manifest")
    ap.add_argument("--platform", help="report one platform in detail")
    ap.add_argument("--show-missing", type=int, default=0,
                    help="print this many drive-only titles per platform")
    args = ap.parse_args()

    drive = load_manifest(args.manifest)
    lib = load_library(args.db)
    out_dir = pathlib.Path(args.out)

    slugs = [args.platform] if args.platform else sorted(set(drive) | set(lib))
    rows, gap = [], {}
    for slug in slugs:
        d, l = drive.get(slug, {}), lib.get(slug, {})
        both = set(d) & set(l)
        only_drive = sorted(set(d) - set(l))
        only_lib = sorted(set(l) - set(d))
        rows.append((slug, len(l), len(d), len(both), len(only_drive), len(only_lib)))
        if only_drive:
            gap[slug] = [d[k]["name"] for k in only_drive]
        if args.show_missing and only_drive:
            print(f"\n{slug} — {len(only_drive)} on the drive, not in the library:")
            for k in only_drive[: args.show_missing]:
                print(f"  {d[k]['name']}")

    print(f"\n{'platform':18}{'library':>9}{'drive':>8}{'both':>8}{'drive only':>12}{'lib only':>10}")
    for r in sorted(rows, key=lambda r: -r[4]):
        print(f"{r[0]:18}{r[1]:9,}{r[2]:8,}{r[3]:8,}{r[4]:12,}{r[5]:10,}")
    tot = [sum(r[i] for r in rows) for i in range(1, 6)]
    print(f"{'TOTAL':18}{tot[0]:9,}{tot[1]:8,}{tot[2]:8,}{tot[3]:12,}{tot[4]:10,}")

    if not args.platform:
        (out_dir / "drive-only.json").write_text(json.dumps(gap, indent=1, ensure_ascii=False))
        print(f"\ndrive-only titles -> {out_dir}/drive-only.json")

        unmapped = collections.Counter()
        with open_manifest(args.manifest) as fh:
            for line in fh:
                p = json.loads(line)["platform"]
                if p not in PLATFORM_MAP:
                    unmapped[p] += 1
        if unmapped:
            print(f"\n{sum(unmapped.values()):,} games on {len(unmapped)} drive platforms "
                  f"the library has no slug for:")
            print("  " + ", ".join(f"{p} ({n:,})" for p, n in unmapped.most_common(12))
                  + (", …" if len(unmapped) > 12 else ""))


if __name__ == "__main__":
    main()
