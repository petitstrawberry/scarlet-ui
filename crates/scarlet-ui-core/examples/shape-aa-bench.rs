use scarlet_ui_core::{
    prelude::*,
    renderer::{CpuPaintRenderer, PaintContext},
};
use std::time::Instant;
fn main() {
    for scale in [1000, 2000] {
        for case in ["controls", "rounded_window", "effects"] {
            let mut ctx = PaintContext::new();
            if case == "rounded_window" {
                ctx.push_rounded_clip(Rect::from_xywh(0., 0., 1000., 800.), 12.);
            }
            ctx.fill_rect(Rect::from_xywh(0., 0., 1000., 800.), Color::WHITE);
            for row in 0..16 {
                for col in 0..5 {
                    let r =
                        Rect::from_xywh(12. + col as f32 * 196., 12. + row as f32 * 48., 180., 36.);
                    if case == "effects" {
                        ctx.draw_rounded_rect_shadow(
                            r,
                            7.,
                            Offset::new(0., 2.),
                            5.,
                            0.,
                            Color::rgba(0, 0, 0, 55),
                        );
                        ctx.fill_vertical_gradient_rounded_rect(
                            r,
                            7.,
                            Color::rgb(252, 253, 255),
                            Color::rgb(232, 234, 239),
                        );
                    } else {
                        ctx.fill_rounded_rect(r, 7., Color::rgb(242, 243, 246));
                    }
                    ctx.stroke_rounded_rect(r, 7., 0.75, Color::rgb(180, 183, 190));
                    ctx.fill_circle(
                        Point::new(r.left() + 160., r.top() + 18.),
                        7.,
                        Color::rgb(200, 30, 60),
                    );
                    ctx.draw_line(
                        Point::new(r.left() + 12., r.top() + 12.),
                        Point::new(r.left() + 24., r.top() + 24.),
                        1.,
                        Color::rgb(100, 105, 115),
                    );
                }
            }
            if case == "rounded_window" {
                ctx.pop_clip();
            }
            let mut r = CpuPaintRenderer::new(Size::new(1000., 800.), scale, Color::WHITE);
            for _ in 0..8 {
                r.execute(&ctx);
            }
            let mut samples = Vec::new();
            for _ in 0..80 {
                let t = Instant::now();
                r.execute(&ctx);
                std::hint::black_box(r.buffer());
                samples.push(t.elapsed().as_secs_f64() * 1000.);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "{case},{scale},median_ms={:.3},p95_ms={:.3}",
                samples[40], samples[76]
            );
        }
    }
}
