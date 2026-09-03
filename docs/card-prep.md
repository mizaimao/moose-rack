# Preparing a card — standard procedure

Four firmwares done this way so far: NextUI (RG SP), spruceOS (Miyoo A30, Miyoo
Flip), dArkOS (MiniLoong Pocket 1), KNULLI (RK3566). The steps are the same
every time; only the layout table changes.

Read `docs/a30-spruce-card.md` for the long-form story of the first one. This is
the short version you actually follow.

## The sequence

1. **Identify the card before touching it.** `diskutil list`. A card in a Mac's
   built-in reader reports as **internal**, so `diskutil list external` shows
   nothing and the habit of "pick the external one" does not protect you. Two
   128 GB cards look identical by size — tell them apart by volume name.
2. **Install the firmware.** Image flash (`dd`) or extract-to-card, depending.
   See the table.
3. **Boot it once on the device.** Non-negotiable. First boot creates the
   folder tree, expands partitions, and in spruce's case unpacks ~28 nested
   archives. Never guess folder names — read them off the card afterwards.
4. **Survey the card, do not assume.** Which system folders exist, what art
   convention, where saves and BIOS go. Every firmware has surprised us at
   least once.
5. **Build the manifest and show it** before copying. Platforms the device
   cannot run are a waste of hours; platforms with no folder are impossible.
6. **Stage**: ROMs, art, gamelists (if ES-based), BIOS, saves, collections,
   favorites, cores.
7. **Verify, then eject.** Checklist at the bottom.

## Layout, by firmware

| | NextUI | spruceOS | ArkOS / dArkOS | KNULLI / Batocera |
|---|---|---|---|---|
| ROM folders | `Roms/<Name> (TAG)` | `Roms/<SYS>` | `<system>` on EASYROMS | `roms/<system>` on SHARE |
| Folder naming | tag in parens maps the emulator | fixed short names | lowercase ES names | lowercase ES names |
| Art path | `.media/<stem>.png` | `Imgs/<stem>.png` | `images/<stem>.png` | `images/<stem>-image.png` |
| Art size | screen-sized | **640x480**, converted to QOI on device | free choice | free choice |
| Gamelist | none | none | `gamelist.xml` | `gamelist.xml` |
| Favorites | n/a | `Saves/pyui-favorites.json` | `<favorite>true</favorite>` | `<favorite>true</favorite>` |
| Collections | n/a | `Collections/collections.json` | ES `custom-*.cfg` (on rootfs) | `system/configs/emulationstation/collections` (on SHARE) |
| Saves sorted by | core | **core**, with arch suffix | core | **system** |
| Per-system core | folder tag | `Emu/<SYS>/config.json` | `<alternativeEmulator><label>` | `<system>.core=` in `knulli.conf` |
| Per-game core | none | none | `<altemulator>LABEL</altemulator>` | `<emulator>`/`<core>` in gamelist |
| BIOS | `Bios/<TAG>/` | `BIOS/` flat + `BIOS/fbneo/` | `bios/` flat on EASYROMS | `bios/` on SHARE — **not flat**: flat *plus* `dc/`, `fbneo/`, `amiga/`, `neocd/` |

## The traps, in the order they bit us

**Format under Windows for spruce.** A macOS-formatted card extracted perfectly
— 10,634 of 10,634 files, `.tmp_update` present, every dot path intact — and hung
on the Loading screen past 40 minutes. Two wasted install cycles. Rufus, FAT32.
Image-flashed firmwares (dArkOS, BaseOS) do not care, because `dd` writes the
partition table itself.

**AppleDouble files.** Finder writes one `._name` sidecar per file onto
FAT32/exFAT and MinUI-family firmware lists them as games. `dot_clean -m` over
every folder you touched, including `images/`, which is created after the copy
loop and gets missed. Delete `.fseventsd`; `.Spotlight-V100` cannot be removed
and does not matter.

**The architecture suffix.** spruce names save folders from
`spruce/scripts/emu/lib/core_mappings.sh`, not from the core's display name, and
appends the arch: `PCSX-ReARMed-32` on the A30 (armhf), `PCSX-ReARMed-64` on the
Flip (aarch64). Reading the display name out of the `.so` gives the wrong answer.
Check `PLATFORM_ARCHITECTURE` in the platform cfg first.

