//! One glyph layout for measurement, caret placement and rasterization.
//!
//! OpenType positioning is delegated to Rustybuzz (the Rust HarfBuzz port).
//! Coordinates stay fractional; only coverage masks quantize to raster phases.
use super::*;
use rustybuzz::{BufferFlags, Direction, ShapePlan, UnicodeBuffer};
use unicode_segmentation::UnicodeSegmentation;

pub(super) struct PositionedGlyph {
    pub id: u16,
    pub font_slot: u8,
    pub x: f32,
    pub y: f32,
}

pub(super) struct ShapedText {
    pub glyphs: Vec<PositionedGlyph>,
    pub prefix_widths: Vec<f32>,
    pub width: f32,
    pub height: f32,
}

struct LayoutEntry {
    text: alloc::string::String,
    size: u32,
    stack: usize,
    layout: Arc<ShapedText>,
    bytes: usize,
}
static LAYOUTS: Mutex<Vec<LayoutEntry>> = Mutex::new(Vec::new());
struct PlanEntry {
    face: usize,
    script: rustybuzz::Script,
    plan: Arc<ShapePlan>,
}
static PLANS: Mutex<Vec<PlanEntry>> = Mutex::new(Vec::new());

pub(super) fn shape_text(text: &str, size: f32, stack: &FontStack) -> Arc<ShapedText> {
    let key = size.to_bits();
    if let Some(entry) = LAYOUTS
        .lock()
        .iter()
        .find(|e| e.size == key && e.stack == stack.cache_id() && e.text == text)
    {
        return Arc::clone(&entry.layout);
    }
    let layout = Arc::new(shape_uncached(text, size, stack));
    let bytes = text.len()
        + layout.prefix_widths.len() * core::mem::size_of::<f32>()
        + layout.glyphs.len() * core::mem::size_of::<PositionedGlyph>();
    // Large document rows keep their own layout. Bound both entry count and
    // retained bytes; warm labels don't rebuild font plans or allocate glyphs.
    if bytes <= 64 * 1024 {
        let mut cache = LAYOUTS.lock();
        while !cache.is_empty()
            && (cache.len() >= 512
                || cache.iter().map(|e| e.bytes).sum::<usize>() + bytes > 4 * 1024 * 1024)
        {
            cache.remove(0);
        }
        cache.push(LayoutEntry {
            text: text.into(),
            size: key,
            stack: stack.cache_id(),
            layout: Arc::clone(&layout),
            bytes,
        });
    }
    layout
}

fn shape_uncached(text: &str, size: f32, stack: &FontStack) -> ShapedText {
    let metrics = font_metrics_sized_with_font_stack(size, stack);
    let line_height = metrics.height() + metrics.leading;
    let mut result = ShapedText {
        glyphs: Vec::new(),
        prefix_widths: alloc::vec![0.0; text.len() + 1],
        width: 0.0,
        height: metrics.height(),
    };
    let mut line_start = 0;
    for (line_index, line) in text.split('\n').enumerate() {
        let mut run = UnicodeBuffer::new();
        let mut run_start = 0;
        let mut run_slot = 0;
        let mut run_font = stack.primary.clone();
        let mut x = 0.0;
        for (byte, ch) in line.char_indices() {
            let (font, slot, render_ch) = if is_zero_width_format(ch) {
                (run_font.clone(), run_slot, ch)
            } else {
                select_font_for_char(ch, stack)
            };
            if slot != run_slot && !run.is_empty() {
                shape_run(
                    &line[run_start..byte],
                    run_start + line_start,
                    run,
                    &run_font,
                    run_slot,
                    size,
                    &mut x,
                    line_index as f32 * line_height,
                    &mut result,
                );
                run = UnicodeBuffer::new();
                run_start = byte;
            }
            run_slot = slot;
            run_font = font;
            run.add(render_ch, (byte - run_start) as u32);
        }
        if !run.is_empty() {
            shape_run(
                &line[run_start..],
                run_start + line_start,
                run,
                &run_font,
                run_slot,
                size,
                &mut x,
                line_index as f32 * line_height,
                &mut result,
            );
        }
        result.prefix_widths[line_start + line.len()] = x;
        result.width = result.width.max(x);
        result.height = metrics.height() + line_index as f32 * line_height;
        line_start += line.len() + 1;
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn shape_run(
    text: &str,
    start: usize,
    mut input: UnicodeBuffer,
    font: &FontFace,
    slot: u8,
    size: f32,
    x: &mut f32,
    y: f32,
    result: &mut ShapedText,
) {
    let face = rustybuzz::Face::from_slice(font.font_data(), font.index)
        .expect("outline and shaping faces use the same validated font data");
    input.guess_segment_properties();
    // The current controls have an LTR layout/caret model. Paragraph bidi is a
    // separate responsibility; don't silently reverse only their painted text.
    input.set_direction(Direction::LeftToRight);
    input.set_flags(BufferFlags::REMOVE_DEFAULT_IGNORABLES);
    // Every face follows its own OpenType features. No font-name, family or
    // monospace special cases, and no artificial tracking/width correction.
    let script = input.script();
    let plan = {
        let mut plans = PLANS.lock();
        if let Some(entry) = plans
            .iter()
            .find(|p| p.face == font.id && p.script == script)
        {
            Arc::clone(&entry.plan)
        } else {
            let plan = Arc::new(ShapePlan::new(
                &face,
                Direction::LeftToRight,
                Some(script),
                None,
                &[],
            ));
            if plans.len() >= 64 {
                plans.remove(0);
            }
            plans.push(PlanEntry {
                face: font.id,
                script,
                plan: Arc::clone(&plan),
            });
            plan
        }
    };
    let shaped = rustybuzz::shape_with_plan(&face, &plan, input);
    let factor = size / face.units_per_em() as f32;
    let infos = shaped.glyph_infos();
    let positions = shaped.glyph_positions();
    let mut i = 0;
    result.prefix_widths[start..=start + text.len()].fill(*x);
    while i < infos.len() {
        let cluster = infos[i].cluster as usize;
        let left = *x;
        let mut end = i + 1;
        while end < infos.len() && infos[end].cluster == infos[i].cluster {
            end += 1;
        }
        let cluster_end = infos
            .get(end)
            .map_or(text.len(), |info| info.cluster as usize);
        for (info, position) in infos[i..end].iter().zip(&positions[i..end]) {
            result.glyphs.push(PositionedGlyph {
                id: info.glyph_id as u16,
                font_slot: slot,
                x: *x + position.x_offset as f32 * factor,
                y: y - position.y_offset as f32 * factor,
            });
            *x += position.x_advance as f32 * factor;
        }
        // Carets stay at grapheme boundaries. For a Latin ligature distribute
        // its advance over the source graphemes, never over UTF-8 bytes/marks.
        let cluster_text = &text[cluster..cluster_end];
        let count = cluster_text.graphemes(true).count().max(1);
        for (n, (byte, grapheme)) in cluster_text.grapheme_indices(true).enumerate() {
            let a = left + (*x - left) * n as f32 / count as f32;
            let b = left + (*x - left) * (n + 1) as f32 / count as f32;
            result.prefix_widths[start + cluster + byte..start + cluster + byte + grapheme.len()]
                .fill(a);
            result.prefix_widths[start + cluster + byte + grapheme.len()] = b;
        }
        i = end;
    }
    result.prefix_widths[start + text.len()] = *x;
}
