#!/usr/bin/env python3
"""Inventory an external ES-DE/RetroBat game drive into one manifest.

Written for the 12 TB "Super Game HDD", which is laid out the way those drives
always are: `roms/<platform>/roms/**` for the games, and a sibling
`gamelist.xml` per platform carrying the part that matters for choosing what to
play — the real title, the year, the genre, the rating and the player count.
Filenames alone are not enough to match a game against a published "best of"
list: `Adventures of Elmo in Grouchland, The (USA).zip` and "The Adventures of
Elmo in Grouchland" are the same game and share no prefix.

So this reads both and joins them:

* every file under a platform's `roms/`, recursively — some platforms nest by
  region (`roms/USA`, `roms/Japan`), and a flat listing undercounts those by an
  order of magnitude. Mega Drive looks like two games and holds nine hundred.
* every `<game>` in `gamelist.xml`, matched to those files by path.

Three things come out, and the third is the one worth reading: entries the
gamelist claims that no file backs, and files no gamelist describes. Both are
normal on a drive assembled by hand, and both mean a title that will not match
a ranking later.

The drive is NTFS and mounted read-only. Nothing here writes to it.

    python3 tools/drive_manifest.py                     # default drive, default output
    python3 tools/drive_manifest.py --drive /Volumes/X --out drive-manifest
"""

import argparse
import json
import os
import pathlib
import re
import sys
import xml.etree.ElementTree as ET

# Asked for by name. These are the big modern systems this library is not for;
# skipping them keeps the manifest to what a retro frontend can actually launch.
EXCLUDE_PLATFORMS = {"ps2", "ps3", "switch", "xbox360"}

# Not consoles. `0EmtpyFolders` is the drive author's spelling, kept because it
# is what is on disk.
NOT_PLATFORMS = {"0EmtpyFolders", "bezels_project"}

# Things that live under `roms/` without being games.
SKIP_SUFFIXES = {".txt", ".xml", ".dat", ".ini", ".cfg", ".db", ".jpg", ".png", ".mp4"}
SKIP_NAMES = {".DS_Store", "Thumbs.db", "desktop.ini"}

REGION_RE = re.compile(r"\((USA|Europe|Japan|World|Asia|Korea|Brazil|France|Germany|Spain|Italy|Australia|Netherlands|Sweden|China|Taiwan|UE|JP|US|EU)\)", re.I)


def region_of(name):
    """Region as the No-Intro/TOSEC filename convention states it, if it does."""
    m = REGION_RE.search(name)
    return m.group(1) if m else None


def year_of(text):
    """`19990101T000000` -> 1999. Absent and malformed both give None."""
    if not text:
        return None
    m = re.match(r"(\d{4})", text.strip())
    if not m:
        return None
    y = int(m.group(1))
    return y if 1950 <= y <= 2035 else None


def norm_path(p):
    """Gamelist paths are `./roms/x.zip`; walked paths are `roms/x.zip`.

    Matched case-insensitively because the source is NTFS, where the gamelist
    and the directory entry routinely disagree about capitalisation.
    """
    return p.replace("\\", "/").lstrip("./").casefold()


def read_gamelist(path):
    """`{normalised path: metadata}` for one platform, or `{}` if unreadable.

    A gamelist that fails to parse is reported and skipped rather than fatal:
    one bad file on a drive of eighty should not cost the other seventy-nine.
    """
    out = {}
    try:
        for _, el in ET.iterparse(path, events=("end",)):
            if el.tag != "game":
                continue
            rel = (el.findtext("path") or "").strip()
            if rel:
                name = (el.findtext("name") or "").strip()
                rating = el.findtext("rating")
                out[norm_path(rel)] = {
                    "name": name or None,
                    "rating": float(rating) if rating and rating.strip() else None,
                    "year": year_of(el.findtext("releasedate")),
                    "genre": (el.findtext("genre") or "").strip() or None,
                    "players": (el.findtext("players") or "").strip() or None,
                    "developer": (el.findtext("developer") or "").strip() or None,
                    "publisher": (el.findtext("publisher") or "").strip() or None,
                }
            el.clear()
    except ET.ParseError as e:
        print(f"  ! gamelist unreadable ({e})", file=sys.stderr)
        return {}
    return out


def walk_roms(roms_dir):
    """Every file under `roms/`, relative to the platform directory."""
    found = []
    for root, dirs, files in os.walk(roms_dir):
        dirs[:] = [d for d in dirs if not d.startswith(".")]
        for f in files:
            if f in SKIP_NAMES or pathlib.Path(f).suffix.lower() in SKIP_SUFFIXES:
                continue
            full = pathlib.Path(root) / f
            try:
                size = full.stat().st_size
            except OSError:
                size = None
            found.append((full.relative_to(roms_dir.parent).as_posix(), size))
    return found


