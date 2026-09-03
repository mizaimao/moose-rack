#!/usr/bin/env python3
"""Bring a RomM server in line with the local ES-DE library.

Today's reorganisation happened on the cards, not the server, so the two have
drifted apart in ways a rescan cannot reconcile:

* `mame` was merged into `arcade` locally. The server still has both, and 743
  of mame's roms now live under arcade — the same game filed twice.
* Neo Geo changed format. The server holds romset-style zips (`blazstar.zip`);
  the cards hold NeoSD `.neo` images named by title ("Blazing Star.zip"). Only
  one of ~160 matches by name, so uploading without deleting doubles the list.

Both need delete-then-upload: RomM's API cannot rename a rom (PUT /api/roms/{id}
takes metadata ids only), and it will not merge two platforms.

Deletions pass `delete_from_fs`, so files leave the server's disk as well as its
database. Nothing here is reversible from the server side — run without --apply
first and read the counts.
"""

import argparse
import base64
import json
import pathlib
import sys
import tomllib
import urllib.error
import urllib.request

CHUNK = 8 * 1024 * 1024


class Api:
    def __init__(self, cfg):
        self.url = cfg["url"].rstrip("/")
        # A client token if one is configured, Basic otherwise. Matches the Rust
        # client, so config.toml does not have to keep a password around purely
        # for this script.
        token = (cfg.get("token") or "").strip()
        if token:
            self.header = f"Bearer {token}"
        else:
            creds = f"{cfg.get('username', '')}:{cfg.get('password', '')}"
            self.header = "Basic " + base64.b64encode(creds.encode()).decode()

    def _req(self, method, path, *, data=None, headers=None, timeout=300):
        h = {"Authorization": self.header}
        h.update(headers or {})
        req = urllib.request.Request(self.url + path, data=data, method=method, headers=h)
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read()
            return json.loads(raw) if raw else None

    def get(self, path):
        return self._req("GET", path)

    def post_json(self, path, obj):
        return self._req("POST", path, data=json.dumps(obj).encode(),
                         headers={"Content-Type": "application/json"})

    def delete(self, path):
        return self._req("DELETE", path)

    def all_roms(self):
        """Every rom on the server. The platform_id query param is ignored by
        this RomM build, so the filtering happens here."""
        out, off = [], 0
        while True:
            d = self.get(f"/api/roms?limit=500&offset={off}")
            items = d["items"] if isinstance(d, dict) else d
            out += items
            if len(items) < 500:
                return out
            off += 500

    def upload(self, platform_id, path):
        size = path.stat().st_size
        chunks = max(1, -(-size // CHUNK))
        started = self._req("POST", "/api/roms/upload/start", headers={
            "x-upload-platform": str(platform_id),
            "x-upload-filename": path.name,
            "x-upload-total-size": str(size),
            "x-upload-total-chunks": str(chunks),
        })
        upload_id = started.get("upload_id") or started.get("id")
        if not upload_id:
            raise RuntimeError(f"no upload id in {started}")
        try:
            with path.open("rb") as fh:
                for i in range(chunks):
                    self._req("PUT", f"/api/roms/upload/{upload_id}",
                              data=fh.read(CHUNK),
                              headers={"x-chunk-index": str(i),
                                       "Content-Type": "application/octet-stream"})
            self._req("POST", f"/api/roms/upload/{upload_id}/complete")
        except Exception:
            try:
                self._req("POST", f"/api/roms/upload/{upload_id}/cancel")
            except Exception:
                pass
            raise


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--roms", default="/Volumes/Retro/Roms",
                    help="local ES-DE ROMs directory to treat as the truth")
    ap.add_argument("--config", default="config.toml")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    cfg = tomllib.load(open(args.config, "rb"))["server"]
    api = Api(cfg)
    roms = pathlib.Path(args.roms)
    if not roms.is_dir():
        sys.exit(f"no ROMs directory at {roms}")

    platforms = {p["fs_slug"]: p for p in api.get("/api/platforms")}
    server = api.all_roms()
    by_platform = {}
    for r in server:
        by_platform.setdefault(r.get("platform_fs_slug"), {})[
            r["fs_name"].rsplit(".", 1)[0]] = r["id"]

    def local(name):
        # Retro hides some systems by renaming the folder, so fall back to that
        d = roms / name
        if not d.is_dir():
            d = roms / f"{name}_hidden"
        return {f.stem: f for f in d.glob("*.zip")
                if not f.name.startswith("._")} if d.is_dir() else {}

    steps = []

    # 1. mame was merged into arcade — the platform should not exist any more
    if "mame" in platforms:
        ids = list(by_platform.get("mame", {}).values())
        steps.append(("delete mame roms", lambda ids=ids: api.post_json(
            "/api/roms/delete", {"roms": ids, "delete_from_fs": ids}), len(ids)))
        pid = platforms["mame"]["id"]
        steps.append(("delete mame platform",
                      lambda pid=pid: api.delete(f"/api/platforms/{pid}"), 1))

    # 2. Neo Geo switched to the .neo format; the old romsets must go first or
    #    the same games appear twice under different names
    neo_local = local("neogeo")
    neo_server = by_platform.get("neogeoaes", {})
    stale = [i for n, i in neo_server.items() if n not in neo_local]
    if stale:
        steps.append(("delete old neogeo romsets", lambda ids=stale: api.post_json(
            "/api/roms/delete", {"roms": ids, "delete_from_fs": ids}), len(stale)))

    # 3. uploads — anything local the server lacks, once the deletions land
    uploads = []
    for lname, slug in (("arcade", "arcade"), ("neogeo", "neogeoaes"),
                        ("pcengine", "pcengine")):
        if slug not in platforms:
            print(f"  skip {slug}: no such platform on the server")
            continue
        have = set(by_platform.get(slug, {}))
        if slug == "neogeoaes":
            have -= {n for n in have if n not in neo_local}   # about to be deleted
        # `mame` needs no special case: its games are absent from server arcade,
        # so they fall out of the comparison below as uploads on their own.
        for stem, path in sorted(local(lname).items()):
            if stem not in have:
                uploads.append((platforms[slug]["id"], slug, path))

    print(f"  server: {len(server)} roms across {len(platforms)} platforms")
    for label, _, n in steps:
        print(f"    {label:<28}{n:>6}")
    print(f"    upload{'':<22}{len(uploads):>6}"
          f"   ({sum(p.stat().st_size for _, _, p in uploads)/1e9:.2f} GB)")

    if not args.apply:
        print("\n  (dry run — pass --apply to make these changes)")
        return

    for label, fn, n in steps:
        print(f"  {label} ({n})...", flush=True)
        print(f"    -> {fn()}")

    done = fail = 0
    for pid, slug, path in uploads:
        try:
            api.upload(pid, path)
            done += 1
        except Exception as e:
            fail += 1
            print(f"    FAILED {slug}/{path.name}: {str(e)[:90]}")
        if (done + fail) % 25 == 0:
            print(f"    {done + fail}/{len(uploads)} uploaded", flush=True)
    print(f"\n  uploaded {done}, failed {fail}")
    print("  RomM needs a manual Scan in its web UI to index the new files "
          "(the REST scan endpoint is disabled on this instance)")


if __name__ == "__main__":
    main()
