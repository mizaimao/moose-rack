#!/usr/bin/env python3
"""Stage games, art, gamelists, BIOS and PICO-8 onto an ArkOS/dArkOS card.

EmulationStation is different from the MinUI-family firmwares in one way that
shapes this whole script: **it reads the artwork path out of gamelist.xml**
rather than looking in a fixed folder. NextUI hardcodes `.media/<stem>.png` and
spruce hardcodes `Imgs/<stem>.png`; ES follows whatever `<image>` says. So the
layout here is ours to choose and only has to be self-consistent:

    <system>/<rom>
    <system>/images/<rom stem>.png
    <system>/gamelist.xml     name, image, desc, developer, genre, players, rating

That also means the gamelist can carry the metadata already in cache.sqlite3 —
summaries, genres, companies, release dates, ratings — which is richer than the
on-device scraper would produce. Note that running dArkOS's own scraper later
may rewrite these files and repoint them at its own paths.

Favourites are `<favorite>true</favorite>` in the same file, so the "Best of"
lists are applied here rather than needing a separate store.

Multi-disc games go in a dot-prefixed folder with the .m3u beside it; ES skips
hidden folders by default, so only the playlist shows in the list.

Dry by default. `--apply` is the only thing that writes.
"""

import argparse
import html
import json
import pathlib
import shutil
import sqlite3
import subprocess
import sys
import tomllib
import urllib.request
from concurrent.futures import ThreadPoolExecutor

# RomM slug -> ArkOS folder. Only systems asked for; ArkOS keeps nes/famicom and
# snes/sfc separate, so unlike the MinUI-family cards there are no collisions.
DEST = [
    ("famicom", "nes"), ("gb", "gb"), ("nes", "nes"), ("gbc", "gbc"),
    ("snes", "snes"), ("n64", "n64"), ("sfc", "snes"), ("megadrive", "megadrive"),
    ("neogeoaes", "neogeo"), ("gba", "gba"), ("arcade", "fbneo"),
    ("dc", "dreamcast"), ("psx", "psx"),
]

# Server collections whose members become ES favourites.
BEST_OF_PREFIX = "★ Best of "

ART_KINDS = ["miximages", "covers"]
ART_EXTS = ["png", "jpg", "webp"]
ART_PX = 640
PREBUILT = pathlib.Path("library/media-640")


def art_for(media_root, slug, stem):
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


def meta_fields(name, summary, meta_json):
    """Turn a cache row into the ES gamelist fields it can fill."""
    out = {}
    if name:
        out["name"] = name
    if summary:
        out["desc"] = summary
    try:
        m = json.loads(meta_json or "{}")
    except Exception:
        m = {}
    if m.get("genres"):
        out["genre"] = ", ".join(m["genres"])
    if m.get("companies"):
        # dedupe, keep order: the first is the developer in this data
        seen, comp = set(), []
        for c in m["companies"]:
            if c not in seen:
                seen.add(c)
                comp.append(c)
        out["developer"] = comp[0]
        if len(comp) > 1:
            out["publisher"] = comp[1]
    if m.get("first_release_date"):
        try:
            import datetime
            d = datetime.datetime.fromtimestamp(
                m["first_release_date"] / 1000, datetime.timezone.utc)
            out["releasedate"] = d.strftime("%Y%m%dT%H%M%S")
        except Exception:
            pass
    if m.get("average_rating"):
        # ES wants 0.0-1.0; the cache holds 0-100
        out["rating"] = f"{max(0.0, min(1.0, m['average_rating'] / 100)):.2f}"
    if m.get("player_count"):
        out["players"] = str(m["player_count"])
    return out


def write_gamelist(folder, entries):
    """entries: list of (relpath, fields dict, favourite bool)"""
    lines = ['<?xml version="1.0"?>', "<gameList>"]
    for rel, fields, fav in entries:
        lines.append("\t<game>")
        lines.append(f"\t\t<path>{html.escape(rel)}</path>")
        for k in ("name", "desc", "image", "releasedate", "developer",
                  "publisher", "genre", "players", "rating"):
            if fields.get(k):
                lines.append(f"\t\t<{k}>{html.escape(str(fields[k]))}</{k}>")
        if fav:
            lines.append("\t\t<favorite>true</favorite>")
        lines.append("\t</game>")
    lines.append("</gameList>\n")
    (folder / "gamelist.xml").write_text("\n".join(lines), encoding="utf-8")


