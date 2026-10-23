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
use alloc::{boxed::Box, rc::Rc, string::String, vec};
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

#[derive(Clone)]
struct DynamicField {
    text: State<String>,
    request: State<u64>,
    visible: State<bool>,
}

impl View for DynamicField {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(crate::element::ComponentElement::new_with_builder(
            self.clone(),
            |view| {
                if view.visible.get() {
                    Box::new(
                        TextField::new(view.text.clone())
                            .autofocus(view.request.get() != 0)
                            .key(alloc::format!("field-{}", view.request.get()))
                            .frame(200.0, 32.0),
                    )
                } else {
                    Box::new(Spacer::new().frame(200.0, 32.0))
                }
            },
        ))
    }
    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        vec![&self.request, &self.visible]
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

#[test]
fn keyed_autofocus_takes_keyboard_and_ime_from_the_previous_grid_target() {
    let field = DynamicField {
        text: state(String::new()),
        request: state(0),
        visible: state(true),
    };
    let grid_focused = state(true);
    let mut pipeline = pipeline(VStack::new((
        field.clone(),
        Spacer::new()
            .frame(200.0, 60.0)
            .focusable(grid_focused.clone()),
    )));
    pipeline.render_for_present().unwrap();
    let grid_id = focused(&pipeline);
    assert!(grid_focused.get());

    for request in [1, 2] {
        field.request.set(request);
        pipeline.render_for_present().unwrap();
        assert!(!grid_focused.get(), "autofocus must release the old owner");
        assert_ne!(focused(&pipeline), grid_id);
        assert_eq!(focused_count(pipeline.element_tree().root().unwrap()), 1);
        assert!(pipeline.element_tree().focused_text_input_state().is_some());
        assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: 'x' })));
        assert!(pipeline.handle_event(&Event::TextInputCommit {
            context_id: 1,
            serial: request as u32,
            text: "語".into(),
        }));
        pipeline.render_for_present().unwrap();
        assert_eq!(focused_count(pipeline.element_tree().root().unwrap()), 1);

        grid_focused.set(true);
        pipeline.render_for_present().unwrap();
        assert_eq!(focused(&pipeline), grid_id);
        assert!(pipeline.element_tree().focused_text_input_state().is_none());
    }
    assert_eq!(field.text.get(), "x語x語");
}

#[test]
fn removing_the_focused_editor_restores_the_ancestor_shortcut_route() {
    let field = DynamicField {
        text: state(String::new()),
        request: state(1),
        visible: state(true),
    };
    let root_focused = state(true);
    let shortcuts = Rc::new(Cell::new(0));
    let count = shortcuts.clone();
    let mut pipeline = pipeline(
        VStack::new((field.clone(), Button::new("Other")))
            .focusable(root_focused.clone())
            .on_event_capture(move |event| {
                if let Event::Keyboard(KeyEvent::Pressed {
                    keycode: KeyCode::Char('3'),
                    modifiers,
                }) = event
                    && modifiers.control
                {
                    count.set(count.get() + 1);
                    return true;
                }
                false
            }),
    );
    pipeline.render_for_present().unwrap();
    assert!(!root_focused.get(), "the child is the sole initial owner");
    assert!(pipeline.element_tree().focused_text_input_state().is_some());
    field.visible.set(false);
    pipeline.render_for_present().unwrap();
    assert!(root_focused.get());
    assert_eq!(focused_count(pipeline.element_tree().root().unwrap()), 1);
    assert!(pipeline.element_tree().focused_text_input_state().is_none());
    assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
        keycode: KeyCode::Char('3'),
        modifiers: KeyModifiers {
            control: true,
            ..KeyModifiers::empty()
        },
    })));
    assert_eq!(shortcuts.get(), 1);
    assert!(tab(&mut pipeline, false));
    assert!(!root_focused.get(), "Tab moves from the restored ancestor");
}

#[test]
fn existing_field_blur_returns_focus_to_its_ancestor() {
    let parent_focused = state(true);
    let mut pipeline = pipeline(
        TextField::new(state(String::from("text")))
            .autofocus(true)
            .focusable(parent_focused.clone()),
    );
    pipeline.render_for_present().unwrap();
    assert!(!parent_focused.get());
    assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
        keycode: KeyCode::Escape,
        modifiers: KeyModifiers::empty(),
    })));
    pipeline.render_for_present().unwrap();
    assert!(parent_focused.get());
    assert_eq!(focused_count(pipeline.element_tree().root().unwrap()), 1);
    assert!(pipeline.element_tree().focused_text_input_state().is_none());
}

fn press(pipeline: &mut RenderingPipeline, keycode: KeyCode) -> bool {
    pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
        keycode,
        modifiers: KeyModifiers::empty(),
    }))
}

