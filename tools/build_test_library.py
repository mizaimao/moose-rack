#!/usr/bin/env python3
"""Build a small, slug-shaped test library by sampling an ES-DE collection.

Samples N "game units" per platform from an ES-DE roms tree and clones them --
plus their ES-DE downloaded_media -- into a local library laid out flat by
platform slug (``roms/<platform_slug>/``, one directory per platform).

On APFS (macOS) files are copied with ``cp -c``, so clones are copy-on-write:
near-instant and costing almost no extra disk. Falls back to a real copy when
cloning is unsupported.

A "game unit" is one playable entry:

* a plain ROM file (``Foo (USA).nes``), or
* an ``.m3u`` playlist *plus every disc it references* (``MultiDisk/Foo (USA)
  (Disc 1).chd`` ...). Discs referenced by a playlist are never sampled on
  their own, so a multi-disc game is always complete or absent.

Only the top level of each platform directory is sampled. ES-DE side-car
directories (``gba/Aftermarket``, ``nes/Multicarts``, ``sfc/AdditionalRoms``,
``arcade/fbneo`` ...) are deliberately skipped -- they are curated extras, not
part of the main list -- except where an ``.m3u`` reaches into one.

Usage:
    python3 tools/build_test_library.py                 # build with defaults
    python3 tools/build_test_library.py --dry-run       # plan only, write nothing
    python3 tools/build_test_library.py --per-platform 5 --seed 99
    python3 tools/build_test_library.py --no-media
"""

from __future__ import annotations

import argparse
import json
import os
import random
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

DEFAULT_SOURCE = Path("/Users/frank/Downloads/roms_pre_compress")
DEFAULT_DEST = Path(__file__).resolve().parent.parent / "library"
DEFAULT_PER_PLATFORM = 10
DEFAULT_SEED = 20260729

# Files that live in the platform directories but are not ROMs.
NON_ROM_SUFFIXES = {
    ".txt", ".db", ".xml", ".bat", ".old", ".state", ".ini", ".cfg", ".dat",
}
NON_ROM_NAMES = {".DS_Store", "systeminfo.txt", "gamelist.xml"}


@dataclass(frozen=True)
class Platform:
    """One platform in the ES-DE source, mapped onto its library slug.

    Attributes:
        esde_dir: Directory name in the ES-DE roms tree.
        server_slug: The library's platform slug. This is what the destination
            directory is named, so a scan of the output creates the right
            platform instead of e.g. ``gamegear_hidden``.
        media_dirs: Candidate ``downloaded_media/`` directories, searched in
            order. ES-DE and the slugs disagree on several names, and a few systems
            have media filed under more than one (``snes`` + ``snesna``).
    """

    esde_dir: str
    server_slug: str
    media_dirs: tuple[str, ...] = ()


# The intersection of platforms present in the ES-DE source and platforms the
# server recognises (its FS_PLATFORMS, read from /api/heartbeat).
#
# Excluded on purpose:
#   0_BIOS, downloaded_media  - not platforms
#   N64_All                   - no platform slug; nested Europe/USA, not flat
#   easyrpg, ports, saturn    - present but empty (systeminfo.txt only)
PLATFORMS: tuple[Platform, ...] = (
    Platform("3do", "3do"),  # no downloaded_media/3do exists
    Platform("arcade", "arcade", ("mame",)),
    Platform("dreamcast", "dc", ("dreamcast",)),
    Platform("famicom", "famicom", ("famicom",)),
    Platform("gamegear_hidden", "gamegear", ("gamegear",)),
    Platform("gb", "gb", ("gb",)),
    Platform("gba", "gba", ("gba",)),
    Platform("gbc", "gbc", ("gbc",)),
    Platform("gc", "ngc", ("gc",)),
    Platform("genesis", "megadrive", ("genesis", "megadrive")),
    Platform("mame", "mame", ("mame",)),
    Platform("mastersystem_hidden", "mastersystem", ("mastersystem",)),
    Platform("n64", "n64", ("n64",)),
    Platform("nds", "nds", ("nds",)),
    Platform("neogeo", "neogeoaes", ("neogeo",)),
    Platform("nes", "nes", ("nes",)),
    Platform("ngp_hidden", "neo-geo-pocket", ("ngp",)),
    Platform("pcengine_hidden", "pcengine", ("pcengine",)),
    Platform("psp", "psp", ("psp",)),
    Platform("psx", "psx", ("psx",)),
    Platform("sfc", "sfc", ("sfc",)),
    Platform("snes", "snes", ("snes", "snesna")),
    Platform("wonderswan_hidden", "wonderswan", ("wonderswan",)),
    Platform("wonderswancolor_hidden", "wonderswancolor", ("wonderswancolor",)),
)


