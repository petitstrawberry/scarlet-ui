use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;
use std::sync::{Arc, Mutex};

use crate::color::ColorPalette;
use crate::element::{ElementRenderObject, LayoutConstraints};
use crate::event::{Event, FocusEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent};
use crate::renderer::PaintCommand;
use crate::state::{State, StateId};
use crate::view::View;
use crate::{geometry::Point, graphics};

use super::*;

#[test]
fn document_round_trips_full_text() {
    for text in ["", "hello", "こんにちは", "😀😃😄", "hello こんにちは 👋"] {
        let document = TextDocument::from_str(text);
        assert_eq!(document.as_str(), Cow::Borrowed(text));
        assert_eq!(document.len(), text.len());
        assert_eq!(document.is_empty(), text.is_empty());
    }
}

#[test]
fn insert_handles_start_middle_end_and_multibyte() {
    let document = TextDocument::from_str("world");
    let (document, delta) = document.insert(TextPosition::new(0), "hello ");
    assert_eq!(document.as_str(), Cow::Borrowed("hello world"));
    assert_eq!(delta.replaced_range, 0..0);
    assert_eq!(delta.inserted, "hello ");
    assert!(delta.is_insertion());

    let middle = "hello ".len();
    let (document, delta) = document.insert(TextPosition::new(middle), "美しい ");
    assert_eq!(document.as_str(), Cow::Borrowed("hello 美しい world"));
    assert_eq!(delta.replaced_range, middle..middle);

    let end = document.len();
    let (document, delta) = document.insert(TextPosition::new(end), " 🌍");
    assert_eq!(document.as_str(), Cow::Borrowed("hello 美しい world 🌍"));
    assert_eq!(delta.replaced_range, end..end);
}

#[test]
fn delete_handles_ranges_and_edges() {
    let document = TextDocument::from_str("abcdef");
    let (unchanged, delta) = document.delete(2..2);
    assert_eq!(unchanged.as_str(), Cow::Borrowed("abcdef"));
    assert_eq!(delta.replaced_range, 2..2);
    assert!(!delta.is_deletion());

    let (document, delta) = document.delete(1..4);
    assert_eq!(document.as_str(), Cow::Borrowed("aef"));
    assert_eq!(delta.deleted_text("abcdef"), "bcd");
    assert!(delta.is_deletion());

    let document = TextDocument::from_str("こんにちは");
    let (document, delta) = document.delete(0.."こんにちは".len());
    assert_eq!(document.as_str(), Cow::Borrowed(""));
    assert_eq!(delta.deleted_text("こんにちは"), "こんにちは");
}

#[test]
fn replace_handles_shorter_and_longer_text() {
    let document = TextDocument::from_str("hello world");
    let (shorter, delta) = document.replace(6..11, "UI");
    assert_eq!(shorter.as_str(), Cow::Borrowed("hello UI"));
    assert_eq!(delta.deleted_text("hello world"), "world");
    assert_eq!(delta.inserted, "UI");
    assert!(delta.is_replacement());

    let (longer, delta) = shorter.replace(6..8, "ScarletUI text view");
    assert_eq!(longer.as_str(), Cow::Borrowed("hello ScarletUI text view"));
    assert_eq!(delta.deleted_text("hello UI"), "UI");
    assert!(delta.is_replacement());
}

#[test]
fn piece_table_handles_repeated_splits_and_deletions() {
    let document = TextDocument::from_str("ace");
    let (document, _) = document.insert(TextPosition::new(1), "b");
    let (document, _) = document.insert(TextPosition::new(3), "d");
    let (document, delta) = document.replace(1..4, "BCD");
    assert_eq!(document.as_str(), Cow::Borrowed("aBCDe"));
    assert_eq!(delta.replaced_range, 1..4);
    assert_eq!(delta.deleted_text("abcde"), "bcd");

    let (document, delta) = document.delete(2..4);
    assert_eq!(document.as_str(), Cow::Borrowed("aBe"));
    assert_eq!(delta.replaced_range, 2..4);
}

#[test]
fn line_count_matches_text_cases() {
    assert_eq!(TextDocument::from_str("").line_count(), 0);
    assert_eq!(TextDocument::from_str("one").line_count(), 1);
    assert_eq!(TextDocument::from_str("one\ntwo\nthree").line_count(), 3);
    assert_eq!(TextDocument::from_str("one\n").line_count(), 2);
}

#[test]
fn line_range_returns_byte_ranges() {
    let document = TextDocument::from_str("a\n日本\nemoji 😀");
    assert_eq!(document.line_range(0), Some(0..2));
    assert_eq!(document.line_text(0), Some(Cow::Borrowed("a\n")));

    let second_start = "a\n".len();
    let second_end = second_start + "日本\n".len();
    assert_eq!(document.line_range(1), Some(second_start..second_end));
    assert_eq!(document.line_text(1), Some(Cow::Borrowed("日本\n")));

    assert_eq!(document.line_range(2), Some(second_end..document.len()));
    assert_eq!(document.line_text(2), Some(Cow::Borrowed("emoji 😀")));
    assert_eq!(document.line_range(3), None);
}

