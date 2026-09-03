# What the app weighs, and why

**Measurement, 2026-08-22 to 08-24.** 192 MB, and 106 MB of it is WebKit.

> **The decomposition, 2026-08-24 — start here.** Everything the app cannot
> avoid on macOS is 192 MB, and it is not our document.

## Yes — the cost follows the window's area, not what is in it

The question `MOOSE_MEASURE_SIZE` was added to ask, and the answer is clean.
Same script, same arcade console, `no-covers` throughout so no artwork exists
in any run, weighed at rest after the browse. The only thing that varied is how
big the window was.

| window | area | WebContent at rest | samples |
|---|---|---|---|
| 730x523 | 0.38 Mpx | **83.2 MB** | 33 |
| 1032x740 | 0.76 Mpx | **114.9 MB** | 9 |
| 1460x1046 | 1.53 Mpx | **193.5 MB** | 24 |

A straight line through three points is not usually worth much. This one is:

    WebContent ≈ 44 MB + 97 MB per CSS megapixel of window

    0.38 Mpx   measured  83.2   fit  81.0
    0.76 Mpx   measured 114.9   fit 118.2
    1.53 Mpx   measured 193.5   fit 192.4

Every point inside 3 MB. **The window is the memory.** Shrink it to half the
area and the page process roughly halves, with exactly the same library, the
same console and the same two and a half thousand rows in it.

This is the same result as "deleting the whole document changed nothing" and
"a full scroll costs no more than a short one", seen from the other side. All
three say the page process is not holding the interface. It is holding the
surface the interface is drawn on.

### What 97 MB per megapixel means

More than it sounds. A CSS megapixel on a 2x display is four device
megapixels, which is 16 MB of RGBA. So 97 MB per CSS megapixel is about **six
full-window backing stores**, held at rest, with nothing animating.

That is an inference from one number and not a measurement — the six could be
five or eight, and nothing here says *which* layers they are. But it is the
first suggestion in this file of a lever that is neither the artwork nor the
framework floor: **count the full-window compositing layers and see whether
six is the number it needs to be.** Candidates worth ruling in or out one at a
time, the way `no-glass` was: the scroller's own tiles, the sticky group
headings, the preview column, the tab row, and the shader backdrop when it is
on.

### The caveat, and it is a real one

This run was contaminated: 127 of its 240 samples tripped the process counter
and one pass of 1032x740 died on launch and recorded nothing. The table above
is the clean subset only, and the two smaller sizes rest on fewer samples than
the largest.

What makes it worth believing anyway is that the discarded samples are noise
in both directions while the surviving ones are monotonic, tightly grouped,
and fit a straight line — and that the three groups are all the same state
(`HELD`/`SETTLED`) of the same script. What would settle it is two clean
passes per size with nothing else running on the machine.

## Nothing was ever released: the releaser tested for the wrong tag

Found by reading, not by measuring, and it needs no measurement to establish —
the code could not have worked.

`observeCovers` sets up a second observer whose whole job is to put the
placeholder back over a card that has scrolled a screen past the edge, so its
decoded picture can go. Its test read:

```js
if (!art?.firstElementChild || art.firstElementChild.tagName !== "IMG") continue;
```

That was right when a cover was an `<img>`. It stopped being right the day
`drawCover` started putting a **`<canvas>`** on the card instead — the change
that draws artwork at tile size, above. A canvas is `CANVAS`, the test skips
it, and **from that commit until 0.2.787 not one cover was ever released.**

The `<img>` branch below it is only the fallback for when `fitted` returns
null, which in normal use is never. So the path that did still work was the
one nothing takes.

### Why the tests passed

jsdom has no `createImageBitmap`, so `fitted` bails at its own guard and every
cover in `covers.test.js` is the fallback `<img>`. The suite has three tests on
releasing and all three were exercising a path the app does not use. There is a
fourth now that stands a canvas up by hand, and it fails against the old
condition.

The general lesson, and it is the same one this file keeps learning: a test
that cannot reach the real path is not evidence about the real path.

### What it was costing

Bounded by windowing on the long lists, and not bounded at all on the short
ones — which is the opposite of where you would want the bug.

Above `visible.THRESHOLD` (400 rows) a card that leaves the window is removed
from the document and its canvas goes with it, so the leak is only the band
between the release margin (1.0 screen) and the window's overscan (1.5
screens) — about one screenful of cards, call it 5 MB.