**Save extensions differ by core, and a folder-level copy is not enough.**
Measured from a real backup:

| Core | Writes |
|---|---|
| Geolith | `.mcr` + `.nv` |
| FB Alpha 2012 | `.fs` |
| FinalBurn Neo | `.fs` + `.hi` |
| Flycast | `.ldci` |
| mGBA, Snes9x, Gambatte, PCSX-ReARMed, SwanStation, Mupen64Plus | `.srm` |

Copying Geolith's `.mcr` into an FB Alpha 2012 folder produces files that core
can never read. **Match the core, not just the system.** Battery saves are raw
dumps and do port between cores of the same system; save *states* never do.

**Device-specific core defaults.** spruce's SFC default is `chimerasnes` on the
A30 but `mednafen_supafaust` on the Flip — the config carries per-device blocks
keyed on `MIYOO_FLIP`. Read the block that names your device, not
`default_emulator`.

**Cores missing from the map.** geolith is absent from `core_mappings.sh`, so
`get_core_folder` returns `"UNKNOWN"` and every save lands in a shared junk
folder. Adding one case line fixes it.

**Junk rows in the library.** Four server entries are directories indexed as
single games and fail every copy: `Battle City`, `Multicarts`, and two
`Aftermarket`. N64 additionally has `USA` and `Europe` — whole region folders,
6.5 GB between them. Filter those or N64 reports 45 games at 7.2 GB.

**Multi-disc games.** Folders of CHDs with an `.m3u`. `stage_arkos_card.py` and
`stage_knulli_card.py` handle them; `stage_spruce_card.py` does not and needs a
manual pass. Layout is a dot-prefixed folder for the discs with the playlist
beside it, so only the playlist shows in the list.

**Neo Geo naming.** The server stores titles (`2020 Super Baseball (set 1).zip`);
FBNeo and FB Alpha identify sets by MAME romset id (`2020bb.zip`). Only 8 of 163
match, so those cores fail on the rest. Either use geolith, or rename via
`data/arcade-names.json`, which maps 147 of 163.

**Silent path validation.** PyUI drops any collection or favorite whose
`rom_file_path` does not resolve, without a word. Build those against what is
actually on the card and verify zero broken paths before ejecting.

## Saves — mapping, moving, renaming

The hardest part of a card, and the only one where a plausible-looking copy can
silently do nothing. `backups/saves` is organized **by core**. Where it lands
depends on the firmware, and *which* folder depends on the core the target
actually runs.

**1. Find the target's active core per system.** Not the generic default — the
device-specific one.

    spruceOS   Emu/<SYS>/config.json — read the menuOption block whose
               "devices" list names your device, e.g. MIYOO_FLIP. Fall back to
               "default_emulator" only if no block matches.
    ArkOS      <alternativeEmulator><label> at the top of gamelist.xml
    KNULLI     <system>.core= in system/knulli.conf
    NextUI     the folder tag itself

**2. Resolve core to folder name.**

    spruceOS   spruce/scripts/emu/lib/core_mappings.sh, NOT the display name in
               the .so. Apply arch_suffix(): -32 on armhf, -64 on aarch64.
               A core missing from that map returns "UNKNOWN" and every save
               for it lands in one shared junk folder — add a case line.
    ArkOS      RetroArch's own display name, sort_savefiles_enable = true
    KNULLI     no core folder at all — saves/<system>/, sort_savefiles_enable
               = false
    NextUI     Saves/<TAG>/

**3. Check the extension against the destination core.** This is the step that
looks done but is not. A save written by one core is only useful in a folder
whose core reads that extension:

| Core | Writes |
|---|---|
| Geolith | `.mcr` + `.nv` |
| FB Alpha 2012 | `.fs` |
| FinalBurn Neo | `.fs` + `.hi` |
| Flycast | `.ldci` |
| Gambatte | `.srm` + `.rtc` |
| mGBA, gpSP, Snes9x, Supafaust, PCSX-ReARMed, SwanStation, Mupen64Plus, FCEUmm | `.srm` |

Copying Geolith's `.mcr` into `FB Alpha 2012/` produced seven files that core
can never read. It looked correct — right system, right game names — and was
worthless. **Match the core, not the system.**

Battery saves are raw SRAM dumps and *do* port between cores of the same system
when the extension matches. Save states never port, between anything.

