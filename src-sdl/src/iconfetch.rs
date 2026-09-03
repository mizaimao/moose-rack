// Getting the console pictures onto the device.
//
// Until now there was no way. The icon-set table ships with the app and names
// nine sets, the settings screen let you pick one, and nothing on the handheld
// could ever put the files where the picker points — they arrived only if the
// desktop app had downloaded them into a media folder the handheld happened to
// share. So the setting offered nine choices of which eight drew a blank
// screen, and there was no wrong thing to fix: the work simply was not written.
//
// This is that work. Same table, same URLs, same folder layout as the desktop's
// `fetch_icon_set`, so a set fetched here and a set fetched there are the same
// files in the same places.
//
// On a thread, with a blocking client. There is no async runtime in this front
// end and a few hundred small files is not a reason to add one — the thread
// reports progress down a channel and the settings screen reads it whenever it
// draws.

use moose_rack::{coremap::CoreMap, iconart, theme};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

/// What the worker says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// `done` of `total` pictures tried.
    Working { done: usize, total: usize },
    /// Finished, with how many landed.
    Done { written: usize },
    Failed(String),
}

/// A download in flight.
pub struct Fetch {
    pub set: String,
    from: Receiver<Progress>,
    last: Progress,
}

impl Fetch {
    /// Start fetching one set into `media_root`.
    ///
    /// `slugs` are the consoles the library actually has. Fetching pictures for
    /// consoles nobody owns is the bulk of the transfer and none of the value —
    /// the table covers about ninety systems and a real library is two dozen.
    pub fn start(media_root: PathBuf, set: &str, slugs: Vec<String>, map: CoreMap) -> Fetch {
        let (tx, rx) = std::sync::mpsc::channel();
        let set_id = set.to_owned();
        let worker = set_id.clone();
        std::thread::spawn(move || {
            let outcome = run(&media_root, &worker, &slugs, &map, &tx);
            let _ = tx.send(match outcome {
                Ok(written) => Progress::Done { written },
                Err(e) => Progress::Failed(format!("{e:#}")),
            });
        });
        Fetch {
            set: set_id,
            from: rx,
            last: Progress::Working { done: 0, total: 0 },
        }
    }

    /// The newest thing the worker said, and whether it has stopped.
    ///
    /// Drains rather than reads one: the worker sends per picture and the
    /// screen draws sixty times a second, so most frames have several waiting
    /// and only the last one is worth anything.
    pub fn poll(&mut self) -> &Progress {
        loop {
            match self.from.try_recv() {
                Ok(p) => self.last = p,
                Err(TryRecvError::Empty) => break,
                // The thread is gone. If it had finished it would have said so,
                // so this is a panic in the worker rather than a quiet success.
                Err(TryRecvError::Disconnected) => {
                    if matches!(self.last, Progress::Working { .. }) {
                        self.last = Progress::Failed("the download stopped".to_owned());
                    }
                    break;
                }
            }
        }
        &self.last
    }

    pub fn finished(&self) -> bool {
        !matches!(self.last, Progress::Working { .. })
    }

    /// One line for the settings row.
    pub fn note(&self) -> String {
        match &self.last {
            Progress::Working { done, total } if *total > 0 => {
                format!("{done} of {total}…")
            }
            Progress::Working { .. } => "starting…".to_owned(),
            Progress::Done { written } => format!("{written} pictures"),
            Progress::Failed(why) => why.clone(),
        }
    }
}

