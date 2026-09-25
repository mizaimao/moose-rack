//! EmulationStation, as far as a change made over ssh has to care.
//!
//! ES reads knulli.conf once, keeps it in memory, and writes the whole file
//! back when its settings change. A patch written under a running ES lasts
//! until ES next saves, and then it is whatever ES remembered. So `--apply`
//! and `--restore` stop ES first, through its own init script, so anything it
//! was holding is written out before our change rather than after, and start
//! it again when they are done. The window does not need this: its launcher
//! has already stopped ES by the time it draws.
//!
//! A game is the other case, and there the answer is no. Stopping ES under a
//! running emulator would end the game.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// One process, as /proc describes it.
#[derive(Clone, Debug)]
pub struct Process {
    pub pid: u32,
    /// `comm`: the name the kernel keeps, cut to 15 bytes.
    pub name: String,
    /// `cmdline`, split at its NULs.
    pub args: Vec<String>,
}

impl Process {
    /// EmulationStation itself. Not `emulationstation-standalone`, the shell
    /// script that restarts it.
    pub fn is_es(&self) -> bool {
        self.args
            .first()
            .and_then(|arg| Path::new(arg).file_name())
            .is_some_and(|name| name == "emulationstation")
    }

    /// A game: RetroArch, or anything started through `emulatorlauncher`,
    /// which is how ES starts every emulator. moose-launch.sh asks the same
    /// before it takes the screen.
    pub fn is_game(&self) -> bool {
        self.name == "retroarch" || self.args.iter().any(|arg| arg.contains("emulatorlauncher"))
    }
}

/// Every process under `proc_root`, except this one. One that exits while
/// this reads is skipped.
pub fn processes(proc_root: &Path) -> Vec<Process> {
    let Ok(entries) = std::fs::read_dir(proc_root) else { return Vec::new() };
    let me = std::process::id();
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            if pid == me {
                return None;
            }
            let name = std::fs::read_to_string(entry.path().join("comm")).ok()?;
            let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
            let args = cmdline
                .split(|byte| *byte == 0)
                .filter(|arg| !arg.is_empty())
                .map(|arg| String::from_utf8_lossy(arg).into_owned())
                .collect();
            Some(Process { pid, name: name.trim_end().to_string(), args })
        })
        .collect()
}

/// What a change over ssh needs from the front end. A trait so the tests can
/// stand in for the device.
pub trait Frontend {
    /// A game that is running, described for a person, if there is one.
    fn game(&self) -> Option<String>;
    fn es_running(&self) -> bool;
    fn stop_es(&mut self) -> Result<()>;
    fn start_es(&mut self) -> Result<()>;
}

/// The handheld's own EmulationStation.
pub struct Device {
    pub proc_root: PathBuf,
    pub init: PathBuf,
    /// How long ES gets to stop, or to come back.
    pub wait: Duration,
}

impl Default for Device {
    fn default() -> Self {
        Device {
            proc_root: PathBuf::from("/proc"),
            init: PathBuf::from("/etc/init.d/S31emulationstation"),
            wait: Duration::from_secs(20),
        }
    }
}

impl Frontend for Device {
    fn game(&self) -> Option<String> {
        processes(&self.proc_root)
            .into_iter()
            .find(Process::is_game)
            .map(|p| format!("{} (pid {})", p.name, p.pid))
    }

    fn es_running(&self) -> bool {
        processes(&self.proc_root).iter().any(Process::is_es)
    }

