# DOS games

**Tested by hand, not built.** Written 2026-09-17 on the `moose-dos` branch.
One eXoDOS game was fetched, packed and played in DOSBox Pure through RetroArch
on the Mac. None of it is in the app yet. Work moved on to Windows 98 games
before any code was written, so this is the record to start from.

## What was decided

- The source is eXoDOS, used as a resource and never run. Its launchers,
  LaunchBox and bundled emulators are Windows-only; everything we need from it
  is zips, text and a torrent, which any machine can read. It lives on the
  server because the library and the service already do.
- The emulator is DOSBox Pure, on the desktop through RetroArch and, later, in the
  browser through EmulatorJS (whose DOS core is the same one). One emulator
  means one config adapter and one save file for both.
- The fallback is DOSBox Staging, per game, for what Pure cannot run, and not
  DOSBox-X: eXoDOS moved all but a few of its DOSBox-X, Daum and custom-build
  games to Staging in 2024 ([eXoDOS#5148](https://github.com/exoscoriae/eXoDOS/issues/5148)),
  and 18 games are left on X.
- eXoDOS's per-game emulator choice is used as a hint. It names Windows
  builds, and ECE (2,100 games) has no Mac build.

## eXoDOS Lite on the server

`dev.lan:/home/frank/moose-library/eXoDOS`: version 6.04 Lite, 11 files,
6.6 GB, checksums verified against the copy it came from. Kept there.

Lite holds the metadata for 7,666 games and one game. The rest are fetched one
at a time from the full collection's torrent (697.6 GB, 14,011 files):

| where | what |
| --- | --- |
| `eXo/util/util.zip` -> `OPTDOS.zip` -> `aria/eXoDOS.torrent` | the full torrent; copied out to `eXo/util/aria/` |
| `eXo/util/util.zip` -> `dosbox.txt` | the DOSBox build eXoDOS picked per game |
| `Content/!DOSmetadata.zip` -> `eXo/eXoDOS/!dos/<short>/dosbox.conf` | the per-game config |
| `Content/XODOSMetadata.zip` -> `xml/all/MS-DOS.xml` | LaunchBox metadata, 37 MB, and artwork by type |
| `eXo/eXoDOS/<Title (Year)>.zip` | a downloaded game; files under `<short>/` inside |
| `Content/GameData/eXoDOS/<Title (Year)>.zip` | that game's extras. DOOM's is 1.0 GB against a 13.5 MB game |

Read the torrent itself for file indices rather than `aria/index.txt`, which
eXo builds by hand-editing `aria2c --show-files` output. A game is fetched the
way eXo's `install.bat` does it:

    aria2c --select-file=<index> --index-out=<index>="<Title (Year)>.zip" \
           --file-allocation=none --seed-time=0 eXoDOS.torrent

DOOM took about a minute from 8 seeders. `aria2c` is installed on the server.

Builds named in `dosbox.txt`: DOSBox 0.74 4,838, ECE 2,100, Staging 707,
DOSBox-X 18, Daum and custom 4.

## Packing a game for DOSBox Pure

One file per game: the eXoDOS zip unchanged, plus `DOSBOX.CONF` at its root.
Pure reads that file when its core option `dosbox_pure_conf` is `inside`
(default `false`). Whatever the conf sets is fixed for the session; lines Pure
has no setting for (`[sdl]`, eXo's MIDI paths) are skipped. Any `[autoexec]`
line skips Pure's start menu. This is from `init_dosbox_load_dosboxconf` in
Pure's source, and it held for DOOM.

The loaded zip already is C:, so eXo's own mount has to go:

| eXo writes | becomes |
| --- | --- |
| `mount c .\eXoDOS\DOOM` (with or without `@`) | removed |
| `c:` | `c:` then `cd \DOOM` |
| `imgmount d .\eXoDOS\dune2\cd\dune2-tbod.cue -t cdrom` | **untested.** Pure mounts the first CD image it finds on C: by itself, which suggests dropping the line |

eXoDOS's `run.bat` often asks questions before the game starts. DOOM's asks
Y/N, then offers a sound card menu. That needs a keyboard, or Pure's on-screen
one from a pad. Some games also have an `exception.bat` that eXo's launcher
runs instead. Dune II's wraps DOSBox in a mouse helper `.exe`.

## Launching: three things that were wrong, all fixed by config

Found by playing DOOM on the Mac. Each fix was confirmed by Frank.

1. The game ran far too fast. It was not frame pacing: 1,400 frames took
   20 s, which is real time at DOOM's 70 Hz, on a 120 Hz display, with audio
   muted or not. RetroArch's keyboard hotkeys were still live, and the log
   recorded `[DBP THROTTLE] NONE 70.08 -> FAST_FORWARD`. Space is fast-forward
   and is DOOM's use key; Esc quits RetroArch and F1 opens its menu. The launch
   config for a DOS game unbinds every keyboard hotkey and keeps F12 for the
   menu.
   The app's pad hotkey block does not cover this. It gates hotkeys behind the
   pad's modifier only when a pad profile is found, and a DOS game is usually
   played with no pad at all.
2. RetroArch said "The frontend MIDI output is not set up correctly". Pure's MIDI default
   is the first soundfont in RetroArch's system directory, and RetroArch's own
   MIDI driver when there is none. eXo's `run.bat` also sets `mididevice` with
   `CONFIG -set`. Put `SoundCanvas.sf2` (47 MB, `EXTDOS.zip` -> `mt32/`) in the
   system directory and set `dosbox_pure_midi = "SoundCanvas.sf2"`. The log
   then reads `MIDI: Opened device:tsf`.
3. The mouse did nothing. RetroArch does not capture the mouse in a window. Set
   `input_auto_mouse_grab = "true"`, which grabs it whenever the window has
   focus. DOOM's `DEFAULT.CFG` already has `use_mouse 1`.
   Game Focus would also grab the mouse and pass the keyboard through, but
   RetroArch only switches it on automatically when its menu closes
   (`retroarch_menu_running_finished`), so a command-line launch cannot rely on it.

Mouse latency felt high over Bluetooth and was not measured. The known
contributors are DOOM reading the mouse once per tic at 35 Hz, RetroArch
polling once per frame, and three Vulkan swapchain images in the log
(`video_max_swapchain_images` would take that to two).

## Saves

Pure writes `<savefile_directory>/DOSBox-pure/<content stem>.pure.zip`: the
files the game changed, as a zip, with the original left untouched. Save sync
knows `.srm`, `.sav` and `.state` only, so it would miss these without a word.

## Not DOS, but found here

The libretro buildbot's Apple Silicon build of Pure has no dynamic recompiler.
Its CPU core option offers only "Normal (interpreter)", although Pure's source
has an ARMv8 dynrec backend. DOS games are fine on it. Windows 9x games on a
Pentium II are not, which is why those went to DOSBox-X.

## Not built

In the order they would be needed:

1. `dos` in `data/esde-core-map.json` and `tools/extract_esde_cores.py`.
   Upstream ES-DE's entry has DOSBox-Pure first, then DOSBox-Core, DOSBox-SVN
   and VirtualXT.
2. The launch config for `dos`: the core options and the three fixes above.
3. The importer, on the server: catalogue from `MS-DOS.xml`, fetch by torrent
   index, pack the zip with its adapted conf into `ROMs/dos/`, write the
   gamelist and artwork.
4. A game that is in the catalogue but not downloaded: a third state beside
   "on this machine" and "on the server".
5. Save sync for `.pure.zip`.
6. The browser, through EmulatorJS. Check first that the pinned 4.2.3 archive
   carries `dosbox_pure`.
7. DOSBox Staging as a per-game fallback on the desktop, which needs a launch path that is
   not RetroArch.

## Testing without touching the machine

A test run of RetroArch with its own `--config` in `/tmp` still writes to
`~/Library/Application Support/RetroArch`. With no
`bundle_assets_extract_last_version` in that config, RetroArch re-extracts its
bundled assets there: about 13,000 files on 2026-09-17, from the same app
version. Put `bundle_assets_extract_enable = "false"` in every test config,
along with the directories (`savefile_directory`, `system_directory`,
`core_options_path`, `playlist_directory`, `cache_directory` and the rest),
and `history_list_enable = "false"`.

`--max-frames=N --max-frames-ss --max-frames-ss-path=<png>` runs N frames and
takes a screenshot. That is how a boot is checked without anyone at the
machine. With the Vulkan driver the first screenshot failed to save; a second
run saved one.

## Working with the other session

A second Claude session works on `main` from another Mac and also deploys the
service. Agreed with it on 2026-09-17:

1. Whoever deploys to `dev.lan:/home/frank/moose-rack-src` writes
   `DEPLOYED` there (branch, commit, date, session) and reads it first. A
   deploy must contain the commit it names.
2. `moose-dos` rebases on `origin/main`; conflicts are resolved on
   `moose-dos`, never by rewriting `main`.
3. The version number: on rebase, `moose-dos` takes `main`'s and bumps on top.
4. The DOS side writes only `eXoDOS/`, `ROMs/dos/`, `ES-DE/gamelists/dos/`
   and `ES-DE/downloaded_media/dos/` under `moose-library`. The other session
   owns `ES-DE/saves/`.
5. Game ids come from `Game.system`, `Game.rel_dir` and `Game.fs_name`
   (`src/gameid.rs` on `main`). Nothing here may change how the scanner
   derives those for games that already scan.
