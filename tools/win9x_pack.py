#!/usr/bin/env python3
"""Put eXoWin9x games into the library, to play in the browser.

    python3 tools/win9x_pack.py "ZuluWar (1996)"            # say what it would do
    python3 tools/win9x_pack.py "ZuluWar (1996)" --apply
    python3 tools/win9x_pack.py --all --apply               # every game

Runs on the server, where eXoWin9x lives (docs/win9x.md). The games are played
where they are: nothing of eXo's is copied or changed. For each game it writes:

1. ROMs/windows9x/<Title (Year)>.w9x -- a DOSBox-X config for js-dos: eXo's
   own Play.conf with the drives pointed at the page's files and the network
   card taken out. Its first lines name the game's zip and the Windows 98 disk
   it boots, which is how the page knows what to fetch.
2. ES-DE/gamelists/windows9x/gamelist.xml -- the game's entry, from eXo's
   LaunchBox metadata.
3. ES-DE/downloaded_media/windows9x/<type>/ -- its artwork, named after the .w9x.

Those three and eXoWin9x/moose/ (the Windows 98 disk) are the Windows side's
to write. Nothing else is touched.
"""

from __future__ import annotations

import argparse
import collections
import os
import re
import sys
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

# On the NAS, beside ROMs and ES-DE (docs/win9x.md).
LIBRARY = Path("/mnt/gamefiles")

# The Windows 98 disk every game boots, inside eXoWin9x/. Ours, not eXo's: a
# clean install with eXo's add-ons moved across (docs/win9x.md). A new build
# gets a new name, because the page keeps the disk and asks for it by name.
BASE = "moose/moose98-2.vhd"

# The only parent our disk stands in for. The rest are eXo's Windows 98 with
# networking, Japanese and Chinese Windows 98, and Windows 95.
PARENT = "w98-c.vhd"

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

# eXo's settings, changed for the browser. Our Windows 98 has no network
# drivers, and with the card in it Windows finds new hardware on every boot and
# waits for an answer. `output` is the window system, which js-dos owns.
SET = {("ne2000", "ne2000"): "false"}
DROP = {("sdl", "output")}


class Refused(Exception):
    """A game this tool will not pack yet, with the reason."""


def members_by_lower(z: zipfile.ZipFile) -> dict[str, str]:
    return {n.lower(): n for n in z.namelist() if not n.endswith("/")}


def in_zip(path: str, year: str, members: dict[str, str]) -> str:
    """eXo's path for a file, as the file's name in the game's zip.

    eXo writes `.\\eXoWin9x\\1996\\ZuluWar (1996)\\ZWDEMO.zip`, and the zip holds
    `ZuluWar (1996)/ZWDEMO.zip`. The page unpacks the zip into js-dos's root, so
    that name is also the file's path there. Matched without regard to case,
    as Windows does, and written as the zip spells it, because js-dos does not.
    """
    bare = path.strip('"').replace("\\", "/")
    head = f"./exowin9x/{year}/"
    if not bare.lower().startswith(head):
        raise Refused(f"names a file outside the game's folder: {path}")
    want = bare[len(head):]
    got = members.get(want.lower())
    if got is None:
        raise Refused(f"names {want}, which is not in the zip")
    return got


def quoted(name: str) -> str:
    return f'"{name}"' if " " in name else name


# A track: `FILE "name" BINARY`. The name is quoted or one word, and the type
# is not always one word: Bad Mojo's sheet says `BINARY BINARY`.
FILE_LINE = re.compile(r'(?im)^(\s*FILE\s+)(?:"([^"\r\n]+)"|(\S+))([^\r\n]*)$')


def fixed_cue(z: zipfile.ZipFile, cue: str, members: dict[str, str]) -> str | None:
    """The CD sheet with its track names spelled as the zip spells them.

    `None` when it needs nothing. 45 of eXo's sheets name `GAME.BIN` beside
    `game.bin`, which only works where file names ignore case, and js-dos's do
    not. The zip is not changed: the page writes the corrected sheet beside the
    original, under a new name, and the mount names that.
    """
    folder = cue.rsplit("/", 1)[0] + "/" if "/" in cue else ""
    raw = z.read(cue)
    # A byte-order mark would hide the first FILE line from the pattern.
    text = raw.removeprefix(b"\xef\xbb\xbf").decode("latin-1")
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
        return f'{m.group(1)}"{got[len(folder):]}"{m.group(4)}'

    out = FILE_LINE.sub(spell, text)
    if not changed:
        return None
    # The .w9x is UTF-8 and the page writes the sheet back as UTF-8, so any
    # other byte would come out as two. None of eXo's 45 has one; refusing is
    # better than a sheet that silently names a file that is not there.
    if not out.isascii():
        raise Refused(f"{cue} needs correcting and is not plain ASCII")
    return out


