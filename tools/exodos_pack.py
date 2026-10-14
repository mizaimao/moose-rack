#!/usr/bin/env python3
"""Pack downloaded eXoDOS games into the library as DOSBox Pure games.

    python3 tools/exodos_pack.py "DOOM (1993)"            # say what it would do
    python3 tools/exodos_pack.py "DOOM (1993)" --apply
    python3 tools/exodos_pack.py --all --apply            # every downloaded game

Runs on the server, where eXoDOS lives (docs/dos.md). A game has to have been
fetched into eXoDOS/eXo/eXoDOS/ first. For each one it writes:

1. ROMs/dos/<Title (Year)>.zip -- eXoDOS's zip unchanged, plus DOSBOX.CONF at
   its root: eXo's own config for the game with the [autoexec] mount taken
   out, because once Pure has loaded the zip, the zip is C:.
2. ES-DE/gamelists/dos/gamelist.xml -- the game's entry, from eXo's LaunchBox
   metadata.
3. ES-DE/downloaded_media/dos/<type>/ -- its artwork, named after the zip.

Those three are the DOS side's to write by agreement ("Working with the other
session" in docs/dos.md). Nothing else is touched.

Games whose [autoexec] mounts a CD image are refused for now: whether Pure
wants the imgmount line dropped is untested.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import sys
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

LIBRARY = Path("/home/frank/moose-library")

# LaunchBox image type -> ES-DE media folder, in the order they are reported.
MEDIA = {
    "Box - Front": "covers",
    "Box - Back": "backcovers",
    "Box - 3D": "3dboxes",
    "Screenshot - Gameplay": "screenshots",
    "Screenshot - Game Title": "titlescreens",
    "Clear Logo": "marquees",
    "Fanart - Background": "fanart",
    "Disc": "physicalmedia",
}

# Where LaunchBox files a region's art when a type is split by region. The
# unsplit folder is tried first; this decides between regions after that.
REGIONS = ["North America", "United States", "World", "Europe", "United Kingdom"]

# Characters LaunchBox replaces with "_" when it names an image after a title.
LB_UNSAFE = re.compile(r"[\\/:*?\"<>|']")


class Refused(Exception):
    """A game this tool will not pack yet, with the reason."""


def adapt_autoexec(conf: str) -> str:
    """eXo's dosbox.conf, with its C: mount made to work in both places it runs.

    eXo mounts a folder under its eXoDOS directory as C: -- the game's own in
    6,140 of 7,666 configs (`mount c .\\eXoDOS\\DOOM`), eXoDOS itself in
    1,465. Here the zip's root stands in for that directory, and it reaches
    DOSBox Pure two ways:

    - The desktop loads the zip, and Pure mounts it as C: before the autoexec
      runs. eXo's mount has to go.
    - EmulatorJS unpacks the zip into its own filesystem root and hands Pure
      this file alone, which runs like plain DOSBox: nothing is mounted.
      `mount c /` is the zip's root there. Pure's own `automount = true` does
      the same mount and crashes the core ("memory access out of bounds").

    So the mount stays, guarded by whether C: holds this file already, and
    `c:` is followed by a `cd` to the folder eXo mounted. Anything eXo does
    after that, its own `cd` included, lands where it always did. Both were
    checked with DOOM: the desktop through RetroArch, the browser through
    EmulatorJS, the same zip.
    """
    nl = "\r\n" if "\r\n" in conf else "\n"
    out: list[str] = []
    in_autoexec = False
    target: str | None = None
    switched = False
    for line in conf.splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            in_autoexec = stripped.lower() == "[autoexec]"
            out.append(line)
            continue
        if in_autoexec:
            cmd = stripped.lstrip("@")
            mount = re.match(r'(?i)mount\s+c\s+("[^"]+"|\S+)', cmd)
            if mount:
                path = mount.group(1).strip('"').replace("/", "\\").rstrip("\\")
                if path.lower() == ".\\exodos":
                    target = ""
                elif path.lower().startswith(".\\exodos\\"):
                    target = path[len(".\\eXoDOS\\"):]
                else:
                    raise Refused(f"mounts C: outside eXoDOS: {stripped}")
                # 8.3, which is how Pure itself looks for it in a zip.
                out.append("@if not exist C:\\DOSBOX.CON mount c /")
                continue
            if re.match(r"(?i)imgmount\s", cmd):
                raise Refused(f"mounts a CD image, untested in Pure: {stripped}")
            if cmd.lower() == "c:":
                if target is None:
                    raise Refused("switches to C: without mounting it")
                out.append(line)
                if target:
                    out.append(f"cd \\{target}")
                switched = True
                continue
        out.append(line)
    if not switched:
        raise Refused("its [autoexec] never switches to C:, so where it starts is unclear")
    return nl.join(out) + nl


def short_name(game_zip: Path) -> str:
    """The game's folder inside its zip, which is also its eXo `!dos` name."""
    with zipfile.ZipFile(game_zip) as z:
        tops = {n.split("/", 1)[0] for n in z.namelist() if "/" in n}
    if len(tops) != 1:
        raise Refused(f"expected one top-level folder in the zip, found {sorted(tops)}")
    return tops.pop()


