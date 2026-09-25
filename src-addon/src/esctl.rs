//! Keeping EmulationStation out of the way while its files are rewritten.
//!
//! ES holds every gamelist and its collections in memory and writes them back
//! when it exits (docs/flip-knulli-changes.md). A star written into
//! `gamelist.xml` while ES runs is overwritten by ES's own copy the next time
//! it stops, and the sync that wrote it has already recorded agreement.
//!
//! So a run over ssh stops ES first, cleanly, which also puts anything
//! starred on screen since ES started onto the card where the sync can read
//! it. Then it does its work and starts ES again, the way the launcher does.
//! Never while a game is running: stopping ES under a game is not ours to do.
//!
//! The on-screen app does not come through here. When L2+R2 opens it, the
//! launcher has already stopped ES. When ES opens it as a Port, ES is still
//! running and waiting on it, and cannot be stopped from underneath; see
//! [`Frontend::es_running`] in `worker::stars_apply`.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};

/// What a run needs to know about, and do to, the frontend.
pub trait Frontend {
    fn es_running(&self) -> bool;
    fn game_running(&self) -> bool;
    /// Stop ES and wait until it has gone.
    fn stop(&self) -> Result<()>;
    fn start(&self) -> Result<()>;
}

/// Run `work` with ES stopped, and start it again afterwards if it was
/// running before, whether `work` succeeded or not.
///
/// Refuses when a game is running.
pub fn with_es_stopped<T>(
    fe: &dyn Frontend,
    say: &dyn Fn(&str),
    work: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if fe.game_running() {
        bail!("a game is running; close it first, then run this again");
    }
    let stopped = fe.es_running();
    if stopped {
        say("stopping EmulationStation, so it cannot write its own copy over this");
        fe.stop()?;
    }
    let out = work();
    if stopped {
        say("starting EmulationStation again");
        if let Err(e) = fe.start() {
            return Err(match out {
                Ok(_) => e.context("the work was done, but EmulationStation did not start again"),
                Err(w) => w.context(format!("and EmulationStation did not start again: {e:#}")),
            });
        }
    }
    out
}

/// The real device: processes read out of `/proc`, ES driven through the
/// same commands `scripts/moose-launch.sh` uses.
pub struct Knulli {
    pub proc: PathBuf,
}

impl Default for Knulli {
    fn default() -> Self {
        Self { proc: PathBuf::from("/proc") }
    }
}

impl Knulli {
    /// Every process's arguments. Empty where there is no `/proc`, which is a
    /// desktop, where nothing here is running.
    fn commands(&self) -> Vec<Vec<String>> {
        let Ok(rd) = std::fs::read_dir(&self.proc) else { return Vec::new() };
        rd.flatten()
            .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
            .filter_map(|e| std::fs::read(e.path().join("cmdline")).ok())
            .map(|raw| {
                raw.split(|b| *b == 0)
                    .filter(|a| !a.is_empty())
                    .map(|a| String::from_utf8_lossy(a).into_owned())
                    .collect::<Vec<_>>()
            })
            .filter(|args| !args.is_empty())
            .collect()
    }
}

fn program(args: &[String]) -> &str {
    args[0].rsplit('/').next().unwrap_or("")
}

impl Frontend for Knulli {
    /// The ES binary itself, not `emulationstation-standalone`, the shell
    /// loop that restarts it. The launcher's `ps | grep '^emulationstation '`
    /// is the same test.
    fn es_running(&self) -> bool {
        self.commands().iter().any(|a| program(a) == "emulationstation")
    }

    /// RetroArch, or anything `emulatorlauncher` started: the standalone
    /// emulators all go through it. The same test the launcher makes before
    /// it takes the screen.
    fn game_running(&self) -> bool {
        self.commands()
            .iter()
            .any(|a| program(a) == "retroarch" || a.iter().any(|x| x.contains("emulatorlauncher")))
    }

