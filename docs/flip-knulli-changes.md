# Everything we have changed on the Flip

**Current.** The Flip's actual state, read back off the device. Read before changing it.

The Miyoo Flip, running **KNULLI** (Batocera 42). This is the record of what
has been done to it and why, read back off the device rather than written from
memory. If you are picking this up cold, read this and
[knulli-addon.md](knulli-addon.md) before touching anything.

    host      knulli.local          root / linux
    ssh       password auth only

**Do not set up ssh keys.** KNULLI's sshd refuses them — `StrictModes` sees a
0777 home — and an earlier attempt lost a day to it. `scripts/knulli.sh` talks
to the device through `expect`.

## The one rule

**Nothing here is hand-edited.** Every change is a patch in `moose-patch`'s
catalogue, applied by the app, and every one of them can be turned off again
from the same list. That is the whole point of the addon: KNULLI updates blow
away `/usr`, and a device set up by hand cannot be put back.

If you find yourself about to `vi` something on the device, add a patch
instead.

## What the OS does underneath you

| Fact | Why it matters |
| --- | --- |
| `/` is an overlay whose writable layer is a 256 MB tmpfs | `/usr` is the stock image again at every boot |
| `/boot` is vfat, mounted **read-only**, 4 GB | The only writable-ish place readable early. Remount to write |
| `/userdata` is exFAT | Where everything persistent lives |
| Init order: `S00bootcustom` → `S02resize` → `S03system-splash` → `S31emulationstation` → `S50triggerhappy` → `S99userservices` | **`S02resize` mounts `/userdata`.** Anything the S00 hook reads must be on `/boot` |
| `configgen` rewrites `retroarch.cfg` at every launch | Changes made inside RetroArch's own menu do not stick. Use `knulli.conf` |
| **`knulli.conf` is first-wins** | `knulli-settings-get` scans from the top and stops. A key repeated lower down is read by nothing |

## 1. `knulli.conf` — five patch blocks

Each is wrapped in `## moose-patch: <id>` … `## moose-patch: <id> end`.

### `hotkeys` — the Menu button plus one

Buttons as `es_input.cfg` names them: `0=A 1=B 2=X 3=Y 4=L1 5=R1 6=L2 7=R2
8=Select 9=Start 10=Menu 13=Up 14=Down 15=Left 16=Right`.

    exit_emulator     B (+ quit_press_twice)     load_state       L1
    menu_toggle       X                          save_state       R1
    shader_toggle     Y                          hold_fast_fwd    R2
    fps_toggle        A                          pause_toggle     L2
    state_slot -/+    Left / Right               shader prev/next Up / Down

`reset`, `screenshot`, `ai_service`, `toggle_fast_forward` and `rewind` are set
to `nul` — they were firing by accident.

### `power` — never sleep

`system.batterysaver.extendedmode=none`. Suspending after 15 minutes idle drops
the network and reads as a dead device.

**This did nothing for weeks.** It appended `=none` under the `=suspend`
KNULLI ships on line 319, and the file is first-wins. The app said ON, the
handheld went on suspending. Fixed 2026-08-26 by making a block comment out an
earlier line setting the same key.

### `charge-awake` — awake while charging

`system.batterysaver.chargingbypass=1`, and it hid the `=0` KNULLI ships.

Plugged in, the handheld stops dimming and stops suspending on its own.
`batteryplus` drops `/var/run/battery-saver/*.pause` whenever the battery is
not discharging, and both idle hooks check for it. **Closing the lid and
pressing power still sleep it** — `lid-control` reads `system.lid` and
`power-button` calls `knulli-suspend`, and neither looks at that file.

No script, no service: KNULLI ships the whole mechanism switched off.

### `shaders`

    global.shaderset=moose-lcd
    global.retroarch.video_shader_dir=/userdata/shaders/moose

It matters that the set is **ours**. configgen resolves a set's presets
relative to `/userdata/shaders`, and RetroArch cycles the directory of the
preset it loaded — not `video_shader_dir`. Point it at a stock set and
Hotkey + D-pad walks the whole 700-preset library, most of which this handheld
cannot afford.

### `bezel-gba`

`gba.bezel=moose` — the silver one. `bezel-gb` and `bezel-gbc` exist in the
list and are off.

### Also in `knulli.conf`, outside the blocks

Cores aligned with moose-rack (written 2026-08-23):

    dreamcast=flycastvl  fbneo=fbneo    gb=gambatte   gba=mgba    gbc=gambatte
    megadrive=genesisplusgx             n64=mupen64plus/rice
    neogeo=geolith       nes=fceumm     psx=pcsx_rearmed          snes=snes9x

**`neogeo=geolith` is deliberate** — the ROMs are geolith-specific. Do not
switch it to fbneo.

**`gba=mgba`, and it stays there.** vba-m was tried on 2026-08-27 and put back
the next day; the vbam-era game saves are on the card at
`/userdata/saves/gba-backup-vbam-20260828/`.

