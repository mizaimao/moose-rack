# Port plan — Flip (SDL) and Android (Tauri)

Written 2026-08-24, revised the same day with Frank's decisions. Design is in
`one-core-two-frontends.md`; Android specifics in `android-port.md`; the Flip's
OS in `handheld-os.md`.

**Nothing here has been built yet.** Sizes are estimates, and every "already
works" claim is from reading the code, not from running it on the hardware.

## The decisions, settled

* **Five schemes**, one per target: `macos`, `windows`, `linux`, `knulli`,
  `android`. Cargo features, not `target_os` — because KNULLI *is*
  `target_os = "linux"` and is not desktop Linux. Tuning `knulli` freely is the
  point of the split.
* **Flip first.** A simple SDL2 frontend, and a full controller scheme defined
  there and **inherited by Android later**. That scheme is a shared deliverable,
  not a Flip detail — it gets its own step.
* **KNULLI's EmulationStation keeps working.** So ROMs never move.
* **RP Mini V2 needs no separate work.** Same Android build as the Thor. The
  Thor's two panels — 1080x1920 and 1080x1240 — cover both resolutions for
  layout debugging, and the RP Mini's 1240x1080 is the Thor's bottom panel
  rotated.

## Answered: does Android have the same file layout as desktop?

Asked before Step 1 because it decides the platform module's shape. It splits
in two and the answers differ.

**The library: yes, identical — and already ES-DE-shaped.** Everything hangs
off one configurable `library.local_root`:

| | |
|---|---|
| `<root>/roms/<system>/` | ROMs |
| `<root>/downloaded_media/<system>/<type>/<stem>.<ext>` | **ES-DE's exact convention** |
| `<root>/system/` | BIOS |
| `<root>/themes/` | ES-DE themes |
| `<root>/retroarch-user.cfg` | appended at launch |

On Android, point `local_root` at the on-device ES-DE install. Nothing
structural changes. This is the good half.

**The app's own files: no.** Three breaks, each with a fix:

1. **`datadir::choose()` anchors from `current_exe()`**, which on Android is the
   zygote (`/system/bin/app_process64`) and nowhere near the data. Its steps 1
   and 3 are dead there. Everything else — `"config.toml"`, `"cache.sqlite3"`,
   `"data/…"` — is a bare relative path that works only because `anchor()`
   calls `set_current_dir()`. `chdir()` does work on Android, so the model
   survives; it must anchor to the app's private files dir instead.
2. **`data/arcade-names.json` is not embedded.** 2.1 MB, read from a relative
   path at sync time, and it is what renames 2,504 arcade games.
   `esde-core-map.json` is fine — `CoreMap::load_or_embedded` compiles it in —
   but this one is not. On Android it ships as a Tauri resource.
3. **Reading ES-DE's folder on shared storage by path** needs
   `MANAGE_EXTERNAL_STORAGE` or SAF. A permission, not a layout — but it decides
   whether "just point `local_root` at it" actually works, so it is verified in
   Step 6 before anything is built on the assumption.

---

# Step 0, closed on the device (2026-08-24)

Both remaining Step 0 items are answered, **measured on the Flip itself** at
`10.10.10.187` (`knulli.local`). No SSH key was set up — password auth driven
through `expect`, which is what the earlier day was lost to. The device answered
every command first time.

## `ls /userdata/roms` — 183 folders, listed in `knulli-roms-folders.txt`

Batocera creates the full set its build supports whether or not games are in
them, so this is the real directory set the scanner will meet.

## Step 2's answer, and it is worse than the estimate

Diffing those 183 against `sys_to_slug` (the core map's 30 systems, plus the 14
`SYSTEM_ALIASES`): **25 resolve, the rest are silently skipped.** `scan()` in
`esde.rs` pushes an unknown directory onto `skipped` and continues — no error,
no count, nothing on screen. That is *why* it is silent, and why a missing
alias reads as "the console isn't in my library" rather than as a bug.

