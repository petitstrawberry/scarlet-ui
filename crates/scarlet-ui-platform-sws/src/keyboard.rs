//! Keep physical keyboard shortcuts separate from committed text input.

use scarlet_ui_core::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) fn keyboard_events(
    keycode: KeyCode,
    mapped_char: Option<char>,
    modifiers: KeyModifiers,
    pressed: bool,
) -> [Option<KeyEvent>; 2] {
    let key = if pressed {
        KeyEvent::Pressed { keycode, modifiers }
    } else {
        KeyEvent::Released { keycode, modifiers }
    };
    // Char carries no modifiers, so emitting it for a shortcut would also
    // insert the shortcut's letter into the focused text control.
    let text = if pressed && !modifiers.control && !modifiers.super_key {
        mapped_char
            .filter(|c| !c.is_control())
            .map(|c| KeyEvent::Char { c })
    } else {
        None
    };
    [Some(key), text]
}
