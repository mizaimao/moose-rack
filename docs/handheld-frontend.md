# A handheld front end

Three pieces of work, in order. The first two are worth doing on their own; the
third is the one that needs a handheld to test against.

Written to be picked up cold. Every number here was measured on 2026-08-20, and
the command that produced it is given so it can be re-checked rather than
trusted.

The machine it ends up on — the OS, the boot chain, why we ship an image rather
than a zip, and what is already installed — is `handheld-device.md`.

## Why this exists

`PLAN.md` §4 chose Rust + Tauri, and the reasoning was sound:

> **UI feel.** Tauri renders in the system webview … so the look is fully
> controllable via HTML/CSS. Flutter paints its own widgets and reads slightly
> off on desktop.
> **Packaging.** ~10 MB bundles with a documented macOS signing/notarization path.

Read what is absent: any mention of a handheld. The alternatives weighed were
Flutter and PySide6, the criteria were desktop feel and macOS notarization, and
the prototype was TUI-only. SDL2 was never a candidate because the requirement
that makes it necessary did not exist yet.

That requirement now exists. The device runs its frontend directly on DRM/KMS
with SDL2 and **no display server at all** — not X11, not Wayland. Tauri needs
`webkit2gtk-4.1` + GTK3, which needs one of them. So it is not that Tauri is
too heavy for the device; it is that there is nothing for it to draw into.

**The target is the Miyoo Flip**: RK3566, quad Cortex-A55 @ 1.8 GHz, Mali-G52
2EE, GLES 3.2, 1 GB LPDDR4, 3.5" **640x480**. Allwinner A33 / Mali-400 is
explicitly **not** a target — it is GLES 2.0 only and the shaders would need
downgrading to GLSL ES 1.00 with `mediump` fragment precision.

## What is already known, so nobody measures it twice

**The split.** `src/` is 23,970 lines and entirely portable. `src-tauri/` is
3,763 lines and is throwaway. `ui/js` is 9,564 lines, of which far less is
rendering than it looks.

**Logic hiding in the front end** — `grep -cE 'document\.|querySelector|createElement|classList|innerHTML|style\.|appendChild|getElementById'`:

| Module | Lines | DOM-touching |
|---|--:|--:|
| `ui/js/bindings.js` | 251 | 0 |
| `ui/js/picker-order.js` | 120 | 2 |
| `ui/js/filter.js` | 140 | 4 |
| `ui/js/sort.js` | 157 | 10 |
| `ui/js/pagefilter.js` | 115 | 10 |
| `ui/js/gamepad.js` | 518 | 10 |
| `ui/js/keys.js` | 441 | 14 |
| **total** | **1,742** | **50** |

**Memory.** Measured with `footprint -p <pid>` on a running build, summed over
the app and the three WebKit XPC helpers that start in the same second:

| Process | Footprint |
|---|--:|
| `moose-gui` | 44 MB |
| WebKit GPU | 43 MB |
| WebKit Networking | 6 MB |
| WebKit **WebContent** | **578 MB** |
| total | **~671 MB** |

Note `ps` on `moose-gui` alone reports ~44 MB and a naive `grep WebKit` sweeps up
every other app on the machine. Neither is the answer.

**What ports, and what does not.** This is the surprising part:

* **`ui/js/backdrop.js` ports nearly as-is.** It is not CSS — it is WebGL2 with
  GLSL fragment shaders, a full-screen quad, and uniforms `u_size`, `u_time`,
  `u_low/u_mid/u_high`, `u_strength`, `u_speed`. SDL2 gives an OpenGL ES context
  directly; on Mali-G52 (GLES 3.2) the shading language maps 1:1.
* **`ui/js/tint.js` ports and gets simpler.** It averages a cover down to one
  color for the selection glow by drawing into an 8x8 canvas. In Rust the
  images are already decoded; the canvas round-trip disappears.
* **CSS effects become draw calls.** `ui/style.css` and `ui/settings.css` are
  3,339 lines with 131 uses of `backdrop-filter`, `blur()`, `border-radius`,
  gradients and transitions. `box-shadow` glow becomes a blurred quad or a
  pre-blurred nine-slice tinted by `tint.js`'s color. `backdrop-filter` is the
  expensive one — it samples what is behind the element, so it needs
  render-to-texture plus a blur pass.