def adapt(conf: str, year: str, z: zipfile.ZipFile) -> tuple[list[str], dict[str, str]]:
    """eXo's Play.conf as the page's DOSBox-X config, comments left out, and
    any corrected CD sheets the page has to write (`fixed_cue`).

    eXo builds C: as a throwaway child of its parent disk, mounts the game's own
    disk as D: and its CDs or zip as E:, then boots C:. The page has our disk as
    `c.vhd` and the zip unpacked, so C: is that file and the rest are paths in
    the zip. eXo's own `xcopy` lines, which fix up a game's INI files before
    Windows starts, are kept as they are.
    """
    members = members_by_lower(z)
    extra: dict[str, str] = {}
    out: list[str] = []
    section = ""
    parent = None
    booted = False
    # Split on line ends only: `splitlines` also breaks at \x85 and \x0c,
    # which latin-1 can produce. And only whole-line comments go, because
    # DOSBox-X keeps a `#` later in a line as part of the value.
    for raw in re.split(r"\r?\n", conf):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        head = re.fullmatch(r"\[(.+)\]", line)
        if head:
            section = head.group(1).lower()
            out.append(f"[{section}]")
            continue
        if section != "autoexec":
            key, _, value = (s.strip() for s in line.partition("="))
            if (section, key.lower()) in DROP:
                continue
            out.append(f"{key}={SET.get((section, key.lower()), value)}")
            continue
        words = re.findall(r'"[^"]+"|\S+', line)
        cmd = words[0].lower()
        if cmd == "vhdmake":
            parent = next((w.strip('"').replace("\\", "/").rsplit("/", 1)[-1].lower() for w in words if "parent" in w.lower()), None)
            continue
        if cmd == "imgmount" and words[1].lower() == "c":
            out.append("imgmount c c.vhd")
            continue
        if cmd in ("imgmount", "mount"):
            drive = words[1].lower()
            files, flags = [], []
            for w in words[2:]:
                (flags if w.startswith("-") or flags else files).append(w)
            names = [in_zip(f, year, members) for f in files]
            for i, n in enumerate(names):
                if n.lower().endswith(".cue") and (text := fixed_cue(z, n, members)) is not None:
                    names[i] = n[:-4] + ".moose.cue"
                    extra[names[i]] = text
            out.append(" ".join([cmd, drive, *map(quoted, names), *flags]))
            continue
        if cmd == "boot":
            booted = True
            out.append(line)
            continue
        if cmd in ("xcopy", "echo", "cls", "@echo"):
            out.append(line)
            continue
        raise Refused(f"its [autoexec] runs something untried: {line}")
    if parent != PARENT:
        raise Refused(f"boots eXo's {parent or 'nothing'}, and ours stands in for {PARENT} only")
    if not booted:
        raise Refused("its [autoexec] never boots Windows")
    return out, extra


def record_for(records: dict[str, dict[str, str]], year: str, stem: str) -> dict[str, str]:
    """The game's LaunchBox record, found by its folder under `!win9x`."""
    rec = records.get(f"\\!win9x\\{year}\\{stem}\\".lower())
    if rec is None:
        raise Refused(f"no LaunchBox record launches !win9x/{year}/{stem}")
    return rec


def load_records(metadata_zip: Path) -> dict[str, dict[str, str]]:
    """Every LaunchBox record, by the game's `!win9x` folder.

    The file is eXo's merge of per-game snippets: `<Game>` after `<Game>` with
    no document element around them, which a parser refuses as it stands. So it
    gets one.
    """
    out = {}
    with zipfile.ZipFile(metadata_zip) as z:
        body = z.read("xml/all/1994-1996.9x").removeprefix(b"\xef\xbb\xbf")
    for el in ET.fromstring(b"<LaunchBox>" + body + b"</LaunchBox>").iter("Game"):
        path = (el.findtext("ApplicationPath") or "").lower()
        m = re.search(r"\\!win9x\\\d{4}\\[^\\]+\\", path)
        if m:
            out[m.group(0)] = {c.tag: (c.text or "").strip() for c in el}
    return out


