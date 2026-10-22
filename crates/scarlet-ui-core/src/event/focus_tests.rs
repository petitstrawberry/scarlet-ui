//! Keyboard focus integration tests through the mounted rendering pipeline.
use super::*;
use crate::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::geometry::Size;
use crate::pipeline::RenderingPipeline;
use crate::state::{State, generate_state_id};
use crate::view::{View, ViewExt};
use crate::views::{
    Button, HStack, ScrollView, Select, Slider, Spacer, TabItem, TabView, TextField, TextSelection,
    TextView, Toggle, VStack, Window,
};
use alloc::{rc::Rc, string::String, vec};
use core::cell::Cell;

fn state<T: Clone + Send + Sync + 'static>(value: T) -> State<T> {
    State::new(generate_state_id(), value)
}

fn pipeline(view: impl View + Clone + 'static) -> RenderingPipeline {
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(
        Window::new("Keyboard focus", view)
            .decorated(false)
            .size(Size::new(500.0, 400.0))
            .create_element(),
    );
    pipeline.layout_initial();
    pipeline
}

fn tab(pipeline: &mut RenderingPipeline, reverse: bool) -> bool {
    pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
        keycode: KeyCode::Tab,
        modifiers: KeyModifiers {
            shift: reverse,
            ..KeyModifiers::empty()
        },
    }))
}

fn focused(pipeline: &RenderingPipeline) -> Option<ElementId> {
    pipeline
        .element_tree()
        .find_keyboard_focus_path_ids()
        .and_then(|path| path.last().copied())
}

fn focused_count(element: &dyn Element) -> usize {
    usize::from(element.wants_keyboard_focus())
        + element
            .children()
            .iter()
            .map(|child| focused_count(child.as_ref()))
            .sum::<usize>()
}

#[test]
fn tab_and_shift_tab_visit_nested_enabled_controls_once_and_wrap() {
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let view = VStack::new((
        TextField::new(state(String::from("text"))).frame(200.0, 32.0),
        HStack::new((
            Toggle::new(state(false)),
            Slider::new(state(0.5)).frame(150.0, 32.0),
        ))
        .spacing(10.0),
        Select::new(vec![String::from("disabled")], state(0)).enabled(false),
        Select::new(vec![String::from("one"), String::from("two")], state(0)),
        Button::new("Done").on_click(move || count.set(count.get() + 1)),
    ))
    .spacing(10.0);
    let mut pipeline = pipeline(view);
    let order = pipeline.element_tree().keyboard_focus_candidates();
    assert_eq!(order.len(), 5);
    for expected in order.iter().copied().chain(core::iter::once(order[0])) {
        assert!(tab(&mut pipeline, false));
        assert_eq!(focused(&pipeline), Some(expected));
        assert_eq!(focused_count(pipeline.element_tree().root().unwrap()), 1);
    }
    assert!(tab(&mut pipeline, true));
    assert_eq!(focused(&pipeline), order.last().copied());
    for key in [
        KeyEvent::Pressed {
            keycode: KeyCode::Enter,
            modifiers: KeyModifiers::empty(),
        },
        KeyEvent::Released {
            keycode: KeyCode::Enter,
            modifiers: KeyModifiers::empty(),
        },
    ] {
        assert!(pipeline.handle_event(&Event::Keyboard(key)));
    }
    assert_eq!(calls.get(), 1);
    for expected in order.iter().rev().skip(1) {
        assert!(tab(&mut pipeline, true));
        assert_eq!(focused(&pipeline), Some(*expected));
    }
}

#[test]
fn reverse_tab_without_focus_starts_at_the_last_target() {
    let mut pipeline = pipeline(HStack::new((
        Toggle::new(state(false)),
        Toggle::new(state(false)),
    )));
    let order = pipeline.element_tree().keyboard_focus_candidates();
    assert_eq!(order.len(), 2);
    assert!(tab(&mut pipeline, true));
    assert_eq!(focused(&pipeline), order.last().copied());
}

