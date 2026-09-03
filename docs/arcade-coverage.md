# Arcade core coverage

Measured by `tools/dat_coverage.py`, which reads no emulator and launches nothing:
it compares the CRC32 of every file a core's DAT says a driver requires against the
CRC32s in our zips (read from the zip central directory, no decompression).

**Validated:** 14 arcade games were launched for real with `probe` (deterministic —
`--max-frames` plus RetroArch's log, since the exit code is 0 whether or not content
loaded). The DAT method predicted all 14 outcomes, including the one MAME failure,
and gave the reason: `sfiii` needs a CHD we do not have.

DATs used: FBNeo master, MAME Arcade 0.287 (installed core is 0.283 — four releases
apart, so a handful of verdicts may be stale), MAME 2003-Plus.

## Result

| platform | games | default | default covers | + per-game | total playable |
|---|---|---|---|---|---|
| `arcade` | 2413 | `fbneo` | 2314 (96%) | +62 | **2376 (98%)** |
| `mame` | 750 | `mame2003_plus` | 621 (83%) | +93 | **714 (95%)** |
| `neogeoaes` | 132 | `fbneo` | 121 (92%) | +0 | **121 (92%)** |
| **all** | **3295** | | | | **3211 (97%)** |

The three platforms are three different romset vintages and want different cores.
An earlier blanket `mame2003_plus` override was right for `mame` and wrong for
`arcade`; measuring them separately is what resolved the long-standing flakiness.

## Per-core coverage

| platform | fbneo | mame | mame2003_plus |
|---|---|---|---|
| `arcade` | 96% | 93% | 53% |
| `mame` | 53% | 58% | 83% |
| `neogeoaes` | 92% | 91% | 0% |

## The 84 no core can run

Not a core-choice problem — these sets are incomplete or need files we do not have.

**`arcade` — 37:** `aligator`, `avengers`, `backfirt`, `bigstrik`, `blazeon`, `bshark`, `bublbust`, `catacomb`, `chuckieegg`, `crazyfgt`, `csilver`, `dietgo`, `galastrm`, `jojoba`, `kikikai`, `looptris`, `lordgun`, `maniacsq`, `matchit`, `mgcrystl`, `missw02`, `nbahangt`, `nbamht`, `neocdz`, `progear`, `punchkid`, `raiden`, `raimais`, `rambo3`, `recalh`, `revx`, `snowboar`, `sxyreac2`, `topshoot`, `touchgo`, `vball`, `wrally2`

**`mame` — 36:** `airduel`, `alcon`, `aligatorun`, `altbeast`, `backfirt`, `bayroute`, `bigstrik`, `bionicc`, `brvblade`, `carrera`, `chokchok`, `choplift`, `crazyfgt`, `cupfinal`, `cyclwarr`, `dbreed`, `ddpdoj`, `ddux`, `drgninja`, `enduror`, `esckids`, `galaga`, `goldnaxe`, `grdian`, `hedpanic`, `jchan`, `ktiger`, `megablst`, `moonwlkb`, `raiden`, `samsho2`, `shogwarr`, `tdragonb`, `tmnt2p`, `uccopsj`, `xmen2pe`

**`neogeoaes` — 11:** `diggerma`, `fatfury2`, `fightfev`, `minasan`, `mosyougi`, `neomrdo`, `pgoal`, `pnyaa`, `ridhero`, `tws96`, `vliner`

Fixing these means sourcing matching romsets, or a CHD for the CD-based ones.
