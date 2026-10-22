use crate::element::{Element, LayoutConstraints};
use crate::event::{
    Event, FocusEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, Phase,
};
use crate::state::{State, generate_state_id};
use crate::view::View;
use crate::views::{Select, Slider, Toggle};
use alloc::{rc::Rc, string::String, vec};
use core::cell::Cell;

fn press(element: &mut dyn Element, keycode: KeyCode) -> bool {
    element.handle_event(
        &Event::Keyboard(KeyEvent::Pressed {
            keycode,
            modifiers: KeyModifiers::default(),
        }),
        Phase::Target,
    )
}
fn key(element: &mut dyn Element, keycode: KeyCode) -> bool {
    let handled = press(element, keycode);
    element.handle_event(&Event::Keyboard(KeyEvent::Released { keycode, modifiers: KeyModifiers::default() }), Phase::Target);
    handled
}
fn focus(element: &mut dyn Element) {
    element.layout(LayoutConstraints::loose(300.0, 100.0));
    assert!(element.accepts_keyboard_focus());
    assert!(element.handle_event(&Event::Focus(FocusEvent::Gained), Phase::Target));
}

#[test]
fn toggle_keyboard_callback_sees_bound_state_and_can_reject_change() {
    let value = State::new(generate_state_id(), false);
    let state = value.clone();
    let count = Rc::new(Cell::new(0));
    let calls = count.clone();
    let view = Toggle::new(value.clone()).on_change(move |requested| {
        assert!(requested);
        assert_eq!(state.get(), requested);
        calls.set(calls.get() + 1);
        state.set(false);
    });
    let mut element = view.create_element();
    focus(element.as_mut());
    assert!(key(element.as_mut(), KeyCode::Space));
    assert!(!value.get());
    assert!(key(element.as_mut(), KeyCode::Enter));
    assert_eq!(count.get(), 2);
    element.handle_event(&Event::Focus(FocusEvent::Lost), Phase::Target);
    assert!(!key(element.as_mut(), KeyCode::Space));
    assert_eq!(count.get(), 2);
}

#[test]
fn select_opens_from_keyboard_cancels_and_commits_only_changed_values() {
    let value = State::new(generate_state_id(), 1);
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let view = Select::new(
        vec![
            String::from("100%"),
            String::from("200%"),
            String::from("300%"),
        ],
        value.clone(),
    )
    .on_change(move |_| count.set(count.get() + 1));
    let mut element = view.create_element();
    focus(element.as_mut());
    assert!(key(element.as_mut(), KeyCode::Enter));
    assert!(view.expanded().get());
    key(element.as_mut(), KeyCode::Down);
    key(element.as_mut(), KeyCode::Escape);
    assert_eq!(value.get(), 1);
    assert_eq!(calls.get(), 0);
    key(element.as_mut(), KeyCode::Space);
    key(element.as_mut(), KeyCode::Space);
    assert_eq!(calls.get(), 0);
    key(element.as_mut(), KeyCode::Enter);
    key(element.as_mut(), KeyCode::Home);
    key(element.as_mut(), KeyCode::Enter);
    assert_eq!(value.get(), 0);
    assert_eq!(calls.get(), 1);
    key(element.as_mut(), KeyCode::Enter);
    element.handle_event(&Event::Focus(FocusEvent::Lost), Phase::Target);
    assert!(!view.expanded().get());
}

#[test]
fn slider_keys_clamp_to_range_and_keep_callback_rollback() {
    let value = State::new(generate_state_id(), 50.0);
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let view = Slider::new(value.clone())
        .min(0.0)
        .max(100.0)
        .on_change(move |_| count.set(count.get() + 1));
    let mut element = view.create_element();
    focus(element.as_mut());
    key(element.as_mut(), KeyCode::Right);
    assert_eq!(value.get(), 51.0);
    key(element.as_mut(), KeyCode::PageDown);
    assert_eq!(value.get(), 41.0);
    key(element.as_mut(), KeyCode::End);
    key(element.as_mut(), KeyCode::Up);
    assert_eq!(value.get(), 100.0);
    assert_eq!(calls.get(), 3);
    key(element.as_mut(), KeyCode::Home);
    key(element.as_mut(), KeyCode::Left);
    assert_eq!(value.get(), 0.0);
    let state = value.clone();
    let mut rejected = Slider::new(value.clone())
        .min(0.0)
        .max(100.0)
        .on_change(move |_| state.set(0.0))
        .create_element();
    focus(rejected.as_mut());
    key(rejected.as_mut(), KeyCode::Right);
    assert_eq!(value.get(), 0.0);
}

