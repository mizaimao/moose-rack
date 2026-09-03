#!/usr/bin/env python3
"""Extract the emulator/core mapping from an ES-DE Android config export.

One-shot archival tool. Reads the three XML files pulled off the Android
handheld and writes a machine-readable core map that the client can consume,
so the XML can be deleted afterwards.

    python3 tools/extract_esde_cores.py            # writes data/esde-core-map.json

ES-DE lists emulators per system in preference order -- the first <command> is
the default. That ordering is the valuable part: it encodes which core ES-DE
reaches for first, which is what we want to match on the desktop.

Core filenames translate cleanly between platforms; only the suffix differs:

    Android : <stem>_libretro_android.so
    macOS   : <stem>_libretro.dylib
    Linux   : <stem>_libretro.so
    Windows : <stem>_libretro.dll
"""

from __future__ import annotations

import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "data" / "esde-core-map.json"

CORE_RE = re.compile(r"/cores/([A-Za-z0-9_\-]+)_libretro_android\.so")
EMU_RE = re.compile(r"%EMULATOR_([A-Za-z0-9_\-]+)%")

# ES-DE base system name -> platform slug, for the 24 platforms in
# library/. ES-DE and the library's slugs disagree on several names (dreamcast/dc,
# neogeo/neogeoaes, ngp/neo-geo-pocket, gc/ngc, megadrive is shared).
BUNDLED_TO_SERVER: dict[str, str] = {
    "3do": "3do",
    "arcade": "arcade",
    "dreamcast": "dc",
    "famicom": "famicom",
    "gamegear": "gamegear",
    "gb": "gb",
    "gba": "gba",
    "gbc": "gbc",
    "gc": "ngc",
    "megadrive": "megadrive",
    "mame": "mame",
    "mastersystem": "mastersystem",
    "n64": "n64",
    "nds": "nds",
    "neogeo": "neogeoaes",
    "nes": "nes",
    "ngp": "neo-geo-pocket",
    "pcengine": "pcengine",
    "psp": "psp",
    "psx": "psx",
    "sfc": "sfc",
    "snes": "snes",
    "wonderswan": "wonderswan",
    "wonderswancolor": "wonderswancolor",
}

# ES-DE system name -> platform slug(s) present in this library.
# The "hack" systems (nesh, snesh, ...) are ROM-hack variants that use exactly
# the same cores as their base system, so they map onto the base platforms.
ESDE_TO_SERVER: dict[str, list[str]] = {
    "gbh": ["gb"],
    "gbch": ["gbc"],
    "gbah": ["gba"],
    "nesh": ["nes", "famicom"],
    "snesh": ["snes", "sfc"],
    "snes-msu1": ["snes", "sfc"],
    "genh": ["megadrive"],
    "msu-md": ["megadrive"],
    "gc": ["ngc"],
    # Present in the export but not in this library:
    "wii": [],
    "n3ds": [],
    "ps2": [],
    "ps3": [],
    "psvita": [],
    "switch": [],
}

# Every platform slug in library/, so we can report what is still missing.
SERVER_PLATFORMS = [
    "3do", "arcade", "dc", "famicom", "gamegear", "gb", "gba", "gbc", "ngc",
    "megadrive", "mame", "mastersystem", "n64", "nds", "neogeoaes", "nes",
    "neo-geo-pocket", "pcengine", "psp", "psx", "sfc", "snes", "wonderswan",
    "wonderswancolor",
]


# A few cores are named differently in the Android builds than on desktop.
# Verified against the libretro buildbot listings for osx/arm64 and osx/x86_64
# on 2026-07-30: the Android names below have no desktop build, the mapped
# names do.
ANDROID_CORE_ALIASES: dict[str, str] = {
    "mamearcade": "mame",                      # Android names current MAME "mamearcade"
    "mupen64plus_next_gles3": "mupen64plus_next",  # GLES3 variant is Android-only
}


def core_filenames(stem: str) -> dict[str, str]:
    """Per-platform core filenames, translating Android-only core names."""
    desktop = ANDROID_CORE_ALIASES.get(stem, stem)
    return {
        "macos": f"{desktop}_libretro.dylib",
        "linux": f"{desktop}_libretro.so",
        "windows": f"{desktop}_libretro.dll",
        "android": f"{stem}_libretro_android.so",
    }


def parse_systems(path: Path, only: set[str] | None = None) -> dict:
    """Parse an es_systems.xml. ``only`` restricts to those system names."""
    systems = {}
    for s in ET.parse(path).getroot().findall("system"):
        name = s.findtext("name", "").strip()
        if only is not None and name not in only:
            continue
        emulators = []
        for idx, c in enumerate(s.findall("command")):
            text = " ".join((c.text or "").split())
            label = c.get("label", "(default)")
            core = CORE_RE.search(text)
            if core:
                stem = core.group(1)
                emulators.append({
                    "label": label,
                    "kind": "libretro",
                    "core": ANDROID_CORE_ALIASES.get(stem, stem),
                    "core_android": stem,
                    "files": core_filenames(stem),
                    "default": idx == 0,
                })
            else:
                emu = EMU_RE.search(text)
                emulators.append({
                    "label": label,
                    "kind": "standalone",
                    "app": emu.group(1) if emu else None,
                    "default": idx == 0,
                    "command": text if not emu else None,
                })
        exts = sorted({
            e.lower()
            for e in (s.findtext("extension", "") or "").split()
            if e.startswith(".")
        })
        slugs = ESDE_TO_SERVER.get(name)
        if slugs is None:
            slug = BUNDLED_TO_SERVER.get(name)
            slugs = [slug] if slug else []
        systems[name] = {
            "fullname": s.findtext("fullname", "").strip(),
            "extensions": exts,
            "server_platforms": slugs,
            "emulators": emulators,
        }
    return systems


