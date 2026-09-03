fn main() {
    // The whole interface is compiled into the binary — `frontendDist` in
    // tauri.conf.json points at `../ui`, and the files are embedded rather than
    // read from disk at run time.
    //
    // Cargo does not know that. It watches Rust sources, so a change to
    // ui/style.css or ui/js/*.js leaves the crate looking up to date and the
    // build is skipped entirely: `tauri build` succeeds in seconds, the
    // packaging step copies the bundle it already had, and the result is a
    // successful build reporting a version of the app that is days old. That is
    // as quiet as a failure gets — nothing errors, the interface just does not
    // change, and the obvious conclusion is that the code is wrong.
    //
    // Naming the directory here puts it back under Cargo's watch. Cargo walks a
    // directory given to `rerun-if-changed` recursively, so this covers every
    // file the interface is made of, present and future.
    println!("cargo:rerun-if-changed=../ui");

    tauri_build::build()
}
