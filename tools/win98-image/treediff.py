"""Files in eXo's C: that are missing from ours or differ in content."""
import hashlib, os, sys

def walk(root):
    out = {}
    for d, _, names in os.walk(root):
        for n in names:
            p = os.path.join(d, n)
            rel = os.path.relpath(p, root)
            out[rel.upper()] = rel
    return out

def digest(p):
    return hashlib.md5(open(p, "rb").read()).hexdigest()

exo, ours = walk("tree/exo"), walk("tree/ours")
for key in sorted(exo):
    rel = exo[key]
    a = os.path.join("tree/exo", rel)
    if key not in ours:
        print(f"NEW  {os.path.getsize(a):>9}  {rel}")
    else:
        b = os.path.join("tree/ours", ours[key])
        if os.path.getsize(a) != os.path.getsize(b) or digest(a) != digest(b):
            print(f"DIFF {os.path.getsize(a):>9}  {rel}")