def classify(files, listed):
    """Split walked files into games, their data, and their assets.

    Counting files is not counting games, and three layouts on this drive prove
    it separately. ScummVM stores each game as a *directory* of assets with one
    `.scummvm` launcher inside it — 65 games, 5,388 files. PC-FX stores discs as
    `.cue` plus `.bin`, so every game is two files and the `.bin` is not a game.
    And a handful of platforms carry loose extras nobody listed.

    The gamelist decides what a game is, because it is the only thing here that
    knows. Everything else is placed relative to that: a file sharing a stem
    with a listed game in the same directory is that game's data, anything under
    a directory containing a listed game is that game's assets, and what is left
    is a real candidate with no metadata — which is worth knowing, because it is
    exactly the set a published ranking will never match.
    """
    game_dirs = {p.rsplit("/", 1)[0] for p in listed if p.count("/") > 1}
    stems = {}
    for p in listed:
        d, _, f = p.rpartition("/")
        stems.setdefault(d, set()).add(pathlib.PurePosixPath(f).stem)

    out = {}
    for rel, size in files:
        key = norm_path(rel)
        if key in listed:
            out[rel] = ("game", size)
            continue
        d, _, f = key.rpartition("/")
        if pathlib.PurePosixPath(f).stem in stems.get(d, ()):
            out[rel] = ("data", size)
        elif any(d == g or d.startswith(g + "/") for g in game_dirs):
            out[rel] = ("asset", size)
        else:
            out[rel] = ("unlisted", size)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--drive", default="/Volumes/Super Game HDD")
    ap.add_argument("--out", default="drive-manifest")
    args = ap.parse_args()

    roms_root = pathlib.Path(args.drive) / "roms"
    if not roms_root.is_dir():
        sys.exit(f"no roms directory at {roms_root} — is the drive mounted?")

    out_dir = pathlib.Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    platforms = sorted(
        d.name for d in roms_root.iterdir()
        if d.is_dir() and d.name not in NOT_PLATFORMS and d.name not in EXCLUDE_PLATFORMS
    )

    records, summary = [], []
    for p in platforms:
        pdir = roms_root / p
        # Nearly every platform keeps its games in `roms/`, but not all: the
        # libretro ports sit directly in the platform folder beside the
        # gamelist, and skipping anything without a `roms/` subdirectory
        # silently lost them. Fall back to the platform folder itself.
        roms_dir = pdir / "roms"
        if not roms_dir.is_dir():
            if not (pdir / "gamelist.xml").is_file():
                print(f"{p:16} no roms/ and no gamelist — skipped")
                continue
            roms_dir = pdir

        print(f"{p:16} reading…", end="", flush=True)
        meta = read_gamelist(pdir / "gamelist.xml") if (pdir / "gamelist.xml").is_file() else {}
        files = walk_roms(roms_dir)

        kinds = classify(files, set(meta))
        sizes = dict(files)

        # A game carries the size of everything that belongs to it, so a
        # ScummVM title reads as its whole directory and a disc as cue plus bin
        # rather than as the few bytes of its index.
        extra = {}
        for rel, (kind, size) in kinds.items():
            if kind in ("data", "asset"):
                d = norm_path(rel).rpartition("/")[0]
                extra[d] = extra.get(d, 0) + (size or 0)

        titled = unlisted = 0
        for rel, (kind, size) in sorted(kinds.items()):
            if kind in ("data", "asset"):
                continue
            m = meta.get(norm_path(rel), {})
            titled += bool(m.get("name"))
            unlisted += kind == "unlisted"
            own = norm_path(rel).rpartition("/")[0]
            records.append({
                "platform": p,
                "file": rel,
                "size": (size or 0) + (extra.pop(own, 0) if kind == "game" else 0),
                "name": m.get("name") or pathlib.Path(rel).stem,
                "titled": bool(m.get("name")),
                "region": region_of(pathlib.Path(rel).name),
                "rating": m.get("rating"),
                "year": m.get("year"),
                "genre": m.get("genre"),
                "players": m.get("players"),
                "developer": m.get("developer"),
                "publisher": m.get("publisher"),
            })
        matched = titled

        # Described but not present: the gamelist was written against a fuller
        # drive than this one, and these are the titles a later match will look
        # for and not find.
        present = {norm_path(rel) for rel, _ in files}
        ghosts = sorted(v.get("name") or k for k, v in meta.items() if k not in present)

        games = sum(1 for k, _ in kinds.values() if k in ("game", "unlisted"))
        summary.append({
            "platform": p,
            "games": games,
            "files": len(files),
            "titled": titled,
            "untitled": unlisted,
            "ghosts": len(ghosts),
            "bytes": sum(s or 0 for _, s in files),
        })
        print(f"\r{p:16} {games:6} games ({len(files):6} files), {unlisted:5} untitled, "
              f"{len(ghosts):5} listed with no file")

        if ghosts:
            (out_dir / "ghosts").mkdir(exist_ok=True)
            (out_dir / "ghosts" / f"{p}.txt").write_text("\n".join(ghosts) + "\n")

    with (out_dir / "manifest.jsonl").open("w") as fh:
        for r in records:
            fh.write(json.dumps(r, ensure_ascii=False) + "\n")

    with (out_dir / "summary.json").open("w") as fh:
        json.dump(summary, fh, indent=1)

    lines = [
        "# Super Game HDD — manifest",
        "",
        f"{len(records):,} games across {len(summary)} platforms, "
        f"{sum(s['bytes'] for s in summary) / 1e12:.2f} TB.",
        "",
        f"Excluded by request: {', '.join(sorted(EXCLUDE_PLATFORMS))}.",
        "",
        "| platform | games | titled | untitled | listed, no file | size |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for s in sorted(summary, key=lambda s: -s["games"]):
        lines.append(
            f"| {s['platform']} | {s['games']:,} | {s['titled']:,} | "
            f"{s['untitled']:,} | {s['ghosts']:,} | {s['bytes'] / 1e9:.1f} GB |"
        )
    (out_dir / "README.md").write_text("\n".join(lines) + "\n")

    print(f"\n{len(records):,} files -> {out_dir}/manifest.jsonl")
    print(f"summary -> {out_dir}/README.md")


if __name__ == "__main__":
    main()
