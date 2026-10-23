use super::*;
use unicode_segmentation::UnicodeSegmentation;

/// Font discovery in another parallel test legitimately invalidates layout.
/// Cache reuse assertions require an unchanged font epoch, so retry the whole
/// scenario only when that external input changed. A stable epoch must still
/// satisfy every allocation/storage assertion; repeated instability fails.
#[track_caller]
fn with_stable_font_revision<T>(mut scenario: impl FnMut() -> T) -> T {
    for _ in 0..8 {
        let before = graphics::text_font_revision();
        let result = scenario();
        let after = graphics::text_font_revision();
        if before == after {
            return result;
        }
        std::eprintln!("font revision changed during cache scenario: {before} -> {after}");
    }
    panic!("could not measure cache reuse within an unchanged font revision");
}

fn check_document_index(document: &TextDocument, expected: &str) {
    assert_eq!(document.as_str(), expected);
    let mut starts = Vec::new();
    if !expected.is_empty() {
        starts.push(0);
        starts.extend(expected.match_indices('\n').map(|(byte, _)| byte + 1));
    }
    assert_eq!(document.line_count(), starts.len());
    for (index, start) in starts.iter().copied().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(expected.len());
        assert_eq!(document.line_range(index), Some(start..end));
        assert!(matches!(document.line_text(index), Some(Cow::Borrowed(_))));
        assert_eq!(document.line_text(index).unwrap(), &expected[start..end]);
    }
    assert_eq!(document.line_range(starts.len()), None);
    for byte in 0..=expected.len() + 2 {
        let line = expected.as_bytes()[..byte.min(expected.len())]
            .iter()
            .filter(|ch| **ch == b'\n')
            .count();
        assert_eq!(document.line_index_at_byte(byte), line);
        assert_eq!(
            document.clamp_position(TextPosition::new(byte)),
            TextPosition::new(byte).clamp_to_grapheme(expected)
        );
    }
}

#[test]
fn indexed_document_matches_string_after_mixed_unicode_edits() {
    let mut expected = String::from("first\r\n日本 e\u{301} 👩‍💻\n\nlast\n");
    let original = TextDocument::from_str(&expected);
    let original_text = expected.clone();
    let mut document = original.clone();
    assert!(document.same_revision(&original));
    for step in 0..80 {
        check_document_index(&document, &expected);
        let boundaries: Vec<_> = expected
            .char_indices()
            .map(|(byte, _)| byte)
            .chain(core::iter::once(expected.len()))
            .collect();
        let start = boundaries[(step * 7) % boundaries.len()];
        let end = boundaries[(step * 13) % boundaries.len()].max(start);
        let replacement = ["", "é", "\n", "\r\n", "\t", "👩‍💻", "e\u{301}"][step % 7];
        let previous = document.clone();
        let previous_text = expected.clone();
        document = document.replace(start..end, replacement).0;
        expected.replace_range(start..end, replacement);
        check_document_index(&previous, &previous_text);
    }
    check_document_index(&document, &expected);
    check_document_index(&original, &original_text);
    check_document_index(&TextDocument::default(), "");
}

fn assert_same_layout(actual: &TextViewLayout, expected: &TextViewLayout, document: &TextDocument) {
    assert_eq!(actual.visible_lines, expected.visible_lines);
    assert_eq!(actual.content_width, expected.content_width);
    assert_eq!(actual.content_height, expected.content_height);
    assert_eq!(actual.visual_lines.len(), expected.visual_lines.len());
    for (a, b) in actual.visual_lines.iter().zip(&expected.visual_lines) {
        assert_eq!(a.text_range, b.text_range);
        assert_eq!(a.logical_line, b.logical_line);
        assert_eq!(a.text(), b.text());
        assert_eq!(a.width, b.width);
        assert_eq!((a.x, a.y), (b.x, b.y));
        for x in [0.0, a.width * 0.5, a.width + 30.0] {
            let point = Point::new(a.x + x, a.y + actual.line_height * 0.5);
            assert_eq!(actual.hit_test(point), expected.hit_test(point));
        }
    }
    for (byte, _) in document.as_str().grapheme_indices(true) {
        assert_eq!(
            actual.cursor_rect(TextPosition::new(byte), document),
            expected.cursor_rect(TextPosition::new(byte), document)
        );
    }
}

