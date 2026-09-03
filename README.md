# Moose Rack

A native desktop and handheld client for your own ROM library. Browse it,
download games on demand, and launch them in RetroArch — with a gamepad, from
the sofa, without a browser.

The library still comes from a self-hosted [RomM](https://romm.app) server
today. Replacing that with a backend of our own is the current direction, and
every remaining mention of RomM in this repo marks something that goes away
when it lands — `docs/library-service.md` is the design.

`PLAN.md` holds the design notes and the reasoning behind specific decisions;
this file is how to run it. `docs/handover.md` is for whoever picks the work
up next — how it is done here, and what has already been learned expensively.

## Why not the web UI

RomM's in-browser player picks its emulator core based on whether
`SharedArrayBuffer` exists, which needs cross-origin isolation **and** a secure
context. Over plain `http://` on a LAN there is no secure context, so it falls
back to a single-threaded core and anything past the 16-bit era struggles. A
native client sidesteps that entirely and uses the RetroArch you already have.

## Building

Rust 1.97.1 — pinned in `rust-toolchain.toml`, so `rustup` will fetch it for you.

```sh
npm ci                      # Tauri CLI only; the UI has no build step
cargo build --release       # CLI and TUI
./scripts/build-macos.sh    # macOS app bundle, into the repo root
```

```sh
cargo test --workspace     # core resolution, RetroArch config layering
npm test                   # ui/js, run against index.html in jsdom
```

The UI tests are worth the jsdom dependency: an exception thrown inside a
`requestAnimationFrame` callback does not stop the loop — the next frame is
already scheduled — so a broken controller path fails silently in the app and
only shows up here.

On Linux the GUI additionally needs `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`librsvg2-dev`, `libayatana-appindicator3-dev` and `patchelf`.

Binaries come from CI — tag a commit `v*`. Three targets: Linux x86_64 as a
binary, an `.AppImage` and a `.deb`; Windows x86_64 as a portable `.exe` with
no installer, since the UI and the core map are both compiled into it; macOS on
Apple silicon as a `.dmg` and a `.app.tar.gz`.

Intel macOS is not built there: the macos-13 runners are being retired and a
dispatched build sat queued for twenty-two minutes without starting, which
would hold up every release. The macOS builds are unsigned, so macOS
quarantines them and one `xattr -dr com.apple.quarantine` is needed after the
first move to /Applications. Building locally with `scripts/build-macos.sh`
skips that entirely — a bundle that never came through a browser is never
marked — so that is still the better route on the machine you develop on.

**The CLI is not published.** It is the same things the app has buttons for,
plus the terminal browser, and it is a build-it-yourself tool: `cargo build
--release --bin moose-rack` from a checkout, then `./target/release/moose-rack
doctor`. CI still builds and tests it on all three platforms.

## Configuring

```sh
cp config.example.toml config.toml
```

At minimum, set `[server]`. Without a `config.toml` the app starts and says so
rather than looking broken.

RetroArch is found automatically in the usual places per OS. If yours lives
somewhere else — a second drive, a portable install — set it in **Settings**, or
as `[retroarch] root`. Missing cores and the slang shader pack are downloaded on
first launch of a game that needs them.

## Using it

One pane at a time by default: a console replaces the screen with its games and
Back undoes it. The pair of buttons left of the search box switches between **Sofa** — one
screen at a time, driven from a controller — and **Desk** — what you are picking from, the games, and a preview of the one
selected — where nothing is ever replaced and both outer columns can be dragged
and remember their widths. Four tabs across the top either way: **Library**,
**My collections**, **History** and **RomM browse**.

The box at the right of the tab row searches the page you are on — consoles,
collections or games — and leaves you where you are, which is the difference
between it and the search box above, which spans the library and takes you to a
different screen. **Filter** narrows a list to what is on this machine, what is not, what is
starred, what you have never played, or what is rated 8 or better; several at
once, and they all have to pass. **Random** picks out of whatever the filters left —
the only realistic way into an arcade set nobody knows by heart, and on the
right stick click as well. Both are per-list and forgotten when the app closes, like the
sort. Consoles are listed alphabetically; collections have an order button above them
(name, most games, fewest, most downloaded) because the server returns them by
size. Within a console, games sort by name, rating, year, size or recently
played — that one is per-console and deliberately forgotten when the app closes.

Holding Select for two seconds puts every list back to the miximage rather
than tapping through seven kinds of artwork to reach it. Arrows or the left
stick move, Enter or the bottom face button opens and plays,
Esc or the right face button goes back, `/` searches, `?` lists every binding and what the marks on a card mean, Space plays and pauses a video,
`Cmd+,` opens Settings. The shoulder buttons move between tabs and the triggers
scroll the game list — how hard you pull decides how fast. Keyboard and
controller are rebound separately in Settings; both persist. Dialogs are
driveable from the pad, so a sync question mid-launch does not send you back to
the mouse.

A game's page opens with tags for what it has — video, manual, trailer — then
the artwork strip, which starts with the gameplay video and runs through every
picture the server holds. Clicking any of them opens the reel full size, and
the arrows walk it; space or the bottom face button pauses a video. It also
carries the game's save states, each with the picture RetroArch saved beside
it, and starting from one enters that slot directly. Right-click deletes one. **History** counts what you have actually played — hours per console, the
games you played most, and the ones you kept opening and putting down — from
sessions this app started, since nothing else can be known.

Arcade games where a single shot needs the button pressed repeatedly (Metal
Slug and its relatives) can hold the fire down for you: hold **LB** — or RB — on its own and
the game fires at the rate in Hz set beside the control on the game's page.
Ten a second is the usual choice: it is about as fast as a good player can
mash, and faster settings mostly waste presses on games that cap the shots on
screen.
It stops the moment the shoulder is released, and nothing is remapped. Do not
hold the fire button as well: RetroArch reports a real press in place of the
repeat, so holding both gives one continuous shot. The choice applies across the whole
arcade system, and the control is absent for games where it would mean
nothing.

Selecting a console shows it in the preview, the way an ES-DE theme does: who
made it, when it arrived, whether it is a console, a handheld or a cabinet, a
line about what it was, and then the game count, the emulator and shader it
will use, and whether that emulator is installed at all.
The top-right tag says which server it is talking to, with a colored dot for
whether it is reachable; hovering gives the game count, the cores, the disk
usage and the folders everything lives in. **Settings → About**, and the macOS
About panel, carry the version and the source.

Launching writes a temporary config that RetroArch layers on top of its own, so
**your `retroarch.cfg` is never modified**. RetroArch loads its own per-core
override — `config/<Core>/<Core>.cfg` — *after* that file, so anything in there
silently wins; when it contradicts something this launch set, the launch says
which keys and ignores the override for that run only, leaving the file alone. That layer carries a consistent
gamepad hotkey set — RetroArch binds keyboard hotkeys but none for a pad, which
otherwise leaves a handheld user unable to quit a game. Hold **Back/Select**:

| with | does |
|---|---|
| A | quit (asks twice) |
| B | toggle shaders |
| X | FPS counter |
| Y | RetroArch menu, paused |
| LB / RB | load / save state |
| RT | fast-forward |
| D-pad ↑ ↓ | previous / next shader |
| D-pad ← → | previous / next save slot |

Anything in your own settings file is applied last and wins.

Those hotkeys are **generated per launch, not shipped as fixed numbers**.
RetroArch hotkeys take raw driver button indices, and the same Xbox controller
reports Select as button 2 on macOS, 6 on Linux and 6 on Windows — with the
d-pad as four buttons in one case and a hat in the others, and the triggers as
axes rather than buttons. So the indices are read out of RetroArch's own
`autoconfig/` profile for the connected pad, which is the only source that is
right for every controller on every OS. To see what your machine produces:

```sh
cargo run --example padprofile_dump -- "Xbox Wireless Controller"
```

If no profile matches, a built-in Xbox-style table for the current OS is used
and the block says so.

## Layout

```
src/            core library — api, cache, download, media, theme, retroarch
src/main.rs     CLI (clap) and TUI
src-tauri/      Tauri GUI shell, thin: it delegates to the library
ui/             static ES modules and CSS, no bundler
ui/js/shell.js  where a view is drawn and what the window looks like while it is
ui/js/visible.js  which rows of a long list are drawn at all
ui/js/settings/ one file per Settings tab: markup and wiring, nothing shared
ui/icons/       Lucide (ISC), vendored — see ui/icons/README.md
ui/test/        jsdom suites, run against the real index.html and stylesheet
ui/test/backend.js  a stand-in for the backend, for tests about the page
tools/          one-shot Python for DAT analysis, BIOS sets, server sync
data/           generated reference data (core map, arcade names, icon sets)
docs/           arcade and BIOS coverage, handheld card builds, device notes
```

The interface logic is in the core, not in the front end that happened to be
written first:

```
src/binds.rs       keys and controller buttons: the tables, and what resolves
src/gamelist.rs    what a list of games knows about itself
src/gamesort.rs    the orders a list can be in
src/gamefilter.rs  what each filter keeps
src/pickorder.rs   how the left column is ordered
src/pagefilter.rs  searching inside the page you are on
src/gridnav.rs     where the cursor goes next, from where the cards landed
src/padpoll.rs     deadzones, repeat timings, and the lock after a game exits
```

Views describe what they need and hand content to `shell.js` by role — the list
you pick from, the games, the preview — rather than reaching for elements.
Which is why the same view code draws both the one-pane and the three-column
window. `PLAN.md` section 19 has the reasoning.

The GUI, TUI and CLI share one launch planner. Adding an emulator quirk in one
place fixes it in all three — which is the point, because it did not used to.
The same now goes for sort orders, filters, key and pad bindings and cursor
movement: the webview asks the core and caches the answer rather than deciding
for itself. The one exception is the controller poll, which runs at the
display's refresh rate inside `requestAnimationFrame` and cannot afford a round
trip per frame — `src/padpoll.rs` is the definition `ui/js/gamepad.js` copies,
and it says so in both.

## Saves

Automatic, in the shape you expect from Steam: the server's copy is pulled
before a game starts and whatever changed is pushed after it exits. `plan.run`
blocks until the emulator quits, so those two moments are a real boundary
rather than a guess about when you stopped playing.

**Every overwrite is backed up first.** Ten copies per game and save slot, in
`library/saves-backup/<rom id>/<slot>/`, oldest dropped. Plain files, so
recovery is copying one back — no tooling, nothing to learn at the moment
something has already gone wrong.

That backup is what makes the automation defensible. A save is the only thing
here that cannot be fetched again, and syncing it unattended without a way back
would be the one action in the app capable of destroying something for good.

Anything changed on both sides since the last sync is a conflict, and **the
launch stops there and asks** — the two copies side by side with their dates,
keep mine or keep the server's. Nothing is written until you answer, and the
copy you do not keep is backed up anyway.

Refusing to start is the point. Playing on top of a save whose ownership is
unsettled means the loser gets overwritten for good when you quit, which is the
one moment where carrying on quietly is worse than stopping.

A server it cannot reach is a different question, and it asks that one too:
**"saves are not syncing — play anyway?"** Starting silently risks an hour on
top of a stale save; refusing outright would mean a server being off stops you
playing at all. Cancel is the focused button, so a stray Enter is the safe
answer.

The TUI does the same sync. It has no dialog to resolve a conflict in, so it
refuses the launch and points at `sync-saves`.

Turn it off with `[saves] auto_sync = false` and sync when you ask instead:

```sh
moose-rack sync-saves            # or Settings -> Sync saves now
moose-rack sync-saves --dry-run  # what would be offered, writing nothing
```

`--dry-run` is the way to check the ROM matching before anything moves.

## What the marks mean

A game carries one of two: a **disk** if it is on this machine and will play
with the server off, a **cloud** if it is still on the server and will be
fetched when you play it. A **star** means it is in one of your starred
collections. Beside a console, a **green dot** means an emulator for it is
installed and a gray one means nothing there will start. `?` shows the same
list, so nothing here has to be remembered.

## Status

Browsing, downloading, launching and save sync all work. Windows has had far
less use than macOS.

`data/` also carries the arcade work: every romset launch-tested against FBNeo
and two MAME versions, with the results in `arcade-core-test.json`.
