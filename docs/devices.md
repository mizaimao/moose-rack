# The four copies of the library

Server, Retro SSD, Android and the Flip. Same games, four different layouts,
four different ways in. This is where each one keeps its ROMs, its artwork, its
gamelists and its hashes, so the next person does not have to find out by
poking around.

[library-sync.md](library-sync.md) is the companion: it covers how the
databases hash things and which one to believe. This file is only about the
machines.

## Getting in

| | |
| --- | --- |
| server | `ssh dev.lan` — key auth, nothing to set up |
| SSD | `/Volumes/Retro`, plugged into the Mac |
| Android | `adb` in `.toolchain/`, over USB |
| Flip | `knulli.local`, root / `linux`, **password only** |

Android needs the toolchain's own adb and its own HOME, or it will not find the
key:

    ADB=.toolchain/android/sdk/platform-tools/adb
    HOME=.toolchain/android/home "$ADB" devices

The Flip refuses ssh keys — its sshd sees a 0777 home and `StrictModes` rejects
them. An earlier attempt lost a day to this, so do not try again. Drive it with
`expect`, which ships with macOS; there is no need to install `sshpass`:

    spawn ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
              -o LogLevel=ERROR root@knulli.local $cmd
    expect { -re "(P|p)assword:" { send "linux\r"; exp_continue }  eof }

`10.10.10.187` appears in older docs as the Flip's address. It no longer
answers. `knulli.local` does.

## Where the ROMs live

    server    /home/frank/romm/assets/roms/<slug>     ( = /romm/library/roms/<slug> in the container)
    SSD       /Volumes/Retro/ROMs/<system>
    Android   /storage/A2FC-A9FB/Roms/<system>
    Flip      /userdata/roms/<system>

**Android's library is on the external card, not internal storage.**
`/sdcard/ROMs` exists and is empty; every game is under `/storage/A2FC-A9FB`.
Looking at the wrong one reads as "the device has nothing".

The SSD is exFAT and case-insensitive, so `ROMs` and `Roms` are the same
directory and both spellings appear in older scripts.

## The system names differ

Three of the four agree; the SSD does not. Get this wrong and a copy lands in a
folder nothing reads.

| server | SSD | Android | Flip |
| --- | --- | --- | --- |
| `megadrive` | `genesis` | `genesis` | `megadrive` |
| `gamegear` | `gamegear_hidden` | `gamegear` | `gamegear` |
| `gb` | `gb` | `gb` | `gb` |

The SSD's `_hidden` suffix is Frank's, on `gamegear`, `mastersystem`, `ngp`,
`pcengine`, `wonderswan` and `wonderswancolor`. **Do not rename them** — he has
said so directly. Treat them as ordinary system folders. Note that only the
*ROM* folder carries the suffix: the SSD's artwork and gamelists for Game Gear
are under plain `gamegear`.

## Artwork and gamelists

The three ES-DE machines find media by convention, from the ROM's base name.
The Flip does not — its gamelist names the file outright.

    SSD       ES-DE/support/downloaded_media/<system>/<type>/<base>.<ext>
              ES-DE/gamelists/<system>/gamelist.xml
    Android   ES-DE/downloaded_media/<system>/<type>/<base>.<ext>
              ES-DE/gamelists/<system>/gamelist.xml
              ES-DE/collections/custom-<name>.cfg
    Flip      /userdata/roms/<system>/images/<base>-image.png
              /userdata/roms/<system>/gamelist.xml   with <image> tags

ES-DE keeps eleven media types per system — `3dboxes`, `backcovers`, `covers`,
`fanart`, `manuals`, `marquees`, `miximages`, `physicalmedia`, `screenshots`,
`titlescreens`, `videos`. A rename has to move all of them.

**The Flip's image is the miximage, downscaled to 640×480**, which is its
screen. Verified against its existing `gb` and `megadrive` images rather than
assumed. `sips -z 480 640 in.png --out "<base>-image.png"` produces the same
thing.

A Flip system with ROMs but no gamelist entries shows no artwork at all —
`pcengine` once had 292 games and 2 entries. When you put a system on the Flip,
write the gamelist too.

