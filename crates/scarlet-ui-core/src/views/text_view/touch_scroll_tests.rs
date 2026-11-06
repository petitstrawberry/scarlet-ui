use super::*;
use crate::event::{GesturePhase, TouchChange, TouchFrame, TouchGesture, TouchPhase};
use crate::pipeline::RenderingPipeline;
use crate::state::generate_state_id;
use crate::view::ViewExt;
use core::sync::atomic::{AtomicUsize, Ordering};

fn state<T>(value: T) -> State<T> {
    State::new(generate_state_id(), value)
}

#[test]
fn touch_scroll_reuses_cached_geometry_and_clamps_both_axes() {
    for bound in [false, true] {
        let selection = state(TextSelection::collapsed(5));
        let scroll = state(TextViewScroll::default());
        let mut view = TextView::new(
            state("long document row ".repeat(12) + &"\nrow".repeat(40)),
            selection.clone(),
        )
        .wrap_mode(WrapMode::None);
        if bound {
            view = view.scroll_state(scroll.clone());
        }
        let mut object = render_object_with_size(&view, 180.0, 100.0);
        let storage = object.layout.visual_lines.as_ptr();
        let document = object.text_document.clone();
        let original_selection = selection.get();
        assert!(object.accepts_touch_scroll());
        assert!(handle_text_view_scroll_gesture(
            &view,
            &mut object,
            &TouchGesture::Scroll {
                id: 1,
                phase: GesturePhase::Started,
                delta_x: -25,
                delta_y: -45,
            }
        ));
        assert_eq!(object.scroll, TextViewScroll { x: 25.0, y: 45.0 });
        assert_eq!(object.requested_scroll, object.scroll);
        if bound {
            assert_eq!(scroll.get(), object.scroll);
        }
        assert!(handle_text_view_scroll_gesture(
            &view,
            &mut object,
            &TouchGesture::ScrollMomentum {
                id: 1,
                phase: GesturePhase::Started,
                delta_x: 0,
                delta_y: 0,
            }
        ));
        let movement = TouchGesture::ScrollMomentum {
            id: 1,
            phase: GesturePhase::Moved,
            delta_x: -10_000,
            delta_y: -10_000,
        };
        assert!(handle_text_view_scroll_gesture(
            &view,
            &mut object,
            &movement
        ));
        assert_eq!(
            object.scroll,
            object.layout.clamp_scroll(TextViewScroll {
                x: 10_000.0,
                y: 10_000.0
            })
        );
        assert!(!handle_text_view_scroll_gesture(
            &view,
            &mut object,
            &movement
        ));
        let at_edge = object.scroll;
        for phase in [GesturePhase::Ended, GesturePhase::Cancelled] {
            assert!(!handle_text_view_scroll_gesture(
                &view,
                &mut object,
                &TouchGesture::Scroll {
                    id: 1,
                    phase,
                    delta_x: 100,
                    delta_y: 100,
                }
            ));
            assert!(!handle_text_view_scroll_gesture(
                &view,
                &mut object,
                &TouchGesture::ScrollMomentum {
                    id: 1,
                    phase,
                    delta_x: 100,
                    delta_y: 100,
                }
            ));
        }
        object.update(&view);
        assert_eq!(object.scroll, at_edge);
        assert_eq!(object.layout.visual_lines.as_ptr(), storage);
        assert!(object.text_document.same_revision(&document));
        assert_eq!(selection.get(), original_selection);
        assert_eq!(object.selection, original_selection);
        assert!(!object.dragging);
        assert!(!object.update_needs_layout());
    }
    let view = TextView::new(
        state(String::from("short")),
        state(TextSelection::collapsed(0)),
    );
    let mut object = render_object_with_size(&view, 300.0, 200.0);
    assert!(!object.accepts_touch_scroll());
    assert!(!handle_text_view_scroll_gesture(
        &view,
        &mut object,
        &TouchGesture::ScrollMomentum {
            id: 1,
            phase: GesturePhase::Started,
            delta_x: 0,
            delta_y: 0,
        }
    ));
}

fn touch(serial: u64, phase: TouchPhase, y: i32) -> Event {
    Event::TouchFrame(TouchFrame {
        seat_id: 0,
        serial,
        time_ns: serial * 16_000_000,
        changes: alloc::vec![TouchChange {
            seat_id: 0,
            serial,
            time_ns: serial * 16_000_000,
            id: 7,
            phase,
            x: 300,
            y,
            pressure: None,
            touch_major: None,
        }],
    })
}

