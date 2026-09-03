# The library rules

**Every file hash-verified. Every filename the consensus database-registered
name.** Both halves, always — renaming is part of the job, not a separate
question to ask.

Frank had to repeat this more than ten times before it stuck. It is not a
preference to confirm per batch; it is the definition of what the library is.

The rest of this file is how that rule is applied, and the mechanics that have
gone wrong at least once. [inventory.md](inventory.md) is the procedure and the
schema; this is the policy.

## Naming

A file's name comes from **its own hash**, never from the game it resembles.

- An alternate keeps its alternate name. A bad dump keeps a name that says
  `[b]`. A translation keeps its translator tag.
- Variant markers — `[a1]`, `[a2]`, `(Alt)`, `[b2]`, `[p1]`, `(T)` — are
  identity, and are never stripped when matching. Stripping them once matched
  seven NES alternates to the consensus dump and deleted them.
- CJK filenames stay CJK. Everything else takes the database name.
- **Arcade, arcade_original and neogeo are never renamed.** The filename is the
  MAME/FBNeo set name the emulator looks up, and parent/clone loading depends
  on it.

`(T)` in a filename means Translation. Patched ROMs are catalogued nowhere,
which is exactly how you recognise one: the plain release matches a database,
the `(T)` copy matches nothing.

## Databases, in order

No-Intro for cartridges, Redump for discs, TOSEC via Hasheous when neither
knows the hash. Record which matched. **No-Intro decides over TOSEC** when they
disagree — two of eighteen files TOSEC flagged one night were exact No-Intro
matches, and acting on the flag would have made the library worse.

Per-console quirks that decide whether a hash matches at all:

    NES / Famicom   iNES header is 16 bytes. Hash the body.
    SNES / SFC      No-Intro is HEADERLESS. Strip a 512-byte copier header
                    when size % 1024 == 512. The opposite of NES.
    N64             three byte orders; No-Intro lists big-endian (z64)
    Arcade          RA hashes the *filename*, not the contents
    CD-based CHD    needs extraction; DVD-based compares straight off the header

## Try the transform before sourcing anything

Trimmed ROMs pad to full cart size — `0xFF` usually, sometimes `0x00`.
Overdumps truncate. Headered versus headerless is a hashing difference, not a
different file.

Eleven Mega Drive files and one NES were fixed this way with no download at
all. Always try it before asking Frank to find something.

## What gets removed

- **Bad dumps**, when a good one exists. If the only dump in circulation is
  bad, keep it and let the name say so.
- **Prototypes and betas**, only when the game had a real release. For a
  proto-only game the prototype *is* the game — check the DAT for a non-proto
  entry rather than guessing.
- **Duplicates**, where the same ROM sits under two names.

Everything removed moves to the session scratch folder. Nothing is deleted
outright.

## What is kept but never verified

Fan translations and hacks. No database has ever registered a hash for them, so
they can be neither verified nor renamed. That is a floor, not a backlog — do
not report it as outstanding work.

## One region per game

US preferred, then World, then Europe, then Japan — unless the contents diverge
enough to be worth keeping on their own. European multi-language builds and PAL
size differences are not divergence; they are the same game.

Rank the region tag, never substring-match it. `(Japan, USA)` is a USA release
and outranks `(Europe)`, but does not contain the string `(USA`.

## Newest revision only

A game held in more than one revision keeps the newest. `(Rev N)`, `(Rev A/B/C)`
and `(vN.N)` all count. Group on the filename with the revision marker stripped
so the comparison stays inside one region and one language set.

## Group by the cartridge id, never by the title

A game released in more than one region is usually renamed in each. Titles
cannot see that; the cartridge header can.

    N64   3-char game id at 0x3B, region letter at 0x3E
    GBA   4-char game code at 0xAC, 4th character is the region
    SNES  4-char code in the extended header, only ~35% of carts carry one
    NES   nothing; there is no reliable key

What this catches that a title match never will:

    Quest 64 (USA) = Holy Magic Century (Europe) = Eltale Monsters (Japan)
    Castlevania - Aria of Sorrow (USA) = Akatsuki no Minuet (Japan)
    Over the Hedge (USA) = Ab durch die Hecke / Vecinos Invasores / four more

Two traps, both hit for real:

- **Do not require the internal cart title to match.** It is localised too, so
  `FINDET NEMO` never equals `FINDING NEMO` and the guard blocks every genuine
  pair. Group on the id plus the *publisher* code instead; that still separates
  bootlegs that reuse an id.
- **The region byte lies.** A Brazilian NTSC release carries `E` like a USA
  cart. Group by the id, but rank by the region tag in the *filename*.

Placeholder ids (`000`) are not identities. Exclude homebrew, aftermarket,
bootlegs, protos and translations from region matching entirely.

## Compilations

A "2 Games in 1" cart is redundant when every game inside is also held
standalone. 32 of 44 on GBA were. Keep the ones holding a game that exists
nowhere else -- the Atari collections are the only copy of Gauntlet, Klax,
Centipede and a dozen more.

## Imports

