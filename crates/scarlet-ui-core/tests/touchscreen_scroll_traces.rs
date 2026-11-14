//! Deterministic native-touch traces, independent of platform input and GPU cost.
use scarlet_ui_core::views::ScrollViewRenderObject;
use scarlet_ui_core::{event::*, pipeline::RenderingPipeline, prelude::*};

fn touch(serial: u64, phase: TouchPhase, x: i32, y: i32) -> Event {
    Event::TouchFrame(TouchFrame {
        seat_id: 0,
        serial,
        time_ns: serial * 16_000_000,
        changes: vec![TouchChange {
            seat_id: 0,
            serial,
            time_ns: serial * 16_000_000,
            id: 7,
            phase,
            x,
            y,
            pressure: None,
            touch_major: None,
        }],
    })
}

fn pipeline(view: impl View + Clone + 'static) -> RenderingPipeline {
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(
        Window::new("Touch trace", view)
            .decorated(false)
            .shadow(false)
            .size(Size::new(100.0, 100.0))
            .create_element(),
    );
    pipeline.layout_initial();
    pipeline.render_for_present().unwrap();
    assert!(!pipeline.has_dirty());
    pipeline
}

fn offset<V: View + 'static>(element: &dyn Element, view: &ScrollView<V>) -> (f32, f32) {
    fn find<V: View + 'static>(element: &dyn Element) -> Option<(f32, f32)> {
        element
            .render_object()
            .and_then(|object| object.as_any().downcast_ref::<ScrollViewRenderObject<V>>())
            .map(|object| object.offset())
            .or_else(|| {
                element
                    .children()
                    .iter()
                    .find_map(|child| find::<V>(child.as_ref()))
            })
    }
    let _ = view; // Infer the concrete content type without exposing it at call sites.
    find::<V>(element).expect("mounted scroll view")
}

#[test]
fn vertical_diagonal_and_reversal_moves_update_before_rendering() {
    // Include horizontal-dominant motion and leaving the hit-test bounds.
    for xs in [
        [50, 50, 50, 50, 50],
        [50, 30, 10, -10, 20],
        [50, 30, 60, -10, 100],
    ] {
        for render_each_move in [false, true] {
            let view = ScrollView::new(Rectangle::new()).content_size(100.0, 1000.0);
            let mut pipeline = pipeline(view.clone());
            pipeline.handle_event(&touch(1, TouchPhase::Down, xs[0], 80));
            for (index, (y, expected)) in [(68, 12.0), (56, 24.0), (44, 36.0), (60, 20.0)]
                .into_iter()
                .enumerate()
            {
                assert!(pipeline.handle_event(&touch(
                    index as u64 + 2,
                    TouchPhase::Move,
                    xs[index + 1],
                    y
                )));
                assert_eq!(
                    offset(pipeline.element_tree().root().unwrap(), &view),
                    (0.0, expected)
                );
                assert!(
                    pipeline.has_dirty(),
                    "movement must invalidate before a render"
                );
                assert!(!pipeline.pipeline_owner().has_dirty_layout());
                if render_each_move {
                    pipeline.render_for_present().unwrap();
                    assert!(!pipeline.has_dirty());
                }
            }
            pipeline.handle_event(&touch(6, TouchPhase::Cancel, xs[4], 60));
            assert!(!pipeline.has_active_animation());
            pipeline.teardown();
        }
    }
}

#[test]
fn nested_horizontal_start_does_not_starve_vertical_diagonal_motion() {
    let inner = ScrollView::new(Rectangle::new()).content_size(100.0, 500.0);
    let outer = ScrollView::new(inner.clone().frame(100.0, 100.0))
        .axes(ScrollAxis::Horizontal)
        .content_size(500.0, 100.0);
    let mut pipeline = pipeline(outer.clone());
    pipeline.handle_event(&touch(1, TouchPhase::Down, 80, 80));
    assert!(pipeline.handle_event(&touch(2, TouchPhase::Move, 60, 80)));
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &outer),
        (20.0, 0.0)
    );
    assert!(pipeline.handle_event(&touch(3, TouchPhase::Move, 59, 60)));
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &inner),
        (0.0, 20.0)
    );
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &outer),
        (20.0, 0.0)
    );
    // Returning to horizontal motion can still hand off to the same ancestor.
    assert!(pipeline.handle_event(&touch(4, TouchPhase::Move, 49, 60)));
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &outer),
        (30.0, 0.0)
    );
    pipeline.handle_event(&touch(5, TouchPhase::Cancel, 49, 60));
    assert!(!pipeline.has_active_animation());
    pipeline.teardown();
}

