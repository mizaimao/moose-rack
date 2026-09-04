#!/usr/bin/env python3
"""Copy artwork off the SSD onto the Miyoo Flip, and point its gamelists at it.

The SSD is canonical -- `tools/pull-android-media.py` keeps it that way -- and
the Flip is the copy that lags. This fills what the Flip is missing and never
overwrites art already there unless the SSD's source is newer.

Three things make this different from the ES-DE machines, and each one has cost
something before:

**The Flip does not find media by convention.** ES-DE looks beside the ROM name;
the Flip reads the path out of `gamelist.xml`. A file dropped into `images/`
with no `<image>` tag pointing at it is invisible, which is how `pcengine` sat
at 292 games and 2 entries. So this writes the tag as well as the file.

**The image is the miximage downscaled to 640x480**, the panel's size. Verified
against the images already on the device rather than assumed.

**System names differ.** The SSD is ES-DE's spelling (`genesis`), the Flip is
the library's (`megadrive`), and the SSD splits `nes`/`famicom` and `snes`/`sfc`
where the Flip merges them. The map below is the whole translation.

    tools/push-flip-media.py                 what would go across
    tools/push-flip-media.py --apply         send it
    tools/push-flip-media.py --only snes     one system
    tools/push-flip-media.py --check-size    resend anything not 640x480
    tools/push-flip-media.py --no-gamelists  files only, tags left alone

Talking to the device is password auth through `expect`: KNULLI's sshd refuses
keys because StrictModes sees a 0777 home, and an earlier attempt lost a day to
that. See `docs/devices.md`.
"""

import argparse
import base64
import gzip
import html
import io
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor

SSD = "/Volumes/Retro/ES-DE/support/downloaded_media"
FLIP = os.environ.get("FLIP", "knulli.local")
PASSWORD = os.environ.get("FLIP_PASSWORD", "linux")
ROMS = "/userdata/roms"
STAGE = "/userdata/system/tmp/moose-media"

# Flip system -> the SSD media folders to look in, in order. The Flip keeps one
# folder where the SSD keeps two: its `nes` holds the famicom set as well.
SYSTEMS = {
    "dreamcast": ["dreamcast"],
    "fbneo": ["arcade", "mame"],
    "gamegear": ["gamegear"],
    "gb": ["gb", "gb_superset"],
    "gba": ["gba"],
    "gbc": ["gbc", "gbc_superset"],
    "megadrive": ["genesis"],
    "n64": ["n64"],
    "neogeo": ["neogeo"],
    "nes": ["nes", "famicom"],
    "pcengine": ["pcengine"],
    "psx": ["psx"],
    "snes": ["snes", "sfc"],
}

# Miximages first: that is what the device already holds, and a cover next to a
# miximage in the same list looks like a bug. Covers are the fallback for a game
# the scraper only ever got a box for.
KINDS = ["miximages", "covers"]
EXTS = [".png", ".jpg", ".webp"]
CACHE = pathlib.Path("library/media-flip")
# The panel, and so the size every image on the device should be.
SCREEN = (640, 480)

EXPECT_SSH = r"""
set timeout 900
spawn ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
          -o LogLevel=ERROR -o ConnectTimeout=8 root@%s [lindex $argv 0]
expect {
  -re "(P|p)assword:" { send "%s\r"; exp_continue }
  timeout { exit 2 }
  eof { }
}
catch wait result
exit [lindex $result 3]
"""

EXPECT_SCP = r"""
set timeout 3600
spawn scp -O -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
          -o LogLevel=ERROR [lindex $argv 0] root@%s:[lindex $argv 1]
expect {
  -re "(P|p)assword:" { send "%s\r"; exp_continue }
  timeout { exit 2 }
  eof { }
}
catch wait result
exit [lindex $result 3]
"""

_scripts = {}


def _expect(kind, body):
    """One temp expect script per kind, kept for the run."""
    if kind not in _scripts:
        f = tempfile.NamedTemporaryFile("w", suffix=".exp", delete=False)
        f.write(body % (FLIP, PASSWORD))
        f.close()
        _scripts[kind] = f.name
    return _scripts[kind]


