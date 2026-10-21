"""Set FAT32 short (8.3) names on a raw disk image to match another listing.

usage: fatshort.py IMAGE WANT_SLT HAVE_SLT
WANT_SLT and HAVE_SLT are `7zz l -slt` listings; every path whose short name
differs by more than letter case gets the wanted name. The long-name entries'
checksums are updated to match, so Windows keeps the long name.
"""
import struct, sys

def slt(p):
    out = {}
    for block in open(p, errors="replace").read().split("\n\n"):
        f = dict(l.split(" = ", 1) for l in block.splitlines() if " = " in l)
        if "Path" in f:
            out[f["Path"].upper()] = f.get("Short Name", "")
    return out

def packed(name):
    base, _, ext = name.upper().partition(".")
    return (base.ljust(8) + ext.ljust(3)).encode("latin-1")

def checksum(n11):
    s = 0
    for c in n11:
        s = (((s & 1) << 7) + (s >> 1) + c) & 0xFF
    return s

class Fat:
    def __init__(self, path):
        self.f = open(path, "r+b")
        mbr = self.read(0, 512)
        self.part = struct.unpack_from("<I", mbr, 0x1BE + 8)[0] * 512
        bpb = self.read(self.part, 512)
        self.bps, = struct.unpack_from("<H", bpb, 11)
        self.spc = bpb[13]
        res, = struct.unpack_from("<H", bpb, 14)
        nfats = bpb[16]
        fatsz, = struct.unpack_from("<I", bpb, 36)
        self.root, = struct.unpack_from("<I", bpb, 44)
        self.fat = self.part + res * self.bps
        self.data = self.fat + nfats * fatsz * self.bps
        self.csize = self.spc * self.bps

    def read(self, off, n):
        self.f.seek(off)
        return self.f.read(n)

    def write(self, off, b):
        self.f.seek(off)
        self.f.write(b)

    def chain(self, c):
        while 2 <= c < 0x0FFFFFF8:
            yield c
            c = struct.unpack("<I", self.read(self.fat + 4 * c, 4))[0] & 0x0FFFFFFF

    def entries(self, cluster):
        """(long name, short-entry offset, LFN offsets, raw 32 bytes) per entry."""
        lfn, offs = [], []
        for c in self.chain(cluster):
            base = self.data + (c - 2) * self.csize
            buf = self.read(base, self.csize)
            for i in range(0, self.csize, 32):
                e = buf[i:i + 32]
                if e[0] == 0:
                    return
                if e[0] == 0xE5:
                    lfn, offs = [], []
                    continue
                if e[11] == 0x0F:
                    part = (e[1:11] + e[14:26] + e[28:32]).decode("utf-16-le", "replace")
                    lfn.insert(0, part.split("\x00")[0])
                    offs.append(base + i)
                    continue
                short = e[0:8].decode("latin-1").rstrip() + ("." + e[8:11].decode("latin-1").rstrip() if e[8:11].strip() else "")
                yield ("".join(lfn) or short), short, base + i, offs, e
                lfn, offs = [], []

    def find_dir(self, parts):
        c = self.root
        for p in parts:
            for name, short, off, offs, e in self.entries(c):
                if name.upper() == p.upper() and e[11] & 0x10:
                    c = struct.unpack_from("<H", e, 20)[0] << 16 | struct.unpack_from("<H", e, 26)[0]
                    break
            else:
                raise SystemExit("no folder " + "/".join(parts))
        return c

    def rename(self, off, offs, n11):
        self.write(off, n11)
        self.write(off + 12, b"\x00")
        ck = bytes([checksum(n11)])
        for o in offs:
            self.write(o + 13, ck)

def main(image, want_p, have_p):
    want, have = slt(want_p), slt(have_p)
    todo = {}
    for p, w in want.items():
        h = have.get(p)
        if h and w and "~" in w + h and h.upper() != w.upper():
            parent, _, leaf = p.rpartition("/")
            todo.setdefault(parent, {})[leaf] = w
    fat = Fat(image)
    for parent, names in sorted(todo.items(), key=lambda kv: -kv[0].count("/")):
        c = fat.find_dir([x for x in parent.split("/") if x])
        rows = list(fat.entries(c))
        taken = {short.upper() for name, short, *_ in rows if name.upper() not in names}
        for name, short, off, offs, e in rows:
            if name.upper() in names:
                w = names[name.upper()]
                if w.upper() in taken:
                    raise SystemExit(f"{parent}/{name}: {w} is taken")
        # Two passes, so names can be swapped within a folder.
        hits = [(name, off, offs) for name, short, off, offs, e in rows if name.upper() in names]
        for i, (name, off, offs) in enumerate(hits):
            fat.rename(off, offs, packed(f"MOOSE~{i:02d}"))
        for name, off, offs in hits:
            fat.rename(off, offs, packed(names[name.upper()]))
            print(f"{parent}/{name} -> {names[name.upper()]}")
    fat.f.close()

main(*sys.argv[1:])