Most of those 197 are ports, engines and systems nobody here owns. Diffing
against **the actual library** (`cache.sqlite3`, 9,371 games) is the number
that matters:

| romm slug | games | reachable? | the folder that is actually there |
|---|--:|---|---|
| arcade | 2,504 | no | `fbneo`, `mame`, `neogeo` |
| sfc | 506 | no | only `snes` |
| famicom | 416 | no | only `nes` |
| wonderswancolor | 73 | no | `wswanc` |
| wonderswan | 70 | no | `wswan` |
| ngc | 65 | no | `gamecube` |

**3,634 of 9,371 games — 39% of the library — scan to nothing.** The plan
guessed "two thirds"; the true figure is lower but the shape is the same, and
`arcade` alone is 2,504 of it. Every folder in the right-hand column above
exists on the device and holds the games; nothing is missing from the Flip, only
from the table that names it.

Four are plain missing aliases, and the fix is four lines in `SYSTEM_ALIASES`:

| add | → | recovers |
|---|---|--:|
| `fbneo` | `arcade` | 2,504 |
| `wswanc` | `wonderswancolor` | 73 |
| `wswan` | `wonderswan` | 70 |
| `gamecube` | `ngc` | 65 |

Worth adding at the same time, though no games ride on them today: `ngpc` →
`neo-geo-pocket`, and KNULLI spells two existing aliases differently — it uses
`3ds` (alias says `n3ds`) and `pico8` (alias says `pico-8`). Three of the 14
current aliases — `genesis`, `n3ds`, `pico-8` — are ES-DE Android names that
match no KNULLI directory at all; they are dead weight here, not bugs.

**`sfc` and `famicom` are not an alias problem and must not be "fixed" as one.**
KNULLI has no `sfc` or `famicom` directory — it has `snes` and `nes`, and those
already resolve to the `snes` and `nes` slugs. So 922 Super Famicom and Famicom
games have to share a directory with their western sets, and the scanner would
label them `snes`/`nes` against RomM's `sfc`/`famicom`. **This is a decision
about how the library is laid out on the card, not a table entry — it needs
Frank's call before Step 2 is written.**

## RetroArch discovery — half the "no new code" claim is wrong

Read from `retroarch.rs` in this repo. The binary side holds:

| | KNULLI has | verified in code |
|---|---|---|
| root | `/usr` | `CANDIDATE_ROOTS` line 106 — yes |
| binary | `/usr/bin/retroarch` | `binary_candidates` tries `<root>/bin/retroarch` — yes |

**`data_dir()` does not.** On Linux it returns `$XDG_CONFIG_HOME/retroarch`,
else `~/.config/retroarch`. KNULLI sets `HOME=/userdata/system`, so that
resolves to `/userdata/system/.config/retroarch` — **and Batocera keeps
RetroArch's files in `/userdata/system/configs/retroarch/` instead.** It is
load-bearing for five things: `cores_dir()`, `config/`, `shaders/`,
`autoconfig/` and `retroarch.cfg` itself.

`cores_dir()` happens to survive — `first_existing` falls through the missing
XDG `cores` directory to `/usr/lib/libretro`, which is in its fallback list. The
other four do not. So `Knulli::retroarch()` is **not** the thin wrapper Step 4
assumes: it must override `data_dir()`. Small, but it is new code, and finding
it during Step 4 debugging on a 640x480 screen would have been expensive.

**Confirmed on the device:** `/userdata/system/.config/retroarch` **does not
exist**; `/userdata/system/configs/retroarch/` does, holding `config/`,
`autoconfig/`, `cores/`, `shaders/` and the rest. `/usr/bin/retroarch` is there
(15.7 MB) and `/usr/lib/libretro` holds **99 cores**. `$HOME` really is
`/userdata/system`. So `cores_dir()` does fall through to the right place by
luck, and the other four really are pointed at a directory that is not there.

---