#[test]
fn text_position_clamps_to_grapheme_boundaries() {
    let text = "éclair";
    assert_eq!(TextPosition::new(1).clamp_to_grapheme(text).byte, 0);

    let family = "👨‍👩‍👧‍👦!";
    assert_eq!(TextPosition::new(5).clamp_to_grapheme(family).byte, 0);
    assert_eq!(
        TextPosition::new("👨‍👩‍👧‍👦".len() - 1)
            .clamp_to_grapheme(family)
            .byte,
        "👨‍👩‍👧‍👦".len()
    );
}

#[test]
fn selection_normalizes_ranges() {
    let collapsed = TextSelection::collapsed(4);
    assert!(collapsed.is_collapsed());
    assert_eq!(collapsed.normalized_range(), None);
    assert_eq!(collapsed.start(), TextPosition::new(4));
    assert_eq!(collapsed.end(), TextPosition::new(4));

    let forward = TextSelection {
        anchor: TextPosition::new(2),
        caret: TextPosition::new(8),
    };
    assert!(!forward.is_collapsed());
    assert_eq!(forward.normalized_range(), Some(2..8));
    assert_eq!(forward.start(), TextPosition::new(2));
    assert_eq!(forward.end(), TextPosition::new(8));

    let backward = TextSelection {
        anchor: TextPosition::new(8),
        caret: TextPosition::new(2),
    };
    assert_eq!(backward.normalized_range(), Some(2..8));
    assert_eq!(backward.start(), TextPosition::new(2));
    assert_eq!(backward.end(), TextPosition::new(8));
}

#[test]
fn edit_delta_classifies_edits() {
    let insertion = EditDelta {
        replaced_range: 3..3,
        inserted: String::from("x"),
    };
    assert!(insertion.is_insertion());
    assert!(!insertion.is_deletion());
    assert!(!insertion.is_replacement());

    let deletion = EditDelta {
        replaced_range: 1..2,
        inserted: String::new(),
    };
    assert!(!deletion.is_insertion());
    assert!(deletion.is_deletion());
    assert!(!deletion.is_replacement());

    let replacement = EditDelta {
        replaced_range: 1..2,
        inserted: String::from("y"),
    };
    assert!(!replacement.is_insertion());
    assert!(!replacement.is_deletion());
    assert!(replacement.is_replacement());
}

#[test]
fn edits_clamp_invalid_utf8_byte_offsets() {
    let document = TextDocument::from_str("éa😀");

    let (inserted, delta) = document.insert(TextPosition::new(1), "X");
    assert_eq!(inserted.as_str(), Cow::Borrowed("Xéa😀"));
    assert_eq!(delta.replaced_range, 0..0);

    let (deleted, delta) = document.delete(1..2);
    assert_eq!(deleted.as_str(), Cow::Borrowed("a😀"));
    assert_eq!(delta.replaced_range, 0..2);
    assert_eq!(delta.deleted_text("éa😀"), "é");

    let emoji_start = "éa".len();
    let (replaced, delta) = document.replace(emoji_start + 1..document.len(), "!");
    assert_eq!(replaced.as_str(), Cow::Borrowed("éa!"));
    assert_eq!(delta.replaced_range, emoji_start..document.len());
}

#[test]
fn char_byte_conversion_clamps_safely() {
    let document = TextDocument::from_str("aé日");
    assert_eq!(document.char_to_byte(0), 0);
    assert_eq!(document.char_to_byte(1), 1);
    assert_eq!(document.char_to_byte(2), 3);
    assert_eq!(document.char_to_byte(99), document.len());

    assert_eq!(document.byte_to_char(0), 0);
    assert_eq!(document.byte_to_char(2), 1);
    assert_eq!(document.byte_to_char(99), 3);
}

#[test]
fn text_view_constructors_expose_expected_states() {
    let text = State::new(StateId::new(100), String::from("hello"));
    let selection = State::new(StateId::new(101), TextSelection::collapsed(0));
    let view = TextView::new(text.clone(), selection.clone())
        .placeholder("Type")
        .font_size(16.0)
        .padding(4.0)
        .tab_mode(TabMode::Spaces(4))
        .line_numbers(true)
        .current_line_highlight(true)
        .wrap_mode(WrapMode::Soft)
        .on_copy(|_| {})
        .on_paste(|| Some(Cow::Borrowed("paste")))
        .on_text_change(|_| {});
    assert!(view.text_state().is_some());
    assert!(view.document_state().is_none());
    assert_eq!(view.selection_state().id(), selection.id());
    assert_eq!(view.listenables().len(), 2);

    let document = State::new(StateId::new(102), TextDocument::from_str("hello"));
    let scroll = State::new(StateId::new(103), TextViewScroll { x: 1.0, y: 2.0 });
    let wrap = State::new(StateId::new(104), WrapMode::Soft);
    let view = TextView::with_document(document, selection)
        .scroll_state(scroll)
        .wrap_mode_state(wrap);
    assert!(view.text_state().is_none());
    assert!(view.document_state().is_some());
    assert_eq!(view.listenables().len(), 4);
}