def ssh(cmd, check=True):
    """Run one shell command on the device and return its stdout.

    stdin is /dev/null: an ssh inside a `while read` loop otherwise eats the
    loop's input and the loop runs once, reporting success.
    """
    p = subprocess.run(["expect", "-f", _expect("ssh", EXPECT_SSH), cmd],
                       capture_output=True, text=True, stdin=subprocess.DEVNULL)
    if check and p.returncode != 0:
        sys.exit(f"ssh failed ({p.returncode}): {cmd}\n{p.stdout}{p.stderr}")
    # The pty brings the spawn line, the password prompt and carriage returns.
    out = p.stdout.replace("\r", "")
    return "\n".join(l for l in out.split("\n")[1:] if "assword:" not in l)


def scp(local, remote):
    p = subprocess.run(["expect", "-f", _expect("scp", EXPECT_SCP), str(local), remote],
                       capture_output=True, text=True, stdin=subprocess.DEVNULL)
    if p.returncode != 0:
        sys.exit(f"scp failed ({p.returncode}): {local} -> {remote}\n{p.stdout}{p.stderr}")


def run_script(text):
    """Send a shell script and run it there.

    Remote paths carry spaces, brackets and apostrophes; quoting one through
    expect, ssh and the remote shell is three layers deep and gets it wrong
    quietly. A file does not have that problem.
    """
    with tempfile.NamedTemporaryFile("w", suffix=".sh", delete=False) as f:
        f.write(text)
        local = f.name
    ssh(f"mkdir -p {STAGE}")
    scp(local, f"{STAGE}/run.sh")
    os.unlink(local)
    return ssh(f"sh {STAGE}/run.sh")


def survey(systems, sizes=False):
    """ROMs, images and gamelists as they are on the device right now."""
    sysl = " ".join(systems)
    script = f"""
for s in {sysl}; do
  d={ROMS}/$s
  [ -d "$d" ] || continue
  find "$d" -type f ! -path "*/images/*" ! -name "gamelist.xml" ! -name "*.txt" \\
       -printf "R\\t$s\\t%P\\n"
  find "$d/images" -maxdepth 1 -type f -printf "I\\t$s\\t%f\\t%T@\\n" 2>/dev/null
done
"""
    listing = run_script(script)
    roms, imgs = {s: set() for s in systems}, {s: {} for s in systems}
    for line in listing.split("\n"):
        parts = line.split("\t")
        if parts[0] == "R" and len(parts) >= 3 and parts[1] in roms:
            roms[parts[1]].add(parts[2])
        elif parts[0] == "I" and len(parts) >= 4 and parts[1] in imgs:
            imgs[parts[1]][parts[2]] = float(parts[3])

    dims = {s: {} for s in systems}
    if sizes:
        # PC Engine's art was 580x680 portrait box scans while every other
        # system held 640x480 miximages, and nothing here could tell: a file of
        # the right name in the right folder passed every check. The PNG header
        # is the only thing that says what the picture actually is, so read it.
        #
        # `IFS= read -r` and a quoted "$f": these filenames carry spaces,
        # brackets and apostrophes, and word splitting turns one of them into
        # four files that do not exist.
        hdr = run_script(f"""
for s in {sysl}; do
  find {ROMS}/$s/images -maxdepth 1 -type f 2>/dev/null | while IFS= read -r f; do
    printf "D\\t%s\\t%s\\t" "$s" "${{f##*/}}"
    od -An -tx1 -N24 "$f" | tr -d " \\n"
    echo
  done
done
""")
        for line in hdr.split("\n"):
            parts = line.split("\t")
            if parts[0] != "D" or len(parts) < 4 or parts[1] not in dims:
                continue
            b = parts[3]
            # 8-byte signature, 4-byte length, "IHDR", then width and height.
            if len(b) < 48 or not b.startswith("89504e47"):
                continue
            dims[parts[1]][parts[2]] = (int(b[32:40], 16), int(b[40:48], 16))
    return roms, imgs, lists_of(systems), dims


