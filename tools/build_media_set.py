#!/usr/bin/env python3
"""Build a resized box-art set once, so cards can be filled without re-resizing.

Reads `library/downloaded_media/<slug>/miximages/` and writes
`library/media-640/<slug>/<rom stem>.png`. **Nothing in downloaded_media is
touched** — this only ever writes into its own output folder.

640x480 because that is what spruceOS asks for: `device_common.py` returns
(640, 480) from all three of `get_boxart_{small,medium,large}_resize_dimensions`,
and it matches the Miyoo Flip's panel exactly. Supplying anything larger is
thrown away on device; anything smaller is what the A30 card got by mistake.

Miximages only. spruce's PyUI references `Imgs`, `Imgs_large`, `Imgs_med` and
`Imgs_small` and nothing else — no video, no manuals — so one image per game is
the whole requirement.
"""

import argparse
import pathlib
import sqlite3
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

EXTS = ["png", "jpg", "webp"]


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--slugs", required=True, help="comma-separated RomM slugs")
    ap.add_argument("--media", default="library/downloaded_media")
    ap.add_argument("--out", default="library/media-640")
    ap.add_argument("--px", type=int, default=640)
    ap.add_argument("--cache", default="cache.sqlite3")
    ap.add_argument("--kind", default="miximages")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    src, out = pathlib.Path(args.media), pathlib.Path(args.out)
    if out.resolve() == src.resolve() or src in out.resolve().parents:
        sys.exit("refusing to write inside the source media tree")
    db = sqlite3.connect(args.cache)

    jobs, missing = [], 0
    for slug in [s.strip() for s in args.slugs.split(",") if s.strip()]:
        q = "SELECT fs_name FROM roms WHERE platform_slug=?"
        if slug == "n64":
            q += " AND fs_name NOT IN ('USA','Europe')"
        for (fs,) in db.execute(q, (slug,)):
            stem = pathlib.PurePath(fs).stem
            found = next((src / slug / args.kind / f"{stem}.{e}"
                          for e in EXTS if (src / slug / args.kind / f"{stem}.{e}").exists()), None)
            if found is None:
                missing += 1
                continue
            jobs.append((found, out / slug / f"{stem}.png"))

    todo = [j for j in jobs if not j[1].exists()]
    print(f"{len(jobs):,} images available, {missing:,} games without one")
    print(f"{len(todo):,} to build at {args.px}px -> {out}")
    if not args.apply:
        print("\ndry run — nothing written. Re-run with --apply.")
        return

    for d in {t.parent for _, t in todo}:
        d.mkdir(parents=True, exist_ok=True)

    done = [0]

    def job(pair):
        s, t = pair
        subprocess.run(["sips", "-Z", str(args.px), "-s", "format", "png",
                        str(s), "--out", str(t)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        done[0] += 1
        if done[0] % 500 == 0:
            print(f"  {done[0]:,}/{len(todo):,}")

    with ThreadPoolExecutor(max_workers=8) as ex:
        list(ex.map(job, todo))

    built = sum(1 for _, t in jobs if t.exists())
    size = sum(t.stat().st_size for _, t in jobs if t.exists())
    print(f"\n{built:,} images in {out}  ({size/1024**3:.2f} GB)")


if __name__ == "__main__":
    main()
