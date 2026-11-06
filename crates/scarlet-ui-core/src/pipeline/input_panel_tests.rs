use super::*;
use crate::event::{KeyEvent, MouseEvent, ScrollSource, WheelPhase};
use crate::geometry::Size;
use crate::pipeline::RenderingPipeline;
use crate::platform::WindowDecoration;
use crate::state::{State, generate_state_id};
use crate::view::{View, ViewExt};
use crate::views::{
    Rectangle, ScrollView, TextField, TextSelection, TextView, TextViewScroll, VStack, Window,
};
use alloc::string::String;

const SURFACE: Size = Size::new(480.0, 600.0);
const KEYBOARD: Rect = Rect::from_xywh(0.0, 360.0, 480.0, 240.0);

fn state<T>(value: T) -> State<T> {
    State::new(generate_state_id(), value)
}

fn mounted(content: impl View + Clone + 'static) -> RenderingPipeline {
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(
        Window::new("Keyboard", content)
            .decoration(WindowDecoration::CUSTOM)
            .shadow(false)
            .size(SURFACE)
            .create_element(),
    );
    pipeline.layout_initial();
    pipeline.handle_event(&Event::TabletPresentationChanged { active: true });
    frame(&mut pipeline);
    pipeline
}

fn frame(pipeline: &mut RenderingPipeline) {
    pipeline.request_redraw();
    let (buffer, _) = pipeline.render_with_damage().expect("mounted CPU frame");
    assert_eq!((buffer.width(), buffer.height()), (480, 600));
    assert_eq!(pipeline.window_size(), SURFACE);
    assert_eq!(
        pipeline.element_tree().root().unwrap().bounds().size,
        SURFACE
    );
}

fn content(pipeline: &RenderingPipeline) -> &dyn Element {
    pipeline
        .element_tree()
        .root()
        .unwrap()
        .children()
        .last()
        .unwrap()
        .as_ref()
}

fn panel(pipeline: &mut RenderingPipeline, occlusion: Option<Rect>) {
    assert!(pipeline.handle_event(&Event::InputPanelOcclusionChanged { occlusion }));
    frame(pipeline);
}

fn visible_caret(pipeline: &RenderingPipeline) -> Rect {
    let caret = pipeline
        .focused_text_input_state()
        .expect("retained keyboard focus")
        .cursor_rect;
    assert!(caret.top() >= 0.0, "caret above viewport: {caret:?}");
    assert!(
        caret.bottom() <= KEYBOARD.top() + 0.01,
        "caret hidden by keyboard: {caret:?}"
    );
    caret
}

#[test]
fn mounted_text_view_reveals_caret_without_resizing_or_overriding_manual_scroll() {
    let initial = "A long editable document line\n".repeat(80);
    let text = state(initial.clone());
    let selection = state(TextSelection::collapsed(initial.len()));
    let scroll = state(TextViewScroll::default());
    let view = TextView::new(text.clone(), selection.clone())
        .autofocus(true)
        .scroll_state(scroll.clone())
        .font_size(16.0)
        .padding(4.0);
    let mut pipeline = mounted(view);
    let id = content(&pipeline).id();
    let original_size = content(&pipeline).bounds().size;
    assert_eq!(original_size, SURFACE);

    panel(&mut pipeline, Some(KEYBOARD));
    visible_caret(&pipeline);
    assert!(scroll.get().y > 0.0);
    assert_eq!(
        content(&pipeline).position(),
        Point::ZERO,
        "TextView should reveal internally"
    );
    let revealed = scroll.get();
    for _ in 0..3 {
        frame(&mut pipeline);
        visible_caret(&pipeline);
        assert_eq!(scroll.get(), revealed);
    }

    assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
        delta_x: 0,
        delta_y: 150,
        x: 80,
        y: 100,
        phase: WheelPhase::Moved,
        source: ScrollSource::Wheel,
    })));
    frame(&mut pipeline);
    let manual = scroll.get();
    assert!(
        manual.y < revealed.y,
        "manual scrolling must move away from the caret"
    );
    for _ in 0..3 {
        frame(&mut pipeline);
        assert_eq!(
            scroll.get(),
            manual,
            "idle frames must not force caret reveal"
        );
    }
    assert_eq!(selection.get(), TextSelection::collapsed(initial.len()));
    assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: 'X' })));
    frame(&mut pipeline);
    visible_caret(&pipeline);
    assert_eq!(text.get(), initial + "X");
    assert!(
        scroll.get().y > manual.y,
        "typing should reveal the caret again"
    );
    assert_eq!(content(&pipeline).id(), id);
    assert_eq!(content(&pipeline).bounds().size, original_size);

    let selection_before_hide = selection.get();
    panel(&mut pipeline, None);
    for _ in 0..3 {
        frame(&mut pipeline);
    }
    assert_eq!(content(&pipeline).bounds().size, original_size);
    assert_eq!(selection.get(), selection_before_hide);
    assert!(
        pipeline
            .focused_text_input_state()
            .unwrap()
            .cursor_rect
            .bottom()
            <= SURFACE.height
    );
    panel(&mut pipeline, Some(KEYBOARD));
    visible_caret(&pipeline);
}

