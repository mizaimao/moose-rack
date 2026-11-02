# The server, the web page and the desktop app

**Design, agreed with Frank on 2026-11-01.** Parts already exist; the table
under "The pieces" says which. The open questions at the end are still open.

## The shape

- The NAS holds the one real library: ROMs, artwork, game lists, saves.
- `moose-service` (on dev.lan) serves it. Its web page browses the library and
  plays the lighter games in the browser.
- The desktop app (macOS and Windows first, Linux later) is a front end in its
  own right. It reads from a local cache and fills the cache from the service
  only when something is missing. Deleting the cache costs a re-download and
  nothing else; the server never depends on it.
- The desktop app also takes Play requests from the web page for games the
  browser can't run (sixth generation and later).

One user for now: the owner. The service already knows an owner token and
named users (`[[auth.users]]`, see [service-api.md](service-api.md)), so adding
people later needs no redesign.

## How it gets used

Examples, not a complete list. The first three are Frank's; the rest follow
from the same design and have to work too.

1. A friend visits. They open the web page and play a simple game in the
   browser, with pad mapping and shaders preset. Built.
2. I find an Xbox 360 game on the web page and press Play. The page hands the
   game to the Moose Rack app on that computer, which makes sure the emulator
   is installed and set up, downloads the ROM if the cache lacks it, and
   starts the game.
3. While the game runs, the app stays open, as Steam does. When the game
   quits, the app finds new or changed saves and uploads them. Quitting the app
   while an upload is unfinished asks first.
4. I open the desktop app directly, with no web page, browse and play. Same
   steps as 2.
5. The server is unreachable (away from home, dev.lan down). Everything in the
   cache still plays. Save uploads wait and go when the server is back; games
   not in the cache say they need the server.
6. A new computer: install the app, sign in to the service, done. Emulators and
   ROMs arrive on first play.
7. The cache is deleted, or the disk is full. Nothing is lost: saves are on the
   server. When space runs short the app offers to remove the ROMs played
   least recently, never unsynced saves.
8. A game this OS can't run, such as Xbox 360 on the Mac. The web page and the
   app say so and name the OS that can, instead of offering Play.
9. The web page on a phone or a friend's laptop without the app. Browser games
   play; the rest say they need the desktop app.
10. I play on the Mac, then continue on Windows or the Nova. The next device
    downloads the newer save before the game starts. If both changed it, the
    app stops and asks which to keep.
11. An emulator has a newer release. Play offers the update; declining plays
    the installed version.
12. The handhelds (Nova, Flip) keep their own setups (ES-DE, moose-patch) and
    share saves through the same service. They are not part of this design
    beyond that.

## The pieces

| piece | status |
|---|---|
| Web library and in-browser play (EmulatorJS, DOS, Windows 9x) | built |
| Desktop cache, ROMs downloaded on demand | built, RetroArch games |
| Saves uploaded after a game exits | built for RetroArch; standalone emulators at milestone 1 ([emulator-saves.md](emulator-saves.md)) |
| Play link from the web page to the app | built; on macOS a link went to the app, downloaded the game and started RetroArch (2026-11-01). Windows not tried yet; the web page change not deployed yet |
| App stays open while a game runs; asks before quitting with an upload pending | built (`src-tauri/src/busy.rs`). On macOS, closing the window during a game was refused and worked once it ended (2026-11-01). The upload question and Cmd+Q are covered by tests only; the Dock's Quit can't be stopped |
| Standalone emulators installed and set up per OS | PCSX2 on macOS built (2026-11-01); the rest not started. See "Standalone emulators" below |
| Lock a game while another device plays it | deferred |
| Server keeps copies of emulator updates | not doing |

## The play link

The web page's Play button for a game the browser can't run links to

    moose-rack://play/<platform>/<file name>

The OS hands any `moose-rack://` address to the installed app, the way
`steam://run/...` reaches Steam. The browser asks once ("Open Moose Rack?").
The service never contacts the desktop, so the desktop opens no port and the
link works away from home too.

- Platform and file name, URL-encoded, name the game. Service row ids are scan
  order and change when the library changes (see the top of
  `src-service/src/main.rs`), so they can't go in a link.
- Registered with Tauri's official deep-link plugin on macOS and Windows.
- Any web page can fire a `moose-rack://` link. The app only accepts `play`
  for a game in its own library listing and never runs anything taken from the
  link itself.
- If the app isn't installed, nothing happens. After a few seconds the page
  says so.
- On the desktop the link is handled by the app's own Play (`play` in
  `ui/js/detail.js`): download if needed, then launch. A game the app's cache
  doesn't know yet triggers a library update first.

Code: `src/playlink.rs` reads the link, `src-tauri/src/lib.rs` receives it
(`take_links`, `take_play_link`), `ui/js/playlink.js` builds it on the web page,
and `watchPlayLinks` in `ui/js/actions.js` plays it. The app logs
`play link: <platform>/<file>` to stderr when one arrives.

