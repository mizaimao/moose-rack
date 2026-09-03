# Hashing the library and checking every dump

A per-file hash store for the Retro SSD, matched against No-Intro, Redump,
TOSEC and MAME, so that every game gets two answers: **is this a good dump**,
and **what should it be called**.

Renaming is not part of this. The database records a proposed name; acting on
it is a separate job, and [devices.md](devices.md) explains why — a filename is
the id for artwork, saves, gamelist entries and RomM rows across four machines.

Agreed with Frank 2026-08-30. Steps below are in order and can be re-run.

## What is covered

31 systems, 9,759 files, about 660 GB.

Excluded, and why:

    switch, ps3, psvita, xbox360    too large to be worth it
    scummvm                         game data folders, no database exists
    dos, easyrpg, macintosh,        empty
    mame, ports, ps4, xbox
    n3ds                            80 .zcci files; no decompressor here
    gc, wii (RVZ), wiiu (WUX)       needs dolphin-tool, which we will not install

`0_BIOS` (170 files) **is** hashed, flagged as BIOS, and dealt with separately
later — a wrong BIOS breaks emulation silently, so it is worth knowing.

## Where it lives

`/Volumes/Retro/_inventory/` — SQLite plus a JSON export. On the SSD rather
than in this repo: it is data, it is large, and another session works in the
repo.

## Schema

One row per file. Two hashes per file, because they answer different questions:
the **container** hash proves a transfer arrived intact, the **inner** hash
identifies the dump. Half of one night's confusion came from conflating them.

| column | |
| --- | --- |
| `device` | `ssd` for now. The same store can later hold server, android, flip |
| `system` | folder name as it is on disk, `gamegear_hidden` and all |
| `path` | relative to the system folder, so nested dirs survive |
| `size` | bytes |
| `container_md5`, `container_sha1` | of the file itself |
| `inner_md5`, `inner_sha1` | of the ROM inside, when it is an archive |
| `inner_name`, `member_count` | what is in the archive |
| `container_format` | `raw`, `zip`, `7z`, `chd`, `iso` |
| `stripped_md5` | header-stripped hash, NES and SNES only — see below |
| `dat_source` | `no-intro`, `redump`, `tosec`, `mame`, several, or none |
| `dat_name` | the canonical filename the match gives |
| `verdict` | `good`, `bad`, `hacked`, `alt`, `unknown`, `error` |
| `proposed_name` | what a rename would use. **Never applied automatically** |
| `status`, `error` | `ok`, `truncated`, `unreadable`, `no-dat-match`, … |
| `group_id` | links multi-disc sets: an `.m3u` and its discs share one |
| `hashed_at` | so it is obvious which rows predate a later change |

When loose files are zipped later, the row keeps its inner hash and gains a
container hash. Nothing has to be recomputed.

## The databases

All four come from one mirror, no account needed:

    https://raw.githubusercontent.com/libretro/libretro-database/master/metadat/<db>/<System>.dat

`no-intro/` for cartridges, `redump/` for discs, `tosec/` for dump flags,
`mame/` and `fbneo-split/` for arcade. A file can match more than one; the row
records which, so "No-Intro says good, TOSEC says `[b]`" is visible rather than
resolved silently in favour of whichever was checked first.

**No-Intro decides for cartridges.** TOSEC's flags are a lead, not a verdict —
two of eighteen files it flagged one night were exact No-Intro matches, and
replacing them would have made the library worse.

## Steps

### 1. Fetch the DATs

One file per system per database, into `_inventory/dats/`. Index them by md5
and by sha1; a lookup is then instant and offline.

### 2. Walk the tree

`find`, not `ls`. Several systems have nested folders — `sfc` has 10, `n64` 3,
`nes` 2, and `N64_All` is 553 zips under region directories. A flat listing
loses them, which has already cost two separate runs.

Normalise names to NFC before comparing anything. macOS and the Flip store the
same CJK characters differently, and a byte comparison reports identical files
as missing.

### 3. Hash

