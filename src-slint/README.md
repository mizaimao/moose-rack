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

## Running it

Build on macserver, as the main app is, then run the binary:

```sh
./scripts/fetch-fonts.sh            # Noto Sans, which the UI embeds
cd src-slint && cargo build --release
./target/release/moose-slint
```

Keys: arrows move, Return opens (Sofa) or plays (Desk), Esc goes back, `/`
searches, Q and E change tab, R picks at random. The mouse works throughout.

It is its own Cargo workspace, so it adds nothing to the main lockfile or CI.