def gamelist_entry(file_name: str, rec: dict[str, str]) -> ET.Element:
    game = ET.Element("game")

    def add(tag: str, value: str | None) -> None:
        if value:
            ET.SubElement(game, tag).text = value

    add("path", f"./{file_name}")
    add("name", rec.get("Title"))
    add("desc", rec.get("Notes"))
    stars = rec.get("CommunityStarRating")
    if stars and float(stars) > 0:
        add("rating", f"{min(float(stars) / 5, 1):.2f}")
    date = rec.get("ReleaseDate", "")
    if re.match(r"\d{4}-\d{2}-\d{2}", date):
        add("releasedate", date[:10].replace("-", "") + "T000000")
    add("developer", rec.get("Developer"))
    add("publisher", rec.get("Publisher"))
    add("genre", ", ".join(g.strip() for g in rec.get("Genre", "").split(";") if g.strip()))
    add("players", rec.get("MaxPlayers"))
    return game


def media_for(by_kind: dict[str, list[str]], title: str, stem: str, game_id: str = "") -> dict[str, str]:
    """ES-DE folder -> the archive member holding that art, one image each.

    eXoWin9x names a picture two ways: after the game's zip, `Afterlife
    (1996).png`, 889 of them, or after its LaunchBox title as eXoDOS does,
    `ZuluWar!-01.jpg`, where the lowest number is not always 01 (`ACME-02.png`
    is ACME's only cover). Matching the title with -01 alone left 118 of 544
    games without a cover. The zip's name is the exact one, so it wins.

    `game_id` is the LaunchBox record's `ID`. Two games with one title are told
    apart by it in the file name (`Mastermind.c682f3a7-...-01.png` is the BBC
    one), so a picture carrying another game's id is never taken.
    """
    lb = LB_UNSAFE.sub("_", title)
    rank = {None: -1, **{r: i for i, r in enumerate(REGIONS)}}
    loose = {loose_name(title), loose_name(stem)}
    found: dict[str, str] = {}
    for kind, folder in MEDIA.items():
        base = re.escape(f"Images/Windows 9x/{kind}/") + r"(?:([^/]+)/)?"
        by_stem = re.compile(base + re.escape(stem) + r"\.(png|jpe?g)$", re.I)
        by_title = re.compile(base + re.escape(lb) + r"-(\d+)\.(png|jpe?g)$", re.I)
        hits = []
        for n in by_kind.get(kind, []):
            if m := by_stem.match(n):
                hits.append((0, rank.get(m.group(1), len(REGIONS)), 0, n))
            elif m := by_title.match(n):
                hits.append((1, rank.get(m.group(1), len(REGIONS)), int(m.group(2)), n))
            elif n.lower().endswith((".png", ".jpg", ".jpeg")) and loose_name(n.rsplit("/", 1)[-1]) in loose:
                tagged = re.search(r"\.([0-9a-f]{8}-[0-9a-f-]{27})-\d+\.\w+$", n, re.I)
                if tagged and tagged.group(1).lower() != game_id.lower():
                    continue
                region = n.split("/")[3] if n.count("/") > 3 else None
                hits.append((2, rank.get(region, len(REGIONS)), 0, n))
        if hits:
            found[folder] = min(hits)[3]
    return found


def loose_name(name: str) -> str:
    """A name with what eXo varies between its pictures taken out, for the
    last try when neither exact form is there (13 games on 2026-10-19).

    The extension, `-01`, a LaunchBox id (`Lighthouse_ The Dark
    Being.1d464c53-...-01.png`), a year or a tag in brackets (`Creatures (1996)
    [us].png`), "the" wherever it sits (`The Incredible Machine 2 (1995).png`
    for `Incredible Machine 2, The (1995)`), and every character that is not a
    letter or a digit.
    """
    s = re.sub(r"\.(png|jpe?g)$", "", name, flags=re.I)
    s = re.sub(r"-\d+$", "", s)
    s = re.sub(r"\.[0-9a-f]{8}-[0-9a-f-]{27}$", "", s, flags=re.I)
    s = re.sub(r"\(\d{4}\)|\[[^\]]*\]", "", s)
    s = re.sub(r"\bthe\b", "", s, flags=re.I)
    return re.sub(r"[^a-z0-9]", "", s.lower())


