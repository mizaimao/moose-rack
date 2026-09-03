# Porting to Android

Written 2026-08-24, for the **AYN Thor**: Snapdragon 8 Gen 2, Android 13,
8–16 GB RAM, a 6" 1080x1920 top panel and a 3.92" 1080x1240 bottom one.

The short version: **this is a Tauri port, not an SDL one, and the browsing
half is nearly free.** The launch half is a second implementation and that is
where the whole cost is.

## The route changed

The earlier plan asked "if we have this SDL build, how far are we from an
Android build?" and answered it down the SDL road — cargo-ndk, a Gradle project
carrying `SDLActivity.java`, cosmic-text reading `/system/fonts`. A fortnight,
roughly.

Two things have changed since, and both point the other way.

**Android has a webview, and the Miyoo Flip does not.** That is the entire
reason `handheld-frontend.md` reaches for SDL: the Flip runs its frontend on
DRM/KMS with no display server at all, and Tauri needs webkit2gtk + GTK3, which
needs one. Android ships Chromium WebView and Tauri v2 targets it directly. So
the SDL front end is the right answer for the Flip and the wrong answer for the
Thor — they are two devices with opposite constraints, and one build cannot be
justified by the other.

**The memory objection is dead on this device.** The SDL work was parked
partly against "Tauri's memory turning out to be unfixable on a 2 GB
Android device". The Thor is not a 2 GB device, and the number is not what this
file used to think it was. Measured 2026-08-24 (see `memory-footprint.md`), the page
process is `44 MB + 24 MB per device megapixel`. Applying that:

| | device px | Mpx | list view | grid view |
|---|---|---|---|---|
| the Mac window we kept measuring | 2920x2092 | 6.11 | — | ~650 MB |
| Thor, top screen | 1080x1920 | 2.07 | **178 MB** | **206 MB** |
| Thor, both screens, one webview | | 3.41 | 211 MB | 239 MB |
| Thor, both screens, two webviews | | | 255 MB | 283 MB |

Two to four per cent of an 8 GB device. The 650 MB that has been driving this
whole line of work was a six-megapixel Retina window, not a property of the
app.

**Those constants are WebKit's and Android's webview is Chromium.** Its
renderer floor is generally heavier — call it 1.3–1.6x, so 250–350 MB. Still
nothing here. The projection is reliable about the *shape* (cost follows screen
pixels, not library size) and worth ±50% on the absolutes.

## What is already done, so nobody does it twice

* **`src/` is portable and is most of the app.** 24k lines against `src-tauri/`'s
  3.8k. The seven modules moved out of the webview in 0.2.503 — `binds`,
  `gamelist`, `gamesort`, `gamefilter`, `pickorder`, `pagefilter`, `gridnav`,
  `padpoll` — are all Rust and all come along.
* **`launch.rs` plans and does not spawn.** 474 lines that decide core, shader,
  overrides and save-state slot, and hand back a `Plan`. That half survives an
  Intent rewrite untouched; it is `retroarch.rs` that does not.
* **The focus ring exists** (0.2.78x, `src/focusring.rs`). Full controller
  navigation "for Android" is wanted, and that work has started.
* **Every `cfg` chain has a fallback.** `not(any(target_os = "macos",
  target_os = "windows"))` throughout, so an Android build *compiles*.

That last one is a trap, not a gift. **Android will take the Linux branch of
every one of those and silently do the wrong thing** — look for RetroArch at
Linux paths, write config where Linux writes it, read displays the Linux way.
It will build and it will be wrong, which is worse than not building. Every
`target_os` site in `src/` needs an Android arm or a deliberate decision that
the Linux one is right: 37 macOS, 26 Windows, 1 Linux, **0 Android** today.

## Step 1 — the toolchain

Mechanical, half a day, nothing to design.

* Android SDK, **NDK r28 or later** (28+ gets Google's 16 KB page alignment for
  free), **JDK 17**, `cargo-ndk`.
* `cargo tauri android init` — generates `src-tauri/gen/android/`. There is no
  `gen/android` in this repo today; mobile has never been initialized.
* `cargo tauri android dev` to run on the Thor over ADB, `android build` for an
  APK. The APK is unsigned by default.

## Step 2 — make it browse

