//! Repeatable CPU backdrop benchmark. Run with `cargo run -p scarlet-ui-core
//! --release --example backdrop-bench`. Timings include a full scene replay,
//! actual backdrop sampling, three separable passes, clipping, and foreground.
use scarlet_ui_core::{
    prelude::*,
    renderer::{CpuPaintRenderer, PaintContext},
};
use std::time::Instant;

fn scene(controls: usize, rounded_window: bool, scale: u32) -> PaintContext<'static> {
    let mut ctx = PaintContext::new();
    if rounded_window {
        ctx.push_rounded_clip(Rect::from_xywh(0.0, 0.0, 1000.0, 800.0), 12.0);
    }
    // Prebuild a busy opaque image outside the timed loop so the baseline
    // distinguishes material cost from unrelated vector-path rasterization.
    let mut background =
        scarlet_ui_core::buffer::Buffer::from_logical_dimensions_with_scale(1000, 800, scale);
    let width = background.width() as usize;
    for y in 0..background.height() as usize {
        for x in 0..width {
            background.as_mut_slice()[y * width + x] = if x / (10 * scale as usize / 1000) % 2 == 0
            {
                Color::rgb(130, 155, 200).to_bgra()
            } else {
                Color::rgb(200, 210, 225).to_bgra()
            };
        }
    }
    ctx.draw_temporary_buffer(Rect::from_xywh(0.0, 0.0, 1000.0, 800.0), background);
    for control in 0..controls {
        let col = control % 5;
        let row = control / 5;
        let rect = Rect::from_xywh(
            12.0 + col as f32 * 196.0,
            12.0 + row as f32 * 48.0,
            180.0,
            36.0,
        );
        ctx.backdrop_filter_rounded_rect(rect, 7.0, 15.0, Color::rgba(255, 255, 255, 110));
        ctx.stroke_rounded_rect(rect, 7.0, 0.75, Color::rgba(255, 255, 255, 120));
        ctx.fill_rect(
            Rect::from_xywh(rect.left() + 12.0, rect.top() + 16.0, 70.0, 2.0),
            Color::rgb(30, 40, 65),
        );
    }
    if rounded_window {
        ctx.pop_clip();
    }
    ctx
}

fn main() {
    println!("case,scale,median_ms,p95_ms,opaque_fallbacks");
    for scale in [1000, 2000] {
        for (name, controls, rounded) in [
            ("baseline", 0, false),
            ("one", 1, false),
            ("three", 3, false),
            ("24_controls", 24, false),
            ("80_controls", 80, false),
            ("80_rounded_window", 80, true),
        ] {
            let ctx = scene(controls, rounded, scale);
            let mut renderer = CpuPaintRenderer::new(Size::new(1000.0, 800.0), scale, Color::WHITE);
            for _ in 0..3 {
                renderer.execute(&ctx);
            }
            let mut samples = Vec::new();
            for _ in 0..20 {
                let start = Instant::now();
                renderer.execute(&ctx);
                std::hint::black_box(renderer.buffer());
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "{name},{scale},{:.3},{:.3},{}",
                samples[10],
                samples[19],
                renderer.backdrop_fallback_count()
            );
        }
    }
}