#[test]
fn cached_scroll_geometry_matches_fresh_layout_with_fractional_offsets() {
    let document =
        TextDocument::from_str(&"日本 e\u{301} 👩‍💻\t abcdefghijklmnopqrstuvwxyz  \r\n\n".repeat(20));
    for wrap in [WrapMode::None, WrapMode::Soft] {
        let size = Size::new(180.5, 96.25);
        let mut layout = TextViewLayout::compute(
            &document,
            14.0,
            8.0,
            wrap,
            size,
            TextViewScroll::default(),
            true,
        );
        for requested in [
            TextViewScroll { x: 30.25, y: 40.75 },
            TextViewScroll {
                x: 10000.0,
                y: 10000.0,
            },
            TextViewScroll { x: -5.0, y: -7.0 },
        ] {
            let scroll = layout.set_scroll(requested);
            let fresh = TextViewLayout::compute(&document, 14.0, 8.0, wrap, size, scroll, true);
            assert_same_layout(&layout, &fresh, &document);
        }
    }
}

#[test]
fn soft_wrap_preserves_every_source_space_tab_and_grapheme() {
    let text = "  alpha   beta\t日本e\u{301} 👩‍💻     trailing   ";
    let document = TextDocument::from_str(text);
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::Soft,
        Size::new(73.0, 150.0),
        TextViewScroll::default(),
        false,
    );
    let mut byte = 0;
    for line in &layout.visual_lines {
        assert_eq!(
            line.text_range.start, byte,
            "wrapping must not drop whitespace"
        );
        let source = &text[line.text_range.clone()];
        assert_eq!(line.text(), source.replace('\t', "    "));
        assert_eq!(
            line.width,
            graphics::measure_text_sized(line.text(), 14.0).0 as f32
        );
        assert_eq!(document.clamp_position(TextPosition::new(byte)).byte, byte);
        byte = line.text_range.end;
    }
    assert_eq!(byte, text.len());
}

#[test]
fn cached_caret_widths_match_measurement_at_all_unicode_boundaries() {
    let text = "Wa é e\u{301} 日本\t👩‍💻🇯🇵 finish";
    let document = TextDocument::from_str(text);
    let layout = TextViewLayout::compute(
        &document,
        15.25,
        8.0,
        WrapMode::None,
        Size::new(700.0, 80.0),
        TextViewScroll::default(),
        false,
    );
    for (byte, _) in text
        .char_indices()
        .chain(core::iter::once((text.len(), ' ')))
    {
        let expected =
            graphics::measure_text_sized(&text[..byte].replace('\t', "    "), 15.25).0 as f32;
        assert_eq!(
            layout
                .cursor_rect(TextPosition::new(byte), &document)
                .origin
                .x,
            8.0 + expected
        );
    }
}

