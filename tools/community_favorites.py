#!/usr/bin/env python3
"""Build per-console "best of" collections from published online rankings.

Replaces an earlier attempt that ranked by the server's own `average_rating`
and by a hand-written canon. Both were wrong: the stored ratings cover half the
library, drop to 1 of 70 games on WonderSwan, and rank Battletoads above Sonic
on Mega Drive; and a list written from memory is not a community opinion.

Sources are real published rankings, fetched and recorded in
`data/community/lists.json` with the URL and method for each console:

* Wikipedia's "Video games listed among the best of the <console>" articles,
  which aggregate the best-of lists of many publications.
* Time Extension guides, ranked by reader votes.
* Metacritic, ranked by Metascore.

A console only gets a collection if a source was actually found for it. Where
fewer than the target number of titles match the library, the collection is
short rather than padded — the alternative is filling it with games nobody
recommended, which is what went wrong the first time.
"""

import argparse
import base64
import json
import pathlib
import re
import sqlite3
import urllib.error
import urllib.request
import unicodedata
import uuid

PREFIX = "★ Best of "


def norm(s):
    """Match key for a game title, in any script.

    `[^a-z0-9]` used to do the stripping, which quietly deleted every
    non-Latin character: `クロノ・トリガー` normalised to the empty string and
    `スーパーマリオブラザーズ3` to `"3"`. Every Japanese title therefore
    collapsed onto the same key -- and because the library's own Japanese
    romsets collapsed there too, an empty key *matched an empty key* and the
    Super Famicom and Famicom lists scored 84-90% against a pool they had
    nothing in common with. A source can be checked, voted on, and completely
    imaginary.

    NFKC first, so full-width and half-width forms of the same character agree
    (`Ｓｕｐｅｒ` vs `Super`, `Ⅶ` vs `V`), then keep whatever is alphanumeric in
    any script. ASCII behaviour is unchanged: NFKC is the identity on it.
    """
    s = unicodedata.normalize("NFKC", s).lower()
    s = re.sub(r"\([^)]*\)|\[[^\]]*\]", " ", s)
    s = re.sub(r"\b(the|a|an)\b", " ", s)
    s = "".join(ch if ch.isalnum() else " " for ch in s)
    return " ".join(s.split())


def build_index(con):
    idx = {}
    for rid, slug, name in con.execute(
        "select id, platform_slug, COALESCE(NULLIF(name,''), fs_name) from roms"
    ):
        idx.setdefault(slug, {}).setdefault(norm(name), (rid, name))
    return idx


def match(table, title):
    """Exact normalised hit, else a unique prefix hit.

    Prefix catches subtitles the library spells out in full — `Phantasy Star IV`
    against `Phantasy Star IV - The End of the Millennium`. An ambiguous prefix
    is dropped rather than guessed at.
    """
    t = norm(title)
    if not t:
        return None
    if t in table:
        return table[t]
    hits = [v for k, v in table.items() if k.startswith(t + " ")]
    return hits[0] if len(hits) == 1 else None


class Api:
    def __init__(self, url, user, password):
        self.url = url
        self.auth = base64.b64encode(f"{user}:{password}".encode()).decode()

    def _send(self, method, path, *, json_body=None, form=None):
        data = ctype = None
        if json_body is not None:
            data, ctype = json.dumps(json_body).encode(), "application/json"
        elif form is not None:
            # Collection creation is multipart/form-data. Posting JSON to it
            # "succeeds" with an empty name, and every later create then fails
            # with "Collection with name '' already exists".
            b = uuid.uuid4().hex
            body = "".join(
                f'--{b}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'
                for k, v in form.items()
            ) + f"--{b}--\r\n"
            data, ctype = body.encode(), f"multipart/form-data; boundary={b}"
        headers = {"Authorization": f"Basic {self.auth}"}
        if ctype:
            headers["Content-Type"] = ctype
        req = urllib.request.Request(f"{self.url}{path}", method=method,
                                     data=data, headers=headers)
        with urllib.request.urlopen(req, timeout=120) as r:
            raw = r.read()
            return json.loads(raw) if raw else None

    def collections(self):
        return self._send("GET", "/api/collections")

    def create(self, name, description):
        return self._send("POST", "/api/collections",
                          form={"name": name, "description": description})

    def set_roms(self, cid, ids):
        return self._send("POST", f"/api/collections/{cid}/roms", json_body={"rom_ids": ids})

    def delete(self, cid):
        return self._send("DELETE", f"/api/collections/{cid}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://dev.lan")
    ap.add_argument("--user", required=True)
    ap.add_argument("--password", required=True)
    ap.add_argument("--size", type=int, default=50)
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    lists = json.loads(pathlib.Path("data/community/lists.json").read_text())
    con = sqlite3.connect("cache.sqlite3")
    idx = build_index(con)

    plans = []
    for slug, spec in sorted(lists.items()):
        table = idx.get(slug)
        if not table:
            print(f"  skip {slug}: no roms cached")
            continue
        picked, seen, missed = [], set(), []
        for title in spec["titles"]:
            hit = match(table, title)
            if hit and hit[0] not in seen:
                seen.add(hit[0])
                picked.append(hit)
            elif not hit:
                missed.append(title)
            if len(picked) >= args.size:
                break
        plans.append({"slug": slug, "ids": [p[0] for p in picked],
                      "listed": len(spec["titles"]), "missed": missed,
                      "source": spec["source"], "how": spec["how"]})

    print(f"\n{'console':<14}{'picked':>7}{'of listed':>11}   source")
    for p in plans:
        print(f"{p['slug']:<14}{len(p['ids']):>7}{p['listed']:>11}   {p['how'][:44]}")
    print(f"\n{len(plans)} consoles, {sum(len(p['ids']) for p in plans)} games")

    if not args.apply:
        print("\n(dry run — pass --apply)")
        return

    api = Api(args.url, args.user, args.password)
    existing = {c["name"]: c["id"] for c in api.collections()}
    covered = {PREFIX + p["slug"] for p in plans}

    # Drop collections left over from the rejected method, so nothing on the
    # server claims to be a recommendation without a source behind it.
    for name, cid in existing.items():
        if name.startswith(PREFIX) and name not in covered:
            api.delete(cid)
            print(f"  removed  {name} (no published source found)")

    for p in plans:
        name = PREFIX + p["slug"]
        desc = (f"{len(p['ids'])} of {p['listed']} games from {p['how']}. "
                f"Source: {p['source']}")
        try:
            # Recreate rather than update: POST /{id}/roms *adds* to whatever is
            # already in the collection, so reusing one leaves the previous
            # membership underneath and the count silently grows.
            if name in existing:
                api.delete(existing[name])
            cid = api.create(name, desc)["id"]
            api.set_roms(cid, p["ids"])
            print(f"  ok       {name} ({len(p['ids'])})")
        except urllib.error.HTTPError as e:
            print(f"  FAILED   {name}: {e.code} {e.read()[:120]}")


if __name__ == "__main__":
    main()
