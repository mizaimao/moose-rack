#!/usr/bin/env python3
"""Find where each ES-DE theme keeps its per-system artwork.

Every theme names its files for the ES-DE system — `snes.svg`, `3do.png` — and
puts them somewhere different. `art/`, `<system>/images/` and `system/systemart/`
are what the four themes `fetch_icons` already handles use; none of the nine in
the Icon sets tab uses any of them. Two examples, both real:

    CodyWheel   assets/systemimages/snes.png
    Iconic      _inc/systems/system/snes.webp

So the directory has to be looked up rather than guessed, and this reads it from
the repository tree — one API call per theme, no clone. The output is
`data/icon-set-art.toml`, which is what lets the tab fetch individual pictures
over raw HTTP and skip a hundreds-of-megabyte checkout.

A directory counts as system artwork when it holds at least `--min` images, on
the reasoning that ES-DE knows around 200 systems and a theme that draws them
draws most of them. Anything smaller is a handful of decorations.

    python3 tools/survey_icon_sets.py
    python3 tools/survey_icon_sets.py --json      # the raw picture, to re-map by hand
"""

import argparse
import collections
import json
import re
import sys
import subprocess
import urllib.error
import urllib.request

THEMES_LIST = "https://gitlab.com/es-de/themes/themes-list/-/raw/master/themes.json"

# The nine asked for by name, kept first in the output so the tab can lead with
# them. Everything else in the official list follows.
FIRST = [
    "codywheel", "diamond", "elegance", "elementerial", "iconic",
    "immersive", "meringue", "razor", "retromega",
]

IMAGE = re.compile(r"\.(svg|png|webp|jpg)$", re.I)

# Directory name -> our IconStyle key. Checked in order, first hit wins, so the
# more specific patterns come first: "carousel-icons" is a controller icon and
# would otherwise be caught by nothing, while "logos_white" must not be read as
# hardware art just because it sits beside some.
# Three styles, matching IconStyle. `consolegame` and `systemart_legacy` were
# dropped: one theme in fifty-four ships legacy art, and an empty style in the
# rotation is a grid of nothing.
STYLE_HINTS = [
    ("controller", ("controller", "carousel-icon")),
    ("logo", ("logo",)),
    ("systemart", ("systemimage", "artwork", "/systems", "systemart")),
]


def api(url):
    """GitHub API, through `gh` when it is signed in.

    Unauthenticated GitHub allows 60 requests an hour and surveying every theme
    takes over a hundred, so a plain run dies two thirds of the way through.
    `gh api` carries the user's token and lifts that to 5,000.
    """
    if HAVE_GH:
        path = url.replace("https://api.github.com/", "")
        out = subprocess.run(["gh", "api", path], capture_output=True, text=True)
        if out.returncode != 0:
            raise RuntimeError(out.stderr.strip().splitlines()[-1] if out.stderr else "gh failed")
        return json.loads(out.stdout)
    req = urllib.request.Request(url, headers={"User-Agent": "moose-rack-survey"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        if e.code == 403:
            sys.exit("GitHub rate limit reached. Sign in with `gh auth login` and retry.")
        raise


def have_gh():
    try:
        return subprocess.run(["gh", "auth", "status"], capture_output=True).returncode == 0
    except FileNotFoundError:
        return False


HAVE_GH = have_gh()


def themes_from_list():
    """`(reponame, owner/repo)` for every GitHub theme in the official list.

    The one GitLab theme is skipped rather than special-cased: a second host
    means a second tree API and a second raw-URL shape, for one theme out of
    sixty-five. It simply carries no preview, which the tab already handles —
    a set with no recorded art says so.
    """
    with urllib.request.urlopen(THEMES_LIST, timeout=30) as r:
        listing = json.load(r)
    out = {}
    for t in listing["themes"]:
        url = t["url"].removesuffix(".git")
        if "github.com/" not in url:
            print(f"  - skipping {t['name']}: not on GitHub ({url})", file=sys.stderr)
            continue
        out[t.get("reponame") or url.rsplit("/", 1)[-1]] = url.split("github.com/", 1)[1]
    return out


def classify(directory):
    """Which of our styles a directory holds, or None."""
    d = directory.lower()
    # Backgrounds and overlays are scenery, not a picture of a console.
    if "background" in d or "overlay" in d:
        return None
    # A theme that ships both modern and classic hardware art: take the modern
    # set, and let the classic one go rather than keeping a style for it.
    if "classic" in d:
        return None
    for style, needles in STYLE_HINTS:
        if any(n in d for n in needles):
            return style
    return None


def survey(name, repo, minimum):
    branch = api(f"https://api.github.com/repos/{repo}")["default_branch"]
    tree = api(f"https://api.github.com/repos/{repo}/git/trees/{branch}?recursive=1")
    if tree.get("truncated"):
        print(f"  ! {name}: tree truncated, results may be partial", file=sys.stderr)

    dirs = collections.defaultdict(list)
    for entry in tree["tree"]:
        if entry["type"] == "blob" and IMAGE.search(entry["path"]):
            d, _, filename = entry["path"].rpartition("/")
            dirs[d].append(filename)

    styles = {}
    for d, files in sorted(dirs.items(), key=lambda kv: -len(kv[1])):
        if len(files) < minimum:
            continue
        style = classify(d)
        # First match wins: directories are walked largest first, so the most
        # complete set of a given kind is the one kept.
        if style and style not in styles:
            ext = collections.Counter(f.rsplit(".", 1)[1].lower() for f in files).most_common(1)[0][0]
            styles[style] = {"dir": d, "ext": ext, "n": len(files)}
    return {"repo": repo, "branch": branch, "styles": styles}


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--min", type=int, default=40,
                    help="images a directory needs before it counts as system art (default 40)")
    ap.add_argument("--json", action="store_true", help="dump the raw findings instead of TOML")
    args = ap.parse_args()

    sets = themes_from_list()
    print(f"{len(sets)} themes on GitHub"
          + (" (using `gh` for the rate limit)" if HAVE_GH else ""), file=sys.stderr)

    out = {}
    for i, (name, repo) in enumerate(sets.items(), 1):
        try:
            found = survey(name, repo, args.min)
        except Exception as e:
            # One unreachable repository is not a reason to lose the other
            # sixty-four; it just gets no preview.
            print(f"  ! {name}: {e}", file=sys.stderr)
            continue
        if not found["styles"]:
            print(f"  - {name}: no system artwork", file=sys.stderr)
            continue
        out[name] = found
        print(f"{i:3}/{len(sets)} {name:34} [{found['branch']}] "
              + ", ".join(f"{k}={v['n']}" for k, v in found["styles"].items()),
              file=sys.stderr)

    # The nine asked for by name lead; the rest follow alphabetically.
    def order(item):
        stem = item[0].removesuffix("-es-de").replace("-", "")
        for i, want in enumerate(FIRST):
            if stem.startswith(want):
                return (0, i, item[0])
        return (1, 0, item[0])

    out = dict(sorted(out.items(), key=order))

    if args.json:
        print(json.dumps(out, indent=1))
        return

    print("# Regenerated by tools/survey_icon_sets.py — see data/icon-set-art.toml.")
    for name, v in out.items():
        print(f"\n[{name}]")
        print(f'repo = "{v["repo"]}"')
        print(f'branch = "{v["branch"]}"')
        for style, s in v["styles"].items():
            print(f'styles.{style} = {{ dir = "{s["dir"]}", ext = "{s["ext"]}" }}')


if __name__ == "__main__":
    main()
