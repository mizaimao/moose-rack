#!/usr/bin/env python3
"""Fetch published "best of" rankings in full, and stage them for the vote.

## Why this exists rather than a summariser

The guides worth reading are paginated, and a summarising fetch returns page
one. Time Extension's NES guide answers with 13 titles and its SNES guide with
10; both hold far more. That failure is silent and it is worse than a short
list, because a truncated ranking is biased toward whatever the site puts
first — and it breaks a consensus vote specifically: a game on Wikipedia's
111-title NES list but absent from a truncated 13 fails "agreed by all" for a
reason that has nothing to do with agreement.

So this pages until the titles stop changing, and reads the markup rather than
prose. On the Hookshot sites (Time Extension, Nintendo Life, Push Square) every
game carries its own system in the same attribute as its name:

    alt="Castlevania III: Dracula's Curse <span class="sys">NES</span>"

which gives the title and the platform together. That second half is the useful
part: a page that answers with the wrong console's games — which is how
Metacritic responds to a system it does not have — is caught per title rather
than per list.

Search goes through the local SearXNG at `--searx`, whose JSON API answers
directly, so finding a guide does not mean guessing its slug. Guessing cost
four 404s before this existed.

    python3 tools/fetch_lists.py --find "best saturn games"
    python3 tools/fetch_lists.py --url https://www.timeextension.com/guides/x --platform saturn
"""

import argparse
import html
import json
import pathlib
import re
import sys
import time
import urllib.parse
import urllib.request

UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 moose-rack/list-fetch"

# Title and system in one attribute, on every Hookshot-family guide.
SYS_RE = re.compile(r'alt="([^"]*?)&lt;span class=&quot;sys&quot;&gt;([^&]*)&lt;/span&gt;"')
# Fallback: plain alt text on a cover image, no system tag.
ALT_RE = re.compile(r'<img[^>]+alt="([^"]{2,120})"[^>]*>')


def get(url, tries=3):
    for i in range(tries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=45) as r:
                return r.read().decode("utf-8", "replace")
        except Exception as e:
            if i == tries - 1:
                print(f"  ! {url}: {e}", file=sys.stderr)
                return ""
            time.sleep(1.5 * (i + 1))
    return ""


def titles_on(page):
    """`[(title, system)]` in page order, deduplicated, system may be None."""
    out, seen = [], set()
    for m in SYS_RE.finditer(page):
        t = html.unescape(m.group(1)).strip()
        if t and t.lower() not in seen:
            seen.add(t.lower())
            out.append((t, m.group(2).strip()))
    if out:
        return out
    for m in ALT_RE.finditer(page):
        t = html.unescape(m.group(1)).strip()
        if t and t.lower() not in seen and not t.lower().startswith(("logo", "icon", "banner")):
            seen.add(t.lower())
            out.append((t, None))
    return out


def fetch_all(url, max_pages=12):
    """Every title across every page of a guide, in rank order.

    Stops when a page adds nothing: paginated guides answer past their last
    page with the last page again, so "no new titles" is the only reliable end.
    """
    sep = "&" if "?" in url else "?"
    all_titles, seen = [], set()
    for page in range(1, max_pages + 1):
        body = get(url if page == 1 else f"{url}{sep}page={page}")
        if not body:
            break
        added = 0
        for t, s in titles_on(body):
            if t.lower() in seen:
                continue
            seen.add(t.lower())
            all_titles.append((t, s))
            added += 1
        print(f"    page {page}: +{added} (total {len(all_titles)})")
        if added == 0:
            break
        time.sleep(0.6)

    # The untagged fallback is per page, and on these sites a later page can
    # lose the game markup while keeping the comment section — at which point
    # the fallback happily harvests reader avatars. A Push Square PSP list came
    # back with "Stephen Tailby", "sanderson72" and "Gamer83" in it, which
    # matched nothing and dragged the whole source below the accept threshold.
    # If any page of a guide tags its games with a system, that is the format
    # of the guide, and anything untagged is not a game.
    if any(s for _, s in all_titles):
        kept = [(t, s) for t, s in all_titles if s]
        if len(kept) != len(all_titles):
            print(f"    dropped {len(all_titles) - len(kept)} untagged entries "
                  f"(comment authors and page furniture)")
        return kept
    return all_titles


def search(searx, query, limit=10):
    q = urllib.parse.urlencode({"q": query, "format": "json"})
    body = get(f"{searx.rstrip('/')}/search?{q}")
    if not body:
        return []
    try:
        return [(r.get("title", ""), r.get("url", "")) for r in json.loads(body).get("results", [])][:limit]
    except json.JSONDecodeError:
        print("  ! searx did not return JSON — is format=json enabled?", file=sys.stderr)
        return []


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--searx", default="http://ml60.lan:8080/")
    ap.add_argument("--find", help="search for guides and print the results")
    ap.add_argument("--url", help="fetch this guide in full")
    ap.add_argument("--platform", help="platform slug to stage the result under")
    ap.add_argument("--how", default="", help="one line describing how the source ranked it")
    ap.add_argument("--expect-sys", help="only keep titles whose system tag matches this")
    ap.add_argument("--raw", default="data/community/raw")
    args = ap.parse_args()

    if args.find:
        for title, url in search(args.searx, args.find):
            print(f"{url}\n    {title}")
        return

    if not (args.url and args.platform):
        sys.exit("need --find, or both --url and --platform")

    print(f"fetching {args.url}")
    got = fetch_all(args.url)
    if not got:
        sys.exit("nothing extracted — the markup is not one this knows")

    systems = {s for _, s in got if s}
    if args.expect_sys:
        want = {w.strip().lower() for w in args.expect_sys.split(",")}
        keep = [(t, s) for t, s in got if s and s.lower() in want]
        dropped = len(got) - len(keep)
        if dropped:
            print(f"  dropped {dropped} titles tagged for another system: "
                  f"{sorted(systems - {s for _, s in keep})}")
        # These sites tag by abbreviation — "SMS", not "Master System" — so a
        # filter written the long way matches nothing and quietly stages an
        # empty list, which `build_lists` then skips as if the source did not
        # exist. Refuse rather than write it, and say what the page actually
        # called itself so the next attempt can be right.
        if not keep:
            sys.exit(f"--expect-sys {args.expect_sys!r} matched none of "
                     f"{sorted(systems)} — nothing written")
        got = keep
    elif systems:
        print(f"  systems tagged on the page: {sorted(systems)}")

    if not got:
        sys.exit("no titles survived — nothing written")

    host = urllib.parse.urlparse(args.url).netloc.replace("www.", "").split(".")[0]
    out = pathlib.Path(args.raw) / f"{args.platform}__{host}.txt"
    out.write_text(
        f"# source: {args.url}\n# how: {args.how or 'published ranking'}\n"
        + "\n".join(t for t, _ in got) + "\n"
    )
    print(f"-> {out} ({len(got)} titles)")


if __name__ == "__main__":
    main()
