// Building the library from what is on the card.
//
// On the Mac there was always a cache: the desktop app had synced one from the
// RomM server long before the SDL front end existed, and every version of this
// front end has opened a file somebody else filled. The handheld arrives with
// nothing. It has 235 GB of ROMs on it and a `cache.sqlite3` with three rows,
// so the Library tab showed Ports, Tools and Emulators and no consoles at all —
// which is not a bug in the front end, it is a step nobody had written.
//
// This is that step, and it needs no server and no network: `esde::scan` walks
// the ROM directories the device already has, and the core map says which
// console each directory is. That is the same work `moose-rack scan` does at
// a terminal, moved to where there is no terminal.
//
// On a thread, because it is thousands of directory reads and the screen
// carries on drawing while it happens.

use moose_rack::{cache::Cache, coremap::CoreMap, esde};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

/// What the scan says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Working,
    /// Games found, and the systems that were skipped for having no mapping.
    Done { games: usize, skipped: Vec<String> },
    Failed(String),
}

pub struct Rescan {
    from: Receiver<Progress>,
    last: Progress,
}

impl Rescan {
    /// Walk the card and write what is found into the cache.
    ///
    /// The worker opens its own connection rather than borrowing the one the
    /// screen is reading through — SQLite will take two, and passing the live
    /// one across would mean the library could not be drawn while the scan ran.
    pub fn start(cache_path: PathBuf) -> Rescan {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // Loaded here rather than handed in: it is a JSON file read, it is
            // wanted on this thread, and taking it as an argument would mean
            // the caller has to have built one before it knows whether a scan
            // is needed at all.
            let map = CoreMap::load_or_embedded(std::path::Path::new(
                "data/esde-core-map.json",
            ));
            let _ = tx.send(match run(&cache_path, &map) {
                Ok((games, skipped)) => Progress::Done { games, skipped },
                Err(e) => Progress::Failed(format!("{e:#}")),
            });
        });
        Rescan {
            from: rx,
            last: Progress::Working,
        }
    }

    pub fn poll(&mut self) -> &Progress {
        match self.from.try_recv() {
            Ok(p) => self.last = p,
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                if self.last == Progress::Working {
                    self.last = Progress::Failed("the scan stopped".to_owned());
                }
            }
        }
        &self.last
    }

    pub fn finished(&self) -> bool {
        self.last != Progress::Working
    }

    /// One line for the screen.
    pub fn note(&self) -> String {
        match &self.last {
            Progress::Working => "Looking at what is on the card…".to_owned(),
            Progress::Done { games, .. } => match games {
                0 => "Nothing found on the card.".to_owned(),
                1 => "Found one game.".to_owned(),
                n => format!("Found {n} games."),
            },
            Progress::Failed(why) => why.clone(),
        }
    }
}

fn run(cache_path: &std::path::Path, map: &CoreMap) -> anyhow::Result<(usize, Vec<String>)> {
    // Where the device keeps its library. A handheld running a known OS image
    // does not need asking — see `platform::default_library`.
    let layout = moose_rack::platform::current()
        .default_library()
        .ok_or_else(|| anyhow::anyhow!("this build does not know where the ROMs are"))?;
    let (games, skipped) = esde::scan(&layout, map)?;
    let found = games.len();
    if found == 0 {
        return Ok((0, skipped));
    }
    // Only now. `replace_from_esde` is a replace: a scan that found nothing
    // because the card was not mounted yet would empty a good library.
    let mut store = Cache::open(cache_path)?;
    store.replace_from_esde(&games)?;
    Ok((found, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line on screen says what is happening in every state.
    #[test]
    fn it_says_what_it_is_doing() {
        let mut r = Rescan {
            from: std::sync::mpsc::channel().1,
            last: Progress::Working,
        };
        assert!(r.note().contains("card"));
        assert!(!r.finished());
        r.last = Progress::Done {
            games: 2506,
            skipped: vec![],
        };
        assert_eq!(r.note(), "Found 2506 games.");
        assert!(r.finished());
        r.last = Progress::Done {
            games: 0,
            skipped: vec![],
        };
        assert!(r.note().contains("Nothing"));
    }

    /// A worker that dies without saying so is a failure, not a scan that never
    /// ends — otherwise the screen sits on "Looking…" forever.
    #[test]
    fn a_worker_that_vanishes_is_a_failure() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx);
        let mut r = Rescan {
            from: rx,
            last: Progress::Working,
        };
        assert!(matches!(r.poll(), Progress::Failed(_)));
    }
}
