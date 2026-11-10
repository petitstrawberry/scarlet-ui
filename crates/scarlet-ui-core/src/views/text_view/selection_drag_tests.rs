use super::*;
use crate::{
    element::{Element, ElementId},
    pipeline::RenderingPipeline,
    view::ViewExt,
    views::{HStack, Rectangle, Window},
};
use core::time::Duration;

fn state<T: Clone + 'static>(value: T) -> State<T> {
    State::new(crate::generate_state_id(), value)
}

fn editor(element: &dyn Element, parent: Point) -> Option<(ElementId, Rect)> {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if element
        .render_object()
        .is_some_and(|object| object.as_any().is::<TextViewRenderObject>())
    {
        return Some((element.id(), Rect::new(origin, element.bounds().size)));
    }
    element
        .children()
        .iter()
        .find_map(|child| editor(child.as_ref(), origin))
}

fn mouse(pipeline: &mut RenderingPipeline, event: MouseEvent) {
    pipeline.handle_event(&Event::Mouse(event));
    pipeline.render();
}

#[test]
fn captured_selection_crosses_view_edges_rebuilds_and_stops_on_release_or_blur() {
    for wrap in [WrapMode::None, WrapMode::Soft] {
        let text = state("猫 e\u{301} 👩‍💻 and a long line of text ".repeat(8) + "\n");
        text.set(text.get().repeat(35));
        let initial = text.get();
        let selection = state(TextSelection::collapsed(0));
        let scroll = state(TextViewScroll::default());
        let window = || {
            Window::new(
                "Selection",
                HStack::new((
                    Rectangle::new().frame(80., 160.),
                    TextView::new(text.clone(), selection.clone())
                        .scroll_state(scroll.clone())
                        .wrap_mode(wrap)
                        .font_size(14.)
                        .padding(6.)
                        .line_numbers(true)
                        .frame(260., 160.),
                    Rectangle::new().frame(80., 160.),
                )),
            )
            .decorated(false)
            .size(Size::new(420., 160.))
        };
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_paint_enabled(false);
        pipeline.set_root(window().create_element());
        pipeline.layout_initial();
        pipeline.render();
        let (id, rect) = editor(pipeline.element_tree().root().unwrap(), Point::ZERO).unwrap();
        let object = pipeline
            .element_tree()
            .find_element(id)
            .unwrap()
            .render_object()
            .unwrap()
            .as_any()
            .downcast_ref::<TextViewRenderObject>()
            .unwrap();
        let x = (rect.origin.x + object.layout.text_origin_x + 35.) as i32;
        let y = (rect.origin.y + 45.) as i32;
        let storage = object.layout.visual_lines.as_ptr();
        mouse(
            &mut pipeline,
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                click_count: 1,
            },
        );
        let anchor = selection.get().anchor;
        mouse(
            &mut pipeline,
            MouseEvent::Moved {
                x: rect.origin.x as i32 - 30,
                y,
            },
        );
        assert_eq!(selection.get().anchor, anchor);
        assert!(
            selection.get().caret.byte < anchor.byte,
            "left edge must keep extending selection"
        );
        let outside = (
            (rect.origin.x + rect.size.width + 40.) as i32,
            (rect.origin.y + rect.size.height + 40.) as i32,
        );
        mouse(
            &mut pipeline,
            MouseEvent::Moved {
                x: outside.0,
                y: outside.1,
            },
        );
        assert!(pipeline.has_active_animation());
        let before = selection.get();
        // Hold the pointer still outside, including a normal workbench rebuild.
        pipeline
            .element_tree_mut()
            .root_mut()
            .unwrap()
            .update(&window());
        pipeline.render();
        for _ in 0..8 {
            assert!(pipeline.advance_animations(Duration::from_millis(16)));
            pipeline.render();
        }
        assert!(scroll.get().y > 0.);
        if wrap == WrapMode::None {
            assert!(scroll.get().x > 0.);
        }
        assert!(selection.get().caret.byte > before.caret.byte);
        assert_eq!(selection.get().anchor, anchor);
        assert!(initial.is_char_boundary(selection.get().caret.byte));
        assert!(
            unicode_segmentation::UnicodeSegmentation::grapheme_indices(initial.as_str(), true)
                .any(|(byte, _)| byte == selection.get().caret.byte)
                || selection.get().caret.byte == initial.len()
        );
        let object = pipeline
            .element_tree()
            .find_element(id)
            .unwrap()
            .render_object()
            .unwrap()
            .as_any()
            .downcast_ref::<TextViewRenderObject>()
            .unwrap();
        assert_eq!(
            object.layout.visual_lines.as_ptr(),
            storage,
            "drag must reuse layout"
        );
        // Reverse above and left of the viewport without releasing capture.
        mouse(
            &mut pipeline,
            MouseEvent::Moved {
                x: rect.origin.x as i32 - 50,
                y: rect.origin.y as i32 - 50,
            },
        );
        for _ in 0..200 {
            if !pipeline.has_active_animation() {
                break;
            }
            pipeline.advance_animations(Duration::from_millis(16));
            pipeline.render();
        }
        assert_eq!(scroll.get(), TextViewScroll::default());
        assert_eq!(selection.get().caret.byte, 0);
        assert!(
            !pipeline.has_active_animation(),
            "document edge must become idle"
        );
        mouse(
            &mut pipeline,
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: -50,
                y: -50,
                click_count: 1,
            },
        );
        let released = selection.get();
        mouse(&mut pipeline, MouseEvent::Moved { x, y });
        assert_eq!(selection.get(), released);
        assert!(!pipeline.has_active_animation());
        mouse(
            &mut pipeline,
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                click_count: 1,
            },
        );
        mouse(
            &mut pipeline,
            MouseEvent::Moved {
                x: outside.0,
                y: outside.1,
            },
        );
        assert!(pipeline.has_active_animation());
        pipeline.handle_event(&Event::Focus(FocusEvent::Lost));
        assert!(!pipeline.has_active_animation());
        let blurred = selection.get();
        pipeline.advance_animations(Duration::from_millis(50));
        mouse(&mut pipeline, MouseEvent::Moved { x, y });
        assert_eq!(selection.get(), blurred);
        assert_eq!(text.get(), initial);
        pipeline.teardown();
    }
}

