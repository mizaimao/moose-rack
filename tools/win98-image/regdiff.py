"""Keys and values eXo's Windows 98 has that ours does not, grouped."""
import collections
import sys
import w9xreg

exo = w9xreg.read_creg("regs/exo/SYSTEM.DAT", "HKEY_LOCAL_MACHINE")
exo.update(w9xreg.read_creg("regs/exo/USER.DAT", "HKEY_USERS"))
ours = w9xreg.read_reg("regs/ours/FULL.REG")

added = {}  # key -> list of (name, text) eXo has that differ or are missing here
for k, (path, vals) in exo.items():
    mine = ours.get(k, (path, {}))[1]
    diff = []
    for vk, (vname, vtype, vdata) in vals.items():
        text = w9xreg.as_text(vtype, vdata)
        if vk not in mine or mine[vk][1].lower() != text.lower():
            diff.append((vname, text))
    if k not in ours or diff:
        added[k] = (path, diff, k not in ours)

depth = int(sys.argv[1]) if len(sys.argv) > 1 else 4
groups = collections.Counter()
for k, (path, diff, new) in added.items():
    parts = path.split("\\")
    groups["\\".join(parts[:depth])] += 1
for g, n in sorted(groups.items(), key=lambda kv: -kv[1])[: int(sys.argv[2]) if len(sys.argv) > 2 else 80]:
    print(f"{n:6d}  {g}")
