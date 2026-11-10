//! Coalesce compatible transient draws across disjoint paint, preserving every
//! overlap and destination-reading barrier. Geometry already includes clipping
//! and raster snapping; large retained GPU meshes keep their replay path.
use super::*;
use core::ops::Range;

const SEARCH_WINDOW: usize = 64;

struct Batch {
    draw: Draw,
    ranges: Vec<Range<usize>>,
    bounds: Option<FloatRect>,
    range_bounds: Vec<FloatRect>,
}

pub(super) fn batch_draws(encoder: &SgfxPaintEncoder, frame: &mut LoweredFrame<'_>) -> Result<()> {
    if frame.draws.len() < 32 {
        return Ok(());
    }
    let stride = PAINT_VERTEX_STRIDE as usize;
    for index in 0..frame.draws.len() {
        let mut draw = frame.draws[index];
        let Some(bytes) = encoder.batch_vertices(&draw) else {
            continue;
        };
        if frame.vertex_bytes.len().saturating_add(bytes.len()) > MAX_FRAME_VERTICES * stride {
            continue;
        }
        append_translated_vertices(frame, &mut draw, bytes)?;
        frame.draws[index] = draw;
    }
    let mut batches: Vec<Batch> = Vec::with_capacity(frame.draws.len());
    let mut merged = false;
    let mut clear_scissors = Vec::new();
    for original in &frame.draws {
        let mut draw = *original;
        let range = if draw.vertex_buffer.is_none() {
            let start = (draw.geometry.first_vertex as usize)
                .checked_mul(stride)
                .ok_or(Error::InvalidFrame)?;
            let end = (draw.geometry.vertex_count as usize)
                .checked_mul(stride)
                .and_then(|n| start.checked_add(n))
                .ok_or(Error::InvalidFrame)?;
            if end > frame.vertex_bytes.len() {
                return Err(Error::InvalidFrame);
            }
            Some(start..end)
        } else {
            None
        };
        // Each fallback recording carries a clear-only sentinel. One per
        // scissor is sufficient for empty damage passes; duplicates consume
        // batch search slots and allocations without drawing any pixels.
        if matches!(draw.source, DrawSource::ClearOnly)
            && draw.vertex_buffer.is_none()
            && draw.offset == [0., 0.]
        {
            if clear_scissors.contains(&draw.geometry.scissor) {
                merged = true;
                continue;
            }
            clear_scissors.push(draw.geometry.scissor);
        }
        let bounds = range.as_ref().and_then(|range| {
            if draw.offset != [0., 0.]
                || matches!(draw.source, DrawSource::Backdrop(_) | DrawSource::ClearOnly)
            {
                return None;
            }
            let b = super::retained::vertex_bounds(&frame.vertex_bytes[range.clone()]);
            let clip = draw.geometry.scissor;
            if b.x >= clip.x as f32
                && b.y >= clip.y as f32
                && b.right() <= clip.x.saturating_add(clip.width) as f32
                && b.bottom() <= clip.y.saturating_add(clip.height) as f32
            {
                // Geometry already clipped by the tessellator needs no extra
                // per-item scissor. Keeping it would prevent otherwise identical
                // glyph/shape draws from sharing a batch.
                draw.geometry.scissor = PixelBounds {
                    x: 0,
                    y: 0,
                    width: encoder.width(),
                    height: encoder.height(),
                };
            }
            // Include raster edge rounding. A conservative box can only prevent
            // batching; it can never permit an overlapping draw to move.
            [b.x, b.y, b.width, b.height, b.right(), b.bottom()]
                .iter()
                .all(|v| v.is_finite())
                .then(|| FloatRect::new(b.x - 1., b.y - 1., b.width + 2., b.height + 2.))
        });
        let mut target = None;
        if let Some(bounds) = bounds {
            for index in (batches.len().saturating_sub(SEARCH_WINDOW)..batches.len()).rev() {
                let previous = &batches[index];
                // ClearOnly is a degenerate geometry sentinel. Keep it in the
                // stream, but it has no pixels that can overlap another draw.
                if matches!(previous.draw.source, DrawSource::ClearOnly) {
                    continue;
                }
                let Some(previous_bounds) = previous.bounds else {
                    break;
                };
                if previous.draw.source == draw.source
                    && previous.draw.geometry.scissor == draw.geometry.scissor
                {
                    target = Some(index);
                    break;
                }
                if previous_bounds.intersect(bounds).is_some()
                    && ranges_overlap(
                        previous,
                        range.as_ref().unwrap(),
                        bounds,
                        &frame.vertex_bytes,
                    )
                {
                    break;
                }
            }
        }
        if let Some(index) = target {
            let batch = &mut batches[index];
            let a = batch.bounds.unwrap();
            let b = bounds.unwrap();
            let x = a.x.min(b.x);
            let y = a.y.min(b.y);
            batch.bounds = Some(FloatRect::new(
                x,
                y,
                a.right().max(b.right()) - x,
                a.bottom().max(b.bottom()) - y,
            ));
            batch.ranges.push(range.unwrap());
            batch.range_bounds.push(bounds.unwrap());
            batch.draw.geometry.vertex_count = batch
                .draw
                .geometry
                .vertex_count
                .checked_add(draw.geometry.vertex_count)
                .ok_or(Error::FrameTooComplex)?;
            merged = true;
        } else {
            batches.push(Batch {
                draw,
                ranges: range.into_iter().collect(),
                range_bounds: bounds.into_iter().collect(),
                bounds,
            });
        }
    }
    if !merged {
        return Ok(());
    }
    let mut bytes = Vec::with_capacity(frame.vertex_bytes.len());
    let mut draws = Vec::with_capacity(batches.len());
    for mut batch in batches {
        if batch.draw.vertex_buffer.is_none() {
            batch.draw.geometry.first_vertex =
                u32::try_from(bytes.len() / stride).map_err(|_| Error::FrameTooComplex)?;
            for range in batch.ranges {
                if bytes.len().saturating_add(range.len()) > MAX_FRAME_VERTICES * stride {
                    return Err(Error::FrameTooComplex);
                }
                bytes.extend_from_slice(&frame.vertex_bytes[range]);
            }
        }
        draws.push(batch.draw);
    }
    frame.vertex_bytes = bytes;
    frame.draws = draws;
    Ok(())
}

