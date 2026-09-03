#!/usr/bin/env python3
"""Copy ES-DE artwork off the handheld's card into this app's media cache.

The card is the authority. It sits at 97% coverage across the library while the
RomM server sits at 71%, because the server holds a partial, differently
organised copy — arcade under `mame` rather than `arcade`, and thousands of
games absent entirely. Scraping those back one at a time from ScreenScraper took
51 minutes to bring one console to 87%; the card had it at 99% the whole time.

Two things this has to get right.

**Folder names.** ES-DE names systems its own way and RomM names them another:
`genesis` against `megadrive`, `neogeo` against `neogeoaes`, `gc` against `ngc`,
`dreamcast` against `dc`. The app looks under RomM's name, so the copy renames
on the way in. The mapping is derived by matching filenames rather than typed
out, because a wrong pair is silent — the files land somewhere nothing looks.

**What to copy.** Only media for games actually in the library. The card serves a
handheld with its own romset, so it carries art for games this library does not
have, and copying those is bytes for nothing.

    scripts/import-esde-media.py [--videos] [--dry-run] [--dest DIR]
"""

import argparse
import os
import shutil
import sqlite3
import sys
import time

CARD = "/Volumes/Retro/ES-DE/support/downloaded_media"
CACHE_DB = "cache.sqlite3"

# Everything ES-DE stores. Videos are separate because they are half the bytes.
PICTURE_KINDS = [
    "miximages", "covers", "backcovers", "3dboxes", "physicalmedia",
    "screenshots", "titlescreens", "marquees", "fanart", "manuals",
]
VIDEO_KINDS = ["videos"]


def card_dir_for(platform, stems, card):
    """The card folder holding this platform's art, by trying each candidate.

    Matching on filenames rather than trusting a table: the two naming schemes
    agree for most systems and disagree silently for the rest, and a wrong guess
    puts the art where nothing will look for it.
    """
    candidates = [platform] + {
        "megadrive": ["genesis"],
        "neogeoaes": ["neogeo"],
        "ngc": ["gc"],
        "dc": ["dreamcast"],
        "neo-geo-pocket": ["ngp"],
        "arcade": ["arcade", "mame"],
        "sfc": ["sfc", "snes"],
    }.get(platform, [])

    best, best_hits = None, 0
    for name in candidates:
        got = set()
        for kind in ("miximages", "covers", "physicalmedia"):
            d = os.path.join(card, name, kind)
            if os.path.isdir(d):
                got |= {os.path.splitext(f)[0] for f in os.listdir(d)}
        hits = len(stems & got)
        if hits > best_hits:
            best, best_hits = name, hits
    return best, best_hits


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--videos", action="store_true", help="also copy gameplay videos")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--card", default=CARD)
    ap.add_argument("--dest", default="library/downloaded_media")
    args = ap.parse_args()

    if not os.path.isdir(args.card):
        sys.exit(f"card not mounted at {args.card}")

    kinds = PICTURE_KINDS + (VIDEO_KINDS if args.videos else [])
    db = sqlite3.connect(CACHE_DB)
    platforms = [r[0] for r in db.execute("SELECT DISTINCT platform_slug FROM roms ORDER BY 1")]

    planned, planned_bytes, copied, copied_bytes, skipped = 0, 0, 0, 0, 0
    started = time.time()

    for platform in platforms:
        stems = {
            os.path.splitext(r[0])[0]
            for r in db.execute("SELECT fs_name FROM roms WHERE platform_slug = ?", (platform,))
        }
        name, hits = card_dir_for(platform, stems, args.card)
        if not name:
            print(f"{platform:16} no folder on the card")
            continue

        for kind in kinds:
            src_dir = os.path.join(args.card, name, kind)
            if not os.path.isdir(src_dir):
                continue
            dst_dir = os.path.join(args.dest, platform, kind)
            for f in os.listdir(src_dir):
                if os.path.splitext(f)[0] not in stems:
                    continue
                src, dst = os.path.join(src_dir, f), os.path.join(dst_dir, f)
                try:
                    size = os.path.getsize(src)
                except OSError:
                    continue
                # Already here and the same size: the card and the cache hold
                # copies of the same file, so size is enough and hashing 90,000
                # files to learn nothing is not.
                if os.path.exists(dst) and os.path.getsize(dst) == size:
                    skipped += 1
                    continue
                planned += 1
                planned_bytes += size
                if args.dry_run:
                    continue
                os.makedirs(dst_dir, exist_ok=True)
                try:
                    shutil.copy2(src, dst)
                    copied += 1
                    copied_bytes += size
                except OSError as e:
                    print(f"  {platform}/{kind}/{f}: {e}")
                if copied and copied % 2000 == 0:
                    rate = copied_bytes / max(time.time() - started, 1) / 1e6
                    print(f"  {copied:,} files, {copied_bytes/1e9:.1f} GB, {rate:.0f} MB/s")

        print(f"{platform:16} <- {name:16} {hits} games matched")

    if args.dry_run:
        print(f"\nwould copy {planned:,} files, {planned_bytes/1e9:.1f} GB "
              f"({skipped:,} already here)")
    else:
        el = time.time() - started
        print(f"\ncopied {copied:,} files, {copied_bytes/1e9:.1f} GB in {el/60:.0f} min "
              f"({skipped:,} already here)")


if __name__ == "__main__":
    main()
