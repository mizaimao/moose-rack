# BIOS / firmware coverage

The canonical set lives on the RomM host at `/romm/library/_retroarch_system`
and is synced to `library/system/` on each machine, which RetroArch is pointed
at via `system_directory`. One folder, identical everywhere.

## How the list was derived

Not guesswork. Two authoritative sources, joined by `tools/bios_manifest.py`:

- `data/vendor/esde_android_es_systems.xml` — ES-DE's own definitions, naming the
  libretro core behind every launch command (**195 systems, 159 cores**).
- RetroArch's `info/*_libretro.info` — each core declares its firmware as
  `firmwareN_path` (the exact name it looks for) plus `firmwareN_opt`, which marks
  whether the core runs without it.

That yields **277 distinct files** across **95 systems** — 43 required, 234 optional.

128 of them sit in a subdirectory (`dc/dc_boot.bin`, `PPSSPP/ppge_atlas.zim`,
`keropi/cgrom.dat`). A flat dump of BIOS files does not work — the layout is part
of the lookup.

## Coverage

**237 of 277 present.** The server's collection was already laid out
in RetroArch's expected structure, so every match was an exact path match — none
needed renaming.

### Required and absent — 1

- `aes.zip` — aes.zip (Neo Geo AES System ROM) (needed by: arcade, mame, neogeo)

### Optional and absent — 39

These only matter if you run the system in question; the cores boot without them.

- **amiga** — `kick33180.A500`, `kick37350.A600`, `kick39106.A1200`, `kick39106.A4000`, `kick40068.A4000`
- **arcade** — `dc/naomi2.zip`, `fbneo/spec1282a.zip`
- **atari5200** — `ATARIBAS.ROM`, `ATARIOSA.ROM`, `ATARIOSB.ROM`, `ATARIXL.ROM`, `BB01R4_OS.ROM`, `XEGAME.ROM`
- **atari7800** — `7800 BIOS (U).rom`
- **gb** — `sgb_boot.bin`
- **gba** — `nds_sd_card.bin`
- **nds** — `dsi_sd_card.bin`
- **palm** — `bootloader-dbvz.rom`, `palmos40-en-m500.rom`, `palmos52-en-t3.rom`, `palmos60-en-t3.rom`
- **scummvm** — `scummvm/extra/achievements.dat`, `scummvm/extra/encoding.dat`, `scummvm/extra/freescape.dat`, `scummvm/extra/grim-patch.lab`, `scummvm/extra/hadesch_translations.dat`, `scummvm/extra/macgui.dat` (+11 more)
- **vircon32** — `Vircon32Bios.v32`

## Refreshing

```sh
python3 tools/bios_manifest.py --info <RetroArch>/info   # recompute needs
ssh dev.lan 'docker exec romm tar -C /romm/library/_retroarch_system -cf - .' \\
  | tar -xf - -C library/system                                  # sync
```