* **Video and PDF do not port.** `ui/js/lightbox.js` plays the gameplay video and
  pauses it with space; `detail.js` and `bulk.js` handle manuals. A webview gives
  `<video>` and PDF rendering free. SDL gives neither — that is ffmpeg or libmpv,
  and it is real work. On a handheld these are reasonable to drop.
* **Text is the quiet cost.** HTML wraps, ellipsises and falls back across fonts
  unasked. SDL_ttf draws a string at a position. This library has Japanese and
  translated sets, so font fallback is not optional. It is the least glamorous
  item here and the one most likely to make the result feel cheap.

## Task 1 — move the logic into `src/` — **done, 0.2.503**

Seven modules in `src/`, 91 tests: `binds` (keys and buttons), `gamelist`
(the row shape and the per-view memory), `gamesort`, `gamefilter`, `pickorder`,
`pagefilter`, `gridnav`, and `padpoll`. The webview reaches them through Tauri
commands and caches the answers, because `renderRows` and the key handler are
synchronous.

One exception, decided deliberately: **the controller poll stays in JS**. It
runs inside `requestAnimationFrame` at 120Hz and a round trip per frame is not
a thing that can be made fast enough. `src/padpoll.rs` holds the deadzones, the
repeat timings, the dominant-axis rule and the settle lock as the definition
`ui/js/gamepad.js` is a copy of — so when one of those numbers is argued about,
it is argued about once. That is the one place two implementations remain.

Two things moved storage on the way: bindings and the column order now live in
`config.toml` rather than in the webview's `localStorage`, which is what lets
the TUI read them and what retires the `storage`-event sync between the main
window and the settings window. Bindings left by an older build are adopted
once, at startup.

The webview's own tests kept the assertions about the *page* — the menu, the
button, the empty result — and handed the ones about the rules to `cargo test`.
They run against a deliberately naive stand-in backend in `ui/test/backend.js`;
its copy of the default binding table is a fixture that `cargo test`
regenerates and checks, so a moved default button fails there rather than
quietly changing what the tests press.

The original brief follows.

**Do this first, and do it whether or not the handheld front end ever happens.**

Move the 1,742 lines above into the core: sort orders, filter predicates,
per-list order, page filtering, and key/pad binding resolution. The front end
then calls into the core instead of reimplementing it.

`README.md` already states the principle for the launch planner — *"The GUI, TUI
and CLI share one launch planner. Adding an emulator quirk in one place fixes it
in all three."* This is the same class of thing and it is currently not shared.

Three returns: the TUI stops being second-class today, duplication goes, and an
SDL front end inherits all of it for nothing.

**How to know it is right:** `ui/test/` holds ~300 jsdom tests asserting
behavior rather than markup, several written after a bug with a comment saying
what they caught. Port the assertions alongside the logic. Both suites must stay
green: `npm test` and `cargo test --workspace`, plus
`cargo clippy --workspace --all-targets -- -D warnings`.

This is narrow, well-specified, test-backed work — the shape a smaller local
model handles well. One module at a time.

## Task 2 — window the middle column — **done, 0.2.606**

Two fixes, and the doc was right that they are the same one.

**The 578 MB.** Covers are let go once a card is well off screen. Two
observers, not one: near, at 300px, fetches; far, at 1600px, puts the
placeholder back and drops the image. The gap between them is the hysteresis —
a card one flick of the wheel off the top of the screen is about to be looked
at again. The version this replaces unobserved a card the moment its cover
arrived, so nothing ever released one.

**The 2,506 nodes.** A flat list over 400 rows draws only the band around the
viewport, with a spacer above and below standing in for the rest at exactly the
height they would have taken — so the scrollbar and every remembered scroll
position are unchanged. Measured on the arcade console, ten across, an 800px
window: **2,506 cards to 100, and `renderRows` from 447ms to 21ms.** Grouped
results are drawn whole; search is capped at 200 and a window over a short list
is machinery with nothing to do.

