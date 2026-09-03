#!/usr/bin/env python3
"""Copy games the server has and the ES-DE card does not, onto the card.

Two hops, because there is no one-shot path. `get` writes into the configured
library root under RomM's slug (`dc`, `ngc`, `megadrive`), and the card is an
ES-DE tree that reads `dreamcast`, `gc`, `genesis`. Pointing the library root at
the card would fill folders the device never looks in. So: download into
`library/roms/<slug>/` first, then copy across under the ES-DE name. The local
copy is not waste — it is what makes a second run nearly free after a failure.

**Only systems the card already has are considered.** A folder this script
created would be a system the device has no core mapped for, which looks like a
bug on the handheld and is one here.

Matching is on two keys, not one. Filenames alone called `CIRCUS.NES` missing
when the card had `Circus Caper (USA).nes`, and called `Mario Kart DS (USA).7z`
missing when the card had the multi-language dump of the same game. So a server
game is absent only if neither its filename stem nor its identified name
matches anything on the card.

Dry by default. `--apply` is the only thing that writes.
"""

import argparse
import os
import pathlib
import re
import shutil
import sqlite3
import subprocess
import sys
import tomllib
import urllib.request

# Card folder -> server slug. Only these are considered; anything not listed
# either has no folder on the card or was deliberately left out.
SYSTEMS = {
    "psx": "psx", "n64": "n64", "snes": "snes", "genesis": "megadrive",
    "gb": "gb", "gbc": "gbc", "gba": "gba", "pcengine": "pcengine",
    "nds": "nds", "dreamcast": "dc", "3do": "3do", "gc": "ngc",
}

# Named exclusions, each for its own reason:
#   the two GameCube Bonus Discs are demo discs, not games
#   Animal Crossing's .rvz is 20 MB, which is not a GameCube disc — a bad dump
#   the N64 region folders are whole romsets the server indexed as one game
EXCLUDE = {
    "Mario Kart - Double Dash!! (USA) (Bonus Disc).rvz",
    "Metroid Prime 2 - Echoes (USA) (Bonus Disc).rvz",
    "Animal Crossing (USA).rvz",
    "USA", "Europe",
}

ROM_EXTS = (r"zip|7z|chd|cue|bin|iso|m3u|pce|sfc|smc|nes|gba|gbc?|n64|z64|v64|"
            r"md|gen|sms|gg|wsc?|ngp|ngc|nds|cso|pbp|rvz|ciso|img|gdi")


def stem(name):
    """Filename with its extension removed and whitespace collapsed."""
    return re.sub(r"\s+", " ", re.sub(rf"\.({ROM_EXTS})$", "", name, flags=re.I)).strip().lower()


def title(name):
    """A game's name reduced to what two dumps of it would have in common.

    Region tags, revisions, language lists and punctuation all vary between
    dumps of the same game; the words do not. `(USA) (Rev A)` and
    `(USA) (En,Fr,De)` collapse to the same key, which is what stops the same
    game being copied a second time under a different label.
    """
    n = re.sub(rf"\.({ROM_EXTS})$", "", name, flags=re.I)
    n = re.sub(r"[\(\[].*?[\)\]]", " ", n)          # (USA), [!], (Rev 1)
    n = re.sub(r",\s*(the|a|an)\b", " ", n, flags=re.I)  # "Zelda, The" -> "Zelda"
    n = re.sub(r"^\s*(the|a|an)\b", " ", n, flags=re.I)
    return re.sub(r"[^a-z0-9]+", "", n.lower())


def card_keys(folder):
    """Every stem and title already present in one card folder."""
    stems, titles = set(), set()
    for e in os.scandir(folder):
        if e.name.startswith("."):
            continue
        stems.add(stem(e.name))
        titles.add(title(e.name))
    return stems, titles


def plan(db, card_root):
    """What the server has that the card does not, per system."""
    out = []
    for cd, slug in SYSTEMS.items():
        folder = card_root / cd
        if not folder.is_dir():
            print(f"skip {cd}: no folder on the card", file=sys.stderr)
            continue
        stems, titles = card_keys(folder)
        rows = db.execute(
            "SELECT id, fs_name, name, fs_size_bytes, multi_file FROM roms "
            "WHERE platform_slug = ? ORDER BY fs_name", (slug,))
        for rid, fs_name, name, size, multi in rows:
            if fs_name in EXCLUDE:
                continue
            if stem(fs_name) in stems:
                continue
            if title(name or fs_name) in titles or title(fs_name) in titles:
                continue
            out.append({
                "id": rid, "fs_name": fs_name, "name": name or fs_name,
                "size": size or 0, "multi": bool(multi),
                "slug": slug, "card_dir": cd,
            })
    return out