**4. Split cores that serve more than one system.** Gambatte covers both GB and
GBC. Decide per game by checking which ROM folder actually holds it — a blind
copy puts all 26 in one folder and half are never found.

**5. Merge cores that serve the same system.** Where two cores hold saves for
the same game, take the **newer file per game**, not one folder wholesale. On a
real library, 25 of 66 PSX saves were newer under SwanStation and 41 under
PCSX-ReARMed; picking either folder alone loses a third of the progress. Same
for N64 across Mupen64Plus-Next and ParaLLEl.

**6. Never rename to force a match.** If the extension is wrong, the core is
wrong. Renaming `.mcr` to `.fs` produces a file the core will try to parse and
reject, or worse, overwrite.

**7. Keep the originals.** Copy into the active core's folder, do not move. If
the core is switched later, the original is still where that core looks.

Mappings used so far, for reference:

| Source core | spruce (A30, armhf) | spruce (Flip, aarch64) | KNULLI |
|---|---|---|---|
| `mGBA` | `gpSP` | `mGBA` | `saves/gba` |
| `Snes9x` | `ChimeraSNES-32` | `Supafaust` | `saves/snes` |
| `SwanStation` + `PCSX-ReARMed` | `PCSX-ReARMed-32` | `SwanStation` | `saves/psx` |
| `Mupen64Plus-Next` + `ParaLLEl N64` | — | `LudicrousN64 2K22 Xtreme Amped` | `saves/n64` |
| `Geolith` | — | `Geolith` | `saves/neogeo` |
| `FinalBurn Neo` | `FinalBurn Neo` | `FinalBurn Neo` | `saves/fbneo` |
| `Gambatte` | `Gambatte` | `Gambatte` | `saves/gb` + `saves/gbc` |

`Genesis Plus GX` saves in that backup are Master System and Game Gear games,
not Mega Drive — check the contents before mapping by core name alone.

## BIOS — copying, and why a manifest-driven copy under-delivers

The step that looks finished and is not. `bios-coverage.md` covers *what the
collection holds*; this is *what a card needs and where it goes*.

**The firmware's own checker is the authority, not our manifest.** Two lists
disagree and only one of them decides whether a game boots:

- `data/bios-manifest.json` — derived from RetroArch core `.info`
  `firmwareN_path` declarations. It is what the cores *declare*.
- The firmware's checker — on KNULLI, **`knulli-systems`** (a Python script with
  the expected path and md5 for every system baked in). It is what the device
  *looks for*.

Run the checker on the device after staging and read it as the verdict. Filter
it to the systems you actually put games on — it reports on ~95 systems and the
noise buries the four lines that matter:

```sh
ssh -tt root@<device> "knulli-systems" | awk '/^> /{s=substr($0,3)} \
  s ~ /^(psx|dreamcast|neogeo|pcengine|n64|snes|gba)$/ {print s"|"$0}'
```

It prints only problems: `MISSING` (absent) and `UNTESTED` (present, md5 not one
it recognizes — a different revision of the right file, which may or may not
work).

### Trap 1 — the manifest does not know every file the device wants

`dc_flash.bin` is required by KNULLI for Dreamcast and **is not in the manifest
at all**, because no core `.info` declares it. It sits in `library/system/` with
the exactly-right md5 and never gets copied, because the copy loop iterates the
manifest. Anything the firmware wants but no core declares is invisible to a
manifest-driven copy. **The checker finds these; the manifest never will.**

### Trap 2 — KNULLI's BIOS folder is not flat, and the stager flattens it

`tools/stage_knulli_card.py` writes every file to
`card/bios/<basename>` — `pathlib.PurePath(e["path"]).name` throws the
subdirectory away. **31 of the 60 files it selects belong in a subdirectory**
(all of `dc/*`, all of `fbneo/*`), so they land one level too high. On the Flip
this is visible as `awbios.zip`, `airlbios.zip`, `f355bios.zip`, `cchip.zip`,
`coleco.zip`, `hiscore.dat` sitting loose in `/userdata/bios/` while
`knulli-systems` reports `bios/dc/awbios.zip` MISSING.

KNULLI mixes the two shapes, so neither blanket rule is right:

