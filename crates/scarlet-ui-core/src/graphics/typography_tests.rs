use super::*;

fn stack() -> FontStack {
    FontStack::new(include_bytes!("../testing/fonts/metrics-primary.ttf"))
        .unwrap()
        .with_fallback(include_bytes!("../testing/fonts/metrics-fallback.ttf"))
        .unwrap()
}

#[test]
fn font_sizes_are_pixels_per_em_for_primary_and_fallback_fonts() {
    let stack = stack();
    // Both faces have 1000 units/em but different ascent/descent metrics.
    // Their fullwidth glyphs must each advance exactly one em.
    assert_eq!(
        measure_text_sized_with_font_stack("日日", 14., &stack).0,
        28
    );
    assert_eq!(measure_text_sized_with_font_stack("☃☃", 14., &stack).0, 28);
    assert_eq!(measure_text_sized_with_font_stack("日☃", 24., &stack).0, 48);
    assert_eq!(
        measure_text_sized_with_font_stack("AViW", 20., &stack).0,
        52
    );
}

#[test]
fn rendering_positions_follow_font_advances_at_integer_and_fractional_dpi() {
    let stack = stack();
    for (text, units) in [
        ("AViW", alloc::vec![700., 690., 280., 920.]),
        ("日日、日", alloc::vec![1000.; 4]),
        ("A日、⏻", alloc::vec![700., 1000., 1000., 900.]),
    ] {
        for size in [12., 14., 18.5, 24.] {
            let advance: f32 = units.iter().sum::<f32>() * size / 1000.;
            assert_eq!(
                measure_text_sized_with_font_stack(text, size, &stack).0,
                ceil_i32(advance) as u32
            );
            for dpi in [1000, 1250, 2000] {
                let glyphs = rasterize_text_with_font_stack(text, size, dpi, &stack);
                assert_eq!(glyphs.len(), units.len());
                let scale = size * dpi as f32 / 1000.;
                let mut position = 0.;
                for (glyph, advance) in glyphs.iter().zip(&units) {
                    // Every fixture glyph has a 40-font-unit left bearing.
                    let expected = position + 40. * scale / 1000.;
                    assert!(
                        (glyph.x as f32 - expected).abs() < 2.,
                        "{text:?} at {size}/{dpi}: {} vs {expected}",
                        glyph.x
                    );
                    position += advance * scale / 1000.;
                }
            }
        }
    }
}

#[test]
fn fractional_size_and_dpi_do_not_reuse_a_different_glyph_mask() {
    let stack = stack();
    let first = rasterize_text_with_font_stack("日", 14.01, 1000, &stack);
    let second = rasterize_text_with_font_stack("日", 14.09, 1000, &stack);
    assert_ne!(first[0].key, second[0].key);
    assert_ne!(first[0].mask, second[0].mask);
    let scaled = rasterize_text_with_font_stack("日", 14.01, 1001, &stack);
    assert_ne!(first[0].key, scaled[0].key);
    assert_ne!(first[0].mask, scaled[0].mask);
    assert_eq!(
        first,
        rasterize_text_with_font_stack("日", 14.01, 1000, &stack)
    );
}

#[test]
fn fallback_font_uses_the_primary_baseline_at_the_same_em_scale() {
    let stack = stack();
    let glyphs = rasterize_text_with_font_stack("日☃", 20., 1250, &stack);
    assert_eq!(glyphs[0].key.font_slot, 0);
    assert_eq!(glyphs[1].key.font_slot, 1);
    assert!((glyphs[0].y - glyphs[1].y).abs() <= 1);
    assert!((glyphs[0].height as i32 - glyphs[1].height as i32).abs() <= 1);
}

#[test]
fn text_layout_prefix_widths_match_complete_run_measurement() {
    let stack = stack();
    let measure = TextMeasure {
        font_stack: Some(stack.clone()),
        font_size: 14.,
        font_revision: stack.cache_id(),
    };
    let text = "AViW日、⏻AViW日、⏻";
    let mut advance = 0.;
    for (offset, ch) in text.char_indices() {
        advance += measure.advance(ch);
        let prefix = &text[..offset + ch.len_utf8()];
        assert_eq!(
            TextMeasure::rounded_width(advance),
            measure_text_sized_with_font_stack(prefix, 14., &stack).0 as f32
        );
    }
}

#[test]
fn multiline_measurement_and_paint_use_the_same_line_advance() {
    let stack = stack();
    let glyphs = rasterize_text_with_font_stack("日\n日\n日", 14., 1000, &stack);
    let line = line_height_sized_with_font_stack(14., &stack);
    assert!((glyphs[1].y - glyphs[0].y - line as i32).abs() <= 1);
    assert!((glyphs[2].y - glyphs[1].y - line as i32).abs() <= 1);
    let (_, height) = measure_text_sized_with_font_stack("日\n日\n日", 14., &stack);
    assert!(glyphs[2].y + glyphs[2].height as i32 <= height as i32);
}

#[test]
fn missing_join_and_variation_controls_add_neither_ink_nor_width() {
    let stack = stack();
    let decorated = "A\u{200b}\u{200c}\u{200d}\u{2060}\u{feff}\u{fe0e}\u{fe0f}\u{e0100}日";
    assert_eq!(
        measure_text_sized_with_font_stack(decorated, 14., &stack),
        measure_text_sized_with_font_stack("A日", 14., &stack)
    );
    assert_eq!(
        rasterize_text_with_font_stack(decorated, 14., 1250, &stack),
        rasterize_text_with_font_stack("A日", 14., 1250, &stack)
    );
    let measure = TextMeasure {
        font_stack: Some(stack.clone()),
        font_size: 14.,
        font_revision: stack.cache_id(),
    };
    for ch in decorated.chars().filter(|ch| is_zero_width_format(*ch)) {
        assert_eq!(measure.advance(ch), 0.0);
    }
}
