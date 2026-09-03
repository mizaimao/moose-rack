//! Guards against the bug this project keeps making: a path separator written
//! literally into a test.
//!
//! Three times now. The `slangp` tests asserted on paths built the same way the
//! code built them, so they could never disagree with it on the host running
//! them, and Windows CI found real mixed separators the suite had declared fine.
//! Then a `savebackup` test asserted `contains("/7/")`, which is correct on
//! macOS and wrong on Windows, and blocked a build.
//!
//! Both share one root cause: `/` is invisible on the machine you write it on.
//! A human reviewing the diff sees a path; Windows sees a string that will never
//! match. So this is checked mechanically rather than remembered.
//!
//! Two rules, checked over the crate's own source:
//!
//! 1. Never turn a path into a string and then substring-match a separator.
//!    Compare components instead — `p.components().any(|c| c.as_os_str() == "7")`
//!    has no separator to get wrong.
//! 2. Never build an expected path by the same means the code under test used.
//!    A test that calls `join` to predict what `join` will produce agrees with
//!    the code on every host, including where both are wrong.
//!
//! Rule 2 cannot be checked mechanically, so it is written down here and the
//! `slangp` tests demonstrate the fix: a Windows-shaped literal directory as
//! input, which is a real check on Windows and a meaningful one everywhere.

use std::path::{Path, PathBuf};

/// Every `.rs` file in the crate, source and tests alike.
fn sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for sub in ["src", "src-tauri/src", "tests", "examples"] {
        walk(&root.join(sub), &mut out);
    }
    out
}

/// Does this string literal contain something that behaves as a path separator
/// on one platform and not the other?
fn has_separator(literal: &str) -> bool {
    literal.contains('/') || literal.contains("\\\\")
}

/// String literals on one line, roughly. Good enough to find `"..."` arguments;
/// this is a lint, not a parser.
fn literals(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    let mut current: Option<String> = None;
    while let Some(c) = chars.next() {
        match (&mut current, c) {
            (None, '"') => current = Some(String::new()),
            (Some(buf), '\\') => {
                buf.push('\\');
                if let Some(next) = chars.next() {
                    buf.push(next);
                }
            }
            (Some(_), '"') => out.push(current.take().unwrap()),
            (Some(buf), other) => buf.push(other),
            _ => {}
        }
    }
    out
}