Cartridges first — 6,300 files, ~15 GB, roughly twenty minutes. Then discs.

    raw file        md5 + sha1
    .zip / .7z      container hash, then the inner ROM's hash
                    (record member_count; a 33-member GoodSNES bundle is not a game)
    .nes            also hash with the 16-byte iNES header stripped
    .sfc/.smc       also hash with a 512-byte copier header stripped when size % 1024 == 512
    .iso            hash directly; Redump lists exactly this
    .chd            see below

### 4. CHDs

Read the header first. `chdman info` reports a metadata tag that decides the
method:

- **`Tag='DVD '`, 2048-byte hunks** — the decompressed image *is* the ISO, so
  the Data SHA1 compares straight to Redump. No extraction.
- **`Tag='CHT2'`, 19,584-byte hunks** — CD, multi-track, with subchannel data
  interleaved. Nothing in Redump corresponds to it. `chdman extractcd` to
  cue+bin, hash each track, compare, delete. One disc at a time keeps scratch
  space under a gigabyte.

By system: psp and most ps2 verify for free; **psx (93), dreamcast (25),
3do (10), saturn (4)** need extraction, and those are the small ones.

Also parse the header directly to catch truncation, which `chdman` reports only
as a vague I/O error. If `map offset` is past the end of the file, the file is
short and nothing can be extracted from it. That is how
`Amplitude (USA).chd` (PS2) was found to be missing 46 MB — a 1.4 GB file that
looked fine in a listing and would have got a happy-looking md5 from a plain
hash pass.

`chdman verify` proves a CHD is not internally corrupt. It says nothing about
whether the dump is right. Different question.

### 5. Match and judge

Look up every hash in every DAT. Record the source, the canonical name, and a
verdict. A file that matches nothing is `unknown`, not `bad` — coverage is thin
on Europe-only releases, and RetroAchievements absence proves nothing at all.

Arcade is verified but **never renamed**: for MAME and FBNeo the filename *is*
the set name, and parent/clone loading depends on it.

### 6. Index the references

Before any rename can be trusted, record every place a filename appears:

    ES-DE/gamelists/<system>/gamelist.xml     <path> and <image>
    ES-DE/support/downloaded_media/<system>/  eleven media types per system
    Saves/ and saves/                         .srm, .state
    .m3u contents                             multi-disc playlists
    RomM                                      the roms table on the server

Roughly doubles the build time. It is the difference between a rename that can
be trusted and one that cannot.

### 7. Report, do not act

Per system: how many good, bad, unknown; what would be renamed and to what.
Frank reviews per system before anything moves.

## Decisions already taken

- `Amplitude (USA).iso` (PS2) stays an ISO. Compression to CHD happens later,
  together with zipping the loose cartridge ROMs — one pass, one set of new
  container hashes.
- NES and Famicom stay separate systems. So do SNES and SFC. Merging later is
  easy; splitting is not.
- `_hidden` folder names are kept exactly as they are.

## First run — 2026-08-30

Built. `/Volumes/Retro/_inventory/` holds `inventory.db`, `inventory.json`,
`report.md`, the DATs, the scripts and the logs. 103 MB.

**14,291 files, 2,124 archive members, 97,618 references.**

Cartridges came out at 6,655 verified good against No-Intro. The remainder is
mostly explainable: `sfc` is 412 multi-member GoodSNES bundles, of which 409
contain a clean dump; `famicom` and `sfc` are heavy with fan translations that
no DAT lists; `wonderswan` and `ngp` are thin in the libretro mirror (219 and 9
entries) rather than wrong on disk.

Discs went as designed. PSP's DVD CHDs verified 109 of 112 straight from the
header with no extraction. Every single-track CD disc that was extracted
matched Redump exactly — psx 90, 3do 18. The 68 that did not match are **all**
multi-track, none genuinely unrecognised, and they need cue-accurate splitting
that reproduces Redump's track boundaries including generated pregaps. Not
guessed at: getting it wrong reports good dumps as bad. PS2's 118 CD-style CHDs
are deferred — extraction is roughly fifteen hours for that system alone.

