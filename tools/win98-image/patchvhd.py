"""Apply `offset base64` lines to a copy of a disk image. usage: patchvhd.py BASE PATCH OUT LENGTH"""
import base64, shutil, sys
base, patch, out, length = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
shutil.copyfile(base, out)
with open(out, "r+b") as f:
    f.truncate(length)
    for line in open(patch):
        off, data = line.split()
        f.seek(int(off))
        f.write(base64.b64decode(data))
print("ok")
