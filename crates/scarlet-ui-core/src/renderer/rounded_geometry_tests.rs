use super::*;

// Signed distance to a rounded box: independent of polygon tessellation and
// nearest-point clipping. Coordinates and radii here are physical pixels.
fn distance(rect: Rect, radius: f32, x: f32, y: f32) -> f32 {
    let hw = rect.size.width / 2.0;
    let hh = rect.size.height / 2.0;
    let r = radius.max(0.0).min(hw).min(hh);
    let qx = (x - rect.origin.x - hw).abs() - (hw - r);
    let qy = (y - rect.origin.y - hh).abs() - (hh - r);
    libm::sqrtf(qx.max(0.0).powi(2) + qy.max(0.0).powi(2)) + qx.max(qy).min(0.0) - r
}

#[test]
fn rounded_paths_include_all_eight_tangent_points() {
    let rect = Rect::from_xywh(4., 6., 120., 44.);
    for radius in [0.0_f32, 10., 100.] {
        let r = radius.min(22.0_f32);
        let tangents = [
            (4. + r, 6.),
            (124. - r, 6.),
            (124., 6. + r),
            (124., 50. - r),
            (124. - r, 50.),
            (4. + r, 50.),
            (4., 50. - r),
            (4., 6. + r),
        ];
        for scale_milli in [1000, 1250, 2000, 3000] {
            let scale = scale_milli as f32 / 1000.;
            let mut renderer =
                CpuPaintRenderer::new(Size::new(132., 56.), scale_milli, Color::CLEAR);
            renderer.rebuild_scaled_rounded_rect(rect, radius);
            for (path, scale) in [
                (path_rounded_rect(rect, radius), 1.),
                (renderer.scratch.scaled_points.clone(), scale),
            ] {
                for (x, y) in tangents {
                    assert!(
                        path.iter().any(|p| (p.x - x * scale).abs() < 0.0001
                            && (p.y - y * scale).abs() < 0.0001),
                        "missing tangent ({x}, {y}), radius={radius}, scale={scale}"
                    );
                }
            }
        }
    }
}

#[test]
fn rounded_fill_is_symmetric_and_matches_circle_within_tessellation_error() {
    for (width, height, radius) in [(120., 44., 10.0_f32), (13., 9., 99.), (20., 20., 10.)] {
        for scale_milli in [1000, 1250, 2000, 3000] {
            let scale = scale_milli as f32 / 1000.;
            let rect = Rect::from_xywh(4., 4., width, height);
            let physical = Rect::from_xywh(4. * scale, 4. * scale, width * scale, height * scale);
            let r = radius.min(width / 2.).min(height / 2.);
            let segments = libm::ceilf(r * 0.75).clamp(8., 32.);
            // Maximum inward error of an inscribed arc segment (its sagitta).
            let error =
                r * scale * (1. - libm::cosf(core::f32::consts::PI / (4. * segments))) + 0.0001;
            for use_path in [false, true] {
                let mut ctx = PaintContext::new();
                if use_path {
                    ctx.fill_path(path_rounded_rect(rect, radius), Color::WHITE);
                } else {
                    ctx.fill_rounded_rect(rect, radius, Color::WHITE);
                }
                let mut renderer = CpuPaintRenderer::new(
                    Size::new(width + 8., height + 8.),
                    scale_milli,
                    Color::CLEAR,
                );
                renderer.execute(&ctx);
                let buffer = renderer.buffer();
                for y in 0..buffer.height() {
                    for x in 0..buffer.width() {
                        let filled = buffer.get_pixel(x, y).unwrap() == Color::WHITE.to_bgra();
                        let d = distance(physical, radius * scale, x as f32 + 0.5, y as f32 + 0.5);
                        if d < -error || d > 0.0001 {
                            assert_eq!(
                                filled,
                                d <= 0.,
                                "size=({width},{height}) radius={radius} scale={scale} path={use_path} pixel=({x},{y}) d={d} tolerance={error}"
                            );
                        }
                        // Reflection maps pixel centers to pixel centers only
                        // when the scaled rectangle extent is integral.
                        if physical.size.width.fract() == 0. && physical.size.height.fract() == 0. {
                            assert_eq!(
                                filled,
                                buffer.get_pixel(buffer.width() - 1 - x, y).unwrap()
                                    == Color::WHITE.to_bgra()
                            );
                            assert_eq!(
                                filled,
                                buffer.get_pixel(x, buffer.height() - 1 - y).unwrap()
                                    == Color::WHITE.to_bgra()
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn negative_radius_fill_matches_square_at_each_scale() {
    for scale in [1000, 1250, 2000] {
        let rect = Rect::from_xywh(4., 4., 32., 20.);
        let mut expected = CpuPaintRenderer::new(Size::new(40., 28.), scale, Color::CLEAR);
        let mut square = PaintContext::new();
        square.fill_rect(rect, Color::WHITE);
        expected.execute(&square);
        for use_path in [false, true] {
            let mut ctx = PaintContext::new();
            if use_path {
                ctx.fill_path(path_rounded_rect(rect, -4.), Color::WHITE);
            } else {
                ctx.fill_rounded_rect(rect, -4., Color::WHITE);
            }
            let mut actual = CpuPaintRenderer::new(Size::new(40., 28.), scale, Color::CLEAR);
            actual.execute(&ctx);
            assert_eq!(actual.buffer().as_slice(), expected.buffer().as_slice());
        }
    }
}

#[test]
fn rounded_border_uses_clamped_outer_radius_before_insetting() {
    for scale_milli in [1000, 1250, 2000] {
        for radius in [0., 3., 100.] {
            for thickness in [1., 4., 12.] {
                let mut ctx = PaintContext::new();
                ctx.stroke_rounded_rect(
                    Rect::from_xywh(4., 4., 32., 20.),
                    radius,
                    thickness,
                    Color::WHITE,
                );
                let mut renderer =
                    CpuPaintRenderer::new(Size::new(40., 28.), scale_milli, Color::CLEAR);
                renderer.execute(&ctx);
                let s = scale_milli as f32 / 1000.;
                let outer = Rect::from_xywh(4. * s, 4. * s, 32. * s, 20. * s);
                let r = radius.min(10.) * s;
                let t = thickness * s;
                let inner = Rect::from_xywh(
                    outer.origin.x + t,
                    outer.origin.y + t,
                    outer.size.width - 2. * t,
                    outer.size.height - 2. * t,
                );
                for y in 0..renderer.buffer().height() {
                    for x in 0..renderer.buffer().width() {
                        let px = x as f32 + 0.5;
                        let py = y as f32 + 0.5;
                        let inside_outer = distance(outer, r, px, py) <= 0.;
                        let inside_inner = inner.size.width > 0.
                            && inner.size.height > 0.
                            && distance(inner, (r - t).max(0.), px, py) <= 0.;
                        assert_eq!(
                            renderer.buffer().get_pixel(x, y).unwrap() == Color::WHITE.to_bgra(),
                            inside_outer && !inside_inner,
                            "radius={radius} thickness={thickness} scale={s} point=({x},{y})"
                        );
                    }
                }
            }
        }
    }
}
