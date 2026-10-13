//! One bounded, persistent GPU coverage image for an immutable shadow stack.
//! Black coverage retains straight-alpha sampling semantics (RGB is zero).
use super::*;
use scarlet_ui_core::{geometry::Point, renderer::DisplayList};

const MAX_SHADOW_PIXELS: u64 = 16_000_000;

pub(super) struct ShadowCache {
    pub(super) list_id: u64,
    origin: Point,
    scale: u32,
    texture: TextureId,
    buffer: BufferId,
    draw: Draw,
    used_frame: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui_core::geometry::Rect;
    use sgfx::ir::{Command, CommandBuffer};

    #[derive(Default)]
    struct Trace {
        passes: Vec<TextureId>,
        textures: Vec<TextureId>,
        uploads: usize,
        fail_next: bool,
    }

    impl CommandExecutor for Trace {
        type Error = ();
        fn execute<'r, 'data>(
            &mut self,
            commands: &CommandBuffer<'r, 'data>,
        ) -> core::result::Result<(), ()> {
            if core::mem::take(&mut self.fail_next) {
                return Err(());
            }
            for command in commands.commands() {
                match command {
                    Command::BeginRenderPass(pass) => self.passes.push(pass.target().id()),
                    Command::WriteBuffer { data, .. } => self.uploads += data.len(),
                    Command::WriteTexture { .. } => panic!("shadow cache must be GPU rendered"),
                    Command::SetTexture(texture) => self.textures.push(texture.id()),
                    _ => {}
                }
            }
            Ok(())
        }
    }

    fn stack(color: UiColor) -> Arc<DisplayList> {
        let mut paint = PaintContext::new();
        for (blur, alpha) in [(10., 0.2), (4., 0.4)] {
            paint.draw_rounded_rect_shadow(
                Rect::from_xywh(10., 10., 60., 40.),
                8.,
                scarlet_ui_core::geometry::Offset::new(0., 2.),
                blur,
                0.,
                color.with_opacity(alpha),
            );
        }
        paint.into_display_list()
    }

    #[test]
    fn warm_shadow_replays_gpu_image_without_shadow_pass_or_vertex_upload() {
        for scale in [1000, 1250, 2000] {
            let mut encoder = SgfxPaintEncoder::new(200, 160, true).unwrap();
            let list = stack(UiColor::BLACK);
            let paint = || {
                let mut paint = PaintContext::new();
                paint.push_clip(Rect::from_xywh(0., 0., 80., 60.));
                paint.draw_display_list(Point::new(2.25, 3.5), Arc::clone(&list));
                paint.pop_clip();
                paint
            };
            let mut cold = Trace::default();
            encoder
                .encode_frame(
                    &mut cold,
                    0,
                    None,
                    &paint(),
                    UiColor::TRANSPARENT,
                    scale,
                    &[(0, 0, 200, 160)],
                )
                .unwrap();
            let texture = encoder.shadow_cache.as_ref().unwrap().texture;
            assert!(cold.passes.contains(&texture));
            assert!(cold.uploads > 0);
            let mut warm = Trace::default();
            encoder
                .encode_frame(
                    &mut warm,
                    1,
                    None,
                    &paint(),
                    UiColor::TRANSPARENT,
                    scale,
                    &[(0, 0, 200, 160)],
                )
                .unwrap();
            assert_eq!(warm.passes, [encoder.target_texture(1).unwrap()]);
            assert_eq!(warm.uploads, 0);
            assert_eq!(warm.textures, [texture]);
        }
    }

    #[test]
    fn clipped_shadow_recording_is_cached_and_unbalanced_state_is_rejected() {
        let shadow = stack(UiColor::BLACK);
        let mut clipped = PaintContext::new();
        for rect in [
            Rect::from_xywh(0., 0., 80., 10.),
            Rect::from_xywh(0., 60., 80., 10.),
        ] {
            clipped.push_clip(rect);
            clipped.append(shadow.context());
            clipped.pop_clip();
        }
        let list = clipped.into_display_list();
        let mut encoder = SgfxPaintEncoder::new(80, 80, true).unwrap();
        let full = PixelBounds {
            x: 0,
            y: 0,
            width: 80,
            height: 80,
        };
        let mut cold = Trace::default();
        encoder.advance_frame_serial();
        assert!(
            encoder
                .cached_shadow(&mut cold, &list, Point::ZERO, 1000, full)
                .unwrap()
                .is_some()
        );
        let mut warm = Trace::default();
        encoder.advance_frame_serial();
        assert!(
            encoder
                .cached_shadow(&mut warm, &list, Point::ZERO, 1000, full)
                .unwrap()
                .is_some()
        );
        assert!(warm.passes.is_empty());
        assert_eq!(warm.uploads, 0);
        let mut invalid = PaintContext::new();
        invalid.push_clip(Rect::from_xywh(0., 0., 80., 10.));
        invalid.append(shadow.context());
        assert!(mask_color(&invalid.into_display_list()).is_none());
    }

