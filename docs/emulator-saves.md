# Standalone emulator saves

**Milestone 1 built, 2026-10-15; not yet run on a device.** Tracked in
Linear MOO-25. How saves kept by
standalone emulators (ARMSX2, PCSX2, PPSSPP first) sync through moose-service,
so that any device running moose-rack carries its progress. RetroArch saves
already sync and are not changed by this.

## What is missing

Save sync today moves one file per game, keyed by `(rom_id, file_name)`: a
`.srm`, a `.mcr`, a state. Standalone emulators do not keep saves that way:

- PPSSPP keeps one folder per save under `PSP/SAVEDATA/`, named by the game's
  9-character ID plus a save name (`UCUS98653DATA00`). PARAM.SFO, ICON0.PNG
  and the encrypted DATA files sit inside. Frank's copy on the NAS has 65 of
  them across 52 games.
- PCSX2 and ARMSX2 keep a PS2 memory card either as one 8 MB file or as a
  folder (a "folder memory card"). In a folder card each top-level folder is
  one save, named by the game's serial (`BASLUS-20035`,
  `BASCUS-97465RATCHET`). PCSX2 builds the card image from those folders when
  it loads the card (`FolderMemoryCard::AddFolder` in
  `pcsx2/SIO/Memcard/MemoryCardFolder.cpp`), so one game's folder can change
  without the others being touched.

Neither fits a file-per-save model, and neither is tied to a ROM in the
library: a PSP save belongs to a game ID, and a PS2 save belongs to a serial
whichever disc image made it.

## The unit

A save unit is a folder the emulator itself treats as one save. It is keyed by
`(family, key)`:

- `family` names the save format, not the app. `pcsx2` covers PCSX2, ARMSX2
  and NetherSX2; `ppsspp` covers PPSSPP and PPSSPP Gold. Two apps share a
  family only when they read each other's folders unchanged.
- `key` is the folder's own name. It is already unique within its card or
  memory stick, it survives a ROM rename, and it is what a person would look
  for.

A device syncs one PS2 folder card, so the key is the game's folder alone:
card names differ between devices (`Mcd001.ps2` here, `ChickenCard.ps2`
there) and a key with the card in it would never pair. Card-level files at the
card's root (`_pcsx2_*`) stay on the device; everything inside a game's folder
travels, its `_pcsx2_index` included.

A unit's hash is md5 over its sorted `relative/path:md5(file)` lines, which is
the scheme `savehash.rs` already uses for zips. `.DS_Store`, `Thumbs.db` and
`desktop.ini` are left out of the hash and the transfer.

Units bigger than 64 MB are not synced and are listed in the plan as skipped.
PSP install data uses the same SAVEDATA folders and the same PARAM.SFO
category (`MS`) as real saves, so size is the only cheap test. In Frank's
copy ULUS10509DAT is 845 MB of installed data, the next largest folder is
22 MB of downloaded content, and every other one is under 5 MB. A device can override the limit and name keys to skip.

## On the wire

New routes beside the save routes, so nothing an existing client sends
changes meaning:

    POST /api/units/negotiate        {device_id, families: [..], units: [{family, key, content_hash}]}
    POST /api/units                  ?family=&key=&device_id=[&overwrite=true], multipart unitFile (a zip)
    GET  /api/units                  ?family=
    GET  /api/units/{id}/content     the current zip
    POST /api/units/{id}/downloaded  ?device_id=

The rules are the save rules (`saves::plan`): one side moved since the device
last agreed, so copy it; both moved, so conflict, never resolved silently;
a first meeting of two different copies is a conflict. Upload answers 409 when
the server's copy moved since the device last agreed, exactly as
`upload_save` does. Matching copies record agreement, which the save path did
not do until dab5754. Two things differ from saves: only the `families` the
device names are planned, so a phone with no PS2 emulator hears nothing about
PS2 saves; and a unit the device agreed on before and no longer holds is a
`skip` ("deleted on this device"), not a download.

## On the server

    <saves_root>/_units/<family>/<key>/current.zip
    <saves_root>/_units/<family>/<key>/unit.json
    <saves_root>/_units/<family>/<key>/history/<unix seconds>-<hash8>.zip

The ten newest replaced copies are kept, the same count `savebackup.rs` keeps
on devices. Plain saves have no server history; units get one because
replacing a card folder loses more than replacing one `.srm`, and because of
the running-emulator problem below. Writes go to a temporary file and are
renamed into place. The agreement ledger is the existing `sync-state.json`
`seen` map, with keys `device\0unit:<family>\0<key>`.

## On the device

`src/unitsync.rs` has an adapter per family, and `config.toml` says where
each emulator keeps its saves, one `[[saves.units]]` per family:

    [[saves.units]]
    family = "ppsspp"
    root = "/storage/A2FC-A9FB/Saves/PPSSPP"       # the memory stick, holding PSP/

    [[saves.units]]
    family = "pcsx2"
    root = "/storage/A2FC-A9FB/Saves/armsx2/memcards/<card>.ps2"   # one folder card