/// Rule 1, checked mechanically.
///
/// The pattern is specific on purpose: converting a path to a string and then
/// substring-matching something containing a separator. That is exactly the
/// `savebackup` failure and it has no legitimate use — URLs and server paths
/// never go through `to_string_lossy` or `display`.
#[test]
fn no_path_is_stringified_and_matched_against_a_literal_separator() {
    const STRINGIFY: &[&str] = &["to_string_lossy()", "display()", "to_str()"];
    const MATCH: &[&str] = &[".contains(", ".starts_with(", ".ends_with(", ".find("];

    let mut offenders = Vec::new();
    for file in sources() {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        for (i, line) in text.lines().enumerate() {
            // An explicit opt-out, for the rare case where a separator really
            // is being asserted on purpose.
            if line.contains("separator-literal-ok") {
                continue;
            }
            if !STRINGIFY.iter().any(|s| line.contains(s)) {
                continue;
            }
            if !MATCH.iter().any(|m| line.contains(m)) {
                continue;
            }
            if literals(line).iter().any(|l| has_separator(l)) {
                offenders.push(format!(
                    "{}:{}\n    {}",
                    file.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(&file).display(),
                    i + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a path was turned into a string and matched against a literal separator.\n\
         That is correct on the machine you wrote it on and wrong on the other one —\n\
         it has blocked a Windows build before. Compare components instead:\n\
         \n    assert!(p.components().any(|c| c.as_os_str() == \"7\"));\n\n\
         If a separator really is intended, add `separator-literal-ok` to the line.\n\n{}",
        offenders.join("\n")
    );
}

/// The same rule for the UI, where `path.replace(\"/\", …)` and friends have the
/// same problem in reverse: JavaScript sees whatever the backend sent, and the
/// backend sends host paths.
#[test]
fn the_ui_does_not_split_backend_paths_on_a_separator() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/js");
    let mut offenders = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for e in entries.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "js")) {
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        for (i, line) in text.lines().enumerate() {
            if line.contains("separator-literal-ok") || line.trim_start().starts_with("//") {
                continue;
            }
            // Splitting or trimming a path on "/" assumes a POSIX host.
            let suspicious = (line.contains(".split(\"/\")") || line.contains(".split('/')"))
                && (line.contains("path") || line.contains("Path") || line.contains("dir"));
            if suspicious {
                offenders.push(format!("ui/js/{}:{}\n    {}",
                    e.file_name().to_string_lossy(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a backend path is being split on \"/\" in the UI, which is wrong on Windows:\n\n{}",
        offenders.join("\n")
    );
}

/// Rule 2, demonstrated rather than enforced: the layout of a generated path is
/// asserted by component, so the assertion means the same thing on every host.
///
/// If `savebackup` ever stops keying by rom id and slot, this fails on macOS,
/// Linux and Windows alike — which is the property the old `contains("/7/")`
/// version did not have.
#[test]
fn generated_paths_are_asserted_by_component_not_by_string() {
    let root = std::env::temp_dir().join("moose-rack-sep-guard");
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(&root).unwrap();
    let save = root.join("Zelda.srm");
    std::fs::write(&save, b"x").unwrap();

    let kept = moose_rack::savebackup::keep(&root, 7, "autosave", &save)
        .unwrap()
        .expect("a backup was taken");

    let parts: Vec<String> = kept
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    assert!(parts.iter().any(|p| p == "saves-backup"), "{parts:?}");
    assert!(parts.iter().any(|p| p == "7"), "keyed by rom id: {parts:?}");
    assert!(parts.iter().any(|p| p == "autosave"), "keyed by slot: {parts:?}");
    assert!(
        kept.file_name().unwrap().to_string_lossy().ends_with("-Zelda.srm"),
        "a file name is not a path, so matching it as a string is fine"
    );

    std::fs::remove_dir_all(&root).ok();
}

/// A save the server sends must land where the scanner looks, and the two ends
/// of that agreement are asserted by component so Windows cannot disagree.
#[test]
fn downloads_land_by_component_on_any_host() {
    let root = Path::new("/ra");
    let state = moose_rack::savesync::download_path(root, "Game.state1", moose_rack::savesync::Destination::Core { core: Some("snes9x") });
    let parts: Vec<String> = state
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    assert!(parts.iter().any(|p| p == "states"), "a state goes in states/: {parts:?}");
    assert!(parts.iter().any(|p| p == "snes9x"), "nested by core: {parts:?}");

    let save = moose_rack::savesync::download_path(root, "Game.srm", moose_rack::savesync::Destination::Core { core: Some("snes9x") });
    let parts: Vec<String> = save
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    assert!(parts.iter().any(|p| p == "saves"), "a game save goes in saves/: {parts:?}");
}

/// The lint has to actually fire, or it is decoration. Exercised against lines
/// that are not in the tree, so this cannot rot into always-passing.
#[test]
fn the_lint_catches_the_bug_it_was_written_for() {
    // The line that blocked the Windows build.
    let bad = r#"assert!(kept.to_string_lossy().contains("/7/"), "keyed by rom id");"#;
    assert!(literals(bad).iter().any(|l| has_separator(l)), "should find /7/");

    // A Windows-shaped one is just as wrong.
    let bad_win = r#"assert!(p.display().to_string().contains("\\Users\\"));"#;
    assert!(literals(bad_win).iter().any(|l| has_separator(l)));

    // The fixed form has no literal separator at all.
    let good = r#"assert!(kept.components().any(|c| c.as_os_str() == "7"));"#;
    assert!(!literals(good).iter().any(|l| has_separator(l)));

    // A file name is not a path; matching one as a string is fine.
    let fine = r#"assert!(p.file_name().unwrap().to_string_lossy().ends_with("-Zelda.srm"));"#;
    assert!(!literals(fine).iter().any(|l| has_separator(l)));
}