    #[test]
    fn shadow_identity_origin_scale_and_discard_invalidate_without_reallocating() {
        let mut encoder = SgfxPaintEncoder::new(200, 160, true).unwrap();
        let full = PixelBounds {
            x: 0,
            y: 0,
            width: 200,
            height: 160,
        };
        let first = stack(UiColor::BLACK);
        let second = stack(UiColor::BLACK);
        let mut ids = Vec::new();
        for (list, origin, scale, discard) in [
            (&first, Point::ZERO, 1000, false),
            (&second, Point::ZERO, 1000, false),
            (&second, Point::new(0.25, 0.), 1000, false),
            (&second, Point::new(0.25, 0.), 1250, false),
            (&second, Point::new(0.25, 0.), 1250, true),
        ] {
            encoder.advance_frame_serial();
            if discard {
                encoder.discard_frame();
            }
            let mut trace = Trace::default();
            encoder
                .cached_shadow(&mut trace, list, origin, scale, full)
                .unwrap()
                .unwrap();
            let cache = encoder.shadow_cache.as_ref().unwrap();
            assert_eq!(trace.passes, [cache.texture]);
            ids.push((cache.texture, cache.buffer));
        }
        assert!(ids.iter().all(|id| *id == ids[0]));
    }

    #[test]
    fn rejected_shadow_submission_is_rebuilt_using_retained_resources() {
        let mut encoder = SgfxPaintEncoder::new(200, 160, true).unwrap();
        let full = PixelBounds {
            x: 0,
            y: 0,
            width: 200,
            height: 160,
        };
        let list = stack(UiColor::BLACK);
        encoder.advance_frame_serial();
        let mut trace = Trace {
            fail_next: true,
            ..Trace::default()
        };
        assert!(
            encoder
                .cached_shadow(&mut trace, &list, Point::ZERO, 1000, full)
                .is_err()
        );
        let texture = encoder.shadow_cache.as_ref().unwrap().texture;
        assert_eq!(encoder.shadow_cache.as_ref().unwrap().list_id, 0);
        encoder.advance_frame_serial();
        encoder
            .cached_shadow(&mut trace, &list, Point::ZERO, 1000, full)
            .unwrap()
            .unwrap();
        assert_eq!(trace.passes, [texture]);
        assert_eq!(
            encoder.shadow_cache.as_ref().unwrap().list_id,
            list.identity()
        );
    }

    #[test]
    fn colored_shadow_and_multiple_recordings_never_overwrite_current_frame_cache() {
        let mut encoder = SgfxPaintEncoder::new(200, 160, true).unwrap();
        let full = PixelBounds {
            x: 0,
            y: 0,
            width: 200,
            height: 160,
        };
        encoder.advance_frame_serial();
        let mut trace = Trace::default();
        assert!(
            encoder
                .cached_shadow(&mut trace, &stack(UiColor::WHITE), Point::ZERO, 1000, full)
                .unwrap()
                .is_none()
        );
        let first = stack(UiColor::BLACK);
        encoder
            .cached_shadow(&mut trace, &first, Point::ZERO, 1000, full)
            .unwrap()
            .unwrap();
        let passes = trace.passes.len();
        assert!(
            encoder
                .cached_shadow(&mut trace, &stack(UiColor::BLACK), Point::ZERO, 1000, full)
                .unwrap()
                .is_none()
        );
        assert_eq!(trace.passes.len(), passes);
        assert_eq!(
            encoder.shadow_cache.as_ref().unwrap().list_id,
            first.identity()
        );
    }
}

