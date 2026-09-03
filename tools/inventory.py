#!/usr/bin/env python3
"""Hash every file on the SSD into a SQLite inventory.

The SSD is the canonical copy. This records what is on it in enough detail that
the expensive part -- reading 1.8 TB over USB -- never has to happen twice.
DAT matching, renaming and reference indexing all run later against these rows.

Design comes from ``docs/inventory.md``, with three additions: CRC32 alongside
MD5 and SHA-1 (No-Intro keys on CRC, Redump on SHA-1, and computing all three
costs one read), the full archive member listing as JSON, and the file
fingerprint used for resume.

    tools/inventory.py                 hash everything not already recorded
    tools/inventory.py --systems gb,gba
    tools/inventory.py --status        what is done, what is left

Resume is by ``(system, path, size, mtime)``. A file whose fingerprint is
unchanged is never read again, so interrupting this is free -- rerun it and it
picks up where it stopped.
"""

import argparse
import binascii
import hashlib
import json
import os
import sqlite3
import struct
import subprocess
import sys
import time
import unicodedata
import zipfile
from pathlib import Path

TOOL_VERSION = 6
DEVICE = "ssd"
ROMS = Path("/Volumes/Retro/Roms")
DB = Path(__file__).resolve().parent.parent / "inventory.db"

# Read size. Large enough that USB stays saturated, small enough to stay cheap.
CHUNK = 4 * 1024 * 1024

# Archives we can open and hash the contents of.
ZIP_EXT = {".zip"}
SEVENZ_EXT = {".7z"}
# Formats with no decompressor here. Container hashes still get recorded; the
# inner dump does not, and `inner_status` says why rather than leaving a NULL
# that looks like an oversight.
OPAQUE = {
    ".zcci": "no decompressor for 3DS .zcci",
    ".xci": "no decompressor for Switch .xci",
    ".rvz": "needs dolphin-tool, not installed",
    ".wux": "needs dolphin-tool, not installed",
    ".vpk": "Vita package, not a dump",
}
# Not games. Recorded so the row count matches the tree, flagged so they are
# easy to exclude.
NON_GAME = {".txt", ".xml", ".lnk", ".jar", ".m3u", ".cue", ".sbi", ".dat"}

# Support archives that are emphatically not dumps. `0_BIOS/mame/cheat.7z` holds
# 138,834 members and its listing alone was 8.5 MB of the database -- two of
# these were a third of the whole file. Container hashes still get recorded,
# because a corrupt cheat archive is worth knowing about; the member list does
# not, because nothing will ever ask what is in it.
CHEAT_STEMS = ("cheat", "cheats", "cheats_ws", "cheats_ni")

SCHEMA = """
CREATE TABLE IF NOT EXISTS files (
    id                INTEGER PRIMARY KEY,
    device            TEXT NOT NULL,
    system            TEXT NOT NULL,
    path              TEXT NOT NULL,      -- relative to the system folder, NFC
    ext               TEXT,
    size              INTEGER,
    mtime             REAL,               -- fingerprint half; also the resume key
    ctime             REAL,
    container_format  TEXT,               -- raw, zip, 7z, chd, iso, opaque, non-game
    container_crc32   TEXT,
    container_md5     TEXT,
    container_sha1    TEXT,
    inner_crc32       TEXT,               -- the dump inside an archive
    inner_md5         TEXT,
    inner_sha1        TEXT,
    inner_name        TEXT,
    inner_size        INTEGER,
    member_count      INTEGER,
    members_json      TEXT,               -- every member: name, size, crc
    inner_status      TEXT,               -- ok, opaque, multi-member, error, n/a
    stripped_kind     TEXT,               -- ines, copier
    stripped_md5      TEXT,
    stripped_sha1     TEXT,
    stripped_crc32    TEXT,
    chd_version       INTEGER,
    chd_hunkbytes     INTEGER,
    chd_logicalbytes  INTEGER,
    chd_sha1          TEXT,               -- CHD's own overall SHA-1
    chd_datasha1      TEXT,               -- raw data SHA-1; compares to Redump for DVDs
    chd_truncated     INTEGER,            -- 1 when the map runs past EOF
    -- filled in later, by the DAT pass
    dat_source        TEXT,
    dat_name          TEXT,
    verdict           TEXT,
    proposed_name     TEXT,
    group_id          TEXT,
    status            TEXT,               -- ok, unreadable, error
    error             TEXT,
    hashed_at         TEXT,
    tool_version      INTEGER,
    UNIQUE (device, system, path)
);
CREATE INDEX IF NOT EXISTS files_md5     ON files (container_md5);
CREATE INDEX IF NOT EXISTS files_sha1    ON files (container_sha1);
CREATE INDEX IF NOT EXISTS files_inner   ON files (inner_sha1);
CREATE INDEX IF NOT EXISTS files_crc     ON files (container_crc32);
CREATE INDEX IF NOT EXISTS files_system  ON files (system);

CREATE TABLE IF NOT EXISTS runs (
    id         INTEGER PRIMARY KEY,
    started_at TEXT,
    ended_at   TEXT,
    device     TEXT,
    files      INTEGER,
    bytes      INTEGER,
    note       TEXT
);
"""