@dataclass
class Unit:
    """A playable entry and every file it needs.

    Attributes:
        name: Basename without extension -- also the key ES-DE files media under.
        files: Paths relative to the platform directory, primary entry first.
    """

    name: str
    files: list[Path] = field(default_factory=list)

    @property
    def size(self) -> int:
        return sum(self._sizes)

    _sizes: list[int] = field(default_factory=list, repr=False)


def is_rom_file(path: Path) -> bool:
    if path.name in NON_ROM_NAMES or path.name.startswith("._"):
        return False
    if path.suffix.lower() in NON_ROM_SUFFIXES:
        return False
    return path.is_file()


def collect_units(platform_dir: Path) -> tuple[list[Unit], list[dict]]:
    """Group the top level of ``platform_dir`` into game units.

    Returns the sampleable units and a list of playlists that were rejected.
    A playlist missing any disc is excluded rather than staged partially: a
    half-populated multi-disc set is a worse test fixture than none at all,
    because it fails deep in an emulator instead of up front here.
    """
    playlists = sorted(platform_dir.glob("*.m3u"))
    units: list[Unit] = []
    broken: list[dict] = []
    # Every disc claimed by a playlist, so it is not also sampled standalone.
    claimed: set[Path] = set()

    for playlist in playlists:
        discs: list[Path] = []
        missing: list[str] = []
        text = playlist.read_text(encoding="utf-8", errors="replace")
        for line in text.splitlines():
            entry = line.strip()
            if not entry or entry.startswith("#"):
                continue
            # Paths inside .m3u are relative to the platform dir and may point
            # into a subdirectory -- MultiDisk/, or 3do's misspelled MuliDisk/.
            disc = platform_dir / entry
            if disc.is_file():
                discs.append(Path(entry))
                claimed.add(disc.resolve())
            else:
                missing.append(entry)

        if missing or not discs:
            broken.append(
                {
                    "playlist": playlist.name,
                    "missing_discs": missing,
                    "resolved_discs": len(discs),
                }
            )
            for m in missing:
                print(f"  ! excluded {playlist.name}: missing {m!r}", file=sys.stderr)
            # Discs that *did* resolve stay claimed, so they are not offered as
            # standalone units -- they belong to a game we know is incomplete.
            continue

        unit = Unit(name=playlist.stem, files=[Path(playlist.name), *discs])
        unit._sizes = [(platform_dir / f).stat().st_size for f in unit.files]
        units.append(unit)

    for entry in sorted(platform_dir.iterdir()):
        if not is_rom_file(entry) or entry.suffix.lower() == ".m3u":
            continue
        if entry.resolve() in claimed:
            continue
        unit = Unit(name=entry.stem, files=[Path(entry.name)])
        unit._sizes = [entry.stat().st_size]
        units.append(unit)

    return units, broken


def find_media(media_root: Path, media_dirs: tuple[str, ...], name: str) -> list[Path]:
    """Return media files for ``name``, as paths relative to ``media_root``.

    ES-DE files media as ``<system>/<media_type>/<rom basename>.<ext>``. The
    first system directory that yields anything wins, so ``genesis`` beats its
    ``megadrive`` fallback rather than merging the two.
    """
    for media_dir in media_dirs:
        system_root = media_root / media_dir
        if not system_root.is_dir():
            continue
        found: list[Path] = []
        for type_dir in sorted(p for p in system_root.iterdir() if p.is_dir()):
            for candidate in sorted(type_dir.glob(f"{glob_escape(name)}.*")):
                if candidate.is_file() and not candidate.name.startswith("._"):
                    found.append(candidate.relative_to(media_root))
        if found:
            return found
    return []


def glob_escape(name: str) -> str:
    """Escape glob metacharacters so bracketed ROM names match literally."""
    out = []
    for ch in name:
        out.append(f"[{ch}]" if ch in "*?[]" else ch)
    return "".join(out)