def human(b):
    return f"{b/1024**3:.2f} GB" if b >= 1024**3 else f"{b/1024**2:.0f} MB"


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--card", default="/Volumes/EASYROMS")
    ap.add_argument("--library", default="library/roms")
    ap.add_argument("--media", default="library/downloaded_media")
    ap.add_argument("--system", default="library/system")
    ap.add_argument("--pico8", default="cfw/pico-8_0.2.7_raspi.zip")
    ap.add_argument("--cache", default="cache.sqlite3")
    ap.add_argument("--config", default="config.toml")
    ap.add_argument("--only", help="one RomM slug")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    card = pathlib.Path(args.card)
    if not (card / "roms").is_dir():
        sys.exit(f"{card} does not look like a KNULLI SHARE partition")
    lib, media = pathlib.Path(args.library), pathlib.Path(args.media)

    cfg = tomllib.loads(pathlib.Path(args.config).read_text())["server"]
    auth, base = f"Bearer {cfg['token'].strip()}", cfg["url"].rstrip("/")
    db = sqlite3.connect(args.cache)

    # Best-of membership -> favourites, keyed by (slug, fs_name)
    favs = set()
    for cid, name in db.execute(
        "SELECT id,name FROM collections WHERE grp='user' AND name LIKE ?",
        (BEST_OF_PREFIX + "%",)):
        for fs, slug in db.execute(
            "SELECT r.fs_name, r.platform_slug FROM collection_roms cr "
            "JOIN roms r ON r.id=cr.rom_id WHERE cr.collection_id=?", (cid,)):
            favs.add((slug, fs))

    items = []
    for slug, sysname in DEST:
        if args.only and slug != args.only:
            continue
        folder = card / "roms" / sysname
        if not folder.is_dir():
            print(f"skip {slug}: no {sysname} folder", file=sys.stderr)
            continue
        q = ("SELECT id, fs_name, name, summary, meta_json, "
             "COALESCE(fs_size_bytes,0), multi_file FROM roms WHERE platform_slug=?")
        if slug == "n64":
            q += " AND fs_name NOT IN ('USA','Europe')"
        for rid, fs, nm, summ, mj, size, multi in db.execute(q + " ORDER BY fs_name", (slug,)):
            stem = pathlib.PurePath(fs).stem
            items.append({
                "id": rid, "slug": slug, "fs": fs, "stem": stem, "size": size,
                "multi": bool(multi), "folder": folder,
                "art": art_for(media, slug, stem),
                "fields": meta_fields(nm, summ, mj),
                "fav": (slug, fs) in favs,
            })

    by = {}
    for i in items:
        b = by.setdefault(i["folder"].name, {"n": 0, "bytes": 0, "art": 0, "fav": 0, "desc": 0})
        b["n"] += 1; b["bytes"] += i["size"]
        b["art"] += bool(i["art"]); b["fav"] += i["fav"]
        b["desc"] += bool(i["fields"].get("desc"))
    print(f"{'folder':14} {'games':>6} {'size':>10} {'art':>6} {'desc':>6} {'fav':>5}")
    for name in sorted(by, key=lambda k: -by[k]["bytes"]):
        b = by[name]
        print(f"{name:14} {b['n']:6,} {human(b['bytes']):>10} {b['art']:6,} {b['desc']:6,} {b['fav']:5,}")
    print(f"{'TOTAL':14} {len(items):6,} {human(sum(i['size'] for i in items)):>10} "
          f"{sum(1 for i in items if i['art']):6,} "
          f"{sum(1 for i in items if i['fields'].get('desc')):6,} "
          f"{sum(1 for i in items if i['fav']):5,}")

    if not args.apply:
        print("\ndry run — nothing written. Re-run with --apply.")
        return

    # ---- BIOS -------------------------------------------------------------
    src_sys = pathlib.Path(args.system)
    man = json.load(open("data/bios-manifest.json"))
    CORES = {"fceumm", "snes9x", "snes9x2010", "gambatte", "mgba", "gpsp",
             "genesis_plus_gx", "picodrive", "pcsx_rearmed", "swanstation",
             "fbalpha2012", "fbneo", "mame2003_plus", "flycast", "geolith",
             "mupen64plus", "parallel_n64"}
    nb = 0
    for e in man:
        if not (set(e.get("cores", [])) & CORES):
            continue
        s = src_sys / e["path"]
        if not s.exists():
            continue
        t = card / "bios" / pathlib.PurePath(e["path"]).name
        if not t.exists():
            shutil.copy2(s, t); nb += 1
    print(f"\nbios: {nb} files -> {card/'bios'}")

    # ---- PICO-8 -----------------------------------------------------------
    p8 = pathlib.Path(args.pico8)
    if p8.exists():
        tmp = pathlib.Path("/tmp/p8_arkos"); shutil.rmtree(tmp, ignore_errors=True)
        tmp.mkdir(parents=True)
        subprocess.run(["unzip", "-q", "-j", str(p8), "pico-8/*", "-d", str(tmp)], check=False)
        n8 = 0
        for f in tmp.iterdir():
            # 64-bit device: the wiki says drop pico8_dyn so pico8_64 is used
            if f.name == "pico8_dyn":
                continue
            t = card / "roms" / "pico8" / f.name
            if not t.exists():
                shutil.copy2(f, t); n8 += 1
        print(f"pico8: {n8} files -> {card/'pico-8'} (pico8_dyn omitted, 64-bit device)")
    else:
        print(f"pico8: {p8} not found, skipped")

    # ---- games ------------------------------------------------------------
    lists, done, failed, arted = {}, 0, [], []
    for n, i in enumerate(items, 1):
        local = lib / i["slug"] / i["fs"]
        target = i["folder"] / i["fs"]
        rel = f"./{i['fs']}"
        try:
            if local.is_dir():
                # multi-disc: discs into a hidden folder, playlist beside it
                discs = sorted(f for f in local.iterdir() if f.suffix.lower() == ".chd")
                m3u = next((f for f in local.iterdir() if f.suffix.lower() == ".m3u"), None)
                if not discs:
                    raise IOError("folder with no discs")
                hidden = i["folder"] / f".{i['fs']}"
                hidden.mkdir(parents=True, exist_ok=True)
                for f in discs:
                    t = hidden / f.name
                    if not t.exists() or t.stat().st_size != f.stat().st_size:
                        shutil.copy2(f, t)
                rel = f"./{i['fs']}.m3u"
                (i["folder"] / f"{i['fs']}.m3u").write_text(
                    "".join(f".{i['fs']}/{f.name}\n" for f in discs))
            else:
                if not local.exists():
                    local.parent.mkdir(parents=True, exist_ok=True)
                    fetch(f"{base}/api/roms/{i['id']}/content/"
                          f"{urllib.request.quote(i['fs'])}", auth, local, i["size"], i["multi"])
                if not (target.exists() and target.stat().st_size == local.stat().st_size):
                    shutil.copy2(local, target)
                    if target.stat().st_size != local.stat().st_size:
                        target.unlink(missing_ok=True)
                        raise IOError("short write to the card")
            fields = dict(i["fields"])
            if i["art"]:
                fields["image"] = f"./images/{i['stem']}-image.png"
                arted.append(i)
            lists.setdefault(i["folder"], []).append((rel, fields, i["fav"]))
            done += 1
        except Exception as e:
            failed.append((i["fs"], str(e)))
        if n % 250 == 0 or n == len(items):
            print(f"  [{n}/{len(items)}] {done} copied, {len(failed)} failed")

    # ---- art --------------------------------------------------------------
    if arted:
        for f in {i["folder"] for i in arted}:
            (f / "images").mkdir(exist_ok=True)
        print(f"\nresizing {len(arted):,} images to {ART_PX}px…")

        def job(i):
            out = i["folder"] / "images" / f"{i['stem']}-image.png"
            if out.exists():
                return
            if i["art"].parent.parent == PREBUILT:
                shutil.copy2(i["art"], out)
            else:
                subprocess.run(["sips", "-Z", str(ART_PX), "-s", "format", "png",
                                str(i["art"]), "--out", str(out)],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        with ThreadPoolExecutor(max_workers=8) as ex:
            list(ex.map(job, arted))

    # ---- gamelists --------------------------------------------------------
    for folder, entries in lists.items():
        write_gamelist(folder, sorted(entries, key=lambda e: e[0].lower()))
        print(f"  gamelist.xml: {folder.name} ({len(entries)} games, "
              f"{sum(1 for e in entries if e[2])} favourites)")

    for f in sorted({i["folder"] for i in items} | {card / "bios", card / "roms" / "pico8"}):
        subprocess.run(["dot_clean", "-m", str(f)], check=False)

    print(f"\ncopied {done}/{len(items)}")
    for name, err in failed[:20]:
        print(f"  failed: {name} — {err}")


if __name__ == "__main__":
    main()