def lists_of(systems):
    """The gamelists, off the device, as text."""
    files = " ".join(f"{s}/gamelist.xml" for s in systems)
    b64 = run_script(f"cd {ROMS} && tar cf - {files} 2>/dev/null | gzip -c | base64\n")
    # Only the base64 lines: the pty brings a password prompt and a shell
    # banner with it, and one stray character makes the whole stream garbage.
    body = "".join(l.strip() for l in b64.split("\n")
                   if l.strip() and re.fullmatch(r"[A-Za-z0-9+/=]+", l.strip()))
    lists = {}
    if body:
        data = base64.b64decode(body + "==")
        tf = tarfile.open(fileobj=io.BytesIO(gzip.decompress(data)))
        for m in tf.getmembers():
            if m.isfile():
                lists[pathlib.PurePath(m.name).parent.name] = \
                    tf.extractfile(m).read().decode("utf-8", "replace")
    return lists


def index_ssd(root, systems):
    """(system, kind, stem) -> source file on the SSD."""
    idx = {}
    for s in systems:
        for folder in SYSTEMS[s]:
            for kind in KINDS:
                d = root / folder / kind
                if not d.is_dir():
                    continue
                for f in d.iterdir():
                    if f.suffix.lower() in EXTS:
                        idx.setdefault((s, kind, f.stem), f)
    return idx


def source_for(idx, system, stem):
    for kind in KINDS:
        hit = idx.get((system, kind, stem))
        if hit:
            return hit
    return None


def blocks_of(xml):
    return re.findall(r"<game>.*?</game>", xml, re.S)


def path_of(block):
    m = re.search(r"<path>(.*?)</path>", block, re.S)
    return html.unescape(m.group(1)).lstrip("./") if m else None


def plan(roms, imgs, lists, idx, dims, refresh, force):
    """What to send, what to tag, and what has no source anywhere."""
    jobs, tags, entries, nosrc, wrong = [], [], [], [], []
    for s in sorted(roms):
        xml = lists.get(s, "")
        tagged = {}
        for b in blocks_of(xml):
            rel = path_of(b)
            if rel:
                tagged[rel] = "<image>" in b
        for rel in sorted(roms[s]):
            stem = pathlib.PurePath(rel).stem
            name = f"{stem}-image.png"
            on_dev = imgs[s].get(name)
            src = source_for(idx, s, stem)
            size = dims.get(s, {}).get(name)
            misfit = size is not None and size != SCREEN
            if misfit:
                wrong.append((s, name, size))
            resend = (on_dev is None or force or misfit
                      or (refresh and src and src.stat().st_mtime > on_dev + 2))
            if src and resend:
                jobs.append((s, stem, src))
            elif not src and on_dev is None:
                nosrc.append((s, rel))
            will_have = on_dev is not None or src is not None
            if not will_have:
                continue
            if rel not in tagged:
                entries.append((s, rel, stem))
            elif not tagged[rel]:
                tags.append((s, rel, stem))
    return jobs, tags, entries, nosrc, wrong


def resize(jobs, apply_):
    """Miximage -> 640x480 PNG in the local cache, named the way the Flip wants.

    `sips -z` is height then width. The cache is kept so a re-run after a failed
    push does not resize two thousand images again.
    """
    todo = []
    for s, stem, src in jobs:
        dst = CACHE / s / f"{stem}-image.png"
        if not dst.exists() or dst.stat().st_mtime < src.stat().st_mtime:
            todo.append((src, dst))
    if not apply_:
        return len(todo)
    for _, dst in todo:
        dst.parent.mkdir(parents=True, exist_ok=True)

    def one(pair):
        src, dst = pair
        p = subprocess.run(["sips", "-z", "480", "640", str(src), "--out", str(dst)],
                           capture_output=True, text=True)
        return None if p.returncode == 0 else f"{src}: {p.stderr.strip()}"

    bad = []
    with ThreadPoolExecutor(max_workers=8) as pool:
        for i, err in enumerate(pool.map(one, todo), 1):
            if err:
                bad.append(err)
            if i % 200 == 0:
                print(f"  resized {i}/{len(todo)}", flush=True)
    for e in bad[:10]:
        print(f"  resize failed: {e}", file=sys.stderr)
    return len(todo) - len(bad)


