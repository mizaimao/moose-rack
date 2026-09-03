#!/usr/bin/env python3
"""Merge the voted "best of" lists into the collections that already exist.

A union, deliberately. The existing nine were built from one published source
each and capped at fifty; the voted lists come from two to five sources with a
vote. Neither is a superset of the other, and replacing one with the other
would drop 142 games somebody can currently browse to gain 315. So this adds
and never removes.

Records the exact membership before and after, so "nothing was dropped" is a
measurement rather than a promise.
"""

import argparse, collections, json, pathlib, re, sys, urllib.error, urllib.request
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from community_favorites import norm  # noqa: E402

def token(p="config.toml"):
    for line in pathlib.Path(p).read_text().splitlines():
        m = re.match(r'\s*token\s*=\s*"(.*)"', line)
        if m: return m.group(1)
    sys.exit("no token in config.toml")

class Api:
    def __init__(self, url, tok): self.url, self.tok = url.rstrip("/"), tok
    def _req(self, method, path, data=None):
        body = json.dumps(data).encode() if data is not None else None
        h = {"Authorization": f"Bearer {self.tok}"}
        if body: h["Content-Type"] = "application/json"
        r = urllib.request.Request(f"{self.url}{path}", method=method, data=body, headers=h)
        with urllib.request.urlopen(r, timeout=120) as resp:
            raw = resp.read()
            return json.loads(raw) if raw else None
    def collections(self): return self._req("GET", "/api/collections?limit=3000")
    def add(self, cid, ids): return self._req("POST", f"/api/collections/{cid}/roms", {"rom_ids": ids})

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--url", default="http://dev.lan")
    ap.add_argument("--db", default="cache.sqlite3")
    ap.add_argument("--voted", default="data/community/voted.json")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    import sqlite3
    con = sqlite3.connect(args.db)
    lib = collections.defaultdict(dict)
    names = {}
    for rid, s, n in con.execute("select id, platform_slug, COALESCE(NULLIF(name,''),fs_name) from roms"):
        names[rid] = n
        if norm(n): lib[s].setdefault(norm(n), rid)
    voted = json.load(open(args.voted))
    api = Api(args.url, token())

    before, after = {}, {}
    for c in api.collections():
        if not c["name"].startswith("★ Best of "): continue
        slug = c["name"].replace("★ Best of ", "")
        old = set(c.get("rom_ids") or [])
        before[c["name"]] = old
        want = {lib[slug][norm(t)] for t in voted.get(slug, {}).get("most", []) if norm(t) in lib[slug]}
        add = sorted(want - old)
        print(f"{c['name']:26} {len(old):4} + {len(add):3}")
        if add and args.apply:
            api.add(c["id"], add)
    if args.apply:
        for c in api.collections():
            if c["name"].startswith("★ Best of "):
                after[c["name"]] = set(c.get("rom_ids") or [])
        print("\n=== VERIFY: anything present before and absent after ===")
        lost = 0
        for name, old in before.items():
            gone = old - after.get(name, set())
            if gone:
                lost += len(gone)
                print(f"  {name}: DROPPED {len(gone)} -> {[names.get(i, i) for i in sorted(gone)][:10]}")
        print(f"  total dropped: {lost}")
        print(f"\n{'collection':26}{'before':>8}{'after':>8}")
        for name in sorted(before):
            print(f"{name:26}{len(before[name]):8}{len(after.get(name,())):8}")
        print(f"{'TOTAL':26}{sum(len(v) for v in before.values()):8}{sum(len(v) for v in after.values()):8}")
    else:
        print("\n(dry run — pass --apply)")

if __name__ == "__main__":
    main()
