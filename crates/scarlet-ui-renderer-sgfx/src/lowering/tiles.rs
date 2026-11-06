//! Full-resolution retained scroll layers. Cold/changed/exposed tiles are drawn
//! on the GPU; warm scrolls only composite their textures. No CPU raster upload.
use super::*;
use scarlet_ui_core::{
    geometry::{Point, Rect},
    renderer::DisplayList,
};
use sgfx::ir::{BlendComponent, BlendFactor, BlendOp};

const SIDE: u32 = 512;
const MAX_TILES: usize = 64; // 64 MiB, recycled without growing resource slots.

pub(super) struct Tile {
    list: u64,
    source: Weak<DisplayList>,
    scale: u32,
    snap: [usize; 2],
    x: i32,
    y: i32,
    texture: TextureId,
    used: u64,
}

impl Tile {
    pub fn invalidate(&mut self) {
        self.list = 0;
    }
}

pub(super) fn pipeline(table: &ResourceTable) -> Result<RenderPipelineId> {
    let stage = |_| Error::sgfx(Stage::DefineResources);
    let layout = VertexBufferLayout::new(
        PAINT_VERTEX_STRIDE,
        alloc::vec![
            VertexAttribute::new(0, VertexFormat::Float32x4, 0),
            VertexAttribute::new(1, VertexFormat::Float32x4, 16),
            VertexAttribute::new(2, VertexFormat::Float32x2, 32),
        ],
    )
    .map_err(stage)?;
    let over = BlendComponent::new(
        BlendFactor::One,
        BlendFactor::OneMinusSourceAlpha,
        BlendOp::Add,
    );
    Ok(table
        .define_render_pipeline(
            RenderPipelineDesc::new(
                TextureFormat::Bgra8Unorm,
                PrimitiveTopology::TriangleList,
                layout,
                FragmentProgram::TextureVertexColor(TextureSampleMode::Rgba),
                BlendState::new(over, over),
                RasterState::new(sgfx::ir::CullMode::None, FrontFace::CounterClockwise),
            )
            .map_err(stage)?,
        )
        .map_err(stage)?
        .id())
}

fn eligible(list: &DisplayList) -> bool {
    list.is_self_contained()
        && list.context().commands().iter().all(|command| {
            matches!(
                command,
                PaintCommand::FillRoundedRect { .. }
                    | PaintCommand::FillVerticalGradientRoundedRect { .. }
                    | PaintCommand::StrokeRect { .. }
                    | PaintCommand::StrokeRoundedRect { .. }
                    | PaintCommand::DrawText { .. }
                    | PaintCommand::DrawIcon { .. }
                    | PaintCommand::FillPath { .. }
                    | PaintCommand::StrokePath { .. }
                    | PaintCommand::DrawRoundedRectShadow { .. }
                    | PaintCommand::BackdropFilterRoundedRect { .. }
                    | PaintCommand::DrawBuffer { .. }
                    | PaintCommand::DrawBufferRect { .. }
                    | PaintCommand::PushClip { .. }
                    | PaintCommand::PopClip
            )
        })
}

fn contains_rect(outer: PixelBounds, inner: FloatRect) -> bool {
    outer.x as f32 <= inner.x
        && outer.y as f32 <= inner.y
        && (outer.x + outer.width) as f32 >= inner.right()
        && (outer.y + outer.height) as f32 >= inner.bottom()
}

