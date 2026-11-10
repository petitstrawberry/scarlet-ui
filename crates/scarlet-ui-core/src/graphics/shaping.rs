//! One glyph layout for measurement, caret placement and rasterization.
//!
//! OpenType positioning is delegated to Rustybuzz (the Rust HarfBuzz port).
//! Coordinates stay fractional; only coverage masks quantize to raster phases.
use super::*;
use rustybuzz::{BufferFlags, Direction, ShapePlan, UnicodeBuffer};
use unicode_segmentation::UnicodeSegmentation;

// A parsed shaping face borrows the immutable outline font's shared bytes.
// Keep that owner and borrow together: no leaked font data or extended lifetime.
// Cloned font stacks share this lazy face; dropping the last clone releases it.
type ParsedFace<'a> = rustybuzz::Face<'a>;
self_cell::self_cell! {
    pub(super) struct OwnedFace {
        owner: FontArc,
        #[covariant]
        dependent: ParsedFace,
    }
}
impl core::fmt::Debug for OwnedFace {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OwnedFace").finish_non_exhaustive()
    }
}
impl FontFace {
    fn shaping_face(&self) -> &rustybuzz::Face<'_> {
        self.shaping
            .get_or_init(|| {
                Box::new(OwnedFace::new(self.font.clone(), |font| {
                    rustybuzz::Face::from_slice(font.font_data(), self.index)
                        .expect("outline and shaping faces use the same validated font data")
                }))
            })
            .borrow_dependent()
    }
}

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

#[derive(Clone, Hash, PartialEq, Eq)]
struct LayoutKey {
    text: Arc<str>,
    size: u32,
    stack: usize,
}
#[derive(Hash)]
struct LayoutQuery<'a> {
    text: &'a str,
    size: u32,
    stack: usize,
}
impl hashbrown::Equivalent<LayoutKey> for LayoutQuery<'_> {
    fn equivalent(&self, key: &LayoutKey) -> bool {
        self.size == key.size && self.stack == key.stack && self.text == key.text.as_ref()
    }
}
struct LayoutEntry {
    layout: Arc<ShapedText>,
    bytes: usize,
}
struct LayoutCache {
    entries: hashbrown::HashMap<LayoutKey, LayoutEntry>,
    order: alloc::collections::VecDeque<LayoutKey>,
    bytes: usize,
}
impl LayoutCache {
    fn new() -> Self {
        Self {
            entries: hashbrown::HashMap::new(),
            order: alloc::collections::VecDeque::new(),
            bytes: 0,
        }
    }
    fn get(&self, query: &LayoutQuery<'_>) -> Option<Arc<ShapedText>> {
        self.entries.get(query).map(|e| Arc::clone(&e.layout))
    }
    fn insert(&mut self, query: LayoutQuery<'_>, layout: Arc<ShapedText>) {
        // Count the shared text once, both key records, and owned layout storage.
        let bytes = query.text.len()
            + 2 * core::mem::size_of::<LayoutKey>()
            + core::mem::size_of::<LayoutEntry>()
            + layout.prefix_widths.len() * core::mem::size_of::<f32>()
            + layout.glyphs.len() * core::mem::size_of::<PositionedGlyph>();
        if bytes > 64 * 1024 || self.entries.contains_key(&query) {
            return;
        }
        while self.entries.len() >= 512 || self.bytes + bytes > 4 * 1024 * 1024 {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&old) {
                self.bytes -= entry.bytes;
            }
        }
        let key = LayoutKey {
            text: query.text.into(),
            size: query.size,
            stack: query.stack,
        };
        self.order.push_back(key.clone());
        self.entries.insert(key, LayoutEntry { layout, bytes });
        self.bytes += bytes;
    }
}
// Lazy allocation keeps the no_std path and avoids a process-global font owner.
static LAYOUTS: Mutex<Option<LayoutCache>> = Mutex::new(None);
struct PlanEntry {
    face: usize,
    script: rustybuzz::Script,
    plan: Arc<ShapePlan>,
}
static PLANS: Mutex<Vec<PlanEntry>> = Mutex::new(Vec::new());

