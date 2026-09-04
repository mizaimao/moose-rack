#!/usr/bin/env python3
"""Pull every system's display name out of ES-DE's own system list.

    python3 tools/extract_esde_names.py     # writes data/esde-system-names.json

The core map in `data/esde-core-map.json` is an *extract*: 38 systems, the ones
whose emulator preferences were worth recording. Names are a different question
and a much simpler one -- ES-DE has a name for all 195 -- and keying them off
the core map meant a console that scanned fine still drew its folder name,
because it happened not to be one of the 38. `saturn` did exactly that.

Source is the bundled upstream file, not a device export: names are identical
for every user, so there is nothing to pull off a handheld.
"""

import json
import pathlib
import sys
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "data/vendor/esde_android_es_systems.xml"
OUT = ROOT / "data/esde-system-names.json"


def main():
    if not SRC.is_file():
        print(f"no {SRC}", file=sys.stderr)
        return 1
    names = {}
    for system in ET.parse(SRC).getroot().iter("system"):
        name, full = system.findtext("name"), system.findtext("fullname")
        if name and full:
            names[name.strip()] = full.strip()
    OUT.write_text(json.dumps(dict(sorted(names.items())), indent=1, ensure_ascii=False) + "\n")
    print(f"  {len(names)} names -> {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
