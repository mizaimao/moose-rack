#!/usr/bin/env python3
"""Copy ES-DE artwork off the Ayn Thor onto the SSD.

The Thor scrapes; the SSD is canonical. Anything the phone has and the SSD does
not should end up on the SSD, and nothing already there should be touched.

Both sides are ES-DE and name systems the same way, so unlike
``scripts/import-esde-media.py`` there is no slug translation to get wrong --
only a path diff:

    Android   <root>/downloaded_media/<system>/<type>/<base>.<ext>
    SSD       /Volumes/Retro/ES-DE/support/downloaded_media/<system>/<type>/<base>.<ext>

    tools/pull-android-media.py --dry-run     what would come across
    tools/pull-android-media.py               copy it
    tools/pull-android-media.py --no-videos   artwork only, no videos

Missing-only by default. A file already on the SSD is never overwritten, because
the SSD is the copy that has been audited and the phone's is not.

Transfers go through one `tar` stream per batch rather than one `adb pull` per
file: several thousand small PNGs pulled individually spend all their time in
round trips.
"""

import argparse
import os
import subprocess
import time
import sys
import unicodedata
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
ADB = HERE / ".toolchain/android/sdk/platform-tools/adb"
ADB_HOME = HERE / ".toolchain/android/home"
SSD_MEDIA = Path("/Volumes/Retro/ES-DE/support/downloaded_media")

# Where ES-DE keeps media on Android. The card UUID varies, so these are tried
# in order and the first that exists wins.
CANDIDATES = [
    "/storage/A2FC-A9FB/ES-DE/downloaded_media",
    "/sdcard/ES-DE/downloaded_media",
    "/storage/emulated/0/ES-DE/downloaded_media",
]

VIDEO_DIRS = {"videos"}
BATCH = 400


def adb(*args, **kw):
    env = dict(os.environ, HOME=str(ADB_HOME))
    return subprocess.run([str(ADB), *args], capture_output=True, text=True, env=env, **kw)


def adb_raw(*args):
    env = dict(os.environ, HOME=str(ADB_HOME))
    return subprocess.Popen([str(ADB), *args], stdout=subprocess.PIPE, env=env)


def nfc(s):
    return unicodedata.normalize("NFC", s)


def device_ready():
    out = adb("devices").stdout
    for line in out.splitlines()[1:]:
        if not line.strip():
            continue
        serial, _, state = line.partition("\t")
        state = state.strip()
        if state == "device":
            return True, serial
        if state == "unauthorized":
            return False, (
                f"{serial} is unauthorized — unlock the Thor and accept the "
                f"'Allow USB debugging' prompt, then run this again"
            )
        return False, f"{serial} is in state {state!r}"
    return False, "no device attached"


def find_root(explicit):
    if explicit:
        return explicit
    for c in CANDIDATES:
        if adb("shell", "test", "-d", c).returncode == 0:
            return c
    # Last resort: ask the device where it is.
    out = adb("shell", "find", "/storage", "-maxdepth", "3", "-name", "downloaded_media", "-type", "d").stdout
    for line in out.splitlines():
        if line.strip():
            return line.strip()
    return None


def remote_listing(root):
    """Every media file on the phone, relative to the media root."""
    out = adb("shell", "find", root, "-type", "f").stdout
    rel = []
    for line in out.splitlines():
        line = line.strip()
        if not line or not line.startswith(root):
            continue
        r = line[len(root):].lstrip("/")
        if r:
            rel.append(nfc(r))
    return rel


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument(
        "--no-videos",
        action="store_true",
        help="skip videos. They are ~40%% of the bytes and the default is to take them: "
        "artwork without them is not the media set, it is most of it.",
    )
    ap.add_argument("--root", help="Android media root, if auto-detection fails")
    ap.add_argument("--dest", default=str(SSD_MEDIA))
    ap.add_argument(
        "--wait",
        type=int,
        metavar="MINUTES",
        help="poll until the device is authorized, then run. Arm this and walk away: "
        "the prompt needs a hand on the device, but nothing after it does.",
    )
    args = ap.parse_args()

    ok, info = device_ready()
    if not ok and args.wait:
        # The whole reason this exists: reporting "unauthorized" and stopping
        # wastes however long it takes somebody to notice. Waiting costs a poll
        # every ten seconds.
        deadline = time.time() + args.wait * 60
        last = None
        while time.time() < deadline:
            ok, info = device_ready()
            if ok:
                break
            if info != last:
                print(f"  waiting: {info}", flush=True)
                last = info
            time.sleep(10)
    if not ok:
        print(f"  {info}")
        return 1
    print(f"  device {info}")

    root = find_root(args.root)
    if not root:
        print("  could not find downloaded_media on the device; pass --root")
        return 1
    print(f"  android  {root}")

    dest = Path(args.dest)
    if not dest.is_dir():
        print(f"  destination missing: {dest} — is the SSD mounted?")
        return 1
    print(f"  ssd      {dest}")

    remote = remote_listing(root)
    if not remote:
        print("  nothing found on the device")
        return 1

    skipped_video = 0
    missing = []
    for r in remote:
        parts = r.split("/")
        if args.no_videos and len(parts) > 1 and parts[1] in VIDEO_DIRS:
            skipped_video += 1
            continue
        if not (dest / r).exists():
            missing.append(r)

    have = len(remote) - len(missing) - skipped_video
    print(f"\n  on device   {len(remote)}")
    print(f"  already ssd {have}")
    if skipped_video:
        print(f"  videos      {skipped_video} skipped (--no-videos was passed)")
    print(f"  to copy     {len(missing)}")

    if not missing:
        print("\n  nothing to do")
        return 0

    by_system = {}
    for m in missing:
        by_system.setdefault(m.split("/")[0], []).append(m)
    print()
    for s in sorted(by_system, key=lambda k: -len(by_system[k])):
        print(f"    {s:<24} {len(by_system[s]):>6}")

    if args.dry_run:
        print("\n  dry run, nothing copied")
        return 0

    copied = failed = 0
    for i in range(0, len(missing), BATCH):
        batch = missing[i : i + BATCH]
        # tar on the device, untar here: one round trip per batch instead of one
        # per file. -k so an existing file is never overwritten even on a rerun.
        proc = adb_raw("exec-out", "tar", "-cf", "-", "-C", root, *batch)
        untar = subprocess.Popen(
            ["tar", "-xf", "-", "-k", "-C", str(dest)],
            stdin=proc.stdout,
            stderr=subprocess.DEVNULL,
        )
        proc.stdout.close()
        untar.wait()
        proc.wait()
        landed = sum(1 for b in batch if (dest / b).exists())
        copied += landed
        failed += len(batch) - landed
        print(f"  {min(i + BATCH, len(missing)):>6}/{len(missing)}  landed {copied}", flush=True)

    print(f"\n  copied {copied}, missing after copy {failed}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