Below 400 rows nothing is windowed, the whole list is in the document, and the
releaser is the **only** thing bounding how many pictures are held. **Fifteen
of Frank's twenty-four consoles are under it:**

| console | games | held after a browse | intended |
|---|---|---|---|
| megadrive | 942 | windowed | — |
| famicom | 416 | windowed | — |
| mastersystem | 352 | **all 352** | ~3 screens |
| gamegear | 330 | **all 330** | ~3 screens |
| pcengine | 290 | **all 290** | ~3 screens |

At about a quarter of a megabyte a canvas, browsing mastersystem end to end
held ~85 MB of artwork where the margins intend about 25 MB. Not measured in
the app yet — the rig kills a running instance and Frank's was up — so that is
arithmetic from the tile size, not a weighing, and it should be weighed before
it is quoted.

Fixed in 0.2.787: `holdsPicture` accepts either tag.

## A full scroll does not cost more than a short one

The obvious question, and the answer is not the obvious one. Same console, same
build, one flag: scroll a tenth of the way down and stop, against scrolling to
the bottom.

| | peak |
|---|---|
| scrolled to the bottom | 692.9 MB |
| scrolled a tenth of the way | 791.7 MB, 674.5 MB |

**Going ten times as far costs nothing extra.** Whatever this is, it is not
accumulating as the list is travelled.

And the shape of it says where it comes from. Weighed at each step of the
short scroll:

    2%: 761 MB   4%: 678 MB   6%: 516 MB   8%: 524 MB

It is at its highest at the *start* and comes down from there. The peak arrives
within a second or two of the first scroll and is a high-water mark; everything
after is WebKit giving memory back faster than the scrolling takes it.

So "a full scroll costs 700 MB" was the wrong way to read the number. What
costs 700 MB is **opening a big console and touching the scroll wheel once**.
Whether you then read one screen or two thousand makes no difference at all.

That moves where the fix has to be. Not the scrolling — the first draw: a list
of 2,504 rows laid out into a container eighty thousand pixels tall, forty
covers asked for at once, and the whole thing rasterised for the first time.
Row recycling, which was the plan, would not touch this. Asking for fewer
covers in the first pass and letting the list settle before filling it might.

## Why the floor is 192 and browsing costs 600

Both numbers are right and they are not in conflict. 192 MB is what the app
weighs when it has finished and been left alone. The rest is WebKit's *working*
memory, which it takes while drawing and hands back twenty to thirty seconds
later — decoded pictures, compositing layer backing stores, and the tiled
backing for a scroller that is eighty thousand pixels tall.

It is not the document. Deleting the entire document changes the page process
by nothing; the table below shows that.

Which means the lever is not "have less on the page" — there is barely anything
on the page — it is "let WebKit throw away what it is not showing".

### Containment: 853 MB down to 704

`content-visibility: auto` tells the engine it may skip rendering a card that
is off screen, and drop what it had already rendered for it.
`contain-intrinsic-size` is what stops the scrollbar jumping about while it
does: a skipped card with no intrinsic size measures zero and the page
collapses.

Two passes each, spread about twenty megabytes:

| | peak over a full scroll | at rest |
|---|---|---|
| as it was | 852.6 MB | 325.9 MB |
| with containment | **704.1 MB** | 335.5 MB |

**148 MB off the peak.** At rest it makes no difference, and that is the right
shape: this is a fix for what the app weighs while it is being used hard, and
the peak is what kills a process on a phone rather than a steady state nobody
is waiting on.

On by default since 0.2.785. `body.uncontained` turns it off, which is how the
table above was measured.

## Where 300 MB actually is

Measured by counting from *inside* the page and weighing from outside at the
same moment, then taking the whole document away and weighing again.

| step | total | page | Rust | GPU | what the page held |
|---|---|---|---|---|---|
| console screen | 356.8 | 261.5 | 55.6 | 32.1 | 393 nodes, 34 pictures |
| arcade open | 335.7 | 237.2 | 57.8 | 33.0 | 456 nodes, 1 picture, 2,504 rows (475 KB) |
| scrolled to the bottom | 334.5 | 237.3 | 56.7 | 32.8 | 449 nodes |
| **left alone 20 seconds** | **193.0** | **106.5** | 49.2 | 29.6 | same |
| **document deleted entirely** | **192.2** | **106.5** | 49.1 | 28.9 | 7 nodes, no arrays, no stylesheet |

Read the last two rows twice. **Deleting the whole document — every node,
every array, the stylesheet disabled — changed the page process by nothing at
all.** 106.5 MB before, 106.5 MB after.

