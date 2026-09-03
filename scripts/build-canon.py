#!/usr/bin/env python3
"""Build a per-platform want-list by consensus across published rankings.

One list is one outlet's opinion. Two agreeing is a signal. The first attempt at
this ranked by the `<rating>` values in a drive's own gamelist.xml — scraper
scores of unknown provenance, a number attached to a file rather than a
judgement anyone stands behind — and produced something that looked
authoritative and was not.

So: several rankings per system, and a game's score is how many of them list it.
Anything appearing on one list alone is kept but marked, because a single
mention is exactly what consensus is supposed to filter.

Sources are the ones that actually serve their content to a plain HTTP client.
Several well-known outlets do not, and a source that cannot be read is not a
source. Each has its own markup, so each gets its own extractor rather than one
clever regex that silently half-works.
"""

import html
import json
import re
import sys
import time
import urllib.error
import urllib.request

UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 moose-rack"


def get(url, tries=2):
    for attempt in range(tries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=30) as r:
                return r.read().decode("utf-8", "replace")
        except Exception:
            if attempt + 1 == tries:
                return ""
            time.sleep(2)
    return ""


def strip_tags(s):
    return html.unescape(re.sub(r"<[^>]+>", "", s)).strip()


def time_extension(slug, tag, pages=8):
    """Reader-ranked guides, ten entries a page, wrapping back to page one.

    Two markups in the wild and no way to tell from the URL which you get: some
    guides put entries in <h2> with a rank prefix ("50. James Pond 2 (MD)"),
    others in <h3> with none. Reading only one of them returns zero games and
    looks exactly like a system nobody has ranked, which is how eleven
    platforms silently came back empty.

    The wrap is the stop condition: requesting page 99 serves page one rather
    than an error, so a fixed page count would duplicate the top ten.
    """
    seen, out, blank = set(), [], 0
    for p in range(1, pages + 1):
        t = get(f"https://www.timeextension.com/guides/{slug}?page={p}")
        if not t:
            break
        names = []
        for x in re.findall(r"<h[23][^>]*>(.*?)</h[23]>", t, re.S):
            x = strip_tags(x)
            x = re.sub(r"^\d+\.\s*", "", x)          # rank prefix, where there is one
            m = re.match(r"^(.{2,70}?)\s*\(([^)]{1,22})\)$", x)
            if m and tag.strip("() ").lower() in m.group(2).lower():
                names.append(m.group(1).strip())
        if not names:
            # Page one is usually an FAQ with no entries on it. Bailing on the
            # first empty page is what made eleven platforms report zero.
            blank += 1
            if blank >= 3:
                break
            continue
        blank = 0
        if names[0] in seen:          # wrapped back to the start
            break
        for n in names:
            if n not in seen:
                seen.add(n)
                out.append(n)
    return out


def headings(url, pattern=r"<h2[^>]*>(.*?)</h2>", drop=()):
    """Generic: numbered headings, which is how most ranked articles are built."""
    t = get(url)
    out = []
    for raw in re.findall(pattern, t, re.S):
        s = strip_tags(raw)
        s = re.sub(r"^\s*\d+[\.\):]?\s*", "", s)  # leading rank number
        if 2 < len(s) < 70 and not any(d.lower() in s.lower() for d in drop):
            out.append(s)
    return out


# Platform -> RomM slug, and the sources that serve it.
# RomM slug -> (Time Extension guide slug, the tag its headings carry).
# Slugs come from the site's own sitemap, not from guessing: eleven of fourteen
# guesses 404'd, and a 404 here is a platform silently reporting zero games.
PLATFORMS = {
    "snes":         ("best-snes-games-of-all-time-super-nintendo-games-you-must-own", "(SNES)"),
    "sfc":          ("best-snes-games-of-all-time-super-nintendo-games-you-must-own", "(SNES)"),
    "nes":          ("best-nes-games-of-all-time", "(NES)"),
    "famicom":      ("best-nes-games-of-all-time", "(NES)"),
    "megadrive":    ("best-sega-genesis-mega-drive-games-of-all-time", "(MD"),
    "n64":          ("best-n64-games-of-all-time", "(N64)"),
    "gb":           ("best-game-boy-games-of-all-time", "(GB)"),
    "gbc":          ("best-game-boy-color-games-of-all-time", "(GBC)"),
    "gba":          ("best-gba-games-of-all-time", "(GBA)"),
    "psx":          ("best-ps1-games-of-all-time-playstation-titles-you-shouldnt-miss", "(PS1)"),
    "dc":           ("best-sega-dreamcast-games-of-all-time", "(Dreamcast)"),
    "saturn":       ("best-sega-saturn-games-of-all-time", "(Saturn)"),
    "mastersystem": ("best-sega-master-system-games-of-all-time", "(SMS)"),
    "gamegear":     ("best-sega-game-gear-games-of-all-time", "(GG)"),
    "pcengine":     ("best-pc-engine-turbografx-16-games", "(TG-16)"),
    "neogeoaes":    ("best-neo-geo-games-of-all-time", "(Neo Geo"),
    "neo-geo-pocket": ("best-neo-geo-pocket-color-games", "(NGPC)"),
    "wonderswan":   ("best-wonderswan-games-of-all-time", "(WS)"),
    "wonderswancolor": ("best-wonderswan-games-of-all-time", "(WS)"),
    "ngc":          ("best-gamecube-games-of-all-time", "(GCN)"),
    "nds":          ("best-nintendo-ds-games-of-all-time", "(DS)"),
    "3do":          ("best-3do-games-of-all-time", "(3DO)"),
}

# Extra rankings for the same system, so a game's score is how many independent
# lists carry it rather than one outlet's ordering.
EXTRA = {
    "snes":      ["best-snes-rpgs-of-all-time"],
    "sfc":       ["best-snes-rpgs-of-all-time"],
    "psx":       ["best-ps1-rpgs-of-all-time"],
    "megadrive": ["best-genesis-mega-drive-rpgs-and-action-adventures-of-all-time"],
}

def main():
    only = sys.argv[1:] or list(PLATFORMS)
    out = {}
    for name in only:
        if name not in PLATFORMS:
            continue
        slug, tag = PLATFORMS[name]
        lists = {}
        got = time_extension(slug, tag)
        if got:
            lists[slug] = got
        for extra in EXTRA.get(name, []):
            more = time_extension(extra, tag)
            if more:
                lists[extra] = more
        n = len(set().union(*lists.values())) if lists else 0
        print(f"{name:16} {len(lists)} list(s), {n:>3} distinct games", flush=True)
        out[name] = lists
    json.dump(out, open("/tmp/canon-lists.json", "w"), indent=1)
    print(f"\nwrote /tmp/canon-lists.json for {len(out)} platforms")


if __name__ == "__main__":
    main()