// Expand to a dependency closure, not a fixed-width edge strip: another glass
// intersecting this region may sample across its edge too. A single rectangle
// is conservative and the bounded recording makes this traversal finite.
fn live_boundary(
    list: &DisplayList,
    origin: Point,
    factor: f32,
    visible: FloatRect,
    clips: &[(Rect, f32)],
    full: PixelBounds,
) -> Option<PixelBounds> {
    let filters = list
        .context()
        .commands()
        .iter()
        .filter_map(|command| {
            let PaintCommand::BackdropFilterRoundedRect {
                rect, blur_radius, ..
            } = command
            else {
                return None;
            };
            let glass = FloatRect::new(
                (rect.left() + origin.x) * factor,
                (rect.top() + origin.y) * factor,
                rect.size.width * factor,
                rect.size.height * factor,
            );
            let r = (blur_radius.max(0.) * factor).min(64.);
            // Refraction bends inward into the material; only the blur
            // footprint reaches outside it. Match backdrop::source_bounds.
            let h = libm::ceilf(r + (r / 4.).max(1.)) + 1.;
            Some((
                glass,
                FloatRect::new(
                    glass.x - h,
                    glass.y - h,
                    glass.width + 2. * h,
                    glass.height + 2. * h,
                ),
            ))
        })
        .collect::<Vec<_>>();
    let full = FloatRect::new(
        full.x as f32,
        full.y as f32,
        full.width as f32,
        full.height as f32,
    );
    let mut live: Option<FloatRect> = None;
    let union = |a: FloatRect, b: FloatRect| {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        FloatRect::new(
            x,
            y,
            a.right().max(b.right()) - x,
            a.bottom().max(b.bottom()) - y,
        )
    };
    for (glass, capture) in &filters {
        if glass.intersect(visible).is_some()
            && (capture.x < visible.x
                || capture.y < visible.y
                || capture.right() > visible.right()
                || capture.bottom() > visible.bottom()
                || clips.iter().any(|(_, radius)| *radius > 0.))
        {
            if let Some(capture) = capture.intersect(full) {
                live = Some(live.map_or(capture, |area| union(area, capture)));
            }
        }
    }
    let mut live = live?;
    loop {
        let old = live;
        for (glass, capture) in &filters {
            if glass.intersect(live).is_some() && glass.intersect(visible).is_some() {
                if let Some(capture) = capture.intersect(full) {
                    live = union(live, capture);
                }
            }
        }
        if old == live {
            break;
        }
    }
    let x = libm::floorf(live.x) as u32;
    let y = libm::floorf(live.y) as u32;
    Some(PixelBounds {
        x,
        y,
        width: libm::ceilf(live.right()) as u32 - x,
        height: libm::ceilf(live.bottom()) as u32 - y,
    })
}

fn subtract_region(scissor: PixelBounds, live: Option<PixelBounds>) -> Vec<PixelBounds> {
    let Some(live) = live.and_then(|live| intersect_bounds(scissor, live)) else {
        return alloc::vec![scissor];
    };
    let right = scissor.x + scissor.width;
    let bottom = scissor.y + scissor.height;
    [
        PixelBounds {
            x: scissor.x,
            y: scissor.y,
            width: scissor.width,
            height: live.y - scissor.y,
        },
        PixelBounds {
            x: scissor.x,
            y: live.y + live.height,
            width: scissor.width,
            height: bottom - live.y - live.height,
        },
        PixelBounds {
            x: scissor.x,
            y: live.y,
            width: live.x - scissor.x,
            height: live.height,
        },
        PixelBounds {
            x: live.x + live.width,
            y: live.y,
            width: right - live.x - live.width,
            height: live.height,
        },
    ]
    .into_iter()
    .filter(|area| area.width > 0 && area.height > 0)
    .collect()
}