def fetch(url, auth, dest, expected, multi):
    """Download to `dest`, resuming a part file if one is there.

    A folder ROM arrives as a zip of its contents, so its transferred size never
    equals the sum of the files it holds. Only flat files are size-checked.
    """
    part = dest.with_suffix(dest.suffix + ".part")
    have = part.stat().st_size if part.exists() else 0
    req = urllib.request.Request(url, headers={"Authorization": auth})
    if have:
        req.add_header("Range", f"bytes={have}-")
    with urllib.request.urlopen(req, timeout=300) as r:
        # 200 to a Range request means the server ignored it and is sending the
        # whole file; anything already in the part file is then stale.
        appending = r.status == 206 and have > 0
        mode = "ab" if appending else "wb"
        written = have if appending else 0
        with open(part, mode) as f:
            while chunk := r.read(4 << 20):
                f.write(chunk)
                written += len(chunk)
    if expected and not multi and written != expected:
        raise IOError(f"size mismatch: expected {expected}, got {written}")
    part.replace(dest)
    return written


def human(b):
    return f"{b/1024**3:.2f} GB" if b >= 1024**3 else f"{b/1024**2:.0f} MB"


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--card", default="/Volumes/1TB/Roms")
    ap.add_argument("--library", default="library/roms")
    ap.add_argument("--cache", default="cache.sqlite3")
    ap.add_argument("--config", default="config.toml")
    ap.add_argument("--system", help="only this card folder")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    card_root = pathlib.Path(args.card)
    if not card_root.is_dir():
        sys.exit(f"card not mounted at {card_root}")
    lib_root = pathlib.Path(args.library)

    cfg = tomllib.loads(pathlib.Path(args.config).read_text())["server"]
    auth = f"Bearer {cfg['token'].strip()}"
    base = cfg["url"].rstrip("/")

    db = sqlite3.connect(args.cache)
    items = plan(db, card_root)
    if args.system:
        items = [i for i in items if i["card_dir"] == args.system]

    by_dir = {}
    for i in items:
        by_dir.setdefault(i["card_dir"], []).append(i)
    total = sum(i["size"] for i in items)

    for cd in sorted(by_dir):
        rows = by_dir[cd]
        print(f"\n{cd}  <- {rows[0]['slug']}  "
              f"{len(rows)} games, {human(sum(r['size'] for r in rows))}")
        for r in rows:
            print(f"    {human(r['size']):>9}  {r['fs_name']}")
    print(f"\n{len(items)} games, {human(total)}")

    if not args.apply:
        print("\ndry run — nothing downloaded or copied. Re-run with --apply.")
        return

    touched, done, failed = set(), 0, []
    for n, r in enumerate(items, 1):
        local = lib_root / r["slug"] / r["fs_name"]
        target = card_root / r["card_dir"] / r["fs_name"]
        print(f"[{n}/{len(items)}] {r['fs_name']}")
        try:
            if not local.exists():
                local.parent.mkdir(parents=True, exist_ok=True)
                url = f"{base}/api/roms/{r['id']}/content/{urllib.request.quote(r['fs_name'])}"
                fetch(url, auth, local, r["size"], r["multi"])
            if target.exists() and target.stat().st_size == local.stat().st_size:
                print("      already on the card")
            else:
                shutil.copy2(local, target)
                # The card is removable and the copy is not hash-checked, so a
                # short write has to be caught here rather than on the handheld.
                if target.stat().st_size != local.stat().st_size:
                    target.unlink(missing_ok=True)
                    raise IOError("short write to the card")
            touched.add(target.parent)
            done += 1
        except Exception as e:
            print(f"      FAILED: {e}")
            failed.append((r["fs_name"], str(e)))

    # Finder writes ._ sidecars onto exFAT, and MinUI-family firmware lists them
    # as games. Clean only what this run touched.
    for d in sorted(touched):
        subprocess.run(["dot_clean", "-m", str(d)], check=False)

    print(f"\ncopied {done}/{len(items)}")
    for name, err in failed:
        print(f"  failed: {name} — {err}")


if __name__ == "__main__":
    main()
