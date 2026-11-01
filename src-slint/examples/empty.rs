//! The memory floor: a Slint window the prototype's size, with nothing in it.

slint::slint! {
    export component Empty inherits Window {
        preferred-width: 1280px;
        preferred-height: 800px;
        background: #14161a;
    }
}

fn main() -> Result<(), slint::PlatformError> {
    Empty::new()?.run()
}
