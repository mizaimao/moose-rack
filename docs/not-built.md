# Designed, not built

**Designed, not built.** A menu, not a queue. Nothing here is promised.

Three pieces of work that were scoped properly and then stopped before any
code. They are kept because the design is the expensive part and it is done;
picking one up should not start from a blank page.

None of this is a queue. Nothing here is promised.

Was `attract-mode.md`, `cartridge-shelf.md` and `features-wanted.md`.


---

## Attract mode, as EmulationStation does it

*Was `attract-mode.md`.*

The arcade-cabinet screensaver: the frontend sits idle, then starts cycling
game videos or screenshots, and a button press launches whatever is on screen.
Not built here. This is the design read out of the two implementations worth
copying, so that starting it does not start with a survey.

Sources, both read at 2026-08-27:

| | |
| --- | --- |
| KNULLI / Batocera | `knulli-cfw/batocera-emulationstation`, a fork of `batocera-linux/` with the same layout. `es-app/src/SystemScreenSaver.{h,cpp}`, `es-core/src/Window.cpp`, `es-core/src/Settings.cpp` |
| ES-DE | `gitlab.com/es-de/emulationstation-de`, `es-app/src/Screensaver.cpp` |

They are the same design with different spellings. Where they differ, it is
noted.

### The trigger is one counter, not a per-view thing

`Window::mTimeSinceLastInput` accumulates every frame. When it passes
`ScreenSaverTime` the window calls `startScreenSaver()`. Any input at all calls
`cancelScreenSaver()`, which zeroes it.

That is the whole gate. It lives at window level, above every view, so no
screen has to know it exists. `ScreenSaverTime` defaults to five minutes and
**`0` means off** — worth keeping, because it is the obvious way to expose a
disable switch without a second setting.

### Five modes, of which two are attract mode

`ScreenSaverBehavior`, default `dim`:

| | |
| --- | --- |
| `dim` | fade the current screen down |
| `black` | fade to black |
| `random video` | **attract mode**, game videos |
| `slideshow` | **attract mode**, game images |
| `suspend` | hand off to the OS |

ES-DE calls the setting `ScreensaverType` and drops `suspend`.

### The state machine

    INACTIVE → FADE_OUT_WINDOW → FADE_IN_VIDEO → SCREENSAVER_ACTIVE

Two fades because it is a crossfade: the old screen goes out over `FADE_TIME`
(500 ms) while the first piece of media comes in. Once `SCREENSAVER_ACTIVE`, a
timer swaps media and the state does not change again until input arrives.

    ScreenSaverSwapVideoTimeout    30000 ms
    ScreenSaverSwapImageTimeout    10000 ms

### Picking a game

Two steps, and the second one is the part worth stealing.

**Build the candidate list once.** `countGameListNodes()` walks every system,
skips collections, `IMAGEVIEWER` and `PLATFORM_IGNORE`, dedupes through a set,
and keeps games whose video path (or image path) is non-empty. The result is
cached behind a `Loaded` flag and only rebuilt when it empties.

**Then sample without replacement.** `pickRandomGameMedia()` takes a random
index and *erases that entry from the list*. Nothing repeats until every game
with media has been shown, and when the list runs dry the flag flips and it
rebuilds. Perhaps twenty lines, and it is the difference between attract mode
feeling curated and feeling broken — a naive random pick shows the same three
games in a row often enough to be noticed.

ES-DE does the same and adds one guard: it keeps `mPreviousGame` and re-rolls
if the fresh list hands back the game that was just on screen, so the seam
between two cycles does not repeat either.

ES-DE also has `ScreensaverVideoOnlyFavorites` / `ScreensaverSlideshowOnlyFavorites`,
which given that favourites already sync here is close to free.

### The trap ES-DE hit, which this app would hit harder

From `generateImageList()`, verbatim:

> This method of building an inventory of all image files isn't pretty, but to
> use the `FileData::getImagePath()` function leads to unacceptable performance
> issues on some platforms like Android that offer very poor disk I/O
> performance. To instead list all files recursively is much faster as this
> avoids `stat()` function calls which are very expensive on such problematic
> platforms.