fn scroll_offsets(element: &dyn Element, offsets: &mut Vec<Point>) {
    if element
        .render_object()
        .is_some_and(|render| render.owns_child_viewport())
    {
        offsets.push(element.children()[0].bounds().origin);
    }
    for child in element.children() {
        scroll_offsets(child.as_ref(), offsets);
    }
}

fn offsets(pipeline: &RenderingPipeline) -> Vec<Point> {
    let mut offsets = Vec::new();
    scroll_offsets(pipeline.element_tree().root().unwrap(), &mut offsets);
    offsets
}

#[test]
fn page_navigation_reveals_offscreen_form_actions_without_pointer_input() {
    let activated = Rc::new(Cell::new(0));
    let saved = activated.clone();
    let mut pipeline = pipeline(
        ScrollView::new(
            VStack::new((
                Select::new(vec![String::from("Style")], state(0)).frame(180.0, 32.0),
                Spacer::new().frame(180.0, 240.0),
                Button::new("Save again")
                    .on_click(move || saved.set(saved.get() + 1))
                    .frame(180.0, 32.0),
            ))
            .spacing(0.0),
        )
        .content_size(200.0, 304.0)
        .frame(200.0, 80.0),
    );
    pipeline.render_for_present().unwrap();
    assert_eq!(pipeline.element_tree().keyboard_focus_candidates().len(), 1);
    assert!(tab(&mut pipeline, false));
    assert!(press(&mut pipeline, KeyCode::PageDown));
    assert_eq!(offsets(&pipeline)[0].y, -72.0);
    assert!(press(&mut pipeline, KeyCode::PageDown));
    assert!(press(&mut pipeline, KeyCode::PageDown));
    pipeline.render_for_present().unwrap();
    assert!(
        tab(&mut pipeline, false),
        "the action is now inside the viewport"
    );
    assert!(press(&mut pipeline, KeyCode::Enter));
    assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Released {
        keycode: KeyCode::Enter,
        modifiers: KeyModifiers::empty(),
    })));
    assert_eq!(activated.get(), 1);
    assert!(press(&mut pipeline, KeyCode::Home));
    assert_eq!(offsets(&pipeline)[0].y, 0.0);
    assert!(press(&mut pipeline, KeyCode::End));
    assert_eq!(offsets(&pipeline)[0].y, -224.0);
    assert!(press(&mut pipeline, KeyCode::PageUp));
    assert_eq!(offsets(&pipeline)[0].y, -152.0);
}

#[test]
fn page_keys_navigate_an_open_select_without_scrolling_its_parent() {
    let selected = state(0);
    let mut pipeline = pipeline(
        ScrollView::new(
            VStack::new((
                Select::new(
                    (0..20).map(|i| alloc::format!("Option {i}")).collect(),
                    selected.clone(),
                )
                .max_visible_rows(4)
                .frame(180.0, 32.0),
                Spacer::new().frame(180.0, 268.0),
            ))
            .spacing(0.0),
        )
        .content_size(200.0, 300.0)
        .frame(200.0, 100.0),
    );
    pipeline.render_for_present().unwrap();
    assert!(tab(&mut pipeline, false));
    assert!(press(&mut pipeline, KeyCode::Enter));
    assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Released {
        keycode: KeyCode::Enter,
        modifiers: KeyModifiers::empty(),
    })));
    for key in [KeyCode::PageDown, KeyCode::PageDown, KeyCode::PageUp] {
        assert!(press(&mut pipeline, key));
        assert_eq!(offsets(&pipeline)[0], Point::ZERO);
    }
    assert!(press(&mut pipeline, KeyCode::Enter));
    assert_eq!(selected.get(), 3);
    assert!(
        press(&mut pipeline, KeyCode::PageDown),
        "closed Select lets the form page"
    );
    assert_eq!(offsets(&pipeline)[0].y, -90.0);
}

#[test]
fn paging_nested_scrolls_moves_the_nearest_viewport_then_bubbles_at_its_edge() {
    let inner = ScrollView::new(
        VStack::new((
            Button::new("Focused").frame(160.0, 32.0),
            Spacer::new().frame(160.0, 168.0),
        ))
        .spacing(0.0),
    )
    .content_size(180.0, 200.0)
    .frame(180.0, 80.0);
    let mut pipeline = pipeline(
        ScrollView::new(VStack::new((inner, Spacer::new().frame(200.0, 320.0))).spacing(0.0))
            .content_size(200.0, 400.0)
            .frame(200.0, 100.0),
    );
    pipeline.render_for_present().unwrap();
    assert!(tab(&mut pipeline, false));
    assert!(press(&mut pipeline, KeyCode::End));
    assert_eq!(
        offsets(&pipeline),
        vec![Point::ZERO, Point::new(0.0, -120.0)]
    );
    assert!(press(&mut pipeline, KeyCode::End));
    assert_eq!(
        offsets(&pipeline),
        vec![Point::new(0.0, -300.0), Point::new(0.0, -120.0)]
    );
    assert!(!press(&mut pipeline, KeyCode::End));
}