#[test]
fn text_view_default_selection_color_uses_palette_primary() {
    let text = State::new(StateId::new(1040), String::new());
    let selection = State::new(StateId::new(1041), TextSelection::collapsed(0));
    let view = TextView::new(text, selection);

    assert_eq!(
        view.selection_color,
        ColorPalette::default().primary().with_opacity(0.3)
    );
}

#[test]
fn text_view_layout_basic_empty_document_is_valid() {
    let document = TextDocument::new();
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(200.0, 120.0),
        TextViewScroll::default(),
        false,
    );
    assert_eq!(layout.visual_lines.len(), 1);
    assert_eq!(layout.content_height, layout.line_height);
    assert_eq!(layout.visible_lines, 0..1);
}

#[test]
fn text_view_layout_with_text_tracks_logical_lines() {
    let document = TextDocument::from_str("one\ntwo\nthree");
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(240.0, 160.0),
        TextViewScroll::default(),
        false,
    );
    assert_eq!(layout.visual_lines.len(), 3);
    assert_eq!(layout.visual_lines[0].logical_line, 0);
    assert_eq!(layout.visual_lines[2].logical_line, 2);
}

#[test]
fn text_view_line_height_uses_graphics_line_advance() {
    let document = TextDocument::from_str("one\ntwo");
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(200.0, 120.0),
        TextViewScroll::default(),
        false,
    );

    assert_eq!(layout.line_height, graphics::line_height_sized(14.0) as f32);
}

#[test]
fn wrap_none_keeps_one_visual_line_per_logical_line() {
    let document = TextDocument::from_str("short\na much longer line");
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(80.0, 120.0),
        TextViewScroll::default(),
        false,
    );
    let expected_width = graphics::measure_text_sized("a much longer line", 14.0).0 as f32;
    assert_eq!(layout.visual_lines.len(), 2);
    assert_eq!(layout.max_line_width, expected_width);
}

#[test]
fn layout_expands_tabs_for_display_but_keeps_document_ranges() {
    let document = TextDocument::from_str("a\tb");
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(240.0, 80.0),
        TextViewScroll::default(),
        false,
    );

    let line = &layout.visual_lines[0];
    assert_eq!(line.text(), "a    b");
    assert_eq!(line.text_range, 0.."a\tb".len());
    let expected_width = graphics::measure_text_sized("a    b", 14.0).0 as f32;
    assert_eq!(line.width, expected_width);

    let rect = layout.cursor_rect(TextPosition::new("a\t".len()), &document);
    let expected_x = 8.0 + graphics::measure_text_sized("a    ", 14.0).0 as f32;
    assert_eq!(rect.origin.x, expected_x);
}

#[test]
fn soft_wrap_splits_long_lines_to_content_width() {
    let document = TextDocument::from_str("alpha beta gamma delta epsilon zeta");
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::Soft,
        Size::new(110.0, 200.0),
        TextViewScroll::default(),
        false,
    );
    assert!(layout.visual_lines.len() > 1);
    for line in &layout.visual_lines {
        assert!(line.width <= layout.content_width);
    }
}

#[test]
fn scroll_clamping_stays_inside_content_bounds() {
    let document = TextDocument::from_str("one\ntwo\nthree\nfour\nfive\nsix\nseven");
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(80.0, 50.0),
        TextViewScroll::default(),
        false,
    );
    let clamped = layout.clamp_scroll(TextViewScroll {
        x: 10_000.0,
        y: 10_000.0,
    });
    assert!(clamped.x <= (layout.content_width - layout.viewport_width).max(0.0));
    assert!(clamped.y <= (layout.content_height - layout.viewport_height).max(0.0));
    assert_eq!(
        layout.clamp_scroll(TextViewScroll { x: -5.0, y: -8.0 }),
        TextViewScroll::default()
    );
}

#[test]
fn visible_range_tracks_viewport_scroll() {
    let document = TextDocument::from_str("0\n1\n2\n3\n4\n5\n6\n7\n8\n9");
    let base = TextViewLayout::compute(
        &document,
        10.0,
        0.0,
        WrapMode::None,
        Size::new(100.0, 24.0),
        TextViewScroll::default(),
        false,
    );
    let scrolled = TextViewLayout::compute(
        &document,
        10.0,
        0.0,
        WrapMode::None,
        Size::new(100.0, 24.0),
        TextViewScroll {
            x: 0.0,
            y: base.line_height * 4.0,
        },
        false,
    );
    assert!(base.visible_lines.start < scrolled.visible_lines.start);
    assert!(scrolled.visible_lines.end < scrolled.visual_lines.len() + 1);
}