The four things that assumed every row exists were the work:

* **The cursor** now moves in rows rather than in drawn nodes, through
  `gridnav::uniform` — a table from two numbers, because a uniform grid needs
  no measuring and most of its cards have no position to measure. A test
  asserts it agrees with the measured table on any layout where both apply.
* **The remembered position** asks the window to reveal its row, which scrolls
  there and draws the band around it. Being far down the list is exactly why it
  was worth remembering.
* **The filter box** narrows the list rather than hiding drawn nodes — hiding
  would have searched a hundred games out of two and a half thousand, finding
  less the further down you had scrolled.
* **The cover observers** are re-attached on every band change, and the
  highlight is put back: the card carrying it is thrown away each time.

The original brief follows.

## Task 2 — window the middle column

The only thing in the app that is slow rather than missing: 2,506 rows are
inserted on every platform
switch, and the cursor, remembered scroll position, pad navigation and the lazy
cover observer all assume every row exists.

**It is also where the 578 MB is.** The observer hygiene in
`ui/js/library.js` is correct — `disconnect()` on rebuild, `unobserve()` once
loaded — but nothing ever releases a decoded image. Every cover scrolled past
stays in memory, and a 512x384 image is ~786 KB as an RGBA bitmap however small
the PNG was.

**Verify before fixing:** restart the app, `footprint` the WebContent process
before touching anything, scroll the full arcade list, measure again. If it
climbs and does not come back, that is it.

Two fixes, and they are the same fix. Windowing removes off-screen rows, and
removing the row removes its image. The cheap stopgap is clearing `img.src` when
a row scrolls well out of view, against the existing observer.

**671 MB is not inherent to Tauri.** The webview costs ~90 MB across GPU and
networking; the rest is this app holding every cover it has ever drawn.

## Task 3 — the SDL2 front end

**What tasks 1 and 2 leave it.** Written down because the point of doing them
first was that this one inherits the result, and a list of what is already
there is the difference between starting from facts and starting from a survey.

Ready to call, tested, no webview anywhere near them:

| | |
|---|---|
| `binds` | the action table, the pad table, defaults, repair, and storage in `config.toml` |
| `gamelist` / `gamesort` / `gamefilter` | the row shape, the orders, the predicates, and the per-view memory |
| `pickorder` | how the left column is ordered, remembered across restarts |
| `pagefilter` | what the filter box matches, and when a heading has nothing left under it |
| `gridnav` | where the cursor goes next — including `uniform`, which needs no geometry at all and is what a windowed list navigates by |
| `padpoll` | deadzones, repeat timings, hold-versus-tap, and the lock after a game exits |

**Two things it has to write for itself, and both are deliberate.** Neither is
an oversight; both are places a round trip per frame was the wrong answer for
the webview and will be the right code to lift for SDL, which has no boundary
to cross:

* **The controller poll.** `padpoll` is the arithmetic; the loop around it —
  reading the pad, routing to a dialog or a player or the library, the settle
  window after an emulator exits — is in `ui/js/gamepad.js` and has no Rust
  equivalent. SDL reads `SDL_GameController` and translates into the button
  indices `binds::PAD_BUTTONS` names before asking anything.
* **Which rows to draw.** `ui/js/visible.js` holds the windowing arithmetic —
  given the column count, a row's height and where the list is scrolled, which
  band to draw and how much empty space to leave either side. Fourteen tests in
  `ui/test/visible.test.js` pin it. A 1 GB handheld needs this more than a Mac
  does; port `slice()` into `src/` early and delete the duplicate reasoning,
  rather than rediscovering it against a 4" screen.

### The direction changed on 2026-08-21: SDL is meant to replace Tauri

This section said "add a fourth front end, do not convert". Frank's call, made
after the argument below was put to him rather than instead of it:

> I know the handheld comes with that 960x720 size, but I still want to make
> the entire frontend unified into a single one, that is, I am currently
> leaning to use SDL to completely replace Tauri. It's a trade off: performance
> and compatibility and maintainability at the cost of ease of development.

**Both sides of it, written down while they are still fresh**, because in six
months the only thing anyone will remember is the conclusion.

