#!/usr/bin/env python3
"""Push the server's "Best of" lists onto an ES-DE card as collections + favorites.

Two writes per list, because ES-DE keeps the two ideas in different places:

  ES-DE/collections/custom-<name>.cfg   the membership, as %ROMPATH% lines
  ES-DE/gamelists/<system>/gamelist.xml a <favorite>true</favorite> per game

A .cfg alone is invisible until its name is in `CollectionSystemsCustom` in
es_settings.xml, so that line is rewritten too.

Games on the card that ES-DE never scraped have no <game> block to flag, so a
minimal one is appended; ES-DE fills in the rest on its next scrape.

Everything touched is copied into backups/ first. Dry by default.
"""

import argparse
import pathlib
import re
import shutil
import sqlite3
import subprocess
import sys
import time

# server collection id -> (ES-DE system dir, collection name on the card)
LISTS = {
    "32": ("nes", "Best of NES"),
    "34": ("snes", "Best of SNES"),
    "29": ("genesis", "Best of Mega Drive"),
    "27": ("gba", "Best of GBA"),
    "26": ("gb", "Best of Game Boy"),
    "33": ("psx", "Best of PlayStation"),
    "28": ("gbc", "Best of Game Boy Color"),
    "31": ("neogeo", "Best of Neo Geo"),
    "30": ("n64", "Best of N64"),
}


def title(n):
    """A game reduced to what two dumps of it share: the words, nothing else."""
    n = re.sub(r"\.[A-Za-z0-9]{2,4}$", "", n)
    n = re.sub(r"[\(\[].*?[\)\]]", " ", n)
    n = re.sub(r",\s*(the|a|an)\b", " ", n, flags=re.I)
    return re.sub(r"[^a-z0-9]+", "", n.lower())


def read(p):
    # surrogateescape so bytes that are not valid UTF-8 survive a round trip;
    # these files carry scraped text from a dozen sources.
    return p.read_text(encoding="utf-8", errors="surrogateescape")


def write(p, s):
    p.write_text(s, encoding="utf-8", errors="surrogateescape")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--card", default="/Volumes/1TB")
    ap.add_argument("--cache", default="cache.sqlite3")
    ap.add_argument("--backups", default="backups")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    card = pathlib.Path(args.card)
    esde, roms = card / "ES-DE", card / "Roms"
    if not esde.is_dir():
        sys.exit(f"no ES-DE folder at {esde}")
    db = sqlite3.connect(args.cache)

    settings = esde / "settings" / "es_settings.xml"
    plan, missing = [], []

    for cid, (system, label) in LISTS.items():
        rom_dir = roms / system
        on_card = {}
        for e in rom_dir.iterdir():
            if not e.name.startswith("."):
                on_card.setdefault(title(e.name), e.name)

        members, absent = [], []
        for fs, nm in db.execute(
            "SELECT r.fs_name, r.name FROM collection_roms cr JOIN roms r ON r.id = cr.rom_id "
            "WHERE cr.collection_id = ? ORDER BY r.fs_name", (cid,)):
            hit = on_card.get(title(nm or fs)) or on_card.get(title(fs))
            (members if hit else absent).append(hit or fs)
        missing += [(label, a) for a in absent]

        gl = esde / "gamelists" / system / "gamelist.xml"
        text = read(gl)
        blocks = re.findall(r"\t<game>.*?\t</game>", text, re.S)
        indexed = {}
        for b in blocks:
            m = re.search(r"<path>(.*?)</path>", b)
            if m:
                indexed[title(pathlib.PurePath(m.group(1)).name)] = b
        star = [f for f in members
                if "<favorite>true</favorite>" not in indexed.get(title(f), "")]
        new = [f for f in star if title(f) not in indexed]
        plan.append({"cid": cid, "system": system, "label": label, "members": members,
                     "star": star, "new": new, "gamelist": gl})

    print(f"{'collection':24} {'members':>8} {'to star':>8} {'new entry':>10}")
    for p in plan:
        print(f"{p['label']:24} {len(p['members']):8} {len(p['star']):8} {len(p['new']):10}")
    print(f"\n{sum(len(p['members']) for p in plan)} members, "
          f"{sum(len(p['star']) for p in plan)} to star "
          f"({sum(len(p['new']) for p in plan)} need a new gamelist entry)")
    if missing:
        print(f"\n{len(missing)} not on the card, skipped:")
        for label, a in missing[:10]:
            print(f"    {label}: {a}")
        if len(missing) > 10:
            print(f"    ... and {len(missing) - 10} more")

    if not args.apply:
        print("\ndry run — nothing written. Re-run with --apply.")
        return

    stamp = time.strftime("%Y%m%d-%H%M%S")
    bk = pathlib.Path(args.backups) / f"card-esde-{stamp}"
    (bk / "gamelists").mkdir(parents=True, exist_ok=True)
    shutil.copy2(settings, bk / "es_settings.xml")
    for p in plan:
        shutil.copy2(p["gamelist"], bk / "gamelists" / f"{p['system']}.xml")
    print(f"\nbacked up to {bk}")

    coll_dir = esde / "collections"
    for p in plan:
        write(coll_dir / f"custom-{p['label']}.cfg",
              "".join(f"%ROMPATH%/{p['system']}/{f}\n" for f in p["members"]))

        text = read(p["gamelist"])
        for f in p["star"]:
            if f in p["new"]:
                continue
            # Flag the existing block by rewriting just that block, so nothing
            # else in the file is disturbed.
            def flag(m, f=f):
                b = m.group(0)
                path = re.search(r"<path>(.*?)</path>", b)
                if not path or title(pathlib.PurePath(path.group(1)).name) != title(f):
                    return b
                if "<favorite>true</favorite>" in b:
                    return b
                return b.replace("\t</game>", "\t\t<favorite>true</favorite>\n\t</game>")
            text = re.sub(r"\t<game>.*?\t</game>", flag, text, flags=re.S)
        # Games the card holds but ES-DE never scraped have no block at all.
        # A path and a flag is a valid entry; the scraper fills in the rest.
        if p["new"]:
            add = "".join(
                f"\t<game>\n\t\t<path>./{f}</path>\n"
                f"\t\t<favorite>true</favorite>\n\t</game>\n" for f in p["new"])
            text = text.replace("</gameList>", add + "</gameList>")
        write(p["gamelist"], text)
        print(f"  {p['label']}: {len(p['members'])} members, {len(p['star'])} starred")

    # ES-DE ignores a .cfg whose name is not in this list.
    s = read(settings)
    current = re.search(r'<string name="CollectionSystemsCustom" value="([^"]*)"', s)
    names = [x for x in (current.group(1).split(",") if current.group(1) else []) if x]
    for p in plan:
        if p["label"] not in names:
            names.append(p["label"])
    s = re.sub(r'(<string name="CollectionSystemsCustom" value=")[^"]*(")',
               lambda m: m.group(1) + ",".join(names) + m.group(2), s)
    write(settings, s)
    print(f"\nes_settings.xml: {len(names)} custom collections enabled")

    for d in {coll_dir, settings.parent} | {p["gamelist"].parent for p in plan}:
        subprocess.run(["dot_clean", "-m", str(d)], check=False)


if __name__ == "__main__":
    main()