#[test]
fn visible_range_contains_only_rows_intersecting_viewport() {
    let document = TextDocument::from_str("0\n1\n2\n3\n4\n5\n6\n7\n8\n9");
    let base = TextViewLayout::compute(
        &document,
        10.0,
        0.0,
        WrapMode::None,
        Size::new(100.0, 100.0),
        TextViewScroll::default(),
        false,
    );
    let line_height = base.line_height;
    let viewport_height = line_height * 3.0;
    let layout = TextViewLayout::compute(
        &document,
        10.0,
        0.0,
        WrapMode::None,
        Size::new(100.0, viewport_height),
        TextViewScroll {
            x: 0.0,
            y: line_height * 4.0,
        },
        false,
    );

    assert_eq!(layout.visible_lines, 4..7);
    for (index, line) in layout.visual_lines.iter().enumerate() {
        let intersects_viewport =
            line.y < 24.0 - BORDER_WIDTH && line.y + layout.line_height > BORDER_WIDTH;
        assert_eq!(
            layout.visible_lines.contains(&index),
            intersects_viewport,
            "line {index}: y={} line_height={} viewport={}..{}",
            line.y,
            layout.line_height,
            BORDER_WIDTH,
            24.0 - BORDER_WIDTH
        );
    }
}

#[test]
fn visible_range_uses_border_clip_not_padding_clip() {
    let document = TextDocument::from_str("0\n1\n2\n3\n4\n5\n6\n7\n8\n9");
    let base = TextViewLayout::compute(
        &document,
        10.0,
        16.0,
        WrapMode::None,
        Size::new(100.0, 100.0),
        TextViewScroll::default(),
        false,
    );
    let line_height = base.line_height;
    let height = 16.0 * 2.0 + line_height * 3.0;
    let layout = TextViewLayout::compute(
        &document,
        10.0,
        16.0,
        WrapMode::None,
        Size::new(100.0, height),
        TextViewScroll {
            x: 0.0,
            y: line_height * 4.0,
        },
        false,
    );

    let expected_start = layout
        .visual_lines
        .iter()
        .position(|line| {
            line.y < height - BORDER_WIDTH && line.y + layout.line_height > BORDER_WIDTH
        })
        .unwrap();
    let expected_end = layout
        .visual_lines
        .iter()
        .rposition(|line| {
            line.y < height - BORDER_WIDTH && line.y + layout.line_height > BORDER_WIDTH
        })
        .unwrap()
        + 1;

    assert_eq!(layout.visible_lines, expected_start..expected_end);
    let mut includes_padding_only_row = false;
    for (index, line) in layout.visual_lines.iter().enumerate() {
        let intersects_border_clip =
            line.y < height - BORDER_WIDTH && line.y + layout.line_height > BORDER_WIDTH;
        let intersects_padding_clip = line.y < layout.padding + layout.viewport_height
            && line.y + layout.line_height > layout.padding;
        includes_padding_only_row |= layout.visible_lines.contains(&index)
            && intersects_border_clip
            && !intersects_padding_clip;
        assert_eq!(
            layout.visible_lines.contains(&index),
            intersects_border_clip,
            "line {index}: y={} line_height={} border_clip={}..{}",
            line.y,
            layout.line_height,
            BORDER_WIDTH,
            height - BORDER_WIDTH
        );
    }
    assert!(includes_padding_only_row);
}

#[test]
fn hit_testing_returns_expected_byte_position() {
    let document = TextDocument::from_str("abcd");
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(200.0, 80.0),
        TextViewScroll::default(),
        false,
    );
    let ab_width = graphics::measure_text_sized("ab", 14.0).0 as f32;
    let position = layout.hit_test(Point::new(8.0 + ab_width + 1.0, 10.0));
    assert_eq!(position, Some(TextPosition::new(2)));
}

#[test]
fn hit_testing_accounts_for_scroll_offset() {
    let document = TextDocument::from_str("first\nsecond\nthird");
    let scroll = TextViewScroll {
        x: 0.0,
        y: 14.0 * 1.2,
    };
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(200.0, 80.0),
        scroll,
        false,
    );
    assert_eq!(
        layout.hit_test(Point::new(8.0, 8.0)),
        Some(TextPosition::new("first\n".len()))
    );
}

#[test]
fn cursor_rect_uses_position_and_line_height() {
    let document = TextDocument::from_str("abc\ndef");
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(200.0, 80.0),
        TextViewScroll::default(),
        false,
    );
    let rect = layout.cursor_rect(TextPosition::new(2), &document);
    let ab_width = graphics::measure_text_sized("ab", 14.0).0 as f32;
    assert_eq!(rect.origin.x, 8.0 + ab_width);
    assert_eq!(rect.origin.y, 8.0);
    assert_eq!(rect.size.height, layout.line_height);
}

#[test]
fn gutter_width_is_reserved_for_line_numbers() {
    let document = TextDocument::from_str("one\ntwo\nthree");
    let without = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(200.0, 80.0),
        TextViewScroll::default(),
        false,
    );
    let with = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(200.0, 80.0),
        TextViewScroll::default(),
        true,
    );
    assert_eq!(without.gutter_width, 0.0);
    assert!(with.gutter_width > 0.0);
}

