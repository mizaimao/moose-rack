# Keeping four copies of the library in step

Server, Retro SSD, Android and the Flip. This is what was done on 2026-08-29,
and — more useful — the things that turned out to be true about comparing ROM
sets, which are not obvious and cost time to learn.

## The server is the cleaned one

Frank spent weeks cleaning the RomM library. **The server is right by
definition.** Do not re-derive that; take it as given and bring the other
devices to it.

## How RomM hashes, which is not how you would guess

| file | what RomM stores in `md5_hash` |
| --- | --- |
| uncompressed ROM | md5 of the file |
| `.zip` | md5 of every member's bytes **concatenated in zip order** |
| `.7z` | something else again — neither the archive nor its contents |

`unzip -p file \| md5` reproduces the zip case exactly. Verified against
`3 Ninjas Kick Back (USA).zip` and a 22-member arcade set, `10yard.zip`.

For `.7z` neither side is comparable, so compare raw file md5 on both ends
instead.

This is why a first comparison said 4,921 files needed deleting and 4,922
copying: every zipped system mismatched because the archive was being hashed
rather than the ROM inside it.

## RetroAchievements hashes differently again

Per console, not per file. **NES strips the 16-byte iNES header**; SNES strips
a 512-byte copier header when present; GB/GBC/GBA/Mega Drive use the whole
file. RomM's `ra_hash` column is produced by `RAHasher`, RA's own tool, so it
is already correct — use it rather than computing your own. A raw `md5` of a
`.nes` will never match RA, and mistaking that for a bad dump nearly caused a
good ROM to be replaced.

`ra_id` in RomM is empty for almost everything, including Pokémon Red. That
means RomM has not resolved its hashes to games, **not** that the ROMs are
wrong. Ignore that column.

## "This ROM is not supported" is usually the core

RetroArch says that when the **emulator core** is not on RetroAchievements'
approved list — nothing to do with the ROM. mGBA is approved for GBA; `vba-m`
is not. Switching the Flip's GBA core to vba-m for an unrelated cheat fix made
every GBA game report unsupported.

Of the library, 46% of hashed ROMs have achievements, 37% are recognised
dumps with no achievement set authored, and 17% are unknown to RA.

**Arcade is hashed by file name, not contents.** RA's arcade hash is the md5 of
the filename with its extension removed — verified against this library, where
`midssio`, `blockade` and `mineswpr` all match exactly. That is why the arcade
folder must keep MAME short names.

**Neo Geo AES cannot be hashed at all.** RetroAchievements has no such console;
Neo Geo sits under Arcade. RomM's `neogeoaes` platform has `ra_id NULL`, so
`RAHasher` is never invoked and its 162 ROMs stay unhashed for ever. That is
not a failure to fix. Moving them to `arcade` would not help either: their
names are descriptive (`3 Count Bout ~ Fire Suplex (NGM-043)(NGH-043).zip`)
rather than MAME set names, so the filename hash would resolve to nothing.

A `.m3u` playlist has no RA hash either — RA hashes discs, not playlists.

## No-Intro decides. Hasheous only points

`https://hasheous.org/api/v1/Lookup/ByHash/MD5/<md5>` aggregates No-Intro,
Redump and TOSEC, and returns the canonical filename in
`signature.rom.name` — including TOSEC's dump flags:

    Home Alone (1991)(THQ)(EU-US).gb                            clean
    Kat's Run - Zennihon K Car Senshuken (1995)(Atlus)(JP)[b3]  bad dump
    Chase H.Q. (1991-03-08)(Taito)(JP)(en)[h]                   hacked

That is a *lead*, not a verdict. **Confirm against the No-Intro DAT before
replacing anything.** On 2026-08-29 a sweep of 336 ROMs flagged 18, and two of
them were fine:

- `ClayFighter (Europe).zip` — flagged `[b]`. It is the exact No-Intro
  `ClayFighter (Europe).md`. TOSEC had matched it against a *US* entry.
- `Star Fox (USA) (Rev 2).zip` — flagged `[a]`. It is the exact No-Intro
  `Star Fox (USA) (Rev 2).sfc`.

Both would have been replaced with a worse file on TOSEC's word alone.

The DATs are fetchable and need no account:

    https://raw.githubusercontent.com/libretro/libretro-database/master/metadat/no-intro/<System>.dat

