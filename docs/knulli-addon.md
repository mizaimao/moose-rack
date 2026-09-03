# The addon

**Current.** `moose-patch`, as shipped.

Frank, 2026-08-26: "SDL app will not be our romm anymore, it's going to be an
'addon' thing to vanilla Kunulli that would survive upgrades and we add manual
patches to it, inclduing our hotkeys, bazels and sync and shit so that's the
scope. Two SDL apps one was romm, archived, and now we need a new patcher one."

So there are two, and only one of them is being built:

- `src-sdl` — the RomM front end. Archived twice. It is
  still in the workspace and still cross-compiles; it is not the thing below.
- the addon — new. A patcher for a stock KNULLI install, with an SDL interface
  because that is the only kind this device can show outside EmulationStation.

## Why a patcher and not a fork

KNULLI updates land as a new image. Everything outside `/userdata` is replaced,
and that is not a detail — it is most of the system:

    /            overlay, writable layer is a 256 MB tmpfs
    /overlay/base squashfs, read-only, replaced wholesale on upgrade
    /boot        vfat, mounted read-only
    /userdata    exFAT, 155 GB, the only thing that survives

Every change made this session had to be placed with that in mind, and each
one is a worked example of where a patch has to live:

| Patch | Where it survives | Why not the obvious place |
|---|---|---|
| RetroArch hotkeys | `knulli.conf` | configgen rewrites `retroarch.cfg` at every launch, so RetroArch's own menu will not hold a change |
| Shader set, per system | `knulli.conf` | same |
| Shader cycling list | `knulli.conf` override of `video_shader_dir` | configgen hard-writes the whole library into `retroarchcustom.cfg` |
| Bezels | `/userdata/decorations` | `/usr/share/knulli/datainit/decorations` is on the squashfs |
| L2+R2 binding | `/userdata/system/configs/multimedia_keys.conf` | `S50triggerhappy` prefers it over `/etc`, which is tmpfs |
| ES ignoring L2/R2 | `/userdata/system/configs/emulationstation/es_input.cfg` | the shipped one is on the squashfs |
| ES's loading logo | blank PNG over `resources/logo.png`, from `/boot/boot-custom.sh` | `/usr` is tmpfs; and it must land before `S31` starts ES |
| Anything at boot | `/userdata/system/custom.sh` | run by `S99userservices` |

The pattern is that KNULLI already provides a `/userdata` override for nearly
everything, and the work is knowing which one. That knowledge is what the
addon is for — not the patching, which is mostly writing a config line.

## Scope

1. Apply and revert each patch individually, and say which are currently on.
2. Survive an upgrade: after a new image, re-apply what was on before.
3. Sync with a RomM server. This is the part KNULLI cannot do at all, and the
   only part that needs the network.

Reverting matters as much as applying. Several patches this session were wrong
and had to come back off, and doing that by hand meant remembering what the
stock value had been.

## Not decided

Whether the addon owns `knulli.conf` blocks by rewriting between markers — the
`## Moose Rack: ...` / `## Moose Rack: ... end` pairs already in there — or keeps its own
file and merges. Markers are simpler and already work; a separate file is
safer against a user editing inside the block.

## Plan

### Shape

A new crate, `src-addon`, in the workspace beside the other two. It depends on:

- `moose-sdl` **as a library** — `gfx`, `text`, `input`, `glass`, `backdrop`,
  `status`, `keyboard` are a working SDL/GLES interface already tuned for this
  exact screen: 640×480, the 1.5 scale, the pad read out of `es_input.cfg`,
  text through cosmic-text with font fallback. That work is done and it would
  be silly to redo it. The RomM-specific modules — `library`, `covers`,
  `rescan`, `iconfetch` — stay behind.
- `moose-rack` — only for the RomM client, and only for the sync part.

### One idea: a patch

    trait Patch {
        fn id(&self) -> &str;
        fn title(&self) -> &str;
        fn why(&self) -> &str;              // one line, shown in the list
        fn state(&self) -> Result<State>;   // Off | On | Changed
        fn apply(&self) -> Result<()>;
        fn revert(&self) -> Result<()>;
    }

`Changed` is the one that earns its keep: KNULLI shipped a new image and the
file underneath is not what we wrote. The addon should say so rather than
silently overwrite or silently skip.

Revert is not a nicety. Several patches this session turned out to be wrong,
and taking them back off by hand meant remembering the stock value.

### Two kinds of patch, and only two

