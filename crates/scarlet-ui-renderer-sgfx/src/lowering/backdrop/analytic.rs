//! Native per-pixel optics. Full-resolution separable filtering and analytic
//! normals avoid displacement-grid seams without adding mesh subdivisions.
use super::*;
use crate::{Error, Result};
use sgfx::ir::*;

const PARAMETER_BYTES: usize = 80 + MAX_BLUR_TAPS * 16;

// Work backwards from the final texture samples. Refraction points inward and
// is capped at 1.4 * min(bezel, 12 * scale); bezel <= 0.35 * min(rect.size),
// so a refracted sample stays inside the material rectangle. An ancestor clip
// can restrict the samples further, but must retain that displacement margin.
// Keep a texel for bilinear reconstruction, then the separable kernel support.
fn filter_regions(filter: Backdrop, scissor: PixelBounds) -> Option<[PixelBounds; 3]> {
    let o = filter.optics?;
    let material = FloatRect::new(o[0], o[1], o[2], o[3]);
    let rect =
        |b: PixelBounds| FloatRect::new(b.x as f32, b.y as f32, b.width as f32, b.height as f32);
    let expand = |r: FloatRect, x: f32, y: f32| {
        FloatRect::new(r.x - x, r.y - y, r.width + 2. * x, r.height + 2. * y)
    };
    let capture = rect(filter.bounds);
    let round = |r: FloatRect| -> Option<PixelBounds> {
        let r = r.intersect(capture)?;
        let x = libm::floorf(r.x) as u32;
        let y = libm::floorf(r.y) as u32;
        Some(PixelBounds {
            x,
            y,
            width: libm::ceilf(r.right()) as u32 - x,
            height: libm::ceilf(r.bottom()) as u32 - y,
        })
    };
    let visible = material.intersect(rect(scissor))?;
    let shift = (12. * o[5]).min(0.35 * o[2].min(o[3])) * 1.4 * o[6];
    let samples = expand(visible, shift + 1., shift + 1.).intersect(expand(material, 1., 1.))?;
    let vertical = round(samples)?;
    let support = libm::ceilf(filter.radius) + 1.;
    let horizontal = round(expand(rect(vertical), 1., support))?;
    let copy = round(expand(rect(horizontal), support, 0.))?;
    Some([copy, horizontal, vertical])
}

pub(super) struct Resources {
    uniform: BufferId,
    snapshot_group: BindGroupId,
    work_group: BindGroupId,
    horizontal: ProgrammableRenderPipelineId,
    vertical: ProgrammableRenderPipelineId,
    erase: ProgrammableRenderPipelineId,
    glass: ProgrammableRenderPipelineId,
}

