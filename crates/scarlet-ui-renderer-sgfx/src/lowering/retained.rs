//! Bounded GPU mesh reuse for immutable local-coordinate recordings.
//! Rounded outer clips and commands with position-dependent rasterization use
//! the normal lowering path whenever replay cannot preserve their semantics.
use super::*;
use scarlet_ui_core::{
    geometry::{Point, Rect},
    renderer::DisplayList,
};
pub(super) const MAX_RETAINED_MESHES: usize = 256;
// Additional small recordings share the frame buffer instead of allocating GPU buffers.
const MAX_CPU_MESHES: usize = 1024;
const RETAINED_BUFFER_BYTES: u64 = 65_536; // At most 16 MiB; slots are recycled.
// Small cached pictures are cheaper to coalesce than to submit individually.
// CPU geometry: at most 1.25 MiB for GPU slots plus 5 MiB for shared-buffer slots.
const MAX_BATCH_GEOMETRY_BYTES: usize = 128 * PAINT_VERTEX_STRIDE as usize;

pub(super) struct RetainedMesh {
    list_id: u64,
    source: Weak<DisplayList>,
    scale: u32,
    buffer: Option<BufferId>,
    draws: Vec<Draw>,
    bounds: FloatRect,
    textures: Vec<(TextureId, u64, u64)>,
    atlases: Vec<(TextureId, u64)>,
    used_frame: u64,
    // Keep bytes until the batched upload succeeds, including across a failed
    // frame. A cache entry is never replayed before pending writes are submitted.
    pending_vertices: Option<Vec<u8>>,
    batch_vertices: Option<Vec<u8>>,
}
impl SgfxPaintEncoder {
    #[cfg(test)]
    pub(super) fn has_cpu_meshes(&self) -> bool {
        self.retained_meshes
            .iter()
            .any(|mesh| mesh.buffer.is_none())
    }

