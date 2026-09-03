#!/usr/bin/env python3
"""Stage games, box art and saves onto a spruceOS card.

spruce's layout, confirmed against the shipped 4.3.4 archive and its own
scraper source rather than inferred:

    Roms/<SYS>/<rom file>
    Roms/<SYS>/Imgs/<rom file without extension>.png   # box_art_scraper.py:556
    Saves/saves/<Core Display Name>/<rom file>.srm     # sort_savefiles_enable=true

Systems are copied smallest-first. PS and FBNEO are 79% of the bytes between
them, so putting them last means the handheld is usable while they finish
rather than after.

Saves are copied under the core name they were written by, not the core spruce
defaults to. Three systems differ — GBA (gpSP vs mGBA), SFC (ChimeraSNES vs
Snes9x), MD (PicoDrive vs Genesis Plus GX) — and every one of them offers the
matching core in its own menu. Renaming the folder to suit the default would be
guessing that two cores agree on a battery format; switching the core is exact.

Dry by default. `--apply` is the only thing that writes.
"""

import argparse
import pathlib
import shutil
import sqlite3
import subprocess
import sys
import tomllib
import urllib.request
from concurrent.futures import ThreadPoolExecutor

# RomM slug -> spruce Roms folder, smallest system first.
DEST = [
    ("famicom", "FC"), ("gb", "GB"), ("nes", "FC"), ("gbc", "GBC"),
    ("snes", "SFC"), ("n64", "N64"), ("sfc", "SFC"), ("megadrive", "MD"),
    ("neogeoaes", "NEOGEO"), ("gba", "GBA"), ("arcade", "FBNEO"),
    ("dc", "DC"), ("psx", "PS"),
]

ART_KINDS = ["miximages", "covers"]
ART_EXTS = ["png", "jpg", "webp"]
ART_PX = 640
PREBUILT = pathlib.Path("library/media-640")


def art_for(media_root, slug, stem):
    # Prefer the set already built at the right size — no resize, straight copy.
    pre = PREBUILT / slug / f"{stem}.png"
    if pre.exists():
        return pre
    for kind in ART_KINDS:
        for ext in ART_EXTS:
            p = media_root / slug / kind / f"{stem}.{ext}"
            if p.exists():
                return p
    return None


def fetch(url, auth, dest, expected, multi):
    part = dest.with_suffix(dest.suffix + ".part")
    have = part.stat().st_size if part.exists() else 0
    req = urllib.request.Request(url, headers={"Authorization": auth})
    if have:
        req.add_header("Range", f"bytes={have}-")
    with urllib.request.urlopen(req, timeout=300) as r:
        appending = r.status == 206 and have > 0
        written = have if appending else 0
        with open(part, "ab" if appending else "wb") as f:
            while chunk := r.read(4 << 20):
                f.write(chunk)
                written += len(chunk)
    if expected and not multi and written != expected:
        raise IOError(f"size mismatch: expected {expected}, got {written}")
    part.replace(dest)


def human(b):
    return f"{b/1024**3:.2f} GB" if b >= 1024**3 else f"{b/1024**2:.0f} MB"