#[test]
fn scrolling_and_state_rebuild_reuse_document_and_layout_allocations() {
    let document = TextDocument::from_str(&"line of text e\u{301} 日本\t\n".repeat(1000));
    let (work, retained_storage) = with_stable_font_revision(|| {
        let state = State::new(StateId::new(7100), document.clone());
        let selection = State::new(StateId::new(7101), TextSelection::collapsed(0));
        let scroll = State::new(StateId::new(7102), TextViewScroll::default());
        let view = TextView::with_document(state, selection)
            .scroll_state(scroll.clone())
            .wrap_mode(WrapMode::Soft);
        let mut object = render_object_with_size(&view, 320.0, 180.0);
        let layout_storage = object.layout.visual_lines.as_ptr();
        let mut retained_storage = true;
        // macOS lazily allocates the subscriber mutex on its first lock.
        // Initialize notification before measuring steady-state UI work.
        scroll.set(TextViewScroll::default());
        let work = crate::testing::alloc_counter::measure_allocations(|| {
            for index in 0..20 {
                scroll.set(TextViewScroll {
                    x: 0.0,
                    y: index as f32 * 3.25,
                });
                object.update(&view);
                object.layout(LayoutConstraints::tight(320.0, 180.0));
                assert!(object.text_document.same_revision(&document));
                retained_storage &= object.layout.visual_lines.as_ptr() == layout_storage;
            }
        });
        (work, retained_storage)
    });
    assert!(
        retained_storage,
        "unchanged-font scrolling rebuilt measured rows"
    );
    assert_eq!(
        work.allocations, 0,
        "scrolling a shared document must not rebuild or copy text"
    );
}

#[test]
fn layout_invalidates_for_text_width_font_padding_and_wrap_changes() {
    let text = State::new(
        StateId::new(7110),
        String::from("alpha beta 日本 e\u{301} end\nnext"),
    );
    let selection = State::new(StateId::new(7111), TextSelection::collapsed(0));
    let mut view = TextView::new(text.clone(), selection);
    let mut object = render_object_with_size(&view, 190.0, 120.0);
    for step in 0..5 {
        match step {
            0 => view = view.font_size(19.0),
            1 => view = view.padding(15.0),
            2 => view = view.wrap_mode(WrapMode::Soft).line_numbers(true),
            3 => text.set("changed\r\n日本\tend\n".repeat(4)),
            _ => {}
        }
        object.update(&view);
        object.layout(LayoutConstraints::tight(90.0 + step as f32 * 13.0, 120.0));
        let expected = TextViewLayout::compute(
            &object.text_document,
            object.font_size,
            object.padding,
            object.wrap_mode,
            object.size,
            object.scroll,
            object.line_numbers,
        );
        assert_same_layout(&object.layout, &expected, &object.text_document);
    }
}

#[test]
fn external_selection_reveal_does_not_override_manual_scrolling_or_ime() {
    let text = State::new(StateId::new(7120), "日本 text\n".repeat(50));
    let selection = State::new(StateId::new(7121), TextSelection::collapsed(0));
    let scroll = State::new(StateId::new(7122), TextViewScroll::default());
    let view = TextView::new(text.clone(), selection.clone())
        .scroll_state(scroll.clone())
        .reveal_selection_on_change(true)
        .autofocus(true);
    let mut object = render_object_with_size(&view, 200.0, 90.0);
    object.set_preedit_state("入力", 6, 3, &[]);
    selection.set(TextSelection::collapsed(text.get().len()));
    object.update(&view);
    assert!(object.scroll.y > 0.0);
    assert!(object.cursor_rect().origin.y < object.size.height);
    assert_eq!(object.preedit(), "入力");
    assert!(object.is_focused());
    scroll.set(TextViewScroll::default());
    object.update(&view);
    assert_eq!(object.scroll.y, 0.0);
    assert!(object.cursor_rect().origin.y > object.size.height);
    object.set_focused(false);
    object.update(&view);
    assert!(
        !object.is_focused(),
        "an unchanged autofocus value must preserve user blur"
    );
    assert!(object.preedit().is_empty());
}

