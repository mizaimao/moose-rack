# moose-patch audit, 2026-09-24

**Record, 2026-09-24.** What was wrong with the handheld addon at 0.4.312, found
by three code reviews and checked on the Flip. A work list, not a description
of how things are: strike an item when it is fixed, delete the file when the
list is empty.

"Live" means it is happening on the Flip today. "Latent" means one ordinary
event away — a Wi-Fi drop, a KNULLI update, a renamed ROM. "Confirmed" means the
code path was followed end to end or reproduced; "suspicion" means it was not.

## Live today

1. ~~**Favourites never reach the server.** moose-service registers only GET
   routes for collections (src-service/src/main.rs:1277-1280). The Flip POSTs
   and DELETEs `/api/collections/{id}/roms` (src/api.rs:679), gets 404, and
   `favrun.rs:266` then skips the whole collection, card side included. A
   single star added on the Flip wedges that list in both directions for good.
   docs/flip-knulli-changes.md says both kinds of list sync both ways; they have
   not since RomM was replaced. Confirmed.~~
2. ~~**The on-screen apply skips the KNULLI version check.** Only `--apply` and
   `--restore` are gated (main.rs:334, 517); `run_queue` (main.rs:705-721)
   applies on any image. Confirmed.~~
3. **"I took this save" goes nowhere.** `confirm_download` posts to
   `/api/saves/{id}/downloaded`, which moose-service does not have, and ignores
   the status (api.rs:931-940). The server records agreement only on upload.
   After any pulled save changes, the next sync calls it a conflict;
   `--keep server` never sticks; a save deleted on the Flip comes back every
   sync. worker.rs:347 and main.rs:223 still describe RomM's rule. Confirmed.
4. **Sync failures are swallowed.** `carry_out` passes on only the headline
   (worker.rs:268-272): failed transfers, "save states did not sync" and rename
   failures are dropped. A run where everything failed reads "in sync" and
   exits 0. Confirmed.
5. **Conflicts are never drawn on screen.** `app.conflicts` is held and not
   shown, so `--sync --keep` over ssh is the only way to answer one. Confirmed.
6. **States only go up.** `statesync::run` walks local files only
   (statesync.rs:201-208), so a state the Flip lacks never comes down; the plan
   covers saves only (worker.rs:441-496), so when saves agree states cannot be
   synced from the screen, and when they differ states move without being
   shown. `--pull-all` is saves only. Confirmed.
7. **Neo Geo saves never sync.** `split_slot` accepts `.srm`, `.sav` and
   `.state` only (saves.rs:131-161); geolith's `Game.zip#Game.mcr` cards and
   `.nv` files are never seen. Confirmed.
8. **Games the server does not hold have no backup.** Tintin: a save and 12+
   states exist only on the Flip. It is on the SSD and the card, not in the
   server's library. Device.
9. **The log records presses, not outcomes.** moose-patch.log has 279 "press
   Down" and no line saying what a sync, apply or refresh did. Device.
10. **The Flip logs in with the token**, against the standing rule, because the
    server has no `[[auth.users]]` account to use instead. Device.

## Latent, and each one loses data

11. **A partial game index overwrites saves.** A save the Flip cannot match is
    not reported (savesync.rs:174-181); the server then offers its copy as
    "this device does not have this save" (src-service/src/saves.rs:139) and
    the download lands on the local file with a backup but no conflict.
    `--refresh` deletes the index before it has the new one (worker.rs:309),
    the pull commits page by page, and `prepare` never checks the index is
    complete. A Wi-Fi drop during a refresh, then a sync, overwrites every save
    in the missing systems. Confirmed.
12. **A save name in two system folders uploads the wrong bytes.** The upload
    and conflict branches find the local file by bare name across all folders
    (savesync.rs:591, 629). 374 names exist in more than one system on the
    server. None collide on the Flip today. Confirmed.
13. **`--keep server` on a state conflict loses the state.** It writes to
    `destination(emu, None)` (statesync.rs:398-402), which on KNULLI is a folder
    nothing reads, then re-uploads the rejected local state over the server's,
    which keeps no old version. Confirmed.
14. ~~**One unreadable byte wipes a config.** `fs::read_to_string(file)
    .unwrap_or_default()` (patch.rs:427) turns any read error — one non-UTF-8
    byte, an EIO — into an empty file, and the write leaves only our block:
    Wi-Fi, cores, everything in knulli.conf gone. Same for custom.sh and the
    hotkey file. All three are valid UTF-8 today. Confirmed by probe.~~
