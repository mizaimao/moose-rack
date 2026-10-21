"""JSDOS.REG: what js-dos's hardware added to the registry.

usage: python3 jsdosreg.py BEFORE/SYSTEM.DAT AFTER/SYSTEM.DAT > JSDOS.REG

BEFORE is the disk as built. AFTER is the same disk once jsdos-capture.mjs has
booted it in js-dos and answered Yes to the restart: patchvhd.py puts the
captured blocks onto a copy, and 7zz extracts WINDOWS/SYSTEM.DAT from that.

Written out: every key under Enum and Services\\Class that the browser's boot
created, and every Enum key whose values it changed (COM1 became COM2 in
8.4.1's case), each with all of its values. JSDOS.BAT imports the file in one
more native boot. Windows' own counters under Control\\PerfStats change on every
boot and are left out.
"""
import sys

import w9xreg

ENUM = "HKEY_LOCAL_MACHINE\\ENUM\\"
CLASS = "\\SERVICES\\CLASS\\"


def changed(before, after):
    """Values as (type, data) by name, so a reordering is not a change."""
    return {n: v[1:] for n, v in before.items()} != {n: v[1:] for n, v in after.items()}


def main(before_path, after_path):
    before = w9xreg.read_creg(before_path, "HKEY_LOCAL_MACHINE")
    after = w9xreg.read_creg(after_path, "HKEY_LOCAL_MACHINE")
    keys = []
    for k in sorted(after):
        if "\\PERFSTATS\\" in k or not (k.startswith(ENUM) or CLASS in k):
            continue
        if k not in before or (k.startswith(ENUM) and changed(before[k][1], after[k][1])):
            keys.append(k)
    out = ["REGEDIT4", ""]
    for k in keys:
        path, values = after[k]
        out.append(f"[{path}]")
        for name, vtype, data in values.values():
            shown = "@" if name == "" else '"' + name.replace("\\", "\\\\").replace('"', '\\"') + '"'
            out.append(f"{shown}={w9xreg.as_text(vtype, data)}")
        out.append("")
    sys.stdout.buffer.write(("\r\n".join(out) + "\r\n").encode("latin-1"))
    print(f"{len(keys)} keys", file=sys.stderr)


if __name__ == "__main__":
    main(*sys.argv[1:])
