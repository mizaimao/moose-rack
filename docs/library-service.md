# Replacing RomM

**Design, not built.** The backend that replaces the dependency on a RomM server.

Written 2026-09-01, at the end of a day spent syncing 12,370 files onto RomM
and finding that 1,254 of them were invisible when it finished.

**Not built.** This is the design and the reasoning, not a record of anything
that exists.

## Why

RomM is a good project. It is the wrong shape for this library, and the
mismatch is structural rather than a set of bugs to report upstream.

Four things went wrong in one day, and they are all the same thing:

| What happened | The underlying cause |
| --- | --- |
| Renaming 4,253 files dangled every row, collection entry and save attached to them | `rom_id` owns identity, so a file's name is load-bearing |
| A scan of 12,370 files took hours and was still wrong at the end | Every scan re-reads and re-hashes work already done |
| 1,254 games in `AdditionalRoms/` never appeared at all | A folder inside a platform is assumed to be one multi-file game |
| 878 rows survived as "missing", showing beside the new names as duplicates | The database is the source of truth, so it cannot simply forget |

The library on the SSD is already canonical: every file hash-verified, every
name the consensus database name. That work is done and it is not going to be
undone. What is needed is something that *reads* that library rather than
keeping its own opinion about it.

## The thing to build is smaller than it looks

`src/` is ~32,000 lines and most of the server already lives there.

| Already built | Lines | What it does |
| --- | --: | --- |
| `cache.rs` | 1,909 | platforms, roms, collections, collection_roms, plays |
| `esde.rs` | 1,157 | reads ES-DE gamelists and the media tree |
| `media.rs` | 1,186 | artwork resolution |
| `savesync.rs` + `statesync.rs` + `syncplan.rs` | 1,755 | save and state sync, and the plan for it |
| `download.rs` | 802 | fetching game files |
| `scrape.rs`, `coverage.rs`, `gamelist.rs` | 1,048 | metadata off disk |

The only thing RomM supplies that this does not is **being the copy several
devices agree on**. `api.rs` — 1,292 lines — is the client for that one thing.

So this is not a rewrite. It is an inversion: put a thin HTTP layer over the
`Cache` that already exists, and point `api.rs` at it instead. Most of the work
is deletion.

## Five rules

Each one exists because of a specific failure above. None of them is a
preference.

### The filesystem is the truth. The index is a cache you can delete.

The index must be rebuildable from disk alone, in minutes, with nothing lost.
Anything that cannot survive `rm index.db` does not belong in it.

This is the whole fix for the rename problem. Today a rename is a migration:
the row, the collection membership, the save, eleven kinds of artwork. It
should be a no-op.

### Identity is the content hash, never the name and never a row id

    key = md5 of the file's bytes
        = md5 of the concatenated members, sorted by name, for an archive

The second line is already the convention here — RomM uses it, the DAT work
uses it, and it is why a zipped copy and a loose copy of the same ROM compare
equal.

Store `crc32`, `md5` and `sha1` together, because No-Intro and Redump want
different ones and computing all three costs one read.

### Hash once. Ever.

Fingerprint on `(path, size, mtime)`. A file whose fingerprint is unchanged is
not read again.

A rename preserves size and mtime, so the fingerprint still matches and the
service updates the path without touching the bytes. That is what makes rule 1
cheap rather than aspirational.

`st_ino` would be the better key and is not available: the library lives on
exFAT, where macOS synthesises inode numbers that do not survive a remount.

### A folder is one game only when it says so

> **Checked against ES-DE, 2026-09-03, and this needs less work than it says
> below.** ES-DE's rule is explicit rather than heuristic: a directory is a game
> **iff its name ends in a file extension configured for that system**. Then
> either a file inside sharing the directory's name is passed to the emulator
> (`Final Fantasy VII.m3u/Final Fantasy VII.m3u`) or the directory itself is
> (`Gran Turismo 5.ps3` to RPCS3). Everything else is a folder you walk into.
>
> `esde.rs` already does the walking, and it is right: n64 comes back as 419
> games with 395 of them inside `AdditionalRoms/USA`, `/Japan` and so on,
> recorded in `rel_dir`. The 1,254 games that vanished were lost to RomM's
> assumption that any directory inside a platform is one multi-file game, and
> that assumption is not ours to inherit.
>
> This library barely uses the feature anyway — PS3 here is 50 `.lnk` shortcuts,
> not `.ps3` directories. What remains is supporting directories-with-extensions
> for completeness, which is small, and not the heuristic below.

The rule RomM applies — any directory is a multi-file game — is right for
`psx/MultiDisk/Final Fantasy IX (USA)` and wrong for
`n64/AdditionalRoms/USA/`, and it cannot tell them apart.

Treat a directory as one game when it holds an `.m3u`, or a `.cue`/`.bin` set,
or files matching `(Disc N)`. Otherwise recurse into it. A `.game` or
`.notagame` marker file overrides either way, because there will be a case
neither heuristic gets right and the answer should be a file rather than a
patch.

### There is no Scan button

Watch the tree. A fingerprint pass over an unchanged library is a `stat` per
file and finishes in seconds; the watcher makes even that unnecessary most of
the time.

Waiting hours to find out whether a sync worked is the single worst part of the
current setup.

## Collections are text files

    collections/★ Best of megadrive.txt
    collections/Arcade Shmups Vertical.txt

One game per line, by canonical name. Optionally by `md5:` prefix where a name
is ambiguous.

This is not a storage optimisation. It is so that a collection can be read,
edited, diffed, restored and reasoned about without the service running — and
so that the failure mode we hit today, where seven of sixty-four entries
silently pointed at files that no longer existed, cannot be represented.