def record_for(metadata_zip: Path, stem: str) -> dict[str, str]:
    """The game's LaunchBox record, found by its launcher `<stem>.bat`."""
    want = f"\\{stem}.bat"
    with zipfile.ZipFile(metadata_zip) as z, z.open("xml/all/MS-DOS.xml") as f:
        for _, el in ET.iterparse(f):
            if el.tag != "Game":
                continue
            if (el.findtext("ApplicationPath") or "").endswith(want):
                return {c.tag: (c.text or "").strip() for c in el}
            el.clear()
    raise Refused(f"no LaunchBox record launches {stem}.bat")


def gamelist_entry(file_name: str, rec: dict[str, str]) -> ET.Element:
    game = ET.Element("game")

    def add(tag: str, value: str | None) -> None:
        if value:
            ET.SubElement(game, tag).text = value

    add("path", f"./{file_name}")
    add("name", rec.get("Title"))
    add("desc", rec.get("Notes"))
    stars = rec.get("CommunityStarRating")
    if stars:
        add("rating", f"{min(float(stars) / 5, 1):.2f}")
    date = rec.get("ReleaseDate", "")
    if re.match(r"\d{4}-\d{2}-\d{2}", date):
        add("releasedate", date[:10].replace("-", "") + "T000000")
    add("developer", rec.get("Developer"))
    add("publisher", rec.get("Publisher"))
    add("genre", ", ".join(g.strip() for g in rec.get("Genre", "").split(";") if g.strip()))
    add("players", rec.get("MaxPlayers"))
    return game


def media_for(names: list[str], title: str) -> dict[str, str]:
    """ES-DE folder -> the archive member holding that art, first image only."""
    lb = LB_UNSAFE.sub("_", title)
    found: dict[str, str] = {}
    for kind, folder in MEDIA.items():
        base = f"Images/MS-DOS/{kind}/"
        pattern = re.compile(re.escape(base) + r"(?:([^/]+)/)?" + re.escape(lb) + r"-01\.(png|jpg|jpeg)$")
        hits = [(m.group(1), n) for n in names if (m := pattern.match(n))]
        if not hits:
            continue
        rank = {None: -1, **{r: i for i, r in enumerate(REGIONS)}}
        hits.sort(key=lambda h: rank.get(h[0], len(REGIONS)))
        found[folder] = hits[0][1]
    return found


def write_atomically(dest: Path, write) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    try:
        write(part)
        os.replace(part, dest)
    finally:
        part.unlink(missing_ok=True)


def merge_gamelist(path: Path, entry: ET.Element) -> None:
    if path.is_file():
        root = ET.parse(path).getroot()
    else:
        root = ET.Element("gameList")
    want = entry.findtext("path")
    for old in [g for g in root.findall("game") if g.findtext("path") == want]:
        root.remove(old)
    root.append(entry)
    ET.indent(root, space="\t")

    def write(part: Path) -> None:
        with open(part, "wb") as f:
            f.write(b'<?xml version="1.0"?>\n')
            f.write(ET.tostring(root, encoding="utf-8"))
            f.write(b"\n")

    write_atomically(path, write)


