"""Turn a dynamic VHD back into a fixed one: the raw disk with the footer after it.

usage: vhdraw.py DYNAMIC.vhd FIXED.vhd

The reverse of vhddyn.py, for changing a disk that has already been published:
a fixed VHD is a raw image that macOS can mount (`hdiutil attach -imagekey
diskimage-class=CRawDiskImage`) and that zerofree.py and vhddyn.py read. The
footer is kept, size, geometry and UUID included, so DOSBox-X sees the same
disk; only the disk type and the data offset change.
"""
import struct
import sys

SECTOR = 512


def checksum(b):
    return (~sum(b)) & 0xFFFFFFFF


def main(src, dst):
    f = open(src, "rb")
    footer = bytearray(f.read(512))
    assert footer[:8] == b"conectix", "no VHD footer at the start"
    assert struct.unpack_from(">I", footer, 60)[0] == 3, "not a dynamic VHD"
    size = struct.unpack_from(">Q", footer, 48)[0]
    header_at = struct.unpack_from(">Q", footer, 16)[0]
    f.seek(header_at)
    header = f.read(1024)
    assert header[:8] == b"cxsparse", "no dynamic header"
    table_at, _, entries, block = struct.unpack_from(">QIII", header, 16)
    f.seek(table_at)
    bat = struct.unpack(f">{entries}I", f.read(entries * 4))
    bitmap = (block // SECTOR + 7) // 8
    bitmap = (bitmap + SECTOR - 1) // SECTOR * SECTOR

    out = open(dst, "wb")
    out.truncate(size)
    used = 0
    for i, at in enumerate(bat):
        if at == 0xFFFFFFFF:
            continue
        # Every sector of a stored block is present: vhddyn.py writes the
        # bitmap all ones, and DOSBox-X does the same for the blocks it adds.
        f.seek(at * SECTOR + bitmap)
        data = f.read(min(block, size - i * block))
        out.seek(i * block)
        out.write(data)
        used += 1

    struct.pack_into(">Q", footer, 16, 0xFFFFFFFFFFFFFFFF)  # no dynamic header
    struct.pack_into(">I", footer, 60, 2)                     # fixed
    struct.pack_into(">I", footer, 64, 0)
    struct.pack_into(">I", footer, 64, checksum(footer))
    out.seek(size)
    out.write(bytes(footer))
    out.close()
    print(f"{used} of {entries} blocks, {size} bytes")


if __name__ == "__main__":
    main(*sys.argv[1:])