#[test]
fn disabled_controls_reject_focus_pointer_and_keyboard_mutation() {
    let toggle = State::new(generate_state_id(), false);
    let select = State::new(generate_state_id(), 0);
    let slider = State::new(generate_state_id(), 50.0);
    let mut controls = vec![
        Toggle::new(toggle.clone()).enabled(false).create_element(),
        Select::new(
            vec![String::from("one"), String::from("two")],
            select.clone(),
        )
        .enabled(false)
        .create_element(),
        Slider::new(slider.clone())
            .min(0.0)
            .max(100.0)
            .enabled(false)
            .create_element(),
    ];
    for element in &mut controls {
        element.layout(LayoutConstraints::loose(300.0, 100.0));
        assert!(!element.accepts_keyboard_focus());
        element.handle_event(&Event::Focus(FocusEvent::Gained), Phase::Target);
        for code in [KeyCode::Enter, KeyCode::Space, KeyCode::Right, KeyCode::End] {
            assert!(!key(element.as_mut(), code));
        }
        for event in [
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 5,
                y: 5,
                click_count: 1,
            },
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 5,
                y: 5,
                click_count: 1,
            },
        ] {
            assert!(!element.handle_event(&Event::Mouse(event), Phase::Target));
        }
    }
    assert!(!toggle.get());
    assert_eq!(select.get(), 0);
    assert_eq!(slider.get(), 50.0);
}

#[test]
fn select_invalid_selection_shows_placeholder_and_can_be_recovered_by_keyboard() {
    let value = State::new(generate_state_id(), usize::MAX);
    let mut element = Select::new(
        vec![String::from("one"), String::from("two")],
        value.clone(),
    )
    .placeholder("Choose a city")
    .create_element();
    focus(element.as_mut());
    assert_eq!(value.get(), usize::MAX);
    key(element.as_mut(), KeyCode::Home);
    key(element.as_mut(), KeyCode::Enter);
    assert_eq!(value.get(), 0);
    let empty = Select::new(vec![], value).create_element();
    assert!(!empty.accepts_keyboard_focus());
}

#[test]
fn toggle_pointer_activation_requires_press_and_touch_uses_same_callback() {
    let value = State::new(generate_state_id(), false);
    let count = Rc::new(Cell::new(0));
    let calls = count.clone();
    let mut element = Toggle::new(value.clone())
        .on_change(move |_| calls.set(calls.get() + 1))
        .create_element();
    element.layout(LayoutConstraints::loose(300.0, 100.0));
    let release = Event::Mouse(MouseEvent::ButtonReleased {
        button: MouseButton::Left,
        x: 5,
        y: 5,
        click_count: 1,
    });
    element.handle_event(&release, Phase::Target);
    assert_eq!(count.get(), 0);
    element.handle_event(
        &Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 5,
            y: 5,
            click_count: 1,
        }),
        Phase::Target,
    );
    element.handle_event(&release, Phase::Target);
    assert!(value.get());
    assert_eq!(count.get(), 1);
    element.handle_event(
        &Event::TouchGesture(crate::event::TouchGesture::Tap { id: 1, x: 5, y: 5 }),
        Phase::Target,
    );
    assert!(!value.get());
    assert_eq!(count.get(), 2);
}

#[test]
fn unselected_select_renders_placeholder_instead_of_claiming_first_choice() {
    use crate::element::ElementRenderObject;
    use crate::geometry::Point;
    use crate::renderer::{PaintCommand, PaintContext};
    use crate::views::SelectRenderObject;
    let mut object = SelectRenderObject::new(
        vec![String::from("one"), String::from("two")],
        usize::MAX,
        false,
        260.0,
        32.0,
        String::from("Choose"),
        8,
    );
    object.layout(LayoutConstraints::loose(300.0, 100.0));
    let mut context = PaintContext::new();
    object.paint(&mut context, Point::ZERO);
    assert!(
        context.commands().iter().any(
            |command| matches!(command, PaintCommand::DrawText { text, .. } if text == "Choose")
        )
    );
    assert!(object.typeahead('o'));
    assert_eq!(object.hovered_index(), Some(0));
}

