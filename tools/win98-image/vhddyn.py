"""Turn a fixed VHD into a dynamic one that stores only non-empty blocks.

usage: vhddyn.py FIXED.vhd DYNAMIC.vhd
The footer (size, geometry, creator, UUID) is kept from the fixed VHD, so
DOSBox-X sees the same disk it made; only the disk type changes.
"""
import struct, sys

BLOCK = 2 * 1024 * 1024
SECTOR = 512

def checksum(b):
    return (~sum(b)) & 0xFFFFFFFF

src, dst = sys.argv[1:]
f = open(src, "rb")
f.seek(-512, 2)
footer = bytearray(f.read(512))
assert footer[:8] == b"conectix" and struct.unpack_from(">I", footer, 60)[0] == 2, "not a fixed VHD"
size = struct.unpack_from(">Q", footer, 48)[0]

struct.pack_into(">Q", footer, 16, 512)      # data offset: the dynamic header
struct.pack_into(">I", footer, 60, 3)        # disk type: dynamic
struct.pack_into(">I", footer, 64, 0)
struct.pack_into(">I", footer, 64, checksum(footer))

entries = (size + BLOCK - 1) // BLOCK
bat_bytes = (entries * 4 + SECTOR - 1) // SECTOR * SECTOR
header = bytearray(1024)
header[:8] = b"cxsparse"
struct.pack_into(">QQIII", header, 8, 0xFFFFFFFFFFFFFFFF, 1536, 0x00010000, entries, BLOCK)
struct.pack_into(">I", header, 36, checksum(header))

bat = [0xFFFFFFFF] * entries
out = open(dst, "wb")
out.write(bytes(footer) + bytes(header) + bytes(bat_bytes))
pos = 1536 + bat_bytes
bitmap = b"\xff" * SECTOR                    # 4096 sectors per block, all present
used = 0
f.seek(0)
for i in range(entries):
    data = f.read(min(BLOCK, size - i * BLOCK))
    if data.count(0) == len(data):
        continue
    data = data.ljust(BLOCK, b"\0")
    bat[i] = pos // SECTOR
    out.write(bitmap + data)
    pos += SECTOR + BLOCK
    used += 1
out.write(bytes(footer))
out.seek(1536)
out.write(struct.pack(f">{entries}I", *bat))
out.close()
print(f"{used} of {entries} blocks stored, {pos // 2**20} MB")