Arcade: 2,097 sets are exact zip-hash matches against the FBNeo and MAME DATs,
507 are known sets from a different emulator version, 156 unrecognised. The
`neogeo` folder is largely unmatched because it uses descriptive names rather
than MAME set names, which is expected and is why nothing there is renamed.

### Four bugs this run had, all found by checking rather than assuming

- The iNES header is 16 bytes. Reading the 4-byte magic and hashing from there
  gave NES **0** good out of 849. Fixed: 752.
- N64 dumps come in three byte orders and No-Intro lists big-endian. Converting
  first took n64 from 27 good to 555, and N64_All from 9 to 544.
- Header stripping ran only on loose files, so every headered `.nes` inside a
  `.zip` missed. Famicom went from 1 good to 256.
- The CHD truncation check used the v5 header layout on v4 files and called 15
  healthy Dreamcast CHDs truncated. It is now version-gated.

The lesson each time: a verdict of "bad" that arrives in bulk is a bug in the
checker until proven otherwise.

### What renaming would touch

1,132 files across the cartridge systems are verified good under a name that
differs from No-Intro's, and 1,127 of those carry references — artwork, saves,
gamelist entries. Nothing has been renamed.

## Where each system stands — 2026-08-30

The rule this all serves: **every file hash-verified, every filename the
consensus database-registered name.** Both halves, always, without asking.
See the memory note `hash-verified-database-names`.

NES, Famicom and Mega Drive are finished. What is left in each is fan
translations and hacks that no database has ever registered a hash for, so they
can be neither verified nor renamed. That is a floor, not a backlog.

    nes         860 / 891       genesis    1191 / 1221
    famicom     838 / 989       nes_unlicensed 1115 / 1118

Tonight took NES from 752 verified to 860, Famicom from 256 to 838, Mega Drive
from 882 to 1191, and imported 259 Mega Drive, 701 NES/Famicom and 1,115
unlicensed titles under their database names.

### Still worth doing

- **sfc, 171 of 709.** 412 files are multi-member GoodSNES bundles — every
  variant of a game in one archive. 409 contain a clean dump. Unpack, verify,
  name. The one substantial job left.
- **wonderswan 3/53, wonderswancolor 25/71, ngp 24/48.** ~95 unexplained, but
  the libretro DATs for these are thin — the Neo Geo Pocket one has **nine
  entries**. Suspect coverage, not bad dumps.
- **gb, gbc, gba, n64, snes** — 68 unexplained between them, scattered.

### Deliberately not work

- **`0_BIOS`, 3,339 files.** Hashed, never matched: BIOS images are not in
  No-Intro. Separate job.
- **arcade, neogeo, arcade_original.** 507 sets come from a different MAME
  version. Recognised, and **never renamed** — the filename is the set name the
  emulator looks up, and parent/clone loading depends on it.
- **psx, dreamcast, saturn** — 68 multi-track CDs needing cue-accurate track
  splitting including generated pregaps. Not guessed at: getting it wrong
  reports good dumps as bad.

### Renaming, and the two ways it bites

1,139 files were renamed to their database names, carrying 9,036 artwork files
and their gamelist entries. Two hazards, both hit for real:

- **exFAT is case-insensitive.** `BattleTech` → `Battletech` is the *same file*.
  Copy-then-delete destroys it — three Mega Drive files were lost this way and
  restored from the payload. Rename via a temporary name instead.
- **The target name may already exist.** 54 renames were blocked that way. All
  turned out to be duplicates the import had created: the title matcher treated
  `Break Time (U)` and `Break Time - The National Pool Tour (USA)` as different
  games. 52 were the same ROM under a different iNES header, 2 the same ROM in
  a differently-compressed zip.

**Variant markers are identity, not noise.** `[a1]`, `[a2]`, `(Alt)`, `[b2]` must
stay in the normalised key. Stripping them matched seven NES alternates to the
consensus dump and deleted them. An alternate keeps whatever name its own hash
carries — Hasheous returns TOSEC names such as
`Jaws (1987-11)(LJN)(US)[a2].nes` for exactly this.

