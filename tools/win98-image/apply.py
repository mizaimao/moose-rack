"""Put the staged eXo add-ons and our scripts onto the mounted moose98.img.

Files go in the order of eXo's short names so that macOS hands out the same
~1/~2 short names eXo's disk has; the registry refers to some of them.
"""
import os, re, shutil

C = "mnt/NO NAME"

short = {}
for block in open("exo-slt.txt", errors="replace").read().split("\n\n"):
    f = dict(l.split(" = ", 1) for l in block.splitlines() if " = " in l)
    if "Path" in f:
        short[f["Path"].upper()] = f.get("Short Name", "")

def put(src, rel):
    dst = os.path.join(C, rel)
    shutil.copyfile(src, dst)
    st = os.stat(src)
    os.utime(dst, (st.st_mtime, st.st_mtime))

files = open("stage-files.txt").read().split()  # never split on spaces: reread
files = [l for l in open("stage-files.txt").read().splitlines() if l]

def key(rel):
    parts = rel.split("/")
    return [short.get("/".join(parts[: i + 1]).upper(), "") or parts[i].upper() for i in range(len(parts))]

for rel in sorted(files, key=key):
    parent = os.path.dirname(rel)
    for i in range(1, len(parent.split("/")) + 1 if parent else 0):
        d = os.path.join(C, *parent.split("/")[:i])
        if not os.path.isdir(d):
            os.mkdir(d)
    put(os.path.join("stage", rel), rel)

os.makedirs(os.path.join(C, "MOOSE"), exist_ok=True)
for n in ("START.VBS", "MOOSE.REG", "TRANS.REG", "NOWELCOME.VBS"):
    put(os.path.join("stage/MOOSE", n), "MOOSE/" + n)
put("IMPORT.BAT", "WINDOWS/Start Menu/Programs/StartUp/IMPORT.BAT")

ini = os.path.join(C, "WINDOWS/SYSTEM.INI")
s = open(ini, newline="", encoding="latin-1").read()
if "VIDC.IV41" not in s:
    s = re.sub(r"(?im)^\[drivers32\]\r\n", lambda m: m.group(0) + "VIDC.IV41=ir41_32.dll\r\nVIDC.YVU9=iyvu9_32.dll\r\n", s, count=1)
    open(ini, "w", newline="", encoding="latin-1").write(s)
ini = os.path.join(C, "WINDOWS/WIN.INI")
s = open(ini, newline="", encoding="latin-1").read()
if "[3Dfx]" not in s:
    if not s.endswith("\r\n"):
        s += "\r\n"
    s += "\r\n[3Dfx]\r\nSST_SWAP_EN_WAIT_ON_VSYNC=1\r\n"
    open(ini, "w", newline="", encoding="latin-1").write(s)

for gone in ("EXP", "W98POST"):
    shutil.rmtree(os.path.join(C, gone), ignore_errors=True)
for gone in ("MSBATCH.INF",):
    p = os.path.join(C, gone)
    if os.path.exists(p):
        os.remove(p)
print("copied", len(files), "files")