#[test]
fn traversal_skips_zero_sized_and_clipped_offscreen_controls() {
    let scroll = ScrollView::new(VStack::new((
        TextField::new(state(String::from("visible"))).frame(180.0, 32.0),
        Spacer::new().frame_height(120.0),
        TextField::new(state(String::from("offscreen"))).frame(180.0, 32.0),
    )))
    .frame(200.0, 50.0);
    let mut pipeline = pipeline(VStack::new((
        Toggle::new(state(false)).frame(50.0, 0.0),
        scroll,
        Toggle::new(state(false)).enabled(false),
        Select::new(vec![], state(0)),
    )));
    let order = pipeline.element_tree().keyboard_focus_candidates();
    assert_eq!(order.len(), 1);
    assert!(tab(&mut pipeline, false));
    assert_eq!(focused(&pipeline), Some(order[0]));
    assert!(tab(&mut pipeline, false));
    assert_eq!(focused(&pipeline), Some(order[0]));
}

#[test]
fn tab_only_visits_the_active_tab_and_recovers_after_its_replacement() {
    let selected = state(0);
    let view = TabView::with_selected_index(
        vec![
            TabItem::new("First", || TextField::new(state(String::from("first")))),
            TabItem::new("Second", || Toggle::new(state(false))),
        ],
        selected.clone(),
    );
    let mut pipeline = pipeline(view);
    assert_eq!(pipeline.element_tree().keyboard_focus_candidates().len(), 1);
    assert!(tab(&mut pipeline, false));
    let first = focused(&pipeline);
    selected.set(1);
    pipeline.render_for_present().unwrap();
    let order = pipeline.element_tree().keyboard_focus_candidates();
    assert_eq!(order.len(), 1);
    assert_ne!(Some(order[0]), first);
    assert!(tab(&mut pipeline, false));
    assert_eq!(focused(&pipeline), Some(order[0]));
}

#[test]
fn moving_focus_closes_select_popup_and_preserves_single_line_text() {
    let text = state(String::from("unchanged"));
    let select = Select::new(vec![String::from("one"), String::from("two")], state(0));
    let expanded = select.expanded().clone();
    let mut pipeline = pipeline(
        VStack::new((
            TextField::new(text.clone()),
            select,
            Toggle::new(state(false)),
        ))
        .spacing(8.0),
    );
    let order = pipeline.element_tree().keyboard_focus_candidates();
    tab(&mut pipeline, false);
    tab(&mut pipeline, false);
    assert_eq!(text.get(), "unchanged");
    pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
        keycode: KeyCode::Enter,
        modifiers: KeyModifiers::empty(),
    }));
    assert!(expanded.get());
    assert!(tab(&mut pipeline, false));
    assert!(!expanded.get());
    assert_eq!(focused(&pipeline), Some(order[2]));
}

#[test]
fn multiline_editor_keeps_tab_for_indentation() {
    let text = state(String::from("abc"));
    let mut pipeline = pipeline(VStack::new((
        TextView::new(text.clone(), state(TextSelection::default())).frame(250.0, 100.0),
        Toggle::new(state(false)),
    )));
    assert!(tab(&mut pipeline, false));
    let editor = focused(&pipeline);
    assert!(tab(&mut pipeline, false));
    assert_eq!(focused(&pipeline), editor);
    assert_eq!(text.get(), "\tabc");
}

#[test]
fn modified_tab_is_not_hijacked_for_focus_navigation() {
    let mut pipeline = pipeline(HStack::new((
        Toggle::new(state(false)),
        Toggle::new(state(false)),
    )));
    tab(&mut pipeline, false);
    let first = focused(&pipeline);
    for modifiers in [
        KeyModifiers {
            control: true,
            ..KeyModifiers::empty()
        },
        KeyModifiers {
            alt: true,
            ..KeyModifiers::empty()
        },
        KeyModifiers {
            super_key: true,
            ..KeyModifiers::empty()
        },
    ] {
        pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Tab,
            modifiers,
        }));
        assert_eq!(focused(&pipeline), first);
    }
}