#[test]
fn composition_cursor_reveals_without_rewrapping_or_mutating_text() {
    for wrap in [WrapMode::None, WrapMode::Soft] {
        let text = "abc 日本 e\u{301} 👩‍💻 ".repeat(4);
        let view = TextView::new(
            State::new(StateId::new(7123), text.clone()),
            State::new(StateId::new(7124), TextSelection::collapsed(text.len())),
        )
        .wrap_mode(wrap)
        .autofocus(true);
        let mut object = render_object_with_size(&view, 160.0, 90.0);
        let rows = object.layout.visual_lines.as_ptr();
        let ranges: Vec<_> = object
            .layout
            .visual_lines
            .iter()
            .map(|line| line.text_range.clone())
            .collect();
        let preedit = "日本語入力中".repeat(8);
        assert!(handle_text_view_text_input(
            &view,
            &mut object,
            &Event::TextInputPreedit {
                context_id: 1,
                serial: 1,
                cursor_byte: preedit.len() as u32,
                anchor_byte: preedit.len() as u32,
                text: preedit,
                spans: Vec::new(),
            }
        ));
        let cursor = object.text_input_state().cursor_rect;
        assert!(object.scroll.x > 0.0);
        assert!(cursor.origin.x >= object.layout.text_origin_x);
        assert!(cursor.origin.x + cursor.size.width <= object.size.width - object.padding);
        assert!(cursor.origin.y >= object.padding);
        assert!(cursor.origin.y + cursor.size.height <= object.size.height - object.padding);
        assert_eq!(object.text_document.as_str(), text);
        assert_eq!(object.layout.visual_lines.as_ptr(), rows);
        assert_eq!(
            object
                .layout
                .visual_lines
                .iter()
                .map(|line| line.text_range.clone())
                .collect::<Vec<_>>(),
            ranges
        );
        object.clear_preedit();
        assert_eq!(object.scroll, object.layout.clamp_scroll(object.scroll));
        if wrap == WrapMode::Soft {
            assert_eq!(object.scroll.x, 0.0);
            assert!(
                object
                    .layout
                    .visual_lines
                    .iter()
                    .all(|line| line.x == object.layout.text_origin_x)
            );
        }
    }
}

#[test]
fn wheel_at_bounds_or_with_zero_delta_does_not_claim_a_change() {
    let view = TextView::new(
        State::new(StateId::new(7130), String::from("short")),
        State::new(StateId::new(7131), TextSelection::collapsed(0)),
    );
    let mut object = render_object_with_size(&view, 200.0, 90.0);
    for delta in [-100, 0, 100] {
        assert!(!handle_text_view_mouse(
            &view,
            &mut object,
            &MouseEvent::Wheel {
                delta_x: delta,
                delta_y: delta,
                x: 30,
                y: 30,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad
            }
        ));
    }
}

#[test]
fn mounted_pipeline_scroll_rebuild_keeps_measured_layout_and_keyboard_focus() {
    use crate::pipeline::RenderingPipeline;
    let document = TextDocument::from_str(&"日本 text\n".repeat(200));
    let value = State::new(StateId::new(7140), document.clone());
    let selection = State::new(StateId::new(7141), TextSelection::collapsed(0));
    let scroll = State::new(StateId::new(7142), TextViewScroll::default());
    let view = TextView::with_document(value.clone(), selection)
        .scroll_state(scroll.clone())
        .autofocus(true);
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_paint_enabled(false);
    pipeline.set_root(view.create_element());
    pipeline.layout_initial();
    pipeline.resize(Size::new(320.0, 180.0));
    pipeline.render();
    let object = |pipeline: &RenderingPipeline| {
        pipeline
            .element_tree()
            .root()
            .unwrap()
            .as_any()
            .downcast_ref::<RenderElement<TextView, TextViewRenderObject>>()
            .unwrap()
            .render_object()
            .layout
            .visual_lines
            .as_ptr()
    };
    let layout_storage = object(&pipeline);
    assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
        delta_x: 0,
        delta_y: -55,
        x: 40,
        y: 40,
        phase: WheelPhase::Moved,
        source: ScrollSource::Trackpad,
    })));
    pipeline.render();
    assert!(scroll.get().y > 0.0);
    assert_eq!(object(&pipeline), layout_storage);
    assert!(value.get().same_revision(&document));
    assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: 'X' })));
    pipeline.render();
    assert!(value.get().as_str().starts_with('X'));
    pipeline.teardown();
}

