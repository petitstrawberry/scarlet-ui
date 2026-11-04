//! Portable, destination-dependent glass material. Only fixed SGFX shaders are
//! used, including on WebGPU. Every filter is an ordered render-pass barrier.
//!
//! One full-target snapshot is retained, but only the expanded source rectangle
//! is copied. The two downsampled working images use 16..256 power-of-two size
//! classes: at most 50 images / 1.88 MiB across all classes. A frame copies at
//! most 8 Mpixels and each filter shades at most 3 * 256 * 256 working pixels.
//! Budget exhaustion or insufficient working resolution uses an explicit
//! opaque-tint fallback, never a fake or silently over-broad blur.
use super::*;
use scarlet_ui_core::geometry::Rect;
use sgfx::ir::{BlendComponent, BlendFactor, BlendOp};
mod analytic;

const MAX_SNAPSHOT_PIXELS: u64 = 16_777_216;
const MAX_FRAME_COPY_PIXELS: u64 = 8_388_608;
const MAX_FRAME_FILTERS: u32 = 64;
const MAX_PHYSICAL_RADIUS: f32 = 64.;
const WEIGHTS: [f32; 9] = [1., 8., 28., 56., 70., 56., 28., 8., 1.];
const QUAD_VERTICES: u32 = 6;
const WORK_VERTICES: u32 = QUAD_VERTICES * 19;

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Backdrop {
    bounds: PixelBounds,
    radius: f32,
    opacity: f32,
    tint: [f32; 4],
    // None uses the full-resolution snapshot and one reusable blur scratch.
    working: Option<usize>,
    optics: Option<[f32; 8]>,
}
struct WorkingImages {
    width: u32,
    height: u32,
    first: TextureId,
    second: TextureId,
}
pub(super) struct Resources {
    snapshot: TextureId,
    vertex_buffer: BufferId,
    replace: RenderPipelineId,
    additive_texture: RenderPipelineId,
    additive_solid: RenderPipelineId,
    erase: RenderPipelineId,
    working: Vec<WorkingImages>,
    full_resolution_work: Option<TextureId>,
    analytic: Option<analytic::Resources>,
}

pub(super) fn contains_backdrop(paint: &PaintContext<'_>) -> bool {
    paint.commands().iter().any(|command| match command {
        PaintCommand::BackdropFilterRoundedRect { .. } => true,
        PaintCommand::DrawDisplayList { list, .. } => list.reads_backdrop(),
        _ => false,
    })
}

