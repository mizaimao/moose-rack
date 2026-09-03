# The Miyoo A30 card — spruceOS

How this card was built, and the things that cost time. `handover.md` is the
wider brief for the app; this is one device, start to finish, written so it can
be reproduced or audited without rediscovering the traps.

Built 2026-08-19 against spruceOS 4.3.4. Tools live in `tools/`.

## The device

Allwinner A33 — four Cortex-A7 at 1.2 GHz, Mali-400 MP2, **512 MB RAM**, 2.8"
640x480. ARMv7, 32-bit. It is an 8- and 16-bit machine that also happens to run
PS1. Everything past that is out, and no amount of core-swapping changes it.

`spruce/scripts/platform/A30.cfg` sets `PLATFORM_ARCHITECTURE="armhf"`. That
single value decides several things below, so check it first on any new device.

## Why spruceOS and not NextUI

NextUI does not support this hardware and cannot be made to. Three independent
confirmations, all worth repeating because the naming invites the mistake:

* NextUI's own docs list TrimUI Brick, Smart Pro and Smart Pro S. No Miyoo.
* The community H700 port adds Anbernic H700 only.
* Every NextUI release zip contains `Emus/tg5040`, `Emus/tg5050` and (in the
  port) `Emus/h700`. Zero matches for `miyoo`, `a30`, `my282`.

Unpacking it anyway gives a card full of binaries the A33 cannot execute.
spruceOS supports the A30 explicitly from 4.0.0 on.

## Formatting the card — the expensive one

**Format under Windows with Rufus. A card formatted by macOS Disk Utility does
not boot.** It got as far as spruce's "Loading" framebuffer screen and hung
there past 40 minutes. Nothing in the logs, nothing wrong with the files: the
extract verified at 10,634 of 10,634, `.tmp_update` was present, all 2,458 dot
paths from the archive survived. It simply would not boot.

That was two wasted install cycles. Do not debug it; reformat under Windows.

FAT32 either way. The card ends up ~119 GB usable on a 128 GB card.

## Installing spruce

`README.md` inside the release says it in one line — extract the 7z to the root
of the card — and the wiki adds the parts that matter:

* **`.tmp_update` must land at the root.** It is hidden; a Finder copy with
  hidden files off silently omits it and spruce will not run.
* **Clean macOS junk after copying.** Finder writes one `._name` sidecar per
  file onto FAT32 and MinUI-family firmware lists them as games. `dot_clean -m`
  over the card, then delete `.fseventsd`. `.Spotlight-V100` cannot be removed
  and does not matter.
* **Power on, then power off and on again.** The second boot is part of the
  procedure, not a workaround.

First boot unpacks 28 nested archives, ~0.40 GB, on a 1.2 GHz A7. Ten to twenty
minutes is normal. Beyond that, read `Saves/spruce/spruce.log` before touching
anything; the documented hang is at "Finishing up unpacking" and the fix is
deleting `spruce/flags/first_boot_A30.lock`.

Fresh-install state is marked by `spruce/flags/first_boot_<PLATFORM>.lock`,
which ships present and is removed on first boot. It is **not** a dot file and
was never at risk from the cleanup above.

## What went on it

`tools/stage_spruce_card.py`, dry by default, `--apply` to write. Systems are
copied smallest-first so the handheld is usable long before the two big ones
finish.

| RomM slug | spruce folder | | RomM slug | spruce folder |
|---|---|---|---|---|
| psx | `PS` | | megadrive | `MD` |
| arcade | `FBNEO` | | mastersystem | `MS` |
| neogeoaes | `NEOGEO` | | gamegear | `GG` |
| gba | `GBA` | | pcengine | `PCE` |
| gb / gbc | `GB` / `GBC` | | wonderswan(color) | `WS` / `WSC` |
| nes + famicom | `FC` | | neo-geo-pocket | `NGP` |
| snes + sfc | `SFC` | | | |

Two RomM platforms share one spruce folder in two places, which is fine — the
few filename collisions are the same game twice.

Result: 8,956 games, ~65 GB. Eleven entries failed; four were server-side
directories indexed as single games (`Battle City`, `Multicarts`, two
`Aftermarket`) and seven were multi-disc PSX, handled below.

## Box art

`Roms/<SYS>/Imgs/<rom filename without extension>.png`. Taken from the shipped
scraper rather than guessed — `App/PyUI/main-ui/utils/boxart/box_art_scraper.py`:

```python
rom_name = os.path.splitext(file)[0]
image_path = os.path.join(target_img_dir, f"{rom_name}.png")
```

Source is the ES-DE miximages in `library/downloaded_media`, downscaled to 512 px
on the long edge with `sips`. Miximages because they are the only art type that
is a consistent shape across every console here; at scrape resolution they
average 570 KB and would outweigh the games.

8,835 of 8,956 games have art.

## Saves — the `-32` trap

Saves came from `backups/saves`, laid out by libretro core, which is what
`sort_savefiles_enable = "true"` produces.

**spruce does not name save folders after the core's display name.** It has its
own table in `spruce/scripts/emu/lib/core_mappings.sh`, and on armhf it appends
an architecture suffix:

```sh
arch_suffix() { [ "$PLATFORM_ARCHITECTURE" = "armhf" ] && echo "32" || echo "64"; }
"chimerasnes_libretro.so") echo "ChimeraSNES-$(arch_suffix)" ;;
"pcsx_rearmed_libretro.so") echo "PCSX-ReARMed-$(arch_suffix)" ;;
```