### Sets are dated, and that matters

Frank's Mega Drive library *was* a 2017 No-Intro set — 898 of 966 files
byte-identical to it. No-Intro has since re-dumped 88 of those. A file that
fails a 2026 DAT while matching a 2017 one is **stale, not broken**.

Better still: 11 of them were reconstructed with no sourcing at all. They were
trimmed ROMs, and padding with `0xFF` to full cart size reproduces the current
No-Intro hash exactly. Four NES overdumps went the other way, fixed by
truncation. Always try the transform before asking for a new file.

### Only the SSD is current

Server, Android and Flip still carry the old names and none of the imports —
roughly 2,000 imports and 1,193 renames behind. That was deliberate: get the
canonical copy right, then push.

## Progress, 2026-08-30

Finished, in order: NES, Famicom, Mega Drive, WonderSwan, WonderSwan Color,
Neo Geo Pocket. The policy these follow is [library-rules.md](library-rules.md).

    nes             860 / 891      wonderswan       96 / 102
    famicom         838 / 989      wonderswancolor 103 / 114
    genesis        1191 / 1221     ngp             106 / 119
    nes_unlicensed 1115 / 1118

Three findings worth carrying forward:

**A folder name is not evidence.** 80 of 124 WonderSwan files sat in the wrong
system — `wonderswan_hidden` was 45 Color games, `wonderswancolor_hidden` was 35
WonderSwan games — and 7 Neo Geo Pocket files had the wrong extension. Verifying
against one system's DAT made a healthy library look broken. File by hash.

**Thin DATs look like bad dumps.** Neo Geo Pocket's libretro DAT has nine
entries. WonderSwan's names are largely catalogued by TOSEC under their serial,
not by No-Intro. "Unverified" there meant uncatalogued, not wrong: ngp went from
24 verified to 106 without a single bad dump being found.

**The set may be worse than what you have.** Two Neo Geo Pocket replacements
were rejected: the set's `Bust-A-Move Pocket (USA)` is a Beta, and its Puyo Puyo
is an older Japanese revision against Frank's v1.06. Check region and variant
before replacing, not just whether the hash verifies.

## Game Boy, 2026-08-30

Source: `Nintendo - Game Boy` in the work folder, 1,919 archives, 1,899 matching
No-Intro 2026.08.01. The 20 that do not are demos, test programs and BIOS.

    gb   1420 / 1363        gbc   493 / 470

What the pass did:

    10  bad dumps replaced from the source
     5  GBC games moved out of gb, hash-matched against the GBC DAT
     8  Sachen rips swapped for the 9 complete multicarts
    20  unverifiable roms filed under AdditionalRoms/Unlicensed
   793  new titles imported
     2  Europe copies dropped where a USA copy was already held

Frank's folder split, which the later systems should follow:

    gb/                            English releases
    gb/AdditionalRoms/Japanese     Japan-only
    gb/AdditionalRoms/Unlicensed   unlicensed, homebrew, multicarts
    gb/Aftermarket                 pre-existing, left alone

Three things learned here:

**ES-DE media mirrors the ROM subfolder.** Moving a rom into `AdditionalRoms/`
means its artwork moves to `downloaded_media/gb/<type>/AdditionalRoms/` too.
`Aftermarket/` already worked this way and showed the convention.

**A GB cart header identifies a multicart rip.** Offset 0x134 reading `GAME`
with a null old-licensee byte at 0x14B means a game carved out of a Sachen
4-in-1. No database registers those individually; the whole cart is the entry.

**exFAT here has a 1 MB allocation block.** A 32 KB rom occupies 1 MB. `gb` is
325 MB of data reported as 2.2 GB. Small-rom systems will always look enormous
on this volume; it is not a fault.

## Game Boy Color and Game Boy Advance, 2026-08-30