#[test]
fn viewport_height_changes_reuse_metrics_and_refresh_visible_range() {
    let document = TextDocument::from_str(&"one two three four five\n".repeat(30));
    let (object, fresh, retained_storage) = with_stable_font_revision(|| {
        let state = State::new(StateId::new(7150), document.clone());
        let view = TextView::with_document(
            state,
            State::new(StateId::new(7151), TextSelection::collapsed(0)),
        )
        .wrap_mode(WrapMode::Soft);
        let mut object = render_object_with_size(&view, 140.0, 90.0);
        let storage = object.layout.visual_lines.as_ptr();
        object.layout(LayoutConstraints::tight(140.0, 230.0));
        let retained_storage = object.layout.visual_lines.as_ptr() == storage;
        let fresh = TextViewLayout::compute(
            &document,
            object.font_size,
            object.padding,
            object.wrap_mode,
            object.size,
            object.scroll,
            false,
        );
        (object, fresh, retained_storage)
    });
    assert!(
        retained_storage,
        "height change rebuilt unchanged-font metrics"
    );
    assert_same_layout(&object.layout, &fresh, &document);
}

#[test]
fn selection_drag_keeps_scrolled_editor_geometry_without_ancestor_layout() {
    use crate::pipeline::RenderingPipeline;
    use crate::state::generate_state_id;
    use crate::view::ViewExt;
    use crate::views::{Text, VStack, Window};
    use core::sync::atomic::{AtomicUsize, Ordering};

    let selection = State::new(generate_state_id(), TextSelection::collapsed(0));
    let scroll = State::new(generate_state_id(), TextViewScroll::default());
    let layouts = Arc::new(AtomicUsize::new(0));
    let count = layouts.clone();
    let view = TextView::new(
        State::new(
            generate_state_id(),
            "日本 long editor line ".repeat(30) + &"\nrow".repeat(100),
        ),
        selection.clone(),
    )
    .line_numbers(true)
    .wrap_mode(WrapMode::None)
    .scroll_state(scroll.clone())
    .reveal_selection_on_change(true)
    .on_geometry_change(
        move |geometry| {
            count.fetch_add(1, Ordering::Relaxed);
            geometry.size()
        },
        |_| {},
    );
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_paint_enabled(false);
    pipeline.set_root(
        Window::new("Selection", VStack::new((view, Text::new("status"))))
            .decorated(false)
            .size(Size::new(320.0, 180.0))
            .create_element(),
    );
    pipeline.layout_initial();
    pipeline.render();
    scroll.set(TextViewScroll { x: 0.0, y: 60.0 });
    pipeline.render();
    let baseline = layouts.load(Ordering::Relaxed);
    let initial_scroll = scroll.get();
    assert!(
        pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 55,
            y: 40,
            click_count: 1,
        }))
    );
    let anchor = selection.get().anchor;
    for y in [45, 55, 65, 55, 45] {
        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 65, y })));
        pipeline.render();
        assert_eq!(selection.get().anchor, anchor);
        assert_eq!(scroll.get(), initial_scroll);
        assert_eq!(
            layouts.load(Ordering::Relaxed),
            baseline,
            "selection must not re-run ancestor measurement passes"
        );
    }
    pipeline.teardown();
}