**A marked block in a config file.** `knulli.conf` takes almost everything —
hotkeys, shader set, shader cycling directory, bezel choice. Apply rewrites
between markers, revert deletes between them, state compares:

    ## moose-addon: hotkeys
    ...
    ## moose-addon: hotkeys end

One mechanism, and it is already proven — the blocks in `knulli.conf` right
now have exactly this shape.

**A file we place.** Bezel PNGs, shader presets, `es_input.cfg`,
`multimedia_keys.conf`, `custom.sh`. These need a backup of whatever was there
first, so revert can put it back. `es_input.cfg` and `multimedia_keys.conf` are
both "copy the shipped one, change two things" — that pattern is worth having
once rather than three times.

### The profile

`/userdata/system/moose-addon/profile.toml`: which patches are on, and their
options. This is the answer to *"even on a newly installed KNULLI we can easily
configure and recover all of those customized settings"* — a fresh device means
copy the binary and the profile, run apply-all.

Which means **the binary has to carry its own assets**. A bezel is a PNG and a
shader preset is a text file; if they live beside the binary then recovery is a
directory to remember rather than a file. Embedded, recovery is one file plus
one profile.

### Where it got to

Steps 1 to 3 are done and running on the device.

`patch.rs` is the engine: a patch is a list of steps, a step is either a marked
block in a text config or a file we own, and both can be asked whether they are
already satisfied. That last part is what lets the menu open at what the device
*actually is* rather than at what it ought to be — and lets it say **changed**
when a patch sits at none of its options, which is what a KNULLI update looks
like from in here.

`catalogue.rs` is ten patches, with their bodies and their files compiled in.
`profile.rs` is the recovery story: `--save`, `--restore`, `--status`, all of
which run without a window, because a device that has just been reflashed
cannot launch a windowed app until some of these patches are on.

Two things the device taught the engine, neither of which was guessed:

`/boot` is mounted **read-only**, and `boot-custom.sh` has to live there because
it is the only hook that runs before EmulationStation starts. So a write that
comes back `EROFS` gets one retry with the mount flipped, and the mount is put
back afterwards whichever way it goes.

A patch that fails must not abandon the ones after it. The first restore on the
real device hit that read-only mount and lost every setting that came later in
the file — on a freshly flashed handheld that is the difference between one
thing to fix and ten.

### Credentials

`config.toml` beside the binary, holding only the `[server]` section copied from
the desktop's. Nothing else on that machine belongs on a handheld, and a
smaller file is a smaller thing to lose. `--status` prints the server and which
credential it found, so "cannot reach RomM" and "no patch is on" are one line
apart instead of guesswork.

### Still to do

The three sync actions are drawn but not wired. `moose_rack::savesync` already
has `SaveConflict`, `Keep` and `Summary`, so this is plumbing rather than design.

`launch-evmapy` **is** in the catalogue now, and is the one measured launch
win: 0.93 s, by not restarting a daemon that has nothing to map. It only
inserts a line into a shell script, so it is closer to the config patches than
to replacing anything.

Replacing the launcher itself is still outside the catalogue. That old note
claimed a preforked configgen daemon went from 1241 ms to 7.9 ms; the
measurement was of import cost alone, not of a launch, and a real launch is
4.26 s of which imports are 1.08 s. The work is now `moose-fastlaunch` — see
[fast-launch.md](fast-launch.md) — and it stays out of the catalogue until it
can generate `retroarchcustom.cfg` and that output has been diffed against
configgen's across every system.

### Order

1. The crate, the `Patch` trait, the marked-block mechanism, and two real
   patches — hotkeys and shaders — with apply, revert and state.
2. The file-placement mechanism, with backups, and the rest of the patches.
3. Profile: save, load, apply-all. This is the part that has to work on a
   device that has never seen the addon.
4. The interface. Three screens: patches, sync, status.
5. Sync.

Steps 1–3 are usable from a shell before there is any interface at all, which
is the order that gets it tested.

### Undecided

The name. `moose-addon` is wrong — it patches KNULLI and only one of its jobs is
RomM.

Which half of sync comes first: pushing saves up, pulling saves down, or taking
games offline.

## Favourites and collections

RomM has no per-game favourite. A favourite there is a **collection**, either
one the server flags `is_favorite` or one somebody named with a star — which is
what the nine `★ Best of …` lists on this library are. So "star this game"
means "put it in that collection", and the star reaches every device for free,
because the collection lives on the server.

EmulationStation keeps the same two ideas in two different places, neither of
which is the server:

| Server | On the card |
| --- | --- |
| `★ Best of snes` | `<favorite>true</favorite>` in `/userdata/roms/snes/gamelist.xml` |
| `Arcade Fighting` | `collections/custom-Arcade Fighting.cfg`, one absolute path per line |

`favrun::held_as` decides which by the name. A `custom-*.cfg` is invisible until
its name is also in `CollectionSystemsCustom` in `es_settings.cfg`, so the sync
writes that too — it is the step that gets forgotten and makes a correct file
look like a broken one.

### Why there is a baseline

`/api/sync` is saves only; there is no server-side negotiation for
collections. So the rule is decided on the device: **remember what the last
sync agreed on**, in `favorites-baseline.json` beside the addon.

With it, every difference explains itself. A star the baseline has not seen was
added since, and goes up. A star the baseline has but the card has lost was
taken off here, and comes off the server too. Without it the only safe move is
to merge, and unstarring never travels — you take a star off on the handheld
and the next sync puts it straight back.

Because a star is a **boolean**, a three-way merge has no conflicts. There are
only two values: if both sides moved away from the baseline they moved to the
same place, and they already agree. Nothing here ever needs to ask a person.

Lists that already agree are written into the baseline as well, even though
nothing moves. Otherwise it only ever learns about lists that happened to
differ, and the first star taken off an agreeing list looks like a list that
has never been synced.

### Two things that are not the same name

* **Folders.** The server files SNES under `sfc`, the card under `snes`. Same
  mapping the saves use — `Platform::save_folder`.
* **Multi-disc games.** RomM holds one rom called `Final Fantasy VII (USA)`.
  The card has a *hidden* `.Final Fantasy VII (USA)/` of discs and a
  `Final Fantasy VII (USA).m3u` beside it, and the playlist is what ES shows
  and therefore what ES stars. Matching the plain name skipped every multi-disc
  game silently: starred on both sides, read as on neither.

### Looking before moving

    moose-patch --stars          what it would do, and both sides' counts
    moose-patch --stars-apply    do it

`--stars` prints a line per collection with how many are starred on the card,
how many on the server, and how many of the server's are on this card at all.
That last number matters: the card holds a subset of the library, and a star
for a game that is not here is left alone rather than read as an unstarring —
otherwise a sync would strip the server of every star for every game the
handheld does not carry.

Print the counts even when everything agrees. A matcher that finds nothing on
either side reports agreement exactly as loudly as one that works.


## knulli.conf is first-wins

`knulli-settings-get` scans from the top of the file and returns the **first**
match. A block appended at the end that repeats a key already set further up is
read by nothing.

This is not hypothetical. `never-sleep` wrote

    system.batterysaver.extendedmode=none

underneath the `=suspend` that KNULLI ships on line 319. The app reported the
patch as on, the file contained exactly what the patch said it should, and the
handheld went on suspending after fifteen minutes idle. Nothing looked wrong
anywhere except on the device.

So `set_block` now comments out any earlier line setting a key the block also
sets, keeping the original verbatim on the marker that displaced it:

    ## moose-patch: power hid: system.batterysaver.extendedmode=suspend

`clear_block` puts it back exactly. That is what makes turning a patch off a
real undo rather than a guess at what KNULLI's default was — deleting our line
and walking away would leave the key unset, which is not the same as the value
it shipped with.

Check a patch on the device, not by reading the file it wrote:

    moose-patch --apply charge-awake=ON
    knulli-settings-get system.batterysaver.chargingbypass   # the reader's answer

## Sleep

Three separate mechanisms, and only one of them is automatic:

| What | Where it is decided | Auto? |
| --- | --- | --- |
| Idle dim, then idle suspend | `idlewatcher` → `/etc/idlewatcher/*.d/` hooks | yes |
| Closing the lid | `lid-control`, reading `system.lid` | no |
| Power button | `power-button` → `knulli-suspend` | no |

Only the first checks `/var/run/battery-saver/*.pause`. That is why
**Awake while charging** — `system.batterysaver.chargingbypass=1` — stops the
handheld dozing off on its own while plugged in without taking away either way
of asking it to sleep. KNULLI already ships the whole mechanism; it is off by
default, so no script and no service were needed.

One trap: those idle hooks are `#!/bin/bash` and `check_pause` uses `compgen`.
Run one with `sh` to see what it would do and `compgen` is not found,
`check_pause` fails open, and the hook suspends the device — which is a fine
way to lose the machine you are testing on mid-session.
