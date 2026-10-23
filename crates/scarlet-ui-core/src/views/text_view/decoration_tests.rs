use super::*;

fn stack() -> FontStack {
    FontStack::new(include_bytes!("../../testing/fonts/metrics-primary.ttf"))
        .unwrap()
        .with_fallback(include_bytes!("../../testing/fonts/metrics-fallback.ttf"))
        .unwrap()
}

fn editor(text: &str) -> TextView {
    TextView::new(
        State::new(StateId::new(7600), text.to_string()),
        State::new(StateId::new(7601), TextSelection::collapsed(0)),
    )
    .font_stack(stack())
    .font_size(20.)
}

fn red() -> Color {
    Color::rgb(200u8, 10u8, 20u8)
}

fn blue() -> Color {
    Color::rgb(10u8, 20u8, 200u8)
}

fn text_commands(object: &TextViewRenderObject) -> Vec<(String, Color, Point)> {
    let mut ctx = PaintContext::new();
    object.paint(&mut ctx, Point::ZERO);
    ctx.commands()
        .iter()
        .filter_map(|command| match command {
            PaintCommand::DrawText {
                text,
                color,
                position,
                font_stack,
                ..
            } => {
                assert_eq!(
                    font_stack.as_deref().map(FontStack::cache_id),
                    object.font_stack.as_deref().map(FontStack::cache_id)
                );
                Some((text.clone(), *color, *position))
            }
            _ => None,
        })
        .collect()
}

