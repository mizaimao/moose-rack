# The cartridge shelf

Games shown as the physical thing — a 3D cartridge with its real label, turned
towards you, with a sound and an insert animation when you pick one. Not built.
Scoped 2026-08-27 so that picking it up starts from facts rather than a survey.

## Where the idea came from

**Socket**, at <https://depmots.com/socket> — a frontend built for small curated
collections, aiming to "mimic the feeling of rummaging in your handheld cover
when you were a kid". Each platform gets its own 3D model with the original
sticker, its own sounds, and its own insert animation.

Read off its own screenshots: a carousel of N64 cartridges, the centre one large
and tilted to a low angle with the connector pins visible, the neighbours
smaller and face-on. Top bar with clock, L/R platform switcher and battery.
Bottom bar with the selected game's name. It is real 3D — perspective, lighting
on the plastic, modelled pins — not a tilted image.

## The expensive part is already done

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

## Two facts about those images that decide the approach

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

## Three tiers

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

## Traps

**Do not build the candidate list by asking each game for its media path.** This
is ES-DE's own warning, from `generateImageList()` in `Screensaver.cpp`: it is a
`stat()` per game, and on Android that is slow enough that they abandoned the
approach and list the media directories recursively instead. The Thor is
Android and this library is large, so it applies here harder than it does to
them. See [attract-mode.md](attract-mode.md), which has the same note.

**Coverage is not total.** There has to be a fallback — cover art on a generic
shell for the platform — or the shelf will have holes in it.

**Desktop and Android only.** The Flip addon is SDL and draws its own interface;
none of this reaches it. `src-sdl` has a GL renderer if that ever changes, but
it is a separate implementation, not a port of this one.

## What to build first

1. A carousel over the existing card grid, using `physicalmedia` where it
   exists and falling back to `covers`.
2. The slide-in, on selection.
3. Per-platform sounds.

Sounds are trivial code and need sourcing; everything else above is
presentation and can be added one setting at a time.