*What SDL wins.* It runs where there is no display server, which is the whole
reason any of this started. Memory goes from ~671 MB to tens. Startup is
instant. One rendering path, fully controlled, instead of a browser engine's.
And **no IPC boundary at all** — the thing that cost two rounds of chasing
during task 1, where a round trip landed on the cursor and on every keystroke.

*What it costs, and none of it is "ease of programming".*

* **Text is a ceiling, not an effort.** HTML gives shaping, bidi, font
  fallback, wrapping, ellipsis and sub-pixel positioning, tuned over decades.
  SDL_ttf draws a string at a point. Matching it for a library with Japanese
  and translated titles means HarfBuzz, a line breaker and a fallback chain.
  Writing that is tractable; making it as good as a browser's is not, and this
  is already the item most likely to make the result feel cheap.
* **Video and manuals go**, unless we take on libmpv and a PDF renderer. A
  webview gave both for free.
* **Accessibility goes.** VoiceOver and Narrator work today for nothing.
* **Design iteration slows.** CSS is the fastest visual-iteration surface
  there is; a shader or layout change is a rebuild, and somebody still has to
  look at it. That cost is human-in-the-loop and does not go away.

*Why the decision still stands.* Every one of those is a known price rather
than a surprise, three of the four are already accepted on the handheld, and
one front end that runs everywhere is worth more than two that share a core but
not a look. **Nothing has to be decided in advance**: building SDL as a peer on
macOS costs exactly the same as building it as a replacement. The only
difference is when Tauri gets deleted, and that can wait for evidence.

**So Tauri stays until SDL is better than it, and then it goes.** Not before.

### One front end means it has to fit every window

960x720 is 4:3, at 4 inches. A desktop is 16:9 or 16:10 at twenty-seven. A
layout in pixels cannot serve both, and "it works at 960x720" is not the
requirement — **that size is the one we test hardest, not the one we build
for.**

Three things have to be true from the first commit, because retrofitting any of
them is a rewrite:

* **Layout in points, not pixels.** One scale factor, derived from the display,
  turns points into pixels. A card is "150 points" everywhere and comes out
  physically similar on a 4" 300-DPI panel and a 27" retina one. Pixels are a
  measurement, never a design unit.
* **Breakpoints on available width in points.** The shell already has this
  idea: `ui/js/shell.js` switches between one pane and three columns, and views
  describe what they need by role rather than reaching for elements. 960 points
  wide is one or two panes; a desktop is three. That design is the thing worth
  porting most carefully.
* **The window resizes, and the layout answers.** Not a fixed backbuffer
  scaled up. On the handheld it happens once at startup; on a desktop it
  happens while somebody drags an edge, and it is the fastest way to find every
  place a pixel got hardcoded.

See `handheld-device.md` for the machine, the image, and what is already
installed on it.

### The original position, for the record

**Add a fourth front end. Do not convert.** The repo is already CLI + TUI + Tauri
over one core. Converting would cost the desktop app its video, manuals and
glass in exchange for nothing — a Mac has a display server and plenty of RAM.
Tauri stays the desktop app; SDL targets RK3566 only.

**It stays Rust.** SDL2 is a C library; the `sdl2` crate binds to it. No C++.

**Scope:** a list, a grid, a detail pane, a few dialogs, and the two-layout
switch. `ui/js/shell.js` is only 269 lines because the architecture is already
right — views *describe* what they need and hand content back by role
(`picker`, `games`, `aside`) rather than reaching for elements. That design
ports; only the paint calls change. Estimate 2,000–2,500 lines of Rust.

**Build strategy.** Prototype windowed on macOS first — SDL2 runs the same source
there, so the loop stays at desktop speed until integration. NextUI ships a
`Tools/desktop` build for exactly this reason. On device, SDL picks `KMSDRM` at
runtime; the same binary source covers Mac, desktop Linux and handheld.

**Cross-compiling is the only real friction**, because SDL2 is C and needs
headers and libs for the target. Building on the device is not an option here:
the Flip's stock userland is Buildroot 2021.11 with no toolchain and no package
manager. So: `cross` against `aarch64-unknown-linux-gnu`, or the `sdl2` crate's
bundled feature, which builds SDL from source and links it statically and
sidesteps version drift between build box and device. The card already carries
SDL2 2.30.8, 2.28.5 and 2.0.22 alongside SDL2_image/ttf/mixer, so linking
against the device's own copy is the third option.

