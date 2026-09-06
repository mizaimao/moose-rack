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

## Saves are the hard part, and the rules already exist

EmulatorJS keeps saves in its own IndexedDB filesystem. Left alone, a browser
session becomes a third place progress lives, and the two hours you put in from
the sofa are invisible to the handheld.

`src-service/src/saves.rs` already answers this for every other device, and the
rule is written down: **a difference is only resolvable when exactly one side
moved.** Agreed on the server's bytes and changed here -> upload. Agreed on
what you still hold and the server moved -> download. Agreed on neither ->
conflict, never resolved silently. A browser is just another device with a
device id.

So: read the save out of EmulatorJS on exit and on a timer, and put it through
`/api/saves` with the same negotiate the Flip uses. `/api/states` is already
built and takes an emulator name. **Do not invent a second sync.**

The trap is that a browser tab closes without warning. A save written only on
exit is a save lost to a closed laptop lid.

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
