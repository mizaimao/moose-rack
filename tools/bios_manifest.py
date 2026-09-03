#!/usr/bin/env python3
"""Work out every BIOS file the ES-DE system set can possibly need.

Two authoritative inputs, neither of them guesswork:

* `data/vendor/esde_android_es_systems.xml` — ES-DE's own system definitions,
  which name the libretro core behind every launch command.
* RetroArch's `info/*_libretro.info` — each core declares its firmware as
  `firmwareN_path` (the exact name it looks up inside the system directory),
  `firmwareN_desc`, and `firmwareN_opt` ("true" when the core runs without it).

The `_path` values matter more than they look: some are bare filenames and some
carry a subdirectory (`dc/dc_boot.bin`, `PPSSPP/ppge_atlas.zim`). A flat dump of
BIOS files will not work — the layout has to be reproduced exactly.

Emits JSON: every required file, which cores want it, and whether any of them
treat it as mandatory.
"""

import argparse
import json
import pathlib
import re
import xml.etree.ElementTree as ET


def esde_cores(xml_path):
    """system name -> set of libretro core stems."""
    root = ET.parse(xml_path).getroot()
    out = {}
    for sysel in root.findall("system"):
        name = sysel.findtext("name") or "?"
        cores = set()
        for cmd in sysel.findall("command"):
            for part in re.split(r"[\s/]+", cmd.text or ""):
                m = re.match(r"^(.+?)_libretro(?:_android)?\.so$", part)
                if m:
                    cores.add(m.group(1))
        if cores:
            out[name] = cores
    return out


def core_firmware(info_dir):
    """core stem -> [(path, desc, optional)]"""
    out = {}
    for f in pathlib.Path(info_dir).glob("*_libretro.info"):
        core = f.name[: -len("_libretro.info")]
        text = f.read_text(errors="replace")
        entries = {}
        for m in re.finditer(r'firmware(\d+)_(path|desc|opt)\s*=\s*"([^"]*)"', text):
            idx, key, val = m.group(1), m.group(2), m.group(3)
            entries.setdefault(idx, {})[key] = val
        items = []
        for e in entries.values():
            if not e.get("path"):
                continue
            items.append((e["path"], e.get("desc", ""), e.get("opt", "false") == "true"))
        if items:
            out[core] = items
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--systems", default="data/vendor/esde_android_es_systems.xml")
    ap.add_argument("--info", required=True, help="RetroArch info/ directory")
    ap.add_argument("--out", default="data/bios-manifest.json")
    args = ap.parse_args()

    systems = esde_cores(args.systems)
    firmware = core_firmware(args.info)

    files = {}
    for system, cores in sorted(systems.items()):
        for core in sorted(cores):
            for path, desc, opt in firmware.get(core, []):
                rec = files.setdefault(
                    path,
                    {"path": path, "desc": desc, "required": False,
                     "cores": set(), "systems": set()},
                )
                rec["cores"].add(core)
                rec["systems"].add(system)
                # Required if *any* core treats it as mandatory: a set that
                # satisfies the strictest core satisfies the rest.
                if not opt:
                    rec["required"] = True
                if desc and not rec["desc"]:
                    rec["desc"] = desc

    out = []
    for rec in files.values():
        rec["cores"] = sorted(rec["cores"])
        rec["systems"] = sorted(rec["systems"])
        out.append(rec)
    out.sort(key=lambda r: (not r["required"], r["path"].lower()))

    req = sum(1 for r in out if r["required"])
    covered = sorted({s for r in out for s in r["systems"]})
    print(f"{len(systems)} ES-DE systems, {len(firmware)} cores declare firmware")
    print(f"{len(out)} distinct BIOS files ({req} required, {len(out) - req} optional)")
    print(f"{len(covered)} systems need at least one file")

    dest = pathlib.Path(args.out)
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(json.dumps(out, indent=1) + "\n")
    print(f"wrote {dest}")


if __name__ == "__main__":
    main()
