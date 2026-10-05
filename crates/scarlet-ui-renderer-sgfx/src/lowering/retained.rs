//! Bounded GPU mesh reuse for immutable local-coordinate recordings.
//! Rounded outer clips and commands with position-dependent rasterization use
//! the normal lowering path whenever replay cannot preserve their semantics.
use super::*;
use scarlet_ui_core::{
    geometry::{Point, Rect},
    renderer::DisplayList,
};
const MAX_RETAINED_MESHES: usize = 256;
const RETAINED_BUFFER_BYTES: u64 = 65_536; // At most 16 MiB; slots are recycled.

pub(super) struct RetainedMesh {
    list_id: u64,
    scale: u32,
    buffer: BufferId,
    draws: Vec<Draw>,
    bounds: FloatRect,
    textures: Vec<(TextureId, u64, u64)>,
    atlases: Vec<(TextureId, u64)>,
    used_frame: u64,
}
impl SgfxPaintEncoder {
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
        match self.try_lower_retained(executor, paint, scale, area) {
            Err(FrameError::Lowering(Error::FrameTooComplex))
                if self.glyph_atlas_rebuild_required =>
            {
                // A reset would invalidate draws already prepared this frame.
                // Rebuild the entire current frame atomically with the flat path.
                for atlas in &mut self.glyph_atlases {
                    atlas.reset(self.frame_serial);
                }
                let flat = paint.flattened();
                let frame = self.lower(&flat, scale, area)?;
                Ok(owned_frame(frame))
            }
            result => result,
        }
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
                if let Some(index) = self.retained_meshes.iter().position(|mesh| {
                    mesh.list_id == list.identity() && mesh.scale == scale && self.mesh_valid(mesh)
                }) {
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
                if opacity == 1. {
                    if let Some(offsets) = replay_offset(list, *origin, scale) {
                        let cached = self.retained_meshes.iter().position(|mesh| {
                            mesh.list_id == list.identity()
                                && mesh.scale == scale
                                && self.mesh_valid(mesh)
                        });
                        let slot = if let Some(index) = cached {
                            Some(index)
                        } else {
                            self.record_mesh(executor, list, scale, full)?
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
                                            scale_text_origin(origin.x + draw.snap_phase[0], scale)
                                                as f32,
                                            scale_text_origin(origin.y + draw.snap_phase[1], scale)
                                                as f32,
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
                                        DrawSource::Solid => offsets.shapes,
                                    };
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
            let one = paint.command_context(index);
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
    fn record_mesh<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        list: &Arc<DisplayList>,
        scale: u32,
        full: PixelBounds,
    ) -> core::result::Result<Option<usize>, FrameError<E::Error>> {
        let slot = self
            .retained_meshes
            .iter()
            .enumerate()
            .filter(|(_, mesh)| mesh.used_frame != self.frame_serial)
            .min_by_key(|(_, mesh)| mesh.used_frame)
            .map(|(index, _)| index);
        if slot.is_none() && self.retained_meshes.len() >= MAX_RETAINED_MESHES {
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
        self.submit_texture_uploads(executor, &lowered)?;
        let buffer = if let Some(index) = slot {
            self.retained_meshes[index].buffer
        } else {
            self.table
                .define_buffer(
                    BufferDesc::new(
                        RETAINED_BUFFER_BYTES,
                        BufferUsage::VERTEX | BufferUsage::COPY_DST,
                    )
                    .map_err(|_| Error::sgfx(Stage::DefineResources))?,
                )
                .map_err(|_| Error::FrameTooComplex)?
                .id()
        };
        let slot = if let Some(index) = slot {
            self.retained_meshes[index].list_id = 0;
            index
        } else {
            self.retained_meshes.push(RetainedMesh {
                list_id: 0,
                scale,
                buffer,
                draws: Vec::new(),
                bounds: FloatRect::new(0., 0., 0., 0.),
                textures: Vec::new(),
                atlases: Vec::new(),
                used_frame: 0,
            });
            self.retained_meshes.len() - 1
        };
        let mut encoder = CommandEncoder::new(&self.table);
        let reference = self
            .table
            .buffer_ref(buffer)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        encoder
            .write_buffer(reference, 0, &lowered.vertex_bytes)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let commands = encoder
            .finish()
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        executor.execute(&commands).map_err(FrameError::Execution)?;
        let mut textures = Vec::new();
        let mut atlases = Vec::new();
        let mut draws = lowered.draws;
        for draw in &mut draws {
            draw.vertex_buffer = Some(buffer);
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
        let mesh = RetainedMesh {
            list_id: list.identity(),
            scale,
            buffer,
            draws,
            bounds,
            textures,
            atlases,
            used_frame: self.frame_serial,
        };
        self.retained_meshes[slot] = mesh;
        Ok(Some(slot))
    }
}
fn owned_frame(frame: LoweredFrame<'_>) -> LoweredFrame<'static> {
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
fn append_frame(target: &mut LoweredFrame<'_>, frame: LoweredFrame<'_>) -> Result<()> {
    let mut frame = owned_frame(frame);
    let first = u32::try_from(target.vertex_bytes.len() / PAINT_VERTEX_STRIDE as usize)
        .map_err(|_| Error::FrameTooComplex)?;
    if target.vertex_bytes.len() + frame.vertex_bytes.len()
        > MAX_FRAME_VERTICES * PAINT_VERTEX_STRIDE as usize
    {
        return Err(Error::FrameTooComplex);
    }
    for draw in &mut frame.draws {
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
            PaintCommand::DrawText { position, .. }
                if scale_milli % 1000 == 0
                    && origin.x >= 0.
                    && origin.y >= 0.
                    && position.x >= 0.
                    && position.y >= 0. => {}
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
fn vertex_bounds(bytes: &[u8]) -> FloatRect {
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
    // Text snaps in logical pixels, images in physical pixels. Include their
    // maximum rounding displacement before accepting a rounded clip's core.
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
        executor.fail_write = true;
        assert!(
            encode(
                &mut encoder,
                &mut executor,
                &paint(shape(), Point::new(20., 20.))
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
    fn fractional_scale_text_and_mixed_commands_use_precise_path() {
        let mut c = PaintContext::new();
        c.draw_text(Point::ZERO, "Hello", UiColor::BLACK, 13.);
        assert!(replay_offset(&c.into_display_list(), Point::new(10.5, 20.5), 1500).is_none());
        let mut c = PaintContext::new();
        c.draw_text(Point::ZERO, "Hello", UiColor::BLACK, 13.);
        c.draw_text(Point::new(0.5, 10.), "World", UiColor::BLACK, 13.);
        assert!(replay_offset(&c.into_display_list(), Point::new(10.5, 20.5), 1000).is_some());
    }
    #[test]
    fn fractional_scroll_replay_matches_flat_shape_text_and_image_vertices() {
        let mut encoder = encoder();
        let mut executor = Executor::default();
        let mut recording = PaintContext::new();
        recording.fill_rounded_rect(Rect::from_xywh(0., 0., 100., 40.), 4., UiColor::WHITE);
        recording.draw_text(Point::new(0.5, 12.5), "Hello", UiColor::BLACK, 13.);
        recording.draw_text(Point::new(60., 13.), "World", UiColor::BLACK, 12.);
        recording.draw_buffer_rect_shared(
            Rect::from_xywh(4.5, 2.5, 8., 8.),
            Rect::from_xywh(0., 0., 8., 8.),
            Arc::new(Buffer::new(Size::new(8., 8.))),
            1.,
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
            .lower_retained(&mut executor, &warm, 2000, area)
            .unwrap();
        let bytes = executor.buffer_data[0].clone();
        for origin in [Point::new(10.25, 20.25), Point::new(10.75, 20.75)] {
            encoder.advance_frame_serial();
            let context = paint(list.clone(), origin);
            let replay = encoder
                .lower_retained(&mut executor, &context, 2000, area)
                .unwrap();
            let flat = context.flattened();
            let reference = encoder.lower_once(&flat, 2000, area).unwrap();
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
