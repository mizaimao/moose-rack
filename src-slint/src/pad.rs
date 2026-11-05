//! Controllers, read through SDL3 and handed over in the numbering of the W3C
//! standard mapping, which is what `moose_rack::binds` is written against.
//!
//! Both number the face buttons by position, not by the letter printed on
//! them: 0 is the bottom one on every pad, which is A on Xbox, Cross on
//! PlayStation and B on Nintendo. So nothing is swapped here; what the letters
//! say is only for drawing the hints, and SDL3 knows that per controller.

use std::collections::BTreeMap;

use sdl3::event::Event;
use sdl3::gamepad::{Axis, Button, ButtonLabel, Gamepad, GamepadType};
use sdl3::joystick::JoystickId;

/// A controller arriving or leaving, for the window to report.
pub enum Change {
    Connected(Info),
    Disconnected(String),
}

/// What the window says about a controller.
#[derive(Clone)]
pub struct Info {
    pub name: String,
    /// "PS5", "Xbox One", "Switch Pro"... or "Other" when SDL cannot tell.
    pub kind: &'static str,
    /// What is printed on the bottom, right, left and top face buttons.
    pub faces: [&'static str; 4],
    /// The shoulder buttons and Start, as this maker names them.
    pub shoulders: [&'static str; 2],
    pub start: &'static str,
}

/// The state of every connected pad, merged, in standard-mapping order.
pub struct Frame {
    pub buttons: [bool; 17],
    pub axes: [f64; 4],
}

pub struct Pads {
    _sdl: sdl3::Sdl,
    subsystem: sdl3::GamepadSubsystem,
    pump: sdl3::EventPump,
    open: BTreeMap<JoystickId, Gamepad>,
}

impl Pads {
    pub fn new() -> Result<Self, String> {
        // There is no SDL window: the Slint one has the focus, so without this
        // SDL would count every press as arriving in the background.
        sdl3::hint::set("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1");
        // SDL otherwise takes Ctrl+C and kill for itself and turns them into a
        // quit event nobody here reads, so the window could not be stopped.
        sdl3::hint::set("SDL_NO_SIGNAL_HANDLERS", "1");
        let sdl = sdl3::init().map_err(|e| e.to_string())?;
        let subsystem = sdl.gamepad().map_err(|e| e.to_string())?;
        let pump = sdl.event_pump().map_err(|e| e.to_string())?;
        // The menu sounds play through the same SDL.
        crate::sound::start(&sdl);
        Ok(Self { _sdl: sdl, subsystem, pump, open: BTreeMap::new() })
    }

    /// Controllers that came or went since the last call.
    pub fn changes(&mut self) -> Vec<Change> {
        let mut changes = Vec::new();
        for event in self.pump.poll_iter() {
            match event {
                Event::GamepadAdded { which, .. } => {
                    if let Ok(pad) = self.subsystem.open(which) {
                        changes.push(Change::Connected(describe(&pad)));
                        self.open.insert(which, pad);
                    }
                }
                Event::GamepadRemoved { which, .. } => {
                    if let Some(pad) = self.open.remove(&which) {
                        changes.push(Change::Disconnected(pad.name().unwrap_or_default()));
                    }
                }
                _ => {}
            }
        }
        changes
    }

    /// Every pad's buttons and left stick, merged, so whichever is in hand
    /// works without choosing one.
    pub fn frame(&self) -> Frame {
        let mut frame = Frame { buttons: [false; 17], axes: [0.0; 4] };
        for pad in self.open.values() {
            for (index, button) in BUTTONS.iter().enumerate() {
                if let Some(button) = button {
                    frame.buttons[index] |= pad.button(*button);
                }
            }
            // The triggers are axes in SDL and buttons 6 and 7 in the
            // standard mapping.
            frame.buttons[6] |= pad.axis(Axis::TriggerLeft) > i16::MAX / 2;
            frame.buttons[7] |= pad.axis(Axis::TriggerRight) > i16::MAX / 2;
            for (i, axis) in [Axis::LeftX, Axis::LeftY, Axis::RightX, Axis::RightY].into_iter().enumerate() {
                let value = pad.axis(axis) as f64 / i16::MAX as f64;
                if value.abs() > frame.axes[i].abs() {
                    frame.axes[i] = value;
                }
            }
        }
        frame
    }
}

/// SDL's buttons in standard-mapping order. 6 and 7 are the triggers, read as
/// axes in `frame`.
const BUTTONS: [Option<Button>; 17] = [
    Some(Button::South),
    Some(Button::East),
    Some(Button::West),
    Some(Button::North),
    Some(Button::LeftShoulder),
    Some(Button::RightShoulder),
    None,
    None,
    Some(Button::Back),
    Some(Button::Start),
    Some(Button::LeftStick),
    Some(Button::RightStick),
    Some(Button::DPadUp),
    Some(Button::DPadDown),
    Some(Button::DPadLeft),
    Some(Button::DPadRight),
    Some(Button::Guide),
];

fn describe(pad: &Gamepad) -> Info {
    let kind = match pad.r#type() {
        GamepadType::Xbox360 => "Xbox 360",
        GamepadType::XboxOne => "Xbox One",
        GamepadType::PS3 => "PS3",
        GamepadType::PS4 => "PS4",
        GamepadType::PS5 => "PS5",
        GamepadType::NintendoSwitchPro => "Switch Pro",
        GamepadType::NintendoSwitchJoyconLeft | GamepadType::NintendoSwitchJoyconRight => "Joy-Con",
        GamepadType::NintendoSwitchJoyconPair => "Joy-Con pair",
        GamepadType::Standard | GamepadType::Unknown => "Other",
    };
    let label = |button| match pad.button_label_for_gamepad_type(button) {
        ButtonLabel::A => "A",
        ButtonLabel::B => "B",
        ButtonLabel::X => "X",
        ButtonLabel::Y => "Y",
        ButtonLabel::Cross => "cross",
        ButtonLabel::Circle => "circle",
        ButtonLabel::Square => "square",
        ButtonLabel::Triangle => "triangle",
        ButtonLabel::Unknown => "?",
    };
    let faces = [label(Button::South), label(Button::East), label(Button::West), label(Button::North)];
    let (shoulders, start) = match kind {
        "PS3" | "PS4" | "PS5" => (["L1", "R1"], "Options"),
        "Switch Pro" | "Joy-Con" | "Joy-Con pair" => (["L", "R"], "+"),
        _ => (["LB", "RB"], "Menu"),
    };
    Info { name: pad.name().unwrap_or_else(|| "Controller".into()), kind, faces, shoulders, start }
}
