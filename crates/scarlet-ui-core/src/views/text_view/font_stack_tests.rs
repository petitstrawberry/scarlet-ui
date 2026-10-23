use super::*;

fn primary() -> FontStack {
    FontStack::new(include_bytes!("../../testing/fonts/metrics-primary.ttf")).unwrap()
}

fn stack() -> FontStack {
    primary()
        .with_fallback(include_bytes!("../../testing/fonts/metrics-fallback.ttf"))
        .unwrap()
}

fn editor(text: &str) -> TextView {
    TextView::new(
        State::new(StateId::new(7500), text.to_string()),
        State::new(StateId::new(7501), TextSelection::collapsed(0)),
    )
    .font_size(20.)
}

fn assert_text_stacks(ctx: &PaintContext<'_>, stack: &FontStack) -> Vec<String> {
    let mut labels = Vec::new();
    for command in ctx.commands() {
        match command {
            PaintCommand::DrawText {
                text, font_stack, ..
            } => {
                assert_eq!(
                    font_stack.as_deref().map(FontStack::cache_id),
                    Some(stack.cache_id())
                );
                labels.push(text.clone());
            }
            PaintCommand::DrawBuffer { .. } | PaintCommand::DrawBufferRect { .. } => {
                panic!("TextView must retain text commands, not a view bitmap")
            }
            _ => {}
        }
    }
    assert!(!labels.is_empty());
    labels
}

#[test]
fn explicit_stack_controls_em_metrics_tabs_wrap_carets_hit_testing_and_selection() {
    let stack = stack();
    let default_revision = graphics::text_font_revision();
    let view = editor("Ai\t日☃\nWW")
        .font_stack(stack.clone())
        .line_numbers(true);
    let mut object = render_object_with_size(&view, 300., 160.);
    assert_eq!(object.layout.line_height, 29.); // ceil(20 * (1160 + 288) / 1000)
    assert_eq!(object.layout.gutter_width, 24.); // missing digit: '?' at 0.6 em + padding
    let line = &object.layout.visual_lines[0];
    assert_eq!(line.text(), "Ai    日☃");
    assert_eq!(line.width, 84.); // ceil(14 + 5.6 + 24 + 20 + 20)
    for byte in [0, 1, 2, 3, 6, 9] {
        let display_byte = line.display_byte_for_document_byte(byte);
        let expected =
            graphics::measure_text_sized_with_font_stack(&line.text()[..display_byte], 20., &stack)
                .0 as f32;
        assert_eq!(line.width_to_byte(byte), expected);
        let caret = object
            .layout
            .cursor_rect(TextPosition::new(byte), &object.text_document);
        assert_eq!(caret.origin.x, line.x + expected);
        assert_eq!(
            object.hit_test(Point::new(caret.origin.x, caret.origin.y + 1.)),
            Some(TextPosition::new(byte))
        );
    }
    object.selection = TextSelection {
        anchor: TextPosition::new(3),
        caret: TextPosition::new(9),
    };
    let mut ctx = PaintContext::new();
    assert!(object.paint(&mut ctx, Point::ZERO));
    #[allow(deprecated)]
    {
        assert!(object.get_buffer().is_none());
    }
    assert_text_stacks(&ctx, &stack);
    assert!(ctx.commands().iter().any(|command| matches!(command,
        PaintCommand::FillPath { path, color }
            if *color == object.selection_color && path[1].x - path[0].x == 40.
    )));

    let wrapped = editor("日日☃☃")
        .font_stack(stack.clone())
        .wrap_mode(WrapMode::Soft);
    let object = render_object_with_size(&wrapped, 57., 160.);
    assert_eq!(object.layout.visual_lines.len(), 2);
    assert_eq!(object.layout.visual_lines[0].text(), "日日");
    assert_eq!(object.layout.visual_lines[1].text(), "☃☃");
    assert!(
        object
            .layout
            .visual_lines
            .iter()
            .all(|line| line.width == 40.)
    );
    assert_eq!(graphics::text_font_revision(), default_revision);
}

