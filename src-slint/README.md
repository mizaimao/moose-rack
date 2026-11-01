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

The data is a sample of real titles with made-up sizes and stand-in covers.
Nothing here calls `moose_rack` yet; sorting and filtering are a few lines in
`src/main.rs` where the real port would use the core.

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
