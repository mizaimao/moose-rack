# One core, two frontends, three devices

Written 2026-08-24, when the answer stopped being "Flip **or** Thor" and became
both.

The targets:

| device | OS | screen | RAM | frontend |
|---|---|---|---|---|
| Miyoo Flip | KNULLI (Batocera 42, BSP 5.10) | 640x480 | 1 GB | SDL2 |
| AYN Thor | Android 13 | 1080x1920 + 1080x1240 | 8–16 GB | Tauri |
| Retroid Pocket Mini V2 | Android 13 | 1240x1080 | 6 GB | Tauri |

The two Android devices are the same problem twice — the RP Mini V2's panel is
the Thor's bottom screen rotated. One Android build serves both.

## The architecture already exists. Do not rebuild it.

"One logic, two frontends" is what this repo already is, and it has been since
0.2.503 moved the rules out of the webview:

| | lines | role |
|---|--:|---|
| `src/` | ~24,000 | the core: cache, api, gamelist, gamesort, gamefilter, gridnav, binds, padpoll, launch, esde, coremap, saves… |
| `src-tauri/` | 3,800 | webview frontend |
| `src-sdl/` | 5,000 | SDL2 frontend |

`src-sdl/src/library.rs` already opens the same `Cache`, and already drives
`gamelist`, `gridnav`, `gamesort`, `gamefilter` and `layout`. The shared core
is real and it is load-bearing.

**The one real gap: SDL cannot launch a game.** `launch::plan` has exactly
three callers — `src/main.rs` (CLI), `src/tui.rs`, and `src-tauri`. The SDL
front end browses and stops. That is the hole to fill, and on KNULLI it is the
*easy* version of the problem.

## The axis of difference is the device, not the frontend

This is the thing worth getting right before writing any code.

SDL-vs-webview is a **rendering** choice. It is not what actually differs
between these three targets. What differs is four questions:

1. **Where does the library live?**
2. **Where does media live?**
3. **How is a game launched?**
4. **How do brightness, battery and the lid work?**

None of those four have anything to do with how a card is drawn. Today they are
answered by `#[cfg(target_os = …)]` scattered across `src/` — 37 macOS, 26
Windows, 1 Linux, **0 Android** — and every chain falls back to
`not(any(macos, windows))`.

That fallback is a trap in both new directions:

* **Android compiles and takes the Linux branch**, so it will look for
  RetroArch at Linux paths and write config where Linux writes it. Builds fine,
  behaves wrong.
* **KNULLI is also `target_os = "linux"`**, and it is not desktop Linux. Same
  cfg, different answers needed. `target_os` cannot tell them apart *at all*.

So the axis is wrong. It wants to be an explicit platform choice.

### A platform module

One implementation per target, selected by a **cargo feature**, not by
`target_os` — because KNULLI and desktop Linux share a `target_os` and need
different answers:

```rust
pub trait Platform {
    /// Where the ES-DE-shaped library and media are.
    fn layout(&self) -> esde::Layout;
    /// Run a plan. Subprocess on desktop and KNULLI; Intent on Android.
    fn launch(&self, plan: &launch::Plan) -> Result<()>;
    /// Hardware, where the device has any.
    fn brightness(&self) -> Option<Brightness> { None }
    fn battery(&self) -> Option<Battery> { None }
}
```

`launch.rs` (474 lines) already plans without spawning, which is exactly the
shape this needs — it stays untouched and only the executor changes. It is the
single best piece of luck in this whole port.

Implementations: `MacOs`, `Windows`, `Knulli`, `Android`. On KNULLI most of the
hardware methods shell out to `knulli-brightness`, `knulli-battery-check` and
friends rather than poking sysfs — see `handheld-os.md`.

## The ES-DE format question is already answered in the code

You asked how to organize files on the Flip while still conforming to ES-DE.
`src/esde.rs` already has the mechanism:

```rust
/// Derive the layout from an ES-DE data directory, allowing an explicit
/// ROMs directory since ES-DE keeps that separate and configurable.
pub fn new(esde_root: &Path, roms: Option<&Path>) -> Self
```

**ES-DE already separates ROMs from media by design.** That is the whole answer.

### Android — nothing to organize