#[test]
fn explicit_stack_updates_invalidate_layout_but_clones_and_scroll_reuse_it() {
    let primary = primary();
    let base = editor("☃☃\nAA\nAA\nAA\nAA\nAA");
    let view = base.clone().font_stack(primary.clone());
    let mut object = render_object_with_size(&view, 90., 60.);
    assert_eq!(object.layout.visual_lines[0].width, 24.);
    let lines = object.layout.visual_lines.as_ptr();
    object.requested_scroll = TextViewScroll { x: 0., y: 20. };
    object.update(&view.clone());
    assert_eq!(object.layout.visual_lines.as_ptr(), lines);
    assert_eq!(object.scroll.y, 20.);

    let fallback = primary
        .with_fallback(include_bytes!("../../testing/fonts/metrics-fallback.ttf"))
        .unwrap();
    object.desired_x = Some(12.);
    object.update(&base.clone().font_stack(fallback.clone()));
    assert_eq!(object.layout.visual_lines[0].width, 40.);
    assert_eq!(object.desired_x, None);
    let mut ctx = PaintContext::new();
    object.paint(&mut ctx, Point::ZERO);
    assert_text_stacks(&ctx, &fallback);
    drop(ctx);

    let other = FontStack::new(include_bytes!("../../testing/fonts/metrics-fallback.ttf")).unwrap();
    object.update(&base.clone().font_stack(other));
    assert_eq!(object.layout.line_height, 34.);
    object.update(&base);
    let default = render_object_with_size(&base, 90., 60.);
    assert_eq!(object.layout.line_height, default.layout.line_height);
    assert_eq!(object.layout.max_line_width, default.layout.max_line_width);
    let mut ctx = PaintContext::new();
    object.paint(&mut ctx, Point::ZERO);
    assert!(ctx.commands().iter().all(|command| !matches!(
        command,
        PaintCommand::DrawText {
            font_stack: Some(_),
            ..
        }
    )));
}

#[test]
fn explicit_stack_covers_placeholder_gutter_and_ime_measurement_and_paint() {
    let stack = stack();
    let view = editor("")
        .font_stack(stack.clone())
        .placeholder("Ai☃")
        .line_numbers(true);
    let object = render_object_with_size(&view, 240., 100.);
    let mut ctx = PaintContext::new();
    object.paint(&mut ctx, Point::ZERO);
    let labels = assert_text_stacks(&ctx, &stack);
    assert!(labels.iter().any(|label| label == "Ai☃"));
    assert!(labels.iter().any(|label| label == "1"));

    let view = editor("AW").font_stack(stack.clone()).line_numbers(true);
    view.selection.set(TextSelection::collapsed(1));
    let mut object = focused_render_object(&view);
    // Highlight only the second composition character; its prefix uses fallback metrics.
    let spans: Vec<u8> = [3u32, 1, 1 << 2]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    object.set_preedit_state("☃i", 4, 3, &spans);
    let caret = object.cursor_rect();
    assert_eq!(
        object.text_input_state().cursor_rect.origin.x,
        caret.origin.x + 20.
    );
    let mut ctx = PaintContext::new();
    object.paint(&mut ctx, Point::ZERO);
    let labels = assert_text_stacks(&ctx, &stack);
    assert!(labels.iter().any(|label| label == "☃i"));
    assert!(ctx.commands().iter().any(|command| matches!(command,
        PaintCommand::DrawText { text, position, .. }
            if text == "W" && position.x == caret.origin.x + 26.
    )));
    assert!(ctx.commands().iter().any(|command| matches!(command,
        PaintCommand::FillPath { path, color }
            if *color == object.text_color && path[0].x == caret.origin.x + 26.
    )));
    assert!(ctx.commands().iter().any(|command| matches!(command,
        PaintCommand::FillPath { path, color }
            if *color == object.focused_border_color
                && path[0].x == caret.origin.x + 20.
                && path[1].x - path[0].x == 6.
    )));
    drop(ctx);
    object.size = Size::new(65., 80.);
    object.compute_layout();
    editing::ensure_caret_visible(&view, &mut object);
    assert!(object.scroll.x > 0.);
    assert!(object.cursor_rect().origin.x + 26. <= object.size.width - BORDER_WIDTH);
}