So: do not build the candidate list by asking each game whether it has media.
List the media directories recursively once and match names against it. This
matters more here than there — the Thor is Android, and the library is large.

### Launching from the screensaver

`ScreenSaverControls`, default true. On input during attract mode,
`launchGame()` sets the gamelist cursor to the game being shown and launches
it. With the setting false it goes to that game's list instead and waits.

Both frontends do this. It is the thing that makes it attract mode rather than
a screensaver, and it is about fifteen lines.

### Not burning the battery

`getNextUpdateTimeout()` tells the main loop how long it may sleep:

| Situation | Timeout |
| --- | --- |
| Fading, or a video is playing | `0` — render continuously |
| Black or dim | 100 ms, only to poll input |
| Slideshow | until the next image swap |
| Slideshow with the clock on | 100 ms, for the seconds |

Without this the frontend renders at full rate to show a still image. On the
Flip that is the difference between attract mode being usable and being a
reason to turn the device off.

### Overlays

Optional, all of them: marquee, game name, system name, date and time with
`strftime` formats, and a decoration frame. `ScreenSaverGameInfo` takes
`never` / `always` / `start & end`.

### What to build first

The two pieces that are actually load-bearing:

1. An idle counter at window level that any input resets.
2. A cached list of games with media, sampled without replacement.

Everything above those is presentation and can be added one setting at a time.


---

## The cartridge shelf

*Was `cartridge-shelf.md`.*

Games shown as the physical thing — a 3D cartridge with its real label, turned
towards you, with a sound and an insert animation when you pick one. Not built.
Scoped 2026-08-27 so that picking it up starts from facts rather than a survey.

### Where the idea came from

**Socket**, at <https://depmots.com/socket> — a frontend built for small curated
collections, aiming to "mimic the feeling of rummaging in your handheld cover
when you were a kid". Each platform gets its own 3D model with the original
sticker, its own sounds, and its own insert animation.

Read off its own screenshots: a carousel of N64 cartridges, the centre one large
and tilted to a low angle with the connector pins visible, the neighbours
smaller and face-on. Top bar with clock, L/R platform switcher and battery.
Bottom bar with the selected game's name. It is real 3D — perspective, lighting
on the plastic, modelled pins — not a tilted image.

### The expensive part is already done

The art is the cost of a feature like this, and it is already on disk.

    library/downloaded_media/<platform>/physicalmedia/
    6,842 files · 23 platforms · 2.3 GB

`physicalmedia` is ES-DE's name for ScreenScraper's "support" media — the
cartridge or disc itself, as opposed to the box. `src/media.rs` already fetches
and stores it, so nothing new has to be scraped.

Coverage tracks `covers` almost one to one, so wherever there is box art there
is usually cartridge art too:

| Platform | physicalmedia | covers |
| --- | --- | --- |
| snes | 463 | 471 |
| sfc | 487 | 489 |
| pcengine | 288 | 288 |
| ngc | 55 | 55 |

### Two facts about those images that decide the approach

**They are the whole cartridge, not the label.** Grey plastic shell, label,
transparent background, drawn flat and face-on. Good for a 2D plane — the shell
is already rendered for you. Awkward for a true 3D model, where the shell in the
image would fight the shell in the geometry.

**They are template renders, uniform per platform.** Every one of the 42 N64
files is 600×386. All 764 GBA are 600×355. All 99 PSX are 600×600. snes,
megadrive and gb are about 90% one size, the remainder being variant shells —
PAL against NTSC and so on.

Which means: if you go true 3D and need the label alone, that is **one crop
rectangle per platform, 23 of them** — not a per-game job. Worth checking before
assuming the images are unusable as textures.

### Three tiers

| | Effort | What it gets you |
| --- | --- | --- |
| **A. 2.5D in CSS** | days | The PNGs as-is on 3D planes: carousel, tilt, slide-in, per-platform sounds |
| **B. Extruded box in WebGL** | 1–2 weeks | Real depth and lighting. A rounded box per cartridge, a cylinder for discs. No modelling |
| **C. Per-platform glTF models** | the actual project | Socket's approach. Mostly art, not code |