# Flip first — Steps 0 to 4

## Step 0 — probe the Flip

**Why first.** `handheld-os.md` ends with a "still to probe" list because the
device auto-suspended mid-session. Three of those feed Step 1 directly, and one
extra command feeds Step 2. Designing without them is designing against guesses.

**Do:**
- `/proc/bus/input/devices` — event nodes for buttons and joypad.
- `/sys/class/backlight/*` — path and `max_brightness`.
- `/sys/class/power_supply/*` — node names, capacity/status mapping.
- Hall/lid switch device and event.
- **`ls /userdata/roms`** — the actual Batocera system directory names. Input to
  Step 2, and it is one command.
- `retroarch --version`, `ls /usr/lib/libretro | head` — confirms the discovery
  assumption in Step 3.

**Done when:** those six outputs are in `handheld-os.md`.

**CLOSED 2026-08-24.** The four hardware items are in `handheld-os.md`; the
other two were answered from source without touching the device — see below.

**Size:** an hour, mostly waiting on the device.

**Risk:** none. Cheapest step, de-risks two others.

---

## Step 1 — the five-scheme platform module

**Why first among the code.** `target_os` cannot express these targets. KNULLI
and desktop Linux share one; Android falls through to the same
`not(any(macos, windows))` branch. Both new targets compile *today* and would
behave wrong — RetroArch looked for at the wrong paths, config written where
Linux writes it. **Build-and-be-wrong is worse than build-and-fail**, and every
later step inherits the mistake.

**Touches:** a new `src/platform/` — `mod.rs`, `macos.rs`, `windows.rs`,
`linux.rs`, `knulli.rs`, `android.rs`. Then 51 `cfg` sites move into it:
`retroarch.rs` (28), `theme.rs` (7), `padprofile.rs` (6), `cores.rs` (4),
`macdisplay.rs` (4), `tweaks.rs` (2).

**Shape:**

```rust
pub trait Platform {
    fn data_root(&self) -> PathBuf;          // fixes datadir break #1
    fn layout(&self) -> esde::Layout;
    fn retroarch(&self) -> Option<RetroArch>;
    fn launch(&self, plan: &launch::Plan) -> Result<()>;
    fn brightness(&self) -> Option<Brightness> { None }
    fn battery(&self) -> Option<Battery> { None }
}
```

**What makes this cheap:** `launch.rs` (474 lines) already plans without
spawning. `Plan` is the seam and it already exists — only the *executor* moves.

On KNULLI, prefer the `knulli-*` / `batocera-*` helper CLIs over raw sysfs for
brightness and battery; `handheld-os.md` lists them.

**Done when:** `cargo test` passes unchanged, the macOS app behaves identically,
and all five features compile.

## DONE 2026-08-24 — `src/platform/`

`mod.rs` plus the five schemes. Selection is by Cargo feature with the host's
`target_os` as the fallback, so a plain `cargo build` on the Mac selects `macos`
exactly as before and the two schemes `target_os` cannot reach are opt-in:
`cargo check --features knulli`.

All five compile; **563 tests pass under every scheme** (564 under `knulli`,
which has one extra gated test), and the default build is unchanged.

Wired through the module so far, chosen because these are the answers that were
actively *wrong* rather than merely per-OS:

| was | now |
|---|---|
| `CANDIDATE_ROOTS` const, `cfg`-gated three ways | `retroarch_roots()` |
| `data_dir()`, three `cfg` branches | `retroarch_data_dir()` — **the KNULLI fix** |
| `cores_dir()`'s hardcoded fallback list | `core_dirs()` |
| `SYSTEM_ALIASES` only | plus `system_aliases()`, per device |
| `NOT_SYSTEMS` only | plus `ignored_systems()`, per device |

Every KNULLI path in the scheme was checked on the device and exists, with one
deliberate exception: `/userdata/ES-DE` is the tree *we* create, so it is absent
until something writes to it.