def clone(src: Path, dst: Path) -> None:
    """Copy-on-write clone ``src`` to ``dst``, falling back to a real copy."""
    dst.parent.mkdir(parents=True, exist_ok=True)
    if dst.exists():
        return
    if sys.platform == "darwin":
        result = subprocess.run(
            ["cp", "-c", str(src), str(dst)],
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            return
    shutil.copy2(src, dst)


def human(num_bytes: int) -> str:
    value = float(num_bytes)
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if abs(value) < 1024.0 or unit == "TB":
            return f"{value:,.1f} {unit}"
        value /= 1024.0
    return f"{value:,.1f} TB"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument("--dest", type=Path, default=DEFAULT_DEST)
    parser.add_argument("--per-platform", type=int, default=DEFAULT_PER_PLATFORM)
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    parser.add_argument("--no-media", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    source: Path = args.source.expanduser()
    dest: Path = args.dest.expanduser()
    if not source.is_dir():
        print(f"source not found: {source}", file=sys.stderr)
        return 1

    media_root = source / "downloaded_media"
    roms_out = dest / "roms"
    media_out = dest / "downloaded_media"

    manifest: dict = {
        "source": str(source),
        "seed": args.seed,
        "per_platform": args.per_platform,
        "layout": "roms/<server_slug>/ flat by platform; media keyed by server_slug",
        "platforms": [],
    }
    total_rom_bytes = total_media_bytes = 0
    total_units = total_rom_files = total_media_files = 0
    broken_playlists: dict[str, list[dict]] = {}

    for platform in PLATFORMS:
        platform_dir = source / platform.esde_dir
        if not platform_dir.is_dir():
            print(f"[skip] {platform.esde_dir}: not found", file=sys.stderr)
            continue

        units, broken = collect_units(platform_dir)
        if broken:
            broken_playlists[platform.server_slug] = broken
        if not units:
            print(f"[skip] {platform.esde_dir}: no ROMs", file=sys.stderr)
            continue

        # Seed per platform so adding or reordering PLATFORMS does not reshuffle
        # every other platform's picks.
        rng = random.Random(f"{args.seed}:{platform.esde_dir}")
        picked = rng.sample(units, min(args.per_platform, len(units)))
        picked.sort(key=lambda u: u.name)

        rom_bytes = media_bytes = 0
        media_count = 0
        entries = []
        for unit in picked:
            for rel in unit.files:
                src_file = platform_dir / rel
                if not args.dry_run:
                    clone(src_file, roms_out / platform.server_slug / rel)
            rom_bytes += unit.size

            media_files: list[Path] = []
            if not args.no_media and platform.media_dirs:
                media_files = find_media(media_root, platform.media_dirs, unit.name)
                for rel in media_files:
                    src_file = media_root / rel
                    # Re-key media under the platform slug, dropping the ES-DE
                    # system name so roms and media share one vocabulary.
                    out_rel = Path(platform.server_slug) / Path(*rel.parts[1:])
                    if not args.dry_run:
                        clone(src_file, media_out / out_rel)
                    media_bytes += src_file.stat().st_size
                media_count += len(media_files)

            entries.append(
                {
                    "name": unit.name,
                    "files": [str(f) for f in unit.files],
                    "multi_disc": len(unit.files) > 1,
                    "size_bytes": unit.size,
                    "media_files": len(media_files),
                }
            )

        print(
            f"{platform.server_slug:<16} {len(picked):>3}/{len(units):<5} units  "
            f"roms {human(rom_bytes):>11}  media {human(media_bytes):>11} "
            f"({media_count} files)"
        )

        manifest["platforms"].append(
            {
                "esde_dir": platform.esde_dir,
                "server_slug": platform.server_slug,
                "media_dirs": list(platform.media_dirs),
                "units_available": len(units),
                "units_picked": len(picked),
                "rom_bytes": rom_bytes,
                "media_bytes": media_bytes,
                "media_files": media_count,
                "games": entries,
            }
        )
        total_units += len(picked)
        total_rom_files += sum(len(u.files) for u in picked)
        total_media_files += media_count
        total_rom_bytes += rom_bytes
        total_media_bytes += media_bytes

    # Defects in the *source* collection, recorded so they are not rediscovered
    # every time someone wonders why a game is absent from the test library.
    manifest["excluded_broken_playlists"] = broken_playlists
    manifest["totals"] = {
        "platforms": len(manifest["platforms"]),
        "units": total_units,
        "rom_files": total_rom_files,
        "media_files": total_media_files,
        "rom_bytes": total_rom_bytes,
        "media_bytes": total_media_bytes,
    }

    print()
    print(
        f"{len(manifest['platforms'])} platforms  {total_units} games  "
        f"{total_rom_files} rom files  {total_media_files} media files"
    )
    print(
        f"roms {human(total_rom_bytes)}  media {human(total_media_bytes)}  "
        f"total {human(total_rom_bytes + total_media_bytes)}"
    )

    if args.dry_run:
        print("\n(dry run -- nothing written)")
        return 0

    dest.mkdir(parents=True, exist_ok=True)
    (dest / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"\nwrote {dest / 'manifest.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