The 2,663 memberships currently in RomM export cleanly: they are already keyed
on collection name plus platform slug plus filename.

## Saves

Keyed by `(game md5, core)`, never by name and never by a row id.

    saves/<md5>/<core>/2026-09-01T14-22-03.srm
    saves/<md5>/<core>/current -> the newest
    saves/_conflicts/<md5>/<core>/...

A renamed game keeps its saves with no migration, which is exactly the bug that
sent 18 saves and 13 states through a manual re-pointing today.

Two-way, newest wins, and **a conflict is parked rather than resolved**. Two
devices that both wrote since the last sync produce a file in `_conflicts/`;
nothing is overwritten and nothing is silently chosen. `savesync.rs` already
implements most of this against RomM's model.

The Flip stores saves per system (`saves/nes/`) and the Mac per core
(`saves/FCEUmm/`), because RetroArch's `sort_savefiles_enable` differs between
them. The service should store per core, since the core is part of a save's
identity and the system can always be derived from the game.

One trap worth writing down: Genesis Plus GX runs Mega Drive, Master System and
Game Gear, so the Flip's `saves/megadrive/` legitimately contains Game Gear
saves. Per-core storage handles that correctly and per-system does not.

## Metadata comes off disk, read-only

ES-DE's `gamelist.xml` and `downloaded_media/` are already the source of truth
for artwork, and ES-DE and Skraper already do the scraping. The service reads
them and never writes them.

Media resolves by game basename at request time against an index built during
the fingerprint pass. No symlinks — RomM's `resources/esde-media/<system>/` →
`library/roms/<system>/` symlinks are container-only paths that read as broken
from the host, which cost an hour of confusion today.

## Speak RomM's API rather than inventing one

This is the decision that makes the whole thing cheap, and it came from Frank:
the devices and apps are *already wired* to RomM. If the new service answers
the same HTTP calls, nothing on the client side changes. `api.rs` is not
rewritten, it is not touched.

**RomM publishes 171 endpoints. This client calls 18.** That is the entire
compatibility surface, pulled from the running server's own
`/openapi.json`:

    GET,POST   /api/collections              GET      /api/roms
    GET,POST   /api/collections/smart        GET      /api/roms/identifiers
    GET        /api/collections/virtual      GET,PUT  /api/roms/{id}
    GET        /api/collections/virtual/identifiers
    GET        /api/config                   GET,POST /api/saves
    GET,POST   /api/devices                  GET,POST /api/states
    GET,POST   /api/firmware                 GET      /api/search/roms
    GET        /api/heartbeat                POST     /api/sync/negotiate
    GET,POST   /api/platforms                GET      /api/users/me

Seventeen paths, twenty-four method/path pairs. That is a weekend, not a
project.

A `/openapi.json` from a running instance is a specification, and
re-implementing an interface is not copying an implementation — which matters,
because of the licence.

Internally the service should still key everything by content hash as above;
`rom_id` becomes a stable integer *derived* from the hash and handed out at the
API boundary for compatibility. The clients never learn the difference, and
the rename problem stays solved underneath.

Room to add non-RomM endpoints later — an SSE `/events` so clients stop
polling, and `since=` on the list calls so the Flip on wifi asks what changed
rather than fetching everything. Additive, so old clients keep working.

## The licence

**RomM is AGPL-3.0** — network copyleft, section 13: anyone interacting with a
modified version over a network must be offered its source.

`moose-rack` has no licence file of its own and ships binaries, so copying
RomM source into it would pull the whole combined work under AGPL-3.0. That is
a real decision, not a formality.

It is also unnecessary. RomM is Python and FastAPI; this is Rust. There was
never going to be a copy-paste, and everything needed to match the interface is
in the schema the running server already serves. **Read the schema, not the
source.**

If any RomM source is ever pasted in, the licence question stops being
theoretical and `moose-rack` needs a licence decision first. Not a lawyer;
this is the practical line and it is worth staying well clear of it.

## What is new and what is not

| | |
| --- | --- |
| New | `src-serve/` — 24 method/path pairs over the existing `Cache` |
| New | filesystem watcher and fingerprint index. Extends `cache.rs` |
| Smaller than thought | directories-with-extensions, ES-DE's actual rule. The heuristic this row planned is not needed — see above |
| One-off | migration from the RomM export already sitting in the scratchpad |
| **Untouched** | **`api.rs`** — the service speaks RomM's API, so the client does not change |
| Unchanged | `esde.rs`, `media.rs`, `gamelist.rs`, `coremap.rs`, `gamesort`, `gamefilter`, `gridnav`, `launch`, `download`, `binds`, `padpoll` |

Cutover is a URL. Point the app at the new host; point it back if it misbehaves.
Both can run at once because neither writes to the other's storage.

## Explicitly not in scope

- **A web UI.** The Tauri and SDL frontends are the UI. This decision can be
  revisited; building a third frontend now cannot.
- **Scraping.** ES-DE and Skraper do it.
- **Users and permissions.** One person, one house, one network.
- **RetroAchievements.** `achievements.rs` already talks to them directly and
  should keep doing that; it has nothing to do with the library index.

## Migration

Everything needed is already exported to the session scratchpad: 2,663
collection memberships, 29 `rom_user` rows, 18 saves, 13 states, and a snapshot
of all 9,489 `roms` rows with their old names and hashes. All of it is keyed on
name plus platform rather than on `rom_id`, so it survives the change.

Run both for a while. RomM stays up, read-only, until the new service has
served the Flip and the Thor for a couple of weeks without anyone noticing.

## What this does not fix

The library still needs artwork for 3,400 games that have never been scraped
anywhere, and no amount of re-architecting finds a cover that does not exist.
That is a scraping job, and it is unrelated to this.
