//! Full-resolution retained scroll layers. Cold/changed/exposed exact-phase
//! tiles are drawn on the GPU; warm interiors only composite their textures.
//! Viewport-boundary dependencies replay in paint order. No CPU raster upload.
use super::*;
use scarlet_ui_core::{
    geometry::{Point, Rect},
    renderer::DisplayList,
};
use sgfx::ir::{BlendComponent, BlendFactor, BlendOp};

const SIDE: u32 = 512;
const BASE_TILES: usize = 64;
// Full-resolution exact-phase variants need a larger visible working set on
// Retina displays. Allocate lazily from 64..128 MiB; reuse immutable resource
// slots instead of cycling through whole phases on each subpixel scroll.
const MAX_TILES: usize = 128;
const MAX_TILE_DEPTH: usize = 8;

pub(super) struct Tile {
    list: u64,
    cache_identity: u64,
    // Snapshot of exactly the pixels in this tile; no chain to older revisions.
    // Released with this bounded LRU slot, even if its element was removed.
    recording: Option<Arc<DisplayList>>,
    scale: u32,
    padding: u32,
    snap: [usize; 2],
    snap_dependencies: Vec<(Point, f32)>,
    phase: [u32; 2],
    x: i32,
    y: i32,
    texture: TextureId,
    used: u64,
}

// Only truncating image/icon origins need a sign-dependent key. Text rounds
// on the physical grid and is invariant under integer translations, including
// when it crosses the window origin.
fn snap_dependencies(list: &DisplayList, tile: FloatRect, scale: u32) -> Vec<(Point, f32)> {
    let factor = scale as f32 / 1000.;
    let intersects = |rect: FloatRect| {
        // Include both signs of physical truncation and fractional coverage.
        FloatRect::new(rect.x - 2., rect.y - 2., rect.width + 4., rect.height + 4.)
            .intersect(tile)
            .is_some()
    };
    list.flat_context()
        .commands()
        .iter()
        .filter_map(|command| match command {
            PaintCommand::DrawIcon { rect, .. }
            | PaintCommand::DrawBuffer { dst: rect, .. }
            | PaintCommand::DrawBufferRect { dst: rect, .. } => {
                intersects(FloatRect::from_logical(*rect, 1.)).then_some((rect.origin, factor))
            }
            _ => None,
        })
        .collect()
}

