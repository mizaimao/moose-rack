"""Short names on moose98.img that differ from eXo's, for the copied files."""
def load(p):
    out = {}
    for block in open(p, errors="replace").read().split("\n\n"):
        f = dict(l.split(" = ", 1) for l in block.splitlines() if " = " in l)
        if "Path" in f:
            out[f["Path"].upper()] = (f["Path"], f.get("Short Name", ""))
    return out
exo, ours = load("exo-slt.txt"), load("moose-slt.txt")
paths = set()
for l in open("stage-files.txt").read().splitlines():
    parts = l.split("/")
    for i in range(1, len(parts) + 1):
        paths.add("/".join(parts[:i]).upper())
bad = 0
for p in sorted(paths):
    a, b = exo.get(p, ("", ""))[1], ours.get(p, ("", "?missing"))[1]
    if a != b:
        bad += 1
        print(f"{p}: eXo {a!r}, ours {b!r}")
junk = [v[0] for k, v in ours.items() if v[0].split("/")[-1].startswith((".", "._"))]
print(bad, "mismatches;", "junk:", junk)