def pack(stem: str, library: Path, apply: bool, art_names: list[str]) -> None:
    exo = library / "eXoDOS"
    game_zip = exo / "eXo" / "eXoDOS" / f"{stem}.zip"
    if not game_zip.is_file():
        raise Refused(f"not downloaded: {game_zip}")
    short = short_name(game_zip)
    with zipfile.ZipFile(exo / "Content" / "!DOSmetadata.zip") as z:
        try:
            conf = z.read(f"eXo/eXoDOS/!dos/{short}/dosbox.conf").decode("utf-8", "replace")
        except KeyError:
            raise Refused(f"no dosbox.conf for !dos/{short}") from None
    adapted = adapt_autoexec(conf)
    rec = record_for(exo / "Content" / "XODOSMetadata.zip", stem)
    art = media_for(art_names, rec.get("Title", stem))

    out_zip = library / "ROMs" / "dos" / f"{stem}.zip"
    autoexec = adapted[adapted.lower().index("[autoexec]"):].strip().splitlines()
    print(f"{stem}")
    print(f"  {game_zip.name} -> {out_zip}, with DOSBOX.CONF: {' | '.join(autoexec[1:])}")
    print(f"  gamelist: {rec.get('Title')}, {rec.get('ReleaseDate', '')[:4]}, {rec.get('Genre', '')}")
    print(f"  art: {', '.join(art) or 'none'}")
    if not apply:
        return

    # DOSBOX.CONF goes first. EmulatorJS starts the first .conf it unpacks, and
    # some games ship a .conf of their own; appended, ours would come after it.
    def write_zip(part: Path) -> None:
        with zipfile.ZipFile(game_zip) as src, zipfile.ZipFile(part, "w") as z:
            z.writestr("DOSBOX.CONF", adapted, compress_type=zipfile.ZIP_DEFLATED)
            for info in src.infolist():
                if info.is_dir():
                    z.writestr(info, b"")
                    continue
                with src.open(info) as f, z.open(info, "w") as g:
                    shutil.copyfileobj(f, g, 1 << 20)

    write_atomically(out_zip, write_zip)
    es = library / "ES-DE"
    merge_gamelist(es / "gamelists" / "dos" / "gamelist.xml", gamelist_entry(out_zip.name, rec))
    with zipfile.ZipFile(exo / "Content" / "XODOSMetadata.zip") as z:
        for folder, member in art.items():
            dest = es / "downloaded_media" / "dos" / folder / f"{stem}{Path(member).suffix.lower()}"
            write_atomically(dest, lambda part, m=member: part.write_bytes(z.read(m)))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("games", nargs="*", help='zip names without .zip, e.g. "DOOM (1993)"')
    ap.add_argument("--all", action="store_true", help="every game downloaded so far")
    ap.add_argument("--apply", action="store_true", help="write; without it, only report")
    ap.add_argument("--library", type=Path, default=LIBRARY)
    args = ap.parse_args()

    downloaded = args.library / "eXoDOS" / "eXo" / "eXoDOS"
    games = list(args.games)
    if args.all:
        games += sorted(p.stem for p in downloaded.glob("*.zip"))
    if not games:
        ap.error("name a game, or pass --all")

    with zipfile.ZipFile(args.library / "eXoDOS" / "Content" / "XODOSMetadata.zip") as z:
        art_names = [n for n in z.namelist() if n.startswith("Images/MS-DOS/")]

    refused = 0
    for stem in dict.fromkeys(games):
        try:
            pack(stem, args.library, args.apply, art_names)
        except Refused as e:
            refused += 1
            print(f"{stem}\n  refused: {e}")
    if not args.apply:
        print("\nNothing written. Pass --apply to write it.")
    return 1 if refused else 0


if __name__ == "__main__":
    sys.exit(main())