**Not yet moved:** the `cfg` sites in `theme.rs` (7), `padprofile.rs` (6),
`cores.rs` (4), `macdisplay.rs` (4) and `tweaks.rs` (2). They are per-OS in the
way `target_os` actually models correctly — font directories, driver
directories, and macOS-only window shaping — so moving them buys tidiness
rather than correctness. `macdisplay.rs` is macOS by nature and should probably
stay as it is. Do these when a scheme needs to disagree with them, not before.

**One thing to decide before Step 4.** Batocera has already scraped this
device: `/userdata/roms/<system>/gamelist.xml` and `<system>/images/` are
populated — 940 images under `megadrive` alone. The layout above deliberately
ignores all of it and builds a parallel ES-DE tree. That is the documented
decision and the code follows it, but it means re-downloading artwork that is
already on the card.

**Size:** 2–4 days. The largest mechanical step, and **no new behavior** — if
anything changes on the Mac, it is a bug.

**Risk:** moving 51 `cfg` sites is where a silent regression hides. Do it as
pure motion, lean on `cargo test`.

---

## Step 2 — audit `SYSTEM_ALIASES` against Batocera 42

**Why.** `esde.rs` maps ES-DE system directory names onto RomM slugs. The table
has 14 entries and was built from an ES-DE **Android** export; Batocera's names
are its own. A missing alias does not error — it **silently skips a console**.
The file's own comment records that `genesis` → `megadrive` alone was worth 942
games.

**Touches:** `SYSTEM_ALIASES` and `NOT_SYSTEMS` in `src/esde.rs`, plus tests.

**Done 2026-08-24 — see *Step 0, closed from source* above for the result: 39%
of the library scans to nothing, four aliases recover 2,712 of it, and `sfc` /
`famicom` need a layout decision first.** The original instruction follows.

**Do:** diff `ls /userdata/roms` (Step 0) against the table and the core map.
Every directory lands in one of three buckets: known system, new alias, or
deliberate non-system (`NOT_SYSTEMS` already covers `0_BIOS`, `bios`, `ports`,
`SourcePorts`, `Ports`).

**Done when:** a test asserts the Batocera 42 directory set maps with nothing
unaccounted for, and the system count found on the device matches the count
expected.

**Size:** half a day, mostly reading.

**Risk:** low effort, high value. Skipping it is what makes the Flip show two
thirds of your library and still look like it works.

---

## Step 3 — the controller scheme, defined on the Flip

**Promoted to its own step** because it is a shared deliverable, not a Flip
detail: Android inherits it wholesale, and the Flip is the honest place to
define it because it has *no pointer at all*. Anything the pad cannot reach on
the Flip is a thing that will be unreachable on the Thor too.

**Most of it already exists, in `src/`, already shared:**

| | lines | what it settles |
|---|--:|---|
| `binds.rs` | 596 | keys and buttons, stored in `config.toml` |
| `padpoll.rs` | 436 | deadzones, repeat timing, dominant-axis rule, settle lock |
| `gridnav.rs` | 412 | movement through a wall of equal cards |
| `focusring.rs` | 400 | reaching everything that is *not* a grid — tabs, header buttons, menus |

`focusring.rs` was written for exactly this and says so in its header: "on an
Android handheld there is no pointer at all, so anything the pad cannot reach
may as well not be drawn."

**So the work is not defining the scheme — it is proving it.** The SDL frontend
is the first consumer that has no mouse to fall back on, so it is where the
gaps show. Expect to find screens `focusring` does not yet cover.

**Done when:** every screen in the SDL frontend is reachable and operable with
the pad alone, and the rules that made it so live in `src/` rather than in
`src-sdl/`.

**Size:** 3–5 days, and it is mostly discovering gaps rather than writing new
rules.

**Note:** `ui/js/gamepad.js` is the one deliberate duplicate — it polls at
120 Hz inside `requestAnimationFrame` and a round trip per frame cannot be made
fast enough. `padpoll.rs` stays the definition it is a copy of. Keep that
arrangement; do not try to unify it here.