def nfc(s):
    """macOS and the Flip store the same CJK characters differently."""
    return unicodedata.normalize("NFC", s)


class Digests:
    """CRC32, MD5 and SHA-1 over one pass of the bytes."""

    def __init__(self):
        self.crc = 0
        self.md5 = hashlib.md5()
        self.sha1 = hashlib.sha1()
        self.size = 0

    def update(self, b):
        self.crc = binascii.crc32(b, self.crc)
        self.md5.update(b)
        self.sha1.update(b)
        self.size += len(b)

    def out(self):
        return (f"{self.crc & 0xFFFFFFFF:08x}", self.md5.hexdigest(), self.sha1.hexdigest())


def hash_stream(fh, skip=0):
    d = Digests()
    if skip:
        fh.read(skip)
    while True:
        b = fh.read(CHUNK)
        if not b:
            break
        d.update(b)
    return d


def hash_file(p, skip=0):
    with open(p, "rb") as fh:
        return hash_stream(fh, skip)


def strip_kind(name, size):
    """Header-stripped hashing, NES and SNES only.

    iNES puts a 16-byte header on the ROM; SNES copiers put 512 bytes on, which
    is why the size test is ``% 1024 == 512`` rather than a fixed offset. Both
    change every hash, so a headered dump matches no DAT until it is stripped.

    Takes a name rather than a path because on this SSD the cartridges are
    zipped: the header is on the member inside, and stripping only the container
    finds nothing. That is worth a header of its own -- a whole NES run came back
    with zero stripped hashes before this took the inner name.
    """
    e = os.path.splitext(str(name))[1].lower()
    if e == ".nes":
        return ("ines", 16)
    if e in (".sfc", ".smc") and size % 1024 == 512:
        return ("copier", 512)
    return (None, 0)


def read_zip(p):
    """Container is already hashed; this is the dump inside."""
    try:
        with zipfile.ZipFile(p) as z:
            infos = [i for i in z.infolist() if not i.is_dir()]
            members = [
                {"name": nfc(i.filename), "size": i.file_size, "crc": f"{i.CRC & 0xFFFFFFFF:08x}"}
                for i in infos
            ]
            if not infos:
                return {"member_count": 0, "members_json": json.dumps(members), "inner_status": "empty"}
            # A 33-member GoodSNES bundle is not a game. Record the listing
            # either way; only hash a single-member archive as "the dump".
            if len(infos) != 1:
                return {
                    "member_count": len(infos),
                    "members_json": json.dumps(members),
                    "inner_status": "multi-member",
                }
            i = infos[0]
            with z.open(i) as fh:
                d = hash_stream(fh)
            crc, md5, sha1 = d.out()
            res = {
                "member_count": 1,
                "members_json": json.dumps(members),
                "inner_crc32": crc,
                "inner_md5": md5,
                "inner_sha1": sha1,
                "inner_name": nfc(i.filename),
                "inner_size": d.size,
                "inner_status": "ok",
            }
            kind, skip = strip_kind(i.filename, d.size)
            if kind:
                with z.open(i) as fh:
                    sd = hash_stream(fh, skip)
                sc, sm, ss = sd.out()
                res.update(stripped_kind=kind, stripped_crc32=sc, stripped_md5=sm, stripped_sha1=ss)
            return res
    except Exception as e:  # a truncated or encrypted zip must not stop the run
        return {"inner_status": "error", "error": f"zip: {e}"}


