// Stamps the build with the git commit it came from, for launch records: the
// "code version" of a launch is this, and a build made from uncommitted
// changes says so with `+dirty`. See docs/server-and-client.md, "Launch
// records".

fn git(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn main() {
    let commit = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    println!("cargo:rustc-env=MOOSE_COMMIT={commit}{}", if dirty { "+dirty" } else { "" });
    // A new commit, a branch switch, or a change staged or unstaged.
    for path in [".git/HEAD", ".git/index", ".git/refs/heads", ".git/packed-refs"] {
        println!("cargo:rerun-if-changed={path}");
    }
}
