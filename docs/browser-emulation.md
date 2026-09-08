# Playing in the browser

**Planned, not built.** Written 2026-09-06, after the service became the app's
own frontend and the Play button stopped being reachable from a browser.

The web UI is the desktop UI. Everything works except the one thing you came
for: `launch_rom` spawns RetroArch on the machine running the backend, which is
a server in another room, so the service refuses it. A game opened at
`http://dev.lan` shows its art, its manual and its saves, and cannot be played.

EmulatorJS closes that: libretro cores compiled to WebAssembly, running in the
page. This is what to build and what will bite.

## The library decides most of it

Payload per game, measured on the server:

| system | games | each | in a browser |
|---|--:|--:|---|
| nes, famicom, gb, gbc, sfc, snes, genesis, gamegear, pcengine, ngp, wonderswan* | 6,900 | under 1 MB | yes |
| gba | 879 | 3.7 MB | yes |
| arcade | 2,507 | 4 MB | yes, FBNeo |
| n64 | 416 | 11 MB | shaky |
| nds | 117 | 42 MB | slow to start |
| saturn | 10 | 260 MB | no |
| psx | 127 | 302 MB | technically |
| dreamcast | 36 | 462 MB | no |
| gc | 88 | 782 MB | never — no browser Dolphin |

So **about 10,700 of 11,867 games are viable**, and the split is a property of
the library rather than of the emulator: a 302 MB disc image has to cross the
network and sit in memory before the first frame. Cartridges are the whole
value here and they are also the easy case.

**Build for cartridges. Offer discs behind a warning, or not at all.**

## Where it hooks in

`ui/js/actions.js` already funnels every play through one function, and it
already has two backends -- `launch_rom` on the desktop, `launchAndroid` on the
Thor. A third is the same shape:

    const result = MOBILE     ? await launchAndroid(id, skipSync)
                 : canLaunch  ? await invoke("launch_rom", {...})
                 : await playInBrowser(id, { entrySlot });

`canLaunch` comes from the backend, not from guessing. `status` already carries
`retroarch`; it needs one honest field saying whether this backend can start a
process for *you*, which is false whenever the viewer is not at the machine.
`viewerIsRemote()` in `ui/js/status-tag.js` already draws that line for the
corner tag and is the same question.

Everything wrapped around the launch stays: the pad guard, the conflict dialog,
the BIOS prompt, the offline prompt. They are about this window, not about the
emulator, and that is why the Android path could reuse them.

## Cores are self-hosted, not fetched from a CDN

EmulatorJS loads its cores from a CDN by default. That is wrong here twice:
this is a LAN service that has to work with the internet down, and it is the
only thing in the app that would phone out. Vendor them the way the fonts are
vendored -- `scripts/fetch-fonts.sh` pins by URL and verifies SHA-256 in a
manifest, and `LICENSES.md` records what each licence asks. Do that, not a
`<script src="https://...">`.

Budget tens of megabytes per core, served from the ROM host, cached by the
browser. Ship the cartridge cores; fetch the rest on demand.

## Saves — built, and measured before it was written

`ui/js/browser-saves.js`, through the negotiate every other device uses. The
rule is in `src-service/src/saves.rs` and is not restated: resolvable only when
exactly one side moved, conflict otherwise, never resolved silently.

**The browser is a device with its own id.** Per browser, not per account: the
guest account is shared, and the conflict rule works by asking what *this
device* last agreed with the server, which two people sharing an id cannot
answer.

Three things were measured in a real browser before any of it was written, and
two of them changed the design:

* `FS.writeFile(getSaveFilePath(), bytes)` then `loadSaveFiles()` puts a save
  into the running core, and it takes -- 8192 of 8192 bytes. That is a stronger
  result than it looks: `getSaveFile()` flushes the core's own SRAM to disk
  before reading, so what comes back is the core's memory rather than an echo of
  the file.
* **EmulatorJS's own IndexedDB copy did not survive a reload** -- 32 of 8192
  bytes. The browser is not somewhere a save can be left, which is what makes
  this worth building rather than a convenience.