#[test]
fn utf8_hit_testing_and_cursor_rect_stay_on_boundaries() {
    let document = TextDocument::from_str("あい😀う");
    let layout = TextViewLayout::compute(
        &document,
        14.0,
        8.0,
        WrapMode::None,
        Size::new(240.0, 80.0),
        TextViewScroll::default(),
        false,
    );
    let prefix_width = graphics::measure_text_sized("あい", 14.0).0 as f32;
    let position = layout.hit_test(Point::new(8.0 + prefix_width + 1.0, 10.0));
    assert!(position.is_some_and(|position| document.as_str().is_char_boundary(position.byte)));

    let emoji_end = "あい😀".len();
    let rect = layout.cursor_rect(TextPosition::new(emoji_end), &document);
    let expected_x = 8.0 + graphics::measure_text_sized("あい😀", 14.0).0 as f32;
    assert_eq!(rect.origin.x, expected_x);
}

fn key(keycode: KeyCode) -> KeyEvent {
    KeyEvent::Pressed {
        keycode,
        modifiers: KeyModifiers::empty(),
    }
}

fn shift_key(keycode: KeyCode) -> KeyEvent {
    KeyEvent::Pressed {
        keycode,
        modifiers: KeyModifiers {
            shift: true,
            ..KeyModifiers::empty()
        },
    }
}

fn primary_key(c: char) -> KeyEvent {
    KeyEvent::Pressed {
        keycode: KeyCode::Char(c),
        modifiers: KeyModifiers {
            control: true,
            ..KeyModifiers::empty()
        },
    }
}

fn focused_render_object(view: &TextView) -> TextViewRenderObject {
    let mut render_object = TextViewRenderObject::from_view(view);
    render_object.layout(LayoutConstraints::tight(240.0, 120.0));
    render_object.set_focused(true);
    render_object
}

fn render_object_with_size(view: &TextView, width: f32, height: f32) -> TextViewRenderObject {
    let mut render_object = TextViewRenderObject::from_view(view);
    render_object.layout(LayoutConstraints::tight(width, height));
    render_object
}

fn text_x(prefix: &str) -> i32 {
    (8.0 + graphics::measure_text_sized(prefix, 14.0).0 as f32 + 1.0) as i32
}

#[test]
fn down_arrow_moves_from_text_line_to_following_empty_line() {
    let text = State::new(StateId::new(199), String::from("first\n\nthird"));
    let selection = State::new(StateId::new(198), TextSelection::collapsed("first".len()));
    let view = TextView::new(text.clone(), selection.clone());
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        KeyEvent::Pressed {
            keycode: KeyCode::Down,
            modifiers: KeyModifiers::empty(),
        }
    ));

    assert_eq!(
        render_object
            .layout
            .line_index_at_byte(render_object.selection.caret.byte),
        1
    );
}

#[test]
fn keyboard_character_insertion_updates_document_and_caret() {
    let text = State::new(StateId::new(200), String::new());
    let selection = State::new(StateId::new(201), TextSelection::collapsed(0));
    let view = TextView::new(text.clone(), selection.clone());
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        KeyEvent::Char { c: 'a' }
    ));

    assert_eq!(render_object.text_document.as_str(), Cow::Borrowed("a"));
    assert_eq!(text.get(), "a");
    assert_eq!(render_object.selection, TextSelection::collapsed(1));
    assert_eq!(selection.get(), TextSelection::collapsed(1));
}

#[test]
fn keyboard_multibyte_insertion_tracks_byte_caret() {
    let text = State::new(StateId::new(202), String::new());
    let selection = State::new(StateId::new(203), TextSelection::collapsed(0));
    let view = TextView::new(text.clone(), selection.clone());
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        KeyEvent::Char { c: '日' }
    ));
    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        KeyEvent::Char { c: '本' }
    ));

    assert_eq!(render_object.text_document.as_str(), Cow::Borrowed("日本"));
    assert_eq!(render_object.selection.caret.byte, "日本".len());
}

#[test]
fn backspace_deletes_grapheme_and_selection() {
    let text = State::new(StateId::new(204), String::from("a👨‍👩‍👧‍👦b"));
    let selection = State::new(StateId::new(205), TextSelection::collapsed("a👨‍👩‍👧‍👦".len()));
    let view = TextView::new(text.clone(), selection);
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        key(KeyCode::Backspace)
    ));
    assert_eq!(render_object.text_document.as_str(), Cow::Borrowed("ab"));

    render_object.selection = TextSelection {
        anchor: TextPosition::new(0),
        caret: TextPosition::new(2),
    };
    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        key(KeyCode::Backspace)
    ));
    assert_eq!(render_object.text_document.as_str(), Cow::Borrowed(""));
}

#[test]
fn delete_forward_deletes_next_grapheme() {
    let text = State::new(StateId::new(206), String::from("aéb"));
    let selection = State::new(StateId::new(207), TextSelection::collapsed(1));
    let view = TextView::new(text, selection);
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        key(KeyCode::Delete)
    ));

    assert_eq!(render_object.text_document.as_str(), Cow::Borrowed("ab"));
    assert_eq!(render_object.selection, TextSelection::collapsed(1));
}

