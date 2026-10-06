# DOS games

**First pass built, 2026-10-05, on `moose-dos` (0.5.1).** DOOM and The
$100,000 Pyramid are in the library and start from the app on the desktop and
in the browser. The 2026-09-17 record below is what the build followed; [What
the first pass built](#what-the-first-pass-built) says what changed, and [Not
built](#not-built) what is still open.

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

## What the first pass built

- `dos` in `data/esde-core-map.json`, ES-DE's upstream entry with DOSBox-Pure
  first. Only that entry was added: regenerating the file with
  `tools/extract_esde_cores.py` drops hand-made additions (the Android
  packages, the easyrpg, wii and saturn defaults), so the generator only knows
  `dos` for next time.
- `tools/exodos_pack.py`, run on the server. It packs a game that is already
  downloaded: the eXo zip with `DOSBOX.CONF` written first, its gamelist entry
  from `MS-DOS.xml`, and eight kinds of artwork. 6,187 of the 7,666 configs
  adapt. 1,438 mount a CD image and 41 never switch to C:, and those are
  refused until someone tries one.
- The desktop launch (`src/tweaks.rs`, `src/keyboard.rs`): `dosbox_pure_conf
  = inside`, MIDI on `SoundCanvas.sf2`, all thirty default keyboard hotkeys
  unbound, the menu on F12 and the mouse grabbed. No shader by default: VGA
  drew 200-line modes twice, so there were never TV scanlines.
- `SoundCanvas.sf2` is a `dos` BIOS (`data/bios-manifest.json`, and `EXTRA` in
  `tools/bios_manifest.py`), so the launch fetches it like any other. It sits
  in `moose-library/bios/`, copied out of `util.zip` -> `EXTDOS.zip` ->
  `mt32/`. That folder is outside the four the agreement below gives the DOS
  side; it is one file, and this is where it is recorded.
- The browser: `dos` plays through EmulatorJS when the page is cross-origin
  isolated, and is refused with the reason when it is not.

Checked: the desktop through the app's own launch planner and RetroArch, with
a screenshot at eXo's menu and `MIDI: Opened device:tsf` in the log; the
browser through the deployed service in headless Chrome, into DOOM's demo after
pressing 3, and Pyramid to its title screen. macOS refuses synthetic
keystrokes from a terminal, so nothing past the menu was driven on the desktop.

### What the browser needed

Three things, none of which the desktop needs:

1. **Threads.** The pinned EmulatorJS builds Pure with threads only
   (`requiresThreads`), and refuses it without `EJS_threads`. Threads need
   `SharedArrayBuffer`, which a browser gives only to a page that is
   cross-origin isolated (the service now sends COOP and COEP on everything
   the web app serves) *and* a secure context: HTTPS, or localhost. Over
   `http://dev.lan` the headers are ignored, so DOS does not play there.
   Through `ssh -L 8080:127.0.0.1:80 dev.lan` and `http://localhost:8080` it
   does.
2. **The C: drive.** EmulatorJS unpacks the zip into its filesystem root and
   hands the core the first file with an extension it lists for the core, and
   for `dos` it lists only `conf`. Pure, given a `.conf` alone, runs it like
   plain DOSBox and mounts nothing (`init_dosbox`, "Loading a .conf file
   behaves like regular DOSBox"). So the packed `[autoexec]` keeps a mount,
   guarded: `@if not exist C:\DOSBOX.CON mount c /`. On the desktop C: is the
   zip already and the line does nothing. Pure's own `automount = true` mounts
   the same folder and crashes the core ("memory access out of bounds").
3. **`DOSBOX.CONF` first in the zip.** EmulatorJS takes the first `.conf` it
   unpacks. Appended at the end, ours would lose to any `.conf` a game ships.

The browser has no soundfont, so Sound Canvas music is silent there. The
SoundBlaster choice in DOOM's menu plays FM music without one.

## Not built

In the order they would be needed:

1. HTTPS on dev.lan, without which the browser half works only on localhost.
2. Fetching by torrent index in `tools/exodos_pack.py`. It packs only what has
   already been downloaded (three games so far).
3. Games that mount CD images: 1,438 of them, Dune II among them. Pure mounts
   the first CD image it finds on C: by itself, which suggests dropping the
   line, untested.
4. Save sync for `.pure.zip`. In the browser Pure keeps no save at all: a
   `.conf` loaded alone runs without Pure's save file.
5. A game that is in the catalogue but not downloaded: a third state beside
   "on this machine" and "on the server".
6. DOSBox Staging as a per-game fallback on the desktop, which needs a launch
   path that is not RetroArch.

## Testing without touching the machine

The browser half can be tried without the app: a page that sets the same
`EJS_*` values, served with COOP and COEP from 127.0.0.1, loads a test zip in
headless Chrome, and `Input.dispatchKeyEvent` types into the game. That is how
the guarded mount was found, against three other layouts.

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
