#!/usr/bin/env python3
"""Copy the games a want-list marks as present on an attached drive.

No shell anywhere. Game filenames are hostile to shell quoting in ways that are
easy to miss — one of these is `WarioWare, Inc. - Mega Party Game$! (USA).rvz`,
and a generated script quoting that in double quotes dies on `$!` under `set -u`
partway through a copy, leaving it half done. Passing argument lists to rsync
sidesteps the whole class: nothing is ever parsed as syntax.

Skips what is already on the server, so it is safe to re-run after an
interruption.

    scripts/copy-wanted.py [--dry-run]
"""

import os
import subprocess
import sys

HOST = "dev.lan"
DEST = "/home/frank/romm/assets/roms"
SSH = ["/usr/bin/ssh", "-o", "BatchMode=yes"]


def remote_listing(platforms):
    """One round trip for every destination, rather than one per file.

    122 files meant 122 SSH handshakes just to ask whether each was already
    there, which cost more than the copying.
    """
    script = "\n".join(f'ls -1 "{DEST}/{p}" 2>/dev/null | sed "s|^|{p}/|"' for p in platforms)
    out = subprocess.run(SSH + [HOST, "bash -s"], input=script, capture_output=True, text=True)
    return set(out.stdout.split("\n"))


def main():
    dry = "--dry-run" in sys.argv
    plan = [l.split("\t") for l in open("/tmp/copyplan.tsv").read().splitlines() if l]
    platforms = sorted({p for p, _, _ in plan})
    have = remote_listing(platforms)

    todo = [(p, s, b) for p, s, b in plan if f"{p}/{b}" not in have]
    total = sum(os.path.getsize(s) for _, s, _ in todo if os.path.exists(s))
    print(f"{len(plan)} in the list, {len(plan) - len(todo)} already on the server")
    print(f"{len(todo)} to copy, {total / 1e9:.1f} GB")
    if dry or not todo:
        return

    done = failed = 0
    sent = 0
    for platform, src, base in todo:
        r = subprocess.run(
            ["rsync", "-a", "-e", " ".join(SSH), src, f"{HOST}:{DEST}/{platform}/"],
            capture_output=True, text=True,
        )
        if r.returncode == 0:
            done += 1
            sent += os.path.getsize(src) if os.path.exists(src) else 0
        else:
            failed += 1
            print(f"  failed: {platform}/{base}: {r.stderr.strip()[:90]}")
        if done and done % 20 == 0:
            print(f"  {done}/{len(todo)}  {sent / 1e9:.1f} GB", flush=True)
    print(f"copied {done}, failed {failed}")


if __name__ == "__main__":
    main()