#[test]
fn enter_inserts_newline_and_updates_line_count() {
    let text = State::new(StateId::new(208), String::from("one"));
    let selection = State::new(StateId::new(209), TextSelection::collapsed(3));
    let view = TextView::new(text, selection);
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        key(KeyCode::Enter)
    ));

    assert_eq!(render_object.text_document.as_str(), Cow::Borrowed("one\n"));
    assert_eq!(render_object.text_document.line_count(), 2);
}

#[test]
fn tab_insertion_respects_tab_mode() {
    let text = State::new(StateId::new(210), String::new());
    let selection = State::new(StateId::new(211), TextSelection::collapsed(0));
    let view = TextView::new(text.clone(), selection.clone()).tab_mode(TabMode::Tab);
    let mut render_object = focused_render_object(&view);
    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        key(KeyCode::Tab)
    ));
    assert_eq!(render_object.text_document.as_str(), Cow::Borrowed("\t"));

    let text = State::new(StateId::new(212), String::new());
    let selection = State::new(StateId::new(213), TextSelection::collapsed(0));
    let view = TextView::new(text, selection).tab_mode(TabMode::Spaces(4));
    let mut render_object = focused_render_object(&view);
    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        key(KeyCode::Tab)
    ));
    assert_eq!(render_object.text_document.as_str(), Cow::Borrowed("    "));
}

#[test]
fn arrows_move_by_grapheme_and_visual_line() {
    let text = State::new(StateId::new(214), String::from("aé\ncd"));
    let selection = State::new(StateId::new(215), TextSelection::collapsed("aé".len()));
    let view = TextView::new(text, selection);
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        key(KeyCode::Left)
    ));
    assert_eq!(render_object.selection.caret.byte, 1);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        key(KeyCode::Right)
    ));
    assert_eq!(render_object.selection.caret.byte, "aé".len());

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        key(KeyCode::Down)
    ));
    assert!(render_object.selection.caret.byte >= "aé\n".len());
}

#[test]
fn shift_arrow_extends_selection() {
    let text = State::new(StateId::new(216), String::from("abc"));
    let selection = State::new(StateId::new(217), TextSelection::collapsed(1));
    let view = TextView::new(text, selection.clone());
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        shift_key(KeyCode::Right)
    ));

    assert_eq!(render_object.selection.anchor.byte, 1);
    assert_eq!(render_object.selection.caret.byte, 2);
    assert_eq!(selection.get(), render_object.selection);
}

#[test]
fn home_end_and_primary_a_update_selection() {
    let text = State::new(StateId::new(218), String::from("one\ntwo"));
    let selection = State::new(StateId::new(219), TextSelection::collapsed("one\nt".len()));
    let view = TextView::new(text, selection);
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        key(KeyCode::Home)
    ));
    assert_eq!(render_object.selection.caret.byte, "one\n".len());

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        key(KeyCode::End)
    ));
    assert_eq!(render_object.selection.caret.byte, "one\ntwo".len());

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        primary_key('a')
    ));
    assert_eq!(
        render_object.selection.normalized_range(),
        Some(0.."one\ntwo".len())
    );
}

#[test]
fn ime_commit_inserts_at_caret() {
    let text = State::new(StateId::new(220), String::from("ab"));
    let selection = State::new(StateId::new(221), TextSelection::collapsed(1));
    let view = TextView::new(text, selection);
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_text_input(
        &view,
        &mut render_object,
        &Event::TextInputCommit {
            context_id: 1,
            serial: 1,
            text: String::from("日本"),
        }
    ));

    assert_eq!(
        render_object.text_document.as_str(),
        Cow::Borrowed("a日本b")
    );
    assert_eq!(render_object.selection.caret.byte, "a日本".len());
}

#[test]
fn ime_preedit_stores_state_without_mutating_document() {
    let text = State::new(StateId::new(222), String::from("base"));
    let selection = State::new(StateId::new(223), TextSelection::collapsed(4));
    let view = TextView::new(text, selection);
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_text_input(
        &view,
        &mut render_object,
        &Event::TextInputPreedit {
            context_id: 1,
            serial: 1,
            cursor_byte: 3,
            anchor_byte: 0,
            text: String::from("か"),
            spans: Vec::new(),
        }
    ));

    assert_eq!(render_object.text_document.as_str(), Cow::Borrowed("base"));
    assert_eq!(render_object.preedit(), "か");
    assert_eq!(render_object.preedit_cursor_byte(), 3);
}

#[test]
fn paint_preedit_text_uses_normal_text_color() {
    let text = State::new(StateId::new(2220), String::from("base"));
    let selection = State::new(StateId::new(2221), TextSelection::collapsed(4));
    let view = TextView::new(text, selection);
    let mut render_object = focused_render_object(&view);
    render_object.set_preedit_state("か", 3, 0, &[]);

    let mut ctx = crate::renderer::PaintContext::new();
    render_object.paint(&mut ctx, Point::ZERO);

    let preedit_text = ctx.commands().iter().find_map(|command| match command {
        PaintCommand::DrawText { text, color, .. } if text == "か" => Some(*color),
        _ => None,
    });
    assert_eq!(preedit_text, Some(render_object.text_color));
}

