"""Zero every free cluster of the FAT32 partition in a raw disk image.

usage: zerofree.py IMAGE
Deleted files leave their data behind, and a dynamic VHD stores every block
that is not all zeroes, so this makes the VHD only as big as what is in use.
"""
import struct, sys

f = open(sys.argv[1], "r+b")
f.seek(0)
mbr = f.read(512)
part = struct.unpack_from("<I", mbr, 0x1BE + 8)[0] * 512
f.seek(part)
bpb = f.read(512)
bps, = struct.unpack_from("<H", bpb, 11)
spc = bpb[13]
res, = struct.unpack_from("<H", bpb, 14)
nfats = bpb[16]
fatsz, = struct.unpack_from("<I", bpb, 36)
total, = struct.unpack_from("<I", bpb, 32)
fat_off = part + res * bps
data = fat_off + nfats * fatsz * bps
csize = spc * bps
nclusters = (total - res - nfats * fatsz) // spc
f.seek(fat_off)
fat = f.read(fatsz * bps)
zero = bytes(csize)
cleared = free = 0
for c in range(2, nclusters + 2):
    if struct.unpack_from("<I", fat, 4 * c)[0] & 0x0FFFFFFF:
        continue
    free += 1
    off = data + (c - 2) * csize
    f.seek(off)
    if f.read(csize) != zero:
        f.seek(off)
        f.write(zero)
        cleared += 1
f.close()
print(f"{free} free clusters, {cleared} had old data ({cleared * csize // 2**20} MB zeroed)")