impl SgfxPaintEncoder {
    pub(super) fn lower_cached_layer<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        list: &Arc<DisplayList>,
        origin: Point,
        scale: u32,
        clips: &[(Rect, f32)],
        full: PixelBounds,
    ) -> core::result::Result<Option<LoweredFrame<'static>>, FrameError<E::Error>> {
        let Some(bounds) = list.raster_cache_bounds() else {
            return Ok(None);
        };
        // Raster text has logical-pixel snapping semantics. Do not resample it
        // at fractional offsets/DPI: retain the precise normal path there.
        if scale % 1000 != 0
            || scale == 0
            || scale > 8000
            || !origin.x.is_finite()
            || !origin.y.is_finite()
            || origin.x != libm::roundf(origin.x)
            || origin.y != libm::roundf(origin.y)
            || (!self
                .raster_tiles
                .iter()
                .any(|tile| tile.list == list.identity())
                && !eligible(list))
        {
            return Ok(None);
        }
        let factor = scale as f32 / 1000.;
        // Truncation towards zero changes phase when a fractional raster origin
        // crosses the window origin. Counts uniquely identify those monotone
        // threshold sets for an immutable list; no lossy hash or image resampling.
        let mut snap = [0; 2];
        for command in list.context().commands() {
            let (point, unit) = match command {
                PaintCommand::DrawText { position, .. } => (*position, 1.),
                PaintCommand::DrawBuffer { dst, .. } | PaintCommand::DrawBufferRect { dst, .. } => {
                    (dst.origin, factor)
                }
                PaintCommand::DrawIcon { rect, .. } => (rect.origin, factor),
                _ => continue,
            };
            for (axis, value, origin) in [(0, point.x, origin.x), (1, point.y, origin.y)] {
                if value * unit != libm::truncf(value * unit) && value + origin < 0. {
                    snap[axis] += 1;
                }
            }
        }
        let mut halo = 0_f32;
        if list.reads_backdrop() {
            for command in list.context().commands() {
                if let PaintCommand::BackdropFilterRoundedRect {
                    blur_radius,
                    refraction,
                    ..
                } = command
                {
                    let blur = (blur_radius.max(0.) * factor).min(64.);
                    halo = halo.max(
                        blur + (blur / 4.).max(1.)
                            + 2.
                            + if *refraction > 0. {
                                12. * 1.4 * factor
                            } else {
                                0.
                            },
                    );
                }
            }
        }
        let unit = scale / 1000;
        let padding = (libm::ceilf(halo / factor) as u32) * unit;
        if padding * 2 >= SIDE {
            return Ok(None);
        }
        let side = (SIDE - 2 * padding) / unit * unit;
        let offset = [origin.x * factor, origin.y * factor];
        let bounds = FloatRect::new(
            (bounds.left() + origin.x) * factor,
            (bounds.top() + origin.y) * factor,
            bounds.size.width * factor,
            bounds.size.height * factor,
        );
        let mut visible = bounds.intersect(FloatRect::new(
            0.,
            0.,
            self.width as f32,
            self.height as f32,
        ));
        for (clip, _) in clips {
            visible = visible.and_then(|v| v.intersect(FloatRect::from_logical(*clip, factor)));
        }
        let Some(visible) = visible else {
            return Ok(Some(LoweredFrame {
                vertex_bytes: Vec::new(),
                draws: Vec::new(),
                uploads: Vec::new(),
            }));
        };
        let live = live_boundary(list, origin, factor, visible, clips, full);
        if live.is_some_and(|area| contains_rect(area, visible)) {
            return Ok(None);
        }
        let x0 = libm::floorf((visible.x - offset[0]) / side as f32) as i32;
        let y0 = libm::floorf((visible.y - offset[1]) / side as f32) as i32;
        let x1 = libm::ceilf((visible.right() - offset[0]) / side as f32) as i32;
        let y1 = libm::ceilf((visible.bottom() - offset[1]) / side as f32) as i32;
        if i64::from(x1 - x0) * i64::from(y1 - y0) > MAX_TILES as i64 {
            return Ok(None);
        }
        // Reserve visible warm tiles before allocating cold exposed tiles.
        for tile in &mut self.raster_tiles {
            if tile.list == list.identity()
                && tile.scale == scale
                && tile.snap == snap
                && tile.x >= x0
                && tile.x < x1
                && tile.y >= y0
                && tile.y < y1
            {
                tile.used = self.frame_serial;
            }
        }
        let mut tessellator = Tessellator::new(
            scale,
            self.width,
            self.height,
            FloatRect::new(
                full.x as f32,
                full.y as f32,
                full.width as f32,
                full.height as f32,
            ),
        )?;
        for (clip, radius) in clips {
            tessellator.push_clip(*clip, *radius)?;
        }
        let mut draws = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let tile_rect = FloatRect::new(
                    offset[0] + x as f32 * side as f32,
                    offset[1] + y as f32 * side as f32,
                    side as f32,
                    side as f32,
                );
                if live.is_some_and(|area| {
                    tile_rect
                        .intersect(visible)
                        .is_some_and(|rect| contains_rect(area, rect))
                }) {
                    continue;
                }

                let existing = self.raster_tiles.iter().position(|tile| {
                    tile.list == list.identity()
                        && tile.scale == scale
                        && tile.snap == snap
                        && tile.x == x
                        && tile.y == y
                });
                let slot = if let Some(slot) = existing {
                    slot
                } else {
                    let candidate = self
                        .raster_tiles
                        .iter()
                        .enumerate()
                        .filter(|(_, tile)| tile.used != self.frame_serial)
                        .min_by_key(|(_, tile)| (tile.source.strong_count() != 0, tile.used))
                        .map(|(slot, _)| slot);
                    let slot = if self.raster_tiles.len() < MAX_TILES
                        && candidate.is_none_or(|i| self.raster_tiles[i].source.strong_count() != 0)
                    {
                        let stage = |_| Error::sgfx(Stage::DefineResources);
                        let texture = self
                            .table
                            .define_texture(
                                TextureDesc::new(
                                    TextureFormat::Bgra8Unorm,
                                    Extent2D::new(SIDE, SIDE).map_err(stage)?,
                                    TextureUsage::SAMPLED
                                        | TextureUsage::RENDER_ATTACHMENT
                                        | TextureUsage::COPY_SRC,
                                )
                                .map_err(stage)?,
                            )
                            .map_err(stage)?
                            .id();
                        self.raster_tiles.push(Tile {
                            list: 0,
                            source: Weak::new(),
                            scale,
                            snap,
                            x,
                            y,
                            texture,
                            used: 0,
                        });
                        self.raster_tiles.len() - 1
                    } else if let Some(slot) = candidate {
                        slot
                    } else {
                        return Ok(None);
                    };
                    // Commit identity only after successful rendering. Rejected
                    // submissions must never turn a partially drawn tile into a hit.
                    self.raster_tiles[slot].list = 0;
                    let texture = self.raster_tiles[slot].texture;
                    let flat = list.context().raster_tile_context(
                        origin,
                        Point::new(
                            (padding as f32 - x as f32 * side as f32) / factor,
                            (padding as f32 - y as f32 * side as f32) / factor,
                        ),
                        scale,
                    );
                    let dimensions = (self.width, self.height);
                    self.width = SIDE;
                    self.height = SIDE;
                    core::mem::swap(&mut self.backdrop, &mut self.tile_backdrop);
                    let budget = (
                        self.backdrop_pixels,
                        self.backdrop_filters,
                        self.backdrop_fallbacks,
                    );
                    self.backdrop_pixels = 0;
                    self.backdrop_filters = 0;
                    self.backdrop_fallbacks = 0;
                    let rendered = (|| {
                        let area = PixelBounds {
                            x: 0,
                            y: 0,
                            width: SIDE,
                            height: SIDE,
                        };
                        let frame = self.lower_once(&flat, scale, area)?;
                        self.submit(executor, texture, UiColor::TRANSPARENT, &[area], &frame)
                    })();
                    let degraded = self.backdrop_fallbacks != 0;
                    (
                        self.backdrop_pixels,
                        self.backdrop_filters,
                        self.backdrop_fallbacks,
                    ) = budget;
                    core::mem::swap(&mut self.backdrop, &mut self.tile_backdrop);
                    (self.width, self.height) = dimensions;
                    rendered?;
                    if degraded {
                        return Ok(None);
                    }
                    self.raster_tiles[slot] = Tile {
                        list: list.identity(),
                        source: Arc::downgrade(list),
                        scale,
                        snap,
                        x,
                        y,
                        texture,
                        used: self.frame_serial,
                    };
                    slot
                };
                let tile = &mut self.raster_tiles[slot];
                tile.used = self.frame_serial;
                let rect = FloatRect::new(
                    offset[0] + x as f32 * side as f32,
                    offset[1] + y as f32 * side as f32,
                    side as f32,
                    side as f32,
                );
                let lo = padding as f32 / SIDE as f32;
                let hi = (padding + side) as f32 / SIDE as f32;
                if let Some(geometry) =
                    tessellator.textured_rect(rect, [[lo, lo], [hi, lo], [hi, hi], [lo, hi]])?
                {
                    for scissor in subtract_region(geometry.scissor, live) {
                        push_draw(
                            &mut draws,
                            &mut tessellator,
                            GeometryRange {
                                scissor,
                                ..geometry
                            },
                            [1.; 4],
                            DrawSource::PremultipliedTexture(tile.texture),
                        )?;
                    }
                }
            }
        }
        let mut frame = LoweredFrame {
            vertex_bytes: encode_paint_vertices(tessellator.vertices())?,
            draws,
            uploads: Vec::new(),
        };
        if let Some(area) = live {
            // Cached output is absent throughout the dependency closure. Replay
            // the original order there, with the original viewport clips, so a
            // filter never samples cached pixels from later foreground paint.
            let mut context = PaintContext::new();
            for (clip, radius) in clips {
                context.push_rounded_clip(*clip, *radius);
            }
            context.push_clip(Rect::from_xywh(
                area.x as f32 / factor,
                area.y as f32 / factor,
                area.width as f32 / factor,
                area.height as f32 / factor,
            ));
            context.draw_display_list(origin, Arc::clone(list));
            context.pop_clip();
            for _ in clips {
                context.pop_clip();
            }
            let flat = context.flattened();
            let live_frame = self.lower_once(&flat, scale, full)?;
            retained::append_frame(&mut frame, live_frame)?;
        }
        Ok(Some(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sgfx::ir::{Command, CommandBuffer};
    #[derive(Default)]
    struct Trace {
        targets: Vec<TextureId>,
        sampled: Vec<TextureId>,
        fail: bool,
    }
    impl CommandExecutor for Trace {
        type Error = ();
        fn execute<'r, 'data>(
            &mut self,
            commands: &CommandBuffer<'r, 'data>,
        ) -> core::result::Result<(), ()> {
            if core::mem::take(&mut self.fail) {
                return Err(());
            }
            for command in commands.commands() {
                if let Command::BeginRenderPass(pass) = command {
                    self.targets.push(pass.target().id());
                }
                if let Command::SetTexture(texture) = command {
                    self.sampled.push(texture.id());
                }
                assert!(
                    !matches!(command, Command::WriteTexture { .. }),
                    "layers are GPU rendered"
                );
            }
            Ok(())
        }
    }
    fn layer(color: UiColor) -> Arc<DisplayList> {
        let mut paint = PaintContext::new();
        for i in 0..100 {
            paint.fill_rounded_rect(Rect::from_xywh(0., i as f32 * 20., 180., 12.), 3., color);
        }
        paint.into_cached_layer(Rect::from_xywh(0., 0., 180., 2000.))
    }
    fn frame(
        renderer: &mut SgfxPaintEncoder,
        trace: &mut Trace,
        list: &Arc<DisplayList>,
        y: f32,
    ) -> core::result::Result<(), FrameError<()>> {
        let mut paint = PaintContext::new();
        paint.push_rounded_clip(Rect::from_xywh(0., 0., 200., 120.), 0.);
        paint.draw_display_list(Point::new(0., y), list.clone());
        paint.pop_clip();
        renderer.encode_frame(
            trace,
            0,
            None,
            &paint,
            UiColor::WHITE,
            1000,
            &[(0, 0, 200, 120)],
        )
    }
    #[test]
    fn warm_scroll_only_composites_tiles_and_exposure_draws_only_new_tiles() {
        let mut renderer = SgfxPaintEncoder::new(200, 120, false).unwrap();
        let target = renderer.target_texture(0).unwrap();
        let mut trace = Trace::default();
        let list = layer(UiColor::RED);
        frame(&mut renderer, &mut trace, &list, 0.).unwrap();
        assert_eq!(trace.targets.iter().filter(|id| **id != target).count(), 1);
        for offset in [-1., -30., -100.] {
            trace.targets.clear();
            frame(&mut renderer, &mut trace, &list, offset).unwrap();
            assert_eq!(
                trace.targets,
                [target],
                "unchanged content must not be rasterized again"
            );
        }
        trace.targets.clear();
        frame(&mut renderer, &mut trace, &list, -450.).unwrap();
        assert_eq!(
            trace.targets.iter().filter(|id| **id != target).count(),
            1,
            "only the exposed tile is new"
        );
        trace.targets.clear();
        frame(&mut renderer, &mut trace, &list, -30.).unwrap();
        assert_eq!(trace.targets, [target], "scrolling back reuses live tiles");
        trace.targets.clear();
        frame(&mut renderer, &mut trace, &layer(UiColor::BLUE), -30.).unwrap();
        assert_eq!(
            trace.targets.iter().filter(|id| **id != target).count(),
            1,
            "changed content is repainted"
        );
    }
    #[test]
    fn long_scroll_recycles_a_bounded_tile_pool() {
        let mut renderer = SgfxPaintEncoder::new(200, 120, false).unwrap();
        let mut paint = PaintContext::new();
        paint.fill_rounded_rect(Rect::from_xywh(0., 0., 200., 100000.), 0., UiColor::WHITE);
        let list = paint.into_cached_layer(Rect::from_xywh(0., 0., 200., 100000.));
        let mut trace = Trace::default();
        for row in 0..MAX_TILES + 16 {
            frame(
                &mut renderer,
                &mut trace,
                &list,
                -(row as f32 * SIDE as f32),
            )
            .unwrap();
            assert!(renderer.raster_tiles.len() <= MAX_TILES);
        }
        assert_eq!(renderer.raster_tiles.len(), MAX_TILES);
    }

    #[test]
    fn local_glass_and_backing_are_cached_but_viewport_crossing_is_live() {
        let mut renderer = SgfxPaintEncoder::new(200, 120, false).unwrap();
        renderer.set_analytic_glass(true);
        let target = renderer.target_texture(0).unwrap();
        let mut paint = PaintContext::new();
        paint.fill_rect(Rect::from_xywh(0., -100., 200., 1200.), UiColor::WHITE);
        paint.fill_rect(Rect::from_xywh(20., 40., 100., 10.), UiColor::RED);
        paint.refractive_glass_rounded_rect(
            Rect::from_xywh(20., 30., 160., 50.),
            12.,
            2.52,
            UiColor::WHITE.with_opacity(0.3),
            1.,
        );
        let list = paint.into_cached_layer(Rect::from_xywh(0., -100., 200., 1200.));
        assert!(list.raster_cache_bounds().is_some());
        let mut trace = Trace::default();
        frame(&mut renderer, &mut trace, &list, 0.).unwrap();
        assert!(trace.targets.iter().any(|id| *id != target));
        trace.targets.clear();
        frame(&mut renderer, &mut trace, &list, -1.).unwrap();
        assert_eq!(
            trace.targets,
            [target],
            "unchanged local refraction is not recomputed"
        );
        let cached_texture = renderer
            .raster_tiles
            .iter()
            .find(|tile| tile.list == list.identity())
            .unwrap()
            .texture;
        trace.targets.clear();
        trace.sampled.clear();
        frame(&mut renderer, &mut trace, &list, -30.).unwrap();
        assert!(
            trace.targets.len() > 1,
            "outside backdrop dependency is live at the scrollport edge"
        );
        assert!(
            trace.sampled.contains(&cached_texture),
            "the unaffected interior still composites its cached texture"
        );
        assert!(
            !trace.targets.contains(&cached_texture),
            "a boundary dependency must not rerasterize the static interior"
        );
    }

    #[test]
    fn failed_tile_is_not_committed_and_restores_frame_dimensions() {
        let mut renderer = SgfxPaintEncoder::new(200, 120, false).unwrap();
        let mut trace = Trace {
            fail: true,
            ..Default::default()
        };
        let list = layer(UiColor::RED);
        assert!(frame(&mut renderer, &mut trace, &list, 0.).is_err());
        assert_eq!((renderer.width(), renderer.height()), (200, 120));
        assert!(renderer.raster_tiles.iter().all(|tile| tile.list == 0));
        frame(&mut renderer, &mut trace, &list, 0.).unwrap();
        assert!(
            renderer
                .raster_tiles
                .iter()
                .any(|tile| tile.list == list.identity())
        );
        renderer.discard_frame();
        assert!(renderer.raster_tiles.iter().all(|tile| tile.list == 0));
    }
}
