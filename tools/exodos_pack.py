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

A game that mounts a CD image gets it mounted in both places it runs: in the
browser by the image's own path, before C: exists; on the desktop by DOSBox
Pure, which mounts the first CD image it finds in the zip as D: by itself.
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

# On the NAS since 2026-10-11: ROMs, ES-DE and eXoDOS all live on the
# GameFiles share, mounted here on the server.
LIBRARY = Path("/mnt/gamefiles")

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

    A CD image (`imgmount d .\\eXoDOS\\kknd\\cd\\kknd.cue -t cdrom`) is mounted
    under the same guard, by its path in the unpacked zip, and before C: is:
    after it, the guard can no longer tell the two places apart. On the
    desktop the guard is false and Pure has already mounted the image as D:.
    """
    nl = "\r\n" if "\r\n" in conf else "\n"
    out: list[str] = []
    in_autoexec = False
    target: str | None = None
    switched = False
    lines = conf.splitlines()
    autoexec = re.split(r"(?mi)^\s*\[autoexec\]\s*$", conf, maxsplit=1)[-1].splitlines()
    cd_mounts = [cd_mount(l.strip()) for l in autoexec if re.match(r"(?i)@?\s*imgmount\s", l.strip())]
    # Moved ahead of C:, a mount stops being conditional. Under a menu that
    # mounts one disc or another, both would run and the choice would be lost.
    if cd_mounts and any(re.match(r"(?i)@?\s*(goto\b|choice\b|if\s+errorlevel\b|:\w)", l.strip()) for l in autoexec):
        raise Refused("mounts a CD from inside a menu, which nothing has tried")
    cds_written = False
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            in_autoexec = stripped.lower() == "[autoexec]"
            out.append(line)
            continue
        if in_autoexec:
            cmd = stripped.lstrip("@").strip()
            mount = re.match(r'(?i)mount\s+c\s+("[^"]+"|\S+)', cmd)
            if mount:
                path = mount.group(1).strip('"').replace("/", "\\").rstrip("\\")
                if path.lower() == ".\\exodos":
                    target = ""
                elif path.lower().startswith(".\\exodos\\"):
                    target = path[len(".\\eXoDOS\\"):]
                else:
                    raise Refused(f"mounts C: outside eXoDOS: {stripped}")
                # 8.3, which is how Pure itself looks for it in a zip. Once,
                # however many times eXo mounts C:.
                if not cds_written:
                    for m in cd_mounts:
                        out.append(f"@if not exist C:\\DOSBOX.CON {m}")
                    cds_written = True
                out.append("@if not exist C:\\DOSBOX.CON mount c /")
                continue
            if re.match(r"(?i)imgmount\s", cmd):
                # Written above, with the C: mount.
                continue
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


def cd_mount(line: str) -> str:
    """eXo's `imgmount` line with its image paths made paths in the unpacked zip.

    `.\\eXoDOS\\kknd\\cd\\kknd.cue` is `/kknd/cd/kknd.cue` there, because the zip's
    root stands in for eXoDOS. Only CD images: a floppy or a hard disk image is
    refused, as nothing has tried one.
    """
    cmd = line.lstrip("@").strip()
    if not re.search(r"(?i)-t\s+(cdrom|iso)\b", cmd):
        raise Refused(f"mounts an image that is not a CD, untested: {line}")
    parts = re.findall(r'"[^"]+"|\S+', cmd)
    out = []
    for part in parts:
        bare = part.strip('"')
        low = bare.lower().replace("/", "\\")
        if low.startswith(".\\exodos\\"):
            path = "/" + bare.replace("\\", "/")[len(".\\eXoDOS\\"):]
            out.append(f'"{path}"' if " " in path else path)
        elif low.startswith(".\\"):
            raise Refused(f"mounts an image outside eXoDOS: {line}")
        else:
            out.append(part)
    return " ".join(out)


# A track in a CD sheet: `FILE "name" BINARY`, the name quoted or one word.
FILE_LINE = re.compile(r'(?im)^(\s*FILE\s+)(?:"([^"\r\n]+)"|(\S+))([^\r\n]*?)(\r?)$')


def spell_cds(adapted: str, game_zip: Path) -> tuple[str, dict[str, bytes]]:
    """The CD mounts with their images named as the zip names them, and the
    sheets whose tracks needed the same, corrected.

    eXo's configs and sheets were written on Windows, where `KKND.CUE` finds
    `kknd.cue`. EmulatorJS unpacks the zip into a filesystem that does not
    ignore case, so there a mount or a track can miss: 45 of eXoWin9x's sheets
    do. The zip written below is ours, so a sheet is corrected inside it.
    """
    sheets: dict[str, bytes] = {}
    with zipfile.ZipFile(game_zip) as z:
        members = {n.lower(): n for n in z.namelist() if not n.endswith("/")}

        def image(m: re.Match) -> str:
            want = m.group(0).strip('"').lstrip("/")
            got = members.get(want.lower())
            if got is None:
                raise Refused(f"mounts {want}, which is not in the zip")
            if got.lower().endswith(".cue"):
                sheet = fixed_sheet(z, got, members)
                if sheet is not None:
                    sheets[got] = sheet
            return f'"/{got}"' if " " in got else f"/{got}"

        out = []
        for line in adapted.splitlines(keepends=True):
            if re.search(r"(?i)\bimgmount\s", line):
                line = re.sub(r'(?<=\s)("/[^"]+"|/[^\s"]+)', image, line)
            out.append(line)
    return "".join(out), sheets


def fixed_sheet(z: zipfile.ZipFile, cue: str, members: dict[str, str]) -> bytes | None:
    """`cue` with its tracks spelled as the zip spells them, or `None` when it
    needs nothing. Bytes in, bytes out, so nothing else in it changes."""
    folder = cue.rsplit("/", 1)[0] + "/" if "/" in cue else ""
    raw = z.read(cue)
    bom = b"\xef\xbb\xbf" if raw.startswith(b"\xef\xbb\xbf") else b""
    text = raw[len(bom):].decode("latin-1")
    changed = False

    def spell(m: re.Match) -> str:
        nonlocal changed
        track = (m.group(2) or m.group(3)).replace("\\", "/")
        if folder + track in z.NameToInfo:
            return m.group(0)
        got = members.get((folder + track).lower())
        if got is None:
            raise Refused(f"{cue} names {track}, which is not in the zip")
        changed = True
        return f'{m.group(1)}"{got[len(folder):]}"{m.group(4)}{m.group(5)}'

    out = FILE_LINE.sub(spell, text)
    return bom + out.encode("latin-1") if changed else None


def short_name(game_zip: Path) -> str:
    """The game's folder inside its zip, which is also its eXo `!dos` name."""
    with zipfile.ZipFile(game_zip) as z:
        tops = {n.split("/", 1)[0] for n in z.namelist() if "/" in n}
    if len(tops) != 1:
        raise Refused(f"expected one top-level folder in the zip, found {sorted(tops)}")
    return tops.pop()