fn snap_signature(dependencies: &[(Point, f32)], origin: Point, phase: [f32; 2]) -> [usize; 2] {
    let mut snap = [0; 2];
    for (point, unit) in dependencies {
        for (axis, value, origin) in [(0, point.x, origin.x), (1, point.y, origin.y)] {
            let phased = (value + phase[axis]) * unit;
            if phased != libm::truncf(phased) && value + origin < 0. {
                snap[axis] += 1;
            }
        }
    }
    snap
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
        && list.flat_context().commands().iter().all(|command| {
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
// intersecting this region may sample across its edge too. A rectangle
// per connected component avoids joining unrelated top/bottom boundaries.
// The bounded recording makes this traversal finite.
fn live_boundary(
    list: &DisplayList,
    origin: Point,
    factor: f32,
    visible: FloatRect,
    clips: &[(Rect, f32)],
    full: PixelBounds,
) -> Vec<PixelBounds> {
    let filters = list
        .flat_context()
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
            let x = libm::floorf(glass.x - h);
            let y = libm::floorf(glass.y - h);
            Some((
                // Include the optical geometry's half-pixel coverage fringe.
                FloatRect::new(
                    glass.x - 1.,
                    glass.y - 1.,
                    glass.width + 2.,
                    glass.height + 2.,
                ),
                FloatRect::new(
                    x,
                    y,
                    libm::ceilf(glass.right() + h) - x,
                    libm::ceilf(glass.bottom() + h) - y,
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
    let mut regions: Vec<FloatRect> = Vec::new();
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
                || clips.iter().any(|(clip, radius)| {
                    if *radius <= 0. {
                        return false;
                    }
                    let clip = FloatRect::from_logical(*clip, factor);
                    let r = (*radius * factor)
                        .min(clip.width * 0.5)
                        .min(clip.height * 0.5);
                    let inside = capture.x >= clip.x
                        && capture.y >= clip.y
                        && capture.right() <= clip.right()
                        && capture.bottom() <= clip.bottom();
                    let band = (capture.x >= clip.x + r && capture.right() <= clip.right() - r)
                        || (capture.y >= clip.y + r && capture.bottom() <= clip.bottom() - r);
                    !(inside && band)
                }))
        {
            if let Some(capture) = capture.intersect(full) {
                regions.push(capture);
            }
        }
    }
    loop {
        let old = regions.clone();
        for live in &mut regions {
            for (glass, capture) in &filters {
                if glass.intersect(*live).is_some() && glass.intersect(visible).is_some() {
                    if let Some(capture) = capture.intersect(full) {
                        *live = union(*live, capture);
                    }
                }
            }
        }
        let mut i = 0;
        while i < regions.len() {
            let mut j = i + 1;
            while j < regions.len() {
                if regions[i].intersect(regions[j]).is_some() {
                    let other = regions.remove(j);
                    regions[i] = union(regions[i], other);
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if old == regions {
            break;
        }
    }
    regions
        .into_iter()
        .map(|live| {
            let x = libm::floorf(live.x) as u32;
            let y = libm::floorf(live.y) as u32;
            PixelBounds {
                x,
                y,
                width: libm::ceilf(live.right()) as u32 - x,
                height: libm::ceilf(live.bottom()) as u32 - y,
            }
        })
        .collect()
}

fn subtract_region(scissor: PixelBounds, live: PixelBounds) -> Vec<PixelBounds> {
    let Some(live) = intersect_bounds(scissor, live) else {
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

fn subtract_regions(scissor: PixelBounds, regions: &[PixelBounds]) -> Vec<PixelBounds> {
    let mut remaining = alloc::vec![scissor];
    for region in regions {
        remaining = remaining
            .into_iter()
            .flat_map(|area| subtract_region(area, *region))
            .collect();
    }
    remaining
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
        // Virtual rows already live inside their scroll layer's tiles. Giving
        // every short row another row of 512px textures wastes most of each
        // texture and can evict the enclosing viewport's working set on Retina
        // displays. Replay simple rows into that layer; retain independent
        // pictures for tall subviews and expensive backdrop materials.
        if self.tile_depth > 0
            && !list.reads_backdrop()
            && bounds.size.height * scale as f32 / 1000. <= SIDE as f32 / 2.
        {
            return Ok(None);
        }
        // Bake fractional offset phases into separate tiles and composite on
        // the integer grid, preserving window-space physical text snapping.
        // Fractional DPI retains the precise normal path.
        if self.tile_depth >= MAX_TILE_DEPTH
            || scale % 1000 != 0
            || scale == 0
            || scale > 8000
            || !origin.x.is_finite()
            || !origin.y.is_finite()
            || (!self
                .raster_tiles
                .iter()
                .any(|tile| tile.list == list.identity())
                && !eligible(list))
        {
            return Ok(None);
        }
        let factor = scale as f32 / 1000.;
        let scene_origin = Point::new(
            origin.x - self.tile_translation.x,
            origin.y - self.tile_translation.y,
        );
        let base = Point::new(libm::floorf(origin.x), libm::floorf(origin.y));
        let phase_values = [origin.x - base.x, origin.y - base.y];
        let phase = phase_values.map(f32::to_bits);
        let mut halo = 0_f32;
        if list.reads_backdrop() {
            for command in list.flat_context().commands() {
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
        let offset = [base.x * factor, base.y * factor];
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
        let live = if list.reads_backdrop() {
            live_boundary(list, origin, factor, visible, clips, full)
        } else {
            Vec::new()
        };
        if live.iter().any(|area| contains_rect(*area, visible)) {
            return Ok(None);
        }
        let x0 = libm::floorf((visible.x - offset[0]) / side as f32) as i32;
        let y0 = libm::floorf((visible.y - offset[1]) / side as f32) as i32;
        let x1 = libm::ceilf((visible.right() - offset[0]) / side as f32) as i32;
        let y1 = libm::ceilf((visible.bottom() - offset[1]) / side as f32) as i32;
        let visible_tiles = i64::from(x1 - x0) * i64::from(y1 - y0);
        if visible_tiles > BASE_TILES as i64 {
            return Ok(None);
        }
        let pool_limit = (visible_tiles as usize * 4).clamp(BASE_TILES, MAX_TILES);
        // Reserve visible warm tiles before allocating cold exposed tiles.
        for tile in &mut self.raster_tiles {
            if tile.cache_identity == list.cache_identity()
                && tile.list != 0
                && tile.scale == scale
                && tile.padding == padding
                && tile.snap == snap_signature(&tile.snap_dependencies, scene_origin, phase_values)
                && tile.phase == phase
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
        let mut revisions: Vec<(u64, Option<Vec<Rect>>)> = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let tile_rect = FloatRect::new(
                    offset[0] + x as f32 * side as f32,
                    offset[1] + y as f32 * side as f32,
                    side as f32,
                    side as f32,
                );
                if live.iter().any(|area| {
                    tile_rect
                        .intersect(visible)
                        .is_some_and(|rect| contains_rect(*area, rect))
                }) {
                    continue;
                }

                let existing = self.raster_tiles.iter().position(|tile| {
                    tile.cache_identity == list.cache_identity()
                        && tile.list != 0
                        && tile.scale == scale
                        && tile.padding == padding
                        && tile.snap
                            == snap_signature(&tile.snap_dependencies, scene_origin, phase_values)
                        && tile.phase == phase
                        && tile.x == x
                        && tile.y == y
                });
                let slot = if let Some(slot) =
                    existing.filter(|slot| self.raster_tiles[*slot].list == list.identity())
                {
                    slot
                } else {
                    let candidate = self
                        .raster_tiles
                        .iter()
                        .enumerate()
                        .filter(|(_, tile)| tile.used != self.frame_serial)
                        .min_by_key(|(_, tile)| tile.used)
                        .map(|(slot, _)| slot);
                    let slot = if let Some(slot) = existing {
                        slot
                    } else if self.raster_tiles.len() < pool_limit {
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
                            cache_identity: list.cache_identity(),
                            recording: None,
                            scale,
                            padding,
                            snap: [0; 2],
                            snap_dependencies: Vec::new(),
                            phase,
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
                    let changes = if let Some(previous) = self.raster_tiles[slot]
                        .recording
                        .as_ref()
                        .filter(|_| existing.is_some())
                    {
                        if let Some((_, changes)) =
                            revisions.iter().find(|(id, _)| *id == previous.identity())
                        {
                            changes.clone()
                        } else {
                            let changes = list.damage_since(previous, scale);
                            revisions.push((previous.identity(), changes.clone()));
                            changes
                        }
                    } else {
                        None
                    };
                    // Commit identity only after successful rendering. Rejected
                    // submissions must never turn a partially drawn tile into a hit.
                    self.raster_tiles[slot].list = 0;
                    // A recursive child render cannot recycle its parent's target.
                    self.raster_tiles[slot].used = self.frame_serial;
                    let texture = self.raster_tiles[slot].texture;
                    let tile_offset = Point::new(
                        (padding as f32 - x as f32 * side as f32) / factor,
                        (padding as f32 - y as f32 * side as f32) / factor,
                    );
                    let flat = list.context().raster_tile_composite_context(
                        scene_origin,
                        tile_offset,
                        scale,
                    );
                    let nested = flat
                        .commands()
                        .iter()
                        .any(|c| matches!(c, PaintCommand::DrawDisplayList { .. }));
                    let tile_bounds =
                        Rect::from_xywh(0., 0., SIDE as f32 / factor, SIDE as f32 / factor);
                    let regions = match changes {
                        None => alloc::vec![tile_bounds],
                        Some(changes) => {
                            let translation = Point::new(
                                (padding as f32 - x as f32 * side as f32) / factor
                                    + phase_values[0],
                                (padding as f32 - y as f32 * side as f32) / factor
                                    + phase_values[1],
                            );
                            let damage = changes
                                .iter()
                                .map(|r| {
                                    Rect::new(
                                        Point::new(
                                            r.left() + translation.x,
                                            r.top() + translation.y,
                                        ),
                                        r.size,
                                    )
                                })
                                .collect::<Vec<_>>();
                            flat.reconstruction_regions(&damage, tile_bounds, scale)
                        }
                    };
                    let dimensions = (self.width, self.height);
                    self.width = SIDE;
                    self.height = SIDE;
                    let depth = self.tile_depth;
                    if self.tile_backdrops.len() == depth {
                        self.tile_backdrops.push(None);
                    }
                    core::mem::swap(&mut self.backdrop, &mut self.tile_backdrops[depth]);
                    let old_translation = self.tile_translation;
                    self.tile_translation = Point::new(
                        tile_offset.x - libm::floorf(scene_origin.x),
                        tile_offset.y - libm::floorf(scene_origin.y),
                    );
                    self.tile_depth += 1;
                    let budget = (
                        self.backdrop_pixels,
                        self.backdrop_filters,
                        self.backdrop_fallbacks,
                    );
                    self.backdrop_pixels = 0;
                    self.backdrop_filters = 0;
                    self.backdrop_fallbacks = 0;
                    let rendered = (|| {
                        for region in &regions {
                            let left = libm::floorf(region.left() * factor).max(0.) as u32;
                            let top = libm::floorf(region.top() * factor).max(0.) as u32;
                            let right = (libm::ceilf(region.right() * factor) as u32).min(SIDE);
                            let bottom = (libm::ceilf(region.bottom() * factor) as u32).min(SIDE);
                            if right <= left || bottom <= top {
                                continue;
                            }
                            let area = PixelBounds {
                                x: left,
                                y: top,
                                width: right - left,
                                height: bottom - top,
                            };
                            let frame = if nested {
                                // Keep the reconstruction scissor around child composites too.
                                let mut clipped = PaintContext::new();
                                clipped.push_clip(Rect::new(
                                    Point::new(left as f32 / factor, top as f32 / factor),
                                    scarlet_ui_core::geometry::Size::new(
                                        (right - left) as f32 / factor,
                                        (bottom - top) as f32 / factor,
                                    ),
                                ));
                                clipped.append_retained_at(&flat, Point::ZERO);
                                clipped.pop_clip();
                                super::retained::owned_frame(
                                    self.lower_retained(executor, &clipped, scale, area)?,
                                )
                            } else {
                                super::retained::owned_frame(self.lower_once(&flat, scale, area)?)
                            };
                            self.submit(executor, texture, UiColor::TRANSPARENT, &[area], &frame)?;
                        }
                        Ok::<_, FrameError<E::Error>>(())
                    })();
                    let degraded = self.backdrop_fallbacks != 0;
                    (
                        self.backdrop_pixels,
                        self.backdrop_filters,
                        self.backdrop_fallbacks,
                    ) = budget;
                    self.tile_depth = depth;
                    self.tile_translation = old_translation;
                    core::mem::swap(&mut self.backdrop, &mut self.tile_backdrops[depth]);
                    (self.width, self.height) = dimensions;
                    rendered?;
                    if degraded {
                        return Ok(None);
                    }
                    let dependencies = snap_dependencies(
                        list,
                        FloatRect::new(
                            (x as f32 * side as f32 - padding as f32) / factor,
                            (y as f32 * side as f32 - padding as f32) / factor,
                            SIDE as f32 / factor,
                            SIDE as f32 / factor,
                        ),
                        scale,
                    );
                    let snap = snap_signature(&dependencies, scene_origin, phase_values);
                    self.raster_tiles[slot] = Tile {
                        list: list.identity(),
                        cache_identity: list.cache_identity(),
                        recording: Some(Arc::clone(list)),
                        scale,
                        padding,
                        snap,
                        snap_dependencies: dependencies,
                        phase,
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
                    for scissor in subtract_regions(geometry.scissor, &live) {
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
        for area in live {
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
            let local;
            if self.tile_depth > 0 {
                local = list.context().raster_tile_context(scene_origin, base, scale);
                context.append_at(&local, Point::ZERO);
            } else {
                context.draw_display_list(origin, Arc::clone(list));
            }
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
        allow_glyph_uploads: bool,
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
                    !matches!(command, Command::WriteTexture { .. }) || self.allow_glyph_uploads,
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
    fn short_virtual_rows_share_parent_tiles_on_retina() {
        let mut renderer = SgfxPaintEncoder::new(1600, 1200, false).unwrap();
        let target = renderer.target_texture(0).unwrap();
        let mut content = PaintContext::new();
        for row in 0..40 {
            let mut paint = PaintContext::new();
            paint.fill_rounded_rect(Rect::from_xywh(0., 0., 800., 40.), 0., UiColor::WHITE);
            paint.fill_rounded_rect(Rect::from_xywh(12., 8., 250., 24.), 3., UiColor::RED);
            let row_list = paint.into_cached_layer(Rect::from_xywh(0., 0., 800., 40.));
            content.draw_display_list(Point::new(0., row as f32 * 40.), row_list);
        }
        let list = content.into_cached_layer(Rect::from_xywh(0., 0., 800., 1600.));
        let mut trace = Trace::default();
        for offset in [0., -10., -20.] {
            trace.targets.clear();
            let mut paint = PaintContext::new();
            paint.draw_display_list(Point::new(0., offset), list.clone());
            renderer
                .encode_frame(
                    &mut trace,
                    0,
                    None,
                    &paint,
                    UiColor::WHITE,
                    2000,
                    &[(0, 0, 1600, 1200)],
                )
                .unwrap();
            assert!(
                renderer
                    .raster_tiles
                    .iter()
                    .all(|tile| tile.cache_identity == list.cache_identity()),
                "short rows must not allocate independent textures inside the parent layer"
            );
            if offset == 0. {
                assert_eq!(trace.targets.iter().filter(|id| **id != target).count(), 12);
            } else {
                assert_eq!(
                    trace.targets,
                    [target],
                    "warm scrolling should only composite the parent tiles"
                );
            }
        }
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
    fn repeated_fractional_scroll_phase_reuses_tiles_without_resampling() {
        let mut renderer = SgfxPaintEncoder::new(200, 120, false).unwrap();
        let list = layer(UiColor::RED);
        let mut trace = Trace::default();
        frame(&mut renderer, &mut trace, &list, -0.5).unwrap();
        assert!(
            !renderer.raster_tiles.is_empty(),
            "a fractional offset must not bypass the layer cache"
        );
        let target = renderer.target_texture(0).unwrap();
        trace.targets.clear();
        frame(&mut renderer, &mut trace, &list, -1.5).unwrap();
        assert_eq!(
            trace.targets,
            [target],
            "a repeated exact fractional phase reuses existing GPU pixels"
        );
    }

    #[test]
    fn text_tiles_stay_warm_when_crossing_the_window_origin() {
        scarlet_ui_core::graphics::set_default_font(include_bytes!(
            "../../../../examples/web-counter/assets/DejaVuSans.ttf"
        ))
        .unwrap();
        for vertical in [false, true] {
            for long_line in [false, true] {
                let (width, height) = if vertical { (200, 800) } else { (800, 200) };
                let bounds = if vertical {
                    Rect::from_xywh(0., 0., 200., 2000.)
                } else {
                    Rect::from_xywh(0., 0., 2000., 200.)
                };
                let mut renderer = SgfxPaintEncoder::new(width, height, false).unwrap();
                let target = renderer.target_texture(0).unwrap();
                let mut paint = PaintContext::new();
                paint.fill_rect(bounds, UiColor::WHITE);
                let text = if long_line {
                    if vertical {
                        "Ink\n".repeat(70)
                    } else {
                        "Ink ".repeat(70)
                    }
                } else {
                    "Short label".into()
                };
                paint.draw_text(Point::new(8.5, 8.5), &text, UiColor::BLACK, 12.);
                let list = paint.into_cached_layer(bounds);
                let mut trace = Trace {
                    allow_glyph_uploads: true,
                    ..Trace::default()
                };
                let render = |renderer: &mut SgfxPaintEncoder, trace: &mut Trace, y| {
                    let mut frame = PaintContext::new();
                    frame.draw_display_list(
                        if vertical {
                            Point::new(0., y)
                        } else {
                            Point::new(y, 0.)
                        },
                        list.clone(),
                    );
                    renderer
                        .encode_frame(
                            trace,
                            0,
                            None,
                            &frame,
                            UiColor::WHITE,
                            1000,
                            &[(0, 0, width, height)],
                        )
                        .unwrap();
                };
                render(&mut renderer, &mut trace, -0.25);
                let interior = renderer
                    .raster_tiles
                    .iter()
                    .find(|t| if vertical { t.y == 1 } else { t.x == 1 })
                    .unwrap()
                    .texture;
                trace.targets.clear();
                trace.sampled.clear();
                render(&mut renderer, &mut trace, -20.25);
                assert_eq!(
                    trace.sampled.contains(&interior),
                    true,
                    "text snapping is translation invariant across the window origin"
                );
                assert_eq!(
                    trace.targets.iter().filter(|id| **id != target).count(),
                    0
                );
                trace.targets.clear();
                render(&mut renderer, &mut trace, -21.25);
                assert_eq!(trace.targets, [target], "unchanged snap phase stays warm");
            }
        }
    }

    #[test]
    fn explicit_blank_glyphs_do_not_invalidate_warm_tiles_when_scrolling() {
        scarlet_ui_core::graphics::set_default_font(include_bytes!(
            "../../../../examples/web-counter/assets/DejaVuSans.ttf"
        ))
        .unwrap();
        let font = scarlet_ui_core::graphics::FontStack::new(include_bytes!(
            "../../../scarlet-ui-core/src/testing/fonts/metrics-blank.ttf"
        ))
        .unwrap();
        let mut renderer = SgfxPaintEncoder::new(800, 200, false).unwrap();
        let target = renderer.target_texture(0).unwrap();
        let bounds = Rect::from_xywh(0., 0., 2000., 200.);
        let mut paint = PaintContext::new();
        paint.fill_rect(bounds, UiColor::WHITE);
        paint.draw_text_with_font_stack(
            Point::new(8.5, 8.5),
            "A ".repeat(100),
            UiColor::BLACK,
            12.,
            Arc::new(font),
        );
        let list = paint.into_cached_layer(bounds);
        let mut trace = Trace {
            allow_glyph_uploads: true,
            ..Trace::default()
        };
        for (step, x) in [-0.25, -20.25, -21.25].into_iter().enumerate() {
            let mut frame = PaintContext::new();
            frame.draw_display_list(Point::new(x, 0.), list.clone());
            trace.targets.clear();
            renderer
                .encode_frame(
                    &mut trace,
                    0,
                    None,
                    &frame,
                    UiColor::WHITE,
                    1000,
                    &[(0, 0, 800, 200)],
                )
                .unwrap();
            if step > 0 {
                assert_eq!(
                    trace.targets,
                    [target],
                    "only actual font ink constrains raster snapping"
                );
            }
        }
    }

    #[test]
    fn changing_one_record_updates_its_tile_without_rerasterizing_static_neighbors() {
        let mut renderer = SgfxPaintEncoder::new(200, 800, false).unwrap();
        let target = renderer.target_texture(0).unwrap();
        let mut background = PaintContext::new();
        background.fill_rect(Rect::from_xywh(0., 0., 200., 1600.), UiColor::WHITE);
        let background = background.into_display_list();
        let mut static_paint = PaintContext::new();
        static_paint.refractive_glass_rounded_rect(
            Rect::from_xywh(30., 600., 140., 60.),
            12.,
            2.52,
            UiColor::WHITE.with_opacity(0.3),
            1.,
        );
        let static_paint = static_paint.into_display_list();
        let make = |color, old: Option<&DisplayList>| {
            let mut changed = PaintContext::new();
            changed.fill_rect(Rect::from_xywh(30., 40., 80., 20.), color);
            let mut paint = PaintContext::new();
            paint.draw_display_list(Point::ZERO, background.clone());
            paint.draw_display_list(Point::ZERO, changed.into_display_list());
            paint.draw_display_list(Point::ZERO, static_paint.clone());
            let bounds = Rect::from_xywh(0., 0., 200., 1600.);
            if let Some(old) = old {
                paint.into_cached_layer_revision(bounds, old)
            } else {
                paint.into_cached_layer(bounds)
            }
        };
        let mut list = make(UiColor::RED, None);
        let mut trace = Trace::default();
        let render =
            |renderer: &mut SgfxPaintEncoder, trace: &mut Trace, list: Arc<DisplayList>| {
                let mut ctx = PaintContext::new();
                ctx.draw_display_list(Point::ZERO, list);
                renderer
                    .encode_frame(
                        trace,
                        0,
                        None,
                        &ctx,
                        UiColor::WHITE,
                        1000,
                        &[(0, 0, 200, 800)],
                    )
                    .unwrap();
            };
        render(&mut renderer, &mut trace, list.clone());
        let static_tiles = renderer
            .raster_tiles
            .iter()
            .filter(|t| t.y > 0)
            .map(|t| t.texture)
            .collect::<Vec<_>>();
        assert!(!static_tiles.is_empty());
        for color in [UiColor::BLUE, UiColor::BLACK, UiColor::RED] {
            trace.targets.clear();
            trace.sampled.clear();
            list = make(color, Some(&list));
            render(&mut renderer, &mut trace, list.clone());
            assert!(
                static_tiles.iter().all(|id| !trace.targets.contains(id)),
                "unrelated static tiles must never be re-rendered"
            );
            assert!(
                static_tiles.iter().all(|id| trace.sampled.contains(id)),
                "unchanged pixels are still composited"
            );
            assert_eq!(
                trace.targets.iter().filter(|id| **id != target).count(),
                1,
                "only the changed tile needs a render pass; distant glass stays cached"
            );
        }
    }

    #[test]
    fn nested_scroll_reuses_child_pixels_and_stopped_parent_pixels() {
        let mut renderer = SgfxPaintEncoder::new(200, 220, false).unwrap();
        let target = renderer.target_texture(0).unwrap();
        let mut inner = PaintContext::new();
        inner.fill_rect(Rect::from_xywh(0., 0., 180., 700.), UiColor::WHITE);
        for row in 0..60 {
            inner.fill_rect(
                Rect::from_xywh(10., row as f32 * 10., 80., 3.),
                UiColor::RED,
            );
        }
        let inner = inner.into_cached_layer(Rect::from_xywh(0., 0., 180., 700.));
        let make = |offset: f32, previous: Option<&DisplayList>| {
            let mut outer = PaintContext::new();
            outer.fill_rect(Rect::from_xywh(0., 0., 200., 800.), UiColor::WHITE);
            outer.push_clip(Rect::from_xywh(10., 20., 160., 140.));
            outer.draw_display_list(Point::new(10., 20. - offset), inner.clone());
            outer.pop_clip();
            outer.fill_rect(Rect::from_xywh(10., 180., 140., 10.), UiColor::BLUE);
            if let Some(old) = previous {
                outer.into_cached_layer_revision(Rect::from_xywh(0., 0., 200., 800.), old)
            } else {
                outer.into_cached_layer(Rect::from_xywh(0., 0., 200., 800.))
            }
        };
        let mut outer = make(0., None);
        let mut trace = Trace::default();
        frame(&mut renderer, &mut trace, &outer, 0.).unwrap();
        let child_tiles = renderer
            .raster_tiles
            .iter()
            .filter(|t| t.list == inner.identity())
            .map(|t| t.texture)
            .collect::<Vec<_>>();
        assert!(
            !child_tiles.is_empty(),
            "child picture must actually have independent raster tiles"
        );
        for offset in [4., 12., 4., 0.] {
            outer = make(offset, Some(&outer));
            trace.targets.clear();
            trace.sampled.clear();
            frame(&mut renderer, &mut trace, &outer, 0.).unwrap();
            assert!(
                child_tiles.iter().all(|t| !trace.targets.contains(t)),
                "inner motion must composite cached pixels"
            );
            assert!(child_tiles.iter().any(|t| trace.sampled.contains(t)));
        }
        trace.targets.clear();
        frame(&mut renderer, &mut trace, &outer, -4.).unwrap();
        assert_eq!(
            trace.targets,
            [target],
            "moving the outer viewport must reuse its complete static picture"
        );
        assert_eq!(renderer.tile_depth, 0);
        assert_eq!(renderer.tile_translation, Point::ZERO);
    }

    #[test]
    fn long_scroll_recycles_a_bounded_tile_pool() {
        let mut renderer = SgfxPaintEncoder::new(200, 120, false).unwrap();
        let mut paint = PaintContext::new();
        paint.fill_rounded_rect(Rect::from_xywh(0., 0., 200., 100000.), 0., UiColor::WHITE);
        let list = paint.into_cached_layer(Rect::from_xywh(0., 0., 200., 100000.));
        let mut trace = Trace::default();
        for row in 0..BASE_TILES + 16 {
            frame(
                &mut renderer,
                &mut trace,
                &list,
                -(row as f32 * SIDE as f32),
            )
            .unwrap();
            assert!(renderer.raster_tiles.len() <= BASE_TILES);
        }
        assert_eq!(renderer.raster_tiles.len(), BASE_TILES);
    }

    #[test]
    fn boundary_closure_includes_rounded_capture_pixels_and_neighbor_coverage() {
        let mut paint = PaintContext::new();
        for rect in [
            Rect::from_xywh(0.25, 20.25, 20., 20.),
            Rect::from_xywh(20., 46.5, 30., 20.),
        ] {
            paint.refractive_glass_rounded_rect(rect, 4., 2.52, UiColor::WHITE, 1.);
        }
        let list = paint.into_display_list();
        let area = live_boundary(
            &list,
            Point::ZERO,
            1.,
            FloatRect::new(0., 0., 200., 200.),
            &[],
            PixelBounds {
                x: 0,
                y: 0,
                width: 200,
                height: 200,
            },
        )[0];
        assert!(
            area.y + area.height >= 72,
            "an adjacent lens's half-pixel coverage enters the rounded live region, so its complete source footprint must be replayed too"
        );
    }

    #[test]
    fn distant_top_and_bottom_lenses_leave_the_static_middle_cached() {
        let mut paint = PaintContext::new();
        for y in [-2., 190.] {
            paint.refractive_glass_rounded_rect(
                Rect::from_xywh(20., y, 160., 16.),
                8.,
                2.52,
                UiColor::WHITE,
                1.,
            );
        }
        let list = paint.into_display_list();
        let full = PixelBounds {
            x: 0,
            y: 0,
            width: 200,
            height: 200,
        };
        let regions = live_boundary(
            &list,
            Point::ZERO,
            1.,
            FloatRect::new(0., 0., 200., 200.),
            &[],
            full,
        );
        assert_eq!(
            regions.len(),
            2,
            "independent edge dependencies must not become a full-viewport replay"
        );
        assert!(
            subtract_regions(full, &regions)
                .iter()
                .any(|area| area.y <= 80 && area.y + area.height >= 120),
            "the unchanged middle remains a cached composite"
        );
    }

    #[test]
    fn rounded_window_does_not_make_interior_glass_live() {
        let mut paint = PaintContext::new();
        paint.refractive_glass_rounded_rect(
            Rect::from_xywh(20., 30., 160., 50.),
            12.,
            2.52,
            UiColor::WHITE,
            1.,
        );
        let list = paint.into_display_list();
        let bounds = Rect::from_xywh(0., 0., 200., 200.);
        let regions = live_boundary(
            &list,
            Point::ZERO,
            1.,
            FloatRect::from_logical(bounds, 1.),
            &[(bounds, 20.)],
            PixelBounds {
                x: 0,
                y: 0,
                width: 200,
                height: 200,
            },
        );
        assert!(
            regions.is_empty(),
            "a rounded Window must not disable caching for lenses well inside its straight core"
        );
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
            .find(|tile| tile.cache_identity == list.cache_identity() && tile.list != 0)
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

    #[derive(Default)]
    struct FailingTileTrace {
        trace: Trace,
        snapshots: Vec<(TextureId, TextureId)>,
        fail_target: Option<TextureId>,
        accepted_before_failure: usize,
        rejected: Option<TextureId>,
    }

    impl CommandExecutor for FailingTileTrace {
        type Error = ();

        fn execute<'r, 'data>(
            &mut self,
            commands: &CommandBuffer<'r, 'data>,
        ) -> core::result::Result<(), ()> {
            if let Some(target) = self.fail_target {
                if commands.commands().iter().any(|command| {
                    matches!(command, Command::BeginRenderPass(pass) if pass.target().id() == target)
                }) {
                    if self.accepted_before_failure == 0 {
                        self.fail_target = None;
                        self.rejected = Some(target);
                        return Err(());
                    }
                    self.accepted_before_failure -= 1;
                }
            }
            self.trace.execute(commands)?;
            for command in commands.commands() {
                if let Command::CopyTextureToTexture {
                    source,
                    destination,
                    ..
                } = command
                {
                    self.snapshots.push((source.id(), destination.id()));
                }
            }
            Ok(())
        }
    }

    impl FailingTileTrace {
        fn snapshot_for(&self, target: TextureId) -> TextureId {
            let snapshots = self
                .snapshots
                .iter()
                .filter_map(|(source, snapshot)| (*source == target).then_some(*snapshot))
                .collect::<Vec<_>>();
            assert!(
                !snapshots.is_empty(),
                "the fixture must exercise backdrop scratch"
            );
            assert!(snapshots.iter().all(|snapshot| *snapshot == snapshots[0]));
            snapshots[0]
        }
    }

    fn check_nested_tile_failure(fail_child: bool) {
        let mut renderer = SgfxPaintEncoder::new(320, 300, false).unwrap();
        let target = renderer.target_texture(0).unwrap();
        let make = |color: UiColor, previous: Option<&(Arc<DisplayList>, Arc<DisplayList>)>| {
            let child_bounds = Rect::from_xywh(0., 0., 160., 200.);
            let mut child = PaintContext::new();
            child.fill_rect(child_bounds, UiColor::WHITE);
            // Two disjoint repairs let one submission modify the old tile before
            // the second submission fails, rather than failing a pristine tile.
            for y in [44., 124.] {
                child.fill_rect(Rect::from_xywh(32., y + 8., 64., 6.), color);
                child.refractive_glass_rounded_rect(
                    Rect::from_xywh(24., y, 88., 26.),
                    8.,
                    2.,
                    UiColor::WHITE.with_opacity(0.3),
                    1.,
                );
            }
            let child = match previous {
                Some((previous_child, _)) => {
                    child.into_cached_layer_revision(child_bounds, previous_child)
                }
                None => child.into_cached_layer(child_bounds),
            };
            let parent_bounds = Rect::from_xywh(0., 0., 240., 400.);
            let mut parent = PaintContext::new();
            parent.fill_rect(parent_bounds, UiColor::WHITE);
            parent.draw_display_list(Point::new(24., 28.), child.clone());
            parent.fill_rect(Rect::from_xywh(40., 264., 144., 6.), color);
            parent.refractive_glass_rounded_rect(
                Rect::from_xywh(40., 256., 144., 26.),
                8.,
                2.,
                UiColor::WHITE.with_opacity(0.3),
                1.,
            );
            let parent = match previous {
                Some((_, previous_parent)) => {
                    parent.into_cached_layer_revision(parent_bounds, previous_parent)
                }
                None => parent.into_cached_layer(parent_bounds),
            };
            (child, parent)
        };
        let render = |renderer: &mut SgfxPaintEncoder,
                      trace: &mut FailingTileTrace,
                      parent: &Arc<DisplayList>| {
            let mut paint = PaintContext::new();
            // Allocate distinct main-frame scratch before entering either tile.
            paint.refractive_glass_rounded_rect(
                Rect::from_xywh(256., 32., 40., 40.),
                8.,
                2.,
                UiColor::WHITE.with_opacity(0.3),
                1.,
            );
            paint.draw_display_list(Point::new(7.25, -3.5), parent.clone());
            renderer.encode_frame(
                trace,
                0,
                None,
                &paint,
                UiColor::WHITE,
                1000,
                &[(0, 0, 320, 300)],
            )
        };
        let tile_for = |renderer: &SgfxPaintEncoder, list: &DisplayList| {
            let tiles = renderer
                .raster_tiles
                .iter()
                .filter(|tile| tile.list == list.identity())
                .map(|tile| tile.texture)
                .collect::<Vec<_>>();
            assert_eq!(
                tiles.len(),
                1,
                "each fixture layer occupies exactly one tile"
            );
            tiles[0]
        };
        let original = make(UiColor::RED, None);
        let mut trace = FailingTileTrace::default();
        render(&mut renderer, &mut trace, &original.1).unwrap();
        let child_target = tile_for(&renderer, &original.0);
        let parent_target = tile_for(&renderer, &original.1);
        let scratch = [
            trace.snapshot_for(target),
            trace.snapshot_for(parent_target),
            trace.snapshot_for(child_target),
        ];
        assert_ne!(scratch[0], scratch[1]);
        assert_ne!(scratch[0], scratch[2]);
        assert_ne!(scratch[1], scratch[2]);
        let budget = (
            renderer.backdrop_pixels,
            renderer.backdrop_filters,
            renderer.backdrop_fallbacks,
        );
        assert_eq!(
            budget.1, 1,
            "nested budgets must not leak into the main frame"
        );
        let changed = make(UiColor::BLUE, Some(&original));
        let failed_target = if fail_child {
            child_target
        } else {
            parent_target
        };
        trace = FailingTileTrace {
            fail_target: Some(failed_target),
            accepted_before_failure: 1,
            ..Default::default()
        };
        assert!(matches!(
            render(&mut renderer, &mut trace, &changed.1),
            Err(FrameError::Execution(()))
        ));
        assert_eq!(trace.rejected, Some(failed_target));
        assert!(
            trace.trace.targets.contains(&failed_target),
            "a prior accepted repair must already have changed the failed tile"
        );
        assert_eq!((renderer.width(), renderer.height()), (320, 300));
        assert_eq!(renderer.tile_depth, 0);
        assert_eq!(renderer.tile_translation, Point::ZERO);
        assert_eq!(renderer.tile_backdrops.len(), 2);
        assert!(renderer.backdrop.is_some());
        assert!(renderer.tile_backdrops.iter().all(Option::is_some));
        assert_eq!(
            (
                renderer.backdrop_pixels,
                renderer.backdrop_filters,
                renderer.backdrop_fallbacks
            ),
            budget
        );
        assert_eq!(
            renderer
                .raster_tiles
                .iter()
                .find(|tile| tile.texture == parent_target)
                .unwrap()
                .list,
            0,
            "a partially reconstructed parent must never retain its old or new identity"
        );
        assert!(
            renderer
                .raster_tiles
                .iter()
                .all(|tile| tile.list != changed.1.identity())
        );
        if fail_child {
            assert_eq!(
                renderer
                    .raster_tiles
                    .iter()
                    .find(|tile| tile.texture == child_target)
                    .unwrap()
                    .list,
                0
            );
        } else {
            assert_eq!(
                tile_for(&renderer, &changed.0),
                child_target,
                "the completed child remains usable when its parent's later repair fails"
            );
        }

        // Retry without discard_frame: even before the platform's conservative
        // invalidation, no incomplete tile may be treated as a cache hit.
        trace = FailingTileTrace::default();
        render(&mut renderer, &mut trace, &changed.1).unwrap();
        let repaired_parent = tile_for(&renderer, &changed.1);
        assert!(trace.trace.targets.contains(&repaired_parent));
        tile_for(&renderer, &changed.0);

        // Force both depths to execute again and observe their snapshot texture
        // IDs, not just Some/None: a misplaced scratch image can otherwise pass
        // dimension/depth checks while retaining the wrong resource ownership.
        let next = make(UiColor::RED, Some(&changed));
        trace = FailingTileTrace::default();
        render(&mut renderer, &mut trace, &next.1).unwrap();
        assert_eq!(trace.snapshot_for(target), scratch[0]);
        assert_eq!(trace.snapshot_for(tile_for(&renderer, &next.1)), scratch[1]);
        assert_eq!(trace.snapshot_for(tile_for(&renderer, &next.0)), scratch[2]);
        assert_eq!((renderer.width(), renderer.height()), (320, 300));
        assert_eq!(renderer.tile_depth, 0);
        assert_eq!(renderer.tile_translation, Point::ZERO);
    }

    #[test]
    fn nested_child_render_failure_restores_state_and_retries_partial_tiles() {
        check_nested_tile_failure(true);
    }

    #[test]
    fn nested_parent_render_failure_restores_state_and_retries_partial_tiles() {
        check_nested_tile_failure(false);
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