Same pass as Game Boy, same folder split.

    gbc  1359 / 1338      gba  2112 / 2097

    gbc    2 bad dumps replaced,  866 imported
    gba   14 bad dumps replaced, 1300 imported

The GBA multiboot set (96 files) was skipped on Frank's word: it is WarioWare
and GameCube kiosk demos plus the 30 Animal Crossing NES games he already owns,
no No-Intro DAT exists for multiboot, and Hasheous knows none of them.

**The libretro GBA DAT is incomplete.** 67 of a 3,379-file No-Intro set are
missing from it, a dozen of them real retail games — Fear Factor Unleashed,
Shamu's Deep Sea Adventures, the Harry Potter and Crash/Spyro compilations.
`Happy Feet (USA)` is byte-identical to the source set's copy and still reads as
unverified. Before calling a file bad, check whether the DAT simply lacks it.

## Region deduplication, 2026-08-30

Frank's rule: one region per game, US preferred, unless the contents diverge
enough to be worth keeping separately. 455 files dropped, 19,321 -> 18,866.

    n64 207   N64_All 204   nes_unlicensed 27   genesis 11   sfc 3   other 3

Nothing diverged in the end. The 186 first flagged as divergent were European
multi-language builds and PAL size differences — `Banjo-Kazooie (Europe)
(En,Fr,De)` is Banjo-Kazooie with French and German, not a different game.

**Rank the region tag, do not substring-match it.** `1080 Snowboarding (Japan,
USA)` does not contain the string `(USA`, so it ranked as Japan and was about to
lose to the Europe copy. Parse the first parenthesised group, split on commas,
and take the best member. This is the second time this bug has nearly cost a
file; the first was Dr. Mario `(JU)`.

The `_hidden` systems were skipped — Frank said leave those be. 10 files there
would otherwise qualify.

Still open: `gamelists/nes/gamelist.xml` is duplicated end to end, two `<?xml`
declarations and two `<gameList>` roots in one file. `n64` and `N64_All` share
339 of their hashes and are effectively the same library twice.

## Nintendo 64, rebuilt 2026-08-30

`n64` was two folders holding one library — `n64` 364 files and `N64_All` 349,
sharing 338 roms. `N64_All` was worth one file. It is gone.

**The library was in mixed byte orders and the extensions lied.**
`Star Fox 64 (USA).n64` was a v64 file; `FIFA 64 (Europe).z64` was a v64 file;
`Super Mario 64 (USA).z64` really was z64. Every one converted to the right
No-Intro hash, so nothing was broken, but comparing raw bytes against a
big-endian source showed 26 matches out of 1,140. Convert before comparing, and
never trust an N64 extension.

Rebuilt from the BigEndian source, everything now genuine z64:

    n64/                             21   the games Frank plays
    n64/AdditionalRoms/USA          334
    n64/AdditionalRoms/Europe       341
    n64/AdditionalRoms/Japan        235
    n64/AdditionalRoms/Unlicensed    16

Three files were kept out of the replacement because the source has no copy: the
Sealed Palace pirate cart, the Majora's Mask Redux romhack, and Wonder Project
J2 `(T-En by Ryu v1.0)`. **That last one nearly went.** Its `dat_name` is the
plain Japanese rom, so sourcing by dat_name would have silently replaced an
English translation with the untranslated game. Check the verdict, not just the
name, before replacing anything marked `mismatch`.

## Newest revision only, 2026-08-30

Frank's rule: when a game is held in more than one revision, keep the newest.
115 games across the cart systems, 145 older copies; 142 dropped.

    n64 84   nes_unlicensed 43   wonderswancolor_hidden 9   ngp_hidden 6

`(Rev N)`, `(Rev A/B/C)` and `(vN.N)` all mark a revision. Group on the filename
with that marker removed — same game, same region, same languages — so
`Bible Adventures (USA) (v1.5) (Unl)` collects its five older siblings but never
pulls in a different region. gb, gbc, gba and sfc came back clean: their imports
already took one copy per title.