This is the part that is nearly free, because the interface is HTML and it
already exists.

* **Feature-gate the TUI out.** `src/lib.rs` has `pub mod tui`, which drags
  `ratatui` and `crossterm` into an Android build that will never draw a
  terminal. Put it behind a `tui` feature that the Android target does not
  enable.
* **`datadir::anchor()` does not survive.** It picks a data root by walking up
  from the executable looking for `data/esde-core-map.json`, then `set_current_dir`s
  to it. On Android there is no meaningful working directory and no executable
  beside your data. This needs an Android arm returning the app's private files
  directory.
* **Check the `asset:` protocol.** Covers are served through it and it must
  match `assetProtocol` in `tauri.conf.json`. Tauri supports it on Android but
  the path shape differs; this is a "verify early" item, not a known problem.
* **Controller input.** Chromium WebView exposes the Gamepad API, which is what
  `ui/js/gamepad.js` already drives at 120 Hz. Expect it to work and budget a
  day for the Thor's specific button mapping.

Browsing should be running on the device inside a week of starting.

## Step 3 — storage

The app's model is a visible, findable library folder. Android's model is
scoped storage. These do not agree and the disagreement is the user's problem,
so it wants deciding rather than discovering.

* The library goes on shared storage the user picks through SAF, or in
  `Android/data/<pkg>/files` where it is visible over USB and dies with an
  uninstall.
* `savesync.rs` (793 lines) and `saves.rs` (608) read and write save files by
  path. Under SAF they would be reading through a content resolver instead.
  This is the quiet second cost of the storage decision and it is not small.

## Step 4 — launching, which is the actual project

**Half of this is done and on the device.** Games start in RetroArch, on the
core ES-DE would have picked, from the card. `android_launch_plan` in
`src-tauri/src/lib.rs` turns a game into an ordered list of components and core
files; `Bridge.startEmulator` in `MainActivity.kt` picks the first one really
installed and sends the Intent; `launch()` in `ui/js/actions.js` is the seam.
Verified on the Thor across the library: 24 of 32 platforms resolve to a core,
and the other 8 are platforms `data/esde-core-map.json` has no entry for —
`easyrpg`, `g-and-w`, `new-nintendo-3ds`, `pico8`, `ps2`, `switch`, `wii`,
`wiiu` — which is a gap in the table, not in the launcher.

**The config half is done too, and the finding below about `--appendconfig` is
answered rather than worked around.** The Intent has no equivalent, but it does
not need one: `CONFIGFILE` takes a whole config, so the app folds its per-launch
fragment onto the user's own `retroarch.cfg` and hands over the result.
`retroarch::merge_config` does the fold — their keys keep their place and their
comments, ours replace theirs, anything of ours they have never had is appended.
Their file is only ever read, and `config_save_on_exit = "false"` stops
RetroArch writing back over even our copy.

Two things only Kotlin can answer, so it does: which RetroArch is installed
(its files live under its own package name) and where we may write something
RetroArch can read. The answer to the second is our *external* files directory —
`Android/data/<us>/files`. RetroArch reads it because it targets **SDK 28** and
is still on legacy storage; we read *its* directory because we hold
MANAGE_EXTERNAL_STORAGE. Both halves of that asymmetry are needed and both were
measured.

Hotkeys come from `padprofile::android()`. On Android the numbers are not
per-controller: the input driver takes raw `KeyEvent` codes and the OS fixes
them, so `BUTTON_A` is 96 everywhere. `RetroArch::pad_profile` switches on the
configured `input_driver` rather than a `cfg`, so the same code path serves
both. The d-pad is deliberately left unbound — Android reports it as a hat on
most pads and as keycodes on some, and a wrong index there fires hotkeys during
play.

Verified on the Thor: the merged file is the same 3,365 lines as the user's,
`video_driver = "vulkan"` and their own binds intact, our hotkeys and save
directories in place, RetroArch logging `[ENV] Config file: …/launch.cfg`, and
Select+Y opening its menu.

**Save sync and play time work too, in two halves.** There is no single moment
to hang them off — `startActivity` returns when the request is accepted, not when
the game ends — so the pull is `android_sync_before` at launch and the push is
`android_after_play` on the way back. What closes the loop is that this activity
regains focus when RetroArch stops: `reportGameFinished` measures the gap with
`elapsedRealtime` and calls `window.__gameFinished(id, seconds)`, once per launch
so a dialog or the recents switcher is not mistaken for a finished game.

