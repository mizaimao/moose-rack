//! What the app is in the middle of that quitting would break.
//!
//! While a game runs, the app stays open, as Steam does: the saves go up when
//! the game ends, and an app that has quit uploads nothing. While saves are
//! uploading, quitting asks first. See `docs/server-and-client.md`.
//!
//! Counted with guards, so a launch that fails or panics halfway still says
//! it's done.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
pub struct Busy {
    /// Names of the games running now. One in practice; a list so two never
    /// confuse the count.
    games: Mutex<Vec<String>>,
    uploads: AtomicUsize,
    /// Set by "Quit anyway", so the exit it starts isn't stopped again.
    leaving: AtomicBool,
}

/// Why the app shouldn't quit right now.
#[derive(Debug, PartialEq, Eq)]
pub enum Hold {
    /// A game is running. Not up for discussion: quit the game first.
    Playing(String),
    /// Saves are uploading. Up to the user.
    Uploading,
}

impl Busy {
    /// Mark a game as running until the guard is dropped.
    pub fn playing(&self, name: &str) -> Playing<'_> {
        if let Ok(mut g) = self.games.lock() {
            g.push(name.to_owned());
        }
        Playing { busy: self, name: name.to_owned() }
    }

    /// Mark a save upload as running until the guard is dropped.
    pub fn uploading(&self) -> Uploading<'_> {
        self.uploads.fetch_add(1, Ordering::SeqCst);
        Uploading(self)
    }

    /// `None` when quitting is fine.
    pub fn hold(&self) -> Option<Hold> {
        if self.leaving.load(Ordering::SeqCst) {
            return None;
        }
        if let Some(name) = self.games.lock().ok().and_then(|g| g.first().cloned()) {
            return Some(Hold::Playing(name));
        }
        (self.uploads.load(Ordering::SeqCst) > 0).then_some(Hold::Uploading)
    }

    /// The user said quit anyway.
    pub fn leave(&self) {
        self.leaving.store(true, Ordering::SeqCst);
    }
}

pub struct Playing<'a> {
    busy: &'a Busy,
    name: String,
}

impl Drop for Playing<'_> {
    fn drop(&mut self) {
        if let Ok(mut g) = self.busy.games.lock()
            && let Some(i) = g.iter().position(|n| *n == self.name)
        {
            g.remove(i);
        }
    }
}

pub struct Uploading<'a>(&'a Busy);

impl Drop for Uploading<'_> {
    fn drop(&mut self) {
        self.0.uploads.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_may_quit() {
        assert_eq!(Busy::default().hold(), None);
    }

    #[test]
    fn a_running_game_holds_until_it_ends() {
        let busy = Busy::default();
        {
            let _g = busy.playing("Pac-Man");
            assert_eq!(busy.hold(), Some(Hold::Playing("Pac-Man".into())));
        }
        assert_eq!(busy.hold(), None);
    }

    #[test]
    fn a_game_outranks_an_upload() {
        let busy = Busy::default();
        let _u = busy.uploading();
        let _g = busy.playing("Halo 3");
        assert_eq!(busy.hold(), Some(Hold::Playing("Halo 3".into())));
    }

    #[test]
    fn uploads_hold_until_the_last_one_ends() {
        let busy = Busy::default();
        let a = busy.uploading();
        let b = busy.uploading();
        drop(a);
        assert_eq!(busy.hold(), Some(Hold::Uploading));
        drop(b);
        assert_eq!(busy.hold(), None);
    }

    #[test]
    fn a_launch_that_fails_still_lets_go() {
        let busy = Busy::default();
        let r: Result<(), &str> = (|| {
            let _g = busy.playing("Broken");
            Err("emulator missing")
        })();
        assert!(r.is_err());
        assert_eq!(busy.hold(), None);
    }

    #[test]
    fn quit_anyway_is_not_stopped_again() {
        let busy = Busy::default();
        let _u = busy.uploading();
        busy.leave();
        assert_eq!(busy.hold(), None);
    }

    #[test]
    fn the_same_game_twice_counts_twice() {
        let busy = Busy::default();
        let a = busy.playing("Tetris");
        let b = busy.playing("Tetris");
        drop(a);
        assert_eq!(busy.hold(), Some(Hold::Playing("Tetris".into())));
        drop(b);
        assert_eq!(busy.hold(), None);
    }
}