def write_atomically(dest: Path, write) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    try:
        write(part)
        os.replace(part, dest)
    finally:
        part.unlink(missing_ok=True)


def write_gamelist(path: Path, entries: list[ET.Element]) -> None:
    """Add or replace these games' entries, keeping every other one."""
    root = ET.parse(path).getroot() if path.is_file() else ET.Element("gameList")
    want = {e.findtext("path") for e in entries}
    for old in [g for g in root.findall("game") if g.findtext("path") in want]:
        root.remove(old)
    root.extend(entries)
    ET.indent(root, space="\t")

    def write(part: Path) -> None:
        with open(part, "wb") as f:
            f.write(b'<?xml version="1.0"?>\n')
            f.write(ET.tostring(root, encoding="utf-8"))
            f.write(b"\n")

    write_atomically(path, write)


class Library:
    def __init__(self, root: Path):
        self.root = root
        self.exo = root / "eXoWin9x"
        self.meta = zipfile.ZipFile(self.exo / "Content" / "!Win9Xmetadata.zip")
        self.confs: dict[str, tuple[str, str]] = {}
        for n in self.meta.namelist():
            m = re.fullmatch(r"eXo/eXoWin9x/!win9x/(\d{4})/([^/]+)/play\.conf", n, re.I)
            if m:
                self.confs[m.group(2)] = (m.group(1), n)
        self.bats = {
            m.group(2): (m.group(1), n)
            for n in self.meta.namelist()
            if (m := re.fullmatch(r"eXo/eXoWin9x/!win9x/(\d{4})/([^/]+)/\2\.bat", n, re.I))
        }
        art = self.exo / "Content" / "XOWin9xMetadata.zip"
        with zipfile.ZipFile(art) as z:
            # By type folder, so each game searches its eight folders and not
            # all 4,900 pictures eight times.
            self.art_by_kind: dict[str, list[str]] = collections.defaultdict(list)
            for n in z.namelist():
                parts = n.split("/")
                if len(parts) > 3 and parts[0] == "Images" and parts[1] == "Windows 9x":
                    self.art_by_kind[parts[2]].append(n)
        self.records = load_records(art)

    def game_zip(self, year: str, stem: str) -> Path:
        return self.exo / "eXo" / "eXoWin9x" / year / f"{stem}.zip"


def pack(stem: str, lib: Library) -> tuple[str, ET.Element, dict[str, str], dict]:
    """The .w9x text, the gamelist entry, the art, and numbers for the report."""
    if stem not in lib.confs:
        if stem in lib.bats:
            raise Refused("eXo runs it in 86Box, which has no browser build here")
        raise Refused("not in eXoWin9x's metadata")
    year, member = lib.confs[stem]
    game_zip = lib.game_zip(year, stem)
    if not game_zip.is_file():
        raise Refused(f"no zip at {game_zip}")
    conf = lib.meta.read(member).decode("latin-1")
    with zipfile.ZipFile(game_zip) as z:
        lines, extra = adapt(conf, year, z)
        unpacked = sum(i.file_size for i in z.infolist())
    rec = record_for(lib.records, year, stem)
    zip_rel = game_zip.relative_to(lib.exo).as_posix()
    # Files for the page to write, each as `# file = <path>` and its lines as
    # `# | <line>`. In comments, so DOSBox-X reads past them.
    files = [l for name, body in extra.items() for l in [f"# file = {name}", *(f"# | {x}" for x in re.split(r"\r?\n", body.rstrip("\r\n")))]]
    text = "\n".join(
        [
            "# Windows 9x game for the browser, written by tools/win9x_pack.py from",
            f"# eXo's !win9x/{year}/{stem}/Play.conf. The page fetches the files named",
            "# below from eXoWin9x/ and boots this config in js-dos.",
            f"# zip = {zip_rel}",
            f"# base = {BASE}",
            f"# bytes = {game_zip.stat().st_size}",
            f"# unpacked = {unpacked}",
            *files,
            *lines,
        ]
    ) + "\n"
    facts = {"zip": game_zip.stat().st_size, "unpacked": unpacked,
             "autoexec": [l for l in lines[lines.index("[autoexec]") + 1:]]}
    return text, gamelist_entry(f"{stem}.w9x", rec), media_for(lib.art_by_kind, rec.get("Title", stem), stem, rec.get("ID", "")), facts


