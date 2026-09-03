# Handover

**START HERE.** How the work goes, what has already cost days, and where things stand.

For whoever picks this up next, human or otherwise. `README.md` says how to
run it and `PLAN.md` says why it is shaped the way it is; this is the part that
is neither — how the work is done here, and what has already been learned the
expensive way.

Read this, then start.

---

## The handheld

Everything that has been changed on the Flip, and why, is in
[flip-knulli-changes.md](flip-knulli-changes.md) — read back off the device
rather than written from memory. Read it before changing anything on there.

## Read this before you touch a button mapping

**Handhelds do not agree on where A is.** Android handhelds tend to be Xbox
layout — the **AYN Thor** is — and the smaller Linux ones tend to be Nintendo,
where the button printed A sits on the right. The **Miyoo Flip** is Nintendo.

SDL makes this look worse than it is, because it names face buttons by their
position on an Xbox pad: `Button::A` is the bottom one, always, as a position
rather than a label.

### The rule that actually works

**Trust the letters in `es_input.cfg`. They are the letters printed on the
plastic.** That file is written by EmulationStation's controller wizard, which
asks you to press A, then B, and writes down whatever you pressed; vendor
defaults are made the same way. `moose_sdl::input` maps ES's `a` to SDL's `a`,
so **SDL's `Button::A` already is the button printed A** — whatever position it
occupies, and whatever the kernel calls its scancode.

So: **no swap, by default, on every device.** If one ever genuinely disagrees,
there is an explicit setting for it — `[controllers] swap_ab` in `config.toml`,
which the desktop app already honours and `moose-patch` reads.

### Two things that look like evidence and are not

**EmulationStation's `InvertButtons` is a preference, not a fact.** It means "I
would rather confirm with the other button" and lives in ES's own interface.
The Flip has it set to `true`. Reading that as a description of the hardware and
swapping A/B on the strength of it made **A quit the app**, which took two
rounds and a user's patience to undo.

**Scancode names do not tell you where a button is.** The Flip's `es_input.cfg`
reads `a` → code 304 `BTN_SOUTH`, `b` → 305 `BTN_EAST`, `x` → 307, `y` → 308.
It is tempting to read `BTN_SOUTH` as "the bottom one" and conclude the letters
are positional. They are not: on this device the button printed **A** is the one
reporting 304. The vendor's driver assigns codes to labels, not to positions,
and there is no way to tell from the file which it did.

### How to tell, when you have to

Not by reasoning. `moose-patch` logs every press it receives to
`/userdata/system/logs/moose-patch.log`; one launch and a few presses says
exactly which button produces which action. That log settled this after two
wrong guesses, and it is cheaper than either of them.

---

## What it is

