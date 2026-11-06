//! Persistent portable rim geometry. Background sampling remains live; only
//! the background-independent optical contour is reused across frames.
use super::*;
use scarlet_ui_core::geometry::Rect;

const MAX_ENTRIES: usize = 64;
// One frame-sized GPU arena, allocated once. Recycle ranges instead of
// defining a new immutable SGFX buffer for every hover, resize or opacity.
const CAPACITY: usize = MAX_FRAME_VERTICES;

struct Entry {
    key: [u32; 6],
    first: usize,
    count: usize,
    bytes: Vec<u8>,
    used: u64,
    uploaded: bool,
}

#[derive(Default)]
pub(super) struct OpticalCache {
    buffer: Option<BufferId>,
    entries: Vec<Entry>,
}

impl OpticalCache {
    fn free_range(&mut self, count: usize) -> Option<usize> {
        self.entries.sort_by_key(|entry| entry.first);
        let mut first = 0;
        for entry in &self.entries {
            if entry.first - first >= count {
                return Some(first);
            }
            first = entry.first + entry.count;
        }
        (CAPACITY - first >= count).then_some(first)
    }

    fn reserve(&mut self, count: usize, frame: u64) -> Option<usize> {
        if count > CAPACITY {
            return None;
        }
        loop {
            if self.entries.len() < MAX_ENTRIES {
                if let Some(first) = self.free_range(count) {
                    return Some(first);
                }
            }
            // Draws already lowered this frame may still refer to these ranges.
            // Never overwrite them, including during nested tile submissions.
            let stale = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.used != frame)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(index, _)| index)?;
            self.entries.remove(stale);
        }
    }
}

impl SgfxPaintEncoder {
    pub(super) fn cached_glass_rim(
        &mut self,
        tessellator: &mut Tessellator,
        rect: Rect,
        radius: f32,
        opacity: f32,
        scale: f32,
    ) -> Result<Option<Draw>> {
        // Preserve precise CPU clipping at rounded ancestor edges. Their
        // straight interior, like rectangular clips, can use a GPU scissor.
        let Some(clip_material) = tessellator.rim_clip_mode(rect, radius) else {
            return Ok(None);
        };
        let Some(scissor) = tessellator.effective_scissor() else {
            return Ok(None);
        };
        let key = [
            rect.size.width.to_bits(),
            rect.size.height.to_bits(),
            radius.to_bits(),
            opacity.to_bits(),
            scale.to_bits(),
            u32::from(clip_material),
        ];
        let index = if let Some(index) = self
            .optical_cache
            .entries
            .iter()
            .position(|entry| entry.key == key)
        {
            index
        } else {
            let template = tessellator.rim_template(rect, radius, clip_material)?;
            if template.is_empty() {
                return Ok(None);
            }
            let Some(first) = self
                .optical_cache
                .reserve(template.len(), self.frame_serial)
            else {
                return Ok(None);
            };
            if self.optical_cache.buffer.is_none() {
                self.optical_cache.buffer = Some(
                    self.table
                        .define_buffer(
                            BufferDesc::new(
                                (CAPACITY * PAINT_VERTEX_STRIDE as usize) as u64,
                                BufferUsage::VERTEX | BufferUsage::COPY_DST,
                            )
                            .map_err(|_| Error::sgfx(Stage::DefineResources))?,
                        )
                        .map_err(|_| Error::sgfx(Stage::DefineResources))?
                        .id(),
                );
            }
            let mut vertices = template.as_ref().clone();
            for vertex in &mut vertices {
                vertex.color[3] *= opacity;
            }
            let bytes = encode_paint_vertices(&vertices)?;
            self.optical_cache.entries.push(Entry {
                key,
                first,
                count: vertices.len(),
                bytes,
                used: self.frame_serial,
                uploaded: false,
            });
            self.optical_cache.entries.len() - 1
        };
        let entry = &mut self.optical_cache.entries[index];
        entry.used = self.frame_serial;
        Ok(Some(Draw {
            geometry: GeometryRange {
                first_vertex: entry.first as u32,
                vertex_count: entry.count as u32,
                scissor,
            },
            source: DrawSource::Solid,
            vertex_buffer: self.optical_cache.buffer,
            offset: [rect.left() * scale, rect.top() * scale],
            snap_phase: [0.; 2],
        }))
    }