So:

* **192 MB is floor.** WebKit 106, our Rust process 49, the GPU process 29,
  networking 6. None of it is the interface, and none of it comes back by
  making the interface smaller.
* **~130 MB is WebKit's cache**, which it holds for twenty to thirty seconds
  after browsing and then returns without being asked. That is the "300 MB"
  and most of the "560 MB peak".
* **The document itself is nothing.** 449 nodes. One picture. 475 KB of row
  data for two and a half thousand games. All of it together does not show up
  against a floor twenty times its size.

Four separate optimisations were proposed in this file before this measurement
and three of them were aimed at the document: the artwork, the glass, the
backdrop. Only the artwork was worth anything, and only for the peak.

### What is actually worth attacking

* **The Rust process at 49 MB.** A Tauri app with nothing in it idles at
  30–80 MB *in total*; ours spends 49 on the native half alone. That is
  entirely ours — SQLite's page cache, the tokio runtime, the API client, and
  whatever the cache holds between calls — and it is the one number here that
  no framework is imposing.
* **WebKit's 106 MB floor.** Not ours to set. It is the price of a webview and
  it is the honest cost of the Tauri decision. Worth knowing before the
  Android port, where the WebView is in-process and this floor lands inside
  the app's own limit.
* **The 34 console pictures**, which were the last artwork in the app still
  loading the old way and cost about 44 MB of the console screen. Fixed in
  0.2.782 — `fitConsoleArt` draws them at tile size like everything else.


> **Read this section first.** The measurements in this file contradict each
> other and none of the conclusions drawn from them are safe. What follows is
> kept so the record is legible, not because it is right. The one thing here
> that is settled is the asset-request bug, and it is settled because it
> reproduces every time.

## Settled: a picture costs by its own size, not the size it is drawn

Three earlier attempts at this contradicted each other, every time because the
layout changed between runs and so did how many pictures were actually
painted. This one holds the layout absolutely still — twenty slots of exactly
200x150, `object-fit: cover`, all of them on screen, identical in every run —
and varies only the resolution of the file behind each slot. Loaded one at a
time, held at 1, 5, 10 and 20 long enough for the sampler outside to catch
each step.

| art | file | drawn at | 0 up | 1 | 5 | 10 | 20 | per picture |
|---|---|---|---|---|---|---|---|---|
| 3dboxes | 325x600 | 200x150 | 280.2 | 328 | 324 | 343 | **331.3** | **2.6 MB** |
| miximages | 1280x960 | 200x150 | 239.4 | 263.1 | 326.2 | 361.9 | **471.1** | **10.9 MB** |

The miximages column is clean — the spread across repeated samples at each
step is 0.1 MB — and it is monotonic: 239, 263, 326, 362, 471. The slope from
ten pictures to twenty is 10.9 MB each.

**Both are drawn at exactly the same size. One costs four times the other.**
The only thing that differs is the file: 1,228,800 pixels against 195,000, a
ratio of 6.3. The cost ratio is 4.2 — not perfectly linear, because there is a
fixed overhead per picture, but the direction is not in doubt.

So the claim made earlier in this file — that WebKit already decodes at the
drawn size, the way Coil does with `inSampleSize` and QML with `sourceSize` —
is **wrong**. It decodes at the file's size. 10.9 MB for a 1,280x960 picture
is about 2.2x a plain RGBA decode of it, so there is a compositing or GPU copy
on top of the decode as well.

Frank's instinct throughout — that the size of the artwork must matter — was
right, and three of my measurements said otherwise because all three were
measuring the layout instead.

### What this makes possible

Ninety cards of miximage artwork is around 980 MB by this measurement, which
is the arcade screen, and it is the whole problem.

The fix does not touch a single file on disk. Fetch the picture as a blob,
`createImageBitmap` it, draw it into a `<canvas>` the size of the tile, and
`close()` the bitmap. What is *retained* is then the canvas — 200x150 at 2x is
0.24 MB — instead of the decoded file. The full-resolution original is still
what the detail pane opens, still exactly the bytes that were downloaded, and
still the only copy that exists.

Twenty pictures would go from 218 MB to about 5 MB. The peak becomes one
decode at a time rather than ninety held at once.

Safari has historically ignored `createImageBitmap`'s `resizeWidth` options,
so the intermediate bitmap may well be full size — but it is transient and
closed immediately, and only one exists at a time. That is the difference
between a 4.9 MB spike and a 980 MB resident set.

