#!/usr/bin/env python3
"""Assemble per-platform "best of" sources into one voted list, and check them.

Each source is a file in `data/community/raw/<platform>__<source>.txt`: two
comment lines giving the URL and the method, then one title per line in the
order the source ranked them. One file per source per platform, so adding a
source is adding a file and nothing else has to change.

## Why every list is checked before it counts

Fetching "the best PSP games" from a site that has no PSP page does not fail.
Metacritic answers `/browse/game/ds/...` and `/browse/game/playstation/...`
with its *all-platform* all-time chart, so a list captured from either comes
back full of real games — Ocarina of Time, Breath of the Wild, Red Dead
Redemption 2 — none of which are DS games. Nothing about that output looks
wrong. It would have put Red Dead Redemption 2 in a Nintendo DS collection.

So a list is only trusted if its titles actually exist for that platform. The
drive manifest and the library together know 45,000 games and which system each
one is for, which is enough to say whether a list is about the console it
claims. Below `--min-match` the list is reported and dropped rather than voted
on: a source that cannot be verified is worse than a missing source, because it
looks like data.

## The vote

Every title carries the number of sources that named it. Two collections come
out per platform, which is what a tiered vote is for:

* `agreed`  — named by every source for that platform. Short, and beyond argument.
* `most`    — named by more than half. Longer, and where the actual browsing happens.

A platform with one source has no vote to take: everything it names lands in
`most`, and `agreed` is left empty rather than being filled with a single
opinion wearing a consensus label.

    python3 tools/build_lists.py
    python3 tools/build_lists.py --min-match 0.5 --report
"""

import argparse
import collections
import json
import pathlib
import sqlite3
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from community_favorites import norm  # noqa: E402
from drive_vs_library import PLATFORM_MAP, open_manifest  # noqa: E402


def read_raw(path):
    """`(source, how, [titles])` from one staged file."""
    source = how = ""
    titles = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line.startswith("# source:"):
            source = line.split(":", 1)[1].strip()
        elif line.startswith("# how:"):
            how = line.split(":", 1)[1].strip()
        elif line and not line.startswith("#"):
            titles.append(line)
    return source, how, titles


def known_titles(manifest, db):
    """Every title we can prove belongs to a platform, from both inventories.

    The union is deliberate. The drive knows systems the library has never
    heard of, and the library holds games the drive does not carry; a list is
    about the right console if *either* recognises what is on it.
    """
    known = collections.defaultdict(set)
    if pathlib.Path(manifest).exists() or pathlib.Path(str(manifest) + ".gz").exists():
        with open_manifest(manifest) as fh:
            for line in fh:
                r = json.loads(line)
                slug = PLATFORM_MAP.get(r["platform"], r["platform"])
                known[slug].add(norm(r["name"]))
    if pathlib.Path(db).exists():
        con = sqlite3.connect(db)
        for slug, name in con.execute(
            "select platform_slug, COALESCE(NULLIF(name,''), fs_name) from roms"
        ):
            known[slug].add(norm(name))
        con.close()
    # An empty key is not a title. It used to be reachable -- `norm` deleted
    # non-Latin text entirely -- and once `""` is in the pool every Japanese
    # title "matches" it, which is how a list about a console it had never
    # heard of scored 90%. Keep the guard even now the cause is gone.
    known.pop("", None)
    for pool in known.values():
        pool.discard("")
    return known


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--raw", default="data/community/raw")
    ap.add_argument("--manifest", default="drive-manifest/manifest.jsonl")
    ap.add_argument("--db", default="cache.sqlite3")
    ap.add_argument("--out", default="data/community/voted.json")
    ap.add_argument("--min-match", type=float, default=0.35,
                    help="reject a list if fewer than this fraction of its titles "
                         "exist for the platform it claims (default 0.35)")
    ap.add_argument("--report", action="store_true", help="print each list's match rate")
    args = ap.parse_args()

    known = known_titles(args.manifest, args.db)
    if not known:
        sys.exit("no manifest and no library — nothing to check lists against")

    by_platform = collections.defaultdict(list)
    for f in sorted(pathlib.Path(args.raw).glob("*.txt")):
        platform = f.stem.split("__", 1)[0]
        source, how, titles = read_raw(f)
        if not titles:
            print(f"  ! {f.name}: no titles", file=sys.stderr)
            continue
        pool = known.get(platform, set())
        hits = sum(1 for t in titles if norm(t) in pool)
        rate = hits / len(titles)
        status = "ok " if rate >= args.min_match else "DROP"
        if args.report or status == "DROP":
            print(f"  {status} {f.stem:34} {hits:4}/{len(titles):<4} "
                  f"{rate:5.0%} of its titles exist for '{platform}'")
        if status == "DROP":
            continue
        by_platform[platform].append(
            {"source": source, "how": how, "titles": titles, "match_rate": round(rate, 3)}
        )

    out = {}
    for platform, lists in sorted(by_platform.items()):
        votes = collections.Counter()
        display = {}
        for lst in lists:
            for t in dict.fromkeys(lst["titles"]):   # a source votes once per title
                k = norm(t)
                if not k:
                    continue
                votes[k] += 1
                display.setdefault(k, t)
        n = len(lists)
        # One source is an opinion, not an agreement. Say so by leaving
        # `agreed` empty rather than promoting it.
        agreed = sorted((display[k] for k, v in votes.items() if v == n)) if n > 1 else []
        most = sorted(display[k] for k, v in votes.items() if v * 2 > n)
        out[platform] = {
            "sources": [{k: v for k, v in l.items() if k != "titles"} for l in lists],
            "source_count": n,
            "agreed": agreed,
            "most": most,
            "votes": {display[k]: v for k, v in votes.most_common()},
        }

    pathlib.Path(args.out).write_text(json.dumps(out, indent=1, ensure_ascii=False) + "\n")

    print(f"\n{'platform':18}{'sources':>8}{'agreed':>8}{'most':>7}{'named':>7}")
    for p, v in sorted(out.items(), key=lambda kv: -kv[1]["source_count"]):
        print(f"{p:18}{v['source_count']:8}{len(v['agreed']):8}{len(v['most']):7}{len(v['votes']):7}")
    print(f"\n-> {args.out}")


if __name__ == "__main__":
    main()