The SSD has a **second** gamelist tree at `ES-DE/Windows/ES-DE/gamelists`. It
is not the one in use. Edit `ES-DE/gamelists`.

Saves live in `/Volumes/Retro/Saves` and `/Volumes/Retro/saves` on the SSD and
`/storage/A2FC-A9FB/Saves` on Android, named after the ROM. A rename orphans
them silently, so check before renaming — as of 2026-08-29 none of the games
renamed that day had any.

## Where the hashes are

Only the server stores hashes. Everywhere else you compute them.

RomM keeps two columns per ROM, and they answer different questions:

| column | what it is |
| --- | --- |
| `md5_hash` | file identity — see the archive rules in [library-sync.md](library-sync.md) |
| `ra_hash` | produced by `RAHasher`, RetroAchievements' own tool. Trust it |

`ra_id` is empty for almost everything and means nothing. Ignore it.

To read them:

    P=$(grep -oP 'MARIADB_ROOT_PASSWORD[=:]\s*\K\S+' /home/frank/romm/docker-compose.yml | head -1 | tr -d '"')
    docker exec romm-db mariadb -u root -p$P romm -N -B -e "SELECT ..."

Computing them elsewhere, the traps are:

- `md5sum` on a device hashes the **archive**, not the ROM inside it. For a
  `.zip`, compare against the local file's archive md5 to prove the transfer,
  and use `unzip -p file | md5` when you need the ROM's identity.
- SNES dumps may carry a 512-byte copier header — 1,049,088 bytes instead of
  1,048,576. No-Intro lists the headerless hash, so strip it first
  (`tail -c +513`) or a good dump looks wrong.

## Housekeeping on the server

RomM does not notice a file you replaced or renamed. The old row stays, points
at nothing, and shows up as an unhashed ROM. After changing files:

    docker exec romm python -c "
    import asyncio
    from endpoints.sockets.scan import scan_platforms
    from handler.scan_handler import ScanType
    asyncio.run(scan_platforms([7,8,13], [], ScanType.QUICK))"

then delete the dead rows through RomM's own handler, so the linked file and
asset rows go too:

    from handler.database import db_rom_handler, db_platform_handler
    p = db_platform_handler.get_platform_by_fs_slug("gb")
    rom = db_rom_handler.get_roms_by_fs_name(platform_id=p.id, fs_names=[name]).get(name)
    db_rom_handler.delete_rom(rom.id)

Check `os.path.exists` first and refuse if the file is still there. Deleting by
raw SQL leaves orphans behind.

Platform ids as of 2026-08-29: gamegear 7, gb 8, megadrive 13.

## Shell traps that have cost real time

**`scp`, `ssh` and `adb` inside `while read` eat the loop's stdin.** The loop
runs once and reports success. Put `</dev/null` on every one of them.

**zsh aborts a loop when a glob matches nothing** — "no matches found" and the
whole script stops partway, having done half the work. Media loops walk
directories where most patterns miss, so run them under
`/bin/bash -c 'shopt -s nullglob; …'`.

**Remote paths with spaces, brackets and apostrophes** are worth avoiding
entirely: `scp` to `/tmp/up_1`, then `mv` into place from a script file on the
device. Quoting a Game Gear filename through expect, ssh and the remote shell
is three layers deep and gets it wrong quietly.

**macOS `tar` writes `._` AppleDouble files.** Use `COPYFILE_DISABLE=1`. On the
Flip's exFAT `/userdata`, `tar` also reports `Cannot change ownership` for
every entry — exFAT has no Unix ownership, extraction succeeded, ignore it.

**A gamelist entry must escape `&`.** Frank's `gamegear` gamelist is currently
malformed on every device for exactly this reason — a raw `&` in
`Puzzle & Action - Ichidanto-R (Japan).gg`. Any strict parser rejects the whole
file. It was like that before we arrived; it is still like that.

**Renaming a ROM can collide with a gamelist entry that already exists.**
Renaming Landstalker's European file to the USA name produced two identical
`<game>` blocks on Android. Count the path before writing, and diff the result
against the untouched copy to prove you removed exactly one.
