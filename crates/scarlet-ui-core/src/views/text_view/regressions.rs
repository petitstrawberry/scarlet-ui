use super::*;
use unicode_segmentation::UnicodeSegmentation;

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
    let state = State::new(StateId::new(7100), document.clone());
    let selection = State::new(StateId::new(7101), TextSelection::collapsed(0));
    let scroll = State::new(StateId::new(7102), TextViewScroll::default());
    let view = TextView::with_document(state, selection)
        .scroll_state(scroll.clone())
        .wrap_mode(WrapMode::Soft);
    let mut object = render_object_with_size(&view, 320.0, 180.0);
    let layout_storage = object.layout.visual_lines.as_ptr();
    let work = crate::testing::alloc_counter::measure_allocations(|| {
        for index in 0..20 {
            scroll.set(TextViewScroll {
                x: 0.0,
                y: index as f32 * 3.25,
            });
            object.update(&view);
            object.layout(LayoutConstraints::tight(320.0, 180.0));
            assert!(object.text_document.same_revision(&document));
            assert_eq!(object.layout.visual_lines.as_ptr(), layout_storage);
        }
    });
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
    let state = State::new(StateId::new(7150), document.clone());
    let view = TextView::with_document(
        state,
        State::new(StateId::new(7151), TextSelection::collapsed(0)),
    )
    .wrap_mode(WrapMode::Soft);
    let mut object = render_object_with_size(&view, 140.0, 90.0);
    let storage = object.layout.visual_lines.as_ptr();
    object.layout(LayoutConstraints::tight(140.0, 230.0));
    assert_eq!(object.layout.visual_lines.as_ptr(), storage);
    let fresh = TextViewLayout::compute(
        &document,
        object.font_size,
        object.padding,
        object.wrap_mode,
        object.size,
        object.scroll,
        false,
    );
    assert_same_layout(&object.layout, &fresh, &document);
}