def record_for(metadata_zip: Path, stem: str, short: str) -> dict[str, str]:
    """The game's LaunchBox record, found by its launcher.

    By the game's `!dos` folder first: the launcher is named after the full
    title, `KKND - Krush, Kill 'N' Destroy (1997).bat`, while the zip drops
    the punctuation, `KKND - Krush Kill N Destroy (1997).zip`. `<stem>.bat`
    is the fallback for a record filed some other way.
    """
    folder = f"\\!dos\\{short}\\".lower()
    want = f"\\{stem}.bat"
    fallback = None
    with zipfile.ZipFile(metadata_zip) as z, z.open("xml/all/MS-DOS.xml") as f:
        for _, el in ET.iterparse(f):
            if el.tag != "Game":
                continue
            path = el.findtext("ApplicationPath") or ""
            if folder in path.lower():
                return {c.tag: (c.text or "").strip() for c in el}
            if fallback is None and path.endswith(want):
                fallback = {c.tag: (c.text or "").strip() for c in el}
            el.clear()
    if fallback is not None:
        return fallback
    raise Refused(f"no LaunchBox record launches !dos/{short} or {stem}.bat")


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
    adapted, sheets = spell_cds(adapt_autoexec(conf), game_zip)
    rec = record_for(exo / "Content" / "XODOSMetadata.zip", stem, short)
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
                if info.filename in sheets:
                    z.writestr(info, sheets[info.filename])
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