    pub(super) fn batch_vertices(&self, draw: &Draw) -> Option<&[u8]> {
        let buffer = draw.vertex_buffer?;
        let mesh = self
            .retained_meshes
            .iter()
            .find(|mesh| mesh.buffer == Some(buffer))?;
        let bytes = mesh.batch_vertices.as_ref()?;
        let start =
            (draw.geometry.first_vertex as usize).checked_mul(PAINT_VERTEX_STRIDE as usize)?;
        let end = (draw.geometry.vertex_count as usize)
            .checked_mul(PAINT_VERTEX_STRIDE as usize)?
            .checked_add(start)?;
        bytes.get(start..end)
    }
    fn cached_mesh(&self, list: &DisplayList, scale: u32) -> Option<usize> {
        self.retained_mesh_index
            .get(&(list.identity(), scale))
            .copied()
            .filter(|&slot| self.mesh_valid(&self.retained_meshes[slot]))
    }
    fn mesh_valid(&self, mesh: &RetainedMesh) -> bool {
        mesh.textures.iter().all(|(id, identity, revision)| {
            self.buffer_textures.iter().any(|texture| {
                texture.texture == *id
                    && texture.buffer_identity == *identity
                    && texture.revision == *revision
                    && texture.upload_state == TextureUploadState::Uploaded
            })
        }) && mesh.atlases.iter().all(|(id, generation)| {
            self.glyph_atlases
                .iter()
                .any(|atlas| atlas.texture == *id && atlas.generation == *generation)
        })
    }
    fn protect_mesh(&mut self, index: usize) {
        let mesh = &mut self.retained_meshes[index];
        mesh.used_frame = self.frame_serial;
        for (id, _, _) in &mesh.textures {
            for texture in &mut self.buffer_textures {
                if texture.texture == *id {
                    texture.used_frame = self.frame_serial;
                }
            }
        }
        for (id, _) in &mesh.atlases {
            for atlas in &mut self.glyph_atlases {
                if atlas.texture == *id {
                    atlas.used_frame = self.frame_serial;
                }
            }
        }
    }
    pub(super) fn lower_retained<'a, E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        paint: &'a PaintContext<'_>,
        scale: u32,
        area: PixelBounds,
    ) -> core::result::Result<LoweredFrame<'a>, FrameError<E::Error>> {
        // Global state changes may intentionally span recording boundaries.
        // Preserve their original sequence with one flat lowering pass.
        if paint.commands().iter().any(|command| {
            matches!(command,
            PaintCommand::DrawDisplayList { list, .. } if !list.is_self_contained())
        }) {
            let flat = paint.flattened();
            return self
                .lower(&flat, scale, area)
                .map(owned_frame)
                .map_err(FrameError::Lowering);
        }
        self.glyph_atlas_rebuild_required = false;
        let backdrop_budget = (
            self.backdrop_pixels,
            self.backdrop_filters,
            self.backdrop_fallbacks,
        );
        match self.try_lower_retained(executor, paint, scale, area) {
            Err(FrameError::Lowering(Error::FrameTooComplex))
                if self.tile_depth == 0 && self.glyph_atlas_rebuild_required =>
            {
                // A reset would invalidate draws already prepared this frame.
                // Propagate nested failures to the outermost frame before
                // rebuilding atomically; enclosing draws can reference atlases too.
                for atlas in &mut self.glyph_atlases {
                    atlas.reset(self.frame_serial);
                }
                (
                    self.backdrop_pixels,
                    self.backdrop_filters,
                    self.backdrop_fallbacks,
                ) = backdrop_budget;
                let flat = paint.flattened();
                let frame = self.lower(&flat, scale, area)?;
                Ok(owned_frame(frame))
            }
            Ok(frame) => {
                self.upload_pending_meshes(executor)?;
                Ok(frame)
            }
            Err(error) => Err(error),
        }
    }

    fn upload_pending_meshes<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
    ) -> core::result::Result<(), FrameError<E::Error>> {
        if !self
            .retained_meshes
            .iter()
            .any(|mesh| mesh.pending_vertices.is_some())
        {
            return Ok(());
        }
        let mut encoder = CommandEncoder::new(&self.table);
        for mesh in &self.retained_meshes {
            if let Some(bytes) = &mesh.pending_vertices {
                let buffer = self
                    .table
                    .buffer_ref(mesh.buffer.ok_or(Error::InvalidFrame)?)
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                encoder
                    .write_buffer(buffer, 0, bytes)
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            }
        }
        let commands = encoder
            .finish()
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        executor.execute(&commands).map_err(FrameError::Execution)?;
        // A rejected or partially accepted batch retains every upload for retry.
        // The bounded mesh cache also bounds this command stream and its bytes.
        for mesh in &mut self.retained_meshes {
            mesh.pending_vertices = None;
        }
        Ok(())
    }
    fn try_lower_retained<'a, E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        paint: &'a PaintContext<'_>,
        scale: u32,
        area: PixelBounds,
    ) -> core::result::Result<LoweredFrame<'a>, FrameError<E::Error>> {
        // Protect every valid referenced resource before processing any new list.
        // Otherwise an earlier cold row could recycle a later warm row's atlas.
        for command in paint.commands() {
            if let PaintCommand::DrawDisplayList { list, .. } = command {
                if let Some(index) = self.cached_mesh(list, scale) {
                    self.protect_mesh(index);
                }
            }
        }
        let mut frame = LoweredFrame {
            vertex_bytes: Vec::new(),
            draws: Vec::new(),
            uploads: Vec::new(),
        };
        let mut clips: Vec<(Rect, f32)> = Vec::new();
        let mut opacity = 1.;
        let full = PixelBounds {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        };
        for (index, command) in paint.commands().iter().enumerate() {
            match command {
                PaintCommand::PushClip {
                    rect,
                    corner_radius,
                } => {
                    clips.push((*rect, *corner_radius));
                    continue;
                }
                PaintCommand::PopClip => {
                    clips.pop();
                    continue;
                }
                PaintCommand::SetOpacity { opacity: value } => {
                    opacity = *value;
                    continue;
                }
                _ => {}
            }
            if let PaintCommand::DrawDisplayList { origin, list } = command {
                // A retained mesh's local coordinates can be entirely valid
                // while its replay origin lies offscreen. Cull before creating
                // or uploading the mesh, not after submitting a clipped draw.
                if !list_has_visible_output(list, *origin, scale, &clips, full) {
                    continue;
                }
                if opacity == 1. {
                    if let Some(layer) =
                        self.lower_cached_layer(executor, list, *origin, scale, &clips, full)?
                    {
                        append_frame(&mut frame, layer)?;
                        continue;
                    }
                }
                if opacity == 1. {
                    // A static shadow stack is rasterized only on a cache miss.
                    // Cache all layers together, then sample its alpha coverage.
                    if clips.iter().all(|(_, radius)| *radius == 0.) {
                        if let Some(mut draw) =
                            self.cached_shadow(executor, list, *origin, scale, full)?
                        {
                            if let Some(scissor) = replay_scissor(
                                FloatRect::new(0., 0., full.width as f32, full.height as f32),
                                [0., 0.],
                                &clips,
                                full,
                                scale,
                            ) {
                                draw.geometry.scissor = scissor;
                                frame.draws.push(draw);
                                continue;
                            }
                        }
                    }
                    // Mesh replay snaps in window coordinates. Nested tile
                    // fallbacks must take the corrected raster path below.
                    let offsets = (self.tile_depth == 0)
                        .then(|| replay_offset(list, *origin, scale))
                        .flatten();
                    if let Some(offsets) = offsets {
                        let cached = self.cached_mesh(list, scale);
                        let slot = if let Some(index) = cached {
                            Some(index)
                        } else if self.admit_mesh(list.identity(), scale) {
                            self.record_mesh(executor, list, scale, full)?
                        } else {
                            None
                        };
                        if let Some(slot) = slot {
                            let mesh = &self.retained_meshes[slot];
                            if let Some(scissor) =
                                replay_scissor(mesh.bounds, offsets.shapes, &clips, full, scale)
                            {
                                self.protect_mesh(slot);
                                for draw in &self.retained_meshes[slot].draws {
                                    let mut draw = *draw;
                                    if draw.geometry.scissor != full {
                                        let local = draw.geometry.scissor;
                                        let rect = FloatRect::new(
                                            local.x as f32 + offsets.shapes[0],
                                            local.y as f32 + offsets.shapes[1],
                                            local.width as f32,
                                            local.height as f32,
                                        );
                                        let limit = FloatRect::new(
                                            scissor.x as f32,
                                            scissor.y as f32,
                                            scissor.width as f32,
                                            scissor.height as f32,
                                        );
                                        let Some(rect) = rect.intersect(limit) else {
                                            continue;
                                        };
                                        let left = libm::floorf(rect.x).max(0.) as u32;
                                        let top = libm::floorf(rect.y).max(0.) as u32;
                                        draw.geometry.scissor = PixelBounds {
                                            x: left,
                                            y: top,
                                            width: libm::ceilf(rect.right()) as u32 - left,
                                            height: libm::ceilf(rect.bottom()) as u32 - top,
                                        };
                                    } else {
                                        draw.geometry.scissor = scissor;
                                    }
                                    draw.offset = match draw.source {
                                        DrawSource::Glyph(_) => [
                                            text_replay_offset(origin.x, draw.snap_phase[0], scale),
                                            text_replay_offset(origin.y, draw.snap_phase[1], scale),
                                        ],
                                        DrawSource::IconGlyph(_)
                                        | DrawSource::Texture(_)
                                        | DrawSource::PixelTexture(_) => {
                                            let factor = scale.max(1) as f32 / 1000.;
                                            [
                                                truncated_scaled(
                                                    origin.x + draw.snap_phase[0],
                                                    factor,
                                                ) - truncated_scaled(draw.snap_phase[0], factor),
                                                truncated_scaled(
                                                    origin.y + draw.snap_phase[1],
                                                    factor,
                                                ) - truncated_scaled(draw.snap_phase[1], factor),
                                            ]
                                        }
                                        DrawSource::Solid | DrawSource::ClearOnly => offsets.shapes,
                                        DrawSource::PremultipliedTexture(_) => offsets.shapes,
                                        DrawSource::Backdrop(_) => {
                                            unreachable!("backdrops bypass mesh reuse")
                                        }
                                    };
                                    if self.retained_meshes[slot].buffer.is_none() {
                                        let bytes = self.retained_meshes[slot]
                                            .batch_vertices
                                            .as_ref()
                                            .ok_or(Error::InvalidFrame)?;
                                        let start = draw.geometry.first_vertex as usize
                                            * PAINT_VERTEX_STRIDE as usize;
                                        let end = start
                                            + draw.geometry.vertex_count as usize
                                                * PAINT_VERTEX_STRIDE as usize;
                                        super::batching::append_translated_vertices(
                                            &mut frame,
                                            &mut draw,
                                            &bytes[start..end],
                                        )?;
                                    }
                                    frame.draws.push(draw);
                                }
                                continue;
                            }
                        }
                    }
                }
            }
            // Keep precise rounded clipping, nested lists, extensions, opacity,
            // fractional-DPI rasterization and oversized recordings on the
            // existing path. Every command retains its original paint order.
            let one = if self.tile_depth > 0 {
                if let PaintCommand::DrawDisplayList { origin, list } = command {
                    // Flat fallback must use the same window-space snapping as
                    // a cached child, even at negative/fractional scroll offsets.
                    list.context().raster_tile_context(
                        Point::new(
                            origin.x - self.tile_translation.x,
                            origin.y - self.tile_translation.y,
                        ),
                        Point::new(libm::floorf(origin.x), libm::floorf(origin.y)),
                        scale,
                    )
                } else {
                    paint.command_context(index)
                }
            } else {
                paint.command_context(index)
            };
            let mut context = PaintContext::new();
            for (rect, radius) in &clips {
                context.push_rounded_clip(*rect, *radius);
            }
            context.set_opacity(opacity);
            context.append(&one);
            // The flat lowerer appends a frame-level clear-only dummy draw.
            // Close inherited clips after this command so an unrelated damage
            // region does not make that dummy's scissor empty/invalid.
            for _ in &clips {
                context.pop_clip();
            }
            let lowered = self.lower_once(&context, scale, area)?;
            append_frame(&mut frame, lowered)?;
        }
        // Ensure an empty scene still clears removed content through a valid pass.
        if frame.draws.is_empty() {
            append_frame(
                &mut frame,
                self.lower_once(&PaintContext::new(), scale, area)?,
            )?;
        }
        Ok(frame)
    }
    fn admit_mesh(&mut self, identity: u64, scale: u32) -> bool {
        let key = (identity, scale);
        // Keep the inexpensive eager path for small scenes and resource rebuilds.
        if self.retained_meshes.len() < MAX_RETAINED_MESHES
            || self.retained_mesh_index.contains_key(&key)
        {
            return true;
        }
        // A one-frame picture (e.g. a changing fill) should not displace stable
        // text/geometry. Admit only after reuse across frames. Capacity is a
        // hard bound; overflow still renders normally without retention.
        const MAX_CANDIDATES: usize = 4096;
        if self.mesh_candidates.len() < MAX_CANDIDATES {
            self.mesh_candidates.insert(key);
        }
        self.previous_mesh_candidates.contains(&key)
    }

    fn record_mesh<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        list: &Arc<DisplayList>,
        scale: u32,
        full: PixelBounds,
    ) -> core::result::Result<Option<usize>, FrameError<E::Error>> {
        // Lowering uses the frame-sized target at local origin. Reject clips
        // that would be truncated there: translating a truncated scissor could
        // reveal pixels that were never recorded.
        let factor = scale.max(1) as f32 / 1000.;
        if list.context().commands().iter().any(|c| matches!(c,
            PaintCommand::PushClip { rect, .. } if !rect.left().is_finite() || !rect.top().is_finite()
                || !rect.right().is_finite() || !rect.bottom().is_finite()
                || rect.left() < 0. || rect.top() < 0.
                || rect.right() * factor > full.width as f32 || rect.bottom() * factor > full.height as f32)) {
            return Ok(None);
        }
        // Scrolling can temporarily omit a still-live recording. Keep its
        // mesh until the cache is full, rather than evicting it on the next
        // frame and uploading the same vertices when it re-enters the viewport.
        let mut slot = self
            .retained_meshes
            .iter()
            .enumerate()
            .filter(|(_, mesh)| mesh.used_frame != self.frame_serial)
            .filter(|(_, mesh)| mesh.source.strong_count() == 0)
            .min_by_key(|(_, mesh)| mesh.used_frame)
            .map(|(index, _)| index)
            .or_else(|| {
                (self.retained_meshes.len() >= MAX_RETAINED_MESHES)
                    .then(|| {
                        self.retained_meshes
                            .iter()
                            .enumerate()
                            .filter(|(_, mesh)| mesh.used_frame != self.frame_serial)
                            .min_by_key(|(_, mesh)| mesh.used_frame)
                            .map(|(index, _)| index)
                    })
                    .flatten()
            });
        if slot.is_none() && self.retained_meshes.len() >= MAX_RETAINED_MESHES + MAX_CPU_MESHES {
            return Ok(None);
        }
        self.recording_mesh = true;
        let lowered = self.lower_once(list.context(), scale, full);
        self.recording_mesh = false;
        let mut lowered = lowered?;
        // The clear-only dummy belongs to the frame, never to each retained item.
        if let Some(last) = lowered.draws.last_mut() {
            last.geometry.vertex_count = last.geometry.vertex_count.saturating_sub(3);
            if last.geometry.vertex_count == 0 {
                lowered.draws.pop();
            }
            lowered.vertex_bytes.truncate(
                lowered
                    .vertex_bytes
                    .len()
                    .saturating_sub(3 * PAINT_VERTEX_STRIDE as usize),
            );
        }
        if lowered.vertex_bytes.len() as u64 > RETAINED_BUFFER_BYTES {
            return Ok(None);
        }
        if lowered.vertex_bytes.is_empty() {
            return Ok(None);
        }
        let small = lowered.vertex_bytes.len() <= MAX_BATCH_GEOMETRY_BYTES;
        if !small && slot.is_some_and(|index| self.retained_meshes[index].buffer.is_none()) {
            slot = self
                .retained_meshes
                .iter()
                .enumerate()
                .filter(|(_, mesh)| mesh.buffer.is_some() && mesh.used_frame != self.frame_serial)
                .min_by_key(|(_, mesh)| mesh.used_frame)
                .map(|(index, _)| index);
        }
        let gpu_count = self.retained_gpu_mesh_count;
        if slot.is_none() && gpu_count >= MAX_RETAINED_MESHES && !small {
            return Ok(None);
        }
        self.submit_texture_uploads(executor, &lowered)?;
        let buffer = if let Some(index) = slot {
            self.retained_meshes[index].buffer
        } else if gpu_count < MAX_RETAINED_MESHES {
            let buffer = self
                .table
                .define_buffer(
                    BufferDesc::new(
                        RETAINED_BUFFER_BYTES,
                        BufferUsage::VERTEX | BufferUsage::COPY_DST,
                    )
                    .map_err(|_| Error::sgfx(Stage::DefineResources))?,
                )
                .map_err(|_| Error::FrameTooComplex)?
                .id();
            self.retained_gpu_mesh_count += 1;
            Some(buffer)
        } else {
            None
        };
        let slot = if let Some(index) = slot {
            let old = &self.retained_meshes[index];
            let key = (old.list_id, old.scale);
            if self.retained_mesh_index.get(&key) == Some(&index) {
                self.retained_mesh_index.remove(&key);
            }
            self.retained_meshes[index].list_id = 0;
            index
        } else {
            self.retained_meshes.push(RetainedMesh {
                list_id: 0,
                source: Weak::new(),
                scale,
                buffer,
                draws: Vec::new(),
                bounds: FloatRect::new(0., 0., 0., 0.),
                textures: Vec::new(),
                atlases: Vec::new(),
                used_frame: 0,
                pending_vertices: None,
                batch_vertices: None,
            });
            self.retained_meshes.len() - 1
        };
        let mut textures = Vec::new();
        let mut atlases = Vec::new();
        let mut draws = lowered.draws;
        for draw in &mut draws {
            draw.vertex_buffer = buffer;
            match draw.source {
                DrawSource::Texture(id) | DrawSource::PixelTexture(id) => {
                    if let Some(texture) = self
                        .buffer_textures
                        .iter()
                        .find(|texture| texture.texture == id)
                    {
                        if !textures.iter().any(|(texture, _, _)| *texture == id) {
                            textures.push((id, texture.buffer_identity, texture.revision));
                        }
                    }
                }
                DrawSource::Glyph(id) | DrawSource::IconGlyph(id) => {
                    if let Some(atlas) = self.glyph_atlases.iter().find(|atlas| atlas.texture == id)
                    {
                        if !atlases.iter().any(|(texture, _)| *texture == id) {
                            atlases.push((id, atlas.generation));
                        }
                    }
                }
                _ => {}
            }
        }
        let bounds = vertex_bounds(&lowered.vertex_bytes);
        let (pending_vertices, batch_vertices) = if buffer.is_none() {
            (None, Some(lowered.vertex_bytes))
        } else {
            let batch = small.then(|| lowered.vertex_bytes.clone());
            (Some(lowered.vertex_bytes), batch)
        };
        let mesh = RetainedMesh {
            batch_vertices,
            pending_vertices,
            list_id: list.identity(),
            source: Arc::downgrade(list),
            scale,
            buffer,
            draws,
            bounds,
            textures,
            atlases,
            used_frame: self.frame_serial,
        };
        self.retained_mesh_index
            .insert((mesh.list_id, mesh.scale), slot);
        self.retained_meshes[slot] = mesh;
        Ok(Some(slot))
    }
}
pub(super) fn owned_frame(frame: LoweredFrame<'_>) -> LoweredFrame<'static> {
    LoweredFrame {
        vertex_bytes: frame.vertex_bytes,
        draws: frame.draws,
        uploads: frame
            .uploads
            .into_iter()
            .map(|upload| TextureUpload {
                texture: upload.texture,
                x: upload.x,
                y: upload.y,
                width: upload.width,
                height: upload.height,
                bytes_per_row: upload.bytes_per_row,
                bytes: UploadBytes::Shared(match upload.bytes {
                    UploadBytes::Shared(bytes) => bytes,
                    UploadBytes::Borrowed(bytes) => Arc::from(bytes),
                }),
            })
            .collect(),
    }
}
pub(super) fn append_frame(target: &mut LoweredFrame<'_>, frame: LoweredFrame<'_>) -> Result<()> {
    let mut frame = owned_frame(frame);
    let first = u32::try_from(target.vertex_bytes.len() / PAINT_VERTEX_STRIDE as usize)
        .map_err(|_| Error::FrameTooComplex)?;
    if target.vertex_bytes.len() + frame.vertex_bytes.len()
        > MAX_FRAME_VERTICES * PAINT_VERTEX_STRIDE as usize
    {
        return Err(Error::FrameTooComplex);
    }
    for draw in &mut frame.draws {
        if draw.vertex_buffer.is_some() {
            continue;
        }
        draw.geometry.first_vertex = draw
            .geometry
            .first_vertex
            .checked_add(first)
            .ok_or(Error::FrameTooComplex)?;
    }
    target.vertex_bytes.extend(frame.vertex_bytes);
    target.draws.extend(frame.draws);
    target.uploads.extend(frame.uploads);
    Ok(())
}
struct ReplayOffsets {
    shapes: [f32; 2],
}

fn list_has_visible_output(
    list: &DisplayList,
    origin: Point,
    scale: u32,
    clips: &[(Rect, f32)],
    full: PixelBounds,
) -> bool {
    use scarlet_ui_core::renderer::PaintRegion;
    // Preserve stateful streams, opaque extension semantics, and filters whose
    // input footprint can extend beyond their visible output.
    if !list.is_self_contained() || list.has_extensions() || list.reads_backdrop() {
        return true;
    }
    let factor = scale.max(1) as f32 / 1000.;
    let mut visible = FloatRect::new(
        0.,
        0.,
        full.width as f32 / factor,
        full.height as f32 / factor,
    );
    for (clip, _) in clips {
        let Some(intersection) = visible.intersect(FloatRect::new(
            clip.left(),
            clip.top(),
            clip.size.width,
            clip.size.height,
        )) else {
            return false;
        };
        visible = intersection;
    }
    list.paint_dependencies(scale)
        .iter()
        .any(|dependency| match dependency.writes {
            PaintRegion::Unknown => true,
            PaintRegion::Empty => false,
            PaintRegion::Rect(rect) => {
                let bounds = FloatRect::new(
                    origin.x + rect.left(),
                    origin.y + rect.top(),
                    rect.size.width,
                    rect.size.height,
                );
                ![
                    bounds.x,
                    bounds.y,
                    bounds.width,
                    bounds.height,
                    bounds.right(),
                    bounds.bottom(),
                ]
                .iter()
                .all(|v| v.is_finite())
                    || bounds.intersect(visible).is_some()
            }
        })
}
fn text_replay_offset(origin: f32, anchor: f32, scale: u32) -> f32 {
    scale_text_origin(origin + anchor, scale) as f32 - scale_text_origin(anchor, scale) as f32
}
fn replay_offset(list: &DisplayList, origin: Point, scale_milli: u32) -> Option<ReplayOffsets> {
    if !origin.x.is_finite() || !origin.y.is_finite() {
        return None;
    }
    let scale = scale_milli.max(1) as f32 / 1000.;
    for command in list.context().commands() {
        match command {
            PaintCommand::FillVerticalGradientRoundedRect { .. }
            | PaintCommand::FillPath { .. }
            | PaintCommand::FillRoundedRect { .. }
            | PaintCommand::StrokePath { .. }
            | PaintCommand::StrokeRect { .. }
            | PaintCommand::StrokeRoundedRect { .. }
            | PaintCommand::DrawRoundedRectShadow { .. } => {}
            PaintCommand::DrawText { .. } => {}
            // A local rectangular scissor is translation invariant at integral
            // physical offsets. Fractional shifts must rerun clipping/rounding.
            PaintCommand::PushClip { corner_radius, .. }
                if *corner_radius == 0.
                    && origin.x * scale == libm::floorf(origin.x * scale)
                    && origin.y * scale == libm::floorf(origin.y * scale) => {}
            PaintCommand::PopClip
                if origin.x * scale == libm::floorf(origin.x * scale)
                    && origin.y * scale == libm::floorf(origin.y * scale) => {}
            PaintCommand::DrawBuffer { dst, .. } | PaintCommand::DrawBufferRect { dst, .. }
                if scale_milli % 1000 == 0
                    && origin.x >= 0.
                    && origin.y >= 0.
                    && dst.origin.x >= 0.
                    && dst.origin.y >= 0. => {}
            PaintCommand::DrawIcon { rect, .. }
                if scale_milli % 1000 == 0
                    && origin.x >= 0.
                    && origin.y >= 0.
                    && rect.origin.x >= 0.
                    && rect.origin.y >= 0. => {}
            _ => return None,
        }
    }
    Some(ReplayOffsets {
        shapes: [origin.x * scale, origin.y * scale],
    })
}
pub(super) fn vertex_bounds(bytes: &[u8]) -> FloatRect {
    let (mut left, mut top, mut right, mut bottom) = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for vertex in bytes.chunks_exact(PAINT_VERTEX_STRIDE as usize) {
        let x = f32::from_le_bytes(vertex[0..4].try_into().unwrap());
        let y = f32::from_le_bytes(vertex[4..8].try_into().unwrap());
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }
    FloatRect::new(left, top, (right - left).max(0.), (bottom - top).max(0.))
}
// None means replay needs precise geometry clipping. Empty intersections are
// deliberately left to the fallback so clear-only damage is still submitted.
fn replay_scissor(
    bounds: FloatRect,
    offset: [f32; 2],
    clips: &[(Rect, f32)],
    full: PixelBounds,
    scale_milli: u32,
) -> Option<PixelBounds> {
    let mut scissor = FloatRect::new(0., 0., full.width as f32, full.height as f32);
    // Include raster rounding displacement before accepting a rounded clip's core.
    let pad = scale_milli.max(1) as f32 / 1000. + 1.;
    let bounds = FloatRect::new(
        bounds.x + offset[0] - pad,
        bounds.y + offset[1] - pad,
        bounds.width + 2. * pad,
        bounds.height + 2. * pad,
    );
    let scale = scale_milli.max(1) as f32 / 1000.;
    for (rect, radius) in clips {
        let clip = FloatRect::new(
            rect.origin.x * scale,
            rect.origin.y * scale,
            rect.size.width * scale,
            rect.size.height * scale,
        );
        let radius = *radius * scale;
        if radius > 0. {
            let radius = radius.min(clip.width / 2.).min(clip.height / 2.);
            let inside = bounds.x >= clip.x
                && bounds.y >= clip.y
                && bounds.right() <= clip.right()
                && bounds.bottom() <= clip.bottom();
            let vertical = bounds.x >= clip.x + radius && bounds.right() <= clip.right() - radius;
            let horizontal =
                bounds.y >= clip.y + radius && bounds.bottom() <= clip.bottom() - radius;
            if !inside || !(vertical || horizontal) {
                return None;
            }
        }
        scissor = scissor.intersect(clip)?;
    }
    let left = libm::floorf(scissor.x).max(0.) as u32;
    let top = libm::floorf(scissor.y).max(0.) as u32;
    let right = libm::ceilf(scissor.right()).min(full.width as f32) as u32;
    let bottom = libm::ceilf(scissor.bottom()).min(full.height as f32) as u32;
    (right > left && bottom > top).then_some(PixelBounds {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui_core::geometry::{Rect, Size};
    use sgfx::ir::{Command, CommandBuffer};
    #[derive(Default)]
    struct Executor {
        write_batches: Vec<usize>,
        writes: usize,
        texture_writes: usize,
        passes: usize,
        fail_write: bool,
        buffer_data: Vec<Vec<u8>>,
    }
    impl CommandExecutor for Executor {
        type Error = ();
        fn execute<'r, 'd>(
            &mut self,
            commands: &CommandBuffer<'r, 'd>,
        ) -> core::result::Result<(), ()> {
            let writes = commands
                .commands()
                .iter()
                .filter(|command| matches!(command, Command::WriteBuffer { .. }))
                .count();
            if writes > 0 {
                self.write_batches.push(writes);
            }
            for command in commands.commands() {
                match command {
                    Command::WriteBuffer { data, .. } => {
                        self.buffer_data.push(data.to_vec());
                        self.writes += 1;
                        if self.fail_write {
                            self.fail_write = false;
                            return Err(());
                        }
                    }
                    Command::WriteTexture { .. } => self.texture_writes += 1,
                    Command::BeginRenderPass(_) => self.passes += 1,
                    _ => {}
                }
            }
            Ok(())
        }
    }
    fn encoder() -> SgfxPaintEncoder {
        super::super::ensure_test_font();
        SgfxPaintEncoder::new(200, 120, false).unwrap()
    }
    fn shape() -> Arc<DisplayList> {
        let mut c = PaintContext::new();
        c.fill_rounded_rect(
            Rect::from_xywh(0., 0., 40., 30.),
            5.,
            UiColor::rgb(200, 20, 40),
        );
        c.into_display_list()
    }
    fn paint(list: Arc<DisplayList>, origin: Point) -> PaintContext<'static> {
        let mut c = PaintContext::new();
        c.draw_display_list(origin, list);
        c
    }
    fn encode(
        encoder: &mut SgfxPaintEncoder,
        executor: &mut Executor,
        paint: &PaintContext<'_>,
    ) -> core::result::Result<(), FrameError<()>> {
        encoder.encode_frame(
            executor,
            0,
            None,
            paint,
            UiColor::WHITE,
            1000,
            &[(0, 0, 200, 120)],
        )
    }
    #[test]
    fn glass_keeps_static_mesh_and_shadow_caches_warm() {
        let mut encoder = encoder();
        let shape = shape();
        let mut shadow = PaintContext::new();
        shadow.draw_rounded_rect_shadow(
            Rect::from_xywh(20.0, 20.0, 100.0, 36.0),
            18.0,
            scarlet_ui_core::geometry::Offset::new(0.0, 2.0),
            8.0,
            0.0,
            UiColor::BLACK.with_opacity(0.1),
        );
        let shadow = shadow.into_display_list();
        let mut glass = PaintContext::new();
        glass.refractive_glass_rounded_rect(
            Rect::from_xywh(20.0, 20.0, 100.0, 36.0),
            18.0,
            6.0,
            UiColor::WHITE.with_opacity(0.4),
            1.0,
        );
        let glass = glass.into_display_list();
        let scene = |x| {
            let mut p = PaintContext::new();
            p.draw_display_list(Point::new(x, 0.0), Arc::clone(&shape));
            p.draw_display_list(Point::ZERO, Arc::clone(&shadow));
            p.draw_display_list(Point::ZERO, Arc::clone(&glass));
            p
        };
        let mut cold = Executor::default();
        encode(&mut encoder, &mut cold, &scene(0.0)).unwrap();
        assert_eq!(encoder.retained_meshes.len(), 1);
        assert_eq!(
            encoder.shadow_cache.as_ref().unwrap().list_id,
            shadow.identity()
        );
        let mut warm = Executor::default();
        encode(&mut encoder, &mut warm, &scene(4.0)).unwrap();
        assert_eq!(encoder.retained_meshes.len(), 1);
        assert_eq!(
            encoder.backdrop_filters, 1,
            "background must be resampled on warm frames"
        );
        assert!(
            warm.writes < cold.writes,
            "unchanged mesh buffers must not be uploaded again"
        );
        assert!(
            warm.passes < cold.passes,
            "shadow coverage must not be rerendered"
        );
    }

    #[test]
    fn recording_state_can_span_opacity_and_clip_boundaries() {
        for opacity in [false, true] {
            let mut local = PaintContext::new();
            if opacity {
                local.set_opacity(0.4);
            } else {
                local.push_clip(Rect::from_xywh(10., 10., 20., 20.));
            }
            let list = local.into_display_list();
            assert!(!list.is_self_contained());
            let mut paint = PaintContext::new();
            paint.draw_display_list(Point::ZERO, list);
            paint.fill_rounded_rect(Rect::from_xywh(0., 0., 80., 80.), 0., UiColor::RED);
            if !opacity {
                paint.pop_clip();
            }
            let flat = paint.flattened();
            let area = PixelBounds {
                x: 0,
                y: 0,
                width: 200,
                height: 120,
            };
            let expected = encoder().lower(&flat, 1000, area).unwrap().vertex_bytes;
            let actual = encoder()
                .lower_retained(&mut Executor::default(), &paint, 1000, area)
                .unwrap()
                .vertex_bytes;
            assert_eq!(
                actual, expected,
                "state must affect commands following the recording"
            );
        }
    }
    #[test]
    fn partial_damage_outside_a_fallback_clip_remains_a_valid_frame() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        let mut local = PaintContext::new();
        local.stroke_rounded_rect(
            Rect::from_xywh(0.5, 0.5, 119., 39.),
            7.5,
            1.,
            UiColor::rgb(220, 220, 220),
        );
        let mut paint = PaintContext::new();
        paint.push_rounded_clip(Rect::from_xywh(10., 70., 120., 40.), 8.);
        paint.draw_display_list(Point::new(10., 70.), local.into_display_list());
        paint.pop_clip();
        encoder
            .encode_frame(
                &mut executor,
                0,
                None,
                &paint,
                UiColor::WHITE,
                1000,
                &[(0, 0, 200, 20)],
            )
            .unwrap();
        assert!(
            executor.passes > 0,
            "clear-only partial damage must still be submitted"
        );
    }
    #[test]
    fn warm_scroll_reuses_gpu_vertices_without_buffer_upload() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        let list = shape();
        encode(
            &mut encoder,
            &mut executor,
            &paint(list.clone(), Point::new(20., 20.)),
        )
        .unwrap();
        let cold = executor.writes;
        assert!(cold > 0);
        for y in [19., 18.5, 15., 10.] {
            encode(
                &mut encoder,
                &mut executor,
                &paint(list.clone(), Point::new(20., y)),
            )
            .unwrap();
        }
        assert_eq!(
            executor.writes, cold,
            "position changes must use GPU transforms instead of vertex uploads"
        );
        assert_eq!(encoder.retained_meshes.len(), 1);
    }
    #[test]
    fn offscreen_live_recordings_keep_their_gpu_meshes() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        let lists: Vec<_> = (0..8).map(|_| shape()).collect();
        for list in &lists {
            encode(
                &mut encoder,
                &mut executor,
                &paint(list.clone(), Point::new(20., 20.)),
            )
            .unwrap();
        }
        let cold = executor.writes;
        for list in lists.iter().rev().chain(lists.iter()) {
            encode(
                &mut encoder,
                &mut executor,
                &paint(list.clone(), Point::new(20., 15.)),
            )
            .unwrap();
        }
        assert_eq!(
            executor.writes, cold,
            "returning live recordings need no vertex upload"
        );
        assert_eq!(encoder.retained_meshes.len(), lists.len());
        drop(lists);
        encode(&mut encoder, &mut executor, &paint(shape(), Point::ZERO)).unwrap();
        assert_eq!(
            encoder.retained_meshes.len(),
            8,
            "expired slots must be reused first"
        );
    }

    #[test]
    fn changed_list_and_failed_slot_replacement_do_not_reuse_stale_vertices() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        let old = shape();
        encode(
            &mut encoder,
            &mut executor,
            &paint(old.clone(), Point::new(20., 20.)),
        )
        .unwrap();
        // Fill the cache with still-live recordings so the next mesh must
        // replace the old LRU slot rather than allocate a fresh buffer.
        let live: Vec<_> = (1..MAX_RETAINED_MESHES).map(|_| shape()).collect();
        for list in &live {
            encode(
                &mut encoder,
                &mut executor,
                &paint(list.clone(), Point::ZERO),
            )
            .unwrap();
        }
        let replacement = shape();
        encode(
            &mut encoder,
            &mut executor,
            &paint(replacement.clone(), Point::new(20., 20.)),
        )
        .unwrap();
        executor.fail_write = true;
        assert!(
            encode(
                &mut encoder,
                &mut executor,
                &paint(replacement, Point::new(20., 20.))
            )
            .is_err()
        );
        let writes = executor.writes;
        encode(
            &mut encoder,
            &mut executor,
            &paint(old, Point::new(20., 20.)),
        )
        .unwrap();
        assert!(
            executor.writes > writes,
            "failed replacement invalidates the previous slot"
        );
    }

    #[test]
    fn offscreen_recordings_do_not_consume_mesh_slots_or_uploads() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        let lists: Vec<_> = (0..1024).map(|_| shape()).collect();
        let mut context = PaintContext::new();
        for list in &lists {
            context.draw_display_list(Point::new(0., 10000.), list.clone());
        }
        context.draw_display_list(Point::new(10., 10.), lists[0].clone());
        encode(&mut encoder, &mut executor, &context).unwrap();
        assert_eq!(encoder.retained_meshes.len(), 1);
        assert_eq!(executor.writes, 1);
        encode(
            &mut encoder,
            &mut executor,
            &paint(lists[1].clone(), Point::ZERO),
        )
        .unwrap();
        assert_eq!(
            encoder.retained_meshes.len(),
            2,
            "newly visible recording must render"
        );
    }

    #[test]
    fn visibility_keeps_fractional_edge_coverage_and_unknown_extension_output() {
        let full = PixelBounds {
            x: 0,
            y: 0,
            width: 200,
            height: 120,
        };
        let shape = shape();
        assert!(list_has_visible_output(
            &shape,
            Point::new(-39.75, 0.),
            2000,
            &[],
            full
        ));
        assert!(!list_has_visible_output(
            &shape,
            Point::new(-100., 0.),
            2000,
            &[],
            full
        ));
        assert!(!list_has_visible_output(
            &shape,
            Point::ZERO,
            2000,
            &[(Rect::from_xywh(80., 0., 20., 20.), 0.)],
            full
        ));
        let mut extension = PaintContext::new();
        extension.draw_extension(Rect::from_xywh(0., 0., 1., 1.), Arc::new(0u8));
        assert!(list_has_visible_output(
            &extension.into_display_list(),
            Point::new(1000., 1000.),
            2000,
            &[],
            full
        ));
    }

    #[test]
    fn cold_mesh_uploads_are_batched_and_failed_batches_are_retried_before_replay() {
        let mut encoder = encoder();
        let lists: Vec<_> = (0..8).map(|_| shape()).collect();
        let mut context = PaintContext::new();
        for (i, list) in lists.iter().enumerate() {
            context.draw_display_list(Point::new(i as f32 * 10., 0.), list.clone());
        }
        let mut executor = Executor {
            fail_write: true,
            ..Default::default()
        };
        assert!(encode(&mut encoder, &mut executor, &context).is_err());
        assert_eq!(executor.write_batches, [8]);
        assert_eq!(executor.passes, 0, "failed uploads must not be drawn");
        assert_eq!(
            encoder
                .retained_meshes
                .iter()
                .filter(|mesh| mesh.pending_vertices.is_some())
                .count(),
            8
        );
        encode(&mut encoder, &mut executor, &context).unwrap();
        assert_eq!(
            executor.write_batches[1], 8,
            "retry every upload after a partially accepted batch"
        );
        assert!(
            encoder
                .retained_meshes
                .iter()
                .all(|mesh| mesh.pending_vertices.is_none())
        );
        let writes = executor.writes;
        encode(&mut encoder, &mut executor, &context).unwrap();
        assert_eq!(executor.writes, writes, "warm meshes need no uploads");
    }
    #[test]
    fn rounded_clip_edges_fall_back_but_interior_uses_replay() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        let list = shape();
        let mut context = PaintContext::new();
        context.push_rounded_clip(Rect::from_xywh(0., 0., 100., 100.), 20.);
        context.draw_display_list(Point::new(30., 30.), list.clone());
        context.pop_clip();
        encode(&mut encoder, &mut executor, &context).unwrap();
        let writes = executor.writes;
        encode(&mut encoder, &mut executor, &context).unwrap();
        assert_eq!(writes, executor.writes);
        let mut edge = PaintContext::new();
        edge.push_rounded_clip(Rect::from_xywh(0., 0., 100., 100.), 20.);
        edge.draw_display_list(Point::new(-5., -5.), list);
        edge.pop_clip();
        encode(&mut encoder, &mut executor, &edge).unwrap();
        assert!(
            executor.writes > writes,
            "rounded geometry crossing the clip edge needs precise clipping"
        );
    }
    #[test]
    fn nested_atlas_exhaustion_defers_rebuild_to_the_complete_frame() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        encoder.glyph_atlases.clear();
        for _ in 0..MAX_GLYPH_ATLASES {
            let texture = define_sampled_texture(
                &encoder.table,
                TextureFormat::R8Unorm,
                GLYPH_ATLAS_SIZE,
                GLYPH_ATLAS_SIZE,
            )
            .unwrap();
            let mut atlas = GlyphAtlas::new(texture);
            atlas.cursor_y = GLYPH_ATLAS_SIZE;
            atlas.used_frame = encoder.frame_serial;
            encoder.glyph_atlases.push(atlas);
        }
        let generations: Vec<_> = encoder.glyph_atlases.iter().map(|a| a.generation).collect();
        let mut paint = PaintContext::new();
        paint.draw_text(Point::new(10., 20.), "Nested", UiColor::BLACK, 14.);
        let area = PixelBounds {
            x: 0,
            y: 0,
            width: 200,
            height: 120,
        };
        encoder.tile_depth = 1;
        assert!(matches!(
            encoder.lower_retained(&mut executor, &paint, 1000, area),
            Err(FrameError::Lowering(Error::FrameTooComplex))
        ));
        assert!(encoder.glyph_atlas_rebuild_required);
        assert_eq!(
            encoder
                .glyph_atlases
                .iter()
                .map(|a| a.generation)
                .collect::<Vec<_>>(),
            generations,
            "a nested retry must not reset atlases referenced by pending enclosing draws"
        );
        encoder.tile_depth = 0;
        encoder
            .lower_retained(&mut executor, &paint, 1000, area)
            .unwrap();
        assert!(
            encoder
                .glyph_atlases
                .iter()
                .zip(generations)
                .all(|(a, old)| a.generation > old)
        );
    }

    #[test]
    fn atlas_reset_and_recycled_image_texture_invalidate_meshes() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        let image = Arc::new(Buffer::new(Size::new(8., 8.)));
        let mut record = PaintContext::new();
        record.draw_buffer_rect_shared(
            Rect::from_xywh(0., 0., 30., 30.),
            Rect::from_xywh(0., 0., 8., 8.),
            image,
            1.,
        );
        let list = record.into_display_list();
        let frame = paint(list.clone(), Point::new(20., 20.));
        encode(&mut encoder, &mut executor, &frame).unwrap();
        let writes = executor.writes;
        encoder.buffer_textures[0].buffer_identity += 100;
        encode(&mut encoder, &mut executor, &frame).unwrap();
        assert!(executor.writes > writes);
        let mut record = PaintContext::new();
        record.draw_text(Point::ZERO, "Hello", UiColor::BLACK, 13.);
        let frame = paint(record.into_display_list(), Point::new(20., 20.));
        encode(&mut encoder, &mut executor, &frame).unwrap();
        let writes = executor.writes;
        for atlas in &mut encoder.glyph_atlases {
            atlas.reset(0);
        }
        encode(&mut encoder, &mut executor, &frame).unwrap();
        assert!(executor.writes > writes);
    }
    #[test]
    fn many_live_small_recordings_share_cpu_cache_without_extra_gpu_buffers() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        let mut context = PaintContext::new();
        for _ in 0..MAX_RETAINED_MESHES + MAX_CPU_MESHES + 20 {
            let mut record = PaintContext::new();
            record.fill_rect(Rect::from_xywh(0., 0., 8., 8.), UiColor::BLACK);
            context.draw_display_list(Point::new(20., 20.), record.into_display_list());
        }
        encode(&mut encoder, &mut executor, &context).unwrap();
        assert_eq!(encoder.retained_meshes.len(), MAX_RETAINED_MESHES);
        // Large scenes admit additional recordings only after cross-frame reuse.
        encode(&mut encoder, &mut executor, &context).unwrap();
        assert_eq!(
            encoder.retained_meshes.len(),
            MAX_RETAINED_MESHES + MAX_CPU_MESHES
        );
        assert_eq!(
            encoder
                .retained_meshes
                .iter()
                .filter(|mesh| mesh.buffer.is_some())
                .count(),
            MAX_RETAINED_MESHES
        );
        assert!(
            encoder
                .retained_meshes
                .iter()
                .filter(|mesh| mesh.buffer.is_none())
                .all(|mesh| mesh.batch_vertices.as_ref().unwrap().len()
                    <= MAX_BATCH_GEOMETRY_BYTES
                    && mesh.pending_vertices.is_none())
        );
        let writes = executor.writes;
        encode(&mut encoder, &mut executor, &context).unwrap();
        assert_eq!(
            executor.writes - writes,
            1,
            "warm CPU records share the single frame upload"
        );
    }

    #[test]
    fn transient_recordings_do_not_displace_stable_large_scene_meshes() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        let small = || {
            let mut p = PaintContext::new();
            p.fill_rect(Rect::from_xywh(0., 0., 8., 8.), UiColor::BLACK);
            p.into_display_list()
        };
        let stable: Vec<_> = (0..MAX_RETAINED_MESHES + 32).map(|_| small()).collect();
        for frame in 0..4 {
            let mut context = PaintContext::new();
            for list in &stable {
                // New immutable IDs every frame: render normally, do not retain.
                context.draw_display_list(Point::new(20., 20.), shape());
                context.draw_display_list(Point::new(20., 20.), list.clone());
            }
            encode(&mut encoder, &mut executor, &context).unwrap();
            if frame >= 2 {
                assert!(
                    stable
                        .iter()
                        .all(|list| encoder.cached_mesh(list, 1000).is_some())
                );
            }
        }
        assert!(encoder.mesh_candidates.len() <= 4096);
        assert!(encoder.previous_mesh_candidates.len() <= 4096);
    }

    #[test]
    fn cache_slots_are_bounded_and_empty_scene_still_clears() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        for _ in 0..MAX_RETAINED_MESHES + 20 {
            encode(
                &mut encoder,
                &mut executor,
                &paint(shape(), Point::new(20., 20.)),
            )
            .unwrap();
        }
        assert!(encoder.retained_meshes.len() <= MAX_RETAINED_MESHES);
        let passes = executor.passes;
        encode(&mut encoder, &mut executor, &PaintContext::new()).unwrap();
        assert!(executor.passes > passes);
    }
    #[test]
    fn clipped_text_mesh_reuses_integral_offsets_and_rejects_unsafe_clip_translation() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        let mut text = PaintContext::new();
        text.push_clip(Rect::from_xywh(0., 0., 40., 14.));
        text.draw_text(Point::new(0.5, 0.25), "clipped label", UiColor::BLACK, 13.);
        text.pop_clip();
        let list = text.into_display_list();
        assert!(replay_offset(&list, Point::new(2., 4.), 1000).is_some());
        assert!(replay_offset(&list, Point::new(2.25, 4.), 1000).is_none());
        assert!(replay_offset(&list, Point::new(2.5, 4.), 2000).is_some());
        let frame = paint(list.clone(), Point::new(20., 20.));
        encode(&mut encoder, &mut executor, &frame).unwrap();
        assert!(encoder.cached_mesh(&list, 1000).is_some());
        let writes = executor.writes;
        encode(
            &mut encoder,
            &mut executor,
            &paint(list, Point::new(24., 16.)),
        )
        .unwrap();
        assert_eq!(executor.writes, writes);
        let mut out = PaintContext::new();
        out.push_clip(Rect::from_xywh(-10., 0., 40., 20.));
        out.draw_text(Point::new(-5., 0.), "outside", UiColor::BLACK, 13.);
        out.pop_clip();
        let out = out.into_display_list();
        encode(
            &mut encoder,
            &mut executor,
            &paint(out.clone(), Point::new(20., 20.)),
        )
        .unwrap();
        assert!(encoder.cached_mesh(&out, 1000).is_none());
    }

    #[test]
    fn fractional_scale_text_keeps_precise_origin_during_replay() {
        let mut c = PaintContext::new();
        c.draw_text(Point::ZERO, "Hello", UiColor::BLACK, 13.);
        assert!(replay_offset(&c.into_display_list(), Point::new(10.5, 20.5), 1500).is_some());
        let mut c = PaintContext::new();
        c.draw_text(Point::ZERO, "Hello", UiColor::BLACK, 13.);
        c.draw_text(Point::new(0.5, 10.), "World", UiColor::BLACK, 13.);
        assert!(replay_offset(&c.into_display_list(), Point::new(10.5, 20.5), 1000).is_some());
    }

    #[test]
    fn explicit_font_stack_uses_glyph_atlases_and_reuses_retained_meshes() {
        use scarlet_ui_core::graphics::{FontStack, rasterize_text_with_font_stack};
        let stack = FontStack::new(include_bytes!(
            "../../../scarlet-ui-core/src/testing/fonts/metrics-primary.ttf"
        ))
        .unwrap()
        .with_fallback(include_bytes!(
            "../../../scarlet-ui-core/src/testing/fonts/metrics-fallback.ttf"
        ))
        .unwrap();
        let mut recording = PaintContext::new();
        recording.draw_text_with_font_stack(
            Point::new(0.5, 2.5),
            "日☃",
            UiColor::BLACK,
            20.,
            Arc::new(stack.clone()),
        );
        let list = recording.into_display_list();
        let area = PixelBounds {
            x: 0,
            y: 0,
            width: 200,
            height: 120,
        };
        for scale in [1000, 1250, 2000] {
            let mut encoder = encoder();
            let mut executor = Executor::default();
            let context = paint(list.clone(), Point::new(10.25, 10.25));
            let flat = context.flattened();
            let frame = encoder.lower_once(&flat, scale, area).unwrap();
            let glyphs = rasterize_text_with_font_stack("日☃", 20., scale, &stack);
            let draws: Vec<_> = frame
                .draws
                .iter()
                .filter(|draw| matches!(draw.source, DrawSource::Glyph(_)))
                .collect();
            assert!(!draws.is_empty());
            assert!(frame.draws.iter().all(|draw| matches!(
                draw.source,
                DrawSource::Glyph(_) | DrawSource::Solid | DrawSource::ClearOnly
            )));
            let glyph_vertices: Vec<_> = draws
                .iter()
                .flat_map(|draw| {
                    let start = draw.geometry.first_vertex as usize * PAINT_VERTEX_STRIDE as usize;
                    let end =
                        start + draw.geometry.vertex_count as usize * PAINT_VERTEX_STRIDE as usize;
                    frame.vertex_bytes[start..end].iter().copied()
                })
                .collect();
            assert_eq!(glyph_vertices.len(), 2 * 6 * PAINT_VERTEX_STRIDE as usize);
            for (vertices, glyph) in glyph_vertices
                .chunks_exact(6 * PAINT_VERTEX_STRIDE as usize)
                .zip(&glyphs)
            {
                let bounds = vertex_bounds(vertices);
                assert_eq!(bounds.x, (scale_text_origin(10.75, scale) + glyph.x) as f32);
                let factor = scale as f32 / 1000.0;
                let ascent = 20.0 * 1.160;
                let baseline = libm::floorf((12.75 + ascent) * factor + 0.5) as i32;
                let cached_ascent = libm::floorf(ascent * factor + 0.5) as i32;
                assert_eq!(bounds.y, (baseline - cached_ascent + glyph.y) as f32);
                assert_eq!(bounds.width, glyph.width as f32);
                assert_eq!(bounds.height, glyph.height as f32);
                assert!(
                    frame
                        .uploads
                        .iter()
                        .any(|upload| upload.bytes.as_slice() == glyph.mask.as_ref())
                );
                assert!(
                    encoder
                        .glyph_atlases
                        .iter()
                        .any(|atlas| atlas.entries.iter().any(|entry| entry.key == glyph.key))
                );
            }
            drop(frame);

            encoder
                .encode_frame(
                    &mut executor,
                    0,
                    None,
                    &context,
                    UiColor::WHITE,
                    scale,
                    &[(0, 0, 200, 120)],
                )
                .unwrap();
            let writes = (executor.writes, executor.texture_writes);
            let moved = paint(list.clone(), Point::new(12.75, 10.75));
            encoder
                .encode_frame(
                    &mut executor,
                    0,
                    None,
                    &moved,
                    UiColor::WHITE,
                    scale,
                    &[(0, 0, 200, 120)],
                )
                .unwrap();
            assert_eq!(encoder.retained_meshes.len(), 1);
            assert_eq!((executor.writes, executor.texture_writes), writes);
            assert!(encoder.buffer_textures.is_empty());
        }
    }
    #[test]
    fn fractional_scroll_replay_matches_flat_shape_text_and_image_vertices() {
        for scale in [1000, 1250, 2000] {
            let mut encoder = encoder();
            let mut executor = Executor::default();
            let mut recording = PaintContext::new();
            recording.fill_rounded_rect(Rect::from_xywh(0., 0., 100., 40.), 4., UiColor::WHITE);
            recording.draw_text(Point::new(0.5, 12.5), "Hello", UiColor::BLACK, 13.);
            recording.draw_text(Point::new(60., 13.), "World", UiColor::BLACK, 12.);
            if scale % 1000 == 0 {
                recording.draw_buffer_rect_shared(
                    Rect::from_xywh(4.5, 2.5, 8., 8.),
                    Rect::from_xywh(0., 0., 8., 8.),
                    Arc::new(Buffer::new(Size::new(8., 8.))),
                    1.,
                );
            }
            let font = Arc::new(
                scarlet_ui_core::graphics::FontStack::new(include_bytes!(
                    "../../../scarlet-ui-core/src/testing/fonts/metrics-primary.ttf"
                ))
                .unwrap(),
            );
            recording.draw_text_with_font_stack(
                Point::new(0.5, 2.5),
                "A日",
                UiColor::BLACK,
                13.25,
                font,
            );
            let list = recording.into_display_list();
            let area = PixelBounds {
                x: 0,
                y: 0,
                width: 200,
                height: 120,
            };
            encoder.advance_frame_serial();
            let warm = paint(list.clone(), Point::new(10., 20.));
            encoder
                .lower_retained(&mut executor, &warm, scale, area)
                .unwrap();
            let bytes = executor.buffer_data[0].clone();
            for origin in [
                Point::new(10.25, 20.25),
                Point::new(10.75, 20.75),
                Point::new(10.85, 20.85),
            ] {
                encoder.advance_frame_serial();
                let context = paint(list.clone(), origin);
                let replay = encoder
                    .lower_retained(&mut executor, &context, scale, area)
                    .unwrap();
                let flat = context.flattened();
                let reference = encoder.lower_once(&flat, scale, area).unwrap();
                let mut positions = Vec::new();
                for draw in replay.draws {
                    let start = draw.geometry.first_vertex as usize * PAINT_VERTEX_STRIDE as usize;
                    let end =
                        start + draw.geometry.vertex_count as usize * PAINT_VERTEX_STRIDE as usize;
                    for vertex in bytes[start..end].chunks_exact(PAINT_VERTEX_STRIDE as usize) {
                        positions.push([
                            f32::from_le_bytes(vertex[0..4].try_into().unwrap()) + draw.offset[0],
                            f32::from_le_bytes(vertex[4..8].try_into().unwrap()) + draw.offset[1],
                        ]);
                    }
                }
                let expected: Vec<_> = reference.vertex_bytes
                    [..reference.vertex_bytes.len() - 3 * PAINT_VERTEX_STRIDE as usize]
                    .chunks_exact(PAINT_VERTEX_STRIDE as usize)
                    .map(|vertex| {
                        [
                            f32::from_le_bytes(vertex[0..4].try_into().unwrap()),
                            f32::from_le_bytes(vertex[4..8].try_into().unwrap()),
                        ]
                    })
                    .collect();
                assert_eq!(positions.len(), expected.len());
                for (actual, expected) in positions.iter().zip(expected) {
                    assert!(
                        (actual[0] - expected[0]).abs() < 0.001
                            && (actual[1] - expected[1]).abs() < 0.001,
                        "{actual:?} != {expected:?}"
                    );
                }
            }
        }
    }
}