**Settings that could not work on Android are gone from it.** Fit to the game,
Title bar and Open games on all fed `window_lines`, which the Android launch does
not generate — RetroArch takes the whole screen from an Intent, and there is no
window to shape and no second display to choose. The light gun column went for a
plainer reason: its binds are mouse buttons, and a handheld has no mouse. The App
icon row went because the launcher icon is baked into the APK.

**What is still not done.** No shader applies unless RetroArch has a shader pack;
`shaders_dir()` reads its `video_shader_dir`, so wherever the user keeps it is
found, but a device without one gets a note rather than a shader. Standalone
emulators are not started at all — this app's manifest can only see the two
RetroArch packages, so `getPackageInfo` answers "not installed" for every other
one whether it is or not. And core options are written but untested: no core on
this device has any set.

`retroarch.rs` is 1,950 lines and its job is to build a `Command`, spawn
RetroArch, and write the config that shapes the session. On Android none of
those three things happen that way.

Launching becomes an Intent:

    am start -n com.retroarch/.browser.retroactivity.RetroActivityFuture \
      -e ROM        <rom path> \
      -e LIBRETRO   <full path to the core .so> \
      -e CONFIGFILE <path to a retroarch.cfg>

All three are required, and since the 2025-01-17 nightly the **full** core path
is required rather than a bare core name. From Tauri that means a small Kotlin
mobile plugin — `develop/plugins/develop-mobile` — exposing one command the
webview can call.

**The finding that costs the most:** the app shapes every session with
`--appendconfig` and `--set-shader=`, and **the Intent interface has no
equivalent.** `--appendconfig` as an Android extra is an open request against
RetroArch (issue #12096), not a feature. So the per-game override mechanism —
core overrides, shader overrides, the motion/BFI pass, light-gun mode,
autofire, `fit_window`, save-state-on-exit — cannot be layered on top of a base
config the way it is on desktop.

What replaces it is merging: compose one complete `retroarch.cfg` per launch,
write it somewhere RetroArch can read, and pass it as `CONFIGFILE`. That is a
real rewrite of the config path and it should be scoped before anything else in
this file is started, because it decides how much of those 1,950 lines is
salvage and how much is a second implementation. `configpatch.rs` and
`tweaks.rs` are where to start reading.

**Do not start from ES-DE** — its Android port is not open source. Read
**Daijishō** and **Lemuroid**, both of which drive RetroArch by Intent from
Kotlin, which is exactly the piece being written.

## Step 5 — the two screens

This is already wanted: the small screen showing a list while the big one
stays a grid. Android exposes a second display and a webview can be put
on it.

Worth noting from the memory table above that two webviews cost meaningfully
more than one spanning both (255 vs 211 MB in list view) — still irrelevant on
this device, but it is the choice that decides whether the two screens share
one page's state or synchronise two.

## What to drop, and what not to

* **Light-gun and display-refresh work.** `macdisplay.rs` and `lightgun.rs` are
  desktop concerns. Gate them off rather than porting them.
* **The window-shaping options.** `fit_window` and `window_decorations` mean
  nothing on a handheld.
* **Do not drop the grid for memory.** List view genuinely costs less — it draws
  no artwork at all, and `observeCovers()` only runs in grid layout — but the
  difference is about 28 MB on the Thor's screen. That is not a reason to take
  the covers away from a device with two OLED panels. Keep it as a setting for
  genuinely tight hardware.

## Honest sizing

| | |
|---|---|
| toolchain and first APK | days |
| browsing on the device | ~1 week |
| storage decided and working | 1–2 weeks |
| launching by Intent, config path rewritten | **the rest of it** |

The old fortnight estimate was for the SDL road and covered
browsing only. Browsing is now cheaper than that and launching is dearer, and
launching was always the half that was a second project rather than a port.

**The decision that gates everything** is still this: if Android is the
handheld target, the RK3566 work in `handheld-device.md` and
`handheld-frontend.md` task 3 is replaced rather than joined. That is worth
settling before any of the above is started.
