# Windows 95 and 98 games

**Probed by hand, not built.** Written 2026-09-17 on the `moose-dos` branch.
Two eXoWin9x games booted in DOSBox-X on the Mac, run from `/tmp`. Zulu War
started by itself. Red Alert started after one networking change, and Frank
played a mission in it. Nothing here is in the app yet.

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

## Not built

1. A DOSBox-X launch path in the app, beside RetroArch: find DOSBox-X, write
   the game's config with the Mac changes above, and run it where its three
   drives resolve.
2. The importer on the server: fetch a game by torrent index and take its
   `Play.conf` from the metadata zip.
3. Saves. The D: disk is the save, so sync means moving a whole disk per game.
4. Building game disks for games eXoWin9x does not carry.
5. The Linux fixes for the CD sheets and paths, if the server is ever to run
   these.