#[test]
fn mounted_scroll_view_reveals_text_field_before_window_fallback() {
    let text = state(String::from("retained"));
    let form = VStack::new((
        Rectangle::new().frame(480.0, 500.0),
        TextField::new(text.clone())
            .autofocus(true)
            .frame(480.0, 40.0),
        Rectangle::new().frame(480.0, 400.0),
    ));
    let mut pipeline = mounted(ScrollView::new(form).vertical());
    let scroll_id = content(&pipeline).id();
    let form_id = content(&pipeline).children()[0].id();
    let form_size = content(&pipeline).children()[0].bounds().size;
    assert!(
        pipeline
            .focused_text_input_state()
            .unwrap()
            .cursor_rect
            .bottom()
            > KEYBOARD.top()
    );
    panel(&mut pipeline, Some(KEYBOARD));
    visible_caret(&pipeline);
    let position = content(&pipeline).children()[0].position();
    assert!(
        position.y < 0.0,
        "the inner ScrollView must perform the reveal"
    );
    assert_eq!(
        content(&pipeline).position(),
        Point::ZERO,
        "no redundant Window pan"
    );
    for _ in 0..4 {
        frame(&mut pipeline);
        visible_caret(&pipeline);
        assert_eq!(content(&pipeline).children()[0].position(), position);
    }
    assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: '!' })));
    frame(&mut pipeline);
    visible_caret(&pipeline);
    assert_eq!(text.get(), "retained!");
    panel(&mut pipeline, None);
    assert_eq!(content(&pipeline).id(), scroll_id);
    assert_eq!(content(&pipeline).bounds().size, SURFACE);
    assert_eq!(content(&pipeline).children()[0].id(), form_id);
    assert_eq!(content(&pipeline).children()[0].bounds().size, form_size);
    assert_eq!(content(&pipeline).position(), Point::ZERO);
}

#[test]
fn mounted_window_fallback_is_stable_across_frames_typing_and_panel_hide() {
    let text = state(String::from("fixed form"));
    let form = VStack::new((
        Rectangle::new().frame(480.0, 500.0),
        TextField::new(text.clone())
            .autofocus(true)
            .frame(480.0, 40.0),
    ));
    let mut pipeline = mounted(form);
    let id = content(&pipeline).id();
    let size = content(&pipeline).bounds().size;
    let before = pipeline.focused_text_input_state().unwrap().cursor_rect;
    assert!(before.bottom() > KEYBOARD.top());
    panel(&mut pipeline, Some(KEYBOARD));
    let caret = visible_caret(&pipeline);
    let position = content(&pipeline).position();
    assert_eq!(position.y, KEYBOARD.top() - before.bottom());
    for _ in 0..5 {
        frame(&mut pipeline);
        assert_eq!(visible_caret(&pipeline), caret);
        assert_eq!(content(&pipeline).position(), position);
        assert_eq!(content(&pipeline).bounds().size, size);
    }
    // Painting and hit testing share the same translated child positions.
    let hit = pipeline.element_tree().hit_test(caret.center()).unwrap();
    assert!(hit.accepts_keyboard_focus());
    assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: '!' })));
    for _ in 0..3 {
        frame(&mut pipeline);
        visible_caret(&pipeline);
    }
    assert_eq!(text.get(), "fixed form!");
    assert_eq!(content(&pipeline).id(), id);
    assert_eq!(content(&pipeline).position(), position);
    panel(&mut pipeline, None);
    assert_eq!(content(&pipeline).position(), Point::ZERO);
    assert_eq!(content(&pipeline).bounds().size, size);
}

#[test]
fn rebuilt_window_has_tablet_content_bounds_in_its_first_painted_frame() {
    let mut pipeline = mounted(Rectangle::new().frame(f32::INFINITY, f32::INFINITY));
    // A structurally replaced root starts from the desktop declaration while
    // OverlayState retains the platform's tablet state. Replay runs after layout.
    pipeline.set_root(
        Window::new(
            "Rebuilt",
            Rectangle::new().frame(f32::INFINITY, f32::INFINITY),
        )
        .decoration(WindowDecoration::CUSTOM)
        .shadow(false)
        .size(SURFACE)
        .create_element(),
    );
    pipeline.layout_initial();
    frame(&mut pipeline);
    assert_eq!(content(&pipeline).bounds(), Rect::new(Point::ZERO, SURFACE));
    let id = content(&pipeline).id();
    frame(&mut pipeline);
    assert_eq!(content(&pipeline).id(), id);
    assert_eq!(content(&pipeline).bounds(), Rect::new(Point::ZERO, SURFACE));
}