15. ~~**A missing seed kills the system keys.** A failed seed copy is ignored
    (patch.rs:422-426), so hotkey-app ON on an image where
    `/etc/triggerhappy/triggers.d/multimedia_keys.conf` moved creates the
    /userdata file with only our two lines; it replaces /etc wholesale, and
    volume, power and lid stop. Confirmed by probe.~~
16. ~~**hotkey-app "off" leaves a frozen copy.** The seeded /userdata file stays
    (catalogue.rs:464-469) and shadows every later KNULLI's version, patch on
    or off, with no apply for the version check to catch. It matches this
    image today. Confirmed; the update effect is a suspicion.~~
17. ~~**The boot hook has no version check.** With `gpu=wayland` every boot copies
    the old blob over a new image's libmali (boot-custom.sh:19-28). Not live:
    gpu is stock. Suspicion that a newer driver would clash.~~
18. ~~**Favourites, once the 404 is fixed.** The baseline records server members
    not on the card (favsync.rs:96-97), so a game that lands later reads as an
    unstar; a list that agrees keeps its stale baseline forever (favrun.rs:181);
    an error loading a `custom-*.cfg` reads as an empty list (favrun.rs:223) and
    becomes a mass unstar; and ES rewrites gamelists from memory on exit,
    undoing whatever the sync wrote. Matching ignores `rel_dir` (favmap.rs:48-58),
    so games in subfolders and same-name twins misbehave. Confirmed.~~
19. ~~**The profile drops patches that drifted.** "changed" patches are left out
    silently (profile.rs:47-51) and the profile is rewritten after every
    on-screen apply, so an ES rewrite of never-sleep can drop it from the
    profile a reflash restores from. Confirmed.~~
20. ~~**es-logo off deletes the stock logo.** "off" means "file absent" for a
    stock file (catalogue.rs:501-504, patch.rs:458); a fresh install reads
    "changed", and a second off deletes logo.png until reboot. Confirmed by
    probe.~~
21. ~~**Patch state reads our block, not KNULLI's reader.** patch.rs:401-407
    never asks `knulli-settings-get`, so ES writing its old value back after an
    "off" leaves the row reading off while the device does the opposite.
    Confirmed.~~

## Wrong or misleading, no data at stake

22. One job slot: starting favourites during a save sync orphans the save job
    and Status sticks on "syncing" (main.rs:650-665).
23. With a save plan held, the confirm dialog shows it on any row; A on
    "Refresh the game list" rebuilds the index instead (ui.rs:230, 345).
24. ~~The favourites second press re-plans instead of running the plan shown
    (main.rs:663).~~
25. A failed `rom_with_files` is dropped (savesync.rs:775-787), so a pulled
    save lands at the top of /userdata/saves where nothing reads it.
26. `--keep=local` is ignored without a word (main.rs:270).
27. ~~After an on-screen apply the row shows the option picked, not a read-back
    (model.rs:193); `--apply` exits 0 on "changed".~~
28. ~~`shaders=off` deletes sets that `shader-gba/gb/gbc` still name.~~
29. Stale text: "a rescan renumbers ids" (rows.rs:26-29, worker.rs:282-291).

## Saves the Flip no longer loads, or cannot match

- Stranded by ROM renames: Apotris (save v4.0.2, ROM v4.1.0), Goodboy Galaxy
  (v1.2 vs v1.3), Skyland (`(Proto)` vs `(Proto 5)`). RetroArch reads the name
  the current ROM gives, so this progress is not loaded in game. Renaming the
  save to the ROM's stem restores it and makes it syncable.
- No ROM on the card: the two Super Mario World hacks.
- Live but unmatched: `Inky and the Alien Aquarium …(Unl).gba.srm` (ROM is
  `….gba.zip`; the server's name differs). `Metroid Fusion (USA, Australia)`:
  the server only has `(USA)`; matching by ROM hash would find it.
- Syncthing `*.sync-conflict-*.srm` leftovers: should be ignored, not counted.
- The `gba-backup-vbam-20260828` folder is scanned as a system.

## Checked on the device and fine

All 20 patches read back at a known setting. 1,259 of the Flip's 1,286 states
and every matched save are byte-identical on the server. knulli.conf, custom.sh
and the hotkey file are valid UTF-8. No save name sits in two system folders.
No server save carries a RomM stamp. The /userdata hotkey file matches this
image's /etc copy apart from our lines.

## Done in the same round

"Take games offline" is gone from the Flip's sync tab, on Frank's word: pulling
ROMs is too heavy for the device, and the card is filled from the SSD.
