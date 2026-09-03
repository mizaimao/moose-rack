# The tint

A flat wash over the whole app on the AYN Thor, Android 13. **Fixed.** It was
Android's default focus highlight, drawn over the entire webview from the first
press of the stick or the d-pad.

Reported by Frank repeatedly from 2026-08-26. Roughly a dozen attempts missed
it, and this file is kept because *how* they missed it is worth more than the
one-line fix. Every one of them was aimed inside the app; the wash was painted
after the app had finished.

## The fix

`MainActivity.onWebViewCreate`, one line:

```kotlin
webView.defaultFocusHighlightEnabled = false
```

Set at creation, before anything can focus the view. Turning it off later does
not help: the drawable is chosen when focus changes, and once it is installed it
stays until focus changes again.

## What it was

Since Oreo, a focused view that has no focus state of its own gets one drawn for
it. `View.switchDefaultFocusHighlight` fetches `?android:attr/selectableItemBackground`
— a ripple — and paints it over the view's whole bounds.

Three conditions, all met here:

* **The view is focused.** The webview fills the window and is the only focusable
  thing in it.
* **It has no focus state of its own.** Its background is `setBackgroundColor(#14161A)`,
  a `ColorDrawable`, which is not stateful. A null background would have done the
  same.
* **The window is out of touch mode.** This is the trigger Frank kept describing
  and nobody followed. A launch starts in touch mode, so the first frames are
  clean; the first d-pad or stick press leaves touch mode and the highlight is
  installed. Touching the screen afterwards does not undo it, because nothing
  re-runs `switchDefaultFocusHighlight` unless focus changes.

The colour is arithmetic, not a coincidence. `colorControlHighlight` in a
night-mode Material theme is `#33FFFFFF` — twenty per cent of white — and
`RippleBackground.FOCUSED_ALPHA` is `0.6f`. Twenty per cent of white at
six-tenths is **twelve per cent of white**, which is what was measured every
time.

## Why every fix aimed at the app missed

Everything follows from *where* it is drawn: a View foreground, after Chromium
has handed over its raster and before the layer reaches the compositor.

* **The page cannot see it.** `getComputedStyle` reports the colour the
  stylesheet asked for, because the stylesheet got what it asked for.
* **Nothing in the app is underneath it.** Page background, webview background,
  window background, theme, splash — all of them are below Chromium's output,
  and the highlight is above it. A full-screen opaque `<div>` at
  `z-index: 2147483647` was still washed.
* **SurfaceFlinger is honest.** `alpha=1`, `blend=NONE`, `colorTransform=[identity]`.
  The wash really is in the raster the app hands over; it is just that the app
  puts it there in `View.draw`, not in Blink.
* **`forceDark` and algorithmic darkening were never involved.** Both were set,
  through both the platform and `androidx.webkit`, and neither changed the
  numbers — because there was nothing there to turn off.

## The measurement that settles it

Run on the device with the app focused after a d-pad press. A full-screen div is
painted a known colour; `Page.captureScreenshot` over the DevTools protocol reads
Chromium's own raster, and `adb exec-out screencap` reads what the device
composites.

| page paints | Chromium raster | device, before | device, after |
|---|---|---|---|
| `#000000` | `#000000` | `#1F1F1F` | `#000000` |
| `#14161A` | `#14161A` | `#313236` | `#14161A` |
| `#FF0000` | `#FF0000` | `#EA3D31` | `#EA3323` |

Chromium's raster was always correct. That single comparison — the same frame,
read at two points in the pipeline — locates the wash between Blink and the
compositor, which is the Android view layer and nothing else. It should have been
the second measurement taken.

Whole-frame check, no div: before, exactly one pixel in 2,073,600 was darker than
`#1F1F1F`. The screen had a floor. After, the darkest pixel is `#000000` and 1.34
million pixels sit below the old floor.

## The number that sent it wrong, and why

The doc that preceded this one concluded, in bold, that the wash was "a contrast
transform, not something drawn on top", and ruled out every overlay theory on the
strength of one measurement: `#FF0000` came out `#EA3D31`, which no white overlay
produces. That was correct arithmetic about the wrong pipeline.

`adb exec-out screencap` reads back in **Display-P3**. Convert twelve per cent of
white over `#FF0000` from sRGB into P3 and you get `(234, 61, 49)` — `#EA3D31`,
exact on all three channels. The overlay theory fitted perfectly the whole time;
the capture was wide-gamut and nobody checked.

The residual `#EA3323` in the table above is the same effect with the wash gone:
sRGB red expressed in P3 is `(234, 51, 35)`. The panel shows the right red. Any
future measurement of a saturated colour on this device has to account for this;
near-neutral darks are unaffected, which is why `#14161A` reads back as itself.

The "canvas pixels are exempt" claim was wrong too, and for a duller reason —
whatever region was sampled for it, the highlight covers the view's bounds and
lands on canvas and artwork alike. The floor measurement above is the check that
should have been run instead.

## What the earlier attempts were worth

Each was built, installed and measured, and none moved the numbers. They are
listed because the list is now evidence, not a lament: nothing reachable from
inside the app could have worked.

* The stylesheet, including the page painted pure black with `!important`.
* The `@supports not (color-mix(...))` fallback block, deleted live on the
  device. `color-mix()` really is unsupported at Chromium 109 and the app really
  does need that block — it is a separate, real problem, and not this one.
* The backdrop canvas: hidden, shown, removed from the DOM, and its WebGL context
  switched to `alpha: false`.
* `backgroundColor` on the window in `tauri.conf.json`.
* The Android theme's `windowBackground`, `colorBackground` and
  `windowSplashScreenBackground`, in both `values/` and `values-night/`. These
  stay — they fix a genuinely pale *first frame* over the launcher wallpaper,
  which is a different bug that was solved along the way.
* `WebView.setBackgroundColor`, in `onWebViewCreate` and again on focus change.
  This stays too: with a backdrop running the page is transparent by design, and
  a transparent webview defaults to white.
* `WebSettings.forceDark`, `isAlgorithmicDarkeningAllowed`, both again through
  `androidx.webkit`. `stopDarkening` stays as housekeeping, not as a fix.
* `<meta name="color-scheme" content="dark">`. Stays for the same reason.
* Android accessibility filters. All off, and screencap would not have shown them
  anyway.

One attempt made things visibly worse before being found by bisection: an
`html.mobile.backdrop-on { background: var(--bg) }` added while looking for the
tint. It was removed, correctly, but the reasoning recorded at the time was
wrong — it was not a contributor to the wash, it was hiding the shader behind an
opaque page. The comment in `style.css` says so now.

## What to do with the next one of these

* **Bisect early.** It found the one real contributor in two steps after ten
  rounds of hypothesis-led guessing found nothing.
* **Believe the trigger.** "The moment I move the left stick or dpad" was in
  every report from the first one. It is a touch-mode transition, it is a
  platform-level fact, and no explanation that could not account for it should
  have been pursued as far as any of these were.
* **Measure at two points in the pipeline before theorising about one.** Blink's
  raster versus the composited frame is one command each and answers "who is
  doing this" outright.
* **Know your capture's colour space.** One un-checked assumption about
  `screencap` produced a confident, bolded, wrong conclusion that steered several
  rounds of work.