    pub(super) fn submit_optical_uploads<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
    ) -> core::result::Result<(), FrameError<E::Error>> {
        let pending = || {
            self.optical_cache
                .entries
                .iter()
                .filter(|entry| entry.used == self.frame_serial && !entry.uploaded)
        };
        if pending().next().is_none() {
            return Ok(());
        }
        let table = Rc::clone(&self.table);
        let buffer = table
            .buffer_ref(self.optical_cache.buffer.ok_or(Error::InvalidFrame)?)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let mut encoder = CommandEncoder::new(&table);
        for entry in pending() {
            encoder
                .write_buffer(
                    buffer,
                    (entry.first * PAINT_VERTEX_STRIDE as usize) as u64,
                    &entry.bytes,
                )
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        }
        let commands = encoder
            .finish()
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        executor.execute(&commands).map_err(FrameError::Execution)?;
        for entry in &mut self.optical_cache.entries {
            if entry.used == self.frame_serial && !entry.uploaded {
                entry.uploaded = true;
                // The GPU arena now owns the reusable geometry. Keep staging
                // bytes only for uploads that still need acceptance/retry.
                entry.bytes = Vec::new();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sgfx::ir::{Command, CommandBuffer};

    #[derive(Default)]
    struct Trace {
        writes: usize,
        fail: bool,
    }
    impl CommandExecutor for Trace {
        type Error = ();
        fn execute<'r, 'data>(
            &mut self,
            commands: &CommandBuffer<'r, 'data>,
        ) -> core::result::Result<(), ()> {
            if self.fail {
                return Err(());
            }
            self.writes += commands
                .commands()
                .iter()
                .filter(|command| matches!(command, Command::WriteBuffer { .. }))
                .count();
            Ok(())
        }
    }

    fn tess(scale: u32) -> Tessellator {
        Tessellator::new(scale, 1200, 2400, FloatRect::new(0., 0., 1200., 2400.)).unwrap()
    }

    #[test]
    fn translated_warm_rim_reuses_gpu_bytes_and_preserves_live_geometry() {
        for scale in [1000, 1250, 2000, 3000] {
            let mut encoder = SgfxPaintEncoder::new(1200, 2400, false).unwrap();
            let mut tess = tess(scale);
            let factor = scale as f32 / 1000.;
            let rect = Rect::from_xywh(11.25, 75.5, 360., 56.);
            encoder.advance_frame_serial();
            let cold = encoder
                .cached_glass_rim(&mut tess, rect, 28., 0.8, factor)
                .unwrap()
                .unwrap();
            let bytes = encoder.optical_cache.entries[0].bytes.clone();
            let mut trace = Trace::default();
            encoder.submit_optical_uploads(&mut trace).unwrap();
            assert_eq!(trace.writes, 1);
            assert!(
                encoder.optical_cache.entries[0].bytes.is_empty(),
                "accepted GPU uploads must release their CPU staging copy"
            );
            encoder.advance_frame_serial();
            let rect = Rect::from_xywh(13.75, 23.25, 360., 56.);
            let warm = encoder
                .cached_glass_rim(&mut tess, rect, 28., 0.8, factor)
                .unwrap()
                .unwrap();
            encoder.submit_optical_uploads(&mut trace).unwrap();
            assert_eq!(
                trace.writes, 1,
                "scrolling must not upload the same contour again"
            );
            assert_eq!(cold.vertex_buffer, warm.vertex_buffer);
            assert_eq!(cold.geometry.first_vertex, warm.geometry.first_vertex);
            assert!(
                tess.vertices().is_empty(),
                "cached rims must not spend the live frame vertex budget"
            );
            let mut live = self::tess(scale);
            live.glass_rim(rect, 28., 0.8).unwrap();
            let translated = bytes
                .chunks_exact(PAINT_VERTEX_STRIDE as usize)
                .map(|bytes| {
                    let f = |start| f32::from_le_bytes(bytes[start..start + 4].try_into().unwrap());
                    Vertex {
                        position: [f(0) + warm.offset[0], f(4) + warm.offset[1]],
                        color: [f(16), f(20), f(24), f(28)],
                        tex_coord: [0.; 2],
                    }
                })
                .collect::<Vec<_>>();
            // Translation can collapse tiny local triangles at f32 precision.
            // Those remain harmless GPU degenerates. The live CPU path drops
            // them; every nondegenerate triangle must still be identical.
            let visible = translated
                .chunks_exact(3)
                .filter(|triangle| {
                    let p = |i: usize, axis: usize| f64::from(triangle[i].position[axis]);
                    (p(1, 0) - p(0, 0)) * (p(2, 1) - p(0, 1))
                        != (p(1, 1) - p(0, 1)) * (p(2, 0) - p(0, 0))
                })
                .flatten()
                .collect::<Vec<_>>();
            assert_eq!(visible.len(), live.vertices().len());
            for (cached, vertex) in visible.iter().zip(live.vertices()) {
                assert_eq!(cached.position, vertex.position);
                assert_eq!(cached.color, vertex.color);
            }
        }
    }

    #[test]
    fn rounded_clips_keep_precise_cpu_geometry_and_opacity_does_not_alias() {
        let mut encoder = SgfxPaintEncoder::new(1200, 2400, false).unwrap();
        let rect = Rect::from_xywh(10., 10., 160., 36.);
        let mut clipped = tess(2000);
        clipped
            .push_clip(Rect::from_xywh(0., 0., 180., 36.), 12.)
            .unwrap();
        assert!(
            encoder
                .cached_glass_rim(&mut clipped, rect, 12., 1., 2.)
                .unwrap()
                .is_none()
        );
        assert!(encoder.optical_cache.entries.is_empty());
        let mut plain = tess(2000);
        for opacity in [0.5, 1.] {
            encoder
                .cached_glass_rim(&mut plain, rect, 12., opacity, 2.)
                .unwrap()
                .unwrap();
        }
        assert_eq!(encoder.optical_cache.entries.len(), 2);
        assert_ne!(
            encoder.optical_cache.entries[0].bytes,
            encoder.optical_cache.entries[1].bytes
        );
    }

    #[test]
    fn material_sized_clip_is_cached_without_dropping_its_precise_boundary() {
        for scale in [1000, 1250, 1333, 2000, 3000] {
            let mut encoder = SgfxPaintEncoder::new(1200, 2400, false).unwrap();
            let mut clipped = tess(scale);
            let rect = Rect::from_xywh(0., 0., 360., 56.);
            clipped.push_clip(rect, 28.).unwrap();
            let draw = encoder
                .cached_glass_rim(&mut clipped, rect, 28., 1., scale as f32 / 1000.)
                .unwrap()
                .unwrap();
            assert!(clipped.vertices().is_empty());
            let mut live = tess(scale);
            live.push_clip(rect, 28.).unwrap();
            live.glass_rim(rect, 28., 1.).unwrap();
            assert_eq!(draw.geometry.vertex_count as usize, live.vertices().len());
            assert_eq!(
                encoder.optical_cache.entries[0].bytes,
                encode_paint_vertices(live.vertices()).unwrap()
            );
        }
    }

    #[test]
    fn rounded_container_interior_keeps_rims_on_the_gpu() {
        let mut encoder = SgfxPaintEncoder::new(1200, 2400, false).unwrap();
        let mut tess = tess(2000);
        tess.push_clip(Rect::from_xywh(4., 190., 492., 300.), 20.)
            .unwrap();
        let rect = Rect::from_xywh(16., 256., 130., 36.);
        assert!(
            encoder
                .cached_glass_rim(&mut tess, rect, 12., 1., 2.)
                .unwrap()
                .is_some()
        );
        assert!(tess.vertices().is_empty());
        let corner = Rect::from_xywh(6., 192., 130., 36.);
        assert!(
            encoder
                .cached_glass_rim(&mut tess, corner, 12., 1., 2.)
                .unwrap()
                .is_none(),
            "a rim that crosses a rounded corner must retain precise polygon clipping"
        );
    }

    #[test]
    fn rejected_upload_is_retried_and_resize_recycles_one_bounded_arena() {
        let mut encoder = SgfxPaintEncoder::new(1200, 2400, false).unwrap();
        let mut tess = tess(3000);
        let mut buffer = None;
        for frame in 0..80 {
            encoder.advance_frame_serial();
            let draw = encoder
                .cached_glass_rim(
                    &mut tess,
                    Rect::from_xywh(0., 0., 180. + frame as f32, 56.),
                    28.,
                    1.,
                    3.,
                )
                .unwrap()
                .unwrap();
            if frame == 0 {
                buffer = draw.vertex_buffer;
                assert!(
                    encoder
                        .submit_optical_uploads(&mut Trace {
                            fail: true,
                            ..Trace::default()
                        })
                        .is_err()
                );
                assert!(!encoder.optical_cache.entries[0].uploaded);
            }
            assert_eq!(buffer, draw.vertex_buffer);
            let mut trace = Trace::default();
            encoder.submit_optical_uploads(&mut trace).unwrap();
            assert_eq!(trace.writes, 1);
            assert!(encoder.optical_cache.entries.len() <= MAX_ENTRIES);
            assert!(
                encoder
                    .optical_cache
                    .entries
                    .iter()
                    .map(|e| e.count)
                    .sum::<usize>()
                    <= CAPACITY
            );
            let mut ranges = encoder
                .optical_cache
                .entries
                .iter()
                .map(|e| (e.first, e.first + e.count))
                .collect::<Vec<_>>();
            ranges.sort_unstable();
            assert!(ranges.windows(2).all(|r| r[0].1 <= r[1].0));
            assert!(ranges.last().unwrap().1 <= CAPACITY);
        }
    }

    #[test]
    fn current_frame_ranges_are_never_recycled_under_pressure() {
        let mut cache = OpticalCache::default();
        cache.entries.push(Entry {
            key: [0; 6],
            first: 0,
            count: CAPACITY,
            bytes: Vec::new(),
            used: 9,
            uploaded: true,
        });
        assert_eq!(cache.reserve(3, 9), None);
        assert_eq!(cache.reserve(3, 10), Some(0));
    }
}