Left undone: genesis, nes and snes have one each — NHLPA Hockey 93, Track &
Field II, Final Fantasy II.

## Discs, 2026-08-31

    psx  117 / 117      ps2  119 / 115      dreamcast  31 / 25
    3do   18 /  18      psp  112 / 109      saturn      4 /  0

**Dreamcast verified nothing because the verifier could not read it.** Its CHDs
carry a `CHGD` tag -- GD-ROM, the Dreamcast's own format -- and `cdverify.py`
only knew `CHT2` and `DVD `, so it skipped 20 discs without recording an error.
Both extract with `chdman extractcd`.

Six Dreamcast discs still do not match, all short by exactly 529,200 bytes on
the last track -- 225 sectors, the same number on six different games. They came
from GDI rips; Redump's dumps include the mastering pattern Sega wrote into the
lead-out. Padding with 0x00 or 0xFF does not reproduce it. They play correctly.

**Multi-disc layout.** The `.m3u` goes at the system root and the discs in
`MultiDisk/`; that is what the gamelists already pointed at. Eight of nine psx
games and both dreamcast games had it the other way round, so every multi-disc
game was showing without its metadata. Fixing it also found 27 psx and 5
dreamcast discs stored twice -- 12 GB.

**An ES-DE gamelist can have two root elements.** `<alternativeEmulator>` sits
before `<gameList>` on 8 systems here. A strict XML parser rejects that, so
every script using ET.parse silently skipped those gamelists and left 92 dead
entries behind. `scratchpad/gledit.py` splits the prologue off and reassembles.
A sweep repointed 475 entries, moved 33 to the right system, removed 224.

Also: 3,533 AppleDouble `._` files, 3 GB, left by `shutil.copy2` preserving
extended attributes exFAT cannot store. Use plain copies.

## Game Boy Advance, rebuilt from the Flip, 2026-08-31

The full set was too large to browse on a handheld, so `gba` became
`gba_superset` (1,642 files) and a fresh `gba` was built from the games the
Flip actually carries, plus everything unlicensed, aftermarket and multicart.

    gba/                            764
    gba/Aftermarket                  24
    gba/AdditionalRoms/Unlicensed    62
    gba/AdditionalRoms/Multicart     29
                                    879 files, 865 verified

770 of the Flip's 786 mapped across -- 736 by filename, 34 by title because the
name had been corrected here since. The copies take the SSD's No-Intro names,
not the Flip's older ones, so the two now disagree on 34 filenames.

**Two patched roms were destroyed and restored.** The morning's repair pass
replaced `Shin chan - Aventuras en Cineland (Spain) (patched)` and
`Final Fantasy VI Advance (USA) double_patched` with vanilla copies because they
failed No-Intro. A file that fails the DAT may be a patch, not a bad dump; the
same trap was caught on Wonder Project J2 for N64 and then walked into here.
Both are now recorded with verdict `patched` so nothing replaces them again.

**Duplicate hunting, what worked and what did not.** Cart id and internal title
found 258 regional duplicates. Content block-hashing found 24 more that no
metadata could see -- `Yu-Gi-Oh! Duel Monsters Expert 3 (Japan)` is
`World Championship Tournament 2004 (USA)` at 99.5%. But content is useless for
localised builds: a Japanese release shares 0% with its US twin because it is
compiled separately, so `Lufia` vs `Chinmoku no Iseki` and `Horsez` vs
`Mein Gestuet` can only be matched by knowing the games.

Filters that turned out to be worthless: ScreenScraper ratings (American Bass
Challenge 1.0, Wario Land 4 0.9), publisher (Zoo Digital *distributed* Alien
Hominid and R-Type III), and "licensed tie-in" as a category -- Frank plays
plenty of those, and the 225 moved out were moved straight back.

There is no community "avoid" list to download. RetroAchievements has sets for
466 of these but 300 more for games not held, so it measures who built a set,
not what is good.

Next: arcade (663 gaps, filename-based hashing), `0_BIOS` (3,339), saturn (4).