    fn stop_es(&mut self) -> Result<()> {
        Command::new(&self.init)
            .arg("stop")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        let until = Instant::now() + self.wait;
        while self.es_running() {
            if Instant::now() > until {
                bail!(
                    "EmulationStation did not stop within {} s, so nothing was written",
                    self.wait.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Ok(())
    }

    /// Through the init script, but in a session of its own, or the ssh
    /// session ending takes ES down with it; and after the profile scripts, or
    /// it comes up with no XDG_RUNTIME_DIR and no sound. Both were found the
    /// hard way (docs/flip-knulli-changes.md).
    fn start_es(&mut self) -> Result<()> {
        let script = format!(
            ". /etc/profile.d/xdg.sh 2>/dev/null; . /etc/profile.d/dbus.sh 2>/dev/null; \
             exec {} start",
            self.init.display()
        );
        let mut child = Command::new("setsid")
            .args(["sh", "-c", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let until = Instant::now() + self.wait;
        while !self.es_running() {
            let _ = child.try_wait();
            if Instant::now() > until {
                bail!(
                    "EmulationStation did not come back within {} s; start it with `{} start`",
                    self.wait.as_secs(),
                    self.init.display()
                );
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let _ = child.try_wait();
        Ok(())
    }
}

/// Run `work` with EmulationStation out of the way.
///
/// Refuses while a game is running, before anything is written. Stops ES if
/// it is running, and starts it again afterwards whether `work` worked or
/// not. ES not coming back is an error too, after the change has been made:
/// the device is a black screen until somebody starts it.
pub fn with_es_stopped<T>(es: &mut dyn Frontend, work: impl FnOnce() -> Result<T>) -> Result<T> {
    if let Some(game) = es.game() {
        bail!(
            "a game is running ({game}). Stopping EmulationStation would end it; quit the game \
             first. Nothing was written"
        );
    }
    let was_running = es.es_running();
    if was_running {
        es.stop_es()?;
    }
    let done = work();
    let started = if was_running { es.start_es() } else { Ok(()) };
    match (done, started) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(e)) => Err(e.context("the change was made, but")),
        (Err(e), Ok(())) => Err(e),
        (Err(e), Err(start)) => {
            Err(e.context(format!("and EmulationStation did not come back: {start:#}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn fake_proc(name: &str, processes: &[(u32, &str, &[&str])]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-es-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        for (pid, comm, args) in processes {
            let at = dir.join(pid.to_string());
            std::fs::create_dir_all(&at).unwrap();
            std::fs::write(at.join("comm"), format!("{comm}\n")).unwrap();
            let mut cmdline = args.join("\0");
            cmdline.push('\0');
            std::fs::write(at.join("cmdline"), cmdline).unwrap();
        }
        // Not a process.
        std::fs::create_dir_all(dir.join("self")).unwrap();
        dir
    }

    fn device(proc_root: PathBuf) -> Device {
        Device { proc_root, ..Device::default() }
    }

    #[test]
    fn es_and_games_are_told_apart_from_proc() {
        let idle = fake_proc(
            "idle",
            &[
                (412, "emulationstatio", &["emulationstation", "--no-splash"]),
                (398, "emulationstatio", &["/bin/bash", "/usr/bin/emulationstation-standalone"]),
            ],
        );
        assert!(device(idle.clone()).es_running());
        assert_eq!(device(idle).game(), None, "ES on its own is not a game");

        let retroarch = fake_proc(
            "retroarch",
            &[(900, "retroarch", &["/usr/bin/retroarch", "-L", "mgba_libretro.so"])],
        );
        assert!(device(retroarch.clone()).game().is_some_and(|g| g.contains("retroarch")));
        assert!(!device(retroarch).es_running());

        let standalone = fake_proc(
            "standalone",
            &[(901, "python3", &["python3", "/usr/bin/emulatorlauncher", "-system", "psp"])],
        );
        assert!(device(standalone).game().is_some());
    }

    /// Stands in for ES and writes down what was done to it.
    struct Fake<'a> {
        running: bool,
        game: Option<&'static str>,
        log: &'a RefCell<Vec<&'static str>>,
        comes_back: bool,
    }

    impl Frontend for Fake<'_> {
        fn game(&self) -> Option<String> {
            self.game.map(str::to_string)
        }
        fn es_running(&self) -> bool {
            self.running
        }
        fn stop_es(&mut self) -> Result<()> {
            self.log.borrow_mut().push("stop");
            self.running = false;
            Ok(())
        }
        fn start_es(&mut self) -> Result<()> {
            self.log.borrow_mut().push("start");
            if !self.comes_back {
                bail!("it did not come back");
            }
            self.running = true;
            Ok(())
        }
    }

    #[test]
    fn es_is_stopped_around_the_work_and_started_even_when_it_fails() {
        let log = RefCell::new(Vec::new());
        let mut es = Fake { running: true, game: None, log: &log, comes_back: true };
        let result: Result<()> = with_es_stopped(&mut es, || {
            log.borrow_mut().push("write");
            bail!("the write failed")
        });
        assert!(result.is_err());
        assert_eq!(*log.borrow(), ["stop", "write", "start"]);
        assert!(es.running);
    }

    #[test]
    fn es_that_does_not_come_back_is_an_error_after_the_change() {
        let log = RefCell::new(Vec::new());
        let mut es = Fake { running: true, game: None, log: &log, comes_back: false };
        let err = with_es_stopped(&mut es, || Ok(())).unwrap_err();
        assert!(format!("{err:#}").contains("the change was made"), "{err:#}");
    }

    #[test]
    fn a_running_game_stops_everything_before_it_starts() {
        let log = RefCell::new(Vec::new());
        let mut es = Fake { running: true, game: Some("retroarch"), log: &log, comes_back: true };
        let result = with_es_stopped(&mut es, || {
            log.borrow_mut().push("write");
            Ok(())
        });
        assert!(result.is_err());
        assert!(log.borrow().is_empty(), "{:?}", log.borrow());
    }

    #[test]
    fn es_that_is_not_running_is_left_alone() {
        let log = RefCell::new(Vec::new());
        let mut es = Fake { running: false, game: None, log: &log, comes_back: true };
        with_es_stopped(&mut es, || {
            log.borrow_mut().push("write");
            Ok(())
        })
        .unwrap();
        assert_eq!(*log.borrow(), ["write"]);
    }
}