| Flat | In a subdirectory |
|---|---|
| `dc_boot.bin`, `dc_flash.bin`, `neogeo.zip`, `syscard3.pce`, `scph*.bin`, `psxonpsp660.bin` | `dc/awbios.zip`, `dc/naomi.zip`, `fbneo/*.zip`, `amiga/kick*`, `neocd/*.rom` |

Note Dreamcast is split across both: the console's own `dc_boot.bin` /
`dc_flash.bin` are flat, while the arcade boards that share the core
(`naomi`, `atomiswave`) read from `dc/`. **Preserve the manifest's relative
path; do not take the basename.**

### Trap 3 — present is not correct

`neogeo.zip` on the Flip is md5 `24b989a4…`; KNULLI expects `dffb72f1…`. The
file is there, the name is right, and it is a different MVS revision — reported
`UNTESTED`, not `MISSING`. Three different `neogeo.zip` files exist in the
library (`system/`, `system/fbneo/`, `roms/neogeoaes/`), all different, **none**
matching what KNULLI wants. Check the hash, not the filename. This one matters
because geolith is the fixed Neo Geo core here (see `handheld-os.md`) and it
needs that ROM.

### Flip status, checked 2026-08-24

| System | State |
|---|---|
| psx | **OK** — `psxonpsp660.bin` + `scph5500/5501/5502` present. The `scph101/1001/7001` the checker lists are alternates |
| dreamcast | ~~`dc_flash.bin` missing~~ **fixed 2026-08-25** — rsynced onto the live card, md5 `0a93f794…`, checker now clean. The manifest still does not know about it (Trap 1), so a re-staged card will lose it again. `dc_boot.bin` present and correct |
| neogeo | `neogeo.zip` present, **wrong revision** (Trap 3) |
| pcengine | `syscard3.pce` missing — irrelevant, both games are HuCard homebrew, not CD |
| nes, snes, megadrive, gb, gbc, gba, n64, fbneo | **nothing needed** — no BIOS entries, or already satisfied |

### Getting files onto a live KNULLI device

`scp` and `ssh 'cat > file' < src` both **hang** against KNULLI (measured
2026-08-24) — the login flow stalls without a PTY, and `-tt` mangles binary.

**`rsync` works. Use it** (re-tested 2026-08-25, `dc_flash.bin` onto the live
card). Drive the password with the same `expect` wrapper `handheld-os.md`
records, wrapping `rsync` instead of `ssh`:

    rsync -av -e {ssh -o StrictHostKeyChecking=accept-new \
      -o UserKnownHostsFile=/dev/null -o PreferredAuthentications=password \
      -o PubkeyAuthentication=no} <src> root@10.10.10.187:<dest>

It ends with `chown ... Operation not permitted` and exit 23. **That is not a
failure** — `/userdata` will not take an ownership change, and the file itself
lands byte-exact. Check the md5 rather than the exit code. Pulling the card is
no longer necessary for a file or two.

## Tools

    tools/build_media_set.py     resize art once into library/media-640, reused per card
    tools/stage_spruce_card.py   spruceOS
    tools/stage_arkos_card.py    ArkOS / dArkOS
    tools/stage_knulli_card.py   KNULLI / Batocera
    tools/stage_sp_card.py       NextUI
    tools/copy_to_card.py        top up an existing ES-DE card from the server
    tools/card_collections.py    ES-DE collections and favorites

All are dry by default; `--apply` is the only thing that writes.

## Before ejecting

```sh
find /Volumes/<CARD> -name '._*' | wc -l          # must be 0
lsof +D /Volumes/<CARD>                            # must be empty
python3 -c "import json;json.load(open('<collections.json>'))"   # PyUI cards
```

For ES-based cards, confirm every gamelist parses — they carry two root
elements (`<alternativeEmulator>` then `<gameList>`), which is not well-formed
XML, so wrap before parsing.

Then `diskutil eject`, or `sudo umount` if it was mounted by hand.

## Recovery point

`dd` cannot skip free space device-to-device. Compress instead — free space
costs almost nothing:

```sh
sudo diskutil unmountDisk /dev/diskN
sudo dd if=/dev/rdiskN bs=4m | gzip -1 > card-$(date +%Y%m%d).img.gz
```

Restore with `gunzip -c ... | sudo dd of=/dev/rdiskN bs=4m`. No need to wipe the
target first. Verify the archive once with `gunzip -t` — a backup that has never
been read is not one.
