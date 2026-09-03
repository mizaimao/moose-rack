#!/usr/bin/env python3
"""Upload the recommended games that are on the drive but not in the library.

Reads `drive-manifest/to-copy.json` — the output of comparing `voted.json`
against both inventories — and pushes each file to romM through the same
chunked endpoints `upload_rom.py` uses.

Dry by default. `--apply` is the only thing that writes to the server.

Already-present titles are skipped by name rather than assumed absent: the plan
is built from a cache that may be older than the server, and uploading a second
copy of a game is worse than skipping one.
"""

import argparse
import json
import pathlib
import re
import sys
import urllib.error

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from upload_rom import Api, CHUNK  # noqa: E402


def token_from_config(path="config.toml"):
    for line in pathlib.Path(path).read_text().splitlines():
        m = re.match(r'\s*token\s*=\s*"(.*)"', line)
        if m:
            return m.group(1)
    sys.exit("no token in config.toml")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--plan", default="drive-manifest/to-copy.json")
    ap.add_argument("--drive", default="/Volumes/Super Game HDD")
    ap.add_argument("--url", default="http://dev.lan")
    ap.add_argument("--platform", help="only this platform")
    ap.add_argument("--limit", type=int, help="stop after this many uploads")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    plan = json.load(open(args.plan))
    if args.platform:
        plan = [x for x in plan if x["platform"] == args.platform]

    api = Api(args.url, token=token_from_config())
    plats = {p["fs_slug"]: p for p in api.platforms()}

    done = failed = skipped = 0
    for item in plan:
        if args.limit and done >= args.limit:
            break
        slug = item["platform"]
        target = plats.get(slug)
        if not target:
            print(f"  ! no platform {slug} on the server — skipped"); skipped += 1; continue
        src = pathlib.Path(args.drive) / "roms" / item["drive_platform"] / item["file"]
        if not src.is_file():
            print(f"  ! missing on drive: {src}"); skipped += 1; continue

        size = src.stat().st_size
        chunks = max(1, -(-size // CHUNK))
        print(f"  [{slug}] {item['title']}  ({size/1e6:.1f} MB, {chunks} chunk(s))")
        if not args.apply:
            continue

        # The server is the authority on what it already has, and the plan is
        # built from a local cache that can be older than it — several of these
        # files turned out to be on the server already, absent only from the
        # cache. romM answers that with a 400 naming the file; that is a skip,
        # not a failure, and neither is a reason to abandon the other hundred.
        try:
            started = api.start(target["id"], src.name, size, chunks)
        except urllib.error.HTTPError as e:
            body = e.read()[:200].decode("utf-8", "replace")
            if e.code == 400 and "already exists" in body:
                print("      already on the server — skipped"); skipped += 1
            else:
                print(f"      FAILED {e.code}: {body}"); failed += 1
            continue
        except Exception as e:
            print(f"      FAILED: {e}"); failed += 1; continue
        upload_id = started.get("upload_id") or started.get("id")
        if not upload_id:
            print(f"      FAILED: no upload id ({started})"); failed += 1; continue
        try:
            with src.open("rb") as fh:
                for i in range(chunks):
                    api.chunk(upload_id, i, fh.read(CHUNK))
            api.complete(upload_id)
            print("      ok")
            done += 1
        except urllib.error.HTTPError as e:
            api.cancel(upload_id); print(f"      FAILED {e.code}: {e.read()[:160]}"); failed += 1
        except Exception as e:
            api.cancel(upload_id); print(f"      FAILED: {e}"); failed += 1

    total = sum(x["size"] or 0 for x in plan)
    print(f"\n{len(plan)} planned, {total/1e9:.1f} GB")
    if args.apply:
        print(f"uploaded {done}, failed {failed}, skipped {skipped}")
    else:
        print("(dry run — pass --apply)")


if __name__ == "__main__":
    main()
