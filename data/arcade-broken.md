# Broken arcade romsets — to revisit

Found by launch-testing every game in Roms/arcade (see data/arcade-core-test.json).
These run on no core: FBNeo, MAME 2003-Plus and MAME - Current all reject them.

| romset | title | missing ROMs | example missing file |
|---|---|---:|---|
| `catacomb` | Catacomb | 1 | `74s288.bin` |
| `chuckieegg` | Chuckie Egg | 1 | `ppokoj2.bin` |
| `kikikai` | KiKi KaiKai | 1 | `a85-01_jph1020p.h8` |
| `raiden` | Raiden (World set 1) | 1 | `4.u023` |
| `dietgo` | Diet Go Go (Europe v1.1 1992.09.26, set 1) | 2 | `may-04_w78_9235kd011.14a` |
| `maniacsq` | Maniac Square (unprotected, version 1.0, checksum BB73) | 2 | `d8-d15.1m` |
| `touchgo` | Touch and Go (World, checksum 059D0235) | 2 | `tg_873d_56_5-2.ic56` |
| `missw02` | Miss World 2002 | 3 | `u81` |
| `packbang` | Pack'n Bang Bang | 3 | `bbp0x3_u23.u23` |
| `avengers` | Avengers (US, rev. D) | 4 | `avu_04d.10n` |
| `jojoba` | JoJo's Bizarre Adventure (Europe 991015, NO CD) | 5 | `jojoba_euro_nocd.29f400.u2` |
| `lordgun` | Lord of Gun (World) | 15 | `lord_gun_u144-ch.u144` |
| `wrally2` | World Rally 2: Twin Racing (version 20-07, checksum B1B8) | 17 | `dallas_usa_wr-2_2_64_usa_e47e_31-7.bin` |

The first four need a single chip dump each — cheapest to replace.
`lordgun` and `wrally2` are missing most of their contents.