impl Resources {
    pub fn new(
        table: &ResourceTable,
        snapshot: TextureId,
        work: TextureId,
        sampler: SamplerId,
    ) -> Result<Self> {
        let stage = |_| Error::sgfx(Stage::DefineResources);
        let layout = BindGroupLayoutDesc::new(alloc::vec![
            BindGroupLayoutEntry::new(
                0,
                ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                BindingType::UniformBuffer
            ),
            BindGroupLayoutEntry::new(1, ShaderStages::FRAGMENT, BindingType::SampledTexture),
            BindGroupLayoutEntry::new(2, ShaderStages::FRAGMENT, BindingType::Sampler),
        ])
        .map_err(stage)?;
        let uniform = table
            .define_buffer(
                BufferDesc::new(
                    PARAMETER_BYTES as u64,
                    BufferUsage::UNIFORM | BufferUsage::COPY_DST,
                )
                .map_err(stage)?,
            )
            .map_err(stage)?
            .id();
        let group = |texture| -> Result<BindGroupId> {
            Ok(table
                .define_bind_group(
                    BindGroupDesc::new(
                        table,
                        layout.clone(),
                        alloc::vec![
                            BindGroupEntry::new(
                                0,
                                BindingResource::Buffer {
                                    buffer: uniform,
                                    offset: 0,
                                    size: PARAMETER_BYTES as u64
                                }
                            ),
                            BindGroupEntry::new(1, BindingResource::Texture(texture)),
                            BindGroupEntry::new(2, BindingResource::Sampler(sampler)),
                        ],
                    )
                    .map_err(stage)?,
                )
                .map_err(stage)?
                .id())
        };
        let shader = table
            .define_shader_module(
                ShaderModuleDesc::wgsl(include_str!("glass.wgsl").into()).map_err(stage)?,
            )
            .map_err(stage)?;
        let pipeline = |vertex: &str,
                        fragment: &str,
                        blend,
                        geometry|
         -> Result<ProgrammableRenderPipelineId> {
            let vertices = if geometry {
                Some(
                    VertexBufferLayout::new(
                        PAINT_VERTEX_STRIDE,
                        alloc::vec![VertexAttribute::new(0, VertexFormat::Float32x4, 0)],
                    )
                    .map_err(stage)?,
                )
            } else {
                None
            };
            Ok(table
                .define_programmable_render_pipeline(
                    ProgrammableRenderPipelineDesc::new(
                        ShaderEntryPoint::new(shader, ShaderStage::Vertex, vertex.into())
                            .map_err(stage)?,
                        ShaderEntryPoint::new(shader, ShaderStage::Fragment, fragment.into())
                            .map_err(stage)?,
                        PipelineLayoutDesc::new(alloc::vec![layout.clone()]).map_err(stage)?,
                        TextureFormat::Bgra8Unorm,
                        vertices,
                        PrimitiveTopology::TriangleList,
                        blend,
                        RasterState::new(CullMode::None, FrontFace::CounterClockwise),
                    )
                    .map_err(stage)?,
                )
                .map_err(stage)?
                .id())
        };
        let add = BlendComponent::new(BlendFactor::One, BlendFactor::One, BlendOp::Add);
        let erase = BlendComponent::new(
            BlendFactor::Zero,
            BlendFactor::OneMinusSourceAlpha,
            BlendOp::Add,
        );
        Ok(Self {
            uniform,
            snapshot_group: group(snapshot)?,
            work_group: group(work)?,
            horizontal: pipeline("blur_vertex", "horizontal", BlendState::REPLACE, false)?,
            vertical: pipeline("blur_vertex", "vertical", BlendState::REPLACE, false)?,
            erase: pipeline("glass_vertex", "erase", BlendState::new(erase, erase), true)?,
            glass: pipeline("glass_vertex", "glass", BlendState::new(add, add), true)?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode<'r, 'data>(
        &self,
        encoder: &mut CommandEncoder<'r, 'data>,
        table: &'r ResourceTable,
        bytes: &'data [u8],
        snapshot: TextureId,
        work: TextureId,
        target: TextureId,
        vertices: BufferId,
        draw: Draw,
        filter: Backdrop,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let stage = |_| Error::sgfx(Stage::EncodeCommands);
        let target = table.texture_ref(target).map_err(stage)?;
        let snapshot = table.texture_ref(snapshot).map_err(stage)?;
        let work = table.texture_ref(work).map_err(stage)?;
        let Some([copy, horizontal, vertical]) = filter_regions(filter, draw.geometry.scissor)
        else {
            return Ok(());
        };
        let copy = pixel_rect(copy)?;
        encoder
            .copy_texture_to_texture(target, copy, snapshot, copy)
            .map_err(stage)?;
        encoder
            .write_buffer(table.buffer_ref(self.uniform).map_err(stage)?, 0, bytes)
            .map_err(stage)?;
        for (destination, group, pipeline, scissor) in [
            (work, self.snapshot_group, self.horizontal, horizontal),
            (snapshot, self.work_group, self.vertical, vertical),
        ] {
            // Keep the original pass coordinates and procedural quad. Only the
            // samples needed by the next pass are shaded; untouched scratch
            // pixels are never consumed. Ordered uploads/passes retain the same
            // resource ownership across materials and in-flight submissions.
            let desc = RenderPassDesc::new(
                table,
                destination,
                pixel_rect(filter.bounds)?,
                LoadOp::Load,
                StoreOp::Store,
            )
            .map_err(stage)?;
            let mut pass = encoder.begin_render_pass(desc).map_err(stage)?;
            pass.set_programmable_pipeline(
                table
                    .programmable_render_pipeline_ref(pipeline)
                    .map_err(stage)?,
            )
            .map_err(stage)?;
            pass.set_bind_group(0, table.bind_group_ref(group).map_err(stage)?)
                .map_err(stage)?;
            pass.set_scissor(Some(pixel_rect(scissor)?))
                .map_err(stage)?;
            pass.draw(6, 0).map_err(stage)?;
            pass.end().map_err(stage)?;
        }
        let desc = RenderPassDesc::new(
            table,
            target,
            PixelRect::new(0, 0, width, height).map_err(stage)?,
            LoadOp::Load,
            StoreOp::Store,
        )
        .map_err(stage)?;
        let mut pass = encoder.begin_render_pass(desc).map_err(stage)?;
        pass.set_vertex_buffer(table.buffer_ref(vertices).map_err(stage)?, 0)
            .map_err(stage)?;
        pass.set_scissor(Some(pixel_rect(draw.geometry.scissor)?))
            .map_err(stage)?;
        for pipeline in [self.erase, self.glass] {
            pass.set_programmable_pipeline(
                table
                    .programmable_render_pipeline_ref(pipeline)
                    .map_err(stage)?,
            )
            .map_err(stage)?;
            pass.set_bind_group(0, table.bind_group_ref(self.snapshot_group).map_err(stage)?)
                .map_err(stage)?;
            pass.draw(draw.geometry.vertex_count, draw.geometry.first_vertex)
                .map_err(stage)?;
        }
        pass.end().map_err(stage)?;
        Ok(())
    }
}

pub(super) fn parameters(filter: Backdrop, width: u32, height: u32) -> Vec<u8> {
    let o = filter.optics.unwrap();
    let b = filter.bounds;
    let kernel = blur_kernel(filter.radius);
    let values = [
        o[0],
        o[1],
        o[2],
        o[3],
        width as f32,
        height as f32,
        o[5],
        o[4],
        filter.tint[0],
        filter.tint[1],
        filter.tint[2],
        filter.tint[3],
        filter.radius,
        o[6],
        filter.opacity,
        kernel.len() as f32,
        b.x as f32,
        b.y as f32,
        b.width as f32,
        b.height as f32,
    ];
    let mut bytes: Vec<u8> = Vec::with_capacity(PARAMETER_BYTES);
    bytes.extend(values.into_iter().flat_map(f32::to_le_bytes));
    for (offset, weight) in kernel {
        bytes.extend(
            [offset, weight, 0., 0.]
                .into_iter()
                .flat_map(f32::to_le_bytes),
        );
    }
    bytes.resize(PARAMETER_BYTES, 0);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui_core::{
        geometry::Point,
        glass::{glass_edge, glass_sample},
    };

    fn bounds(x: u32, y: u32, width: u32, height: u32) -> PixelBounds {
        PixelBounds {
            x,
            y,
            width,
            height,
        }
    }

    fn filter(rect: FloatRect, radius: f32, scale: f32, width: u32, height: u32) -> Backdrop {
        Backdrop {
            bounds: source_bounds(rect, radius, width, height).unwrap(),
            radius,
            opacity: 1.,
            tint: [0.; 4],
            working: None,
            optics: Some([
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                12. * scale,
                scale,
                1.,
                0.,
            ]),
        }
    }

    fn pixels(b: PixelBounds, mut visit: impl FnMut(u32, u32)) {
        for y in b.y..b.y + b.height {
            for x in b.x..b.x + b.width {
                visit(x, y);
            }
        }
    }

    fn sample(image: &[f32], width: u32, height: u32, x: f32, y: f32) -> f32 {
        let ix = libm::floorf(x - 0.5);
        let iy = libm::floorf(y - 0.5);
        let fx = x - 0.5 - ix;
        let fy = y - 0.5 - iy;
        let at = |x: f32, y: f32| {
            image[(y.clamp(0., (height - 1) as f32) as u32 * width
                + x.clamp(0., (width - 1) as f32) as u32) as usize]
        };
        let top = at(ix, iy) * (1. - fx) + at(ix + 1., iy) * fx;
        let bottom = at(ix, iy + 1.) * (1. - fx) + at(ix + 1., iy + 1.) * fx;
        top * (1. - fy) + bottom * fy
    }

    #[test]
    fn cropped_filter_matches_full_capture_with_stale_scratch_across_four_frames() {
        let (width, height) = (160, 120);
        let screen = bounds(0, 0, width, height);
        for (rect, scale, clip) in [
            (FloatRect::new(30.25, 35.75, 80.5, 42.5), 1., screen),
            (FloatRect::new(-12.5, -8.25, 92., 60.), 2., screen),
            (FloatRect::new(115.25, 90.5, 60., 50.), 1.5, screen),
            (
                FloatRect::new(12.25, 18.5, 130., 90.),
                3.,
                bounds(50, 35, 12, 9),
            ),
            (FloatRect::new(60.25, 40.5, 4.5, 3.25), 1., screen),
        ] {
            for radius in [0.5, 4., 12.5, 32., 64.] {
                let filter = filter(rect, radius, scale, width, height);
                let [copy, horizontal, vertical] = filter_regions(filter, clip).unwrap();
                let kernel = blur_kernel(radius);
                let size = (width * height) as usize;
                let mut snapshot = alloc::vec![-1000.; size];
                let mut work = alloc::vec![1000.; size];
                let mut full_snapshot = snapshot.clone();
                let mut full_work = work.clone();
                let blur = |source: &[f32], destination: &mut [f32], area, axis: [f32; 2]| {
                    pixels(area, |x, y| {
                        destination[(y * width + x) as usize] = kernel
                            .iter()
                            .map(|(offset, weight)| {
                                sample(
                                    source,
                                    width,
                                    height,
                                    x as f32 + 0.5 + axis[0] * offset,
                                    y as f32 + 0.5 + axis[1] * offset,
                                ) * weight
                            })
                            .sum();
                    });
                };
                // Retain deliberately stale scratch between changing scenes,
                // just as ordered submissions reuse the same two textures.
                for frame in 0..4 {
                    let source: Vec<_> = (0..size)
                        .map(|i| ((i * 17 + frame * 53) % 251) as f32 / 251.)
                        .collect();
                    pixels(filter.bounds, |x, y| {
                        let i = (y * width + x) as usize;
                        full_snapshot[i] = source[i];
                    });
                    pixels(copy, |x, y| {
                        let i = (y * width + x) as usize;
                        snapshot[i] = source[i];
                    });
                    blur(&full_snapshot, &mut full_work, filter.bounds, [1., 0.]);
                    blur(&full_work, &mut full_snapshot, filter.bounds, [0., 1.]);
                    blur(&snapshot, &mut work, horizontal, [1., 0.]);
                    blur(&work, &mut snapshot, vertical, [0., 1.]);
                    let logical = Rect::from_xywh(
                        rect.x / scale,
                        rect.y / scale,
                        rect.width / scale,
                        rect.height / scale,
                    );
                    pixels(clip, |x, y| {
                        let p = Point::new((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale);
                        if x as f32 + 0.5 < rect.x
                            || x as f32 + 0.5 >= rect.right()
                            || y as f32 + 0.5 < rect.y
                            || y as f32 + 0.5 >= rect.bottom()
                        {
                            return;
                        }
                        let distance = glass_edge(logical, 12., p).0;
                        if distance * scale <= -0.5 {
                            return;
                        }
                        // WGSL refracts only nonnegative inward distances.
                        let p = if distance >= 0. {
                            glass_sample(logical, 12., p, 1.)
                        } else {
                            p
                        };
                        let actual = sample(&snapshot, width, height, p.x * scale, p.y * scale);
                        let expected =
                            sample(&full_snapshot, width, height, p.x * scale, p.y * scale);
                        assert_eq!(
                            actual, expected,
                            "frame={frame}, radius={radius}, scale={scale}, pixel=({x},{y})"
                        );
                    });
                }
            }
        }
    }

    #[test]
    fn regions_remove_unused_halo_and_cull_empty_clips() {
        let filter = filter(FloatRect::new(100., 100., 600., 400.), 32., 1., 1000, 800);
        let area = |b: PixelBounds| u64::from(b.width) * u64::from(b.height);
        let [copy, horizontal, vertical] = filter_regions(filter, bounds(0, 0, 1000, 800)).unwrap();
        assert_eq!(horizontal, bounds(98, 66, 604, 468));
        assert_eq!(vertical, bounds(99, 99, 602, 402));
        assert!(area(copy) < area(filter.bounds));
        assert!((area(horizontal) + area(vertical)) * 100 < area(filter.bounds) * 2 * 81);
        let clipped = filter_regions(filter, bounds(200, 200, 20, 20)).unwrap();
        assert!(area(clipped[0]) * 10 < area(copy));
        assert!(filter_regions(filter, bounds(0, 0, 10, 10)).is_none());
    }

    #[test]
    fn encoding_preserves_order_and_shared_resources_across_four_targets() {
        #[derive(Default)]
        struct Audit {
            frames: Vec<(TextureId, TextureId, BufferId)>,
        }
        impl CommandExecutor for Audit {
            type Error = ();
            fn execute<'r, 'data>(
                &mut self,
                commands: &CommandBuffer<'r, 'data>,
            ) -> core::result::Result<(), ()> {
                let expected = filter(FloatRect::new(100., 100., 600., 400.), 32., 1., 1000, 800);
                let [copy, horizontal, vertical] =
                    filter_regions(expected, bounds(0, 0, 1000, 800)).unwrap();
                let mut copied = None;
                let mut uniform = None;
                let mut scissors = Vec::new();
                let mut programmable_draws = 0;
                for command in commands.commands() {
                    match command {
                        Command::CopyTextureToTexture {
                            source,
                            source_rect,
                            destination,
                            destination_rect,
                        } => {
                            assert!(copied.is_none(), "one snapshot copy per material");
                            assert_eq!(*source_rect, pixel_rect(copy).unwrap());
                            assert_eq!(source_rect, destination_rect);
                            copied = Some((source.id(), destination.id()));
                        }
                        Command::WriteBuffer { buffer, data, .. }
                            if data.len() == PARAMETER_BYTES =>
                        {
                            assert!(copied.is_some(), "uniform write follows snapshot copy");
                            uniform = Some(buffer.id());
                        }
                        Command::SetScissor(Some(scissor)) if copied.is_some() => {
                            scissors.push(*scissor)
                        }
                        Command::SetProgrammablePipeline(_) => {
                            assert!(uniform.is_some(), "upload precedes each dependent pass");
                            programmable_draws += 1;
                        }
                        _ => {}
                    }
                }
                assert_eq!(
                    programmable_draws, 4,
                    "H/V and erase/glass stay ordered in one submission"
                );
                assert_eq!(
                    &scissors[..2],
                    &[
                        pixel_rect(horizontal).unwrap(),
                        pixel_rect(vertical).unwrap()
                    ]
                );
                let (target, snapshot) = copied.unwrap();
                self.frames.push((target, snapshot, uniform.unwrap()));
                Ok(())
            }
        }
        let mut renderer = SgfxPaintEncoder::with_target_count(1000, 800, false, 4).unwrap();
        renderer.set_analytic_glass(true);
        let mut paint = PaintContext::new();
        paint.refractive_glass_rounded_rect(
            Rect::from_xywh(100., 100., 600., 400.),
            12.,
            32.,
            UiColor::WHITE,
            1.,
        );
        let mut audit = Audit::default();
        for target in 0..4 {
            renderer
                .encode_frame(
                    &mut audit,
                    target,
                    None,
                    &paint,
                    UiColor::BLACK,
                    1000,
                    &[(0, 0, 1000, 800)],
                )
                .unwrap();
        }
        assert_eq!(audit.frames.len(), 4);
        for frame in &audit.frames[1..] {
            assert_ne!(frame.0, audit.frames[0].0);
            assert_eq!(frame.1, audit.frames[0].1, "snapshot identity stays stable");
            assert_eq!(frame.2, audit.frames[0].2, "UBO identity stays stable");
        }
    }
}
