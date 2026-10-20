# Windows 95 and 98 games

**First pass built, 2026-10-19, on `win9x`.** 544 eXoWin9x games are packed
into the library and play in the browser, from the library service, in js-dos
on a Windows 98 disk of our own. [What the first pass
built](#what-the-first-pass-built) says how; [Still not built](#still-not-built)
what is open. Everything above that section is the 2026-09-17 probe, when two
games booted in DOSBox-X on the Mac: Zulu War started by itself, and Red Alert
after one networking change, and Frank played a mission in it.

## Why DOSBox-X, and why eXoWin9x's layout

DOSBox Pure can install Windows 98 and run it inside RetroArch, but the
libretro buildbot's Apple Silicon build has only the interpreter CPU core (see
[dos.md](dos.md)). Windows-era games need something close to a Pentium II, so
Pure is out on the Mac.

eXoWin9x Volume 1 covers 1994 to 1996: 662 games in a 282.3 GB torrent. It
runs almost all of them in DOSBox-X (it ships 2025.02.01 for Windows) and a
handful in 86Box. The Mac runs used DOSBox-X 2026.03.29, the arm64 SDL2 build
in Frank's Downloads.

DOS games do not go through Windows. DOSBox-X or DOSBox Pure run them directly,
which is how Frank's KKND setup in Downloads already works.

## How eXoWin9x runs a game

Three drives, set up in the game's `Play.conf`:

| drive | what it is |
| --- | --- |
| C: | `emulators/dosbox/x98/W98-C.vhd`, a differencing disk on `x98/parent/W98-C.vhd` (396 MB). `vhdmake -f -l` recreates it on every launch, so Windows starts clean every time |
| D: | the game's own dynamic VHD from its zip. Zulu War's is 8 MB of a 2 GB disk, Red Alert's 42 MB. Everything that persists lives here |
| E: | CD images, `IMGMOUNT e a.cue b.cue -t cdrom -ide 2m`, or a zip, `MOUNT e x.zip` |

    vhdmake -f -l .\emulators\dosbox\x98\parent/W98-C.vhd .\emulators\dosbox\x98\W98-C.vhd
    IMGMOUNT c .\emulators\dosbox\x98\W98-C.vhd
    IMGMOUNT d ".\eXoWin9x\1996\ZuluWar (1996)\ZuluWar (1996).vhd"
    MOUNT e ".\eXoWin9x\1996\ZuluWar (1996)\ZWDEMO.zip"
    BOOT -l c

A clean Windows still has to find the game. The game disk carries three things
besides the game: `Reg.reg`, the registry changes captured while it was
installed; `Windows/`, whatever the installer put under `C:\Windows`, such as
fonts; and a shortcut or `.vbs` in `Windows/Desktop`. The parent image's StartUp
folder runs `C:\eXo\Setup.vbs`, which copies `D:\Windows` over `C:\Windows`,
runs `regedit /s d:\reg.reg`, and then starts the one shortcut in
`D:\Windows\Desktop`, or asks when there are several.

Parents for DOSBox-X are `W98-C`, `W98-C-Net` (child `W98-H`, used by Red
Alert), `W98-C-Net2` (child `W98-J`), `Win95DX8`, `win98Jap` and
`Win98Chinese`. 86Box has `W98-P`, `ME-P` and two network ones.

The launch is `dosbox-x -conf !win9x/<year>/<game>/Play.conf -conf
emulators/dosbox/options9x.conf`. `Play.conf` is a complete DOSBox-X config;
Red Alert's asks for `svga_s3`, 64 MB, `pentium_mmx`, `core = auto`, a Voodoo
and an SB16.

## What the Mac needed

1. `[autoexec]` paths are written with backslashes. Rewrite them to `/`.
2. Zulu War's config has `output = direct3d`, which exists only on Windows.
   Override with `-set "sdl output=opengl"`.
3. The network parents set `ne2000 backend = pcap` with a Windows adapter name.
   pcap does not open on the Mac, and Red Alert hung on a black screen after
   its CD check without ever changing video mode. `-set "ne2000 backend=slirp"`
   fixed it: the log switched to 640x480 and the game played.