def read_7z(p):
    try:
        out = subprocess.run(
            ["7z", "l", "-slt", "-ba", str(p)], capture_output=True, text=True, timeout=300
        ).stdout
        members, cur = [], {}
        for line in out.splitlines():
            if line.startswith("Path = "):
                if cur.get("name"):
                    members.append(cur)
                cur = {"name": nfc(line[7:])}
            elif line.startswith("Size = ") and line[7:].strip():
                cur["size"] = int(line[7:])
            elif line.startswith("CRC = ") and line[6:].strip():
                cur["crc"] = line[6:].strip().lower()
        if cur.get("name"):
            members.append(cur)
        files = [m for m in members if m.get("size") is not None]
        res = {"member_count": len(files), "members_json": json.dumps(files)}
        if len(files) != 1:
            res["inner_status"] = "multi-member" if files else "empty"
            return res
        # One member: stream it out and hash without writing to disk.
        proc = subprocess.Popen(["7z", "x", "-so", str(p)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        d = hash_stream(proc.stdout)
        proc.wait()
        crc, md5, sha1 = d.out()
        res.update(
            inner_crc32=crc, inner_md5=md5, inner_sha1=sha1,
            inner_name=files[0]["name"], inner_size=d.size, inner_status="ok",
        )
        kind, skip = strip_kind(files[0]["name"], d.size)
        if kind:
            proc = subprocess.Popen(["7z", "x", "-so", str(p)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            sd = hash_stream(proc.stdout, skip)
            proc.wait()
            sc, sm, ss = sd.out()
            res.update(stripped_kind=kind, stripped_crc32=sc, stripped_md5=sm, stripped_sha1=ss)
        return res
    except Exception as e:
        return {"inner_status": "error", "error": f"7z: {e}"}


def read_chd_header(p):
    """Parse the CHD header directly.

    `chdman` reports truncation only as a vague I/O error, so the map offset is
    checked against the real file size here. That is how a 1.4 GB PS2 CHD was
    found to be missing 46 MB while looking perfectly healthy in a listing.
    """
    try:
        with open(p, "rb") as fh:
            head = fh.read(124)
        if head[:8] != b"MComprHD":
            return {"error": "not a CHD"}
        ver = struct.unpack(">I", head[12:16])[0]
        res = {"chd_version": ver}
        if ver == 5:
            logical, mapoffset, metaoffset = struct.unpack(">QQQ", head[32:56])
            hunkbytes = struct.unpack(">I", head[56:60])[0]
            sha1 = head[84:104].hex()
            rawsha1 = head[64:84].hex()
            actual = p.stat().st_size
            res.update(
                chd_logicalbytes=logical,
                chd_hunkbytes=hunkbytes,
                chd_sha1=sha1,
                chd_datasha1=rawsha1,
                chd_truncated=1 if (mapoffset > actual or metaoffset > actual) else 0,
            )
        return res
    except Exception as e:
        return {"error": f"chd: {e}"}


def is_cheat_db(p):
    return os.path.splitext(p.name)[0].lower() in CHEAT_STEMS


def fmt_of(p):
    e = p.suffix.lower()
    if is_cheat_db(p):
        return "support"
    if e in ZIP_EXT:
        return "zip"
    if e in SEVENZ_EXT:
        return "7z"
    if e == ".chd":
        return "chd"
    if e == ".iso":
        return "iso"
    if e in OPAQUE:
        return "opaque"
    if e in NON_GAME:
        return "non-game"
    return "raw"


def record(p, system, rel):
    st = p.stat()
    row = {
        "device": DEVICE,
        "system": system,
        "path": nfc(rel),
        "ext": p.suffix.lower(),
        "size": st.st_size,
        "mtime": st.st_mtime,
        "ctime": st.st_ctime,
        "container_format": fmt_of(p),
        "status": "ok",
        "hashed_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "tool_version": TOOL_VERSION,
    }
    try:
        crc, md5, sha1 = hash_file(p).out()
        row.update(container_crc32=crc, container_md5=md5, container_sha1=sha1)
    except Exception as e:
        row.update(status="unreadable", error=str(e))
        return row

    fmt = row["container_format"]
    if fmt == "support":
        # Explicit NULLs, not omissions: the upsert only writes columns the row
        # carries, so a file that was enumerated under an older version would
        # keep its stale 8.5 MB listing forever.
        row.update(
            inner_status="cheat-db, not enumerated",
            members_json=None, member_count=None,
            inner_crc32=None, inner_md5=None, inner_sha1=None,
            inner_name=None, inner_size=None,
        )
    elif fmt == "zip":
        row.update(read_zip(p))
    elif fmt == "7z":
        row.update(read_7z(p))
    elif fmt == "chd":
        row.update(read_chd_header(p))
        row["inner_status"] = "n/a"
    elif fmt == "opaque":
        row["inner_status"] = OPAQUE.get(row["ext"], "opaque")
    else:
        row["inner_status"] = "n/a"

    kind, skip = strip_kind(p.name, row["size"])
    if kind and not row.get("stripped_kind"):
        try:
            c, m, s = hash_file(p, skip).out()
            row.update(stripped_kind=kind, stripped_crc32=c, stripped_md5=m, stripped_sha1=s)
        except Exception as e:
            row["error"] = f"strip: {e}"
    return row


def connect():
    db = sqlite3.connect(DB, timeout=60)
    db.execute("PRAGMA journal_mode=WAL")
    db.execute("PRAGMA synchronous=NORMAL")
    db.executescript(SCHEMA)
    return db


def walk(system_dir):
    """`find`, not `ls` -- several systems nest, and a flat listing loses them."""
    for dirpath, _dirs, names in os.walk(system_dir):
        for n in names:
            if n.startswith("."):
                continue
            p = Path(dirpath) / n
            if p.is_file():
                yield p, nfc(str(p.relative_to(system_dir)))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--systems", help="comma-separated, default all")
    ap.add_argument("--status", action="store_true")
    ap.add_argument("--roms", default=str(ROMS))
    args = ap.parse_args()

    roms = Path(args.roms)
    db = connect()

    if args.status:
        for sysname, n, done, b in db.execute(
            "SELECT system, COUNT(*), SUM(status='ok'), SUM(size) FROM files GROUP BY system ORDER BY 4 DESC"
        ):
            print(f"  {sysname:<24} {n:>6} files  {done:>6} ok  {(b or 0)/2**30:8.1f} GB")
        tot = db.execute("SELECT COUNT(*), SUM(size) FROM files").fetchone()
        print(f"  {'TOTAL':<24} {tot[0]:>6} files  {(tot[1] or 0)/2**30:20.1f} GB")
        return

    if args.systems:
        systems = args.systems.split(",")
    else:
        systems = [d.name for d in roms.iterdir() if d.is_dir()]

    # Smallest system first, so the cartridge data -- the part with DAT coverage
    # and the part worth having tonight -- lands within the hour rather than
    # behind 500 GB of PS3. Sizing is a metadata walk, cheap next to the hashing.
    def total(s):
        n = 0
        for _dp, _dirs, names in os.walk(roms / s):
            for f in names:
                try:
                    n += os.stat(os.path.join(_dp, f)).st_size
                except OSError:
                    pass
        return n

    systems.sort(key=total)

    run = db.execute(
        "INSERT INTO runs (started_at, device, note) VALUES (?,?,?)",
        (time.strftime("%Y-%m-%dT%H:%M:%S"), DEVICE, f"tool v{TOOL_VERSION}"),
    ).lastrowid
    db.commit()

    # Version is part of the fingerprint: when the hasher learns something new
    # -- as it did about headers inside archives -- old rows are stale and have
    # to be read again, however unchanged the file is.
    seen = {
        (s, p): (sz, mt)
        for s, p, sz, mt in db.execute(
            "SELECT system, path, size, mtime FROM files WHERE device=? AND tool_version >= ?",
            (DEVICE, TOOL_VERSION),
        )
    }
    n_new = n_skip = 0
    bytes_done = 0
    t0 = time.time()

    for system in systems:
        sysdir = roms / system
        if not sysdir.is_dir():
            continue
        batch = []
        for p, rel in walk(sysdir):
            try:
                st = p.stat()
            except OSError:
                continue
            prev = seen.get((system, rel))
            if prev and abs(prev[0] - st.st_size) == 0 and abs(prev[1] - st.st_mtime) < 1e-6:
                n_skip += 1
                continue
            row = record(p, system, rel)
            batch.append(row)
            n_new += 1
            bytes_done += row.get("size") or 0
            if len(batch) >= 25:
                flush(db, batch)
                batch = []
        if batch:
            flush(db, batch)
        el = time.time() - t0
        rate = bytes_done / el / 2**20 if el else 0
        print(
            f"  {system:<24} done | new {n_new:>6} skip {n_skip:>6} "
            f"| {bytes_done/2**30:7.1f} GB | {rate:6.1f} MB/s",
            flush=True,
        )

    db.execute(
        "UPDATE runs SET ended_at=?, files=?, bytes=? WHERE id=?",
        (time.strftime("%Y-%m-%dT%H:%M:%S"), n_new, bytes_done, run),
    )
    db.commit()
    print(f"\n  {n_new} hashed, {n_skip} already current, {bytes_done/2**30:.1f} GB read")


def flush(db, batch):
    cols = sorted({k for r in batch for k in r})
    sql = (
        f"INSERT INTO files ({','.join(cols)}) VALUES ({','.join('?' * len(cols))}) "
        f"ON CONFLICT(device,system,path) DO UPDATE SET "
        + ",".join(f"{c}=excluded.{c}" for c in cols if c not in ("device", "system", "path"))
    )
    db.executemany(sql, [[r.get(c) for c in cols] for r in batch])
    db.commit()


if __name__ == "__main__":
    sys.exit(main())