## The peak is the list, not the pictures

Three completed runs of the same browse, same build, one flag apart:

| scrolling the arcade console | peak |
|---|---|
| no artwork at all (`no-covers`) | **573.7 MB** |
| covers drawn into canvases | 653.6 MB |
| covers as plain `<img>` | 972.3 MB |

So of the 654 MB peak, **574 MB is there with no pictures on the screen at
all.** The artwork is the difference between 574 and 654; before the canvas
change it was the difference between 574 and 972. That part is dealt with. The
list is not.

At the peak, 451 MB of the 574 was the page process.

### The suspect, untested

`backdrop-filter` appears eighteen times in `ui/style.css`, and one of them is
on `.gcard .art`. That means every card on screen is its own compositing layer
holding its own blurred copy of what is behind it. One is cheap; eighty-five,
replaced wholesale every time a windowed list jumps, is the obvious candidate
for 451 MB of page process.

`body.plain-cards` turns it off and leaves a flat panel of the same color. It
is in the stylesheet and inert — nothing sets it but the measuring switch — and
it is worth having as a real "reduce transparency" setting whatever the
measurement says, because a handheld wants it anyway.

**It has not been measured.** Three attempts to measure it wedged, which is the
next section.

### The rig, repeatable at last — and the glass is not the answer

Three passes per setting, every pass from the same screen, no artwork loaded
at all, and a process counter that says out loud when it has not found exactly
four processes. Every one of the six browsed.

| | peak | at rest | page at rest |
|---|---|---|---|
| `no-covers` | 561.1 MB (spread 17) | 298.1 MB (spread 5) | 209 MB |
| `no-covers,no-glass` | 560.0 MB (spread 39) | 300.5 MB (spread 4) | 208 MB |

**`backdrop-filter` costs nothing measurable.** One megabyte between them at
the peak and two at rest, against a spread of up to thirty-nine. The suspect
that had been named twice in this file is cleared. `body.plain-cards` stays as
a "reduce transparency" setting, which is worth having on its own terms, but it
is not a memory fix and this file should not have implied it would be.

> **Narrower than it reads, found 2026-08-24.** `body.plain-cards` turned the
> glass off on `.gcard .art`, `.card` and `.row.sel` — and not on `#detail`.
> The preview column is 460px of full-height glass at `blur(18px)`: the
> strongest blur in the app, on much the largest surface carrying one, on
> screen the entire time in three columns. So what this table clears is the
> *cards*, which are small and numerous. The single big pane was never in the
> comparison and its cost is still unknown.
>
> That matters more since the area result above: 97 MB per CSS megapixel is
> about six full-window backing stores, and a blur that size needs a snapshot
> of what is behind it plus intermediates to blur it in. `#detail` is in the
> switch now, so the next `no-glass` run asks the question this one did not.

The rig itself took three goes to get right, and each wrong version produced a
confident number: `pid+8` guessing missed the helpers entirely and read 54 MB;
counting every WebKit process on the machine picked up Safari's and read
2,476 MB. It now records which WebKit processes existed before launch and
counts only the ones that did not, and prints a warning when the count is not
four. One sample in this run tripped it and was discarded.

### So where the memory actually is

Three numbers, all of them now repeatable:

* **236 MB** — idle on the console screen, nothing of ours painted.
* **299 MB** — a console with 2,504 games open, sitting still, no artwork.
* **561 MB** — the same, while being scrolled top to bottom.

The list at rest costs about **63 MB** over the floor, which for 2,504 rows is
reasonable. The peak is a **260 MB transient that arrives during scrolling and
falls away afterwards**, and it is neither the artwork (there is none in these
runs) nor the glass (it is the same with it off).

What is left is the churn: the windowed list throws away eighty-five cards and
builds eighty-five more on every jump, thirty times over the length of this
browse, and WebKit does not hand the memory back until well afterwards. The
scroll here is a deliberate worst case — thirty instant jumps, each replacing
the whole window — where a person scrolling would reuse most of the rows.

The fix, if this is worth fixing, is the one every native list view uses:
recycle the row elements instead of rebuilding them. That is a real piece of
work in `visible.js` and it is now the only image-free optimisation on the
table.

### There is no wedge. It was the rig.

The section that stood here said opening a big console froze the page, and
that it was probably behind "it feels slow and laggy when browsing platform
games". Both claims were wrong and they are withdrawn.