#[test]
fn hover_reports_scrolled_unicode_byte_and_leave_without_selection_or_scroll_changes() {
    let hits = Arc::new(Mutex::new(Vec::new()));
    let observed = hits.clone();
    let selection = State::new(StateId::new(7160), TextSelection::collapsed(0));
    let view = TextView::new(
        State::new(
            StateId::new(7161),
            "日本 e\u{301} 👩‍💻 long line\n".repeat(20),
        ),
        selection.clone(),
    )
    .line_numbers(true)
    .on_hover(move |hit| observed.lock().unwrap().push(hit));
    let mut object = render_object_with_size(&view, 180.0, 90.0);
    object.requested_scroll = TextViewScroll {
        x: 10.0,
        y: object.layout.line_height * 2.0 + 4.0,
    };
    object.compute_layout();
    let expected_byte = "日本 e\u{301} 👩‍💻 long line\n".repeat(3).len() + "日本 ".len();
    let caret = object
        .layout
        .cursor_rect(TextPosition::new(expected_byte), &object.text_document);
    let point = Point::new(
        caret.origin.x,
        (caret.origin.y + caret.size.height / 2.0).round(),
    );
    assert!(point.x > object.layout.text_origin_x);
    let initial_scroll = object.scroll;
    assert!(!handle_text_view_mouse(
        &view,
        &mut object,
        &MouseEvent::Moved {
            x: point.x as i32,
            y: point.y as i32,
        }
    ));
    assert_eq!(
        hits.lock().unwrap().last().copied().flatten(),
        Some(TextViewHit {
            byte_offset: expected_byte,
            position: point,
        })
    );
    for event in [
        MouseEvent::Moved { x: 0, y: 40 },
        MouseEvent::Exited { x: 500, y: 40 },
    ] {
        handle_text_view_mouse(&view, &mut object, &event);
        assert_eq!(hits.lock().unwrap().last(), Some(&None));
    }
    assert_eq!(selection.get(), TextSelection::collapsed(0));
    assert_eq!(object.scroll, initial_scroll);
    assert!(!object.dragging);
}

#[test]
fn caret_geometry_binding_tracks_layout_selection_scroll_and_ime_only_when_changed() {
    use crate::geometry::Rect;
    use crate::state::{Listenable, generate_state_id};
    use core::sync::atomic::{AtomicUsize, Ordering};
    let rect = State::new(generate_state_id(), Rect::from_xywh(0.0, 0.0, 0.0, 0.0));
    let notifications = Arc::new(AtomicUsize::new(0));
    let count = notifications.clone();
    rect.subscribe_any(Arc::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
    }));
    let selection = State::new(generate_state_id(), TextSelection::collapsed(0));
    let scroll = State::new(generate_state_id(), TextViewScroll::default());
    let view = TextView::new(
        State::new(generate_state_id(), "日本 text\n".repeat(30)),
        selection.clone(),
    )
    .caret_rect_state(rect.clone())
    .scroll_state(scroll.clone())
    .autofocus(true);
    let mut object = render_object_with_size(&view, 200.0, 90.0);
    object.flush_layout_notifications();
    assert_eq!(rect.get(), object.cursor_rect());
    let baseline = notifications.load(Ordering::Relaxed);
    for _ in 0..5 {
        object.update(&view);
        object.layout(LayoutConstraints::tight(200.0, 90.0));
    }
    assert_eq!(notifications.load(Ordering::Relaxed), baseline);
    // Parent stacks may measure a temporary larger viewport before assigning
    // the final one. Observers must see only the completed layout geometry.
    object.layout(LayoutConstraints::tight(200.0, 300.0));
    object.layout(LayoutConstraints::tight(200.0, 90.0));
    object.flush_layout_notifications();
    assert_eq!(notifications.load(Ordering::Relaxed), baseline);
    handle_text_view_keyboard(&view, &mut object, key(KeyCode::Right));
    assert_eq!(rect.get(), object.cursor_rect());
    assert!(rect.get().origin.x > object.padding);
    selection.set(TextSelection::collapsed("日本 text\n".repeat(3).len()));
    object.update(&view);
    assert_eq!(rect.get(), object.cursor_rect());
    let before_scroll = rect.get();
    scroll.set(TextViewScroll { x: 0.0, y: 10.0 });
    object.update(&view);
    assert_eq!(rect.get().origin.y, before_scroll.origin.y - 10.0);
    object.set_preedit_state("入力", 6, 3, &[]);
    assert_eq!(rect.get(), object.text_input_state().cursor_rect);
    object.clear_preedit();
    assert_eq!(rect.get(), object.cursor_rect());
}