#[test]
fn button_keyboard_activates_once_on_release_and_cancels_on_focus_loss() {
    use crate::views::Button;
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let mut element = Button::new("Save")
        .on_click(move || count.set(count.get() + 1))
        .create_element();
    focus(element.as_mut());
    for code in [KeyCode::Space, KeyCode::Enter] {
        assert!(press(element.as_mut(), code));
        assert!(press(element.as_mut(), code));
        assert!(element.handle_event(&Event::Keyboard(KeyEvent::Char { c: ' ' }), Phase::Target));
        let before = calls.get();
        assert!(element.handle_event(
            &Event::Keyboard(KeyEvent::Released {
                keycode: code,
                modifiers: KeyModifiers::default()
            }),
            Phase::Target
        ));
        assert_eq!(calls.get(), before + 1);
        element.handle_event(
            &Event::Keyboard(KeyEvent::Released {
                keycode: code,
                modifiers: KeyModifiers::default(),
            }),
            Phase::Target,
        );
        assert_eq!(calls.get(), before + 1);
    }
    assert_eq!(calls.get(), 2);
    press(element.as_mut(), KeyCode::Space);
    element.handle_event(&Event::Focus(FocusEvent::Lost), Phase::Target);
    element.handle_event(&Event::Focus(FocusEvent::Gained), Phase::Target);
    element.handle_event(
        &Event::Keyboard(KeyEvent::Released {
            keycode: KeyCode::Space,
            modifiers: KeyModifiers::default(),
        }),
        Phase::Target,
    );
    assert_eq!(calls.get(), 2);
}

#[test]
fn disabled_button_blocks_keyboard_mouse_touch_and_repeat() {
    use crate::views::Button;
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let view = Button::new("Unavailable")
        .enabled(false)
        .on_click(move || count.set(count.get() + 1));
    let mut element = view.create_element();
    element.layout(LayoutConstraints::loose(300.0, 100.0));
    assert!(!element.accepts_keyboard_focus());
    element.handle_event(&Event::Focus(FocusEvent::Gained), Phase::Target);
    assert!(!press(element.as_mut(), KeyCode::Space));
    for event in [
        Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 5,
            y: 5,
            click_count: 1,
        }),
        Event::Mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: 5,
            y: 5,
            click_count: 1,
        }),
        Event::TouchGesture(crate::event::TouchGesture::Tap { id: 1, x: 5, y: 5 }),
    ] {
        assert!(!element.handle_event(&event, Phase::Target));
    }
    view.invoke_on_click();
    assert!(view.press_repeat_timing().is_none());
    assert_eq!(calls.get(), 0);
}

#[test]
fn focused_button_paints_a_focus_ring_and_disabling_cancels_held_key() {
    use crate::element::ElementRenderObject;
    use crate::geometry::Point;
    use crate::renderer::{PaintCommand, PaintContext};
    use crate::views::{Button, ButtonRenderObject};
    let view = Button::new("Apply");
    let mut element = view.create_element();
    focus(element.as_mut());
    press(element.as_mut(), KeyCode::Space);
    let object = element
        .render_object_mut()
        .unwrap()
        .as_any_mut()
        .downcast_mut::<ButtonRenderObject>()
        .unwrap();
    let mut ctx = PaintContext::new();
    object.paint(&mut ctx, Point::ZERO);
    assert!(ctx.commands().iter().any(|command| matches!(command, PaintCommand::StrokeRoundedRect { color, .. } if *color == crate::color::ColorPalette::default().primary())));
    object.update(&view.enabled(false));
    assert!(!object.is_focused());
    assert!(!object.is_pressed());
    assert_eq!(
        object.handle_keyboard(KeyEvent::Released {
            keycode: KeyCode::Space,
            modifiers: KeyModifiers::default()
        }),
        None
    );
}

#[test]
fn held_activation_does_not_toggle_twice_or_commit_a_new_select_popup() {
    let state = State::new(generate_state_id(), false);
    let mut toggle = Toggle::new(state.clone()).create_element();
    focus(toggle.as_mut());
    press(toggle.as_mut(), KeyCode::Space);
    press(toggle.as_mut(), KeyCode::Space);
    assert!(state.get());
    let index = State::new(generate_state_id(), 0);
    let select = Select::new(vec![String::from("One"), String::from("Two")], index.clone());
    let mut element = select.create_element();
    focus(element.as_mut());
    press(element.as_mut(), KeyCode::Enter);
    press(element.as_mut(), KeyCode::Enter);
    assert!(select.expanded().get());
    key(element.as_mut(), KeyCode::Down);
    press(element.as_mut(), KeyCode::Enter);
    assert_eq!(index.get(), 0);
    element.handle_event(&Event::Keyboard(KeyEvent::Released { keycode: KeyCode::Enter, modifiers: KeyModifiers::default() }), Phase::Target);
    key(element.as_mut(), KeyCode::Enter);
    assert_eq!(index.get(), 1);
    assert!(!select.expanded().get());
}