def push(jobs):
    """One tar per system. Hundreds of scp calls spend their time in round trips.

    COPYFILE_DISABLE keeps macOS from writing `._` members into the archive.
    `tar` on the device complains `Cannot change ownership` for every entry --
    /userdata is exFAT and has no Unix ownership; the extraction worked.
    """
    by_sys = {}
    for s, stem, _ in jobs:
        by_sys.setdefault(s, []).append(f"{stem}-image.png")
    for s, names in sorted(by_sys.items()):
        with tempfile.NamedTemporaryFile(suffix=".tar.gz", delete=False) as f:
            tarball = f.name
        listing = tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False)
        listing.write("\n".join(names) + "\n")
        listing.close()
        env = dict(os.environ, COPYFILE_DISABLE="1")
        subprocess.run(["tar", "czf", tarball, "-C", str(CACHE / s), "-T", listing.name],
                       check=True, env=env)
        os.unlink(listing.name)
        size = os.path.getsize(tarball) / 1024 ** 2
        print(f"  {s}: sending {len(names)} images ({size:.0f} MB)", flush=True)
        t0 = time.time()
        scp(tarball, f"{STAGE}/{s}.tar.gz")
        os.unlink(tarball)
        out = run_script(
            f"mkdir -p {ROMS}/{s}/images\n"
            f"tar xzf {STAGE}/{s}.tar.gz -C {ROMS}/{s}/images 2>/dev/null\n"
            f"rm -f {STAGE}/{s}.tar.gz\n"
            f"ls {ROMS}/{s}/images | wc -l\n")
        got = re.findall(r"\d+", out)
        print(f"  {s}: {time.time()-t0:.0f}s, {got[-1] if got else '?'} images on device")


def rewrite(xml, tags, entries):
    """Add `<image>` where a game has none, and a whole entry where it has none.

    Text edits, not an XML parse: `gamegear`'s list carries a raw `&` from a
    filename and a strict parser rejects the file outright. It was like that
    before we arrived, and rewriting is not the place to find out.
    """
    want = {rel: stem for _, rel, stem in tags}
    added = 0
    out, pos = [], 0
    for m in re.finditer(r"<game>.*?</game>", xml, re.S):
        b = m.group(0)
        out.append(xml[pos:m.start()])
        rel = path_of(b)
        if rel in want and "<image>" not in b:
            img = html.escape(f"./images/{want[rel]}-image.png")
            b = re.sub(r"(</path>)", lambda mm: mm.group(1) + f"\n\t\t<image>{img}</image>",
                       b, count=1)
            added += 1
        out.append(b)
        pos = m.end()
    out.append(xml[pos:])
    xml = "".join(out)

    new = []
    for _, rel, stem in entries:
        img = html.escape(f"./images/{stem}-image.png")
        new.append("\t<game>\n"
                   f"\t\t<path>{html.escape('./' + rel)}</path>\n"
                   f"\t\t<name>{html.escape(pathlib.PurePath(rel).stem)}</name>\n"
                   f"\t\t<image>{img}</image>\n"
                   "\t</game>\n")
    if new:
        if "</gameList>" in xml:
            xml = xml.replace("</gameList>", "".join(new) + "</gameList>", 1)
        else:
            xml = xml.rstrip() + "\n" + "".join(new) + "</gameList>\n"
    return xml, added, len(new)