#[test]
fn native_keyboard_reveal_first_reveals_inside_a_fully_occluded_nested_scroll() {
    let inner = ScrollView::new(
        VStack::new((
            Rectangle::new().frame(400.0, 250.0),
            TextField::new(state(String::from("nested")))
                .autofocus(true)
                .frame(400.0, 40.0),
            Rectangle::new().frame(400.0, 100.0),
        ))
        .spacing(0.0),
    )
    .vertical()
    .content_size(400.0, 390.0)
    .frame(400.0, 200.0);
    let mut pipeline = mounted(
        ScrollView::new(
            VStack::new((
                Rectangle::new().frame(480.0, 400.0),
                inner,
                Rectangle::new().frame(480.0, 500.0),
            ))
            .spacing(0.0),
        )
        .vertical()
        .content_size(480.0, 1100.0),
    );
    panel(&mut pipeline, Some(KEYBOARD));
    let caret = visible_caret(&pipeline);
    fn check_viewports(element: &dyn Element, parent: Point, caret: Rect) {
        let position = element.position();
        let origin = Point::new(parent.x + position.x, parent.y + position.y);
        if element
            .render_object()
            .is_some_and(|r| r.owns_child_viewport())
        {
            let bounds = Rect::new(origin, element.bounds().size);
            assert!(
                caret.top() >= bounds.top() && caret.bottom() <= bounds.bottom() + 0.01,
                "the caret must be visible inside every nested viewport: {caret:?}, {bounds:?}"
            );
        }
        for child in element.children() {
            check_viewports(child.as_ref(), origin, caret);
        }
    }
    for _ in 0..3 {
        check_viewports(pipeline.element_tree().root().unwrap(), Point::ZERO, caret);
        frame(&mut pipeline);
        assert_eq!(visible_caret(&pipeline), caret);
    }
    assert_eq!(content(&pipeline).position(), Point::ZERO, "no Window pan");
}

#[test]
fn native_keyboard_reveal_keeps_the_caret_below_the_scroll_safe_area() {
    use crate::geometry::{EdgeInsets, SafeAreaEdges};
    let form = VStack::new((
        Rectangle::new().frame(480.0, 100.0),
        TextField::new(state(String::from("safe")))
            .autofocus(true)
            .frame(480.0, 40.0),
        Rectangle::new().frame(480.0, 1000.0),
    ))
    .spacing(0.0);
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(
        Window::new(
            "Safe keyboard",
            ScrollView::new(form).vertical().wheel_sensitivity(1.0),
        )
        .content_safe_area(SafeAreaEdges::NONE)
        .decoration(WindowDecoration::NONE)
        .shadow(false)
        .size(SURFACE)
        .create_element(),
    );
    pipeline.layout_initial();
    pipeline.set_safe_area_insets(EdgeInsets::new(0.0, 47.0, 0.0, 29.0));
    pipeline.handle_event(&Event::TabletPresentationChanged { active: true });
    panel(&mut pipeline, Some(KEYBOARD));
    assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
        delta_x: 0,
        delta_y: -140,
        x: 450,
        y: 100,
        phase: WheelPhase::Moved,
        source: ScrollSource::Wheel,
    })));
    frame(&mut pipeline);
    let manual_caret = visible_caret(&pipeline);
    assert!(
        manual_caret.top() < 47.0,
        "fixture must scroll under the top inset"
    );
    assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: '!' })));
    frame(&mut pipeline);
    assert!(
        visible_caret(&pipeline).top() >= 47.0,
        "native reveal must honor the same safe top as Web reveal"
    );
    assert_eq!(content(&pipeline).bounds(), Rect::new(Point::ZERO, SURFACE));
}

#[test]
fn native_window_pan_only_invalidates_composition() {
    let mut pipeline = mounted(VStack::new((
        Rectangle::new().frame(480.0, 500.0),
        TextField::new(state(String::from("static")))
            .autofocus(true)
            .frame(480.0, 40.0),
    )));
    // Drain presentation/layout work before measuring a position-only reveal.
    frame(&mut pipeline);
    let root_id = pipeline.element_tree().root().unwrap().id();
    for occlusion in [Some(KEYBOARD), None, Some(KEYBOARD)] {
        panel(&mut pipeline, occlusion);
        assert!(
            pipeline.pipeline_owner.last_paint_ids().is_empty(),
            "moving the Window's content must not repaint its static subtree"
        );
        assert!(
            pipeline
                .pipeline_owner
                .last_composite_ids()
                .contains(&root_id)
        );
    }
}