A desktop client for a self-hosted [RomM](https://romm.app) server: browse the
library, download what you want to keep, launch it in RetroArch with the right
core, shader and controller layout already set. Rust workspace, Tauri v2 window,
plain ES modules for the UI — no bundler, no framework, no build step for the
front end.

Three front ends over one core library, and that is the point:

    src/            the library: api, cache, download, media, launch, retroarch
    src/main.rs     the CLI and the terminal browser
    src-tauri/      the window; thin, delegating to the library
    ui/             the front end: static modules and one stylesheet
    ui/test/        jsdom suites, run against the real index.html and CSS
    data/           generated reference data (core map, arcade names, icon sets)
    docs/           coverage reports, device notes, and design records

Adding an emulator quirk in the library fixes it in all three. Putting one in
`src-tauri` fixes it in none of the others, so do not.

## How the work goes

**Frank names the version, and the number moves by however many things were
asked for.** Four items in a message means the patch number goes up by four.
An explicit instruction ("bump to 165", "bump by 8") overrides the count. The
number lives in exactly one place — `[workspace.package] version` in the root
`Cargo.toml` — inherited by both crates, with no `version` key in
`tauri.conf.json`. Do not reintroduce a second copy.

**Rebuild the bundle every round.** `./scripts/build-macos.sh`. He runs
`Moose Rack.app`, not the CLI, and a stale bundle means testing old code —
which has wasted rounds. The title bar carries the version so a screenshot says
which build it is; check it against `Info.plist` after building.

**Never push, tag or release unless that message says so.** Committing freely
is fine. "Run CI" is permission to push, because that is what runs CI.

**Write only inside the project.** The RetroArch install, `~/Library`,
everything else on the machine: read to diagnose, never write. When something
outside genuinely needs changing, back it up into `backups/` (gitignored), and
hand over the command rather than running it.

**Plain words.** "Game save", not "battery save". Short answers, verdict first.

## Testing, and the traps in it

`npm test` (jsdom, ~350 tests) and `cargo test --workspace` (~490). Both must
pass, plus `cargo clippy --workspace --all-targets -- -D warnings`. CI runs the
same on ubuntu-22.04, macos-14 and windows-latest.

**Which suite a test belongs in** is now a real question rather than a matter
of taste. The rules — what a filter keeps, which order a list opens in, where
the cursor goes next, what a binding resolves to — live in `src/` and are
asserted by `cargo test` against the implementation. The jsdom suites are about
the *page*: that the menu opens, that the button counts what is on, that an
empty result says why. They run against `ui/test/backend.js`, a deliberately
naive stand-in — it orders nothing and filters nothing — so a test that would
fail because the stand-in is naive is a test that belongs in Rust. Its copy of
the default binding table is a fixture `cargo test` regenerates and checks, so
a moved default button fails there rather than quietly changing what the tests
press.

What jsdom does **not** have, each of which has produced a green suite over
broken software:

* **No WebGL.** `getContext("webgl2")` returns null, so the shader backdrop's
  entire body was never executed by any test — until a reference error in it
  threw out of startup and took the tab row, the settings button and the page
  background down. `ui/test/backdrop.test.js` now supplies a stand-in context.
  Anything drawing to a canvas needs the same treatment.
* **No layout.** `scrollTop` clamps to 0, `offsetParent` is always null,
  `scrollIntoView` does not exist, and every `getBoundingClientRect` is zeros.
  Tests that care about geometry stub the rects; tests that care about *rules*
  read `ui/style.css` and assert on the declarations. The middle column now
  draws only the rows it can see, and *every* measurement it makes comes back
  zero here — so it decides there is nothing to window and silently draws
  everything, which is a green suite over the exact bug. `ui/test/windowing.test.js`
  gives the page a layout: cards report a height, the grid reports its columns,
  the list reports how much of itself is on screen. Its first assertion is that
  the windowing happened at all.
* **`.click()` is not a click.** It skips `pointerdown`, which is where the
  menu bug lived for months: the menu closed on the next pointerdown anywhere,
  so it came off the page between press and release and no click ever reached
  the item. jsdom will happily deliver a click to a node that has been removed;
  a browser will not. Press the way a mouse does — down, up, click — and assert
  the thing survived the press.
* **CSS `:has()` is unsupported**, and `CSS` is not a bare global.

The habit that catches all of this: **put the bug back and watch the test
fail.** A test written after a fix that has never seen the failure is a test
that proves nothing. Several in here are commented with what they caught.

## What has already cost days

**RetroArch's per-core overrides beat `--appendconfig`.** `config/<Core>/<Core>.cfg`
is applied *after* everything passed on the command line. Three files on the
user's disk held four lines each of turbo settings and quietly replaced what
every launch asked for — ten rounds of chasing a rapid-fire setting that was
correct in the file we wrote. `RetroArch::override_clash` now compares our keys
against theirs, names the collisions in the launch notes, and disables override
loading for that run. If a setting "does not take", suspect this first.

**Autoconfig beats the config file for player binds.** A pad's autoconfig
profile is applied when the pad connects and overwrites `input_playerN_*` from
the config. Remaps (`.rmp`) are applied last and do work. This is why button
moves go in a remap and never in the launch config.

**RetroArch turbo is not what the documentation says.** Mode 0 "classic"
latches: the bit stays set until the *face* button is released, which reads as
a toggle. Mode 3 "single button hold" is the one that behaves — and it reports
the repeat **only on frames where the button is not physically held**, so
holding the modifier *and* the fire button gives one continuous press and no
repeat at all. That single fact invalidated three designs before it was found;
it is in RetroArch's source, not its docs.

**Windows paths.** `canonicalize` returns `\\?\C:\...`, which is a fine path
for the Windows API and not one that survives a URL on its way to an `<img>`.
Everything handed to the webview goes through `util::webview_path`.

**Cores are not interchangeable.** SwanStation on macOS asks for Vulkan, gets
MoltenVK, and loses the GPU device a second in — pinned to OpenGL there. Neo
Geo runs `geolith`, arcade runs `fbneo`, and FBNeo refuses romsets that fail its
own audit unless told otherwise.

## The shape of the UI

`ui/js/shell.js` is the seam. A view *describes* what it wants — a title, which
buttons, whether the zoom slider means anything here — and hands over content by
role (`picker`, `games`, `aside`) rather than by element. That is what makes one
set of view code serve both arrangements: **Sofa**, one screen at a time, and
**Desk**, everything side by side. Nothing in `shell.js` knows what a console
is; no view knows what a column is. Keep it that way — a layout change should be
an implementation of `enter` and `paint`, not an edit to every view.

Two search boxes, deliberately: the header's searches the whole library and
takes you elsewhere; the one in the tab row (`pagefilter.js`) narrows the page
you are on and leaves you there.

Per-list order is remembered (`picker-order.js`); the game sort and the game
filters are deliberately forgotten when the app closes — a question you asked
about one console is not a setting.

## Where it stands

Working: browsing, downloading, launching, save sync with conflict resolution,
save states, play history, four controllers, rapid fire, arcade coverage work,
and starring — which syncs, because a favourite in RomM is a *collection* and
collections live on the server. `*` stars the selected game; the handheld syncs
both ways through moose-patch. See `docs/knulli-addon.md` for why that one has
a baseline file and the save sync does not.
Released through CI for Linux, Windows and Apple silicon macOS.

Least covered by tests: `settings-window.js` and `settings.js` (the window
frame itself, as opposed to the panes, which are tested), and `src/tui.rs`.

The next things worth doing, in the order I would take them. The top two:

1. **Windowing the middle column.** 2,506 rows are still inserted on every
   platform switch. It is the only thing in the app that is *slow* rather than
   missing, and the cursor, the remembered scroll position, the pad navigation
   and the lazy cover observer all currently assume every row exists.
2. **Offering to clean up RetroArch's per-core overrides.** The app can see
   them fighting its settings and says so; it will not touch them, because
   editing the user's RetroArch directory breaks the promise the README makes.
   A prompt is the honest version, and it is not built.

Work that has been **scoped but not built** goes in
[not-built.md](not-built.md), one section per subject, and the design is
written out in full — the design is the expensive part and it is what makes the
thing pickup-able cold. If Frank defers something, write it down that way — in
his words — rather than leaving it in a chat summary he cannot search. Work
that is deferred *with a reason and a cost* goes in [parked.md](parked.md)
instead; work that is finished or superseded moves to
[archive/](archive/README.md).

## One last thing

Frank reports symptoms precisely and is not interested in theories. When
something cannot be reproduced from here — Windows, a pad nobody has, a config
on his disk — the useful move is not a guess dressed as a fix: make the thing
say what it did (the launch notes exist for exactly this), then ask for the one
line of output that settles it.