What settles it: a diagnostic that ticks a counter every 100 ms and traces
every command in and out, so a blocked thread and a Rust call that never
answers can be told apart. Clicking into arcade —

    about to click arcade: found
    click returned after 3 ms
    beat +19 in flight: none
    cards appeared: 50 after 1s

Nineteen to twenty beats every two seconds throughout, never a dropped one,
never a command outstanding. **The console opens in one second and the page is
never blocked.**

The freeze was mine. The measuring script killed the app and relaunched it
three seconds later, and three seconds is not enough — the new instance came up
beside the previous one's WebKit helpers, which outlive their parent, and sat
there doing nothing. `peak.sh` now waits for the process to actually be gone
and then waits six seconds more, and every pass since has browsed.

Two days were spent chasing a bug in a test harness and reported as a bug in
the app. The lesson is in the rig now: a run that does not reach `scrolled 80%`
is a run that did not happen, and it is checked rather than assumed.

### Whether the glass costs anything: still not known

`body.plain-cards` exists and works. What it is worth does not survive
measurement:

| | peak | at rest |
|---|---|---|
| `no-covers,no-glass` | 577.5 MB | 301.9 MB |
| `canvas` | 805.7 MB | 322.9 MB |
| `canvas,no-glass` | 1,023.6 MB | 777.7 MB |

The last row is impossible — turning the glass *off* cannot cost 450 MB — so
the run-to-run variance is larger than the effect being looked for. An earlier
`no-covers` run peaked at 573.7 MB and this one at 577.5, which is
reassuringly close; `canvas` peaked at 653.6 MB before and 805.7 now, which is
not.

What the rig still needs before this question can be asked again: an identical
starting screen every run rather than whatever the app restored, a fixed number
of covers loaded, and several passes per configuration so the spread is visible
instead of being mistaken for a result. Until then, no claim.

The one thing that did survive is the earlier three-way run, where all three
passes completed and the ordering was large and consistent: **574 MB with no
artwork at all, 654 with canvases, 972 with `<img>`.** The list dominates the
peak and the artwork change is worth about 300 MB of it.

## What drawing at the shown size saved

One build, one browse, one flag — `MOOSE_MEASURE_FLAGS=no-canvas` turns
`fitpicture.js` off and the covers go back to plain `<img>`. Same arcade
console, same scroll to the bottom, same twenty pictures on forty-four cards
at the end. Window moved off the side of the display rather than hidden: a
hidden window stops being rendered and the page never lays out.

| | `<img>` | canvas | saved |
|---|---|---|---|
| peak while scrolling | 972.3 MB | 653.6 MB | **319 MB (33%)** |
| settled, straight after browsing | 535.6 MB | 335.6 MB | **200 MB (37%)** |
| after half a minute idle | 355.5 MB | 335.6 MB | 20 MB (6%) |

Against the 235.6 MB floor, the arcade grid itself costs **300 MB before and
100 MB after** — a third of what it was.

The last row is the honest caveat: left alone for thirty seconds WebKit prunes
its own cache and the two converge. The saving is in the peak and in the
working set — what the app weighs *while it is being used* — and that is
exactly the number that matters on a phone, where a spike is what gets the
process killed rather than a steady state nobody was waiting on.

## The baseline, at last: 235.6 MB with nothing of ours on the screen

Asked for repeatedly and put off for too long while I chased the artwork.
Measured with a full-screen opaque overlay, so no card, no cover and no
backdrop is being painted, and with the per-process split kept for every
sample:

| | as loaded | fully covered |
|---|---|---|
| moose-gui (Rust: cache, API client, Tauri) | 48.7 MB | 48.7 MB |
| WebKit GPU | 27.6 MB | 27.6 MB |
| WebKit Networking | 6.2 MB | 6.2 MB |
| WebKit WebContent (the page) | 153.1 MB | 153.0 MB |
| **total** | **235.6 MB** | **235.5 MB** |

Covering everything changes nothing — a tenth of a megabyte. So on the console
screen the floor is structural, not painted: it is the runtime, the page, the
row data and WebKit's own baseline, and hiding things does not touch it.

Two thirds of it is WebContent. A blank WKWebView is usually somewhere around
60–90 MB before a page is loaded, so our page is plausibly another 60–90 on
top — but that split has *not* been measured and should not be quoted as if it
had.

This is the number every optimisation gets measured against.

## A burst of asset requests wedges the page