#[test]
fn borderless_document_surface_has_square_fill_and_no_control_outline() {
    let view = TextView::new(
        State::new(StateId::new(7190), "code".into()),
        State::new(StateId::new(7191), TextSelection::collapsed(0)),
    )
    .borderless()
    .autofocus(true);
    let object = render_object_with_size(&view, 240.0, 120.0);
    let mut context = PaintContext::new();
    assert!(object.paint(&mut context, Point::ZERO));
    assert!(
        !context.commands().iter().any(|command| matches!(
            command,
            PaintCommand::FillRoundedRect { .. } | PaintCommand::StrokeRoundedRect { .. }
        )),
        "a document pane must not paint a rounded form control"
    );
    let regular = render_object_with_size(
        &TextView::new(
            view.text_state().unwrap().clone(),
            view.selection_state().clone(),
        ),
        240.0,
        120.0,
    );
    assert_eq!(object.cursor_rect(), regular.cursor_rect());
}

#[test]
fn parent_measurement_probes_do_not_reset_the_committed_editor_scroll() {
    let view = TextView::new(
        State::new(
            StateId::new(7192),
            "long line ".repeat(160) + &"\nlast".repeat(60),
        ),
        State::new(StateId::new(7193), TextSelection::collapsed(0)),
    );
    let mut object = render_object_with_size(&view, 240.0, 120.0);
    handle_text_view_mouse(
        &view,
        &mut object,
        &MouseEvent::Wheel {
            delta_x: -800,
            delta_y: -1500,
            x: 100,
            y: 60,
            phase: WheelPhase::Moved,
            source: ScrollSource::Trackpad,
        },
    );
    let committed = object.scroll;
    assert!(committed.x > 0.0 && committed.y > 0.0);
    object.layout(LayoutConstraints::tight(1600.0, 1400.0));
    object.layout(LayoutConstraints::tight(240.0, 120.0));
    object.flush_layout_notifications();
    assert_eq!(
        object.scroll, committed,
        "intrinsic measurement must not commit a temporary viewport's scroll clamp"
    );
}

#[test]
fn selection_drag_cancels_hover_and_does_not_schedule_more_hover_requests() {
    let hits = Arc::new(Mutex::new(Vec::new()));
    let observed = hits.clone();
    let view = TextView::new(
        State::new(StateId::new(7194), "alpha beta gamma".into()),
        State::new(StateId::new(7195), TextSelection::collapsed(0)),
    )
    .on_hover(move |hit| observed.lock().unwrap().push(hit));
    let mut object = render_object_with_size(&view, 300.0, 120.0);
    handle_text_view_mouse(&view, &mut object, &MouseEvent::Moved { x: 25, y: 20 });
    assert!(hits.lock().unwrap().last().unwrap().is_some());
    handle_text_view_mouse(
        &view,
        &mut object,
        &MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 25,
            y: 20,
            click_count: 1,
        },
    );
    assert_eq!(hits.lock().unwrap().last(), Some(&None));
    let count = hits.lock().unwrap().len();
    for x in [45, 65, 85] {
        handle_text_view_mouse(&view, &mut object, &MouseEvent::Moved { x, y: 20 });
    }
    assert_eq!(hits.lock().unwrap().len(), count);
    assert_ne!(view.selection.get().anchor, view.selection.get().caret);
    handle_text_view_mouse(
        &view,
        &mut object,
        &MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: 85,
            y: 20,
            click_count: 1,
        },
    );
    handle_text_view_mouse(&view, &mut object, &MouseEvent::Moved { x: 90, y: 20 });
    assert_eq!(hits.lock().unwrap().len(), count + 1);
}