#[test]
fn newline_selection_has_width_for_blank_lines_crlf_and_only_hard_wraps() {
    for (text, range, expected) in [
        ("a\n\nb", 1..3, 2),
        ("a\r\n\r\nb", 2..5, 2),
        ("abc", 3..3, 0),
        ("long text to wrap across several visual rows\n", 0..45, 1),
    ] {
        for wrap in [WrapMode::None, WrapMode::Soft] {
            let document = TextDocument::from_str(text);
            let layout = TextViewLayout::compute(
                &document,
                14.,
                6.,
                wrap,
                Size::new(100., 500.),
                TextViewScroll::default(),
                false,
            );
            let mut ctx = PaintContext::new();
            super::super::paint::paint_selection(
                &mut ctx,
                Point::ZERO,
                &layout,
                &document,
                range.clone(),
                Color::BLUE,
            );
            let rects: Vec<_> = ctx
                .commands()
                .iter()
                .filter_map(|command| match command {
                    PaintCommand::FillPath { path, color } if *color == Color::BLUE => {
                        Some(Rect::from_xywh(
                            path[0].x,
                            path[0].y,
                            path[1].x - path[0].x,
                            path[2].y - path[0].y,
                        ))
                    }
                    _ => None,
                })
                .collect();
            let markers = layout
                .visual_lines
                .iter()
                .filter(|line| {
                    rects.iter().any(|rect| {
                        (rect.origin.y - line.y).abs() < 0.01
                            && rect.right() > line.x + line.width + 0.5
                    })
                })
                .count();
            assert_eq!(markers, expected, "text={text:?} wrap={wrap:?}");
            assert!(rects.iter().all(|rect| rect.size.width >= 1.));
        }
    }
}

#[test]
fn captured_drag_clamps_at_document_end_and_cancellation_stops_motion() {
    let text = state("猫\n\nlast".to_string());
    let selection = state(TextSelection::collapsed(0));
    let view = TextView::new(text.clone(), selection.clone()).padding(4.);
    let mut object = render_object_with_size(&view, 120., 32.);
    handle_text_view_mouse(
        &view,
        &mut object,
        &MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 5,
            y: 5,
            click_count: 1,
        },
    );
    handle_text_view_mouse(&view, &mut object, &MouseEvent::Moved { x: 500, y: 500 });
    for _ in 0..20 {
        advance_text_selection_drag(&view, &mut object, Duration::from_millis(50));
    }
    assert_eq!(selection.get().caret.byte, text.get().len());
    assert!(!object.selection_drag_needs_animation());
    // A final motion/release over empty space below a short document is EOF.
    let mut large = render_object_with_size(&view, 120., 320.);
    handle_text_view_mouse(
        &view,
        &mut large,
        &MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 5,
            y: 5,
            click_count: 1,
        },
    );
    handle_text_view_mouse(&view, &mut large, &MouseEvent::Moved { x: 5, y: 250 });
    assert_eq!(selection.get().caret.byte, text.get().len());
    handle_text_view_mouse(&view, &mut object, &MouseEvent::Moved { x: -100, y: -100 });
    assert!(object.selection_drag_needs_animation());
    handle_text_view_mouse(
        &view,
        &mut object,
        &MouseEvent::ButtonCancelled {
            button: MouseButton::Left,
            x: -100,
            y: -100,
        },
    );
    let stopped = selection.get();
    assert!(!advance_text_selection_drag(
        &view,
        &mut object,
        Duration::from_millis(50)
    ));
    handle_text_view_mouse(&view, &mut object, &MouseEvent::Moved { x: 10, y: 10 });
    assert_eq!(selection.get(), stopped);
}
