//! Opt-in CPU benchmark; no wall-clock thresholds in the test suite.
//! Run with `cargo test -p scarlet-ui-core --release text_view_cpu_benchmark
//! -- --ignored --nocapture --test-threads=1`.

use super::*;
use core::hint::black_box;
use std::time::Instant;

/// Same public-API workload before/after document storage changes. Snapshot
/// retention is deliberately inside timing; String-reference checks are outside.
#[test]
#[ignore = "release document-core edit benchmark; no view layout or paint"]
fn text_document_edit_benchmark() {
    let lines =
        std::env::var("DOCUMENT_BENCH_LINES").unwrap_or_else(|_| "1000,10000,100000".into());
    let samples: usize = std::env::var("DOCUMENT_BENCH_SAMPLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);
    let operations: usize = std::env::var("DOCUMENT_BENCH_EDITS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    assert!(samples > 0 && operations > 0);
    std::println!(
        "document_bench_environment manifest={} debug_assertions={} retained_snapshots=4",
        env!("CARGO_MANIFEST_DIR"),
        cfg!(debug_assertions)
    );
    let clamp = |text: &str, byte: usize| {
        let mut byte = byte.min(text.len());
        while !text.is_char_boundary(byte) {
            byte -= 1;
        }
        byte
    };
    let hash = |text: &str| {
        text.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        })
    };
    for count in lines.split(',').map(|s| s.parse::<usize>().unwrap()) {
        let source =
            "A local edit row: 日本語 e\u{301} 👩‍💻 🇯🇵; persistent snapshots\r\n".repeat(count);
        let base = TextDocument::from_str(&source);
        for materialize in [false, true] {
            let mut timings = Vec::new();
            let mut expected_hash = None;
            for _ in 0..samples {
                let mut document = base.clone();
                let mut snapshots = Vec::new();
                let mut edits = Vec::new();
                // Generate String-reference edits before timing, including their
                // intentionally non-character-aligned raw byte positions.
                let mut reference = source.clone();
                let mut expected_snapshots = Vec::new();
                for edit in 0..operations {
                    let raw_start = reference.len() / 2 + edit * 17;
                    let raw_end = raw_start + edit % 5;
                    let start = clamp(&reference, raw_start);
                    let end = clamp(&reference, raw_end).max(start);
                    let replacement = ["x", "\n日本", "e\u{301}", "", "👨‍👩‍👧‍👦"][edit % 5];
                    edits.push((raw_start..raw_end, replacement));
                    expected_snapshots.push(reference.clone());
                    if expected_snapshots.len() > 4 {
                        expected_snapshots.remove(0);
                    }
                    reference.replace_range(start..end, replacement);
                }
                let started = Instant::now();
                for (range, replacement) in &edits {
                    snapshots.push(document.clone());
                    if snapshots.len() > 4 {
                        snapshots.remove(0);
                    }
                    document = document.replace(range.clone(), replacement).0;
                    black_box(document.line_count());
                    black_box(document.line_index_at_byte(range.start));
                    black_box(
                        document.clamp_position(TextPosition::new(range.start + replacement.len())),
                    );
                    if materialize {
                        black_box(document.as_str());
                    }
                }
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(document.as_str(), reference);
                assert_eq!(
                    document.line_count(),
                    reference.bytes().filter(|b| *b == b'\n').count()
                        + usize::from(!reference.is_empty())
                );
                for (snapshot, expected) in snapshots.iter().zip(&expected_snapshots) {
                    assert_eq!(snapshot.as_str(), expected.as_str());
                }
                let witness = hash(&document.as_str());
                assert_eq!(*expected_hash.get_or_insert(witness), witness);
                timings.push(elapsed_ms / operations as f64);
            }
            timings.sort_by(f64::total_cmp);
            std::println!(
                "document_edit lines={count} bytes={} materialize={materialize} edits={operations} samples={samples} checksum={:016x} median_ms_per_edit={:.6} p95_ms_per_edit={:.6} sample_ms_per_edit={timings:?}",
                source.len(),
                expected_hash.unwrap(),
                timings[samples / 2],
                timings[(samples * 95 / 100).min(samples - 1)]
            );
        }
    }
}

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

// BEGIN VERIFIED SCROLL BENCHMARK
// This section uses APIs shared by the original app pin and the optimized
// implementation so the identical harness can be injected into git archives.
// Verification and paint-command inspection happen outside the timed interval.

fn verified_paint_witness(paint: &PaintContext<'_>) -> (usize, usize, u64) {
    let mut draw_text = 0usize;
    let mut body_text = 0usize;
    let mut hash = 0xcbf29ce484222325u64;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash = (hash ^ *byte as u64).wrapping_mul(0x100000001b3);
        }
    };
    for command in paint.commands() {
        if let PaintCommand::DrawText {
            position,
            text,
            font_size_px,
            ..
        } = command
        {
            draw_text += 1;
            body_text += usize::from(text.contains(": A unique editor row"));
            feed(&position.x.to_bits().to_le_bytes());
            feed(&position.y.to_bits().to_le_bytes());
            feed(&font_size_px.to_bits().to_le_bytes());
            feed(&(text.len() as u64).to_le_bytes());
            feed(text.as_bytes());
        }
    }
    (draw_text, body_text, hash)
}