#[test]
fn ime_delete_surrounding_text_deletes_around_caret() {
    let text = State::new(StateId::new(224), String::from("abcd"));
    let selection = State::new(StateId::new(225), TextSelection::collapsed(2));
    let view = TextView::new(text, selection);
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_text_input(
        &view,
        &mut render_object,
        &Event::TextInputDeleteSurroundingText {
            context_id: 1,
            serial: 1,
            before_bytes: 1,
            after_bytes: 1,
        }
    ));

    assert_eq!(render_object.text_document.as_str(), Cow::Borrowed("ad"));
    assert_eq!(render_object.selection, TextSelection::collapsed(1));
}

#[test]
fn focus_events_toggle_focused_and_clear_preedit() {
    let text = State::new(StateId::new(226), String::new());
    let selection = State::new(StateId::new(227), TextSelection::collapsed(0));
    let view = TextView::new(text, selection);
    let mut render_object = TextViewRenderObject::from_view(&view);

    assert!(handle_text_view_focus(
        &mut render_object,
        FocusEvent::Gained
    ));
    assert!(render_object.is_focused());
    render_object.set_preedit_state("かな", 6, 0, &[]);
    assert!(handle_text_view_focus(&mut render_object, FocusEvent::Lost));
    assert!(!render_object.is_focused());
    assert_eq!(render_object.preedit(), "");
}

#[test]
fn edits_sync_state_and_fire_text_change_callback() {
    let text = State::new(StateId::new(228), String::new());
    let selection = State::new(StateId::new(229), TextSelection::collapsed(0));
    let deltas = Arc::new(Mutex::new(Vec::<EditDelta>::new()));
    let deltas_for_callback = deltas.clone();
    let view = TextView::new(text.clone(), selection.clone()).on_text_change(move |delta| {
        deltas_for_callback.lock().unwrap().push(delta.clone());
    });
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        KeyEvent::Char { c: 'x' }
    ));

    assert_eq!(text.get(), "x");
    assert_eq!(selection.get(), TextSelection::collapsed(1));
    let deltas = deltas.lock().unwrap();
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].replaced_range, 0..0);
    assert_eq!(deltas[0].inserted, "x");
}

#[test]
fn text_input_state_reports_cursor_and_anchor_bytes() {
    let text = State::new(StateId::new(230), String::from("hello"));
    let selection = State::new(
        StateId::new(231),
        TextSelection {
            anchor: TextPosition::new(1),
            caret: TextPosition::new(4),
        },
    );
    let view = TextView::new(text, selection);
    let render_object = focused_render_object(&view);

    let state = render_object.text_input_state();
    assert_eq!(state.surrounding_text, "hello");
    assert_eq!(state.cursor_byte, 4);
    assert_eq!(state.anchor_byte, 1);
    assert!(state.cursor_rect.size.height > 0.0);
}

#[test]
fn mouse_single_click_positions_caret_and_collapses_selection() {
    let text = State::new(StateId::new(232), String::from("abcd"));
    let selection = State::new(StateId::new(233), TextSelection::collapsed(4));
    let view = TextView::new(text, selection.clone());
    let mut render_object = render_object_with_size(&view, 240.0, 120.0);

    assert!(handle_text_view_mouse(
        &view,
        &mut render_object,
        &MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: text_x("ab"),
            y: 10,
            click_count: 1,
        }
    ));

    assert_eq!(render_object.selection, TextSelection::collapsed(2));
    assert_eq!(selection.get(), TextSelection::collapsed(2));
    assert!(render_object.is_focused());
    assert!(render_object.dragging);
}

#[test]
fn mouse_double_click_selects_word_under_cursor() {
    let text = State::new(StateId::new(234), String::from("hello world"));
    let selection = State::new(StateId::new(235), TextSelection::collapsed(0));
    let view = TextView::new(text, selection.clone());
    let mut render_object = render_object_with_size(&view, 240.0, 120.0);

    assert!(handle_text_view_mouse(
        &view,
        &mut render_object,
        &MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: text_x("hello wo"),
            y: 10,
            click_count: 2,
        }
    ));

    assert_eq!(render_object.selection.anchor.byte, "hello ".len());
    assert_eq!(render_object.selection.caret.byte, "hello world".len());
    assert_eq!(selection.get(), render_object.selection);
}

#[test]
fn mouse_triple_click_selects_full_line_without_newline() {
    let text = State::new(StateId::new(236), String::from("one\ntwo\nthree"));
    let selection = State::new(StateId::new(237), TextSelection::collapsed(0));
    let view = TextView::new(text, selection.clone());
    let mut render_object = render_object_with_size(&view, 240.0, 120.0);
    let second_line_y = (8.0 + render_object.layout.line_height + 1.0) as i32;

    assert!(handle_text_view_mouse(
        &view,
        &mut render_object,
        &MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 8,
            y: second_line_y,
            click_count: 3,
        }
    ));

    assert_eq!(render_object.selection.anchor.byte, "one\n".len());
    assert_eq!(render_object.selection.caret.byte, "one\ntwo".len());
    assert_eq!(selection.get(), render_object.selection);
}