pub(super) fn shape_text(text: &str, size: f32, stack: &FontStack) -> Arc<ShapedText> {
    let query = LayoutQuery {
        text,
        size: size.to_bits(),
        stack: stack.cache_id(),
    };
    if let Some(layout) = LAYOUTS.lock().as_ref().and_then(|cache| cache.get(&query)) {
        return layout;
    }
    let layout = Arc::new(shape_uncached(text, size, stack));
    LAYOUTS
        .lock()
        .get_or_insert_with(LayoutCache::new)
        .insert(query, Arc::clone(&layout));
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
        let mut run_font = &stack.primary;
        let mut x = 0.0;
        for (byte, ch) in line.char_indices() {
            let (font, slot, render_ch) = if is_zero_width_format(ch) {
                (run_font, run_slot, ch)
            } else {
                select_font_for_char(ch, stack)
            };
            if slot != run_slot && !run.is_empty() {
                shape_run(
                    &line[run_start..byte],
                    run_start + line_start,
                    run,
                    run_font,
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
                run_font,
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
    let face = font.shaping_face();
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
                face,
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
    let shaped = rustybuzz::shape_with_plan(face, &plan, input);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(face: &rustybuzz::Face<'_>, text: &str) -> Vec<(u32, u32, i32, i32, i32, i32)> {
        let mut input = UnicodeBuffer::new();
        input.push_str(text);
        input.guess_segment_properties();
        input.set_direction(Direction::LeftToRight);
        input.set_flags(BufferFlags::REMOVE_DEFAULT_IGNORABLES);
        let result = rustybuzz::shape(face, &[], input);
        result
            .glyph_infos()
            .iter()
            .zip(result.glyph_positions())
            .map(|(i, p)| {
                (
                    i.glyph_id,
                    i.cluster,
                    p.x_advance,
                    p.y_advance,
                    p.x_offset,
                    p.y_offset,
                )
            })
            .collect()
    }

    #[test]
    fn shared_face_matches_fresh_parser_for_static_owned_and_collection_fonts() {
        let bytes = include_bytes!("../testing/fonts/shaping.ttf");
        let mut fonts = alloc::vec![
            FontFace::from_slice(bytes).unwrap(),
            FontFace::from_vec(bytes.to_vec()).unwrap()
        ];
        #[cfg(all(feature = "std", not(target_arch = "wasm32")))]
        fonts.push(
            discovered_font_ref(DiscoveredFont {
                bytes: include_bytes!("../testing/fonts/shaping-collection.ttc").to_vec(),
                index: 1,
            })
            .unwrap(),
        );
        for font in fonts {
            let cached = font.shaping_face();
            let fresh = rustybuzz::Face::from_slice(font.font_data(), font.index).unwrap();
            for text in [
                "AVfi",
                "A\u{301}V fi",
                "AV\u{200d}fi",
                "item 00124 · 0042",
                "日本語",
                "مرحبا",
            ] {
                assert_eq!(signature(cached, text), signature(&fresh, text));
            }
            assert_eq!(cached.units_per_em(), fresh.units_per_em());
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn concurrent_clones_share_the_face_and_release_it_with_the_font() {
        let font =
            FontFace::from_vec(include_bytes!("../testing/fonts/shaping.ttf").to_vec()).unwrap();
        let weak = Arc::downgrade(&font.shaping);
        assert!(font.shaping.get().is_none());
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let clone = font.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let face = clone.shaping_face();
                    let glyphs = signature(face, "AVfi");
                    (face as *const _ as usize, glyphs)
                })
            })
            .collect();
        let expected = (
            font.shaping_face() as *const _ as usize,
            signature(font.shaping_face(), "AVfi"),
        );
        for worker in workers {
            assert_eq!(worker.join().unwrap(), expected);
        }
        drop(font);
        assert!(
            weak.upgrade().is_none(),
            "parsed faces must not leak or live in a global owner cache"
        );
    }

    #[test]
    fn indexed_layout_cache_preserves_keys_fifo_and_memory_bounds() {
        let mut cache = LayoutCache::new();
        let layout = Arc::new(ShapedText {
            glyphs: Vec::new(),
            prefix_widths: alloc::vec![0.; 8],
            width: 12.,
            height: 13.,
        });
        fn query(text: &str) -> LayoutQuery<'_> {
            LayoutQuery {
                text,
                size: 13f32.to_bits(),
                stack: 1,
            }
        }
        for i in 0..600 {
            cache.insert(query(&alloc::format!("label {i}")), layout.clone());
        }
        assert_eq!(cache.entries.len(), 512);
        assert_eq!(cache.order.len(), 512);
        assert!(cache.get(&query("label 87")).is_none());
        assert!(Arc::ptr_eq(
            &cache.get(&query("label 88")).unwrap(),
            &layout
        ));
        assert!(
            cache
                .get(&LayoutQuery {
                    text: "label 88",
                    size: 14f32.to_bits(),
                    stack: 1
                })
                .is_none()
        );
        assert!(
            cache
                .get(&LayoutQuery {
                    text: "label 88",
                    size: 13f32.to_bits(),
                    stack: 2
                })
                .is_none()
        );
        let bytes = cache.bytes;
        cache.insert(query("label 88"), layout.clone());
        assert_eq!(cache.bytes, bytes);
        assert_eq!(cache.order.len(), 512);
        let large = Arc::new(ShapedText {
            glyphs: Vec::new(),
            prefix_widths: alloc::vec![0.; 8192],
            width: 10.,
            height: 10.,
        });
        for i in 0..600 {
            cache.insert(query(&alloc::format!("large {i}")), large.clone());
        }
        assert!(cache.entries.len() < 512 && cache.bytes <= 4 * 1024 * 1024);
        assert_eq!(
            cache.bytes,
            cache.entries.values().map(|e| e.bytes).sum::<usize>()
        );
        let oversized = Arc::new(ShapedText {
            glyphs: Vec::new(),
            prefix_widths: alloc::vec![0.; 20000],
            width: 10.,
            height: 10.,
        });
        cache.insert(query("oversized"), oversized);
        assert!(cache.get(&query("oversized")).is_none());
        assert_eq!(
            (layout.width, layout.height),
            (12., 13.),
            "eviction leaves outstanding layouts valid"
        );
    }
}