Found while testing on macserver: a fresh build opened through Finder or
`open` couldn't reach `dev.lan` ("No route to host", os error 65) while `curl`
on the same machine could. That fits macOS asking "allow Moose Rack to find
devices on your local network?" on screen, with nobody there to click it.
Started from a shell, the same build connected. On a machine where someone
answers that prompt once, it doesn't come back.

## Play, step by step

1. Emulator for this platform on this OS: installed and set up? If not, offer
   to install it from its official release, pinned to a known version, with
   Moose Rack's settings for it.
2. Firmware or keys it needs: present? If not, fetch them from the NAS `bios`
   folder.
3. ROM in the cache? If not, download it, with progress.
4. Launch, fullscreen, as a separate process ([PLAN.md](../PLAN.md) §10).
5. Wait for it to exit. The app can't be closed meanwhile.
6. Find changed saves and upload them.

## The cache, per OS

Each OS gets its own complete set, because the emulators differ (Xenia runs
only on Windows):

    <Moose Rack folder>/
    ├── emulators/   one folder per emulator, portable mode where it has one
    ├── roms/<platform>/
    ├── media/<platform>/
    ├── saves/       working copies; the server holds the real ones
    └── bios/

Today's layout ([PLAN.md](../PLAN.md) §11) is `library/roms` and
`library/downloaded_media` beside the app; moving to this is part of the work.

Emulators per system, as a starting point:

| system | macOS | Windows |
|---|---|---|
| PS2 | PCSX2 | PCSX2 |
| GameCube, Wii | Dolphin | Dolphin |
| Wii U | Cemu | Cemu |
| Xbox | xemu | xemu |
| Xbox 360 | none | Xenia Canary |
| PS3 | RPCS3 | RPCS3 |
| Switch | Eden (macOS build to check) | Eden |
| PSP | PPSSPP | PPSSPP |
| 3DS | Azahar | Azahar |

ARMSX2 is Android-only; the Mac uses PCSX2.

## Standalone emulators

`src/standalone.rs`. Each one installs into `<library>/emulators/<id>/` in
its own portable mode, at a version pinned in that file. A launch that finds
it missing returns `EMULATOR_MISSING:`, and the page asks "Install and play?".

PCSX2 (PS2), first, on 2026-11-01:

- v2.8.2, the official release. Its macOS build is Intel-only, so Apple
  silicon runs it through Rosetta. ARMSX2 has a native Mac build, but only as
  nightlies; switch when it has stable Mac releases. Both read the same cards.
- `portable.txt` beside `PCSX2.app` keeps its settings, BIOS choice and cards
  in its folder (`EmuFolders::SetAppRoot` in PCSX2).
- The app owns a set of keys in `inis/PCSX2.ini` (`standalone::settings`) and
  leaves the rest. `[UI] SettingsVersion = 1` is one of them: without it PCSX2
  offers to reset every setting on start.
- Pads bound by position, south button Cross. Hold Select as in RetroArch
  games: bottom face quits, top face opens the menu, shoulders load and save a
  state, right trigger fast-forwards.
- BIOS: the server's `bios/PS2` files are fetched into `<system dir>/PS2`;
  SCPH-39001 (US) is used when it's there.
- Saves: one folder card, `memcards/Moose.ps2`, synced as `pcsx2` units
  before and after each game and in "Sync saves" (`commands::unit_cfgs`).
  PCSX2's own new folder card is unformatted, so every game would ask to
  format it; the app writes a formatted header instead
  (`data/pcsx2-folder-card-superblock.bin`, taken from ChickenCard.ps2).
  Slot 2 is off, so no save lands on a card that doesn't sync.
- Started as `PCSX2 -nogui -fullscreen -- <rom>`; the app waits for it.

Tested on macserver: a play link for Crazy Taxi asked to install PCSX2,
installed it, fetched 17 BIOS files, and booted the game on the US BIOS with
the folder card in slot 1. Windows not tried yet.

Found: the PS2 was missing from the library. `ROMs/ps2` on the NAS had a
`noload.txt` (ES-DE's switch-off marker), as do `ps3`, `switch`, `wii`,
`wiiu`, `xbox360` and `n3ds`, all dated 2026-10-14, so the service skipped
them. The Crazy Taxi test used a local copy. On 2026-11-01 Frank had the PS2
turned back on: the marker is now `noload.txt.masked-off`, and after a service
restart the PS2 lists 123 games. The others are still off. `xbox` has no marker but no
platform mapping either.

## Order of work

1. The play link, with RetroArch games, since their launch path already works.
2. Stay open while a game runs, and ask before quitting with an upload pending.
3. Standalone emulators, one at a time, each with install, settings, launch and
   saves before the next starts.

## Open questions

- Which emulator comes first.
- Emulator settings: Moose Rack writes its own known-good settings, as it does
  for RetroArch, or uses whatever the emulator has. Leaning towards its own.
- Updates: pinned versions, with an "update available" offer at Play time,
  checked against the official release page. No copies on the server.
- Disk: Switch and PS3 games run to tens of GB. How much the cache may use, and
  what gets removed first when it's full.
- Locking a game in use on another device: decide while building save sync for
  standalone emulators.
- What a save conflict looks like to the user when two devices both changed it.