impl SgfxPaintEncoder {
    /// Enable per-pixel glass on backends supporting SGFX WGSL pipelines.
    /// Portable fixed-pipeline backends keep the mesh implementation by default.
    pub fn set_analytic_glass(&mut self, enabled: bool) {
        self.analytic_glass = enabled;
    }
    /// Number of materials rendered as opaque tint because the bounded blur
    /// budget or minimum working resolution was exceeded in the latest frame.
    pub const fn backdrop_fallback_count(&self) -> u32 {
        self.backdrop_fallbacks
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_backdrop(
        &mut self,
        tessellator: &mut Tessellator,
        draws: &mut Vec<Draw>,
        rect: Rect,
        corner_radius: f32,
        blur_radius: f32,
        tint: UiColor,
        refraction: f32,
        opacity: f32,
        scale: f32,
    ) -> Result<()> {
        if !blur_radius.is_finite() || !corner_radius.is_finite() || !refraction.is_finite() {
            return Err(Error::InvalidFrame);
        }
        let color = ui_color(tint, 1.)?;
        let opacity = opacity.clamp(0., 1.);
        if opacity <= 0. {
            return Ok(());
        }
        let physical = FloatRect::from_logical(rect, scale);
        if physical.is_empty() {
            return Ok(());
        }
        // Scrolling can leave whole materials outside an ancestor's viewport.
        // Reject those before building optical geometry or requesting scratch images.
        let Some(visible) = tessellator.effective_scissor() else {
            return Ok(());
        };
        if physical
            .intersect(FloatRect::new(
                visible.x as f32,
                visible.y as f32,
                visible.width as f32,
                visible.height as f32,
            ))
            .is_none()
        {
            return Ok(());
        }
        let radius = (blur_radius.max(0.) * scale).min(MAX_PHYSICAL_RADIUS);
        let Some(bounds) = source_bounds(physical, radius, self.width, self.height) else {
            return Ok(());
        };
        if radius <= 0. {
            if let Some(geometry) = tessellator.fill_rounded_rect(rect, corner_radius)? {
                return push_draw(
                    draws,
                    tessellator,
                    geometry,
                    ui_color(tint, opacity)?,
                    DrawSource::Solid,
                );
            }
            return Ok(());
        }
        let pixels = u64::from(bounds.width) * u64::from(bounds.height);
        if u64::from(self.width) * u64::from(self.height) > MAX_SNAPSHOT_PIXELS
            || self.backdrop_pixels.saturating_add(pixels) > MAX_FRAME_COPY_PIXELS
            || self.backdrop_filters >= MAX_FRAME_FILTERS
            || (refraction <= 0.0 && !adequate_resolution(bounds, radius))
        {
            if let Some(geometry) = tessellator.fill_rounded_rect(rect, corner_radius)? {
                self.backdrop_fallbacks = self.backdrop_fallbacks.saturating_add(1);
                return push_draw(
                    draws,
                    tessellator,
                    geometry,
                    [color[0], color[1], color[2], opacity],
                    DrawSource::Solid,
                );
            }
            return Ok(());
        }
        // Optical detail is optional under load. Keep room for clipping and
        // foreground geometry; degraded glass still uses the ordinary blur.
        let full_resolution = refraction > 0.0;
        let analytic = self.analytic_glass && full_resolution;
        let refraction = if analytic
            || (self.backdrop_filters < 16
                && tessellator.vertices().len() + 16_384 < crate::geometry::MAX_FRAME_VERTICES)
        {
            refraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        // The material and ancestor clips affect only the output, never the halo.
        if !analytic {
            tessellator.push_clip(rect, corner_radius)?;
        }
        let sampling = if full_resolution {
            FloatRect::new(0., 0., self.width as f32, self.height as f32)
        } else {
            FloatRect::new(
                bounds.x as f32,
                bounds.y as f32,
                bounds.width as f32,
                bounds.height as f32,
            )
        };
        let u0 = (physical.x - sampling.x) / sampling.width;
        let v0 = (physical.y - sampling.y) / sampling.height;
        let u1 = (physical.right() - sampling.x) / sampling.width;
        let v1 = (physical.bottom() - sampling.y) / sampling.height;
        let geometry = if analytic {
            tessellator.textured_rect(physical, [[u0, v0], [u1, v0], [u1, v1], [u0, v1]])
        } else if refraction > 0.0 {
            tessellator.refractive_rect(rect, corner_radius, sampling, refraction.clamp(0.0, 1.0))
        } else {
            tessellator.textured_rect(physical, [[u0, v0], [u1, v0], [u1, v1], [u0, v1]])
        };
        if !analytic {
            tessellator.pop_clip();
        }
        let Some(geometry) = geometry? else {
            return Ok(());
        };
        let working = self.prepare_backdrop(bounds, radius, full_resolution)?;
        self.backdrop_pixels += pixels;
        self.backdrop_filters += 1;
        // Even a material at the start of the scene must observe the background
        // clear. SGFX does not admit an empty render pass.
        if draws.is_empty() {
            let dummy = tessellator.dummy_draw()?;
            push_draw(draws, tessellator, dummy, [0.; 4], DrawSource::Solid)?;
        }
        push_draw(
            draws,
            tessellator,
            geometry,
            [1.; 4],
            DrawSource::Backdrop(Backdrop {
                bounds,
                radius,
                opacity,
                tint: color,
                working,
                optics: analytic.then_some([
                    physical.x,
                    physical.y,
                    physical.width,
                    physical.height,
                    corner_radius * scale,
                    scale,
                    refraction,
                    0.,
                ]),
            }),
        )?;
        if refraction > 0.0 && !analytic {
            tessellator.push_clip(rect, corner_radius)?;
            let rim =
                tessellator.glass_rim(rect, corner_radius, opacity * refraction.clamp(0.0, 1.0));
            tessellator.pop_clip();
            if let Some(geometry) = rim? {
                draws.try_reserve(1).map_err(|_| Error::FrameTooComplex)?;
                draws.push(Draw {
                    geometry,
                    source: DrawSource::Solid,
                    vertex_buffer: None,
                    offset: [0.0, 0.0],
                    snap_phase: [0.0, 0.0],
                });
            }
        }
        Ok(())
    }

    fn prepare_backdrop(
        &mut self,
        bounds: PixelBounds,
        radius: f32,
        full_resolution: bool,
    ) -> Result<Option<usize>> {
        if self.backdrop.is_none() {
            let table = &self.table;
            let component = BlendComponent::new(BlendFactor::One, BlendFactor::One, BlendOp::Add);
            let additive = BlendState::new(component, component);
            let component = BlendComponent::new(
                BlendFactor::Zero,
                BlendFactor::OneMinusSourceAlpha,
                BlendOp::Add,
            );
            let erase = BlendState::new(component, component);
            self.backdrop = Some(Resources {
                snapshot: texture(
                    table,
                    self.width,
                    self.height,
                    TextureUsage::COPY_DST
                        | TextureUsage::SAMPLED
                        | TextureUsage::RENDER_ATTACHMENT,
                )?,
                vertex_buffer: table
                    .define_buffer(
                        BufferDesc::new(
                            u64::from(WORK_VERTICES * PAINT_VERTEX_STRIDE),
                            BufferUsage::VERTEX | BufferUsage::COPY_DST,
                        )
                        .map_err(|_| Error::sgfx(Stage::DefineResources))?,
                    )
                    .map_err(|_| Error::sgfx(Stage::DefineResources))?
                    .id(),
                replace: pipeline(table, true, BlendState::REPLACE)?,
                additive_texture: pipeline(table, true, additive)?,
                additive_solid: pipeline(table, false, additive)?,
                erase: pipeline(table, false, erase)?,
                working: Vec::new(),
                full_resolution_work: None,
                analytic: None,
            });
        }
        let (width, height) = working_extent(bounds, radius);
        let resources = self.backdrop.as_mut().ok_or(Error::InvalidFrame)?;
        if full_resolution {
            if resources.full_resolution_work.is_none() {
                resources.full_resolution_work = Some(texture(
                    &self.table,
                    self.width,
                    self.height,
                    TextureUsage::SAMPLED | TextureUsage::RENDER_ATTACHMENT,
                )?);
            }
            if self.analytic_glass && resources.analytic.is_none() {
                resources.analytic = Some(analytic::Resources::new(
                    &self.table,
                    resources.snapshot,
                    resources.full_resolution_work.unwrap(),
                    self.sampler,
                )?);
            }
            return Ok(None);
        }
        if let Some(index) = resources
            .working
            .iter()
            .position(|images| images.width == width && images.height == height)
        {
            return Ok(Some(index));
        }
        let usage = TextureUsage::SAMPLED | TextureUsage::RENDER_ATTACHMENT;
        resources
            .working
            .try_reserve(1)
            .map_err(|_| Error::FrameTooComplex)?;
        resources.working.push(WorkingImages {
            width,
            height,
            first: texture(&self.table, width, height, usage)?,
            second: texture(&self.table, width, height, usage)?,
        });
        Ok(Some(resources.working.len() - 1))
    }

    pub(super) fn backdrop_work_bytes(&self, filter: Backdrop) -> Result<Vec<u8>> {
        if filter.optics.is_some() {
            return Ok(analytic::parameters(filter, self.width, self.height));
        }
        let resources = self.backdrop.as_ref().ok_or(Error::InvalidFrame)?;
        let working = filter.working.map(|index| &resources.working[index]);
        encode_paint_vertices(&work_vertices(filter, working, self.width, self.height))
    }

    /// Append barriers and passes in-order without a queue submission per glass.
    /// Buffer uploads are ordered copies, so the small work buffer is reusable.
    pub(super) fn encode_backdrop<'r, 'data>(
        &self,
        encoder: &mut CommandEncoder<'r, 'data>,
        table: &'r ResourceTable,
        bytes: &'data [u8],
        target: TextureId,
        draw: Draw,
        filter: Backdrop,
    ) -> Result<()> {
        let resources = self.backdrop.as_ref().ok_or(Error::InvalidFrame)?;
        if filter.optics.is_some() {
            return resources
                .analytic
                .as_ref()
                .ok_or(Error::InvalidFrame)?
                .encode(
                    encoder,
                    table,
                    bytes,
                    resources.snapshot,
                    resources.full_resolution_work.unwrap(),
                    target,
                    self.vertex_buffer,
                    draw,
                    filter,
                    self.width,
                    self.height,
                );
        }
        let working = filter.working.map(|index| &resources.working[index]);
        let stage = |_| Error::sgfx(Stage::EncodeCommands);
        let target = table.texture_ref(target).map_err(stage)?;
        let snapshot = table.texture_ref(resources.snapshot).map_err(stage)?;
        let first = match working {
            Some(working) => table.texture_ref(working.first).map_err(stage)?,
            None => snapshot,
        };
        let second = table
            .texture_ref(match working {
                Some(working) => working.second,
                None => resources.full_resolution_work.ok_or(Error::InvalidFrame)?,
            })
            .map_err(stage)?;
        let buffer = table.buffer_ref(resources.vertex_buffer).map_err(stage)?;
        let sampler = table.sampler_ref(self.sampler).map_err(stage)?;
        let (work_width, work_height, area) = match working {
            Some(working) => (
                working.width,
                working.height,
                PixelRect::new(0, 0, working.width, working.height).map_err(stage)?,
            ),
            None => (self.width, self.height, pixel_rect(filter.bounds)?),
        };
        let copy = pixel_rect(filter.bounds)?;
        encoder
            .copy_texture_to_texture(target, copy, snapshot, copy)
            .map_err(stage)?;
        encoder.write_buffer(buffer, 0, bytes).map_err(stage)?;
        let transparent = ir_color([0.; 4])?;
        let white = ir_color([1.; 4])?;
        for (source, destination, pipeline, first_vertex, vertex_count) in [
            (snapshot, first, resources.replace, 0, 6),
            (first, second, resources.additive_texture, 6, 54),
            (second, first, resources.additive_texture, 60, 54),
        ] {
            // Refractive glass keeps source pixels at their original resolution.
            // Blur directly between snapshot and scratch; no downsample pass.
            if working.is_none() && first_vertex == 0 {
                continue;
            }
            let descriptor = RenderPassDesc::new(
                table,
                destination,
                area,
                LoadOp::Clear(transparent),
                StoreOp::Store,
            )
            .map_err(stage)?;
            let mut pass = encoder.begin_render_pass(descriptor).map_err(stage)?;
            pass.set_pipeline(table.render_pipeline_ref(pipeline).map_err(stage)?)
                .map_err(stage)?;
            pass.set_vertex_buffer(buffer, 0).map_err(stage)?;
            pass.set_texture(source).map_err(stage)?;
            pass.set_sampler(sampler).map_err(stage)?;
            pass.set_uniforms(DrawUniforms::new(
                pixel_transform(work_width, work_height)?,
                white,
            ))
            .map_err(stage)?;
            pass.draw(vertex_count, first_vertex).map_err(stage)?;
            pass.end().map_err(stage)?;
        }
        let descriptor = RenderPassDesc::new(
            table,
            target,
            PixelRect::new(0, 0, self.width, self.height).map_err(stage)?,
            LoadOp::Load,
            StoreOp::Store,
        )
        .map_err(stage)?;
        let mut pass = encoder.begin_render_pass(descriptor).map_err(stage)?;
        pass.set_vertex_buffer(table.buffer_ref(self.vertex_buffer).map_err(stage)?, 0)
            .map_err(stage)?;
        pass.set_scissor(Some(pixel_rect(draw.geometry.scissor)?))
            .map_err(stage)?;
        let transform = pixel_transform(self.width, self.height)?;
        // Replace the affected destination by its blurred/tinted value, weighted
        // by opacity. Samples are already premultiplied, so never multiply their
        // RGB by sampled alpha again. This also handles transparent backdrops.
        for (pipeline, color, textured) in [
            (resources.erase, [0., 0., 0., filter.opacity], false),
            (
                resources.additive_texture,
                [filter.opacity * (1. - filter.tint[3]); 4],
                true,
            ),
            (
                resources.additive_solid,
                [
                    filter.tint[0] * filter.tint[3] * filter.opacity,
                    filter.tint[1] * filter.tint[3] * filter.opacity,
                    filter.tint[2] * filter.tint[3] * filter.opacity,
                    filter.tint[3] * filter.opacity,
                ],
                false,
            ),
        ] {
            pass.set_pipeline(table.render_pipeline_ref(pipeline).map_err(stage)?)
                .map_err(stage)?;
            if textured {
                pass.set_texture(first).map_err(stage)?;
                pass.set_sampler(sampler).map_err(stage)?;
            }
            pass.set_uniforms(DrawUniforms::new(transform, ir_color(color)?))
                .map_err(stage)?;
            pass.draw(draw.geometry.vertex_count, draw.geometry.first_vertex)
                .map_err(stage)?;
        }
        pass.end().map_err(stage)?;
        Ok(())
    }
}

fn source_bounds(rect: FloatRect, radius: f32, width: u32, height: u32) -> Option<PixelBounds> {
    // Both downsampling and reconstruction consume source pixels. Keep this
    // footprint in addition to the kernel support so stale snapshot padding is
    // never sampled by the material's output geometry.
    let halo = libm::ceilf(radius + permitted_reduction(radius)) + 1.;
    let x = libm::floorf(rect.x - halo).max(0.).min(width as f32) as u32;
    let y = libm::floorf(rect.y - halo).max(0.).min(height as f32) as u32;
    let right = libm::ceilf(rect.right() + halo).max(0.).min(width as f32) as u32;
    let bottom = libm::ceilf(rect.bottom() + halo).max(0.).min(height as f32) as u32;
    (right > x && bottom > y).then_some(PixelBounds {
        x,
        y,
        width: right.saturating_sub(x),
        height: bottom.saturating_sub(y),
    })
}
fn permitted_reduction(radius: f32) -> f32 {
    (radius / 4.).max(1.)
}
fn adequate_resolution(bounds: PixelBounds, radius: f32) -> bool {
    let maximum = 256. * permitted_reduction(radius);
    bounds.width as f32 <= maximum && bounds.height as f32 <= maximum
}

fn working_extent(bounds: PixelBounds, radius: f32) -> (u32, u32) {
    let reduction = permitted_reduction(radius);
    let size = |value| {
        (libm::ceilf(value as f32 / reduction) as u32)
            .clamp(16, 256)
            .next_power_of_two()
    };
    (size(bounds.width), size(bounds.height))
}
fn pixel_rect(bounds: PixelBounds) -> Result<PixelRect> {
    PixelRect::new(bounds.x, bounds.y, bounds.width, bounds.height)
        .map_err(|_| Error::sgfx(Stage::EncodeCommands))
}
fn texture(
    table: &ResourceTable,
    width: u32,
    height: u32,
    usage: TextureUsage,
) -> Result<TextureId> {
    let extent = Extent2D::new(width, height).map_err(|_| Error::sgfx(Stage::DefineResources))?;
    table
        .define_texture(
            TextureDesc::new(TextureFormat::Bgra8Unorm, extent, usage)
                .map_err(|_| Error::sgfx(Stage::DefineResources))?,
        )
        .map(|texture| texture.id())
        .map_err(|_| Error::sgfx(Stage::DefineResources))
}
fn pipeline(table: &ResourceTable, textured: bool, blend: BlendState) -> Result<RenderPipelineId> {
    let mut attributes = alloc::vec![
        VertexAttribute::new(0, VertexFormat::Float32x4, 0),
        VertexAttribute::new(1, VertexFormat::Float32x4, 16)
    ];
    if textured {
        attributes.push(VertexAttribute::new(2, VertexFormat::Float32x2, 32));
    }
    let layout = VertexBufferLayout::new(PAINT_VERTEX_STRIDE, attributes)
        .map_err(|_| Error::sgfx(Stage::DefineResources))?;
    let descriptor = RenderPipelineDesc::new(
        TextureFormat::Bgra8Unorm,
        PrimitiveTopology::TriangleList,
        layout,
        if textured {
            FragmentProgram::TextureVertexColor(TextureSampleMode::Rgba)
        } else {
            FragmentProgram::VertexColor
        },
        blend,
        RasterState::new(sgfx::ir::CullMode::None, FrontFace::CounterClockwise),
    )
    .map_err(|_| Error::sgfx(Stage::DefineResources))?;
    table
        .define_render_pipeline(descriptor)
        .map(|pipeline| pipeline.id())
        .map_err(|_| Error::sgfx(Stage::DefineResources))
}
fn work_vertices(
    filter: Backdrop,
    working: Option<&WorkingImages>,
    width: u32,
    height: u32,
) -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(WORK_VERTICES as usize);
    let b = filter.bounds;
    let mut quad = |uv: [[f32; 2]; 4], weight| {
        let (x, y, w, h) = match working {
            Some(working) => (0., 0., working.width as f32, working.height as f32),
            None => (b.x as f32, b.y as f32, b.width as f32, b.height as f32),
        };
        let positions = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        for index in [0, 1, 2, 0, 2, 3] {
            vertices.push(Vertex {
                position: positions[index],
                color: [weight; 4],
                tex_coord: uv[index],
            });
        }
    };
    let (u0, v0, u1, v1) = (
        b.x as f32 / width as f32,
        b.y as f32 / height as f32,
        (b.x + b.width) as f32 / width as f32,
        (b.y + b.height) as f32 / height as f32,
    );
    quad([[u0, v0], [u1, v0], [u1, v1], [u0, v1]], 1.);
    let (left, top, right, bottom, sample_width, sample_height) = if working.is_some() {
        (0., 0., 1., 1., b.width, b.height)
    } else {
        (u0, v0, u1, v1, width, height)
    };
    for axis in 0..2 {
        for (index, weight) in WEIGHTS.into_iter().enumerate() {
            let shift = (index as f32 - 4.) * filter.radius
                / 4.
                / if axis == 0 {
                    sample_width as f32
                } else {
                    sample_height as f32
                };
            let (u, v) = if axis == 0 { (shift, 0.) } else { (0., shift) };
            quad(
                [
                    [left + u, top + v],
                    [right + u, top + v],
                    [right + u, bottom + v],
                    [left + u, bottom + v],
                ],
                weight / 256.,
            );
        }
    }
    vertices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lowering::tests::RecordingExecutor;
    use scarlet_ui_core::geometry::Point;

    #[test]
    fn optical_mesh_is_bounded_and_degrades_to_blur_after_sixteen_filters() {
        let mut encoder = SgfxPaintEncoder::new(200, 120, false).unwrap();
        let mut paint = PaintContext::new();
        for _ in 0..20 {
            paint.refractive_glass_rounded_rect(
                Rect::from_xywh(20., 20., 80., 40.),
                12.,
                12.,
                UiColor::rgba(1., 1., 1., 0.2),
                1.,
            );
        }
        let frame = encoder
            .lower(
                &paint,
                1000,
                PixelBounds {
                    x: 0,
                    y: 0,
                    width: 200,
                    height: 120,
                },
            )
            .unwrap();
        let glass: Vec<_> = frame
            .draws
            .iter()
            .filter(|d| matches!(d.source, DrawSource::Backdrop(_)))
            .collect();
        assert_eq!(glass.len(), 20);
        assert!(
            glass[..16]
                .iter()
                .all(|d| d.geometry.vertex_count > 6 && d.geometry.vertex_count < 8192)
        );
        assert!(glass[16..].iter().all(|d| d.geometry.vertex_count < 1000));
        assert_eq!(
            encoder.backdrop_fallback_count(),
            0,
            "optical cap still preserves blur"
        );
    }

    #[test]
    fn backdrop_is_an_ordered_barrier_and_forces_full_replay() {
        let mut renderer = SgfxPaintEncoder::new(200, 120, false).unwrap();
        let mut paint = PaintContext::new();
        paint.fill_rounded_rect(Rect::from_xywh(0., 0., 200., 120.), 0., UiColor::WHITE);
        paint.backdrop_filter_rounded_rect(
            Rect::from_xywh(20., 20., 80., 40.),
            8.,
            12.,
            UiColor::rgba(1., 1., 1., 0.2),
        );
        paint.fill_rounded_rect(Rect::from_xywh(40., 30., 2., 20.), 0., UiColor::BLACK);
        let mut executor = RecordingExecutor::default();
        renderer
            .encode_frame(
                &mut executor,
                0,
                None,
                &paint,
                UiColor::BLACK,
                1000,
                &[(21, 21, 1, 1)],
            )
            .unwrap();
        let kinds: Vec<_> = executor.command_kinds.iter().flatten().copied().collect();
        let copy = kinds.iter().position(|kind| *kind == "copy").unwrap();
        assert!(kinds[..copy].contains(&"end-pass"));
        assert!(
            kinds[copy..]
                .iter()
                .filter(|kind| **kind == "begin-pass")
                .count()
                >= 5
        );
        assert_eq!(
            executor.render_areas[0],
            PixelRect::new(0, 0, 200, 120).unwrap()
        );
        assert!(!kinds.contains(&"set-programmable-pipeline"));
        assert_eq!(renderer.backdrop_fallback_count(), 0);
    }

    #[test]
    fn multiple_glasses_share_one_submission_and_cull_outside_scrollport() {
        let mut renderer = SgfxPaintEncoder::new(200, 120, false).unwrap();
        let mut paint = PaintContext::new();
        paint.fill_rounded_rect(Rect::from_xywh(0., 0., 200., 120.), 0., UiColor::WHITE);
        paint.push_rounded_clip(Rect::from_xywh(0., 0., 100., 120.), 0.);
        for i in 0..12 {
            paint.backdrop_filter_rounded_rect(
                Rect::from_xywh(10., 10. + i as f32 * 5., 60., 30.),
                8.,
                8.,
                UiColor::WHITE.with_opacity(0.2),
            );
        }
        for _ in 0..64 {
            paint.refractive_glass_rounded_rect(
                Rect::from_xywh(110., 20., 80., 40.),
                12.,
                8.,
                UiColor::WHITE,
                1.,
            );
        }
        paint.pop_clip();
        let mut executor = RecordingExecutor::default();
        renderer
            .encode_frame(
                &mut executor,
                0,
                None,
                &paint,
                UiColor::BLACK,
                1000,
                &[(0, 0, 200, 120)],
            )
            .unwrap();
        assert_eq!(
            executor.command_kinds.len(),
            1,
            "no per-material queue submission"
        );
        assert_eq!(
            executor.command_kinds[0]
                .iter()
                .filter(|k| **k == "copy")
                .count(),
            12
        );
        assert_eq!(
            renderer.backdrop_filters, 12,
            "invisible materials consume no filter budget"
        );
        assert_eq!(renderer.backdrop_fallback_count(), 0);
    }

    #[test]
    fn nested_material_is_never_an_isolated_retained_mesh() {
        let mut local = PaintContext::new();
        local.backdrop_filter_rounded_rect(
            Rect::from_xywh(0., 0., 80., 40.),
            8.,
            12.,
            UiColor::WHITE,
        );
        let mut paint = PaintContext::new();
        paint.draw_display_list(Point::new(20., 20.), local.into_display_list());
        let mut renderer = SgfxPaintEncoder::new(200, 120, false).unwrap();
        let mut executor = RecordingExecutor::default();
        for _ in 0..2 {
            renderer
                .encode_frame(
                    &mut executor,
                    0,
                    None,
                    &paint,
                    UiColor::BLACK,
                    1000,
                    &[(0, 0, 200, 120)],
                )
                .unwrap();
        }
        assert!(renderer.retained_meshes.is_empty());
        assert_eq!(renderer.backdrop.as_ref().unwrap().working.len(), 1);
        assert_eq!(
            executor
                .command_kinds
                .iter()
                .flatten()
                .filter(|kind| **kind == "copy")
                .count(),
            2
        );
    }

    #[test]
    fn scratch_classes_and_copy_work_are_bounded() {
        for width in [1, 17, 200, 2_000, 20_000] {
            for radius in [0., 1., 12., 64.] {
                let (w, h) = working_extent(
                    PixelBounds {
                        x: 0,
                        y: 0,
                        width,
                        height: width,
                    },
                    radius,
                );
                assert!((16..=256).contains(&w) && (16..=256).contains(&h));
                assert!(w.is_power_of_two() && h.is_power_of_two());
            }
        }
        let mut renderer = SgfxPaintEncoder::new(5000, 5000, false).unwrap();
        let mut paint = PaintContext::new();
        paint.backdrop_filter_rounded_rect(
            Rect::from_xywh(20., 20., 80., 40.),
            8.,
            12.,
            UiColor::rgba(0.8, 0.8, 0.8, 0.2),
        );
        let frame = renderer
            .lower(
                &paint,
                1000,
                PixelBounds {
                    x: 0,
                    y: 0,
                    width: 5000,
                    height: 5000,
                },
            )
            .unwrap();
        assert!(
            frame
                .draws
                .iter()
                .all(|draw| !matches!(draw.source, DrawSource::Backdrop(_)))
        );
        assert_eq!(renderer.backdrop_fallback_count(), 1);
        assert!(renderer.backdrop.is_none());

        // A small blur on a large surface must not silently become a broad,
        // aliased blur because of the working-image size cap.
        let mut renderer = SgfxPaintEncoder::new(4000, 100, false).unwrap();
        let mut paint = PaintContext::new();
        paint.backdrop_filter_rounded_rect(
            Rect::from_xywh(0., 0., 4000., 100.),
            8.,
            1.,
            UiColor::WHITE,
        );
        let frame = renderer
            .lower(
                &paint,
                1000,
                PixelBounds {
                    x: 0,
                    y: 0,
                    width: 4000,
                    height: 100,
                },
            )
            .unwrap();
        assert!(
            frame
                .draws
                .iter()
                .all(|draw| !matches!(draw.source, DrawSource::Backdrop(_)))
        );
        assert_eq!(renderer.backdrop_fallback_count(), 1);
        assert!(renderer.backdrop.is_none());
    }

    #[test]
    fn blur_weights_preserve_premultiplied_channels_and_scale() {
        let filter = Backdrop {
            bounds: PixelBounds {
                x: 10,
                y: 10,
                width: 100,
                height: 50,
            },
            radius: 12.,
            opacity: 1.,
            tint: [0.; 4],
            working: Some(0),
            optics: None,
        };
        assert_eq!(WEIGHTS.iter().sum::<f32>(), 256.);
        let mut renderer = SgfxPaintEncoder::new(200, 100, false).unwrap();
        renderer
            .prepare_backdrop(filter.bounds, filter.radius, false)
            .unwrap();
        let resources = renderer.backdrop.as_ref().unwrap();
        let vertices = work_vertices(filter, Some(&resources.working[0]), 200, 100);
        assert_eq!(vertices.len(), WORK_VERTICES as usize);
        for vertex in &vertices[6..] {
            assert_eq!(vertex.color, [vertex.color[0]; 4]);
        }
        assert_eq!(
            source_bounds(FloatRect::new(20., 20., 40., 20.), 12., 200, 100).unwrap(),
            PixelBounds {
                x: 4,
                y: 4,
                width: 72,
                height: 52
            }
        );
    }
}