`Nintendo - Game Boy.dat`, `Sega - Game Gear.dat`,
`Sega - Mega Drive - Genesis.dat`,
`Nintendo - Super Nintendo Entertainment System.dat`. Each `rom (` line carries
`name`, `size`, `crc`, `md5`, `sha1`; index them by md5 and a lookup is
instant. Note the Game Gear DAT names some entries `.sms` — Game Gear and
Master System share a format, and those are still Game Gear entries.

**Hasheous's MD5 index has holes.** `Taito Chase H.Q. (USA)` returned no match
by MD5 and resolved correctly by SHA1. If MD5 comes back empty, try
`/Lookup/ByHash/SHA1/<sha1>` before concluding anything.

Being *absent* from RetroAchievements proves nothing either — their coverage is
thin on Europe-only releases and revisions. Of 336 ROMs RA did not recognise,
318 were clean dumps it simply does not cover.

## Traps that produced wrong answers

**Multi-disc and multi-file roms.** A game can be one row on the server and
many files on a card: `.Final Fantasy VII (USA)/` plus a `.m3u`, or a folder
RomM marks `multi_file=1` such as `gba/Aftermarket`. Comparing filenames
deletes the parts. 24 of them were held back only because this was caught.

**Unhashed server rows.** 204 rows under `sfc/AdditionalRoms` have no md5 at
all. A hash-only comparison called their card copies orphans and would have
deleted 195 files. Always fall back to a name match before deleting.

**Basenames are not paths.** Listing with `basename` loses the subfolder, so
`gb/Aftermarket/Foo.zip` becomes `gb/Foo.zip` and every later operation misses
it. This cost two separate runs.

**Extensions belong to the destination.** The server may hold a game zipped
where a device holds it raw. Taking the server's *name* including its
extension renames a raw `.gg` to `.zip` — a file no emulator opens, and
`unzip -p` then reads it as empty so two such files hash identically and look
like duplicates. Take the name, keep the local extension.

**macOS adds junk.** `tar` and `7zz` on this Mac write `._` AppleDouble files
onto exFAT. 920 landed on the SD card and 10 in the SSD's ROM tree. On exFAT
some are not real files at all but the xattrs of a real one — `rm` fails and
`xattr -c` is what clears them.

**`scp` inside `while read` eats stdin.** The loop runs once and reports
success. Use `</dev/null` on every `ssh`/`scp` inside a loop. Also `rsync`
failed against this SSD every time with "unexpected end of file" where `scp`
worked.

## What a sync run actually has to move

A rename is four things, not one: the ROM, its `-image.png` artwork, its
`.srm`/`.state` saves, and its `gamelist.xml` entry. Renaming only the ROM
orphans the rest silently.

`gamelist.xml` is edited as text, never parsed and rewritten — it holds
scraped tags like `<cheevosHash>` that a rewrite would drop. Escape `&` when
inserting a filename, or the file stops being valid XML and ES reads nothing.

## Where it stands

**2026-08-29.** All four copies agree on dump quality. Fourteen bad dumps were
replaced with exact No-Intro matches across server, SSD, Android and Flip, each
verified after it landed rather than only before it was sent:

    gamegear  Chase H.Q. · Indiana Jones · NHL All-Star Hockey · Poker Face Paul's
              Poker · R.B.I. Baseball '94 · Super Off Road · Surf Ninjas · Tails Adventure
    gb        Rocky and Bullwinkle · Garfield Labyrinth · FIFA Road to WC 98 · Spot
    megadrive Landstalker (USA) · Flashback - The Quest for Identity (USA)

Six of those were renamed to their No-Intro names, which also moved their
artwork and gamelist entries. The Game Gear library — 328 ROMs — was copied to
the Flip, which previously had none.

Hashing is complete: 1,556 unhashed rows down to the 162 Neo Geo AES and one
`.m3u`, neither of which can ever hash. Every ROM on disk that RA can hash has
a hash.

Still open:

- The Flip is behind on the older backlog — 56 merged games, the Home Alone fix
  and the two Oddworld discs.
- `Flashback (Japan).zip` is not a No-Intro dump on any device. Left alone;
  only the USA release was asked for.
- The SSD and Android still have artwork filed under pre-rename names for
  `Ecco the Dolphin (USA, Europe, Brazil) (En)` and `Madden 96 (USA, Europe)`.
  The Flip's copies were mapped across, so only those two are stale.
- No Game Gear favourites exist anywhere — not in any gamelist, not as a
  RomM collection. The other nine systems have a `★ Best of <system>`.
- Compressing the 4,158 raw cart ROMs (~4.5 GB saving) is deliberately not
  done: it renames files, so it desyncs any device it is not done on. Frank
  wants tests first.