**Start at A.** The webview already has the primitives:
`ui/style.css` sets `perspective: 900px` and `transform-style: preserve-3d` on
the card grid, and `ui/js/tilt.js` already drives `rotateX`/`rotateY` from
pointer position. A is a carousel and an animation on top of what exists, not a
new rendering layer.

**A's one real limit** is that a plane vanishes edge-on. Socket's low-angle
cartridge with the pins showing is not reachable in CSS; that needs B.

**B is the honest sweet spot** if A reads as flat. three.js has to be vendored
into `ui/` — no CDN, same rule as the Lucide icons — and about 600 KB is nothing
against the 106 MB of WebKit the bundle already carries. A rounded box with the
`physicalmedia` PNG on the front face and a flat colour on the sides gets most
of the depth with no modelling at all. The disc platforms — psx, dc, psp, ngc —
need a different primitive.

**C is only worth it if this becomes the point of the app.**

### Traps

**Do not build the candidate list by asking each game for its media path.** This
is ES-DE's own warning, from `generateImageList()` in `Screensaver.cpp`: it is a
`stat()` per game, and on Android that is slow enough that they abandoned the
approach and list the media directories recursively instead. The Thor is
Android and this library is large, so it applies here harder than it does to
them. The attract-mode section above carries the same note.

**Coverage is not total.** There has to be a fallback — cover art on a generic
shell for the platform — or the shelf will have holes in it.

**Desktop and Android only.** The Flip addon is SDL and draws its own interface;
none of this reaches it. `src-sdl` has a GL renderer if that ever changes, but
it is a separate implementation, not a port of this one.

### What to build first

1. A carousel over the existing card grid, using `physicalmedia` where it
   exists and falling back to `covers`.
2. The slide-in, on selection.
3. Per-platform sounds.

Sounds are trivial code and need sourcing; everything else above is
presentation and can be added one setting at a time.


---

## Features wanted

*Was `features-wanted.md`.*

Things a retro frontend normally has that this one does not. A menu to choose
from and order. Nothing here is committed to.

Written 2026-08-20 after a survey of what the app already does. Several
candidates were struck out on the spot and are recorded at the bottom, because
"we decided against this" is worth as much as "we want this".

Each entry says what it is, what already exists to build on, and what makes it
hard. **Size** is rough: S is an afternoon, M is a day, L is more than that.

---

### 1. Per-game controller remaps — M

Cores are choosable per game; button layouts are per platform only. A vertical
shooter and a fighting game on the same console want different buttons, and
arcade especially — six-button games and two-button games share a platform.

**Exists:** `.rmp` remap files are already written and deleted per core by
`prepare_tweaks`; the machinery is threaded through the launch path. The rapid
fire work proved it out the hard way.
**Hard:** the control surface. A remap UI is sixteen buttons times four
players, and the useful version is probably "copy the platform's layout, change
two things" rather than a full grid.

### 2. ROM auditing — L

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

### 3. Attract mode — M

An idle screensaver cycling video previews, the thing that makes a cabinet look
alive rather than parked. Deferred rather than dropped: the video previews in
this library are low resolution, so it would show off the weakest artwork the
app has.

**Exists:** videos are already downloaded per game and the viewer plays them.
**Hard:** not the code — the material. Worth revisiting if the video artwork
is ever replaced with something higher resolution.

### 4. Richer filters — S, partly done

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

### Elsewhere

**Cheats** — deliberately deferred. Not dropped, just not now.

**Statistics** — already built. `ui/js/history.js` has hours by console, most
played, and the games picked up and put down. It was listed here in error.

**Manuals** — already partly covered.

**Screenshot gallery** — dropped. "No screenshot not fun."

### Decided against

**Netplay.** RetroArch supports it and this is a single-user, self-hosted
setup. The lobby, the port forwarding and the version-matching are a large
amount of machinery for something with nobody on the other end.

**A self-updater.** The update *check* is built (0.2.441, Settings → About).
Replacing a running binary needs code signing, a rollback path and a story for
the half-written case, and none of that is worth carrying for a tool one person
runs on three machines. It reports and links.

---

### Order

Not yet decided. The cheap ones — statistics, filters — are cheap because the
data is already there and only the presentation is missing; the expensive ones
are expensive because they need data the app does not have.