Found on the way. Sixty `<img>` pointed at `asset://` URLs *at once* never
load and never fail: no `onload`, no `onerror`, nothing in two minutes, and
the process sits perfectly still. The same sixty loaded one after another all
succeed, every time.

`flushCovers` sets up to forty `src`s in a single pass. That is very likely
the same burst, and very likely why three attempts to open the arcade console
from a script froze the page — and worth holding against "it feels slow and
laggy when browsing platform games", which has been an open complaint for a
long time and was never explained.

This is a real bug and it outranks anything about memory.


Measured on 2026-08-22, on Frank's Mac, against his own library — 24 consoles,
2,504 arcade games, `list_art = "miximages"`. Numbers are macOS *physical
footprint*, which is what Activity Monitor's Memory column shows; RSS
overcounts shared libraries badly and is not comparable between the two front
ends.

Repeat any of this with `MOOSE_MEASURE=tools/browse.js` (Tauri) or
`MOOSE_SDL_SHOT` / `MOOSE_SDL_FRAMES` (SDL). Neither needs anyone watching.

## The numbers

| | Tauri | SDL |
|---|---|---|
| Fresh launch, consoles screen | **228 MB** | **227 MB** |
| Arcade grid open, scrolled to the bottom | **peak 1,030 MB**, settling to 550 | not measured |

The two front ends start out level. The whole difference is what happens when
a wall of artwork goes past.

Tauri's 228 MB splits as moose-gui 33, WebKit GPU 29, Networking 6, WebContent
160. After browsing, WebContent alone is 447 MB and the peak was over 850.

## The A/B that changes the reading

Same script, same library, only `list_art` different — so only the size of the
artwork changed:

| `list_art` | decoded, each | peak | settles at |
|---|---|---|---|
| `3dboxes` (325x600) | 0.78 MB | 691 MB | **436 MB** |
| `miximages` (1280x960) | 4.9 MB | 1,030 MB | **550 MB** |

Six times the artwork did not cost six times the memory. It cost 1.26x.

Work back from the settled figures, over the 228 MB consoles-screen floor:

* 3dboxes: 208 MB of images at 0.78 MB each = about 266 pictures held.
* miximages: 322 MB of images at 4.9 MB each = about 66 pictures held.

Sixty-six against two hundred and sixty-six. **WebKit is already bounding its
decode cache by bytes rather than by count** — the same shape as ES-DE's
`MaxVRAM`, arrived at independently, and it holds a few hundred megabytes of
pictures whatever size they happen to be.

That is the opposite of what the first measurement suggested, and it matters:
there is no unbounded leak in the webview to go and fix. What is left is a
budget somebody else chose, sized for a Mac with plenty of memory, and a
**peak** — 691 MB, 1,030 MB — reached before the pruning catches up.

The peak is the number that matters on Android. A steady state of 550 MB on a
6 GB device is uninteresting; a transient gigabyte inside a single-process
WebView on a 2 GB device is a kill.

## Why the pictures cost what they do


The artwork is 1,280×960. Decoded that is 1,280 × 960 × 4 = **4.9 MB per
picture**, and the grid draws them 150 points wide. Every cover on screen
costs about ten times what it needs to.

That single number explains everything else:

* 447 MB of WebContent ÷ 4.9 MB ≈ 91 covers held. It is not WebKit being fat
  in general. It is ninety-one full-size pictures inside a budget that would
  have held four hundred small ones.
* `src-sdl/src/covers.rs` caps the cache at 192 *textures* and its comment
  works that out as 150 MB, on the assumption that a cover is 786 KB. At
  4.9 MB the same cap is **940 MB**. The bound is wrong, and it is wrong in
  the dangerous direction — the SDL front end only looks thrifty because
  nothing has yet scrolled far enough to reach it.

A count is not a budget. Both caches need to count bytes.

## How ES-DE gets away with it

It is the obvious comparison: ES-DE runs on Frank's MagicX Mini Zero 28, which
has 2 GB, with the same full-size artwork, and it is fine. It does not
downscale — `TextureData.cpp` loads at native size and uploads BGRA8 straight
to the GPU, four bytes a pixel, exactly like ours.

What it has is a byte budget with eviction. From `TextureDataManager.cpp`:

```cpp
size_t settingVRAM {static_cast<size_t>(Settings::getInstance()->getInt("MaxVRAM"))};
size_t max_texture {settingVRAM * 1024 * 1024};
...
for (auto it = mTextures.crbegin(); it != mTextures.crend(); ++it) {
    if (size < max_texture)
        break;
    (*it)->releaseVRAM();
    (*it)->releaseRAM();
    mLoader->remove(*it);
    size = TextureResource::getTotalMemUsage();
}
```

