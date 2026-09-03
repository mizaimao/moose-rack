// The desktop entry point, and nothing else.
//
// Everything is in `lib.rs`, because Android does not start at `main`: the APK
// is Java that loads a `.so` and calls a symbol in it. A crate with only a
// `[[bin]]` target produces no `.so` at all, which is what `cargo tauri android
// build` fails on — "no library targets found in package `moose-gui`".
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    moose_gui_lib::run()
}