What the swap was for is still true: mGBA's cheats are broken under RetroArch.
Enable one and the core keeps running while the frontend stops taking input.
mGBA reports non-linear memory segments through
`RETRO_ENVIRONMENT_SET_MEMORY_MAPS` and RetroArch's cheat manager cannot follow
them — [RetroArch#7387][ra7387], open since 2018, with core and frontend each
saying it is the other's fix. vba-m handles CodeBreaker and GameShark codes
itself and does not go through that path. So if cheats are ever wanted on a
particular GBA game, `gba["<rom>.gba"].core=vbam` is the per-game way to get
them without moving the whole system.

Switching either way is cheap for game saves and not for save states.
RetroArch owns the `.srm` filename rather than the core, and
`sort_savefiles_enable` is false, so both cores write into
`/userdata/saves/gba/` with no per-core subfolder. Save states do not survive a
core change and never do.

The one thing to watch is EEPROM games — the two cores can disagree on layout
where they cannot on flat SRAM. In this library that is Shantae Advance and
TOCA World Touring Cars. Everything else is SRAM or flash.

[ra7387]: https://github.com/libretro/RetroArch/issues/7387

## 2. `/boot` — the things that must survive the tmpfs

    boot-custom.sh              runs as S00
    moose-blank-logo.png        a blank PNG
    moose-libmali-stock.so      43 MB
    moose-libmali-wayland.so    56 MB
    moose-gpu                   which blob to install (absent = stock)

`boot-custom.sh` does two jobs at every boot:

* **`apply_gpu`** — copies the chosen Mali blob over `/usr/lib/libmali.so.1`.
* **`blank_es_logo`** — copies the blank PNG over
  `/usr/share/emulationstation/resources/logo.png`, before S31 starts ES.

Everything it reads is on `/boot`, and that is not a preference. It runs as
S00; `/userdata` is not mounted until S02. Both halves of this file lived in
`/userdata` at first, so both did nothing, silently, at every boot.

## 3. `/userdata/system/custom.sh` — `splash`

Zeroes `/dev/fb0` at start. `S03system-splash` paints the KNULLI logo into the
framebuffer and leaves it there; it is invisible while ES or an emulator owns a
DRM plane and flashes up every time one is torn down — so at every game launch
and every exit.

## 4. triggerhappy — L2+R2 opens the addon

In `/userdata/system/configs/multimedia_keys.conf` (which overrides `/etc`):

    BTN_TR2+BTN_TL2 1   /userdata/system/moose-patch/moose-launch.sh
    BTN_TL2+BTN_TR2 1   /userdata/system/moose-patch/moose-launch.sh

Both orderings, because triggerhappy matches the **exact set of held keys** and
the trigger must be the second button pressed.

## 5. EmulationStation

    InvertButtons            true
    CollectionSystemsAuto    recent
    CollectionSystemsCustom  18 Arcade collections

`InvertButtons` is a **UI preference, not a hardware fact** — it says nothing
about which button is physically A. Trust the letters in `es_input.cfg`; they
are the letters printed on the plastic. See the warning at the top of
[handover.md](handover.md).

Favourites in `/userdata/roms/<system>/gamelist.xml` mirror the server's
`★ Best of …` collections; the custom `.cfg` files mirror the Arcade ones.
Both sync both ways now — see [knulli-addon.md](knulli-addon.md).

## 6. Files we put on the card

    /userdata/shaders/moose/            4 presets + retroarch.glslp
    /userdata/decorations/moose/        the GBA bezel
    /userdata/system/moose-patch/       binary, launcher, cache, config, backups
    /userdata/roms/ports/moose-patch.sh L2+R2's target, also reachable from Ports
    /userdata/roms/ports/Moose Rack.sh        the archived SDL front end

## 7. Saves

`/userdata/saves/<system>/` — **by system, not by core**, which is what
configgen tells RetroArch to do. The desktop and Android use RetroArch's own
`saves/<Core>/` layout. `Platform::save_layout()` is what keeps the two apart;
`Platform::save_folder()` maps the server's slug to the card's folder
(`sfc`→`snes`, `famicom`→`nes`, `arcade`→`fbneo`, `neogeoaes`→`neogeo`, …).

## 8. Making games start faster

A warm GBA launch was **4.26 s**, measured 2026-08-28. RetroArch is 0.83 s of
that — core, ROM and ten shader passes. The other 3.43 s is `configgen`.

`launch-evmapy` takes 0.93 s off it. `batocera-evmapy start` kills the daemon,
touches a flag and blocks on `inotifywait` until it comes back; it is a process
round trip, not work. configgen writes a per-device `.json` into
`/var/run/evmapy` *before* calling `start`, and `libretro.keys` asks only for a
lightgun combo — so a libretro launch with no gun writes nothing at all and
then waits for a daemon with no job.

The guard is one line inserted into `/usr/bin/batocera-evmapy`:

    ls /var/run/evmapy/*.json >/dev/null 2>&1 || exit 0

It deliberately knows nothing about libretro. No device config means nothing to
map. **The other 54 `.keys` files all declare real `actions_playerN` mappings**
— flycast, amiberry, hatari, azahar, gsplus — so every standalone emulator is
untouched. An unconditional stub, which is what a first A/B of this actually
did, would have broken all of them.

Measured 3.43 s → 2.50 s, three runs each side, identical to within 0.02 s.

`/usr` is a tmpfs, so `boot-custom.sh` puts the line back at every boot — the
same mechanism as `es-logo` and `gpu`. It is *inserted* rather than copied over,
so a KNULLI update to that script keeps its own changes and only gains our
line, and a marker comment makes it idempotent.

The rest of the 3.43 s is being taken by a native launcher — see
[fast-launch.md](fast-launch.md).

## What has cost the most time

Each of these looked like something else first.

* **`knulli.conf` is first-wins.** A patch can write exactly what it says it
  wrote and still change nothing. Check with `knulli-settings-get`, not by
  reading the file.
* **S00 cannot read `/userdata`.** A boot hook that reads from there fails by
  doing nothing at all, at every boot, in silence.
* **Restarting ES over ssh loses sound.** It needs `setsid` plus
  `/etc/profile.d/xdg.sh` and `dbus.sh`, or it comes up with no
  `XDG_RUNTIME_DIR`. `S31emulationstation start` backgrounds it from the
  calling shell, so SSH's SIGHUP kills it.
* **The launch logo is `resources/logo.png`**, not a setting. Three wrong
  theories went before that: a RetroArch animation, an ES setting, `/dev/fb0`.
* **RomM reports two slugs.** `platform_slug` is the catalogue name (`sfam`),
  `platform_fs_slug` is the library folder (`sfc`). The cache keys on the
  second. Using the first matched nothing and looked like an empty library.
* **Multi-disc games are `.m3u` on the card.** RomM has one ROM,
  `Final Fantasy VII (USA)`; the card has a hidden `.Final Fantasy VII (USA)/`
  of discs and a playlist beside it, and the playlist is what ES shows.
* **The idle hooks are `#!/bin/bash` and use `compgen`.** Run one with `sh` to
  see what it would do and the pause check fails open — it suspends the device
  you are testing on.
* **Never bulk-launch RetroArch on the Mac.** Every launch opens a window;
  `video_driver=null` does not prevent it.

## 9. The library on the card

Layout, which is not the ES-DE layout the SSD and Android use:

    /userdata/roms/<system>/                    the ROMs
    /userdata/roms/<system>/images/<base>-image.png
    /userdata/roms/<system>/gamelist.xml        with explicit <image> tags

The image is the **miximage downscaled to 640×480**, the screen's size.
`sips -z 480 640 in.png --out "<base>-image.png"` matches what is already
there. A system with ROMs and no gamelist entries shows no artwork at all —
`pcengine` sat at 292 games and 2 entries until that was noticed.

Copy a whole system as one tar rather than hundreds of scp calls; 204 MB took
55 seconds that way. `COPYFILE_DISABLE=1` keeps macOS from writing `._` files
into it, and `tar` complaining `Cannot change ownership` on extract is exFAT
having no Unix ownership — the extraction worked.

**Game Gear, 2026-08-29.** The card had none: `/userdata/roms/gamegear` held
only `_info.txt`. It now has 328 ROMs, 331 images and a 328-entry gamelist,
every ROM hashed on the device and compared against the SSD. 226 MB.

Note it has **no favourites**. Every other system does — nes 85, snes 71,
megadrive 67, gba 44, gb 41, gbc 31, all as `<favorite>true</favorite>` in the
gamelist. Game Gear has never been favourited on any device, so there was
nothing to bring across.

Dump fixes the same day, all confirmed against the No-Intro DAT after landing:
Rocky and Bullwinkle, Garfield Labyrinth, Spot and FIFA on `gb`, plus
Flashback USA on `megadrive`, with `Landstalker   The Treasures of King Nole
(Europe).7z` and both `Flashback (Europe)` files removed. The card already had
the correct `Landstalker (USA).zip`. See [library-sync.md](library-sync.md).

Still behind: 56 merged games, the Home Alone fix, the two Oddworld discs.

## Where it stands

    hotkeys          ON            hotkey-app       ON
    shaders          shimmerless + LCD/CRT          es-shoulders     ON
    bezel-gba        silver        es-logo          ON
    bezel-gb         off           boot-splash      ON
    bezel-gbc        off           never-sleep      ON
    shader-gba/gb/gbc  follow global                charge-awake     ON
    wifi-awake       off           gpu              stock
    launch-evmapy    ON

Read it back yourself with:

    ssh root@knulli.local
    cd /userdata/system/moose-patch && ./moose-patch --status

and change one without a controller in hand with:

    ./moose-patch --apply charge-awake=ON

`--apply` reads the state back rather than reporting what it was asked for,
which is the difference that catches the first-wins class of bug.

**Open question for Frank:** `never-sleep` and `charge-awake` are both on. Now
that blocks actually override KNULLI's own values, `never-sleep` will do what
it says — never suspend, on battery too. If the battery should behave normally,
turn `never-sleep` off and leave `charge-awake` to do the job.