Least-recently-used first, and it drops *both* copies — the GPU texture and
the decoded bytes in RAM. `MaxVRAM` is a user setting, clamped to 128–2048 MiB.

So the answer to "how do they show full-res artwork on 2 GB" is: they hold
however many full-res pictures fit in a fixed number of megabytes, and throw
the rest away. Sixty-four full-size covers is 313 MB and is far more than is
ever on screen at once. Nothing is resampled and nothing is lost.

## What that means for us

**SDL:** the fix is small — make `covers.rs` count bytes instead of textures,
with a budget rather than a count, and pick the budget from the machine.

**Tauri:** we cannot set the budget — WebKit's decoded-image cache is not ours
and there is no API for "hold at most 200 MB of images". But the A/B above
says it already has one. What we can change is what goes into it, and the
peak, which is where the danger is.

The lever we do have is the asset protocol: the page asks for a picture over
a URL we serve, so the *served* bytes can be a resized copy while the file on
disk stays exactly as downloaded. That is the same trade Android's Coil and
Glide make with `inSampleSize` — decode at the size you are going to draw —
and it is invisible to the source. The detail pane, which draws big, keeps
asking for the original.

Halving the served size quarters the bytes, which both lowers the peak and
lets four times as many covers sit inside whatever budget the platform has
chosen. It is the only lever that works on a cache we do not own.

This matters most on Android, where the WebView is single-process: it runs
*inside* our app, against the per-app limit, with no separate WebContent
process to absorb the spike. A 1,030 MB peak is not survivable on a 2 GB
device. Chromium also decodes the whole image before resampling it down, so
the intrinsic size is what costs, not the drawn size.


## What is measured and what is not

Measured: everything in the tables above, with the script's own notes
confirming the browse actually happened. `tools/browse.js` reports through the
`measure_note` command because the page's `console.log` goes to the webview
console and nowhere a shell can read it — one run looked flat and was in fact
a browse that never started.

Not measured, and worth doing before anything is built on this:

* **Android.** Every number here is macOS and WKWebView. Chromium sizes its
  decode cache from device memory and decodes differently. The 2 GB device is
  the question and nothing here answers it.
* **The peak.** Sampled every four seconds, so the true peak is higher than
  1,030 MB. It wants a tighter sampler, or a run under `instruments`.
* **The floor.** 228 MB on the consoles screen is itself unexplained — 24
  console pictures and a strip of covers should not cost that. Worth taking
  apart before optimising anything downstream of it.
* Opening the arcade console by script hung twice with `list_art =
  "miximages"` and worked with `3dboxes`. Frank browses it daily without
  trouble, so this is most likely the script driving the page in a way a
  person does not. It has not been explained, and the miximages row above came
  from a session-restore rather than a confirmed scripted browse.


## Correction, 2026-08-22: the measurements below are not trustworthy

Frank, on being told that showing fewer pictures cost the same memory:

> I strongly disaggree with your testing results. Showing fewer images eat the
> same amount of ram this is mathmatically impossible.

He is right, and the claim is withdrawn. It rested on one pair of numbers —
436 MB before the margin change, 439 MB after — and **the image count was
never measured**, so it was never established that fewer pictures were on the
page at all. A number that does not move is equally consistent with a cache
that will not let go and with a change that never took effect, and those want
opposite fixes. Asserting the first one without checking was the error.

Three attempts to check it since have all failed, and they failed in ways that
say the rig is wrong rather than the app:

* Opening the arcade console from a script freezes the page. Twice through
  `showRoms`, once through a synthetic click on the card. No error, no further
  notes, memory flat. Frank does it by hand daily without trouble.
* A direct experiment — sixty full-size miximages put on screen through the
  asset protocol, weighed, removed, weighed again — never got past putting
  them up. Neither `onload` nor `onerror` fired for any of the sixty in two
  minutes, and the footprint never moved off 228 MB. Assets requested this way
  hang rather than fail, which is worth understanding on its own.

So the only figure below that came from a confirmed browse is the `3dboxes`
row. The `miximages` row came from a session restore. Everything derived from
comparing them — including "WebKit already has ES-DE's budget" — is a
hypothesis with one leg to stand on, not a finding.