One region per title, preferring **USA > World > Europe > Japan**. Skip anything
already held, by hash *and* by title. Exclude betas, protos, demos and
plug-and-play compilation variants. Name from the database at the moment the
file lands, never carry in the source set's filename.

Unlicensed titles go to their own folder — `nes_unlicensed`, sorted into
`Bootleg Ports`, `Originals` and `Multicarts`.

## Mechanics that have gone wrong, and are now guarded

- **exFAT is case-insensitive.** `BattleTech` and `Battletech` are the same
  file. A case-only rename goes via a temporary name; copy-then-delete destroys
  it. Three Mega Drive files were lost this way and restored from the payload.
- **Never overwrite an existing file at the target name.** Check first whether
  it is the same ROM: 54 blocked renames turned out to be duplicates the import
  had created, 52 the same ROM under a different iNES header.
- **A rename carries everything named after the file** — artwork across eleven
  media types, saves, `.m3u` entries, the gamelist entry, the RomM row.
- **Normalise to NFC before comparing names.** macOS and the Flip store the
  same CJK characters differently; a byte comparison calls identical files
  missing.
- **A folder name is not evidence.** 80 of 124 WonderSwan files were in the
  wrong system and 7 Neo Geo Pocket files had the wrong extension. The hash
  decides which system a file belongs to.
- **A set is dated.** A file that fails a current DAT while matching an older
  one is stale, not broken.

## Cleaning a system, in order

The sequence that worked on GBA, 2,110 files down to 879. Run it in this order;
each step narrows what the next has to look at.

1. **Hash the source set against No-Intro.** Repair before sourcing -- 12 bad
   dumps were fixed from the set itself, and 5 turned out to be GBC games in the
   wrong folder. The hash decides the system, not the folder name.
2. **Import what is missing**, one region per title, into `AdditionalRoms/`
   split by `Japanese`, `Unlicensed`, `Multicart`.
3. **Deduplicate by cartridge id.** See the section above. 258 regional
   duplicates on GBA that no title match could see.
4. **Newest revision only.** See that section.
5. **Drop compilations** whose every game is held standalone -- 32 of 44.
6. **Content block-hash the folder** for what metadata cannot see. 64 KB blocks,
   skip the all-`00`/all-`FF` padding, pairs sharing >=55% are the same game.
   This found `Yu-Gi-Oh! Duel Monsters Expert 3 (Japan)` = `World Championship
   Tournament 2004 (USA)` at 99.5%.
7. **Drop games with no English at all** where no English release exists.
8. **Rename everything to its database name.**
9. **Rebuild from a reference list** if the set is still too big -- the Flip's
   own library is the best one, since it is what Frank actually carries. Keep
   the SSD's corrected names, not the reference's older ones.

### Four traps, all of which bit

- **Never require the internal cart title or the publisher code to match.** Both
  are localised. `FINDET NEMO` is not `FINDING NEMO`; Konami Japan is `EM` and
  Konami USA is `A4`. Adding either as a guard blocks every genuine cross-region
  pair -- this killed the method twice in one session before it was spotted.
- **A file that fails its DAT may be a patch, not a bad dump.** The GBA repair
  pass replaced Frank's Shin-chan English patch and a double-patched FF6 with
  vanilla copies. Check the size: a patch is usually *larger* than the base rom
  and shares ~98% of its blocks. Record them with verdict `patched`.
- **Content matching is useless across localisations.** A Japanese build shares
  0% with its US twin because it is compiled separately. `Lufia` and `Chinmoku
  no Iseki` can only be matched by knowing the games.
- **The region byte lies.** A Brazilian NTSC release carries `E` like a USA
  cart. Group by the id, rank by the region tag in the *filename*.

### Filters that do not work

Tried and discarded, so nobody tries them again:

- **ScreenScraper ratings.** American Bass Challenge 1.0, Wario Land 4 0.9. No
  vote count is stored, so two votes read the same as two hundred.
- **Publisher.** Zoo Digital *distributed* Alien Hominid, R-Type III and SimCity
  2000 in Europe. A budget label is not a bad game.
- **"Licensed tie-in" as a category.** 225 were moved out and moved straight
  back; Frank plays plenty of them.
- **RetroAchievements as a quality list.** Sets exist for 466 of the GBA library
  but also for 300 games not held. It measures who built a set, not what is good.

There is no community "avoid" list to download. Naming bad games is judgement,
and it should be offered as judgement rather than dressed up as data.

## Scope

SSD only. Server, Android and Flip get pushed once the canonical copy is right.

## Where it stands

See [inventory.md](inventory.md). Finished: NES, Famicom, Mega Drive,
WonderSwan, WonderSwan Color, Neo Geo Pocket, Game Boy, Game Boy Color, Game
Boy Advance, Super Famicom, SNES, Nintendo 64.

Remaining: disc systems, arcade, `0_BIOS`.

Game Boy set the folder split the later systems follow: English releases at the
top, `AdditionalRoms/Japanese` and `AdditionalRoms/Unlicensed` beneath it.