`max_mb` raises the 64 MB limit and `skip` names folders to leave alone. A
file card, or the memcards folder instead of a card, is refused with the way
out. Reading the path from the emulator's own config (`PCSX2.ini`,
`PCSX2-Android.ini`) is not built.

A download unpacks into a temporary folder beside the target, then swaps it
in: the old folder is renamed aside, the new one renamed in, the old one
deleted. A crash leaves either the old save or the new one, never half of
each. The replaced folder is zipped to
`<library>/saves-backup/units/<family>/<key>/` first, ten kept, as
`savebackup.rs` does for files.

Units sync with a manual sync: "Sync saves" and its plan sheet, and the
CLI's `sync-saves` (whose `--dry-run` lists them). Not yet the Flip addon,
and not the per-game sync around a RetroArch launch. The plan sheet names a
PSP save by its PARAM.SFO title. A unit conflict is a `SaveConflict` named
`family/key`, so the conflict picker and `--keep` settle it like a save.

## The running emulator

moose-rack does not start standalone emulators on Android
(`commands.rs`: "this app does not start those yet"), so it cannot sync just
before a launch and just after an exit the way it does for RetroArch. PCSX2
also holds the card in memory and writes it back on its own schedule. A unit
written while the game runs can be overwritten by that write-back, and the next
sync then uploads the old copy as a change.

Milestone 1 handles this two ways. Sync leaves a family alone while its
emulator runs, where the platform can tell (`pgrep` on macOS, Linux and
KNULLI). Where it cannot (Android), the plan sheet says to close the emulator
first. The server history is what recovers a save if that is
ignored. Launching standalone emulators from moose-rack, using the
`android_packages` table `coremap.rs` already carries, would remove the
problem and is the obvious follow-up.

## Milestone 1

PPSSPP SAVEDATA and PS2 folder cards, on the Nova, this Mac and dev.lan:

1. Built: the routes, the store with history, the ledger keys
   (`src-service/src/units.rs`).
2. Built: the two adapters, the unit hash, the zip and swap
   (`src/saveunit.rs`, `src/unitsync.rs`).
3. Built: the plan sheet and CLI show units.
4. Checked in a test over a real socket
   (`the_app_syncs_a_psp_save_through_the_service`): two devices, a save
   each way, a conflict, keeping the server's copy with the device's backed
   up. Not yet checked on the Nova.

One thing to measure early: whether PCSX2 rewrites `_pcsx2_index` in a folder
when the save inside did not change. If it does, every load looks like a
change, and two devices produce conflicts that are not real ones. The fix
would be hashing without that file and carrying it anyway.

## Later

- Dolphin: GameCube folder cards (`GC/<region>/Card A/`) are one file per
  save, so they fit as units of one file. Wii saves are folders under
  `Wii/title/`.
- The Switch emulators (Eden, Citron, yuzu): saves live under a profile ID that
  differs per install, so the adapter has to translate it.
- RPCS3, Cemu, Xenia, which all have saves on the NAS already.
- Standalone save states, which are per emulator build and should stay
  separate from saves, as RetroArch states are now.

## Not covered

- PS2 file cards. An 8 MB `.ps2` file holds every game's saves, so two
  devices playing different games conflict on every sync. Of the eight PS2
  cards on the NAS, seven are file cards (from Windows, macOS, NetherSX2, and
  two in `PCSX2/memcards/`); only ChickenCard.ps2 is a folder card. PCSX2 converts a
  file card to a folder card from its memory card settings; converting is the
  route to syncing them.
- Emulators that keep saves in Android private storage
  (`/data/data/<package>`), which another app cannot read without root.

## Found on the way

Defects found while mapping the current sync, with what happened to each:

- Fixed, 43e67c3: uploads over 2 MB were stored as empty files and answered
  200. 30 save states from the Nova arrived empty on 2026-10-14. The empty
  copies were removed from the server, so the Nova uploads its own on its
  next sync.
- Fixed, 43e67c3: `upload_save` did not reduce the file name to a bare name,
  so `../` could write outside the store.
- Fixed, dab5754: a device whose copy matched the server's on first contact
  had no agreement on file, so its next change came back as a conflict.
The open ones below were found by reading the code and have not been
reproduced, except where a number says otherwise.

- Open: a per-game sync before a launch acts on every operation the server
  returns, including other games' downloads, and a resulting conflict blocks
  the launch.
- Open: different callers scan different roots on Android (`ra.root` for the
  launch sync, `cfg.saves.root` for the manual one), and a download with no
  emulator recorded lands loose in `saves/` on a by-core device. That is how
  82 of the Nova's saves ended up loose.
- Fixed with milestone 1: a manual sync never filled `pending_conflicts`, so
  the conflict picker answered "no pending conflict" unless a launch left the
  same one.
- Fixed with milestone 1: the plan sheet's first line was blank, because
  `Review` never sent its headline; only the page tests' stand-in did.
- Open: the client hashes a zip save by its contents and the server hashes
  the raw bytes, so a zipped save never matches.
- Open: the save routes take a free-text `device_id` and ignore the signed-in
  user.