fn run(
    media_root: &std::path::Path,
    set: &str,
    slugs: &[String],
    map: &CoreMap,
    say: &std::sync::mpsc::Sender<Progress>,
) -> anyhow::Result<usize> {
    let art = iconart::of(set).ok_or_else(|| anyhow::anyhow!("no artwork recorded for {set}"))?;
    moose_rack::util::install_tls();
    let http = reqwest::blocking::Client::builder()
        .user_agent(concat!("moose-rack/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    // Start from nothing, the same as the desktop does. A set fetched under an
    // older mapping leaves pictures in folders this one does not write, and
    // those go on being drawn as if they were current.
    let _ = theme::remove_set(media_root, set);

    let wanted = theme::esde_names_for(map, slugs);
    let total = art.looks.len() * wanted.len();
    let mut done = 0usize;
    let mut written = 0usize;

    for look in &art.looks {
        let out = theme::set_dir(media_root, set, &look.id);
        std::fs::create_dir_all(&out)?;
        for (slug, names) in &wanted {
            done += 1;
            // Every eighth: a message per picture is a wake-up per picture for
            // a number that changes too fast to read.
            if done.is_multiple_of(8) {
                let _ = say.send(Progress::Working { done, total });
            }
            // A theme files a console under whichever ES-DE name it knows, so
            // each candidate is tried rather than assuming our slug is it.
            for name in names {
                let Some(url) = art.url(&look.id, name) else {
                    continue;
                };
                let Ok(resp) = http.get(&url).send() else {
                    continue;
                };
                if !resp.status().is_success() {
                    continue;
                }
                let Ok(bytes) = resp.bytes() else { continue };
                if std::fs::write(out.join(format!("{slug}.{}", look.ext)), &bytes).is_ok() {
                    written += 1;
                    break;
                }
            }
        }
    }
    Ok(written)
}

/// A few sample pictures from one set, so it can be looked at before it is
/// downloaded.
///
/// The desktop's Icon sets tab shows every set with its own artwork, which is
/// the only honest way to choose between nine names like `pixel-art` and
/// `steel`. Same idea here: the highlighted set fetches half a dozen consoles
/// out of the library into a cache folder, and the picker draws them.
///
/// Cached on disk rather than in memory, and kept separate from the installed
/// sets so a preview is never mistaken for one — `installed_sets` counts what
/// is under `sets/`, and this writes under `previews/`.
pub struct Previews {
    pub set: String,
    from: Receiver<PathBuf>,
    pub found: Vec<PathBuf>,
}

/// Where sample pictures live. Not under `sets/`, which is what counts as
/// installed.
pub fn preview_dir(media_root: &std::path::Path, set: &str) -> PathBuf {
    media_root.join("_platforms").join("previews").join(set)
}

impl Previews {
    /// How many consoles to show. Six is what the desktop shows, and it is
    /// about what fits beside a list on a 640-point panel.
    pub const WANT: usize = 6;

    pub fn start(media_root: PathBuf, set: &str, slugs: Vec<String>, map: CoreMap) -> Previews {
        let (tx, rx) = std::sync::mpsc::channel();
        let id = set.to_owned();
        let worker = id.clone();
        std::thread::spawn(move || {
            let Some(art) = iconart::of(&worker) else { return };
            let Some(look) = art.best_look().map(|l| l.id.clone()) else {
                return;
            };
            let ext = art
                .look(&look)
                .map(|l| l.ext.clone())
                .unwrap_or_else(|| "png".to_owned());
            let dir = preview_dir(&media_root, &worker);
            if std::fs::create_dir_all(&dir).is_err() {
                return;
            }
            moose_rack::util::install_tls();
            let Ok(http) = reqwest::blocking::Client::builder()
                .user_agent(concat!("moose-rack/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(20))
                .build()
            else {
                return;
            };
            for system in theme::preview_systems(&map, &slugs, Self::WANT) {
                let at = dir.join(format!("{system}.{ext}"));
                // Already fetched: send it and move on. A preview is a picture
                // of a repository that changes about never.
                if at.is_file() {
                    let _ = tx.send(at);
                    continue;
                }
                let Some(url) = art.url(&look, &system) else {
                    continue;
                };
                let Ok(resp) = http.get(&url).send() else { continue };
                if !resp.status().is_success() {
                    continue;
                }
                let Ok(bytes) = resp.bytes() else { continue };
                if std::fs::write(&at, &bytes).is_ok() && tx.send(at).is_err() {
                    // Nobody is listening: the cursor moved to another set.
                    return;
                }
            }
        });
        Previews {
            set: id,
            from: rx,
            found: Vec::new(),
        }
    }

    /// Whatever has landed since the last look.
    pub fn poll(&mut self) -> &[PathBuf] {
        while let Ok(path) = self.from.try_recv() {
            if !self.found.contains(&path) {
                self.found.push(path);
            }
        }
        &self.found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line under the row says what is happening, in all four states.
    #[test]
    fn it_says_where_it_has_got_to() {
        let mut f = Fetch {
            set: "x".to_owned(),
            from: std::sync::mpsc::channel().1,
            last: Progress::Working { done: 0, total: 0 },
        };
        assert_eq!(f.note(), "starting…");
        f.last = Progress::Working {
            done: 40,
            total: 96,
        };
        assert_eq!(f.note(), "40 of 96…");
        assert!(!f.finished());
        f.last = Progress::Done { written: 88 };
        assert_eq!(f.note(), "88 pictures");
        assert!(f.finished(), "a finished download still reads as running");
        f.last = Progress::Failed("no network".to_owned());
        assert!(f.finished());
    }

    /// A worker that dies without saying so is a failure, not a silent
    /// success — otherwise the row sits at "starting…" forever.
    #[test]
    fn a_worker_that_vanishes_is_a_failure() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx);
        let mut f = Fetch {
            set: "x".to_owned(),
            from: rx,
            last: Progress::Working { done: 0, total: 0 },
        };
        assert!(matches!(f.poll(), Progress::Failed(_)));
    }
}