fn verified_benchmark_fonts() {
    if let Ok(paths) = std::env::var("TEXT_BENCH_FONT_FILES") {
        let mut paths = paths.split(':');
        let bytes = |path: &str| -> &'static [u8] {
            Box::leak(
                std::fs::read(path)
                    .expect("benchmark font")
                    .into_boxed_slice(),
            )
        };
        let mut stack = graphics::FontStack::new(bytes(paths.next().expect("primary font")))
            .expect("valid primary font");
        for path in paths {
            stack
                .add_fallback(bytes(path))
                .expect("valid fallback font");
        }
        graphics::set_default_font_stack(stack);
    }
    black_box(graphics::measure_text_sized(
        "ASCII 日本語 e\u{301} 👩‍💻",
        15.0,
    ));
}

#[test]
#[ignore = "verified release CPU benchmark; no rasterization or presentation"]
fn text_view_verified_scroll_benchmark() {
    let compiled_manifest = env!("CARGO_MANIFEST_DIR");
    if let Ok(expected) = std::env::var("TEXT_BENCH_EXPECT_MANIFEST_DIR") {
        assert_eq!(
            compiled_manifest, expected,
            "benchmark executable must come from the requested source snapshot"
        );
    }
    std::println!(
        "text_view_verified_environment compiled_manifest={compiled_manifest} debug_assertions={}",
        cfg!(debug_assertions)
    );
    verified_benchmark_fonts();
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
        let value = State::new(StateId::new(6020), text);
        let selection = State::new(StateId::new(6021), TextSelection::collapsed(0));
        let scroll = State::new(StateId::new(6022), TextViewScroll::default());
        let view = TextView::new(value, selection)
            .scroll_state(scroll.clone())
            .wrap_mode(WrapMode::Soft)
            .line_numbers(true)
            .font_size(15.0)
            .padding(12.0);
        let started = Instant::now();
        let mut object = render_object_with_size(&view, 640.0, 480.0);
        let initial_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut previous_hash = {
            let mut paint = PaintContext::new();
            assert!(object.paint(&mut paint, Point::ZERO));
            verified_paint_witness(&paint).2
        };
        for record in [false, true] {
            let mode = if record {
                "wheel_update_layout_record"
            } else {
                "wheel_only"
            };
            let mut timings = Vec::new();
            for sample in 0..samples {
                let delta_y = if sample % 2 == 0 { -31 } else { 29 };
                let before_y = object.scroll.y;
                let before_row_y = object.layout.visual_lines[0].y;
                let mut paint = PaintContext::new();
                let started = Instant::now();
                let handled = handle_text_view_mouse(
                    &view,
                    &mut object,
                    &MouseEvent::Wheel {
                        delta_x: 0,
                        delta_y,
                        x: 100,
                        y: 100,
                        phase: WheelPhase::Moved,
                        source: ScrollSource::Trackpad,
                    },
                );
                let mut painted = false;
                if record {
                    object.update(&view);
                    object.layout(LayoutConstraints::tight(640.0, 480.0));
                    painted = object.paint(&mut paint, Point::ZERO);
                }
                black_box(&object.layout);
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                // Assertions, witness hashing and recording in wheel-only mode
                // are deliberately outside the measured interval.
                if !record {
                    painted = object.paint(&mut paint, Point::ZERO);
                }
                let (draw_text, body_text, hash) = verified_paint_witness(&paint);
                let after_y = object.scroll.y;
                assert!(handled && painted);
                assert_eq!(
                    after_y,
                    before_y - delta_y as f32,
                    "wheel must move by the requested pixels without clamping"
                );
                assert_eq!(
                    scroll.get(),
                    object.scroll,
                    "external scroll state must agree"
                );
                assert_eq!(
                    object.layout.visual_lines[0].y - before_row_y,
                    -(after_y - before_y),
                    "row coordinates must move with the viewport"
                );
                assert!(
                    draw_text > 0 && body_text > 0,
                    "record visible document text"
                );
                assert_ne!(
                    hash, previous_hash,
                    "text content or positions must change on every sample"
                );
                previous_hash = hash;
                std::println!(
                    "text_view_verified_sample lines={count} mode={mode} sample={sample} before_y={before_y} after_y={after_y} visible_start={} visible_end={} visual_rows={} commands={} draw_text={draw_text} body_text={body_text} text_hash={hash:016x} elapsed_ms={elapsed_ms:.6}",
                    object.layout.visible_lines.start,
                    object.layout.visible_lines.end,
                    object.layout.visual_lines.len(),
                    paint.commands().len(),
                );
                timings.push(elapsed_ms);
            }
            timings.sort_by(f64::total_cmp);
            std::println!(
                "text_view_verified_summary lines={count} bytes={bytes} mode={mode} initial_ms={initial_ms:.3} samples={samples} median_ms={:.6} max_ms={:.6}",
                timings[samples / 2],
                timings[samples - 1],
            );
        }
    }
}