**Do not port the backdrop last.** It is the most portable thing in the front end
and getting it up early proves the GL context works on the device.

## What not to do

* Do not add X11 or a Wayland compositor to the handheld to run Tauri. It is
  possible on Debian — `cage`, or bare Xorg — and it means carrying a browser
  engine on a 1 GB device to avoid rewriting a UI layer.
* Do not treat SDL as a memory fix for the desktop app. Task 2 is that fix.
* Do not target Mali-400 / A33 without accepting a GLSL ES 1.00 shader rewrite.

## Open questions

* ~~Font fallback strategy for Japanese and translated titles under SDL_ttf.~~
  **Answered 2026-08-21:** the rootfs already ships `fonts-noto-cjk`,
  `libfreetype6` and `libfontconfig1-dev`, so we resolve fallback through
  fontconfig and bundle nothing. See `handheld-device.md`. What is *not*
  answered is the shaping and line-breaking above it, which is task 3's
  highest-stakes item now that SDL is meant to be the only front end.
* ~~Whether video is worth libmpv on the handheld, or simply absent there.~~
  **Reopened, wider:** if SDL replaces Tauri then this is not a handheld
  question, it is a product one — the desktop app has a gameplay-video lightbox
  and PDF manuals today, and both come free from the webview. Absent on the
  handheld is easy to defend; absent on the desktop is a regression somebody
  will notice. The cheap answer for manuals is to hand the file to the system
  viewer on desktop and drop them on the handheld; video wants libmpv or
  nothing.

  **Decided on 2026-08-21, and deferred:** *"I will want to try to provide PDF
  and video support in desktop and handheld. But we worry about that later."*
  So neither is dropped — both are wanted on both, and neither blocks the front
  end. Plan the detail pane so a video frame and a page image are things it can
  be handed, rather than assuming they will never exist.
* ~~Which language a title is in, and therefore which shapes to draw it
  with.~~ **Answered 2026-08-21, and it needed no metadata.**

  Han unification is real: Chinese, Japanese and Korean share code points for
  characters whose correct shapes differ — 直, 骨, 話, 令 — and a reader of one
  sees the other immediately. EmulationStation's answer, on this very handheld,
  is `DroidSansFallbackFull.ttf`: Android's single pan-CJK fallback, built on
  simplified forms, used for all three. That is why Japanese titles look subtly
  wrong in every one of these front ends.

  The first plan here was to take the language from the ROM's region. Frank's
  objection was the right one — *"don't understand why you need ROM files to
  implement that"* — and it is not needed. **The title says.** Kana is proof of
  Japanese; hangul is proof of Korean; Han alone is Chinese, and which Chinese
  is settled by counting the characters that exist in only one of the two
  writing systems. `src/script.rs`, eight tests, no metadata and no
  configuration.

  In the core rather than the front end because the webview has the same
  problem and the same fix, spelled differently: `Script::language_tag` gives
  `zh-Hant`, and a `lang` attribute on the element fixes it in a browser with
  no font names involved at all.

  On this Mac all four now land where they should, traditional Chinese
  included — it was being handed to a *simplified* face before. Startup prints
  what was asked for beside what the machine gave, so a handheld with only a
  pan-CJK fallback says so rather than quietly drawing one language in
  another's forms.

* **The app's own words, in other languages.** Untouched, and a different job
  entirely: every string in the interface is currently an English literal in
  the source. Nothing about the front end forbids it — the text engine draws
  any script already — but there is no catalog, no lookup and no plural
  handling. Parked rather than started; it wants doing once, properly, and not
  while the front end is still moving.

* Whether the animated backdrop should default off on battery — there is already
  a strength slider and per-shape settings to hang that off.
* Whether the desktop app writes `config.toml` onto the card when it is
  mounted, so a handheld never has to be told its own server. Strongly
  recommended in `handheld-device.md`; not built.