Point `Layout` at the on-device ES-DE install and read it where it lies. The
core map was built from ES-DE's *Android* system list in the first place, so
the naming already matches better here than anywhere else.

### KNULLI/Flip — split, and nothing moves

Batocera colocates media with ROMs under `/userdata/roms/<system>/`. ES-DE
separates them. Reconciling those by *moving ROMs* would break KNULLI's own
EmulationStation and its configgen. So do not move ROMs:

| | path |
|---|---|
| ROMs | `/userdata/roms/<system>/` — **unmoved**, KNULLI still works |
| media | `/userdata/ES-DE/downloaded_media/<system>/<type>/<stem>.<ext>` |
| gamelists | `/userdata/ES-DE/gamelists/<system>/gamelist.xml` |

`Layout::new("/userdata/ES-DE", Some("/userdata/roms"))` and it is done. That
conforms to the ES-DE media convention exactly, adds a tree KNULLI ignores, and
does not touch a single ROM. `/userdata` is persistent (`mmcblk1p4`, 235 G,
mode 0777) and survives OS-image updates.

### The one genuine piece of work: system names

Batocera's system directory names are not ES-DE's. `esde.rs` already has
`SYSTEM_ALIASES` for exactly this problem — it exists because a real install
called it `genesis` where the map says `megadrive`, and that one alias was
worth 942 games.

That table has 14 entries and was built against an ES-DE Android export. **It
needs auditing against Batocera 42's actual system set**, which is sitting on
the device at `/userdata/roms`. That is a concrete, finite, testable task and it
is the thing most likely to silently drop a console if skipped.

## Launching, per device

| device | mechanism | state |
|---|---|---|
| macOS / Windows | subprocess, `Command::new` | done |
| KNULLI / Flip | subprocess — `/usr/bin/retroarch`, 99 cores in `/usr/lib/libretro` | **easy, not wired** |
| Android | Intent to `com.retroarch` | the hard one |

KNULLI is the easy case and it is closer than it looks: the binary and the
cores are already on the device, and `retroarch.rs` already knows how to build
the command. It mostly needs a `Knulli` platform that answers "where is
RetroArch" and "where are the cores", plus wiring SDL to call `launch::plan`.

Android is the one that costs. `--appendconfig` and `--set-shader=` have no
Intent equivalent, so per-game overrides must be merged into one composed
`retroarch.cfg` per launch. See `android-port.md` — that is the item to scope
before anything else.

## What "minimal" means on the Flip

1 GB of RAM, 640x480, and a Mali-G52. Cut:

* **Video and PDF.** A webview gives `<video>` and PDF free; SDL gives neither.
  Manuals and gameplay videos are reasonable to drop here and keep on Android.
* **The console-art grid**, probably. 640x480 wants a list more than a wall.
* **Backdrop shader** — keep it. `backdrop.js` is WebGL2 with GLSL fragment
  shaders and maps 1:1 onto Mali-G52's GLES 3.2; `handheld-frontend.md` already
  worked this out. It is one of the few effects that gets *cheaper* in SDL.
* **Text is the quiet cost.** Font fallback across a library with Japanese sets
  is not optional and SDL_ttf does not do it for free.

None of that applies to Android, which has 6–16 GB and a real browser. **Do not
let the Flip's constraints shape the Android build** — that is how one codebase
becomes two mediocre frontends instead of one good one and one small one.

## Order of work

1. **The platform module.** Everything else hangs off it, and it is the thing
   that stops `target_os` lying about KNULLI. Do it first even though nothing
   visible happens.
2. **Audit `SYSTEM_ALIASES` against Batocera 42** on the device. Cheap, and it
   gates whether the Flip sees your whole library.
3. **Wire SDL to `launch::plan`** with a `Knulli` platform. This makes the Flip
   a working device end to end and it is the smallest remaining step to that.
4. **Scope the Android config merge** — the `--appendconfig` problem. Do not
   start the Android build before this is understood.
5. **Android toolchain and browsing** — `tauri android init`, NDK 28+, JDK 17.
   Nearly free once 1 and 4 are settled.
6. **Android launching.** The rest of the budget.

Steps 1–3 make the Flip real. Steps 4–6 make the Thor and the RP Mini real. The
shared core means neither path re-implements the other's work — which is the
thing that was worth checking before committing to both, and it holds.