    fn stop(&self) -> Result<()> {
        let status = std::process::Command::new("/etc/init.d/S31emulationstation")
            .arg("stop")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if let Err(e) = status {
            bail!("could not run /etc/init.d/S31emulationstation stop: {e}");
        }
        // Twenty seconds, as the launcher waits. ES writes every gamelist on
        // the way out, and on an exFAT card that is not instant.
        for _ in 0..40 {
            if !self.es_running() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        bail!("EmulationStation was still running 20 s after being asked to stop; nothing was written")
    }

    /// `setsid`, and the two profile scripts, or ES comes up as a child of
    /// this ssh session with no `XDG_RUNTIME_DIR`: no sound, and killed by
    /// SIGHUP when the session closes. Never a second one: two fight over
    /// the display and neither survives.
    fn start(&self) -> Result<()> {
        if self.es_running() {
            return Ok(());
        }
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(
                ". /etc/profile.d/xdg.sh 2>/dev/null; . /etc/profile.d/dbus.sh 2>/dev/null; \
                 setsid /usr/bin/emulationstation-standalone </dev/null >/dev/null 2>&1 &",
            )
            .status()?;
        if !status.success() {
            bail!("starting EmulationStation exited {status}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        es: bool,
        game: bool,
        start_fails: bool,
        log: RefCell<Vec<&'static str>>,
    }

    impl Frontend for Fake {
        fn es_running(&self) -> bool {
            self.es
        }
        fn game_running(&self) -> bool {
            self.game
        }
        fn stop(&self) -> Result<()> {
            self.log.borrow_mut().push("stop");
            Ok(())
        }
        fn start(&self) -> Result<()> {
            self.log.borrow_mut().push("start");
            if self.start_fails { bail!("no display") } else { Ok(()) }
        }
    }

    fn run(fe: &Fake, ok: bool) -> Result<()> {
        with_es_stopped(fe, &|_| {}, || {
            fe.log.borrow_mut().push("work");
            if ok { Ok(()) } else { bail!("server said no") }
        })
    }

    /// ES writes its gamelists back from memory when it exits, so the work
    /// happens with it stopped, and it comes back afterwards.
    #[test]
    fn es_is_stopped_for_the_work_and_started_after() {
        let fe = Fake { es: true, ..Default::default() };
        run(&fe, true).unwrap();
        assert_eq!(*fe.log.borrow(), ["stop", "work", "start"]);
    }

    /// Even when the work failed: a failed sync must not leave the handheld
    /// on a black screen.
    #[test]
    fn es_comes_back_after_a_failure_too() {
        let fe = Fake { es: true, ..Default::default() };
        assert!(run(&fe, false).is_err());
        assert_eq!(*fe.log.borrow(), ["stop", "work", "start"]);
    }

    #[test]
    fn es_that_was_not_running_is_not_started() {
        let fe = Fake::default();
        run(&fe, true).unwrap();
        assert_eq!(*fe.log.borrow(), ["work"]);
    }

    #[test]
    fn nothing_happens_while_a_game_is_running() {
        let fe = Fake { es: true, game: true, ..Default::default() };
        let e = run(&fe, true).unwrap_err().to_string();
        assert!(e.contains("game is running"), "{e}");
        assert!(fe.log.borrow().is_empty(), "{:?}", fe.log.borrow());
    }

    #[test]
    fn es_failing_to_start_is_reported_even_when_the_work_went_well() {
        let fe = Fake { es: true, start_fails: true, ..Default::default() };
        let e = format!("{:#}", run(&fe, true).unwrap_err());
        assert!(e.contains("did not start again"), "{e}");
    }

    fn fake_proc(name: &str, procs: &[&[&str]]) -> Knulli {
        let dir = std::env::temp_dir().join(format!("moose-esctl-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        for (i, args) in procs.iter().enumerate() {
            let p = dir.join((100 + i).to_string());
            std::fs::create_dir_all(&p).unwrap();
            let mut raw = args.join("\0").into_bytes();
            raw.push(0);
            std::fs::write(p.join("cmdline"), raw).unwrap();
        }
        std::fs::create_dir_all(dir.join("self")).unwrap();
        Knulli { proc: dir }
    }

    #[test]
    fn es_is_the_binary_and_not_its_restart_loop() {
        let only_the_loop = fake_proc("loop", &[&["/bin/bash", "/usr/bin/emulationstation-standalone"]]);
        assert!(!only_the_loop.es_running());
        let running = fake_proc(
            "es",
            &[&["/bin/bash", "/usr/bin/emulationstation-standalone"], &["emulationstation", "--no-splash"]],
        );
        assert!(running.es_running());
        assert!(!running.game_running());
    }

    #[test]
    fn a_game_is_retroarch_or_anything_the_launcher_started() {
        assert!(fake_proc("ra", &[&["/usr/bin/retroarch", "-L", "core.so"]]).game_running());
        assert!(
            fake_proc("launcher", &[&["python", "/usr/bin/emulatorlauncher", "-system", "psp"]]).game_running()
        );
        assert!(!fake_proc("idle", &[&["/sbin/init"], &["sshd"]]).game_running());
        // No /proc at all is a desktop, where none of this runs.
        let none = Knulli { proc: PathBuf::from("/nonexistent-proc") };
        assert!(!none.es_running() && !none.game_running());
    }
}