**Nothing should be built on this until the rig is fixed.** What the rig needs:
a browse that is confirmed to have happened, a count of the pictures on the
page beside every weight, and a sampler faster than four seconds.

## Tightening the margins did nothing — unverified

Frank's idea, on 2026-08-22:

> even at 1080P or 1440P the screen is capped at 7 or 6 inches. That means not
> many tiles can be displayed. Can we do optimizations that we only load
> images that are on the display and off load the ones that do not so that we
> effectively reduce the total image size to less than 12 or soemthing?

The release margin was a flat 1,600px, which is two screens on the window it
was written against and *eight* on a 720-tall handheld — so the machine with
the least memory hoarded the most. That is worth fixing whatever else is true,
and it is fixed: both margins are now a fraction of the list's own height,
0.4 screens to load and 1.0 to release.

It moved the number by three megabytes. 436 MB before, 439 after — and see the
correction above: nothing counted the pictures, so this does not show what it
was claimed to show. The margin change is right regardless, because eight
screenfuls of hoarding on a handheld is wrong whatever it costs.

The hypothesis it suggested — that releasing the `<img>` does not release the
decode, because WebKit's image cache is keyed by URL and outlives the element —
is a real and documented behavior, but it has not been demonstrated *here*.
It is the first thing the fixed rig should settle.

If it turns out to be true, two things work:

1. **Serve fewer bytes.** Resize in the asset protocol, source untouched. A
   500 KB cover instead of a 4.9 MB one means the same budget holds ten times
   as many, and the peak — the number that kills an app on Android — falls
   with it.

2. **Own the decode.** `fetch` the file as a blob, `createImageBitmap` it,
   draw it into a canvas the size of the tile, and `close()` the bitmap. The
   URL never goes through an `<img>`, so WebKit's image cache never gets an
   entry, and what is retained is a canvas backing store — 300x420, half a
   megabyte — that we can throw away when we say. This is the honest port of
   ES-DE's budget into a webview, and it is the only way to actually get to
   Frank's "twelve pictures".

Both are worth doing and (1) is most of the win for a tenth of the work.


## What the open-source Android frontends actually do

Asked for on 2026-08-22, because "praised on a 2 GB device" and "brute-force
the RAM" cannot both be true. They are not doing anything exotic. They are
doing four ordinary things, and **not one of them touches the source file.**

**1. Decode at the size you are going to draw.** Universal, and the big one.
Coil and Glide do it through `inSampleSize` — read the header, work out the
factor, decode at a fraction (Lemuroid and Emulair are both Coil). Pegasus does
it through QML's `sourceSize`, which Qt's own documentation is blunt about:
images are usually the greatest user of memory in a QML interface, and anything
not part of the interface should have its size bounded this way. Flutter's
`ResizeImage` is the same idea for Yuno.

This is not reducing anyone's artwork. The file stays 1280x960. What changes is
that the decoder is not asked to produce a million pixels for a tile that can
show fifty thousand. The full file is still what the detail pane opens.

**2. A memory cache measured in bytes, with LRU eviction.** Coil's default is
`maxSizePercent(context, 0.25)` — a quarter of what the device has, so it is
80 MB on a 2 GB handheld and 800 MB on a desktop, with no code change.
ES-DE's `MaxVRAM` is the same idea with a manual slider, 128–2048 MiB, and its
eviction drops the GPU texture *and* the decoded bytes.

**3. A disk cache of the decoded result**, so the resize happens once ever
rather than once per scroll. Coil's is `DiskCache.maxSizePercent(0.02)`. This
is exactly Frank's own suggestion — build a thumbnail cache at first launch,
keep the full-resolution files intact — arrived at independently.

**4. Two things that only exist on Android**, and are most of why a 2 GB phone
copes at all:

* `Bitmap.Config.RGB_565` — half the bytes of `ARGB_8888` at the *same*
  resolution, for artwork with no transparency. Glide used it by default for
  years.
* **Hardware bitmaps** (`Bitmap.Config.HARDWARE`, Coil's default on API 28+).
  The decoded image lives in graphics memory rather than the app's heap, so it
  does not count against the per-app limit at all.

A webview has (1) — `createImageBitmap` into a tile-sized canvas — and can be
given (2) and (3). It has no equivalent of (4), and that is a real structural
disadvantage of shipping a webview to Android rather than a native view.

The fallback Frank named is also what the field does: ES-DE's list view, and
most others' default, show one piece of artwork for the selected game and
nothing for the rest. One picture on screen is one picture in memory.
