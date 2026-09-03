#!/usr/bin/env python3
"""Stage games and box art onto a MinUI/NextUI card.

MinUI's layout is filename-driven, not database-driven:

    Roms/<Name> (TAG)/<rom file>
    Roms/<Name> (TAG)/.media/<rom file without extension>.png

The folder's parenthesised TAG is what maps it to an emulator, so the folders
NextUI created on first boot are used as-is rather than invented here. Several
RomM platforms share one NextUI folder — NES and Famicom are both `(FC)`, SNES
and Super Famicom both `(SFC)` — which is why the mapping is many-to-one.

Art comes from the ES-DE miximages already in `library/downloaded_media`, which
are the only art type that is a consistent shape across every console here
(1280x960 throughout, while covers run from 0.73 to 1.37 aspect). They are
downscaled to 512px on the long edge: the panel is 720x480, and shipping 570 KB
scrape-resolution PNGs would cost more card space than the games do.

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

# RomM platform slug -> the folder NextUI created. Anything not here has no
# folder on the card, and a folder invented for it would have no emulator.
DEST = {
    "gba": "Game Boy Advance (GBA)",
    "gb": "Game Boy (GB)",
    "gbc": "Game Boy Color (GBC)",
    "nes": "Nintendo Entertainment System (FC)",
    "famicom": "Nintendo Entertainment System (FC)",
    "snes": "Super Nintendo Entertainment System (SFC)",
    "sfc": "Super Nintendo Entertainment System (SFC)",
    "megadrive": "Sega Genesis (MD)",
    "mastersystem": "Sega Master System (SMS)",
    "gamegear": "Sega Game Gear (GG)",
    "pcengine": "TurboGrafx-16 (PCE)",
    "neo-geo-pocket": "Neo Geo Pocket Color (NGPC)",
}

ART_KINDS = ["miximages", "covers"]      # in order of preference
ART_EXTS = ["png", "jpg", "webp"]
ART_PX = 512


def art_for(media_root, slug, stem):
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
    # A folder ROM arrives as a zip of its members, so its size never equals the
    # sum of them. Only flat files can be size-checked.
    if expected and not multi and written != expected:
        raise IOError(f"size mismatch: expected {expected}, got {written}")
    part.replace(dest)


def human(b):
    return f"{b/1024**3:.2f} GB" if b >= 1024**3 else f"{b/1024**2:.0f} MB"


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--card", default="/Volumes/BASEOS")
    ap.add_argument("--library", default="library/roms")
    ap.add_argument("--media", default="library/downloaded_media")
    ap.add_argument("--cache", default="cache.sqlite3")
    ap.add_argument("--config", default="config.toml")
    ap.add_argument("--platform", help="only this RomM slug")
    ap.add_argument("--no-art", action="store_true")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    card = pathlib.Path(args.card)
    roms_root = card / "Roms"
    if not roms_root.is_dir():
        sys.exit(f"no Roms folder at {roms_root} — is the NextUI card mounted?")
    lib, media = pathlib.Path(args.library), pathlib.Path(args.media)

    cfg = tomllib.loads(pathlib.Path(args.config).read_text())["server"]
    auth, base = f"Bearer {cfg['token'].strip()}", cfg["url"].rstrip("/")
    db = sqlite3.connect(args.cache)

    slugs = [args.platform] if args.platform else list(DEST)
    items, missing_dest = [], set()
    for slug in slugs:
        folder = roms_root / DEST[slug]
        if not folder.is_dir():
            missing_dest.add(DEST[slug])
            continue
        for rid, fs, size, multi in db.execute(
            "SELECT id, fs_name, COALESCE(fs_size_bytes,0), multi_file FROM roms "
            "WHERE platform_slug = ? ORDER BY fs_name", (slug,)):
            stem = pathlib.PurePath(fs).stem
            items.append({"id": rid, "slug": slug, "fs": fs, "stem": stem,
                          "size": size, "multi": bool(multi), "folder": folder,
                          "art": None if args.no_art else art_for(media, slug, stem)})

    by = {}
    for i in items:
        b = by.setdefault(i["folder"].name, {"n": 0, "bytes": 0, "art": 0, "have": 0})
        b["n"] += 1
        b["bytes"] += i["size"]
        b["art"] += bool(i["art"])
        b["have"] += (lib / i["slug"] / i["fs"]).exists()

    print(f"{'folder':44} {'games':>6} {'size':>10} {'art':>6} {'local':>6}")
    for name in sorted(by, key=lambda k: -by[k]["bytes"]):
        b = by[name]
        print(f"{name:44} {b['n']:6,} {human(b['bytes']):>10} {b['art']:6,} {b['have']:6,}")
    tot = sum(i["size"] for i in items)
    need = sum(i["size"] for i in items if not (lib / i["slug"] / i["fs"]).exists())
    print(f"\n{len(items):,} games, {human(tot)} — {human(need)} to download")
    print(f"{sum(1 for i in items if i['art']):,} have art, "
          f"{sum(1 for i in items if not i['art']):,} do not")
    if missing_dest:
        print(f"\nno such folder on the card: {', '.join(sorted(missing_dest))}")
    if not args.apply:
        print("\ndry run — nothing written. Re-run with --apply.")
        return

    def art_job(i):
        out = i["folder"] / ".media" / f"{i['stem']}.png"
        if out.exists():
            return
        # sips is on every Mac; no Pillow, no Homebrew. -Z fits the long edge,
        # so a 1280x960 miximage lands at 512x384.
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
            (f / ".media").mkdir(exist_ok=True)
        print(f"\nresizing {len(arted):,} images to {ART_PX}px…")
        with ThreadPoolExecutor(max_workers=8) as ex:
            list(ex.map(art_job, arted))

    # Finder's ._ sidecars list as games in MinUI-family firmware.
    for f in sorted({i["folder"] for i in items}):
        subprocess.run(["dot_clean", "-m", str(f)], check=False)
    subprocess.run(["dot_clean", "-m", str(roms_root)], check=False)

    print(f"\ncopied {done}/{len(items)}")
    for name, err in failed[:20]:
        print(f"  failed: {name} — {err}")
    if len(failed) > 20:
        print(f"  ... and {len(failed)-20} more")


if __name__ == "__main__":
    main()
