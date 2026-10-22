//! Opt-in CPU benchmark; no wall-clock thresholds in the test suite.
//! Run with `cargo test -p scarlet-ui-core --release text_view_cpu_benchmark
//! -- --ignored --nocapture --test-threads=1`.

use super::*;
use core::hint::black_box;
use std::time::Instant;

#[test]
#[ignore = "release CPU benchmark, not a timing assertion"]
fn text_view_cpu_benchmark() {
    // Resolve fonts before timing so discovery is not charged to the first case.
    black_box(graphics::measure_text_sized(
        "ASCII 日本語 e\u{301} 👩‍💻",
        15.0,
    ));
    let sizes = std::env::var("TEXT_BENCH_LINES").unwrap_or_else(|_| "100,1000,3000".into());
    let samples: usize = std::env::var("TEXT_BENCH_SAMPLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);
    assert!(samples > 0);
    for count in sizes
        .split(',')
        .map(|value| value.parse::<usize>().unwrap())
    {
        let text: String = (0..count).map(|line| alloc::format!(
            "{line:06}: A unique editor row, 日本語 e\u{301} 👩‍💻\tand more text for wrapping.\n"
        )).collect();
        let bytes = text.len();
        let value = State::new(StateId::new(6000), text);
        let selection = State::new(StateId::new(6001), TextSelection::collapsed(0));
        let scroll = State::new(StateId::new(6002), TextViewScroll::default());
        let view = TextView::new(value, selection)
            .scroll_state(scroll)
            .wrap_mode(WrapMode::Soft)
            .line_numbers(true)
            .font_size(15.0)
            .padding(12.0);
        let started = Instant::now();
        let mut object = render_object_with_size(&view, 640.0, 480.0);
        let initial = started.elapsed().as_secs_f64() * 1000.0;
        for rebuild in [false, true] {
            let mut timings = Vec::new();
            for sample in 0..samples {
                let start = Instant::now();
                handle_text_view_mouse(
                    &view,
                    &mut object,
                    &MouseEvent::Wheel {
                        delta_x: 0,
                        delta_y: if sample % 2 == 0 { -31 } else { 29 },
                        x: 100,
                        y: 100,
                        phase: WheelPhase::Moved,
                        source: ScrollSource::Trackpad,
                    },
                );
                if rebuild {
                    object.update(&view);
                    object.layout(LayoutConstraints::tight(640.0, 480.0));
                    let mut paint = PaintContext::new();
                    object.paint(&mut paint, Point::ZERO);
                    black_box(paint);
                }
                black_box(&object.layout);
                timings.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            timings.sort_by(f64::total_cmp);
            std::println!(
                "text_view_cpu lines={count} bytes={bytes} rebuild_and_record={rebuild} initial_ms={initial:.3} samples={samples} median_ms={:.6} p95_ms={:.6}",
                timings[samples / 2],
                timings[(samples * 95 / 100).min(samples - 1)]
            );
        }
    }
}

#[test]
#[ignore = "release long-line CPU benchmark, no GPU presentation"]
fn text_view_long_line_benchmark() {
    black_box(graphics::measure_text_sized(
        "ASCII 日本語 e\u{301} 👩‍💻",
        15.0,
    ));
    for repeats in [100, 1000, 10000] {
        let text = "日本 e\u{301} 👩‍💻 abcdefghijklmnopqrstuvwxyz ".repeat(repeats);
        let bytes = text.len();
        let view = TextView::with_document(
            State::new(StateId::new(6010), TextDocument::from_str(&text)),
            State::new(StateId::new(6011), TextSelection::collapsed(text.len() / 2)),
        )
        .wrap_mode(WrapMode::None)
        .autofocus(true);
        let start = Instant::now();
        let object = render_object_with_size(&view, 640.0, 90.0);
        let initial = start.elapsed().as_secs_f64() * 1000.0;
        let line = &object.layout.visual_lines[0];
        let start = Instant::now();
        for index in 0..10000 {
            black_box(
                object
                    .layout
                    .byte_at_line_x(0, line.width * ((index % 101) as f32 / 100.0)),
            );
            black_box(object.cursor_rect());
        }
        let queries = start.elapsed().as_secs_f64() * 1000.0 / 10000.0;
        let start = Instant::now();
        for _ in 0..20 {
            let mut paint = PaintContext::new();
            object.paint(&mut paint, Point::ZERO);
            black_box(paint);
        }
        let paint = start.elapsed().as_secs_f64() * 1000.0 / 20.0;
        std::println!(
            "text_view_long_line bytes={bytes} initial_ms={initial:.3} hit_and_caret_ms={queries:.6} record_ms={paint:.6}"
        );
    }
}
