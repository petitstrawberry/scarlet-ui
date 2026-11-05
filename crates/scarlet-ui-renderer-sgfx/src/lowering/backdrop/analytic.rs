//! Native per-pixel optics. Full-resolution separable filtering and analytic
//! normals avoid displacement-grid seams without adding mesh subdivisions.
use super::*;
use crate::{Error, Result};
use sgfx::ir::*;

const PARAMETER_BYTES: usize = 80 + MAX_BLUR_TAPS * 16;

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
        let copy = pixel_rect(filter.bounds)?;
        encoder
            .copy_texture_to_texture(target, copy, snapshot, copy)
            .map_err(stage)?;
        encoder
            .write_buffer(table.buffer_ref(self.uniform).map_err(stage)?, 0, bytes)
            .map_err(stage)?;
        for (destination, group, pipeline) in [
            (work, self.snapshot_group, self.horizontal),
            (snapshot, self.work_group, self.vertical),
        ] {
            // All pixels in the capture are overwritten; no attachment clear or
            // scaling pass is needed. The halo protects the visible output.
            let desc = RenderPassDesc::new(table, destination, copy, LoadOp::Load, StoreOp::Store)
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
