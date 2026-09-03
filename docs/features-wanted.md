# Features wanted

Things a retro frontend normally has that this one does not. A menu to choose
from and order. Nothing here is committed to.

Written 2026-08-20 after a survey of what the app already does. Several
candidates were struck out on the spot and are recorded at the bottom, because
"we decided against this" is worth as much as "we want this".

Each entry says what it is, what already exists to build on, and what makes it
hard. **Size** is rough: S is an afternoon, M is a day, L is more than that.

---

## 1. Per-game controller remaps — M

Cores are choosable per game; button layouts are per platform only. A vertical
shooter and a fighting game on the same console want different buttons, and
arcade especially — six-button games and two-button games share a platform.

**Exists:** `.rmp` remap files are already written and deleted per core by
`prepare_tweaks`; the machinery is threaded through the launch path. The rapid
fire work proved it out the hard way.
**Hard:** the control surface. A remap UI is sixteen buttons times four
players, and the useful version is probably "copy the platform's layout, change
two things" rather than a full grid.

## 2. ROM auditing — L

Verify what is on disk against the No-Intro and Redump catalogs: bad dumps,
wrong regions, overdumps, duplicates, files claiming to be something they are
not. A library assembled from a 12 TB drive of unknown provenance has all of
these.

**Exists:** the drive manifest (30,808 games), the arcade probe verdicts
(2,504), and hashes are already computed for save sync.
**Hard:** DAT files are large, versioned and per-system; matching wants CRC or
SHA-1 of the *inner* file for zipped sets, which means reading into archives.
Arcade is a different problem again — MAME romsets are audited by a different
mechanism entirely, and that part is half-built already in `coverage`.

## 3. Attract mode — M

An idle screensaver cycling video previews, the thing that makes a cabinet look
alive rather than parked. Deferred rather than dropped: the video previews in
this library are low resolution, so it would show off the weakest artwork the
app has.

**Exists:** videos are already downloaded per game and the viewer plays them.
**Hard:** not the code — the material. Worth revisiting if the video artwork
is ever replaced with something higher resolution.

## 4. Richer filters — S, partly done

**"Two players or more" shipped in 0.2.442.** 2,731 games on this library; the
6,366 with no player count at all are excluded rather than assumed, or the
filter would let two thirds of the library through and mean nothing.

What is left is the same trick for the other fields already sitting unread in
the metadata blob: genre, developer, and decade from the `year` that is already
on the row. "Shoot-em-ups I have never played" is a question the data can
answer and the menu cannot ask.

**Exists:** `RomView` now carries `players` beside `year` and `rating`, parsed
in Rust so the page does not reparse the same JSON per game on every redraw.
The next field follows the same three lines.
**Hard:** nothing, beyond how many entries a filter menu holds before it wants
its own screen. It is at seven.

---

## Elsewhere

**Cheats** — deliberately deferred. Not dropped, just not now.

**Statistics** — already built. `ui/js/history.js` has hours by console, most
played, and the games picked up and put down. It was listed here in error.

**Manuals** — already partly covered.

**Screenshot gallery** — dropped. "No screenshot not fun."

## Decided against

**Netplay.** RetroArch supports it and this is a single-user, self-hosted
setup. The lobby, the port forwarding and the version-matching are a large
amount of machinery for something with nobody on the other end.

**A self-updater.** The update *check* is built (0.2.441, Settings → About).
Replacing a running binary needs code signing, a rollback path and a story for
the half-written case, and none of that is worth carrying for a tool one person
runs on three machines. It reports and links.

---

## Order

Not yet decided. The cheap ones — statistics, filters — are cheap because the
data is already there and only the presentation is missing; the expensive ones
are expensive because they need data the app does not have.