pub(super) fn append_translated_vertices(
    frame: &mut LoweredFrame<'_>,
    draw: &mut Draw,
    bytes: &[u8],
) -> Result<()> {
    let stride = PAINT_VERTEX_STRIDE as usize;
    if frame.vertex_bytes.len().saturating_add(bytes.len()) > MAX_FRAME_VERTICES * stride {
        return Err(Error::FrameTooComplex);
    }
    let start = frame.vertex_bytes.len();
    frame.vertex_bytes.extend_from_slice(bytes);
    // Replay offsets already include per-source raster snapping.
    for vertex in frame.vertex_bytes[start..].chunks_exact_mut(stride) {
        for (axis, offset) in draw.offset.iter().enumerate() {
            let at = axis * 4;
            let value = f32::from_le_bytes(vertex[at..at + 4].try_into().unwrap()) + offset;
            vertex[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    draw.geometry.first_vertex = (start / stride) as u32;
    draw.vertex_buffer = None;
    draw.offset = [0., 0.];
    Ok(())
}

// A hollow border's bounding box overlaps its label even though its triangles
// do not. Refine only ambiguous boxes, and stop conservatively at a fixed work
// budget so an adversarial mesh cannot make batching quadratic in vertex count.
fn ranges_overlap(
    previous: &Batch,
    current: &Range<usize>,
    bounds: FloatRect,
    bytes: &[u8],
) -> bool {
    let triangle_bytes = PAINT_VERTEX_STRIDE as usize * 3;
    let mut budget = 16384usize;
    for (range, prior_bounds) in previous.ranges.iter().zip(&previous.range_bounds) {
        if prior_bounds.intersect(bounds).is_none() {
            continue;
        }
        for a in bytes[range.clone()].chunks_exact(triangle_bytes) {
            let a = triangle_bounds(a);
            if a.intersect(bounds).is_none() {
                continue;
            }
            for b in bytes[current.clone()].chunks_exact(triangle_bytes) {
                if budget == 0 {
                    return true;
                }
                budget -= 1;
                if a.intersect(triangle_bounds(b)).is_some() {
                    return true;
                }
            }
        }
    }
    false
}

fn triangle_bounds(bytes: &[u8]) -> FloatRect {
    let b = super::retained::vertex_bounds(bytes);
    FloatRect::new(b.x - 1., b.y - 1., b.width + 2., b.height + 2.)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use scarlet_ui_core::geometry::{Point, Rect};

    #[test]
    fn batched_retained_draws_match_ordered_gpu_pixels_at_fractional_scales() {
        super::super::ensure_test_font();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            assert!(
                std::env::var_os("SCARLET_REQUIRE_GPU_TESTS").is_none(),
                "GPU required"
            );
            return;
        };
        eprintln!("batch pixel-test adapter: {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let context = sgfx_backend_wgpu::Device::new(device, queue).create_context();
        let queue = context.create_queue();
        for scale in [1000, 1250, 2000] {
            for (phase, cached) in [
                (0., true),
                (0.25, true),
                (-0.75, true),
                (0., false),
                (0.25, false),
                (-0.75, false),
            ] {
                let (width, height) = (240 * scale / 1000, 120 * scale / 1000);
                let mut encoder = SgfxPaintEncoder::new(width, height, false).unwrap();
                let mut resources = context.create_resources(encoder.resource_table());
                let targets = [
                    encoder.target_texture(0).unwrap(),
                    encoder.target_texture(1).unwrap(),
                ];
                for target in targets {
                    resources
                        .map_image(
                            target,
                            context
                                .create_image(width, height, TextureFormat::Bgra8Unorm)
                                .unwrap(),
                        )
                        .unwrap();
                }
                let mut paint = PaintContext::new();
                // Inherited opacity forces the flat fallback, including its
                // per-recording clear-only sentinels, instead of cached meshes.
                if !cached {
                    paint.set_opacity(0.75);
                }
                paint.push_rounded_clip(Rect::from_xywh(3., 2., 232., 110.), 5.);
                if cached && phase != 0. {
                    // Fill the GPU slot budget in the same frame so the labels
                    // below exercise CPU-record replay at fractional origins.
                    for _ in 0..super::super::retained::MAX_RETAINED_MESHES {
                        let mut tiny = PaintContext::new();
                        tiny.fill_rect(
                            Rect::from_xywh(5., 5., 1., 1.),
                            UiColor::rgba(20, 30, 40, 30),
                        );
                        paint.draw_display_list(Point::ZERO, tiny.into_display_list());
                    }
                }
                for i in 0..40 {
                    let mut local = PaintContext::new();
                    // Some neighboring cells overlap; alpha and text make any
                    // illegal painter-order change observable in the readback.
                    local.fill_rounded_rect(
                        Rect::from_xywh(0., 0., if i % 3 == 0 { 32. } else { 24. }, 18.),
                        0.,
                        UiColor::rgba(40 + i * 3, 100, 170, 128),
                    );
                    local.draw_text(
                        Point::new(1.25, 2.5),
                        "ab",
                        UiColor::rgba(230, 240, 250, 200),
                        10.,
                    );
                    local.stroke_rounded_rect(
                        Rect::from_xywh(0., 0., if i % 3 == 0 { 32. } else { 24. }, 18.),
                        0.,
                        1.,
                        UiColor::rgba(240, 70, 90, 180),
                    );
                    paint.draw_display_list(
                        Point::new((i % 8) as f32 * 28. + phase, (i / 8) as f32 * 20. + phase),
                        local.into_display_list(),
                    );
                    if i == 19 {
                        let mut filter = PaintContext::new();
                        filter.backdrop_filter_rounded_rect(
                            Rect::from_xywh(12., 14., 110., 42.),
                            3.,
                            3.,
                            UiColor::rgba(0, 0, 0, 0),
                        );
                        paint.draw_display_list(Point::ZERO, filter.into_display_list());
                    }
                }
                paint.pop_clip();
                let full = PixelBounds {
                    x: 0,
                    y: 0,
                    width,
                    height,
                };
                // Warm admission before comparing both retained replay paths.
                encoder.advance_frame_serial();
                encoder
                    .lower_retained(&mut queue.executor(&mut resources), &paint, scale, full)
                    .unwrap();
                let mut counts = [0; 2];
                for slot in 0..2 {
                    encoder.advance_frame_serial();
                    encoder.backdrop_pixels = 0;
                    encoder.backdrop_filters = 0;
                    encoder.backdrop_fallbacks = 0;
                    let mut frame = encoder
                        .lower_retained(&mut queue.executor(&mut resources), &paint, scale, full)
                        .unwrap();
                    if cached && phase != 0. {
                        assert!(encoder.has_cpu_meshes());
                    }
                    if slot == 1 {
                        batch_draws(&encoder, &mut frame).unwrap();
                    }
                    counts[slot] = frame
                        .draws
                        .iter()
                        .filter(|d| !matches!(d.source, DrawSource::ClearOnly))
                        .count();
                    encoder
                        .submit(
                            &mut queue.executor(&mut resources),
                            targets[slot],
                            UiColor::rgb(10, 20, 30),
                            &[full],
                            &frame,
                        )
                        .unwrap();
                }
                let before = resources.read_texture(targets[0]).unwrap();
                let after = resources.read_texture(targets[1]).unwrap();
                let max = before
                    .iter()
                    .zip(&after)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                assert!(
                    max <= 1,
                    "scale={scale}, phase={phase}, max pixel difference={max}"
                );
                assert!(
                    counts[1] < counts[0],
                    "expected draw coalescing: {counts:?}"
                );
            }
        }
    }
}
