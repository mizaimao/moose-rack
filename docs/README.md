# docs

Start with **[handover.md](handover.md)** — how the work goes, what has already
cost days, and where things stand.

Every file below opens with a **status line** saying whether it is current
truth, a dated measurement, a design that was never built, or a record of
something already done. Read that line before trusting the rest: several of
these look like to-do lists and are not.

## The devices

| | |
| --- | --- |
| [devices.md](devices.md) | **Current.** How to reach each of the four copies and where everything is on it — ROMs, artwork, gamelists, hashes, and the shell traps that waste a day. Start here before touching any of them |
| [flip-knulli-changes.md](flip-knulli-changes.md) | **Current.** Every change made to the Miyoo Flip, read back off the device rather than written from memory. Read before touching it |
| [knulli-addon.md](knulli-addon.md) | **Current.** `moose-patch` — what it patches, how a patch is undone, the favourites sync |
| [handheld-os.md](handheld-os.md) | **Current, one section superseded.** Which OS the Flip runs and why. The ROCKNIX reasoning lost to KNULLI on 2026-08-24 and is kept for history; the on-device findings after it are live |
| [flip-wayland-and-the-gpu-blob.md](flip-wayland-and-the-gpu-blob.md) | **Finding, closed.** Why KNULLI's Mali driver has no Wayland support, and the blob swap that makes Weston and Sway run |
| [card-prep.md](card-prep.md) | **Current.** Preparing a card — the standard procedure, four firmwares in |
| [a30-spruce-card.md](a30-spruce-card.md) | **Current.** The Miyoo A30 on spruceOS, start to finish |
| [android-port.md](android-port.md) | **Plan, not built.** The AYN Thor |
| [tint.md](tint.md) | **Fixed.** The flat wash over the app on Android. Kept because the cause was not where it looked |

## The app

| | |
| --- | --- |
| [one-core-two-frontends.md](one-core-two-frontends.md) | **Current.** The shape the project settled into once the answer became "Flip **and** Thor" |
| [memory-footprint.md](memory-footprint.md) | **Measurement.** What the app weighs and why — 192 MB, and 106 MB of it is WebKit |
| [fast-launch.md](fast-launch.md) | **Measurement + work in progress.** Why a game took 4.26 s to start, and the launcher written to fix it |
| [library-service.md](library-service.md) | **Design, not built.** Replacing RomM — why it is the wrong shape for this library, and the 24 method/path pairs a drop-in has to answer |
| [not-built.md](not-built.md) | **Designed, not built.** The arcade screensaver, the cartridge shelf, and the features a retro frontend normally has and this one does not. A menu, not a queue |

## The library

| | |
| --- | --- |
| [library-rules.md](library-rules.md) | **Current. The rules** — every file hash-verified, every filename the database name — and the mechanics that have gone wrong at least once. Read before touching the library |
| [inventory.md](inventory.md) | **Current, re-runnable.** Hashing every file on the SSD and checking it against No-Intro, Redump, TOSEC and MAME — the plan, the schema, and how CHDs and headered ROMs are handled |
| [library-sync.md](library-sync.md) | **Current.** Keeping server, SSD, Android and Flip in step — how RomM, RetroAchievements and Hasheous each hash differently, and why No-Intro decides |
| [library-audit.md](library-audit.md) | **Record, 2026-08-28.** Every ROM hashed against No-Intro and Redump, and this machine compared with the server |
| [coverage.md](coverage.md) | **Measurement.** What of the arcade set actually runs, the 13 of 2,504 that will not, and the canonical BIOS set underneath both |
| [lists/](lists/) | **Snapshots.** One-off want-lists and arcade label checks. Nothing regenerates them |

## Not current

| | |
| --- | --- |
| [parked.md](parked.md) | **Deferred, with a cost.** Work scoped and priced, then put down on purpose |
| [outbox.md](outbox.md) | **Written, not sent.** A RomM bug report and a ScreenScraper credentials request. Sending either is Frank's call |
| [archive/](archive/README.md) | **Finished or superseded.** The rename records, the closed port plan, the original SDL front-end design. Read for *why*, never for *what is true now* |