---

## Step 4 — wire SDL to `launch::plan`, with the `knulli` scheme

**This is what makes the Flip real.** `src-sdl` browses and stops:
`launch::plan` has exactly three callers — CLI, TUI, Tauri — and SDL is not one
of them.

**Smaller than it looks.** Reading `retroarch.rs`, KNULLI's install should be
discovered with **no new code**:

| | KNULLI has | the code already looks there |
|---|---|---|
| root | `/usr` | `CANDIDATE_ROOTS` includes `/usr` |
| binary | `/usr/bin/retroarch` | `binary_candidates` tries `<root>/bin/retroarch` |
| cores | `/usr/lib/libretro` (99) | `cores_dir()` includes `/usr/lib/libretro` |

**Closed 2026-08-24 — and it only half holds.** The binary is found; `data_dir()`
is wrong on KNULLI and drives four more things. `Knulli::retroarch()` must
override it. See *Step 0, closed from source* above.

**The ES-DE layout on KNULLI**, per the decision above — ROMs never move:

| | path |
|---|---|
| ROMs | `/userdata/roms/<system>/` — unmoved, KNULLI's ES and configgen keep working |
| media | `/userdata/ES-DE/downloaded_media/<system>/<type>/<stem>.<ext>` |
| gamelists | `/userdata/ES-DE/gamelists/<system>/gamelist.xml` |

`Layout::new("/userdata/ES-DE", Some("/userdata/roms"))` and it is done.
`/userdata` is persistent (`mmcblk1p4`, 235 G, 0777) and survives OS updates.

**Done when:** a game launches from the SDL frontend on the Flip, exits, and
returns to the frontend.

**Size:** 3–5 days, most of it on-device debugging rather than code.

**Risk:** the frontend swap. `handheld-os.md` gives two clean routes —
`/userdata/system/custom.sh` (preferred, survives updates) or replacing
`S31emulationstation`. **ES restarts itself on stop or crash**, so a bare
`killall` brings it straight back.

**What "minimal" means here:** 1 GB, 640x480, Mali-G52. Drop video and PDF (SDL
gives neither; a webview gives both free). Probably drop the console-art wall
for a list. **Keep the backdrop shader** — it is WebGL2/GLSL and maps 1:1 onto
GLES 3.2; it is one of the few effects that gets *cheaper* in SDL. Text is the
quiet cost: font fallback across Japanese sets is not optional and SDL_ttf does
not do it for free.

**At this point the Flip is a working device.** Everything below is Android.

---

# Android — Steps 5 to 8

## Step 5 — scope the config merge

**A scoping step, not a building one, and it gates Step 8.**

The app shapes every session with `--appendconfig` and `--set-shader=`.
RetroArch's Android Intent interface has **neither**: the extras are `ROM`,
`LIBRETRO` and `CONFIGFILE`, and `APPENDCONFIG` is an open request against
RetroArch (issue #12096), not a feature.

So per-game overrides — core overrides, shader overrides, the motion/BFI pass,
light-gun mode, autofire, save-state-on-exit — cannot be layered onto a base
config. They must be **merged into one complete `retroarch.cfg` composed per
launch** and passed as `CONFIGFILE`.

**Do:** read `configpatch.rs` and `tweaks.rs`, and answer one question — how
much of `retroarch.rs`'s 1,950 lines is *composition* (survives, becomes the
merge) and how much is `Command`-building and desktop window shaping (does not).

**Done when:** there is a written answer to "what does the Android launch path
consist of", with a size on it.

**Size:** 1–2 days of reading. **Do not start Step 8 before this exists.**

**Risk:** the item most likely to be bigger than it looks, which is precisely
why it is scoped separately rather than discovered mid-build.

---

## Step 6 — Android toolchain, storage, and browsing