def copy_saves(src_root, card, apply):
    """Copy battery saves under the core folder that wrote them."""
    dest_root = card / "Saves" / "saves"
    rows, total = [], 0
    for core in sorted(p for p in src_root.iterdir() if p.is_dir()):
        files = [f for f in core.rglob("*") if f.is_file() and not f.name.startswith(".")]
        if not files:
            continue
        rows.append((core.name, len(files), sum(f.stat().st_size for f in files)))
        if not apply:
            continue
        for f in files:
            tgt = dest_root / core.name / f.relative_to(core)
            tgt.parent.mkdir(parents=True, exist_ok=True)
            if not tgt.exists():
                shutil.copy2(f, tgt)
                total += 1
    return rows, total


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--card", default="/Volumes/A30")
    ap.add_argument("--library", default="library/roms")
    ap.add_argument("--media", default="library/downloaded_media")
    ap.add_argument("--saves", default="backups/saves")
    ap.add_argument("--cache", default="cache.sqlite3")
    ap.add_argument("--config", default="config.toml")
    ap.add_argument("--skip", default="", help="comma-separated RomM slugs to leave out")
    ap.add_argument("--no-art", action="store_true")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    card = pathlib.Path(args.card)
    roms_root = card / "Roms"
    if not roms_root.is_dir():
        sys.exit(f"no Roms folder at {roms_root} — is the spruce card mounted?")
    lib, media = pathlib.Path(args.library), pathlib.Path(args.media)
    skip = {s.strip() for s in args.skip.split(",") if s.strip()}

    cfg = tomllib.loads(pathlib.Path(args.config).read_text())["server"]
    auth, base = f"Bearer {cfg['token'].strip()}", cfg["url"].rstrip("/")
    db = sqlite3.connect(args.cache)

    items = []
    for slug, sysname in DEST:
        if slug in skip:
            continue
        folder = roms_root / sysname
        if not folder.is_dir():
            print(f"skip {slug}: no {sysname} folder on the card", file=sys.stderr)
            continue
        q = ("SELECT id, fs_name, COALESCE(fs_size_bytes,0), multi_file FROM roms "
             "WHERE platform_slug = ?")
        if slug == "n64":
            # Two server rows are whole region folders indexed as one game,
            # 6.5 GB between them. Not games.
            q += " AND fs_name NOT IN ('USA','Europe')"
        for rid, fs, size, multi in db.execute(q + " ORDER BY fs_name", (slug,)):
            stem = pathlib.PurePath(fs).stem
            items.append({"id": rid, "slug": slug, "fs": fs, "stem": stem,
                          "size": size, "multi": bool(multi), "folder": folder,
                          "art": None if args.no_art else art_for(media, slug, stem)})

    order, seen = [], set()
    for _, s in DEST:
        if s not in seen:
            seen.add(s)
            order.append(s)
    by = {}
    for i in items:
        b = by.setdefault(i["folder"].name, {"n": 0, "bytes": 0, "art": 0})
        b["n"] += 1
        b["bytes"] += i["size"]
        b["art"] += bool(i["art"])
    print(f"{'system':10} {'games':>6} {'size':>10} {'art':>7}")
    for s in order:
        if s in by:
            b = by[s]
            print(f"{s:10} {b['n']:6,} {human(b['bytes']):>10} {b['art']:7,}")
    print(f"{'TOTAL':10} {len(items):6,} "
          f"{human(sum(i['size'] for i in items)):>10} "
          f"{sum(1 for i in items if i['art']):7,}")

    save_rows, _ = copy_saves(pathlib.Path(args.saves), card, False)
    print(f"\nsaves ({len(save_rows)} cores):")
    for name, n, b in sorted(save_rows, key=lambda r: -r[1]):
        print(f"  {name:22} {n:4} files {human(b):>9}")

    if not args.apply:
        print("\ndry run — nothing written. Re-run with --apply.")
        return

    _, nsaves = copy_saves(pathlib.Path(args.saves), card, True)
    print(f"\ncopied {nsaves} save files")

    def art_job(i):
        out = i["folder"] / "Imgs" / f"{i['stem']}.png"
        if not out.exists():
            subprocess.run(["sips", "-Z", str(ART_PX), "-s", "format", "png",
                            str(i["art"]), "--out", str(out)],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    done, failed, arted = 0, [], []
    for n, i in enumerate(items, 1):
        local = lib / i["slug"] / i["fs"]
        target = i["folder"] / i["fs"]
        try:
            if not local.exists():
                local.parent.mkdir(parents=True, exist_ok=True)
                fetch(f"{base}/api/roms/{i['id']}/content/"
                      f"{urllib.request.quote(i['fs'])}", auth, local, i["size"], i["multi"])
            if not (target.exists() and target.stat().st_size == local.stat().st_size):
                shutil.copy2(local, target)
                if target.stat().st_size != local.stat().st_size:
                    target.unlink(missing_ok=True)
                    raise IOError("short write to the card")
            done += 1
            if i["art"]:
                arted.append(i)
        except Exception as e:
            failed.append((i["fs"], str(e)))
        if n % 250 == 0 or n == len(items):
            print(f"  [{n}/{len(items)}] {done} copied, {len(failed)} failed")

    if arted:
        for f in {i["folder"] for i in arted}:
            (f / "Imgs").mkdir(exist_ok=True)
        print(f"\nresizing {len(arted):,} images to {ART_PX}px…")
        with ThreadPoolExecutor(max_workers=8) as ex:
            list(ex.map(art_job, arted))

    for f in sorted({i["folder"] for i in items} | {card / "Saves" / "saves"}):
        subprocess.run(["dot_clean", "-m", str(f)], check=False)

    print(f"\ncopied {done}/{len(items)}")
    for name, err in failed[:20]:
        print(f"  failed: {name} — {err}")


if __name__ == "__main__":
    main()