#[test]
fn mouse_drag_extends_selection_and_release_clears_dragging() {
    let text = State::new(StateId::new(238), String::from("abcdef"));
    let selection = State::new(StateId::new(239), TextSelection::collapsed(0));
    let view = TextView::new(text, selection.clone());
    let mut render_object = render_object_with_size(&view, 240.0, 120.0);

    assert!(handle_text_view_mouse(
        &view,
        &mut render_object,
        &MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: text_x("a"),
            y: 10,
            click_count: 1,
        }
    ));
    let anchor = render_object.selection.anchor;

    assert!(handle_text_view_mouse(
        &view,
        &mut render_object,
        &MouseEvent::Moved {
            x: text_x("abcd"),
            y: 10,
        }
    ));
    assert_eq!(render_object.selection.anchor, anchor);
    assert_eq!(render_object.selection.caret.byte, 4);
    assert_eq!(selection.get(), render_object.selection);

    assert!(handle_text_view_mouse(
        &view,
        &mut render_object,
        &MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: text_x("abcd"),
            y: 10,
            click_count: 1,
        }
    ));
    assert!(!render_object.dragging);
}

#[test]
fn mouse_wheel_scrolls_vertically_and_clamps() {
    let text = State::new(
        StateId::new(240),
        String::from("0\n1\n2\n3\n4\n5\n6\n7\n8\n9"),
    );
    let selection = State::new(StateId::new(241), TextSelection::collapsed(0));
    let scroll = State::new(StateId::new(242), TextViewScroll::default());
    let view = TextView::new(text, selection).scroll_state(scroll.clone());
    let mut render_object = render_object_with_size(&view, 120.0, 40.0);

    assert!(handle_text_view_mouse(
        &view,
        &mut render_object,
        &MouseEvent::Wheel {
            delta_x: 0,
            delta_y: 10_000,
        }
    ));

    let max_y =
        (render_object.layout.content_height - render_object.layout.viewport_height).max(0.0);
    assert_eq!(render_object.scroll.y, max_y);
    assert_eq!(scroll.get(), render_object.scroll);
}

#[test]
fn mouse_wheel_scrolls_horizontally_without_wrapping() {
    let text = State::new(
        StateId::new(243),
        String::from("a very long line that should require horizontal scrolling"),
    );
    let selection = State::new(StateId::new(244), TextSelection::collapsed(0));
    let scroll = State::new(StateId::new(245), TextViewScroll::default());
    let view = TextView::new(text, selection)
        .scroll_state(scroll.clone())
        .wrap_mode(WrapMode::None);
    let mut render_object = render_object_with_size(&view, 80.0, 80.0);

    assert!(handle_text_view_mouse(
        &view,
        &mut render_object,
        &MouseEvent::Wheel {
            delta_x: 40,
            delta_y: 0,
        }
    ));

    assert!(render_object.scroll.x > 0.0);
    assert_eq!(scroll.get(), render_object.scroll);
}

#[test]
fn clipboard_copy_paste_and_cut_callbacks_work() {
    let text = State::new(StateId::new(246), String::from("hello world"));
    let selection = State::new(
        StateId::new(247),
        TextSelection {
            anchor: TextPosition::new(0),
            caret: TextPosition::new(5),
        },
    );
    let copied = Arc::new(Mutex::new(String::new()));
    let copied_for_callback = copied.clone();
    let view = TextView::new(text.clone(), selection)
        .on_copy(move |value| *copied_for_callback.lock().unwrap() = value.into_owned())
        .on_paste(|| Some(Cow::Borrowed("Scarlet")));
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        primary_key('c')
    ));
    assert_eq!(&*copied.lock().unwrap(), "hello");

    render_object.selection = TextSelection::collapsed(6);
    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        primary_key('v')
    ));
    assert_eq!(text.get(), "hello Scarletworld");

    render_object.selection = TextSelection {
        anchor: TextPosition::new(6),
        caret: TextPosition::new(13),
    };
    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        primary_key('x')
    ));
    assert_eq!(&*copied.lock().unwrap(), "Scarlet");
    assert_eq!(text.get(), "hello world");
}

#[test]
fn clipboard_shortcuts_without_callbacks_are_no_ops() {
    let text = State::new(StateId::new(248), String::from("hello"));
    let selection = State::new(
        StateId::new(249),
        TextSelection {
            anchor: TextPosition::new(0),
            caret: TextPosition::new(5),
        },
    );
    let view = TextView::new(text.clone(), selection);
    let mut render_object = focused_render_object(&view);

    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        primary_key('c')
    ));
    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        primary_key('v')
    ));
    assert!(handle_text_view_keyboard(
        &view,
        &mut render_object,
        primary_key('x')
    ));

    assert_eq!(text.get(), "hello");
    assert_eq!(render_object.text_document.as_str(), Cow::Borrowed("hello"));
}
