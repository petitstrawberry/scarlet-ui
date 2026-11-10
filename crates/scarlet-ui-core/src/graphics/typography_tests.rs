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
fn fractional_baselines_use_the_nearest_physical_pixel() {
    let font = stack();
    // The fixture's ascent is 1160 units/em and ink top is 740 units/em.
    // At 12px its physical baselines are 13.92, 17.4 and 27.84 pixels.
    for (dpi, ink_top) in [(1000, 5), (1250, 5), (2000, 10)] {
        let glyphs = rasterize_text_with_font_stack("A", 12., dpi, &font);
        assert_eq!(glyphs[0].y, ink_top, "dpi={dpi}");
    }
}

#[test]
fn fractional_origins_snap_on_the_output_grid_without_an_upward_bias() {
    // A 17px line centered in a 36px control starts at 9.5, not 9.
    assert_eq!(text_origin_physical(9.5, 1000), 10);
    assert_eq!(text_origin_physical(9.5, 1250), 12);
    assert_eq!(text_origin_physical(9.5, 2000), 19);
    for dpi in [1000, 2000, 3000] {
        for x in [-20.75, -10.5, -0.5, 0.25, 9.5] {
            assert_eq!(
                text_origin_physical(x + 20., dpi) - text_origin_physical(x, dpi),
                20 * (dpi / 1000) as i32,
            );
        }
    }
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

#[test]
fn centered_controls_use_fractional_ascent_descent_without_line_leading() {
    let stack = stack();
    for size in [9.5, 13.0, 17.25] {
        let metrics = font_metrics_sized_with_font_stack(size, &stack);
        assert!((metrics.ascent - size * 1.160).abs() < 0.0001);
        assert!((metrics.descent - size * 0.288).abs() < 0.0001);
        for height in [24.0, 31.0, 40.0] {
            let baseline = metrics.centered_baseline(height);
            let above = baseline - metrics.ascent;
            let below = height - baseline - metrics.descent;
            assert!((above - below).abs() < 0.0001);
            let spaced = FontMetrics {
                leading: 9.75,
                ..metrics
            };
            assert_eq!(spaced.centered_baseline(height), baseline);
            // Integer line-height placement must not silently return here.
            let rounded_top =
                (height - line_height_sized_with_font_stack(size, &stack) as f32) * 0.5;
            assert!((above - rounded_top).abs() > 0.01);
        }
    }
}

#[test]
fn centered_label_baseline_rounds_once_on_the_physical_grid() {
    let stack = stack();
    let mut independent_rounding_would_differ = false;
    for size in [9.5, 13.0, 17.25] {
        let logical = font_metrics_sized_with_font_stack(size, &stack);
        for dpi in [1000, 1250, 1500, 2000, 3000] {
            let scale = dpi as f32 / 1000.0;
            let physical = font_metrics_sized_with_font_stack(size * scale, &stack);
            for height in [24.0, 31.0, 40.0] {
                for translation in [-10.25, 0.0, 0.5, 20.25] {
                    let baseline = logical.centered_baseline(height) + translation;
                    let top = baseline - logical.ascent;
                    let glyph_baseline = libm::floorf(physical.ascent + 0.5) as i32;
                    let actual =
                        text_origin_y_physical(top, size, dpi, Some(&stack)) + glyph_baseline;
                    let expected = text_origin_physical(baseline, dpi);
                    assert_eq!(actual, expected, "size={size}, dpi={dpi}, height={height}");
                    independent_rounding_would_differ |=
                        text_origin_physical(top, dpi) + glyph_baseline != expected;
                }
            }
        }
    }
    assert!(
        independent_rounding_would_differ,
        "fixture must catch double rounding"
    );
}

fn shaping_stack() -> FontStack {
    FontStack::new(include_bytes!("../testing/fonts/shaping.ttf")).unwrap()
}

#[test]
fn opentype_positions_and_carets_share_the_shaped_string() {
    let font = shaping_stack();
    let layout = shaping::shape_text("AVfiA\u{301}", 20., &font);
    // GPOS A/V -120, GSUB fi 510 instead of 350+280, combining acute zero.
    assert!((layout.width - (700. - 120. + 690. + 510. + 700.) * 0.02).abs() < 0.001);
    assert_eq!(layout.glyphs.len(), 5);
    assert!((layout.glyphs[1].x - 11.6).abs() < 0.001);
    assert!((layout.glyphs[4].x - (35.6 + 7.)).abs() < 0.001);
    assert!((layout.glyphs[4].y + 16.).abs() < 0.001);
    let measure = TextMeasure::with_font_stack(20., &font);
    assert_eq!(measure.prefix_widths("AVfiA\u{301}"), layout.prefix_widths);
    assert_eq!(*layout.prefix_widths.last().unwrap(), layout.width);
    assert_eq!(measure_text_sized_with_font_stack("AVfiA\u{301}", 20., &font).0, 50);
}

#[test]
fn fractional_glyph_coverage_preserves_spacing_instead_of_truncating_each_origin() {
    let font = shaping_stack();
    for scale in [1000, 1250, 1500, 2000] {
        for text in ["AVAVAV", "iiiiiiii", "fi fi A\u{301}"] {
            let size = 13.25 * scale as f32 / 1000.;
            let layout = shaping::shape_text(text, size, &font);
            let masks = rasterize_text_with_font_stack(text, 13.25, scale, &font);
            let scaled = font.primary.as_scaled(em_scale(&font.primary, size));
            let mut index = 0;
            for glyph in layout.glyphs {
                let x = libm::floorf(glyph.x * 4. + 0.5) / 4.;
                let baseline = libm::floorf(scaled.ascent() + glyph.y + 0.5);
                let outline = font.primary.outline_glyph(ab_glyph::GlyphId(glyph.id)
                    .with_scale_and_position(scaled.scale, point(x, baseline)));
                let Some(outline) = outline else { continue };
                let bounds = outline.px_bounds();
                let mask = &masks[index];
                assert_eq!((mask.x, mask.y), (bounds.min.x as i32, bounds.min.y as i32));
                assert_eq!((mask.width, mask.height), (bounds.width() as u32, bounds.height() as u32));
                // Independent full-position outline vs cached fractional mask.
                outline.draw(|px, py, coverage| {
                    let actual = mask.mask[(py * mask.width + px) as usize];
                    assert!((actual as i32 - (coverage * 255.) as i32).abs() <= 1);
                });
                index += 1;
            }
            assert_eq!(index, masks.len());
        }
    }
    let masks = rasterize_text_with_font_stack("iiii", 13., 1000, &font);
    assert_ne!(masks[0].key.subpixel_x, masks[1].key.subpixel_x);
    assert_ne!(masks[0].mask, masks[1].mask);
}

#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
#[test]
fn shaping_uses_the_selected_collection_face() {
    let face = discovered_font_ref(DiscoveredFont {
        bytes: include_bytes!("../testing/fonts/shaping-collection.ttc").to_vec(), index: 1,
    }).unwrap();
    let font = FontStack { primary: face, fallbacks: Vec::new(), cache_id: next_font_stack_id() };
    let expected = shaping::shape_text("AVfi", 20., &shaping_stack());
    let actual = shaping::shape_text("AVfi", 20., &font);
    assert_eq!(actual.width, expected.width);
    assert_eq!(actual.glyphs.len(), 3);
    assert_eq!(rasterize_text_with_font_stack("AVfi", 20., 2000, &font).len(), 3);
}
