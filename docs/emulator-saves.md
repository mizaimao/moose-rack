# Standalone emulator saves

**Design, 2026-10-15. Not built.** Tracked in Linear MOO-25. How saves kept by
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

For a PS2 folder card the key is `<card name>/<folder>`, because a game can
have a save on two cards. Card-level files at the card's root (`_pcsx2_*`)
stay on the device; everything inside a game's folder travels.

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

    POST /api/units/negotiate        {device_id, units: [{family, key, content_hash, updated_at, size}]}
    POST /api/units                  ?family=&key=&device_id=[&overwrite=true], multipart unitFile (a zip)
    GET  /api/units                  ?family=
    GET  /api/units/{id}/content     the current zip
    POST /api/units/{id}/downloaded  ?device_id=

The rules are the save rules (`saves::plan`): one side moved since the device
last agreed, so copy it; both moved, so conflict, never resolved silently;
a first meeting of two different copies is a conflict. Upload answers 409 when
the server's copy moved since the device last agreed, exactly as
`upload_save` does. Matching copies record agreement, which the save path did
not do until dab5754.

## On the server

    <saves_root>/_units/<family>/<key>/current.zip
    <saves_root>/_units/<family>/<key>/history/<UTC time>-<hash8>.zip

The ten newest replaced copies are kept, the same count `savebackup.rs` keeps
on devices. Plain saves have no server history; units get one because
replacing a card folder loses more than replacing one `.srm`, and because of
the running-emulator problem below. Writes go to a temporary file and are
renamed into place. The agreement ledger is the existing `sync-state.json`
`seen` map, with keys `device\0unit:<family>\0<key>`.

## On the device

An adapter per family finds the folders and knows the format:

- `ppsspp`: the memory stick root, where `PSP/SAVEDATA/` lives. On the Nova
  that is `/storage/A2FC-A9FB/Saves/PPSSPP`.
- `pcsx2`: the memcards folder, with each folder card in it a source of
  units. On the Nova that is `/storage/A2FC-A9FB/Saves/armsx2/memcards`.

The adapter reads the emulator's own config for the path where it can
(`PCSX2.ini`, `PCSX2-Android.ini`, `ppsspp.ini`), and `config.toml` can name it
outright:

    [[units]]
    family = "ppsspp"
    root = "/storage/A2FC-A9FB/Saves/PPSSPP"

A download unpacks into a temporary folder beside the target, then swaps it
in: the old folder is renamed aside, the new one renamed in, the old one
deleted. A crash leaves either the old save or the new one, never half of
each. `savebackup.rs` keeps the replaced folder first; it handles single files
today and needs a folder case.

Units sync whenever saves do: "Sync saves", the plan sheet, the CLI's
`sync-saves`, and the Flip addon. The plan sheet lists them by the title in
PARAM.SFO or `icon.sys` rather than the folder name.

## The running emulator

moose-rack does not start standalone emulators on Android
(`commands.rs`: "this app does not start those yet"), so it cannot sync just
before a launch and just after an exit the way it does for RetroArch. PCSX2
also holds the card in memory and writes it back on its own schedule. A unit
written while the game runs can be overwritten by that write-back, and the next
sync then uploads the old copy as a change.

Milestone 1 handles this two ways. Sync refuses to write into a card or memory
stick whose emulator is running, where the platform can tell (desktop:
the process list). Where it cannot (Android), the plan sheet says to close the
emulator first. The server history is what recovers a save if that is
ignored. Launching standalone emulators from moose-rack, using the
`android_packages` table `coremap.rs` already carries, would remove the
problem and is the obvious follow-up.

## Milestone 1

PPSSPP SAVEDATA and PS2 folder cards, on the Nova, this Mac and dev.lan:

1. Server: the routes, the store with history, the ledger keys, tests in
   the style of the save tests.
2. Core: the two adapters, the unit hash, the zip and swap, a unit case in the
   plan and in `run_all`.
3. The plan sheet and CLI show units.
4. Checked end to end: change a save on the Nova, sync, see it on the Mac;
   then the reverse; then a deliberate conflict.

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
- Open: a manual desktop sync never fills `pending_conflicts`, so the
  conflict picker answers "no pending conflict" unless a launch left the same
  one.
- Open: the client hashes a zip save by its contents and the server hashes
  the raw bytes, so a zipped save never matches.
- Open: the save routes take a free-text `device_id` and ignore the signed-in
  user.