* The download happens on the emulator's `start` event, deliberately *after* its
  own restore rather than racing it. I had expected to have to write before
  start and win a race; strictly later cannot lose.

Two bugs the measurements found, neither of which reading would have:

* **Save ids were too wide for a browser.** `save_id` is md5-derived, and
  `4585479350140525600` is above 2^53: JavaScript parses it to the nearest
  double and quotes back a different number, so every download 404'd. Ids are
  masked to 2^53 - 1 now, and a test asserts `id as f64 as i64 == id`.
* **An untouched cartridge is not a save.** A core that has never loaded one
  still has SRAM and it is all zeros. Reported as a save, the first sync of a
  game you played on the handheld is a *conflict* -- both sides have something,
  they differ, and this browser has agreed to nothing yet. Safe, and wrong.

Flushed every two minutes and on `visibilitychange` and `pagehide`, not only on
Stop. A tab closes without warning and a save written only on exit is a save
lost to a closed lid.

### Save states

Buttons, not a timer. A save is battery memory that changes continuously; a
state is a freeze-frame somebody made at a moment they chose, and
`/api/states` answers no conflict because one cannot be merged with another.
Taking one every two minutes would fill the shelf with moments nobody picked.

Verified end to end: pressing Save state put an 823 KB state on the server with
`emulator=snes9x`, and loading it back wiped a change made in the meantime --
which is how you can tell a restore happened rather than a no-op.

The emulator name is the core that *made* it, read from `EJS_emulator.coreName`
rather than the system name we asked for. EmulatorJS resolves `snes` to
`snes9x`, and a state belongs to the build that wrote it: restoring one into
another SNES core is a crash rather than a wrong picture.

### Never `confirm()`

It blocks the page's main thread, and here that thread is running a game. In
headless Chrome it blocks for ever, because nothing answers it -- which is how
the save sync froze the whole page the first time it met a conflict, and how
`browser-check.mjs` went from passing to hanging with no error anywhere.

`ask()` puts the question in the stage and resolves to whichever button was
pressed. The same applies to the disc-size warning.

### Shaders

Four CRT presets ship with EmulatorJS and `[shaders.by_platform]` names
RetroArch's, so `toBrowserShader` matches on the last path segment --
`crt/crt-geom`, `shaders_slang/crt/crt-geom.slangp` and `crt-geom.glslp` are one
shader named three ways. Anything with no counterpart draws none rather than a
lookalike: a handheld LCD grid is not a CRT mask, and substituting one is worse
than nothing because it looks deliberate.

**Handhelds are offered no CRT at all.** A Game Boy is a reflective LCD held a
foot from your face: it has no scanlines, no aperture grille and no curved
glass. The preference is stored per browser, so without an explicit rule a CRT
chosen for the SNES would follow you onto the Game Boy.

## Check it in a browser, not in jsdom

`node tools/browser-check.mjs http://dev.lan` drives a real Chrome: loads the
app, double-clicks a game with real mouse events at real coordinates, and
reports whether the stage opened, the canvas exists and the shader picker does
anything. `--shot out.png` saves what it looked like. Firefox can be driven the
same way over WebDriver BiDi on `--remote-debugging-port`, with
`input.performActions` for the click.

**Not having this cost a day.** The jsdom suites run the app's own modules and
prove the code executes; they have no layout, no top layer, no view transitions
and no hit-testing, and every one of those turned out to matter. Six fixes
looked right in jsdom, shipped, and did nothing.

What it found in one run:

    click    target=CANVAS  card=-10793  detail=1
    click    target=HTML    card=NONE    detail=2
    dblclick target=HTML    card=NONE    detail=2

The first click opens the detail pane, the grid reflows, and the card moves out
from under the pointer. The second click and the `dblclick` land on `<html>` --
outside the list entirely -- so the delegated handler never ran. No error, no
log, nothing to see from the server.

It also found the `confirm()` freeze above, by hanging on it.