def write_gamelists(lists, tags, entries, apply_):
    by_sys = {}
    for kind, rows in (("tag", tags), ("entry", entries)):
        for s, rel, stem in rows:
            by_sys.setdefault(s, {"tag": [], "entry": []})[kind].append((s, rel, stem))
    stamp = time.strftime("%Y%m%d-%H%M")
    for s, work in sorted(by_sys.items()):
        xml = lists.get(s)
        if xml is None:
            print(f"  {s}: no gamelist on the device, skipped", file=sys.stderr)
            continue
        new, added, appended = rewrite(xml, work["tag"], work["entry"])
        # Favourites are in this file too, and losing one is silent. Count them.
        before, after = xml.count("<favorite>"), new.count("<favorite>")
        games_b, games_a = xml.count("<game>"), new.count("<game>")
        ok = (after == before and games_a == games_b + appended
              and new.count("<image>") == xml.count("<image>") + added + appended)
        print(f"  {s}: +{added} tags, +{appended} entries, "
              f"favourites {before}->{after}, games {games_b}->{games_a}"
              f"{'' if ok else '   REFUSED'}")
        if not ok:
            print(f"  {s}: rewrite did not add up, left alone", file=sys.stderr)
            continue
        if not apply_:
            continue
        with tempfile.NamedTemporaryFile("w", suffix=".xml", delete=False,
                                         encoding="utf-8") as f:
            f.write(new)
            local = f.name
        ssh(f"mkdir -p {STAGE}")
        scp(local, f"{STAGE}/{s}-gamelist.xml")
        os.unlink(local)
        run_script(
            f"cp {ROMS}/{s}/gamelist.xml {ROMS}/{s}/gamelist.xml.bak-{stamp}\n"
            f"mv {STAGE}/{s}-gamelist.xml {ROMS}/{s}/gamelist.xml\n"
            f"grep -c '<game>' {ROMS}/{s}/gamelist.xml\n")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--media", default=SSD, help="the SSD's downloaded_media")
    ap.add_argument("--only", help="one Flip system")
    ap.add_argument("--refresh", action="store_true",
                    help="also resend art whose SSD source is newer than the device's")
    ap.add_argument("--check-size", action="store_true",
                    help="read every device image's header and resend anything "
                         "that is not 640x480. Slow, and how the PC Engine box "
                         "scans were found")
    ap.add_argument("--force", action="store_true",
                    help="resend every game the SSD has art for, whatever is there")
    ap.add_argument("--no-gamelists", action="store_true",
                    help="send files, leave the <image> tags alone")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    root = pathlib.Path(args.media)
    if not root.is_dir():
        sys.exit(f"{root} is not there -- is the SSD plugged in?")
    if not shutil.which("expect"):
        sys.exit("expect is missing; it ships with macOS")
    systems = [args.only] if args.only else sorted(SYSTEMS)
    for s in systems:
        if s not in SYSTEMS:
            sys.exit(f"unknown system {s}; known: {', '.join(sorted(SYSTEMS))}")

    print(f"reading {FLIP}")
    roms, imgs, lists, dims = survey(systems, sizes=args.check_size)
    idx = index_ssd(root, systems)
    jobs, tags, entries, nosrc, wrong = plan(
        roms, imgs, lists, idx, dims, args.refresh, args.force)
    if wrong:
        seen = {}
        for s, _, size in wrong:
            seen.setdefault(s, {}).setdefault(size, 0)
            seen[s][size] += 1
        for s, sizes in sorted(seen.items()):
            shape = ", ".join(f"{w}x{h}: {n}" for (w, h), n in sorted(sizes.items()))
            print(f"  {s}: {sum(sizes.values())} images are not "
                  f"{SCREEN[0]}x{SCREEN[1]} ({shape})")

    print(f"\n{'system':11}{'roms':>6}{'onflip':>8}{'send':>6}{'tag':>6}{'entry':>7}{'nosrc':>7}")
    for s in systems:
        c = lambda rows: sum(1 for r in rows if r[0] == s)  # noqa: E731
        print(f"{s:11}{len(roms[s]):6}{len(imgs[s]):8}{c(jobs):6}"
              f"{c(tags):6}{c(entries):7}{c(nosrc):7}")
    print(f"{'total':11}{sum(len(v) for v in roms.values()):6}"
          f"{sum(len(v) for v in imgs.values()):8}{len(jobs):6}"
          f"{len(tags):6}{len(entries):7}{len(nosrc):7}")

    if not jobs and not tags and not entries:
        print("\nnothing to do")
        return
    if not args.apply:
        print(f"\n{resize(jobs, False)} to resize. Dry run; --apply sends it.")
        if not args.no_gamelists:
            print("gamelists:")
            write_gamelists(lists, tags, entries, False)
        return

    print(f"\nresizing to 640x480 into {CACHE}")
    print(f"  {resize(jobs, True)} written")
    print("sending")
    push(jobs)
    if args.no_gamelists:
        print("gamelists left alone (--no-gamelists)")
    else:
        print("gamelists")
        write_gamelists(lists, tags, entries, True)
    ssh(f"rm -rf {STAGE}", check=False)
    print("done")


if __name__ == "__main__":
    main()