def parse_find_rules(path: Path) -> dict:
    """Android package/activity per standalone emulator. Android-only value."""
    rules = {}
    for e in ET.parse(path).getroot().findall("emulator"):
        entries = [
            (n.text or "").strip()
            for r in e.findall("rule")
            for n in r.findall("entry")
        ]
        rules[e.get("name", "?")] = entries
    return rules


def parse_settings(path: Path) -> dict:
    """Only the handful of settings that affect emulator selection.

    es_settings.xml is not well-formed: it is a flat run of <bool/> and
    <string/> elements with no root, so it needs a synthetic wrapper.
    """
    body = path.read_text(encoding="utf-8", errors="replace")
    body = re.sub(r"<\?xml[^>]*\?>", "", body, count=1)
    root = ET.fromstring(f"<esSettings>{body}</esSettings>")
    keep = {}
    for el in root:
        n = el.get("name")
        if n and "mulator" in n:
            keep[n] = el.get("value")
    return keep


def main() -> int:
    systems_xml = REPO / "es_systems.xml"
    rules_xml = REPO / "es_find_rules.xml"
    settings_xml = REPO / "es_settings.xml"
    for p in (systems_xml, rules_xml, settings_xml):
        if not p.is_file():
            print(f"missing: {p}", file=sys.stderr)
            return 1

    # Two sources. The bundled upstream list covers the 24 base platforms in
    # library/; the device export adds ROM-hack variants and records which
    # alternative emulators this particular handheld offers.
    bundled_xml = REPO / "data" / "vendor" / "esde_android_es_systems.xml"
    bundled = (
        parse_systems(bundled_xml, only=set(BUNDLED_TO_SERVER))
        if bundled_xml.is_file()
        else {}
    )
    custom = parse_systems(systems_xml)
    systems = {**bundled, **custom}

    # Which platforms do we have a default libretro core for?
    covered: dict[str, str] = {}
    for name, sysdef in systems.items():
        default = next(
            (e for e in sysdef["emulators"] if e["default"] and e["kind"] == "libretro"),
            None,
        )
        if not default:
            continue
        for slug in sysdef["server_platforms"]:
            covered.setdefault(slug, default["core"])

    missing = sorted(set(SERVER_PLATFORMS) - set(covered))

    doc = {
        "_comment": (
            "Extracted from an ES-DE Android config export (custom_systems). "
            "Source XML was deleted after extraction; regenerate only from a "
            "fresh device pull. Emulators are listed in ES-DE preference order; "
            "the first entry is the default."
        ),
        "source": {
            "device_export": {
                "device": "Android handheld running ES-DE",
                "files": [f.name for f in (systems_xml, rules_xml, settings_xml)],
                "note": (
                    "es_systems.xml from the device is the custom_systems "
                    "override file (ROM-hack and modern-console systems), NOT "
                    "ES-DE's full system list. es_find_rules.xml is Android "
                    "package names only and has no value off Android. "
                    "es_settings.xml sets AlternativeEmulatorPerGame=true, so "
                    "per-game emulator overrides live in the gamelist.xml "
                    "files, which were not exported."
                ),
            },
            "bundled": {
                "file": "data/vendor/esde_android_es_systems.xml",
                "origin": (
                    "https://gitlab.com/es-de/emulationstation-de/-/raw/master/"
                    "resources/systems/android/es_systems.xml"
                ),
                "note": "Upstream ES-DE defaults; identical for every user.",
            },
        },
        "core_filename_pattern": {
            "macos": "<stem>_libretro.dylib",
            "linux": "<stem>_libretro.so",
            "windows": "<stem>_libretro.dll",
            "android": "<stem>_libretro_android.so",
            "buildbot": "https://buildbot.libretro.com/nightly/apple/osx/arm64/latest/<stem>_libretro.dylib.zip",
        },
        "default_core_by_server_platform": dict(sorted(covered.items())),
        "server_platforms_without_mapping": missing,
        "systems": systems,
        "android_packages": parse_find_rules(rules_xml),
        "relevant_settings": parse_settings(settings_xml),
    }

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(doc, indent=2) + "\n")

    print(f"wrote {OUT.relative_to(REPO)}")
    print(f"\n{len(systems)} ES-DE systems parsed")
    print(f"\ndefault core for {len(covered)} platforms:")
    for slug, core in sorted(covered.items()):
        print(f"  {slug:<16} {core}")
    print(f"\n{len(missing)} platforms still unmapped:")
    print("  " + ", ".join(missing))
    return 0


if __name__ == "__main__":
    sys.exit(main())