**It runs muted.** Headless is not silent: the first run played ActRaiser's
title theme out of the speakers of somebody who had not asked for it and could
not see where it was coming from.

There was a `/play-test` route for a while, added to bisect "press Play and
nothing happens" from inside the browser. It is gone: this answers the same
question and more, and a route that exists only for debugging is a route that
stays for ever.

## What will bite

**Input.** `binds.rs` and `padpoll.rs` own the gamepad, and EmulatorJS has its
own handling. Two things reading the same pad is how a menu opens under a
running game. One has to yield while the other is live -- the pad is already
suspended around a native launch (`suspendPad()`), so the seam exists.

**Core names.** `coremap` speaks libretro (`snes9x`, `fceumm`, `mupen64plus_next`)
and EmulatorJS speaks its own dialect. A table, and a test that every platform
the server serves maps to something -- the same shape as
`every_aliased_system_has_a_name`.

**Guests.** `[auth] guest = true` lets anyone on the LAN read the library.
Playing in the browser means the ROM bytes cross to their machine. That is a
decision to take deliberately rather than discover.

**Zip.** Most cartridges are `.zip` and EmulatorJS unzips in the page. Arcade
sets must stay zipped -- FBNeo wants the archive, not its contents.

**Licence.** EmulatorJS is GPL-3.0 and the cores carry their own. `LICENSES.md`
is the place that answers this, and it has to be filled in before anything
ships, not after.

## What to build first

1. **One platform, one core, no saves.** SNES, `snes9x`, a ROM fetched from
   `/api/roms/{id}/content/{name}` and handed to EmulatorJS. This answers
   whether the whole idea works in an afternoon.
2. **`can_launch` in `status`**, and the third branch in `actions.js`. Now the
   Play button does the right thing wherever it is pressed.
3. **Saves through the existing negotiate.** With a periodic flush, not just on
   exit.
4. **The rest of the cartridge cores**, plus the core-name table and its test.
5. **Discs, or a clear refusal.** 302 MB is not a thing to start by accident:
   either a confirm with the size on it, or the button says why not.

## Explicitly not in scope

- **GameCube, Dreamcast, Saturn.** No usable browser core, and the payloads are
  hundreds of megabytes. The desktop app plays these.
- **Replacing the native path.** RetroArch is better where it is available, and
  the shaders, the rewind and the achievements all live there.
- **Netplay.**

## The desktop window plays too

Added 2026-09-08. RetroArch may have no core for a system, or not be installed
at all, and the window should still be able to run a cartridge. It is the same
player, and almost none of it needed changing -- what differs is where the files
come from and where the save goes.

**A URI scheme, not the asset protocol.** There is no HTTP server behind the
desktop window, and the 296 MB of vendored cores must not be embedded in the
binary; they are not even in git. So the window reads them off disk through
`moose://localhost/data/...`, with the ROM at `moose://localhost/rom/<id>`.
Not through the asset protocol that serves artwork: EmulatorJS builds its own
URLs by appending to `EJS_pathtodata`, and that protocol's percent-encoded
whole-path form cannot be appended to. Windows has no custom schemes in
WebView2, where Tauri maps the same thing onto `http://moose.localhost/`, so the
page asks `browser_play_base` rather than working it out -- and `null` from that
is a real answer meaning EmulatorJS was never fetched.

Three things the scheme has to get right, all of them tested in
`moose_rack::ejs`: `Range` (EmulatorJS asks for the tail of a zip before the
rest, and `bytes=-4` means the last four bytes, not the first four),
`Content-Type` (a `.wasm` served as `application/octet-stream` will not
instantiate), and `Access-Control-Expose-Headers` -- the page is
`tauri://localhost` and this is a different origin, so without it the ranges are
right and `Content-Range` reads back as `null`.

