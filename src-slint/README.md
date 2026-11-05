# Slint prototype

A try-out of [Slint](https://slint.dev) as the one front end for Android, macOS
and Windows (and Linux after them), against the Tauri page in `ui/`. Branch
`slint-prototype`.

It redraws the current app's building blocks with its colours, icons and font:
the header (Back, title, Sofa/Desk switch, search, icon size, grid or list,
filter, random, sort, sync, settings), the tab row with its Q and E keys, the
console column, the cover grid and list rows, the detail pane with chips,
buttons and a download bar, the Sofa detail panel, Settings (toggles, a
segmented choice, a slider) and the toast.

The data is the real library, through the shared core (`src/library.rs`): the
consoles and games in the same cache the desktop app uses, kept current by the
same library update (`moose_rack::commands::sync_library`) in the background.
Covers load only near what's on screen and are shrunk to card size on a
background thread (`fetch_covers` in `src/main.rs`); every game in a list has
a card, and full-size art for the arcade's 2,501 would take gigabytes. With no
config, with `MOOSE_SLINT_SAMPLE=1`, or for a snapshot without
`MOOSE_SLINT_REAL=1`, it shows the old sample instead (`src/sample.rs`).
`MOOSE_SLINT_PLATFORM=gba` opens on one console, and
`MOOSE_SLINT_SNAPSHOT_WAIT` gives a snapshot time for covers to arrive.

On macserver with the real library (13,041 games, 2026-11-02), `footprint`
gave 77 MB open on the Game Boy Advance with covers drawn, against 192 MB for
the desktop app (`docs/memory-footprint.md`).

Play goes through the shared launch (`moose_rack::play`): the checklist first,
shown as a window only when the game or its emulator has something to fetch,
with each line's size and a Download button for the total; then the saves,
the emulator and the launch record. The launch runs on a thread of its own,
and the window won't close while a game runs. A save conflict or an
unreachable server is said in a toast for now; the desktop app's dialogs for
them are still to port. `MOOSE_SLINT_TEST_PLAY=ps2/Crazy Taxi (USA).chd` presses
Play on a game and `MOOSE_SLINT_TEST_ACCEPT=1` presses the checklist's button,
for running it where nobody is at the screen. On macserver (2026-11-03) that
downloaded Crazy Taxi and PCSX2, booted the game and uploaded its launch
record. The game details and the remaining controller rules are next.

## Renderer

Skia, decided with Frank on 2026-10-31. Slint hands it the drawing, and only
the last step differs per platform: Metal on macOS, Direct3D on Windows,
OpenGL ES or Vulkan on Android. The same drawing code and the font we ship
should make every platform look alike; the snapshot mode below is how that gets
checked rather than assumed.

What the prototype costs on macserver at 2560x1600 Retina, by `footprint`:

| Renderer | Memory |
|---|---|
| Skia (Metal) | 18 MB |
| Software | 81 MB |
| FemtoVG (OpenGL) | 147 MB, of which an empty window is 118 MB |

Android still has to be measured on the Nova; the Mac number says nothing
about its drivers.

The current app's frosted glass needs `backdrop-blur`, which arrives in Slint
1.19 (unreleased on 2026-10-31) and works under Skia.

## Controller first

One cursor goes everywhere: the header's controls, the console column, the
grid, the detail's buttons, Settings' rows. Keys and pad buttons both become
the actions `moose_rack::binds` names, with the core's repeat timing and stick
dead zone (`padpoll`), so the prototype moves the way the app does. The mouse
works but nothing needs it.

The cursor is never lost (rule 2 in `docs/controller-ux.md`): each console and
tab remembers the game it was on, clearing a search goes back to it, a library
update leaves it on the same game, the header keeps its control and Settings
its row, and closing Settings returns to wherever it was opened from. Start
opens and closes Settings.

Each button keeps its job (rule 5): the left face button stars the game, the
top one opens Options for wherever the cursor is (take offline or remove, the
star, search, sort, grid or list, a random game; `o` on the keyboard). Settings
and Options take every press while open, and the hint bar shows a button only
where it does something.

Big lists are quick to cross (rule 6): a held direction or trigger speeds up
the longer it is held, the triggers move a screen at a time, and Options has
Jump to letter (left and right pick it) when the list is sorted by name.

Search works with the pad alone (rule 7): with a controller connected it opens
an on-screen keyboard along the bottom, and the results narrow above it as
each key types. The left face button deletes, the top one types a space,
Start or Done closes it, and a real keyboard types into it too.
`MOOSE_SLINT_TEST_KEYS=right,down,settings,back`
plays those actions (`type:zelda` types a search), prints where the cursor is
after each, and quits.

Pads are read with SDL3 (`src/pad.rs`), which reports what kind each one is
(Xbox 360, Xbox One, PS3 to PS5, Switch Pro, Joy-Con) and what is printed on
its buttons; the hint bar draws those, PlayStation's shapes included, and the
keyboard's keys when no pad is connected. Confirm is the right face button and
back the bottom one on every pad, whatever is printed there: A on Nintendo,
Circle on PlayStation, B on Xbox (Frank's call, 2026-10-31). That is the core's
A/B swap, `Bindings::pad_map_swapped`.

It links the SDL3 Homebrew installed (macserver and the laptop both have
3.4.18). On Android the pad arrives as the system's own key events instead;
that path is not written yet.

## Running it

Build on macserver, as the main app is, then run the binary:

```sh
./scripts/fetch-fonts.sh            # Noto Sans, which the UI embeds
cd src-slint && cargo build --release
./target/release/moose-slint
```

Keys: arrows move, Return opens (Sofa) or plays (Desk), Esc goes back, `/`
searches, Q and E change tab, R picks at random. The mouse works throughout.

`MOOSE_SLINT_SNAPSHOT=out.png` saves the window as a picture and quits;
`MOOSE_SLINT_SCENE` picks desk, sofa, detail, list, settings or download.

It is its own Cargo workspace, so it adds nothing to the main lockfile or CI.
