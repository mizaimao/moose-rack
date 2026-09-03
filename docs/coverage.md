# Coverage — what of the set actually runs

**Measurement.** Arcade measured against the DATs, and the firmware underneath it.

Three measurements of the same question: given the files we hold, what
actually starts. Arcade is measured against the DATs, the thirteen failures
are listed with the exact chips they want, and the BIOS set is the firmware
all of it depends on.

Was `arcade-coverage.md`, `arcade-missing-roms.md` and `bios-coverage.md`.


---

## Arcade core coverage

*Was `arcade-coverage.md`.*

Measured by `tools/dat_coverage.py`, which reads no emulator and launches nothing:
it compares the CRC32 of every file a core's DAT says a driver requires against the
CRC32s in our zips (read from the zip central directory, no decompression).

**Validated:** 14 arcade games were launched for real with `probe` (deterministic —
`--max-frames` plus RetroArch's log, since the exit code is 0 whether or not content
loaded). The DAT method predicted all 14 outcomes, including the one MAME failure,
and gave the reason: `sfiii` needs a CHD we do not have.

DATs used: FBNeo master, MAME Arcade 0.287 (installed core is 0.283 — four releases
apart, so a handful of verdicts may be stale), MAME 2003-Plus.

### Result

| platform | games | default | default covers | + per-game | total playable |
|---|---|---|---|---|---|
| `arcade` | 2413 | `fbneo` | 2314 (96%) | +62 | **2376 (98%)** |
| `mame` | 750 | `mame2003_plus` | 621 (83%) | +93 | **714 (95%)** |
| `neogeoaes` | 132 | `fbneo` | 121 (92%) | +0 | **121 (92%)** |
| **all** | **3295** | | | | **3211 (97%)** |

The three platforms are three different romset vintages and want different cores.
An earlier blanket `mame2003_plus` override was right for `mame` and wrong for
`arcade`; measuring them separately is what resolved the long-standing flakiness.

### Per-core coverage

| platform | fbneo | mame | mame2003_plus |
|---|---|---|---|
| `arcade` | 96% | 93% | 53% |
| `mame` | 53% | 58% | 83% |
| `neogeoaes` | 92% | 91% | 0% |

### The 84 no core can run

Not a core-choice problem — these sets are incomplete or need files we do not have.

**`arcade` — 37:** `aligator`, `avengers`, `backfirt`, `bigstrik`, `blazeon`, `bshark`, `bublbust`, `catacomb`, `chuckieegg`, `crazyfgt`, `csilver`, `dietgo`, `galastrm`, `jojoba`, `kikikai`, `looptris`, `lordgun`, `maniacsq`, `matchit`, `mgcrystl`, `missw02`, `nbahangt`, `nbamht`, `neocdz`, `progear`, `punchkid`, `raiden`, `raimais`, `rambo3`, `recalh`, `revx`, `snowboar`, `sxyreac2`, `topshoot`, `touchgo`, `vball`, `wrally2`

**`mame` — 36:** `airduel`, `alcon`, `aligatorun`, `altbeast`, `backfirt`, `bayroute`, `bigstrik`, `bionicc`, `brvblade`, `carrera`, `chokchok`, `choplift`, `crazyfgt`, `cupfinal`, `cyclwarr`, `dbreed`, `ddpdoj`, `ddux`, `drgninja`, `enduror`, `esckids`, `galaga`, `goldnaxe`, `grdian`, `hedpanic`, `jchan`, `ktiger`, `megablst`, `moonwlkb`, `raiden`, `samsho2`, `shogwarr`, `tdragonb`, `tmnt2p`, `uccopsj`, `xmen2pe`

**`neogeoaes` — 11:** `diggerma`, `fatfury2`, `fightfev`, `minasan`, `mosyougi`, `neomrdo`, `pgoal`, `pnyaa`, `ridhero`, `tws96`, `vliner`

Fixing these means sourcing matching romsets, or a CHD for the CD-based ones.


---

## Arcade games that will not run, and the files they need

*Was `arcade-missing-roms.md`.*

13 of 2,504. Each zip holds a different revision of the game than the
emulator's driver of that name expects — usually a different region's board.
The CRC is a fingerprint of the exact chip contents, so it identifies which
version to look for rather than just the filename.

Only the core that gets furthest is listed for each game.

### Avengers (US, rev. D)  (`avengers.zip`)

Best core: **mame** — needs 1 file(s):

- `avu_04d.10n`  —

### Catacomb  (`catacomb.zip`)

Best core: **fbneo** — needs 1 file(s):

- `74s288.bin`  0x7e0b79cb

### Chuckie Egg  (`chuckieegg.zip`)

Best core: **fbneo** — needs 1 file(s):

- `ppokoj2.bin`  0x80285be4

### Bad Dudes vs. Dragon Ninja  (`drgninja.zip`)

Best core: **fbneo** — needs 1 file(s):

- `eg25.15h`  0x6791bc20

### Ninja Kazan  (`iganinju.zip`)

Best core: **fbneo** — needs 1 file(s):

- `iga.14m`  0x1d877538

### Jojo's Bizarre Adventure  (`jojoba.zip`)

Best core: **mame** — needs 1 file(s):

- `jojoba_euro_nocd.29f400.u2`  —

### KiKi KaiKai  (`kikikai.zip`)

Best core: **fbneo** — needs 1 file(s):

- `a85-01_jph1020p.h8`  0x01771197

### Lord Of Gun  (`lordgun.zip`)

Best core: **mame** — needs 1 file(s):

- `lord_gun_u144-ch.u144`  —

### Maniac Square  (`maniacsq.zip`)

Best core: **mame** — needs 1 file(s):

- `d8-d15.1m`  —

### Koutetsu Yousai Strahl  (`strahl.zip`)

Best core: **fbneo** — needs 1 file(s):

- `nmk004.bin`  0x8ae61a09

### Touch & Go  (`touchgo.zip`)

Best core: **mame** — needs 1 file(s):

- `tg_873d_56_5-2.ic56`  —

### Ajax  (`typhoon.zip`)

Best core: **mame** — needs 1 file(s):

- `770c13.n22`  —

### U.s. Championship V'ball  (`vball.zip`)

Best core: **fbneo** — needs 1 file(s):

- `25a2-4.124`  0xbe04c2b5


---

## BIOS / firmware coverage

*Was `bios-coverage.md`.*

The canonical set lives on the RomM host at `/romm/library/_retroarch_system`
and is synced to `library/system/` on each machine, which RetroArch is pointed
at via `system_directory`. One folder, identical everywhere.

### How the list was derived

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

### Coverage

**237 of 277 present.** The server's collection was already laid out
in RetroArch's expected structure, so every match was an exact path match — none
needed renaming.

#### Required and absent — 1

- `aes.zip` — aes.zip (Neo Geo AES System ROM) (needed by: arcade, mame, neogeo)

#### Optional and absent — 39

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

### Refreshing

```sh
python3 tools/bios_manifest.py --info <RetroArch>/info   # recompute needs
ssh dev.lan 'docker exec romm tar -C /romm/library/_retroarch_system -cf - .' \\
  | tar -xf - -C library/system                                  # sync
```