**The save is this machine's save file.** A browser on another machine is a
different device and negotiates through `/api/sync/negotiate`. The desktop
window is not: it is another emulator on the machine that holds the library, so
what it writes has to *be* the file RetroArch reads and the file `sync_saves`
sends. `local_save` and `put_local_save` read and write the local tree at the
path `savesync::download_path` gives, and the device's existing sync carries it
to the server unchanged. Negotiating here would make one machine two devices and
give one game two saves on one disk.

**Save states are not carried across.** SRAM is the cartridge's battery and any
core can read it. A state is a snapshot of one WebAssembly build's memory, and
RetroArch's snes9x cannot load one written by EmulatorJS's. The stage's state
buttons are not drawn on the desktop rather than wired to something that would
half-work.

**When it is offered.** Only when `launch::plan` says "no installed core for
platform", and then only if the system is one the browser player supports.
RetroArch is better where it is available. The dialog says what is different
about the window before anything starts.

### Measuring this, and one trap in it

Everything above was verified in the real desktop window through the
`MOOSE_MEASURE` hook, which runs a script in the page and prints what it says.
The trap cost most of a day: **a window macOS is not rendering gets no
`requestAnimationFrame`, and the page then looks like it has crashed.** Off the
side of the display -- which is where the hook puts a window by default, because
weighing the app should not throw a window at anyone -- EmulatorJS appeared to
kill the content process partway through unpacking a core, twice over, in every
configuration tried. It was doing nothing of the kind. `MOOSE_MEASURE_POS` puts
the window on the display and focuses it, and the same run decompresses the
core, starts, sizes its canvas and plays.

One real bug fell out of that: `settled()` waited on two animation frames with
nothing racing it, so pressing Play in a backgrounded tab or a minimised window
hung the launch for ever. It has a 250ms timer beside it now.

**Nothing leaves the machine.** EmulatorJS checks its own version against
`cdn.emulatorjs.org` on every game start. `scripts/fetch-emulatorjs.sh` points
that at a page-relative name so it 404s locally, and the script fails loudly if
the string it patches is not where it expects it -- a vendored file that
silently stopped matching would quietly put the call back. Checked by hooking
`fetch` for a whole launch: the only requests are `moose://`, `blob:` and
`tauri://`.

### Choosing it on purpose

The fallback above only fires when RetroArch has no core, which on a machine
with RetroArch installed is never. So the game's **Core** dropdown in the detail
pane has an entry of its own: *This window (snes)*. Picking it is remembered for
that game and `launch()` checks it before anything else, because it is a choice
rather than a fallback. Picking a real core again takes it off.

Not stored through `set_game_core`. That writes a libretro core name into
`config.toml` and every launch resolves against it, so a pseudo-core in there
would be a lie the rest of the app has to keep reading. It is a choice about
*which emulator* runs the game, not about which core RetroArch should use, and
it lives in the browser beside the shader choice.

The entry is offered even when the dropdown would otherwise say "none
installed": a system with nothing to run it is exactly what this is for.

## The Back animation, and what was actually wrong with it

Measured in the window on 2026-09-08, leaving SNES for the console screen. Two
faults, neither of them the animation itself.

**74ms with the page frozen.** A view transition holds the old snapshot on
screen for exactly as long as its callback takes, and `showPlatforms` did two
`invoke` round trips in there -- the console list and the Continue-playing strip
-- plus the whole grid rebuild. Press Back, nothing happens for 74ms, then the
name moves. `prefetchPlatforms` does the fetching before the transition starts
and the callback is down to 9ms.

**The covers.** Forty ids across the IPC boundary, forty paths resolved against
the SSD, the answer parsed, then decoded and drawn to canvas -- all on the
thread the animation is running on. Entering SNES that was 32ms a frame against
17ms with them held; leaving, a single 83ms stall in the middle of the move.
`whileMovingScreens` holds the queue and lets it go in a `finally`; the pictures
arrive a third of a second later and nobody can tell.

Out: p50 17ms, 20 frames in 343ms, nothing over 33ms. In: still around 30fps for
its 300ms, and there is no backend call during it -- that one is compositing two
full-page snapshots, and is not fixed.
