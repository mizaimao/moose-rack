# docs

Start with **[handover.md](handover.md)** — how the work goes, what has already
cost days, and where things stand. Everything else here is one subject.

## The devices

| | |
| --- | --- |
| [devices.md](devices.md) | **How to reach each of the four copies and where everything is on it** — ROMs, artwork, gamelists, hashes, and the shell traps that waste a day. Start here before touching any of them |
| [flip-knulli-changes.md](flip-knulli-changes.md) | **Every change made to the Miyoo Flip**, read back off the device. Read before touching it |
| [knulli-addon.md](knulli-addon.md) | `moose-patch` — what it patches, how a patch is undone, the favourites sync |
| [handheld-os.md](handheld-os.md) | Which OS the Flip should run, and why |
| [flip-wayland-and-the-gpu-blob.md](flip-wayland-and-the-gpu-blob.md) | Why KNULLI's Mali driver has no Wayland support, and the two blobs |
| [a30-spruce-card.md](a30-spruce-card.md) | The Miyoo A30 on spruceOS, start to finish |
| [card-prep.md](card-prep.md) | Preparing a card — the standard procedure, four firmwares in |
| [android-port.md](android-port.md) | The AYN Thor |
| [tint.md](tint.md) | The flat wash over the app on Android. Fixed; kept because the cause was not where it looked |

## The app

| | |
| --- | --- |
| [features-wanted.md](features-wanted.md) | Things a retro frontend normally has and this one does not. A menu, not a queue |
| [memory-footprint.md](memory-footprint.md) | What the app weighs and why — 192 MB, and 106 MB of it is WebKit |
| [fast-launch.md](fast-launch.md) | Why a game takes 4.26 s to start, and the launcher being written to fix it |
| [attract-mode.md](attract-mode.md) | How ES and ES-DE do the arcade screensaver, read out of both sources. Not built |
| [cartridge-shelf.md](cartridge-shelf.md) | Games shown as the physical cartridge, Socket-style. Scoped, not built |
| [library-sync.md](library-sync.md) | Keeping server, SSD, Android and Flip in step — how RomM, RetroAchievements and Hasheous each hash differently, and why No-Intro is the one that decides |
| [library-service.md](library-service.md) | **Replacing RomM** — why it is the wrong shape for this library, and the 18 endpoints a drop-in replacement has to answer. Not built |
| [one-core-two-frontends.md](one-core-two-frontends.md) | The shape once the answer became "Flip **and** Thor" |
| [port-plan.md](port-plan.md) | The plan that came out of it |
| [handheld-frontend.md](handheld-frontend.md) | The SDL front end. Superseded by the addon — see `knulli-addon.md` |

## The library

| | |
| --- | --- |
| [library-rules.md](library-rules.md) | **The rules: every file hash-verified, every filename the database name** — and the mechanics that have gone wrong at least once. Read before touching the library |
| [inventory.md](inventory.md) | **Hashing every file on the SSD and checking it against No-Intro, Redump, TOSEC and MAME** — the plan, the schema, and how CHDs and headered ROMs are handled. Step by step, re-runnable |
| [library-audit.md](library-audit.md) | Every ROM hashed against No-Intro and Redump, and this machine compared with the server. 2026-08-28 |
| [arcade-coverage.md](arcade-coverage.md) | What of the arcade set actually runs, measured against the DATs |
| [arcade-missing-roms.md](arcade-missing-roms.md) | The 13 of 2,504 that will not, and the files they need |
| [bios-coverage.md](bios-coverage.md) | The canonical BIOS set and where it lives |
| [lists/](lists/) | One-off snapshots — want-lists, arcade label checks. Nothing regenerates them |

## Waiting on somebody else

Both are written and neither has been sent. They are here so they are not
forgotten, not because they are finished work.

| | |
| --- | --- |
| [upstream-romfile-500.md](upstream-romfile-500.md) | A RomM bug report. Not filed — `gh` is not authenticated here, and posting is Frank's call |
| [screenscraper-devid-request.md](screenscraper-devid-request.md) | A forum post asking ScreenScraper for developer credentials |
