# Attract mode, as EmulationStation does it

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

## The trigger is one counter, not a per-view thing

`Window::mTimeSinceLastInput` accumulates every frame. When it passes
`ScreenSaverTime` the window calls `startScreenSaver()`. Any input at all calls
`cancelScreenSaver()`, which zeroes it.

That is the whole gate. It lives at window level, above every view, so no
screen has to know it exists. `ScreenSaverTime` defaults to five minutes and
**`0` means off** — worth keeping, because it is the obvious way to expose a
disable switch without a second setting.

## Five modes, of which two are attract mode

`ScreenSaverBehavior`, default `dim`:

| | |
| --- | --- |
| `dim` | fade the current screen down |
| `black` | fade to black |
| `random video` | **attract mode**, game videos |
| `slideshow` | **attract mode**, game images |
| `suspend` | hand off to the OS |

ES-DE calls the setting `ScreensaverType` and drops `suspend`.

## The state machine

    INACTIVE → FADE_OUT_WINDOW → FADE_IN_VIDEO → SCREENSAVER_ACTIVE

Two fades because it is a crossfade: the old screen goes out over `FADE_TIME`
(500 ms) while the first piece of media comes in. Once `SCREENSAVER_ACTIVE`, a
timer swaps media and the state does not change again until input arrives.

    ScreenSaverSwapVideoTimeout    30000 ms
    ScreenSaverSwapImageTimeout    10000 ms

## Picking a game

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

## The trap ES-DE hit, which this app would hit harder

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

## Launching from the screensaver

`ScreenSaverControls`, default true. On input during attract mode,
`launchGame()` sets the gamelist cursor to the game being shown and launches
it. With the setting false it goes to that game's list instead and waits.

Both frontends do this. It is the thing that makes it attract mode rather than
a screensaver, and it is about fifteen lines.

## Not burning the battery

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

## Overlays

Optional, all of them: marquee, game name, system name, date and time with
`strftime` formats, and a decoration frame. `ScreenSaverGameInfo` takes
`never` / `always` / `start & end`.

## What to build first

The two pieces that are actually load-bearing:

1. An idle counter at window level that any input resets.
2. A cached list of games with media, sampled without replacement.

Everything above those is presentation and can be added one setting at a time.
