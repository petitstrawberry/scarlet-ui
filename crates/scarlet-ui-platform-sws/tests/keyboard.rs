#[path = "../src/keyboard.rs"]
mod keyboard;

use scarlet_ui_core::event::{KeyCode, KeyEvent, KeyModifiers};

#[test]
fn control_shortcuts_keep_letter_identity_without_text() {
    let modifiers = KeyModifiers {
        control: true,
        ..Default::default()
    };
    for (letter, control) in [
        ('a', '\u{1}'),
        ('s', '\u{13}'),
        ('f', '\u{6}'),
        ('c', '\u{3}'),
    ] {
        for pressed in [true, false] {
            let events =
                keyboard::keyboard_events(KeyCode::Char(letter), Some(control), modifiers, pressed);
            let expected = if pressed {
                KeyEvent::Pressed {
                    keycode: KeyCode::Char(letter),
                    modifiers,
                }
            } else {
                KeyEvent::Released {
                    keycode: KeyCode::Char(letter),
                    modifiers,
                }
            };
            assert_eq!(events, [Some(expected), None]);
        }
    }
}

#[test]
fn shifted_text_preserves_physical_key_and_committed_character() {
    let modifiers = KeyModifiers {
        shift: true,
        ..Default::default()
    };
    assert_eq!(
        keyboard::keyboard_events(KeyCode::Char('a'), Some('A'), modifiers, true),
        [
            Some(KeyEvent::Pressed {
                keycode: KeyCode::Char('a'),
                modifiers
            }),
            Some(KeyEvent::Char { c: 'A' }),
        ]
    );
    assert_eq!(
        keyboard::keyboard_events(KeyCode::Char('a'), Some('A'), modifiers, false)[1],
        None
    );
}

#[test]
fn super_shortcuts_do_not_insert_letters() {
    let modifiers = KeyModifiers {
        super_key: true,
        ..Default::default()
    };
    assert_eq!(
        keyboard::keyboard_events(KeyCode::Char('s'), Some('s'), modifiers, true)[1],
        None
    );
}

#[test]
fn editing_keys_are_not_replaced_by_control_text() {
    for (keycode, text) in [
        (KeyCode::Enter, '\n'),
        (KeyCode::Tab, '\t'),
        (KeyCode::Backspace, '\u{8}'),
    ] {
        assert_eq!(
            keyboard::keyboard_events(keycode, Some(text), KeyModifiers::default(), true),
            [
                Some(KeyEvent::Pressed {
                    keycode,
                    modifiers: KeyModifiers::default()
                }),
                None,
            ]
        );
    }
}
