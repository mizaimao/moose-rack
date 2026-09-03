// The parts of the front end that are worth testing without a window around
// them — and, for `gfx` and `backdrop`, with a real one.
//
// The binary keeps the loop and the layout; everything it draws through lives
// here so `tests/rendering.rs` can put a hidden window in front of it and read
// the pixels back.

pub mod backdrop;
pub mod covers;
pub mod gfx;
pub mod glass;
pub mod input;
pub mod keyboard;
pub mod library;
pub mod settings;
pub mod ports;
pub mod rescan;
pub mod status;
pub mod iconfetch;
pub mod sysinfo;
pub mod wifi;
pub mod text;