**Toolchain:** Android SDK, **NDK r28+** (28+ gets Google's 16 KB page
alignment free), **JDK 17**, `cargo-ndk`. Then `cargo tauri android init`,
which generates `src-tauri/gen/android/`. There is no `gen/android` in this
repo — mobile has never been initialized.

**Verify the permission first, before anything is built on it:** can the app
read the on-device ES-DE folder by path with `MANAGE_EXTERNAL_STORAGE`, or does
it need SAF? A sideloaded app can hold that permission; the answer decides
whether `local_root` is a plain path or a content resolver. **If it is SAF,
`savesync.rs` (793 lines) and `saves.rs` (608) need a second access path** —
that is the quiet cost and it is not small.

**Code changes, all small:**
- **Feature-gate the TUI out.** `src/lib.rs` has `pub mod tui`, dragging
  `ratatui` and `crossterm` into a build that will never draw a terminal.
- **`Android::data_root()`** — the app's private files dir, from Step 1's trait.
- **Ship `data/arcade-names.json` as a Tauri resource** and resolve it through
  the resource API rather than a relative path.
- **Verify the `asset:` protocol.** Covers are served through it and it must
  match `assetProtocol` in `tauri.conf.json`. Supported, but the path shape
  differs. Cheap to check, expensive to discover late.

**Controller:** Chromium WebView exposes the Gamepad API, which
`ui/js/gamepad.js` already drives. The scheme from Step 3 arrives here for free
— that was the point of defining it on the Flip.

**Done when:** the app browses your library on the Thor over ADB, with covers,
driven entirely by the pad.

**Size:** ~1 week. The cheap part, because the interface already exists.

**Memory sanity check** (`memory-footprint.md`: `44 MB + 24 MB per device megapixel`):
Thor top panel ~206 MB, bottom ~189 MB, against 8–16 GB. Add Chromium's heavier
floor (1.3–1.6x) and it is 2–5% of the device. **Not a constraint on either
Android target.**

---

## Step 7 — Android launching

Shape known; size comes out of Step 5.

- A **Kotlin mobile plugin** (`develop/plugins/develop-mobile`) exposing one
  command: fire the Intent.
- `am start -n com.retroarch/.browser.retroactivity.RetroActivityFuture -e ROM <path> -e LIBRETRO <full core path> -e CONFIGFILE <path>`.
  Since the 2025-01-17 nightly the **full** core path is required, not a bare
  core name.
- The composed-config work from Step 5.

**Read Daijishō and Lemuroid** — both drive RetroArch by Intent from Kotlin,
which is exactly this piece. **Do not read ES-DE**: its Android port is not open
source.

**Done when:** a game launches on the Thor with its overrides applied, and a
save written on the device syncs back.

**Size:** unknown until Step 5. Weeks, not days.

---

## Step 8 — the Thor's second screen

**Optional, and last.** The small screen should be a list and the big one a
grid. Two webviews cost more than one spanning both (255 vs 211 MB in list
view) — irrelevant on this hardware, but it decides whether the screens share
one page's state or synchronise two, which is an architecture choice rather than
a performance one.

Until then the Thor's two panels are simply the layout test rig: 1080x1920 and
1080x1240 in one hand, which is also the RP Mini V2's resolution rotated.

**Do not build this until Steps 6 and 7 are done.** Most tempting item here,
least load-bearing.

---

## What this plan deliberately does not do

- **No shared-core rewrite.** `src-sdl/src/library.rs` already opens the same
  `Cache` and drives the same `gamelist`, `gridnav`, `gamesort`, `gamefilter`.
  "One logic, two frontends" is already what this repo is.
- **No moving ROMs on the Flip.** They stay where Batocera's own ES and
  configgen need them.
- **No cutting the Android build to the Flip's constraints.** 1 GB and 640x480
  shape the SDL frontend; 6–16 GB and a real browser shape the Android one.
  Letting the Flip drive both is how you get two mediocre frontends instead of
  one good one and one small one.
