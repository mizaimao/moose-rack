"""What eXo's Windows 98 image has that a plain Windows 98 SE install does not.

Reads 7z's -slt listing of the image and the file names in the CD's cabinets.
"""
import collections
import re
import sys

listing, cabs = sys.argv[1], sys.argv[2]
known = set(open(cabs).read().split())

files = []
cur = {}
for line in open(listing, encoding="utf-8", errors="replace"):
    line = line.rstrip("\n")
    if line.startswith("Path = "):
        if cur:
            files.append(cur)
        cur = {"path": line[7:]}
    elif " = " in line and cur:
        k, v = line.split(" = ", 1)
        cur[k] = v
if cur:
    files.append(cur)
files = [f for f in files if f.get("Folder") == "-" and "Short Name" in f]

# Made by Windows itself on first boot or by use, not installed.
generated = re.compile(r"(?i)(\.dat|\.da0|\.log|\.pwl|\.grp|\.bak|\.old|\.tmp|\.lnk|\.pif|\.url|\.ini|\.inf|\.pnf|\.cat|\.000|\.001|\.---|\.txt|\.bmp|\.htt|\.cpl|\.scr|\.wav|\.ico)$|^windows/(temp|recent|cookies|history|temporary internet files|application data|sysbckup)/")

added = [f for f in files if f["path"].split("/")[-1].lower() not in known]
by_top = collections.defaultdict(list)
for f in added:
    p = f["path"]
    parts = p.split("/")
    if parts[0].lower() == "program files" and len(parts) > 2:
        key = "Program Files/" + parts[1]
    elif parts[0].lower() == "windows" and len(parts) > 2:
        key = "Windows/" + parts[1]
    else:
        key = parts[0] if len(parts) > 1 else "(root)"
    by_top[key].append(f)

def size(fs):
    return sum(int(f.get("Size", "0") or 0) for f in fs)

print(f"image: {len(files)} files; not on the CD: {len(added)}")
for key, fs in sorted(by_top.items(), key=lambda kv: -size(kv[1])):
    names = sorted({f['path'].split('/')[-1] for f in fs if not generated.search(f['path'])})
    print(f"\n## {key}: {len(fs)} files, {size(fs)/1e6:.1f} MB")
    print("   " + ", ".join(names[:60]) + (" …" if len(names) > 60 else ""))
