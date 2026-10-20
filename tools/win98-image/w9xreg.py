"""Read a Windows 95/98 registry file (SYSTEM.DAT, USER.DAT: the CREG format)
or a REGEDIT4 export into {key: {value: (type, data)}}, keys in upper case.

The CREG layout follows Wine's old loader: a CREG header, one RGKN block of
fixed 0x1c-byte key nodes linked by parent/child/next offsets, then RGDB
blocks holding each key's name and values, matched to a node by its id.
"""
import struct
import sys

def read_creg(path, root):
    b = open(path, "rb").read()
    assert b[:4] == b"CREG", "not a CREG file"
    rgdb_off, = struct.unpack_from("<I", b, 8)
    rgdb_num, = struct.unpack_from("<H", b, 0x10)
    rgkn = 0x20
    assert b[rgkn:rgkn + 4] == b"RGKN"
    root_off, = struct.unpack_from("<I", b, rgkn + 8)
    # Key records by (block, id).
    recs = {}
    off = rgdb_off
    for _ in range(rgdb_num):
        assert b[off:off + 4] == b"RGDB", hex(off)
        size, = struct.unpack_from("<I", b, off + 4)
        p, end = off + 0x20, off + size
        while p + 20 <= end:
            ksize, nr_ls, nr_ms, used, nlen, nvals = struct.unpack_from("<IHHIHH", b, p)
            if ksize == 0 or ksize == 0xFFFFFFFF:
                break
            if ksize < 20:
                break
            # A free record: its id is 0xFFFF and what follows is leftover.
            if nr_ls == 0xFFFF or used > ksize:
                p += ksize
                continue
            name = b[p + 20:p + 20 + nlen].decode("latin-1")
            vals = {}
            q = p + 20 + nlen
            rec_end = p + used
            for _ in range(nvals):
                if q + 12 > rec_end:
                    break
                vtype, _x, vnlen, vdlen = struct.unpack_from("<IIHH", b, q)
                vname = b[q + 12:q + 12 + vnlen].decode("latin-1")
                vdata = b[q + 12 + vnlen:q + 12 + vnlen + vdlen]
                vals[vname.upper()] = (vname, vtype, vdata)
                q += 12 + vnlen + vdlen
            recs[(nr_ms, nr_ls)] = (name, vals)
            p += ksize
        off += size
    out = {}
    def walk(node_off, path):
        while node_off != 0xFFFFFFFF:
            n = rgkn + node_off
            _inuse, _h, _nf, _par, child, nxt, nr_ls, nr_ms = struct.unpack_from("<IIIIIIHH", b, n)
            rec = recs.get((nr_ms, nr_ls))
            name, vals = rec if rec else ("", {})
            p = f"{path}\\{name}" if path else name
            if rec is not None:
                out[p.upper()] = (p, vals)
            if child != 0xFFFFFFFF:
                walk(child, p)
            node_off = nxt
    sys.setrecursionlimit(100000)
    _inuse, _h, _nf, _par, child, nxt, nr_ls, nr_ms = struct.unpack_from("<IIIIIIHH", b, rgkn + root_off)
    out[root.upper()] = (root, recs.get((nr_ms, nr_ls), ("", {}))[1])
    if child != 0xFFFFFFFF:
        walk(child, root)
    return out

def read_reg(path):
    """A REGEDIT4 export, values kept as their text form."""
    text = open(path, encoding="latin-1").read().replace("\r\n", "\n")
    lines, buf = [], ""
    for line in text.split("\n"):
        if line.endswith("\\") and not line.startswith("["):
            buf += line[:-1].strip()
            continue
        lines.append(buf + line.strip() if buf else line)
        buf = ""
    out, cur = {}, None
    for line in lines:
        if line.startswith("[") and line.endswith("]"):
            key = line[1:-1]
            cur = out.setdefault(key.upper(), (key, {}))
        elif cur is not None and "=" in line:
            if line.startswith("@="):
                name, data = "", line[2:]
            elif line.startswith('"'):
                i, esc = 1, False
                while i < len(line):
                    if line[i] == "\\" and not esc:
                        esc = True
                    elif line[i] == '"' and not esc:
                        break
                    else:
                        esc = False
                    i += 1
                name, data = line[1:i].replace('\\"', '"').replace("\\\\", "\\"), line[i + 2:]
            else:
                continue
            cur[1][name.upper()] = (name, data)
    return out

def as_text(vtype, data):
    """A CREG value in REGEDIT4 form."""
    if vtype == 1:
        s = data.rstrip(b"\0").decode("latin-1")
        return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'
    if vtype == 4 and len(data) == 4:
        return "dword:%08x" % struct.unpack("<I", data)[0]
    if vtype == 3:
        return "hex:" + ",".join("%02x" % c for c in data)
    return "hex(%x):" % vtype + ",".join("%02x" % c for c in data)

if __name__ == "__main__":
    reg = read_creg(sys.argv[1], sys.argv[2])
    print(len(reg), "keys")
    for k in list(reg)[:5]:
        print(reg[k][0], len(reg[k][1]))