fn marks(object: &TextViewRenderObject, color: Color) -> Vec<Rect> {
    let mut ctx = PaintContext::new();
    object.paint(&mut ctx, Point::ZERO);
    ctx.commands()
        .iter()
        .filter_map(|command| match command {
            PaintCommand::FillPath {
                path,
                color: painted,
            } if *painted == color => Some(Rect::from_xywh(
                path[0].x,
                path[0].y,
                path[1].x - path[0].x,
                path[2].y - path[0].y,
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn foreground_unsorted_overlaps_use_input_order_and_restore_default_color() {
    let view = editor("AiWAAi").text_spans(vec![
        TextStyleSpan {
            range: 2..5,
            color: red(),
        },
        TextStyleSpan {
            range: 0..3,
            color: blue(),
        },
        TextStyleSpan {
            range: 3..4,
            color: blue(),
        },
    ]);
    let object = render_object_with_size(&view, 400., 100.);
    let commands = text_commands(&object);
    assert_eq!(
        commands
            .iter()
            .map(|(text, color, _)| (text.as_str(), *color))
            .collect::<Vec<_>>(),
        vec![
            ("Ai", blue()),
            ("W", blue()),
            ("A", blue()),
            ("A", red()),
            ("i", object.text_color)
        ]
    );
    let line = &object.layout.visual_lines[0];
    let mut byte = 0;
    for (text, _, point) in commands {
        assert_eq!(point.x, line.x + line.width_to_byte(byte));
        byte += text.len();
    }
}

#[test]
fn utf8_ranges_match_scalar_intersection_even_for_invalid_endpoints() {
    let text = "A日☃i";
    let endpoints = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, usize::MAX];
    for start in endpoints {
        for end in endpoints {
            let view = editor(text).text_spans(vec![TextStyleSpan {
                range: start..end,
                color: red(),
            }]);
            let object = render_object_with_size(&view, 400., 100.);
            let actual: Vec<_> = text_commands(&object)
                .into_iter()
                .flat_map(|(text, color, _)| {
                    text.chars().map(move |ch| (ch, color)).collect::<Vec<_>>()
                })
                .collect();
            let expected: Vec<_> = text
                .char_indices()
                .map(|(byte, ch)| {
                    let covered = start < end && start < byte + ch.len_utf8() && end > byte;
                    (ch, if covered { red() } else { object.text_color })
                })
                .collect();
            assert_eq!(actual, expected, "range {start}..{end}");
        }
    }
}

#[test]
fn underline_utf8_ranges_ignore_empty_reversed_newline_and_outside_spans() {
    let view = editor("A日\r\ni").diagnostic_spans(vec![
        TextDiagnosticSpan {
            range: 2..3,
            color: red(),
        },
        TextDiagnosticSpan {
            range: 4..6,
            color: red(),
        },
        TextDiagnosticSpan {
            range: 0..0,
            color: red(),
        },
        TextDiagnosticSpan {
            range: 8..2,
            color: red(),
        },
        TextDiagnosticSpan {
            range: usize::MAX - 1..usize::MAX,
            color: red(),
        },
    ]);
    let object = render_object_with_size(&view, 400., 140.);
    let line = &object.layout.visual_lines[0];
    assert_eq!(
        marks(&object, red()),
        vec![Rect::from_xywh(
            line.x + line.width_to_byte(1),
            line.y + object.layout.line_height - 2.,
            line.width_to_byte(4) - line.width_to_byte(1),
            1.
        )]
    );
    assert!(
        text_commands(&object)
            .iter()
            .all(|(_, color, _)| *color == object.text_color)
    );
}

#[test]
fn multiline_spans_cover_tabs_and_soft_wrapped_rows_without_coloring_gutter() {
    let text = "A\t日日\r\n☃AA";
    let view = editor(text)
        .wrap_mode(WrapMode::Soft)
        .line_numbers(true)
        .text_spans(vec![TextStyleSpan {
            range: 0..usize::MAX,
            color: red(),
        }])
        .diagnostic_spans(vec![TextDiagnosticSpan {
            range: 0..usize::MAX,
            color: blue(),
        }]);
    let object = render_object_with_size(&view, 95., 400.);
    assert!(object.layout.visual_lines.len() > 2);
    let commands = text_commands(&object);
    let colored: String = commands
        .iter()
        .filter(|(_, color, _)| *color == red())
        .map(|(text, _, _)| text.as_str())
        .collect();
    assert_eq!(colored, "A    日日☃AA");
    assert!(
        commands
            .iter()
            .any(|(text, color, _)| text == "1" && *color == object.placeholder_color)
    );
    let underlines = marks(&object, blue());
    assert_eq!(underlines.len(), object.layout.visual_lines.len());
    for (mark, line) in underlines.iter().zip(&object.layout.visual_lines) {
        assert_eq!(mark.origin.x, line.x);
        assert_eq!(mark.size.width, line.width);
    }
}

#[test]
fn decoration_updates_preserve_layout_selection_scroll_and_composition() {
    let view = editor(&"Ai日☃\tW\n".repeat(20)).autofocus(true);
    let mut object = render_object_with_size(&view, 90., 80.);
    object.selection = TextSelection::collapsed(1);
    view.selection.set(object.selection);
    object.scroll = TextViewScroll { x: 12., y: 40. };
    object.compute_layout();
    object.set_preedit_state("日", 3, 3, &[]);
    let storage = object.layout.visual_lines.as_ptr();
    let scroll = object.scroll;
    let caret = object.cursor_rect();
    let decorated = view
        .clone()
        .text_spans(vec![TextStyleSpan {
            range: 0..usize::MAX,
            color: red(),
        }])
        .diagnostic_spans(vec![TextDiagnosticSpan {
            range: 0..usize::MAX,
            color: blue(),
        }]);
    object.update(&decorated);
    assert_eq!(object.layout.visual_lines.as_ptr(), storage);
    assert_eq!(object.scroll, scroll);
    assert_eq!(object.selection, view.selection.get());
    assert_eq!(object.cursor_rect(), caret);
    assert_eq!(object.preedit(), "日");
    assert!(
        text_commands(&object)
            .iter()
            .any(|(_, color, _)| *color == red())
    );
    object.update(&view);
    assert_eq!(object.layout.visual_lines.as_ptr(), storage);
    assert!(
        text_commands(&object)
            .iter()
            .all(|(_, color, _)| *color == object.text_color)
    );
    assert!(marks(&object, blue()).is_empty());
}

#[test]
fn ime_insertion_splits_underlines_and_keeps_preedit_in_default_color() {
    let view = editor("Ai☃")
        .autofocus(true)
        .text_spans(vec![TextStyleSpan {
            range: 0..8,
            color: red(),
        }])
        .diagnostic_spans(vec![TextDiagnosticSpan {
            range: 0..8,
            color: blue(),
        }]);
    view.selection.set(TextSelection::collapsed(1));
    let mut object = render_object_with_size(&view, 400., 100.);
    object.set_preedit_state("日", 3, 3, &[]);
    let commands = text_commands(&object);
    let line = &object.layout.visual_lines[0];
    let caret = object.cursor_rect();
    let width = object.layout.measure_text("日").0 as f32;
    assert!(commands.iter().any(|(text, color, point)| text == "i☃"
        && *color == red()
        && point.x == caret.origin.x + width));
    assert!(commands.iter().any(|(text, color, point)| text == "日"
        && *color == object.text_color
        && point.x == caret.origin.x));
    let underlines = marks(&object, blue());
    assert_eq!(underlines.len(), 2);
    assert_eq!(underlines[0].origin.x, line.x);
    assert_eq!(underlines[0].size.width, line.width_to_byte(1));
    assert_eq!(underlines[1].origin.x, caret.origin.x + width);
    assert_eq!(underlines[1].size.width, line.width - line.width_to_byte(1));
}

#[test]
fn painting_large_scrolled_documents_emits_only_visible_rows_and_columns() {
    let text = ("Ai日☃".repeat(200) + "\n").repeat(100);
    let view = editor(&text)
        .text_spans(vec![TextStyleSpan {
            range: 0..usize::MAX,
            color: red(),
        }])
        .diagnostic_spans(vec![TextDiagnosticSpan {
            range: 0..usize::MAX,
            color: blue(),
        }]);
    let mut object = render_object_with_size(&view, 150., 90.);
    object.scroll = TextViewScroll {
        x: 1200.5,
        y: 600.25,
    };
    object.compute_layout();
    let commands = text_commands(&object);
    assert_eq!(commands.len(), object.layout.visible_lines.len());
    for ((text, color, point), line) in commands
        .iter()
        .zip(&object.layout.visual_lines[object.layout.visible_lines.clone()])
    {
        assert_eq!(*color, red());
        assert!(
            text.len() < 100,
            "offscreen columns reached paint: {} bytes",
            text.len()
        );
        assert_eq!(point.y, line.y);
        assert!(point.x >= object.layout.text_origin_x - 40.);
    }
    for mark in marks(&object, blue()) {
        assert!(mark.origin.x >= object.layout.text_origin_x);
        assert!(mark.origin.x + mark.size.width <= object.size.width - BORDER_WIDTH);
    }
}

#[test]
fn empty_document_decorations_do_not_style_placeholder() {
    let view = editor("")
        .placeholder("日A")
        .text_spans(vec![TextStyleSpan {
            range: 0..usize::MAX,
            color: red(),
        }])
        .diagnostic_spans(vec![TextDiagnosticSpan {
            range: 0..usize::MAX,
            color: blue(),
        }]);
    let object = render_object_with_size(&view, 200., 100.);
    assert_eq!(text_commands(&object)[0].1, object.placeholder_color);
    assert!(marks(&object, blue()).is_empty());
}

#[test]
fn public_prelude_exports_decoration_types() {
    let _: crate::prelude::TextStyleSpan = TextStyleSpan {
        range: 0..1,
        color: red(),
    };
    let _: crate::prelude::TextDiagnosticSpan = TextDiagnosticSpan {
        range: 0..1,
        color: blue(),
    };
}