#[test]
fn nested_initial_edge_handoff_allows_child_to_scroll_after_reversal() {
    let inner = ScrollView::new(Rectangle::new())
        .content_size(100.0, 300.0)
        .scroll_to_index(Some(2), 100.0);
    let outer = ScrollView::new(inner.clone().frame(100.0, 100.0)).content_size(100.0, 500.0);
    let mut pipeline = pipeline(outer.clone());
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &inner),
        (0.0, 200.0)
    );
    pipeline.handle_event(&touch(1, TouchPhase::Down, 50, 80));
    assert!(pipeline.handle_event(&touch(2, TouchPhase::Move, 40, 60)));
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &outer),
        (0.0, 20.0)
    );
    assert!(pipeline.handle_event(&touch(3, TouchPhase::Move, 45, 70)));
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &inner),
        (0.0, 190.0)
    );
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &outer),
        (0.0, 20.0)
    );
    pipeline.handle_event(&touch(4, TouchPhase::Cancel, 45, 70));
    assert!(!pipeline.has_active_animation());
    pipeline.teardown();
}

#[test]
fn child_first_handoff_returns_on_reversal_and_momentum_follows_last_consumer() {
    let inner = ScrollView::new(Rectangle::new()).content_size(100.0, 120.0);
    let outer = ScrollView::new(inner.clone().frame(100.0, 100.0)).content_size(100.0, 500.0);
    let mut pipeline = pipeline(outer.clone());
    pipeline.handle_event(&touch(1, TouchPhase::Down, 50, 80));
    assert!(pipeline.handle_event(&touch(2, TouchPhase::Move, 40, 60)));
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &inner),
        (0.0, 20.0)
    );
    assert!(pipeline.handle_event(&touch(3, TouchPhase::Move, 30, 40)));
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &outer),
        (0.0, 20.0)
    );
    assert!(pipeline.handle_event(&touch(4, TouchPhase::Move, 35, 50)));
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &inner),
        (0.0, 10.0)
    );
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &outer),
        (0.0, 20.0)
    );
    // A final upward move leaves the child as the last consumer. Release starts
    // motion there, even though the ancestor consumed an earlier move.
    assert!(pipeline.handle_event(&touch(5, TouchPhase::Move, 30, 45)));
    pipeline.handle_event(&touch(6, TouchPhase::Up, 30, 45));
    assert!(pipeline.has_active_animation());
    assert!(pipeline.advance_animations(core::time::Duration::from_millis(8)));
    assert!(offset(pipeline.element_tree().root().unwrap(), &inner).1 > 15.0);
    assert_eq!(
        offset(pipeline.element_tree().root().unwrap(), &outer),
        (0.0, 20.0)
    );
    pipeline.handle_event(&touch(7, TouchPhase::Down, 50, 50));
    assert!(
        !pipeline.has_active_animation(),
        "a new contact stops the fling"
    );
    pipeline.handle_event(&touch(8, TouchPhase::Cancel, 50, 50));
    pipeline.teardown();
}

#[test]
fn touch_arena_slop_is_radial_and_scroll_never_relocks_on_reversal() {
    let mut arena = TouchArena::default();
    let change = |serial, phase, x, y| match touch(serial, phase, x, y) {
        Event::TouchFrame(frame) => frame.changes[0],
        _ => unreachable!(),
    };
    arena.process(change(1, TouchPhase::Down, 0, 0), true);
    assert!(
        arena
            .process(change(2, TouchPhase::Move, 5, 5), true)
            .is_empty()
    );
    assert_eq!(
        arena.process(change(3, TouchPhase::Move, 6, 6), true),
        vec![
            TouchGesture::CancelPress { id: 7 },
            TouchGesture::Scroll {
                id: 7,
                phase: GesturePhase::Started,
                delta_x: 6,
                delta_y: 6
            },
        ]
    );
    assert_eq!(
        arena.process(change(4, TouchPhase::Move, 5, 4), true),
        vec![TouchGesture::Scroll {
            id: 7,
            phase: GesturePhase::Moved,
            delta_x: -1,
            delta_y: -2
        },]
    );
    assert_eq!(
        arena.process(change(5, TouchPhase::Up, 5, 4), true),
        vec![TouchGesture::Scroll {
            id: 7,
            phase: GesturePhase::Ended,
            delta_x: 0,
            delta_y: 0
        },]
    );
}