fn editor(element: &dyn Element) -> &TextViewRenderObject {
    if let Some(object) = element
        .render_object()
        .and_then(|object| object.as_any().downcast_ref::<TextViewRenderObject>())
    {
        return object;
    }
    editor(element.children().last().unwrap().as_ref())
}

#[test]
fn mounted_touch_frame_pan_and_momentum_preserve_keyboard_selection_and_layout() {
    let initial = "A long editable document line\n".repeat(23) + "last line";
    let text = state(initial.clone());
    let selection = state(TextSelection::collapsed(initial.len()));
    let scroll = state(TextViewScroll::default());
    let layouts = Arc::new(AtomicUsize::new(0));
    let count = layouts.clone();
    let view = TextView::new(text.clone(), selection.clone())
        .autofocus(true)
        .scroll_state(scroll.clone())
        .reveal_selection_on_change(true)
        .font_size(16.0)
        .padding(4.0)
        .on_geometry_change(
            move |geometry| {
                count.fetch_add(1, Ordering::Relaxed);
                geometry.size()
            },
            |_| {},
        );
    let surface = Size::new(480.0, 600.0);
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(
        crate::views::Window::new("Touch editor", view)
            .decoration(crate::platform::WindowDecoration::CUSTOM)
            .shadow(false)
            .size(surface)
            .create_element(),
    );
    pipeline.layout_initial();
    pipeline.handle_event(&Event::TabletPresentationChanged { active: true });
    pipeline.handle_event(&Event::InputPanelOcclusionChanged {
        occlusion: Some(Rect::from_xywh(0.0, 360.0, 480.0, 240.0)),
    });
    pipeline.render();
    let revealed = scroll.get();
    assert!(revealed.y > 0.0);
    assert!(
        pipeline
            .focused_text_input_state()
            .unwrap()
            .cursor_rect
            .bottom()
            <= 360.0
    );
    let baseline = layouts.load(Ordering::Relaxed);
    let storage = editor(pipeline.element_tree().root().unwrap())
        .layout
        .visual_lines
        .as_ptr();

    // Physical (600,180)->(600,340) at 2x scale, in logical coordinates.
    for (serial, phase, y) in [
        (1, TouchPhase::Down, 90),
        (2, TouchPhase::Move, 110),
        (3, TouchPhase::Move, 140),
        (4, TouchPhase::Move, 170),
    ] {
        pipeline.handle_event(&touch(serial, phase, y));
        pipeline.render();
        assert_eq!(selection.get(), TextSelection::collapsed(initial.len()));
        assert!(pipeline.focused_text_input_state().is_some());
        assert_eq!(layouts.load(Ordering::Relaxed), baseline);
    }
    assert_eq!(scroll.get().y, revealed.y - 80.0);
    assert!(
        pipeline
            .focused_text_input_state()
            .unwrap()
            .cursor_rect
            .bottom()
            > 360.0
    );
    let manual = scroll.get();
    for _ in 0..3 {
        pipeline.request_redraw();
        pipeline.render();
        assert_eq!(
            scroll.get(),
            manual,
            "idle keyboard avoidance must not undo a pan"
        );
    }
    pipeline.handle_event(&touch(5, TouchPhase::Up, 170));
    assert!(
        pipeline.has_active_animation(),
        "zero-delta momentum start must be accepted"
    );
    assert!(pipeline.advance_animations(core::time::Duration::from_millis(16)));
    pipeline.render();
    assert!(scroll.get().y < manual.y);
    for _ in 0..240 {
        if !pipeline.has_active_animation() {
            break;
        }
        pipeline.advance_animations(core::time::Duration::from_millis(16));
        pipeline.render();
    }
    assert!(!pipeline.has_active_animation());
    assert_eq!(layouts.load(Ordering::Relaxed), baseline);
    let object = editor(pipeline.element_tree().root().unwrap());
    assert_eq!(object.layout.visual_lines.as_ptr(), storage);
    assert_eq!(object.size, surface);
    assert!(!object.dragging);
    assert_eq!(object.keyboard_bottom, Some(360.0));
    assert_eq!(selection.get(), TextSelection::collapsed(initial.len()));
    assert_eq!(text.get(), initial);
    assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: 'X' })));
    pipeline.render();
    assert_eq!(text.get(), initial + "X");
    assert!(
        pipeline
            .focused_text_input_state()
            .unwrap()
            .cursor_rect
            .bottom()
            <= 360.0
    );
    pipeline.teardown();
}