#[test]
fn editor_and_slider_navigation_do_not_scroll_the_enclosing_form() {
    let selection = state(TextSelection::default());
    let text = state("line\n".repeat(50));
    let single = state(String::from("single line"));
    let value = state(0.5);
    let mut pipeline = pipeline(
        ScrollView::new(
            VStack::new((
                TextView::new(text.clone(), selection.clone()).frame(180.0, 60.0),
                TextField::new(single.clone()).frame(180.0, 32.0),
                Slider::new(value.clone()).frame(180.0, 32.0),
                Spacer::new().frame(180.0, 300.0),
            ))
            .spacing(0.0),
        )
        .content_size(200.0, 424.0)
        .frame(200.0, 140.0),
    );
    pipeline.render_for_present().unwrap();
    assert!(tab(&mut pipeline, false));
    for key in [KeyCode::PageDown, KeyCode::Home, KeyCode::End] {
        assert!(press(&mut pipeline, key));
        assert_eq!(offsets(&pipeline)[0], Point::ZERO);
    }
    assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
        keycode: KeyCode::PageDown,
        modifiers: KeyModifiers {
            shift: true,
            ..KeyModifiers::empty()
        },
    })));
    assert!(
        !selection.get().is_collapsed(),
        "Shift+PageDown extends the editor selection"
    );
    assert_eq!(offsets(&pipeline)[0], Point::ZERO);
    // A pointer-free programmatic focus handoff avoids changing the multiline
    // editor's intentional Tab-for-indentation behavior.
    let targets = pipeline.element_tree().keyboard_focus_candidates();
    for (index, key) in [(1, KeyCode::End), (2, KeyCode::PageDown)] {
        let id = targets[index];
        let tree = pipeline.element_tree_mut();
        for claim in tree.keyboard_focus_claims() {
            tree.find_element_mut(claim)
                .unwrap()
                .handle_event(&Event::Focus(crate::event::FocusEvent::Lost), Phase::Target);
        }
        tree.find_element_mut(id).unwrap().handle_event(
            &Event::Focus(crate::event::FocusEvent::Gained),
            Phase::Target,
        );
        assert!(press(&mut pipeline, key));
        assert_eq!(offsets(&pipeline)[0], Point::ZERO);
    }
    assert!(value.get() < 0.5);
    assert_eq!(single.get(), "single line");
    assert_eq!(text.get(), "line\n".repeat(50));
}

#[test]
fn modifier_only_primary_click_reports_text_hit_without_selecting_then_plain_click_selects() {
    use crate::event::{MouseButton, MouseEvent};
    use crate::views::TextViewHit;
    use core::cell::RefCell;
    let selection = state(TextSelection::collapsed(0));
    let hits = Rc::new(RefCell::new(Vec::<TextViewHit>::new()));
    let calls = hits.clone();
    let mut pipeline = pipeline(
        TextView::new(state(String::from("hello 日本")), selection.clone())
            .on_modified_click(move |hit| calls.borrow_mut().push(hit)),
    );
    let primary = KeyModifiers {
        control: !cfg!(target_os = "macos"),
        super_key: cfg!(target_os = "macos"),
        ..KeyModifiers::empty()
    };
    pipeline.handle_event(&Event::ModifiersChanged(primary));
    let press = Event::Mouse(MouseEvent::ButtonPressed {
        button: MouseButton::Left,
        x: 35,
        y: 20,
        click_count: 1,
    });
    assert!(pipeline.handle_event(&press));
    assert_eq!(hits.borrow().len(), 1);
    assert!(hits.borrow()[0].byte_offset > 0);
    assert_eq!(hits.borrow()[0].position, Point::new(35.0, 20.0));
    assert_eq!(selection.get(), TextSelection::collapsed(0));
    pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 75, y: 20 }));
    assert_eq!(selection.get(), TextSelection::collapsed(0));
    pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
        button: MouseButton::Left,
        x: 75,
        y: 20,
        click_count: 1,
    }));
    pipeline.handle_event(&Event::ModifiersChanged(KeyModifiers::empty()));
    assert!(pipeline.handle_event(&press));
    assert!(selection.get().caret.byte > 0);
    assert_eq!(hits.borrow().len(), 1);
    pipeline.teardown();
}

#[test]
fn primary_modifier_uses_command_on_macos_and_control_elsewhere() {
    let control = KeyModifiers {
        control: true,
        ..KeyModifiers::empty()
    };
    let meta = KeyModifiers {
        super_key: true,
        ..KeyModifiers::empty()
    };
    assert_eq!(control.primary(), !cfg!(target_os = "macos"));
    assert_eq!(meta.primary(), cfg!(target_os = "macos"));
    assert!(!KeyModifiers::empty().primary());
}