// Only black shadow stacks can use the current straight-alpha texture
// pipeline without unpremultiplication. Colored shadows and other primitives
// retain the original path; their RGB/alpha semantics are never changed.
fn mask_color(list: &DisplayList) -> Option<[f32; 4]> {
    let mut depth = 0usize;
    let mut shadows = 0usize;
    for command in list.context().commands() {
        match command {
            PaintCommand::DrawRoundedRectShadow { color, .. } => {
                let color = ui_color(*color, 1.).ok()?;
                if color[..3] != [0., 0., 0.] {
                    return None;
                }
                shadows += 1;
            }
            PaintCommand::PushClip { corner_radius, .. } if *corner_radius == 0. => depth += 1,
            PaintCommand::PopClip if depth > 0 => depth -= 1,
            _ => return None,
        }
    }
    (shadows > 0 && depth == 0).then_some([1.; 4])
}

impl SgfxPaintEncoder {
    pub(super) fn cached_shadow<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        list: &Arc<DisplayList>,
        origin: Point,
        scale: u32,
        full: PixelBounds,
    ) -> core::result::Result<Option<Draw>, FrameError<E::Error>> {
        let Some(color) = mask_color(list) else {
            return Ok(None);
        };
        if !origin.x.is_finite()
            || !origin.y.is_finite()
            || u64::from(self.width) * u64::from(self.height) > MAX_SHADOW_PIXELS
        {
            return Ok(None);
        }
        if let Some(cache) = self.shadow_cache.as_mut() {
            if cache.list_id == list.identity() && cache.origin == origin && cache.scale == scale {
                cache.used_frame = self.frame_serial;
                return Ok(Some(cache.draw));
            }
            // Never overwrite a texture referenced by an earlier draw in this
            // frame. Other shadow recordings use the existing mesh path.
            if cache.used_frame == self.frame_serial {
                return Ok(None);
            }
        }

        let mut paint = PaintContext::new();
        paint.draw_display_list(origin, Arc::clone(list));
        let flat = paint.flattened();
        let shadow = self.lower_once(&flat, scale, full)?;
        let (texture, buffer) = if let Some(cache) = self.shadow_cache.as_mut() {
            cache.list_id = 0;
            (cache.texture, cache.buffer)
        } else {
            let extent = Extent2D::new(self.width, self.height).map_err(|_| Error::InvalidFrame)?;
            let texture = self
                .table
                .define_texture(
                    TextureDesc::new(
                        TextureFormat::Bgra8Unorm,
                        extent,
                        TextureUsage::RENDER_ATTACHMENT | TextureUsage::SAMPLED,
                    )
                    .map_err(|_| Error::sgfx(Stage::DefineResources))?,
                )
                .map_err(|_| Error::sgfx(Stage::DefineResources))?
                .id();
            let buffer = self
                .table
                .define_buffer(
                    BufferDesc::new(
                        6 * u64::from(PAINT_VERTEX_STRIDE),
                        BufferUsage::VERTEX | BufferUsage::COPY_DST,
                    )
                    .map_err(|_| Error::sgfx(Stage::DefineResources))?,
                )
                .map_err(|_| Error::sgfx(Stage::DefineResources))?
                .id();
            (texture, buffer)
        };
        let bounds = FloatRect::new(0., 0., self.width as f32, self.height as f32);
        let mut tessellator = Tessellator::new(1000, self.width, self.height, bounds)?;
        let geometry = tessellator
            .textured_rect(bounds, CANVAS_TARGET_TEX_COORDS)?
            .ok_or(Error::InvalidFrame)?;
        let mut draws = Vec::new();
        push_draw(
            &mut draws,
            &mut tessellator,
            geometry,
            color,
            DrawSource::Texture(texture),
        )?;
        let vertices = encode_paint_vertices(tessellator.vertices())?;
        let mut draw = draws[0];
        draw.vertex_buffer = Some(buffer);
        // Keep allocated resources even when a submission is rejected. The
        // invalid identity forces a complete redraw before the next use.
        self.shadow_cache = Some(ShadowCache {
            list_id: 0,
            origin,
            scale,
            texture,
            buffer,
            draw,
            used_frame: 0,
        });
        // Preserve original raster coordinates, including fractional origins.
        self.submit(executor, texture, UiColor::TRANSPARENT, &[full], &shadow)?;
        let mut encoder = CommandEncoder::new(&self.table);
        encoder
            .write_buffer(
                self.table
                    .buffer_ref(buffer)
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?,
                0,
                &vertices,
            )
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let commands = encoder
            .finish()
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        executor.execute(&commands).map_err(FrameError::Execution)?;
        let cache = self.shadow_cache.as_mut().unwrap();
        cache.list_id = list.identity();
        cache.used_frame = self.frame_serial;
        Ok(Some(draw))
    }
}