def kind(reason: str) -> str:
    """A refusal without the game's own names in it, so the totals group."""
    for pattern, label in [
        (r"boots eXo's (\S+),", r"boots eXo's \1"),
        (r"which is not in the zip", "names a file that is not in the zip"),
        (r"no zip at", "not downloaded"),
        (r"runs something untried: (\S+)", r"[autoexec] runs \1"),
    ]:
        if m := re.search(pattern, reason):
            return m.expand(label)
    return reason


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("games", nargs="*", help='zip names without .zip, e.g. "ZuluWar (1996)"')
    ap.add_argument("--all", action="store_true", help="every game in eXoWin9x")
    ap.add_argument("--apply", action="store_true", help="write; without it, only report")
    ap.add_argument("--library", type=Path, default=LIBRARY)
    args = ap.parse_args()

    lib = Library(args.library)
    games = list(args.games)
    if args.all:
        games += sorted(set(lib.confs) | set(lib.bats))
    if not games:
        ap.error("name a game, or pass --all")
    if not (lib.exo / BASE).is_file():
        print(f"warning: the Windows 98 disk is not at {lib.exo / BASE}; nothing will boot", file=sys.stderr)

    reasons: collections.Counter[str] = collections.Counter()
    packed: list[tuple[str, str, ET.Element, dict[str, str]]] = []
    for stem in dict.fromkeys(games):
        try:
            text, entry, art, facts = pack(stem, lib)
        except Refused as e:
            reasons[kind(str(e))] += 1
            print(f"{stem}\n  refused: {e}")
            continue
        except (zipfile.BadZipFile, OSError, KeyError, ValueError, IndexError) as e:
            # A zip aria2c left full-size and mostly empty, a bad CRC, a field
            # that is not a number: this game, not all 664.
            reasons[f"could not be read ({type(e).__name__})"] += 1
            print(f"{stem}\n  refused: could not be read: {e}")
            continue
        packed.append((stem, text, entry, art))
        print(f"{stem}\n  {facts['zip'] / 1e6:.0f} MB zip, {facts['unpacked'] / 1e6:.0f} MB unpacked: {' | '.join(facts['autoexec'])}")
        print(f"  gamelist: {entry.findtext('name')}; art: {', '.join(art) or 'none'}")

    print(f"\n{len(packed)} packed, {sum(reasons.values())} refused")
    for why, n in reasons.most_common():
        print(f"  {n:4}  {why}")
    if not args.apply:
        print("\nNothing written. Pass --apply to write it.")
        return 0

    roms = args.library / "ROMs" / "windows9x"
    es = args.library / "ES-DE"
    # The gamelist first: an entry whose .w9x is not written yet is ignored by
    # the scan, while a .w9x with no entry shows up under its file name.
    write_gamelist(es / "gamelists" / "windows9x" / "gamelist.xml", [e for _, _, e, _ in packed])
    with zipfile.ZipFile(lib.exo / "Content" / "XOWin9xMetadata.zip") as z:
        for stem, text, _, art in packed:
            write_atomically(roms / f"{stem}.w9x", lambda part, t=text: part.write_text(t, encoding="utf-8"))
            for folder, member in art.items():
                dest = es / "downloaded_media" / "windows9x" / folder / f"{stem}{Path(member).suffix.lower()}"
                write_atomically(dest, lambda part, m=member: part.write_bytes(z.read(m)))
                # This game's picture in another format from an earlier run
                # would win over this one: the library tries .png first.
                for ext in (".png", ".jpg", ".jpeg"):
                    old = dest.with_suffix(ext)
                    if old != dest and old.is_file():
                        old.unlink()
    # Reported, not removed: a .w9x from an earlier run for a game that no
    # longer packs. Taking games out of somebody's library is theirs to decide.
    if args.all:
        done = {stem for stem, *_ in packed}
        stale = sorted(p.stem for p in roms.glob("*.w9x") if p.stem not in done)
        if stale:
            print(f"\n{len(stale)} .w9x left from an earlier run, for games that did not pack now:")
            for s in stale:
                print(f"  {s}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