So PSX saves belong in `PCSX-ReARMed-32`, not `PCSX-ReARMed`. Reading the
display name out of the `.so` gives the wrong answer. Read `core_mappings.sh`.

Where spruce's default core differs from the core that wrote the save, the
saves were **copied** into the default core's folder rather than switching
cores. Every file is a raw battery dump and the sizes prove the formats match —
GBA at 512/8192/32768/65536/131072 is exactly the five GBA battery types, and
both PSX cores produce 131072-byte memory cards. Originals were kept.

| From | To | Files |
|---|---|--:|
| `mGBA` | `gpSP` | 188 |
| `Snes9x` | `ChimeraSNES-32` | 35 |
| `SwanStation` + `PCSX-ReARMed` | `PCSX-ReARMed-32` | 66 |

The PSX two overlapped on 51 games with different playthroughs in each, so the
merge takes the newer file per game: 41 came from one folder, 25 from the other.
Picking a folder wholesale would have lost a quarter of the progress.

`Gambatte`, `Genesis Plus GX`, `FCEUmm`, `FB Alpha 2012` and `FinalBurn Neo`
already matched and needed nothing.

Worth knowing: spruce migrates saves itself when you change core, via
`handle_changed_core` in `spruce/scripts/emu/lib/ra_functions.sh`. It matches
`<rom name>.*` — any extension — backs up whatever is already there as
`.bak-<timestamp>`, and deliberately does not copy save states. The behavior is
controlled by a `keepSavesBetweenCores` setting (Always / Never / Prompt).

## BIOS — three locations, not one

`retroarch-A30.cfg` sets `system_directory = "/mnt/SDCARD/BIOS"`.

1. **`BIOS/`, flat** — console BIOS. 32 files, from `library/system`, selected by
   cross-referencing `data/bios-manifest.json` against the cores spruce uses.
2. **`BIOS/fbneo/`** — FBNeo's BIOS romsets and `hiscore.dat`. Confirmed from
   the core binary, which contains `system/fbneo/` and states *"you also need
   the file hiscore.dat in your system/fbneo/ folder"*. spruce does not create
   this folder; it has to be made.
3. **`Roms/FBNEO/` and `Roms/NEOGEO/`** — the same FBNeo romsets again, because
   FBNeo also resolves parent sets from the content directory. The wiki names
   Neo Geo as the one system whose BIOS lives with the games.

Only `fbneo/spec1282a.zip` is absent, and nothing here uses it.

## PICO-8

Native PICO-8 needs the **Raspberry Pi** release. `Pico8.Native.INFO.txt` at the
card root asks for `pico8.dat`, `pico8_64` and `pico8_dyn` directly in `BIOS/`.
`runtimeHelper.sh` picks by architecture:

```sh
if [ "$PLATFORM_ARCHITECTURE" = "armhf" ]; then PICO8_EXE="pico8_dyn"
else PICO8_EXE="pico8_64"; fi
```

so the A30 runs `pico8_dyn`. `Roms/PICO8/` already contains the Splore launcher.
The `fake08` core and `Roms/FAKE08/` are the unlicensed fallback — do not use
both.

## Multi-disc PSX

Seven games arrived as folders of CHDs plus an `.m3u` and failed the flat copy.
spruce's own `m3u_generator.sh` shows the intended layout: discs in a **hidden**
folder, playlist beside it.

```
Roms/PS/Final Fantasy VII (USA).m3u          <- the only visible entry
Roms/PS/.Final Fantasy VII (USA)/            <- leading dot, holds the discs
Roms/PS/Imgs/Final Fantasy VII (USA).png     <- art keyed to the m3u name
```

The `.m3u` lines are `.<Game>/<Game> (Disc N).chd`. PS ends up with 102 entries:
94 single-disc games and 8 playlists.

## Collections and favorites

Neither format is documented; both were read from the shipped PyUI source.

* `Collections/collections.json` — `collections_manager.py`
* `Saves/pyui-favorites.json` — `roms_list_manager.py`

```json
[{"collection_name": "Arcade Fighting",
  "game_list": [{"rom_file_path": "/mnt/SDCARD/Roms/FBNEO/sf2.zip",
                 "game_system_name": "FBNEO"}]}]
```

Favorites are a flat list of the same entry shape plus `display_name`.

`game_system_name` is the **`Emu/` folder name**, not the label —
`games/utils/game_system.py` says so in a comment on `system_name`. `FC`, not
`NES`.

Paths are absolute as the device sees them: `/mnt/SDCARD/Roms/...`.

**PyUI validates every path with `os.path.exists` on load and silently drops
failures**, so build these against what is actually on the card and verify
before ejecting. 26 collections / 2,641 entries and 380 favorites were written
with zero broken paths. "Best of n64" was dropped — no N64 on this device.

## Verifying before eject

```sh
find /Volumes/A30 -name '._*' | wc -l          # must be 0
python3 -c "import json;json.load(open('/Volumes/A30/Collections/collections.json'))"
lsof +D /Volumes/A30                            # must be empty
diskutil eject /Volumes/A30
```

## What is not on it, and why

N64, DS, Dreamcast, Saturn, 3DO, PSP, GameCube. spruce ships folders for several
of them because the same card runs on Brick and Smart Pro; a folder existing is
not the device promising to run it. On an A33 they do not.