4. Red Alert's `Play.conf` looks for CD 2 under `Command & Conquer - Red Alert
   (1996)`, a folder that does not exist; the zip unpacks to `Command And
   Conquer`. Its `.cue` files name the `.bin` in upper case against lower-case
   files, which works only on a case-insensitive disk. Both break on the Linux
   server as shipped.
5. With `HOME` pointed into `/tmp`, DOSBox-X wrote nothing outside it across
   three runs. Without that it writes its preferences into `~/Library`.

Speed was not measured. `screencapture` is refused for this process, so the
runs were checked by the log and by Frank at the machine.

## On the server

`dev.lan:/home/frank/moose-library/`:

| path | what |
| --- | --- |
| `eXoWin9x-torrent/eXoWin9x.torrent` | the torrent |
| `eXoWin9x/eXo/util/utilWin9x.zip` | 2.5 GB; holds `EXTWin9x.zip` (emulators, parent images) and `OPTWin9x.zip`, both also unpacked beside it |
| `eXoWin9x/Content/!Win9Xmetadata.zip` | 8.4 GB; per game `Play.conf`, `Install.bat`, the launcher `.bat` and the extras, which are most of the size |
| `eXoWin9x/eXo/eXoWin9x/1996/` | Zulu War and Red Alert |
| `eXoWin9x/eXo/emulators/dosbox/x98/W98-{C,H,J}.vhd` | the three child disks, unpacked while probing |

Fetch a file into an empty scratch directory and move only that file out. aria2c
also writes the pieces of neighbouring files that share a piece with the one
selected, and those files come out full-sized and mostly empty. Zulu War's zip
was one: it listed, then failed to unpack.

## Games eXoWin9x does not have

Volume 1 stops at 1996. Dune 2000 (1998) and Red Alert 2 (2000) are ISOs in
`~/Data/Games/Emulation/emulation_files/W98SE_dosboxx/images`. The plan for
them is the same layout: eXo's parent, a new D: disk from `vhdmake`, the game
installed from its ISO, then `Reg.reg`, the `Windows/` files and a Desktop
shortcut added to the disk. eXo captured installs with CyberMedia UnInstaller,
whose monitor files are still on the Zulu War disk. We need a capture step of
our own. Not done.

KKND 2: Krossfire in Downloads is GOG's `setup_kknd2_2.0.0.7.exe`. GOG
installers are usually built for later Windows and may not run in 98, in which
case unpack it on the host instead.

The other Windows zips in Downloads (Air Force Missions, Marble Blast Ultra,
the two Robokill games) are from 2007 onward. Windows 98 in DOSBox-X is the
wrong tool for them.

## What the first pass built

### Where things are

eXoWin9x moved to the NAS with the rest of the library: `GameFiles/eXoWin9x`,
`/mnt/gamefiles/eXoWin9x` on the server, 263 GB. The games are zips under
`eXo/eXoWin9x/<year>/`, 248 GB, each holding the game's D: disk and its CD
images or zip. They are played where they are and never copied.

| path under `/mnt/gamefiles` | what | written by |
| --- | --- | --- |
| `ROMs/windows9x/<Title (Year)>.w9x` | one per game, a few KB | `tools/win9x_pack.py` |
| `ES-DE/gamelists/windows9x/gamelist.xml` | names, descriptions, dates | `tools/win9x_pack.py` |
| `ES-DE/downloaded_media/windows9x/<type>/` | eXo's artwork | `tools/win9x_pack.py` |
| `eXoWin9x/moose/moose98-1.vhd` | our Windows 98 disk, 252 MB | by hand, below |

Those four are the Windows side's to write; nothing else under the library is.

### Our Windows 98 disk

eXo's parent disk is someone else's install, with their wallpaper, sounds and
start-up jingle. Ours is a clean Windows 98 SE install with only the parts of
eXo's that games need moved across, and it stands in for eXo's `W98-C.vhd`.
The scripts are in `tools/win98-image/`; they were run by hand, on macserver
(DOSBox-X and `hdiutil`) and dev.lan (the registry work).

1. `w98gen.py` writes an unattended `MSBATCH.INF` and the DOSBox-X configs.
   The product key is read from a file and goes only into `MSBATCH.INF`. The
   hardware is eXo's: `svga_s3`, 64 MB, `pentium_mmx`, SB16, APM, ISA PnP,
   no network card. Setup copies `MSBATCH.INF` to `C:\WINDOWS` and keeps a
   registry copy in `C:\SYSTEM.1ST`; both hold the key and were deleted from
   the finished disk. The live registry still has it, as every install does.
2. `treediff.py`, `exo-diff.py`: what eXo's disk has that ours does not, by
   content, not by name. By name alone it missed DirectX 7, which replaces
   files Windows 98 ships.
3. `w9xreg.py` reads Windows 9x registry files (`SYSTEM.DAT`, `USER.DAT`)
   offline; `transplant.py` and `regdiff.py` write `TRANS.REG` from eXo's
   registry, limited to what the moved files need: DirectX 7, the 3dfx Voodoo
   driver and Glide, QuickTime, Indeo, the VB and WinG runtimes, and the PCI
   entries that tell Windows the Voodoo is already set up. Without those, a
   boot with the Voodoo on stops at "new hardware found".
4. `pick.py` stages 146 files, `apply.py` writes them and our scripts onto the
   mounted image. macOS gives long file names different 8.3 names from the
   ones Windows gave them on eXo's disk, and the registry names some of them
   (`QUICKT~1.QTX`, `3DFXIN~2.INF`), so `fatshort.py` renames those directory
   entries to eXo's and `shortcheck.py` checks.
5. One boot runs `IMPORT.BAT` from StartUp: `TRANS.REG`, `MOOSE.REG` (our
   start-up entry), `NOWELCOME.VBS` (the Welcome screen and its music start
   from a `Run` entry that a `.reg` file cannot delete), then it shuts down.
   The two last ran as separate boots; the folder has them combined.
6. Remove `IMPORT.BAT` and the files it used, `zerofree.py` to zero the
   space deleted files left, then `VHDMAKE -convert` in DOSBox-X for a fixed
   VHD with DOSBox-X's own geometry, and `vhddyn.py` to make it a dynamic
   VHD that stores only the blocks in use: 252 MB, against eXo's 396.

`START.VBS` runs at every logon and does what eXo's `Setup.vbs` did: copies
`D:\Windows` over `C:\Windows`, imports `D:\reg.reg`, and starts the one
shortcut on `D:\Windows\Desktop`, or asks which. `C:\eXo\` keeps eXo's two
resolution tools at the paths game shortcuts use.

Checked on macserver: a boot with the Voodoo on reached the desktop with no
new hardware, and the registry's PCI keys did not change. Zulu War booted the
eXo way, its fonts and shortcut were copied across, and the log switched to
640x480. Frank tried it in the browser and it worked.

A new build of the disk gets a new name, `moose98-2.vhd`, and `BASE` in
`win9x_pack.py` moves with it: the browser keeps the disk by name.

### Packing

`tools/win9x_pack.py` on the server, like the DOS packer:

    python3 tools/win9x_pack.py --all            # report only
    python3 tools/win9x_pack.py --all --apply

A `.w9x` is eXo's `Play.conf` with its comments out, `ne2000 = false` (our
disk has no network driver; with the card in, Windows finds it on every boot
and waits for an answer) and `sdl output` dropped, and its `[autoexec]`
rewritten for the page:

    imgmount c c.vhd
    imgmount d "ZuluWar (1996)/ZuluWar (1996).vhd"
    mount e "ZuluWar (1996)/ZWDEMO.zip"
    BOOT -l c

The paths are names in the game's zip, spelled as the zip spells them. Its
first lines, comments to DOSBox-X, name the zip and the disk, and carry any
corrected CD sheets: 45 of eXo's name `GAME.BIN` beside `game.bin`, which only
works where file names ignore case. The page writes the corrected sheet beside
the original as `<name>.moose.cue`, and the mount names that.

Of eXo's 664 games, 544 pack. Refused, 2026-10-19:

| | |
| --- | --- |
| 66 | boot eXo's Windows 98 with networking (`W98-C-Net`, `-Net2`) |
| 29 | run in 86Box |
| 21 | Japanese or Chinese Windows 98 |
| 2 | Windows 95 |
| 2 | their `Play.conf` names a file their zip does not have |

635 of the 664 configs ask for the same machine; only five set their own CPU
speed, and the network ones differ in network settings.

### Playing

`ui/js/win9x-player.js`, reached from Play when the platform is `windows9x`
(`OWN_PLAYER` in `ejs-systems.js`):

1. The `.w9x` from `/rom`, like any game.
2. The Windows 98 disk from `/win9x?path=moose/moose98-1.vhd`, kept in Cache
   Storage after the first time.
3. The game's zip from `/win9x?path=eXo/eXoWin9x/...`. `/win9x` serves files
   inside eXoWin9x and nothing else (`resolve_win9x` in src-service/src/web.rs).
4. js-dos with the `.w9x` as its config and three kinds of file: the disk as
   `c.vhd`, the zip as a bundle it unpacks itself, the corrected CD sheets.
   DOSBox-X backend, no js-dos cloud, no network relay.

js-dos 8.4.1 is vendored like EmulatorJS: `scripts/fetch-jsdos.sh`, pinned in
`assets/jsdos/MANIFEST.tsv`, served at `/jsdos/8.4.1/` and marked immutable.
The script points one address in it, a benchmark's upload, at nothing.

Before a download over 64 MB the page asks, and says when the disk is part of
it. A game that unpacks to more than 1.5 GB is warned about: it may not fit in
a tab. 50 of the 544 unpack to over 1 GB, the largest to 4.8 GB.

The desktop app has no route to eXoWin9x and no core for the system, so Play
there says the games play in a browser from the library server, and the
platform shows "no core". On the service it shows as playable.

## Still not built

1. Saves. Everything a game writes is on its D: disk, which in the browser is a
   copy in the tab's memory: closing the tab loses it.
2. Speed. The prototype was laggy. js-dos ships a JSPI build of DOSBox-X
   (`wdosbox-x-jspi`) that is not tried yet, and the 50 biggest games are not
   tried at all.
3. The 120 refused games: a networked Windows 98 of our own, Japanese and
   Chinese ones, Windows 95, and 86Box.
4. Playing on the desktop or a handheld.
5. Building game disks for games eXoWin9x does not carry.
