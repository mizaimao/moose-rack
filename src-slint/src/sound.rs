//! Menu sounds (rule 9 in docs/controller-ux.md): a short tick when the cursor
//! moves, a rising pair of notes on Confirm, a falling one on Back. They are
//! made here as a few milliseconds of sine wave rather than read from files,
//! so there is nothing to ship or license, and played through the SDL3 that
//! already reads the controllers. Settings' "Menu sounds" turns them off.
//!
//! Android has no SDL here; there the sounds are silent for now.

/// Which sound a press makes.
#[derive(Clone, Copy)]
pub enum Cue {
    Move,
    Confirm,
    Back,
}

#[cfg(not(target_os = "android"))]
mod imp {
    use std::cell::RefCell;

    use super::Cue;
    use sdl3::audio::{AudioFormat, AudioSpec, AudioStreamOwner};

    const RATE: i32 = 48_000;
    /// Loud enough to hear over a fan, quiet beside a game.
    const GAIN: f32 = 0.18;

    pub struct Player {
        stream: AudioStreamOwner,
        cues: [Vec<f32>; 3],
    }

    thread_local! {
        static PLAYER: RefCell<Option<Player>> = const { RefCell::new(None) };
    }

    /// One note: a sine at `hz` for `ms`, with a fast attack and a decay so it
    /// clicks neither in nor out.
    fn note(hz: f32, ms: f32) -> Vec<f32> {
        let n = (RATE as f32 * ms / 1000.0) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let attack = (i as f32 / (RATE as f32 * 0.002)).min(1.0);
                let decay = 1.0 - i as f32 / n as f32;
                (t * hz * std::f32::consts::TAU).sin() * attack * decay * decay * GAIN
            })
            .collect()
    }

    pub fn start(sdl: &sdl3::Sdl) {
        let opened = sdl.audio().map_err(|e| e.to_string()).and_then(|audio| {
            let spec = AudioSpec { freq: Some(RATE), channels: Some(1), format: Some(AudioFormat::F32LE) };
            let stream = audio.default_playback_device().open_device_stream(Some(&spec)).map_err(|e| e.to_string())?;
            stream.resume().map_err(|e| e.to_string())?;
            Ok(stream)
        });
        match opened {
            Ok(stream) => {
                let cues = [
                    note(1320.0, 22.0),
                    [note(880.0, 35.0), note(1320.0, 45.0)].concat(),
                    [note(990.0, 35.0), note(660.0, 45.0)].concat(),
                ];
                PLAYER.with(|p| *p.borrow_mut() = Some(Player { stream, cues }));
            }
            Err(e) => eprintln!("menu sounds: {e}"),
        }
    }

    pub fn play(cue: Cue) {
        PLAYER.with(|p| {
            if let Some(p) = p.borrow().as_ref() {
                // A new sound cuts off the last one rather than queueing behind
                // it, so a held direction ticks in time with the cursor.
                let _ = p.stream.clear();
                let _ = p.stream.put_data_f32(&p.cues[cue as usize]);
            }
        });
    }
}

#[cfg(not(target_os = "android"))]
pub use imp::start;

/// Play a cue, if sounds could be opened.
pub fn play(cue: Cue) {
    #[cfg(not(target_os = "android"))]
    imp::play(cue);
    #[cfg(target_os = "android")]
    let _ = cue;
}

