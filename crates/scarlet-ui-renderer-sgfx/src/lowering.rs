//! Paint-command to persistent SGFX IR lowering.

use alloc::rc::Rc;
use alloc::sync::Arc;
use alloc::vec::Vec;

use scarlet_ui_core::buffer::Buffer;
use scarlet_ui_core::color::Color as UiColor;
use scarlet_ui_core::graphics::{GlyphRasterKey, rasterize_text};
use scarlet_ui_core::renderer::{BufferHandle, PaintCommand, PaintContext};
use sgfx::ir::{
    AddressMode, BlendState, BufferDesc, BufferId, BufferUsage, Color, CommandEncoder,
    DrawUniforms, Extent2D, FilterMode, FragmentProgram, FrontFace, LoadOp, PixelRect,
    PrimitiveTopology, RasterState, RenderPassDesc, RenderPipelineDesc, RenderPipelineId,
    ResourceTable, SamplerDesc, SamplerId, StoreOp, TextureDesc, TextureFormat, TextureId,
    TextureSampleMode, TextureUsage, TextureWrite, Transform, VertexAttribute,
    VertexBufferLayout, VertexFormat,
};
use sgfx::{Context, Image, IrResources, Queue};

use crate::canvas::{SgfxCanvasFrame, SgfxCanvasPaint, SgfxMesh, SgfxTexture};
use crate::error::{Error, Result, Stage};
use crate::geometry::{
    FloatRect, GeometryRange, MAX_FRAME_VERTICES, PixelBounds, Tessellator, Vertex,
};

const VERTEX_STRIDE: u32 = 16;
// VirGL accepts a 64 KiB opaque stream. Canonical vertices are 32 bytes, while
// draw state is about 200 bytes for solid draws and at most 260 bytes for a
// textured draw that also initializes a new sampler view. Pack against that
// byte budget instead of imposing an unrelated fixed draw limit.
const MAX_PASS_VERTICES: u32 = 1_920;
const MAX_OPAQUE_COMMAND_BYTES: u32 = 65_536;
const PASS_FIXED_COMMAND_BYTES: u32 = 2_112;
const CANONICAL_VERTEX_BYTES: u32 = 32;
const SOLID_DRAW_COMMAND_BYTES: u32 = 200;
const TEXTURED_DRAW_COMMAND_BYTES: u32 = 260;
const GLYPH_ATLAS_SIZE: u32 = 2_048;
const GLYPH_ATLAS_PADDING: u32 = 1;
const MAX_GLYPH_ENTRIES: usize = 512;
const MAX_BUFFER_TEXTURES: usize = 128;
const CANVAS_VERTEX_STRIDE: u32 = 40;
const MAX_CANVASES: usize = 32;
const MAX_CANVAS_MESHES: usize = 256;
const MAX_CANVAS_TEXTURES: usize = 128;
const MAX_CANVAS_DRAWS: usize = 240;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawSource {
    Solid,
    Texture(TextureId),
    Glyph(TextureId),
}

#[derive(Clone, Copy)]
struct Draw {
    geometry: GeometryRange,
    color: [f32; 4],
    source: DrawSource,
}

enum UploadBytes<'frame> {
    Borrowed(&'frame [u8]),
    Shared(Arc<[u8]>),
}

impl UploadBytes<'_> {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}

struct TextureUpload<'frame> {
    texture: TextureId,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    bytes_per_row: u32,
    bytes: UploadBytes<'frame>,
}

struct LoweredFrame<'frame> {
    vertex_bytes: Vec<u8>,
    draws: Vec<Draw>,
    uploads: Vec<TextureUpload<'frame>>,
}

struct BufferTexture {
    texture: TextureId,
    buffer_identity: u64,
    revision: u64,
    width: u32,
    height: u32,
    used_frame: u64,
}

struct GlyphTexture {
    key: GlyphRasterKey,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct GlyphAtlas {
    texture: TextureId,
    entries: Vec<GlyphTexture>,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

struct CanvasTarget {
    handle_id: u64,
    texture: TextureId,
    width: u32,
    height: u32,
    revision: u64,
    initialized: bool,
}

struct CanvasMesh {
    mesh_id: u64,
    buffer: BufferId,
    vertex_count: u32,
    uploaded: bool,
}

struct CanvasTexture {
    texture_id: u64,
    texture: TextureId,
    source: Arc<SgfxTexture>,
    uploaded: bool,
}

/// Persistent resources for one two-image allocation generation.
pub(crate) struct RenderSession {
    table: Rc<ResourceTable>,
    cache: IrResources,
    images: Vec<Rc<Image>>,
    targets: Vec<TextureId>,
    vertex_buffer: BufferId,
    solid_pipeline: RenderPipelineId,
    texture_pipeline: RenderPipelineId,
    glyph_pipeline: RenderPipelineId,
    sampler: SamplerId,
    buffer_textures: Vec<BufferTexture>,
    glyph_atlas: GlyphAtlas,
    canvas_pipeline: RenderPipelineId,
    canvas_texture_pipeline: RenderPipelineId,
    canvas_dummy_buffer: BufferId,
    canvas_targets: Vec<CanvasTarget>,
    canvas_meshes: Vec<CanvasMesh>,
    canvas_textures: Vec<CanvasTexture>,
    frame_serial: u64,
    width: u32,
    height: u32,
}

impl RenderSession {
    pub(crate) fn new(context: &Context, width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidFrame);
        }
        let table = Rc::new(ResourceTable::new());
        let extent = Extent2D::new(width, height)
            .map_err(|_| Error::sgfx(Stage::DefineResources))?;
        let target_usage = TextureUsage::RENDER_ATTACHMENT
            | TextureUsage::COPY_SRC
            | TextureUsage::COPY_DST
            | TextureUsage::PRESENT;

        let mut targets = Vec::new();
        let mut images = Vec::new();
        targets
            .try_reserve_exact(2)
            .map_err(|_| Error::FrameTooComplex)?;
        images
            .try_reserve_exact(2)
            .map_err(|_| Error::FrameTooComplex)?;
        for _ in 0..2 {
            let target = table
                .define_texture(
                    TextureDesc::new(TextureFormat::Bgra8Unorm, extent, target_usage)
                        .map_err(|_| Error::sgfx(Stage::DefineResources))?,
                )
                .map_err(|_| Error::sgfx(Stage::DefineResources))?
                .id();
            let image = Rc::new(
                context
                    .create_shared_image(width, height)
                    .map_err(|_| Error::sgfx(Stage::CreateSharedImage))?,
            );
            targets.push(target);
            images.push(image);
        }

        let vertex_bytes = u64::try_from(MAX_FRAME_VERTICES)
            .ok()
            .and_then(|count| count.checked_mul(u64::from(VERTEX_STRIDE)))
            .ok_or(Error::FrameTooComplex)?;
        let vertex_buffer = table
            .define_buffer(
                BufferDesc::new(vertex_bytes, BufferUsage::VERTEX | BufferUsage::COPY_DST)
                    .map_err(|_| Error::sgfx(Stage::DefineResources))?,
            )
            .map_err(|_| Error::sgfx(Stage::DefineResources))?
            .id();

        let solid_pipeline = define_pipeline(&table, FragmentProgram::Solid)?.id();
        let texture_pipeline = define_pipeline(
            &table,
            FragmentProgram::Texture(TextureSampleMode::Rgba),
        )?
        .id();
        let glyph_pipeline = define_pipeline(
            &table,
            FragmentProgram::Texture(TextureSampleMode::AlphaMask),
        )?
        .id();
        let sampler = table
            .define_sampler(SamplerDesc::new(
                FilterMode::Nearest,
                FilterMode::Nearest,
                AddressMode::ClampToEdge,
                AddressMode::ClampToEdge,
            ))
            .map_err(|_| Error::sgfx(Stage::DefineResources))?
            .id();
        let glyph_atlas = GlyphAtlas {
            texture: define_sampled_texture(
                &table,
                TextureFormat::R8Unorm,
                GLYPH_ATLAS_SIZE,
                GLYPH_ATLAS_SIZE,
            )?,
            entries: Vec::new(),
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
        };
        let canvas_pipeline = define_canvas_pipeline(&table)?.id();
        let canvas_texture_pipeline = define_canvas_texture_pipeline(&table)?.id();
        let canvas_dummy_buffer = table
            .define_buffer(
                BufferDesc::new(
                    u64::from(CANVAS_VERTEX_STRIDE) * 3,
                    BufferUsage::VERTEX | BufferUsage::COPY_DST,
                )
                .map_err(|_| Error::sgfx(Stage::DefineResources))?,
            )
            .map_err(|_| Error::sgfx(Stage::DefineResources))?
            .id();

        let mut cache = context
            .create_ir_resources(Rc::clone(&table))
            .map_err(|_| Error::sgfx(Stage::CreateIrResources))?;
        for index in 0..2 {
            cache
                .map_image(targets[index], Rc::clone(&images[index]))
                .map_err(|_| Error::sgfx(Stage::MapSharedImage))?;
        }

        Ok(Self {
            table,
            cache,
            images,
            targets,
            vertex_buffer,
            solid_pipeline,
            texture_pipeline,
            glyph_pipeline,
            sampler,
            buffer_textures: Vec::new(),
            glyph_atlas,
            canvas_pipeline,
            canvas_texture_pipeline,
            canvas_dummy_buffer,
            canvas_targets: Vec::new(),
            canvas_meshes: Vec::new(),
            canvas_textures: Vec::new(),
            frame_serial: 0,
            width,
            height,
        })
    }

    pub(crate) fn image(&self, slot: usize) -> Option<&Image> {
        self.images.get(slot).map(Rc::as_ref)
    }

    pub(crate) fn into_images(self) -> Vec<Rc<Image>> {
        let Self {
            cache,
            images,
            table,
            targets: _,
            vertex_buffer: _,
            solid_pipeline: _,
            texture_pipeline: _,
            glyph_pipeline: _,
            sampler: _,
            buffer_textures: _,
            glyph_atlas: _,
            canvas_pipeline: _,
            canvas_texture_pipeline: _,
            canvas_dummy_buffer: _,
            canvas_targets: _,
            canvas_meshes: _,
            canvas_textures: _,
            frame_serial: _,
            width: _,
            height: _,
        } = self;
        drop(cache);
        drop(table);
        images
    }

    pub(crate) fn render(
        &mut self,
        context: &Context,
        queue: &Queue,
        slot: usize,
        copy_from: Option<usize>,
        paint: &PaintContext<'_>,
        background: UiColor,
        scale_milli: u32,
        render_areas: &[PixelBounds],
    ) -> Result<()> {
        let target = *self.targets.get(slot).ok_or(Error::InvalidFrame)?;
        if let Some(source_slot) = copy_from {
            self.copy_target(context, queue, source_slot, slot)?;
        }
        let render_bounds = bounding_area(render_areas).ok_or(Error::InvalidFrame)?;
        self.advance_frame_serial();
        self.prepare_canvases(context, queue, paint, scale_milli)?;
        let lowered = self.lower(paint, scale_milli, render_bounds)?;
        self.submit(context, queue, target, background, render_areas, &lowered)
    }

    fn copy_target(
        &mut self,
        context: &Context,
        queue: &Queue,
        source_slot: usize,
        destination_slot: usize,
    ) -> Result<()> {
        if source_slot == destination_slot {
            return Err(Error::InvalidFrame);
        }
        let table = Rc::clone(&self.table);
        let source = table
            .texture_ref(*self.targets.get(source_slot).ok_or(Error::InvalidFrame)?)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let destination = table
            .texture_ref(
                *self
                    .targets
                    .get(destination_slot)
                    .ok_or(Error::InvalidFrame)?,
            )
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let full_rect = PixelRect::new(0, 0, self.width, self.height)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let mut encoder = CommandEncoder::new(&table);
        encoder
            .copy_texture_to_texture(source, full_rect, destination, full_rect)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let commands = encoder
            .finish()
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        queue
            .submit_ir(context, &mut self.cache, &commands)
            .map_err(|_| Error::sgfx(Stage::SubmitCommands))
    }

    fn advance_frame_serial(&mut self) {
        self.frame_serial = self.frame_serial.wrapping_add(1);
        if self.frame_serial == 0 {
            self.frame_serial = 1;
            for texture in &mut self.buffer_textures {
                texture.used_frame = 0;
            }
        }
    }

    fn lower<'frame>(
        &mut self,
        paint: &'frame PaintContext<'_>,
        scale_milli: u32,
        render_area: PixelBounds,
    ) -> Result<LoweredFrame<'frame>> {
        let render_bounds = FloatRect::new(
            render_area.x as f32,
            render_area.y as f32,
            render_area.width as f32,
            render_area.height as f32,
        );
        let mut tessellator =
            Tessellator::new(scale_milli, self.width, self.height, render_bounds)?;
        let mut draws = Vec::new();
        let mut uploads = Vec::new();
        let mut buffer_mappings: Vec<(u64, TextureId)> = Vec::new();
        let opacity = 1.0f32;
        let scale = scale_milli.max(1) as f32 / 1000.0;

        for command in paint.commands() {
            match command {
                PaintCommand::FillPath { path, color } => {
                    if let Some(geometry) = tessellator.fill_path(path)? {
                        push_draw(
                            &mut draws,
                            geometry,
                            ui_color(*color, opacity)?,
                            DrawSource::Solid,
                        )?;
                    }
                }
                PaintCommand::StrokePath {
                    path,
                    stroke_width,
                    color,
                } => {
                    if let Some(geometry) = tessellator.stroke_path(path, *stroke_width)? {
                        push_draw(
                            &mut draws,
                            geometry,
                            ui_color(*color, opacity)?,
                            DrawSource::Solid,
                        )?;
                    }
                }
                PaintCommand::StrokeRect {
                    rect,
                    stroke_width,
                    color,
                } => {
                    if let Some(geometry) =
                        tessellator.stroke_rect(*rect, 0.0, *stroke_width)?
                    {
                        push_draw(
                            &mut draws,
                            geometry,
                            ui_color(*color, opacity)?,
                            DrawSource::Solid,
                        )?;
                    }
                }
                PaintCommand::StrokeRoundedRect {
                    rect,
                    corner_radius,
                    stroke_width,
                    color,
                } => {
                    if let Some(geometry) =
                        tessellator.stroke_rect(*rect, *corner_radius, *stroke_width)?
                    {
                        push_draw(
                            &mut draws,
                            geometry,
                            ui_color(*color, opacity)?,
                            DrawSource::Solid,
                        )?;
                    }
                }
                PaintCommand::DrawText {
                    position,
                    text,
                    color,
                    font_size_px,
                } => {
                    if !position.x.is_finite()
                        || !position.y.is_finite()
                        || !font_size_px.is_finite()
                    {
                        return Err(Error::InvalidFrame);
                    }
                    let origin_x = scale_text_origin(position.x, scale_milli);
                    let origin_y = scale_text_origin(position.y, scale_milli);
                    let color = ui_color(*color, opacity)?;
                    let glyphs = rasterize_text(text, *font_size_px, scale_milli);
                    for glyph in glyphs {
                        if glyph.width == 0 || glyph.height == 0 {
                            continue;
                        }
                        let (texture, atlas_bounds, upload_required) = self.glyph_texture(
                            glyph.key,
                            glyph.width,
                            glyph.height,
                        )?;
                        if upload_required {
                            uploads
                                .try_reserve(1)
                                .map_err(|_| Error::FrameTooComplex)?;
                            uploads.push(TextureUpload {
                                texture,
                                x: atlas_bounds.x,
                                y: atlas_bounds.y,
                                width: glyph.width,
                                height: glyph.height,
                                bytes_per_row: glyph.width,
                                bytes: UploadBytes::Shared(glyph.mask),
                            });
                        }
                        let destination = FloatRect::new(
                            origin_x.saturating_add(glyph.x) as f32,
                            origin_y.saturating_add(glyph.y) as f32,
                            glyph.width as f32,
                            glyph.height as f32,
                        );
                        if let Some(geometry) = tessellator.textured_rect(
                            destination,
                            atlas_tex_coords(atlas_bounds),
                        )? {
                            push_draw(&mut draws, geometry, color, DrawSource::Glyph(texture))?;
                        }
                    }
                }
                PaintCommand::DrawBuffer { dst, buffer_idx } => {
                    let Some(buffer) = paint.buffer(BufferHandle(*buffer_idx)) else {
                        continue;
                    };
                    if let Some((geometry, texture, upload)) = self.lower_buffer(
                        &mut tessellator,
                        &mut buffer_mappings,
                        buffer,
                        FloatRect::new(0.0, 0.0, buffer.width() as f32, buffer.height() as f32),
                        FloatRect::new(
                            truncated_scaled(dst.origin.x, scale),
                            truncated_scaled(dst.origin.y, scale),
                            buffer.width() as f32,
                            buffer.height() as f32,
                        ),
                    )? {
                        if let Some(upload) = upload {
                            uploads.push(upload);
                        }
                        push_draw(
                            &mut draws,
                            geometry,
                            [1.0, 1.0, 1.0, opacity],
                            DrawSource::Texture(texture),
                        )?;
                    }
                }
                PaintCommand::DrawBufferRect {
                    dst,
                    src,
                    buffer_idx,
                    opacity: command_opacity,
                } => {
                    let Some(buffer) = paint.buffer(BufferHandle(*buffer_idx)) else {
                        continue;
                    };
                    let scaled_source = FloatRect::from_logical(*src, scale);
                    let source = FloatRect::new(
                        truncated(scaled_source.x),
                        truncated(scaled_source.y),
                        truncated(scaled_source.width),
                        truncated(scaled_source.height),
                    );
                    let destination = FloatRect::new(
                        truncated_scaled(dst.origin.x, scale),
                        truncated_scaled(dst.origin.y, scale),
                        source.width,
                        source.height,
                    );
                    if let Some((geometry, texture, upload)) = self.lower_buffer(
                        &mut tessellator,
                        &mut buffer_mappings,
                        buffer,
                        source,
                        destination,
                    )? {
                        if let Some(upload) = upload {
                            uploads.push(upload);
                        }
                        let combined_opacity = finite_unit(*command_opacity)? * opacity;
                        push_draw(
                            &mut draws,
                            geometry,
                            [1.0, 1.0, 1.0, combined_opacity],
                            DrawSource::Texture(texture),
                        )?;
                    }
                }
                PaintCommand::PushClip {
                    rect,
                    corner_radius,
                } => tessellator.push_clip(*rect, *corner_radius)?,
                PaintCommand::PopClip => tessellator.pop_clip(),
                PaintCommand::SetOpacity { opacity: _ } => {}
                PaintCommand::Extension { rect, payload } => {
                    let Some(canvas) = payload.as_any().downcast_ref::<SgfxCanvasPaint>() else {
                        continue;
                    };
                    let Some(texture) = self.canvas_targets.iter().find(|target| {
                        target.handle_id == canvas.handle.id() && target.initialized
                    }) else {
                        continue;
                    };
                    let destination = FloatRect::new(
                        truncated_scaled(rect.origin.x, scale),
                        truncated_scaled(rect.origin.y, scale),
                        truncated_scaled(rect.size.width, scale),
                        truncated_scaled(rect.size.height, scale),
                    );
                    let tex_coords = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
                    if let Some(geometry) =
                        tessellator.textured_rect(destination, tex_coords)?
                    {
                        push_draw(
                            &mut draws,
                            geometry,
                            [1.0, 1.0, 1.0, opacity],
                            DrawSource::Texture(texture.texture),
                        )?;
                    }
                }
            }
        }

        // SGFX requires every render pass to contain at least one draw. Keep a
        // transparent degenerate draw so a damage rectangle that only clears
        // removed content still has a valid pass.
        let geometry = tessellator.dummy_draw()?;
        draws
            .try_reserve(1)
            .map_err(|_| Error::FrameTooComplex)?;
        draws.push(Draw {
            geometry,
            color: [0.0, 0.0, 0.0, 0.0],
            source: DrawSource::Solid,
        });
        let vertex_bytes = encode_vertices(tessellator.vertices())?;
        Ok(LoweredFrame {
            vertex_bytes,
            draws,
            uploads,
        })
    }

    fn prepare_canvases(
        &mut self,
        context: &Context,
        queue: &Queue,
        paint: &PaintContext<'_>,
        scale_milli: u32,
    ) -> Result<()> {
        let scale = scale_milli.max(1) as f32 / 1000.0;
        for command in paint.commands() {
            let PaintCommand::Extension { rect, payload } = command else {
                continue;
            };
            let Some(canvas) = payload.as_any().downcast_ref::<SgfxCanvasPaint>() else {
                continue;
            };
            let width = physical_canvas_extent(rect.size.width, scale)?;
            let height = physical_canvas_extent(rect.size.height, scale)?;
            let target_index = self.canvas_target(canvas.handle.id(), width, height)?;
            let unchanged = {
                let target = &self.canvas_targets[target_index];
                target.initialized && target.revision == canvas.frame.revision
            };
            if unchanged {
                continue;
            }
            self.render_canvas(context, queue, target_index, &canvas.frame)?;
            let target = &mut self.canvas_targets[target_index];
            target.revision = canvas.frame.revision;
            target.initialized = true;
        }
        Ok(())
    }

    fn canvas_target(&mut self, handle_id: u64, width: u32, height: u32) -> Result<usize> {
        if let Some(index) = self.canvas_targets.iter().position(|target| {
            target.handle_id == handle_id && target.width == width && target.height == height
        }) {
            return Ok(index);
        }
        if self.canvas_targets.len() >= MAX_CANVASES {
            return Err(Error::FrameTooComplex);
        }
        let extent = Extent2D::new(width, height).map_err(|_| Error::InvalidFrame)?;
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
        self.canvas_targets.push(CanvasTarget {
            handle_id,
            texture,
            width,
            height,
            revision: 0,
            initialized: false,
        });
        Ok(self.canvas_targets.len() - 1)
    }

    fn canvas_mesh(&mut self, mesh: &SgfxMesh) -> Result<usize> {
        if let Some(index) = self
            .canvas_meshes
            .iter()
            .position(|cached| cached.mesh_id == mesh.id)
        {
            return Ok(index);
        }
        if self.canvas_meshes.len() >= MAX_CANVAS_MESHES
            || mesh.vertices.is_empty()
            || mesh.vertices.len() % 3 != 0
        {
            return Err(Error::FrameTooComplex);
        }
        if !mesh.vertices.iter().all(|vertex| {
            vertex.position.iter().all(|value| value.is_finite())
                && vertex.color.iter().all(|value| value.is_finite())
                && vertex.tex_coord.iter().all(|value| value.is_finite())
        }) {
            return Err(Error::InvalidFrame);
        }
        let vertex_count = u32::try_from(mesh.vertices.len()).map_err(|_| Error::FrameTooComplex)?;
        let byte_size = u64::from(vertex_count)
            .checked_mul(u64::from(CANVAS_VERTEX_STRIDE))
            .ok_or(Error::FrameTooComplex)?;
        let buffer = self
            .table
            .define_buffer(
                BufferDesc::new(byte_size, BufferUsage::VERTEX | BufferUsage::COPY_DST)
                    .map_err(|_| Error::sgfx(Stage::DefineResources))?,
            )
            .map_err(|_| Error::sgfx(Stage::DefineResources))?
            .id();
        self.canvas_meshes.push(CanvasMesh {
            mesh_id: mesh.id,
            buffer,
            vertex_count,
            uploaded: false,
        });
        Ok(self.canvas_meshes.len() - 1)
    }

    fn canvas_texture(&mut self, texture: &Arc<SgfxTexture>) -> Result<usize> {
        if let Some(index) = self
            .canvas_textures
            .iter()
            .position(|cached| cached.texture_id == texture.id)
        {
            return Ok(index);
        }
        let expected_len = usize::try_from(texture.width)
            .ok()
            .and_then(|width| {
                usize::try_from(texture.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(Error::FrameTooComplex)?;
        if self.canvas_textures.len() >= MAX_CANVAS_TEXTURES
            || texture.width == 0
            || texture.height == 0
            || texture.pixels.len() != expected_len
        {
            return Err(Error::InvalidFrame);
        }
        let texture_id = define_sampled_texture(
            &self.table,
            TextureFormat::Rgba8Unorm,
            texture.width,
            texture.height,
        )?;
        self.canvas_textures.push(CanvasTexture {
            texture_id: texture.id,
            texture: texture_id,
            source: Arc::clone(texture),
            uploaded: false,
        });
        Ok(self.canvas_textures.len() - 1)
    }

    fn render_canvas(
        &mut self,
        context: &Context,
        queue: &Queue,
        target_index: usize,
        frame: &SgfxCanvasFrame,
    ) -> Result<()> {
        if frame.draws.len() > MAX_CANVAS_DRAWS {
            return Err(Error::FrameTooComplex);
        }
        let mut mesh_indices = Vec::new();
        let mut texture_indices = Vec::new();
        mesh_indices
            .try_reserve_exact(frame.draws.len())
            .map_err(|_| Error::FrameTooComplex)?;
        texture_indices
            .try_reserve_exact(frame.draws.len())
            .map_err(|_| Error::FrameTooComplex)?;
        for draw in &frame.draws {
            mesh_indices.push(self.canvas_mesh(&draw.mesh)?);
            texture_indices.push(
                draw.texture
                    .as_ref()
                    .map(|texture| self.canvas_texture(texture))
                    .transpose()?,
            );
        }

        let table = Rc::clone(&self.table);
        let target_state = self
            .canvas_targets
            .get(target_index)
            .ok_or(Error::InvalidFrame)?;
        let target = table
            .texture_ref(target_state.texture)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let area = PixelRect::new(0, 0, target_state.width, target_state.height)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let color_pipeline = table
            .render_pipeline_ref(self.canvas_pipeline)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let texture_pipeline = table
            .render_pipeline_ref(self.canvas_texture_pipeline)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let sampler = table
            .sampler_ref(self.sampler)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let mut uploads: Vec<(usize, Vec<u8>)> = Vec::new();
        for (draw, mesh_index) in frame.draws.iter().zip(mesh_indices.iter().copied()) {
            let cached = &self.canvas_meshes[mesh_index];
            if cached.uploaded || uploads.iter().any(|(index, _)| *index == mesh_index) {
                continue;
            }
            let bytes = encode_canvas_vertices(&draw.mesh)?;
            uploads.push((mesh_index, bytes));
        }
        let mut texture_uploads = Vec::new();
        for texture_index in texture_indices.iter().flatten().copied() {
            if self.canvas_textures[texture_index].uploaded
                || texture_uploads.contains(&texture_index)
            {
                continue;
            }
            texture_uploads.push(texture_index);
        }
        let clear = ir_color(ui_color(frame.clear_color, 1.0)?)?;
        if frame.draws.is_empty() {
            let mut encoder = CommandEncoder::new(&table);
            let descriptor = RenderPassDesc::new(
                &table,
                target,
                area,
                LoadOp::Clear(clear),
                StoreOp::Store,
            )
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            let mut pass = encoder
                .begin_render_pass(descriptor)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            pass.set_pipeline(color_pipeline)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            let dummy = table
                .buffer_ref(self.canvas_dummy_buffer)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            pass.set_vertex_buffer(dummy, 0)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            pass.set_uniforms(DrawUniforms::new(
                Transform::identity(),
                Color::rgba(0.0, 0.0, 0.0, 0.0).map_err(|_| Error::InvalidFrame)?,
            ))
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            pass.draw(3, 0)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            pass.end()
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            let commands = encoder
                .finish()
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            queue
                .submit_ir(context, &mut self.cache, &commands)
                .map_err(|_| Error::sgfx(Stage::SubmitCommands))?;
        } else {
            let mut draw_index = 0usize;
            let mut draw_offset = 0u32;
            let mut first_submission = true;
            while draw_index < frame.draws.len() {
                let mut encoder = CommandEncoder::new(&table);
                if first_submission {
                    for (mesh_index, bytes) in &uploads {
                        let cached = &self.canvas_meshes[*mesh_index];
                        let buffer = table
                            .buffer_ref(cached.buffer)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        encoder
                            .write_buffer(buffer, 0, bytes)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    }
                    for texture_index in &texture_uploads {
                        let cached = &self.canvas_textures[*texture_index];
                        let texture = table
                            .texture_ref(cached.texture)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        let destination = PixelRect::new(
                            0,
                            0,
                            cached.source.width,
                            cached.source.height,
                        )
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        let bytes_per_row = cached
                            .source
                            .width
                            .checked_mul(4)
                            .ok_or(Error::FrameTooComplex)?;
                        let write =
                            TextureWrite::new(destination, bytes_per_row, &cached.source.pixels)
                                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        encoder
                            .write_texture(texture, write)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    }
                }
                let load = if first_submission {
                    LoadOp::Clear(clear)
                } else {
                    LoadOp::Load
                };
                let descriptor =
                    RenderPassDesc::new(&table, target, area, load, StoreOp::Store)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                let mut pass = encoder
                    .begin_render_pass(descriptor)
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                let mut pass_vertices = 0u32;
                let mut estimated_bytes = PASS_FIXED_COMMAND_BYTES;

                while draw_index < frame.draws.len() && pass_vertices < MAX_PASS_VERTICES {
                    let draw = &frame.draws[draw_index];
                    let mesh_index = mesh_indices[draw_index];
                    let texture_index = texture_indices[draw_index];
                    let cached = &self.canvas_meshes[mesh_index];
                    let remaining = cached.vertex_count.saturating_sub(draw_offset);
                    let draw_bytes = if texture_index.is_some() {
                        TEXTURED_DRAW_COMMAND_BYTES
                    } else {
                        SOLID_DRAW_COMMAND_BYTES
                    };
                    let available_bytes = MAX_OPAQUE_COMMAND_BYTES
                        .saturating_sub(estimated_bytes)
                        .saturating_sub(draw_bytes);
                    let available_vertices = (MAX_PASS_VERTICES - pass_vertices)
                        .min(available_bytes / CANONICAL_VERTEX_BYTES);
                    let chunk = remaining.min(available_vertices) / 3 * 3;
                    if chunk == 0 {
                        break;
                    }

                    let buffer = table
                        .buffer_ref(cached.buffer)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    let transform = Transform::from_columns(draw.transform)
                        .map_err(|_| Error::InvalidFrame)?;
                    let tint = ir_color(ui_color(draw.tint, 1.0)?)?;
                    if let Some(texture_index) = texture_index {
                        let texture = table
                            .texture_ref(self.canvas_textures[texture_index].texture)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        pass.set_pipeline(texture_pipeline)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        pass.set_texture(texture)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        pass.set_sampler(sampler)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    } else {
                        pass.set_pipeline(color_pipeline)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    }
                    pass.set_vertex_buffer(buffer, 0)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    pass.set_uniforms(DrawUniforms::new(transform, tint))
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    pass.draw(chunk, draw_offset)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;

                    pass_vertices = pass_vertices.saturating_add(chunk);
                    estimated_bytes = estimated_bytes
                        .saturating_add(draw_bytes)
                        .saturating_add(chunk.saturating_mul(CANONICAL_VERTEX_BYTES));
                    draw_offset = draw_offset.saturating_add(chunk);
                    if draw_offset == cached.vertex_count {
                        draw_index += 1;
                        draw_offset = 0;
                    }
                }
                if pass_vertices == 0 {
                    return Err(Error::FrameTooComplex);
                }
                pass.end()
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                let commands = encoder
                    .finish()
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                queue
                    .submit_ir(context, &mut self.cache, &commands)
                    .map_err(|_| Error::sgfx(Stage::SubmitCommands))?;
                first_submission = false;
            }
        }
        for (mesh_index, _) in &uploads {
            self.canvas_meshes[*mesh_index].uploaded = true;
        }
        for texture_index in texture_uploads {
            self.canvas_textures[texture_index].uploaded = true;
        }
        Ok(())
    }

    fn lower_buffer<'frame>(
        &mut self,
        tessellator: &mut Tessellator,
        mappings: &mut Vec<(u64, TextureId)>,
        buffer: &'frame Buffer,
        source: FloatRect,
        destination: FloatRect,
    ) -> Result<Option<(GeometryRange, TextureId, Option<TextureUpload<'frame>>)>> {
        if buffer.width() == 0 || buffer.height() == 0 || source.is_empty() {
            return Ok(None);
        }
        let source_left = source.x.max(0.0).min(buffer.width() as f32);
        let source_top = source.y.max(0.0).min(buffer.height() as f32);
        let source_right = source.right().max(0.0).min(buffer.width() as f32);
        let source_bottom = source.bottom().max(0.0).min(buffer.height() as f32);
        if source_right <= source_left || source_bottom <= source_top {
            return Ok(None);
        }
        let clipped_destination = FloatRect::new(
            destination.x + source_left - source.x,
            destination.y + source_top - source.y,
            source_right - source_left,
            source_bottom - source_top,
        );
        let inverse_width = 1.0 / buffer.width() as f32;
        let inverse_height = 1.0 / buffer.height() as f32;
        let tex_coords = [
            [source_left * inverse_width, source_top * inverse_height],
            [source_right * inverse_width, source_top * inverse_height],
            [source_right * inverse_width, source_bottom * inverse_height],
            [source_left * inverse_width, source_bottom * inverse_height],
        ];
        let Some(geometry) = tessellator.textured_rect(clipped_destination, tex_coords)? else {
            return Ok(None);
        };

        let buffer_identity = buffer.identity();
        if let Some((_, texture)) = mappings
            .iter()
            .find(|(mapped_identity, _)| *mapped_identity == buffer_identity)
        {
            return Ok(Some((geometry, *texture, None)));
        }
        let (texture, upload_required) = self.buffer_texture(buffer)?;
        mappings
            .try_reserve(1)
            .map_err(|_| Error::FrameTooComplex)?;
        mappings.push((buffer_identity, texture));
        if !upload_required {
            return Ok(Some((geometry, texture, None)));
        }
        let bytes_per_row = buffer
            .width()
            .checked_mul(4)
            .ok_or(Error::FrameTooComplex)?;
        Ok(Some((
            geometry,
            texture,
            Some(TextureUpload {
                texture,
                x: 0,
                y: 0,
                width: buffer.width(),
                height: buffer.height(),
                bytes_per_row,
                bytes: UploadBytes::Borrowed(buffer.data()),
            }),
        )))
    }

    fn buffer_texture(&mut self, buffer: &Buffer) -> Result<(TextureId, bool)> {
        let buffer_identity = buffer.identity();
        let revision = buffer.revision();
        let width = buffer.width();
        let height = buffer.height();
        if let Some(texture) = self.buffer_textures.iter_mut().find(|texture| {
            texture.buffer_identity == buffer_identity
                && texture.width == width
                && texture.height == height
        }) {
            let upload_required = texture.revision != revision;
            texture.revision = revision;
            texture.used_frame = self.frame_serial;
            return Ok((texture.texture, upload_required));
        }
        if let Some(texture) = self.buffer_textures.iter_mut().find(|texture| {
            texture.width == width
                && texture.height == height
                && texture.used_frame != self.frame_serial
        }) {
            texture.buffer_identity = buffer_identity;
            texture.revision = revision;
            texture.used_frame = self.frame_serial;
            return Ok((texture.texture, true));
        }
        if self.buffer_textures.len() >= MAX_BUFFER_TEXTURES {
            return Err(Error::FrameTooComplex);
        }
        let texture = define_sampled_texture(&self.table, TextureFormat::Bgra8Unorm, width, height)?;
        self.buffer_textures.push(BufferTexture {
            texture,
            buffer_identity,
            revision,
            width,
            height,
            used_frame: self.frame_serial,
        });
        Ok((texture, true))
    }

    fn glyph_texture(
        &mut self,
        key: GlyphRasterKey,
        width: u32,
        height: u32,
    ) -> Result<(TextureId, PixelBounds, bool)> {
        if let Some(entry) = self
            .glyph_atlas
            .entries
            .iter()
            .find(|entry| entry.key == key && entry.width == width && entry.height == height)
        {
            return Ok((
                self.glyph_atlas.texture,
                PixelBounds {
                    x: entry.x,
                    y: entry.y,
                    width: entry.width,
                    height: entry.height,
                },
                false,
            ));
        }
        if self.glyph_atlas.entries.len() >= MAX_GLYPH_ENTRIES {
            return Err(Error::FrameTooComplex);
        }
        let padded_width = width
            .checked_add(GLYPH_ATLAS_PADDING)
            .ok_or(Error::FrameTooComplex)?;
        let padded_height = height
            .checked_add(GLYPH_ATLAS_PADDING)
            .ok_or(Error::FrameTooComplex)?;
        if padded_width > GLYPH_ATLAS_SIZE || padded_height > GLYPH_ATLAS_SIZE {
            return Err(Error::FrameTooComplex);
        }
        if self
            .glyph_atlas
            .cursor_x
            .checked_add(padded_width)
            .is_none_or(|right| right > GLYPH_ATLAS_SIZE)
        {
            self.glyph_atlas.cursor_x = 0;
            self.glyph_atlas.cursor_y = self
                .glyph_atlas
                .cursor_y
                .checked_add(self.glyph_atlas.row_height)
                .ok_or(Error::FrameTooComplex)?;
            self.glyph_atlas.row_height = 0;
        }
        if self
            .glyph_atlas
            .cursor_y
            .checked_add(padded_height)
            .is_none_or(|bottom| bottom > GLYPH_ATLAS_SIZE)
        {
            return Err(Error::FrameTooComplex);
        }
        let bounds = PixelBounds {
            x: self.glyph_atlas.cursor_x,
            y: self.glyph_atlas.cursor_y,
            width,
            height,
        };
        self.glyph_atlas.entries.push(GlyphTexture {
            key,
            x: bounds.x,
            y: bounds.y,
            width,
            height,
        });
        self.glyph_atlas.cursor_x = self
            .glyph_atlas
            .cursor_x
            .checked_add(padded_width)
            .ok_or(Error::FrameTooComplex)?;
        self.glyph_atlas.row_height = self.glyph_atlas.row_height.max(padded_height);
        Ok((self.glyph_atlas.texture, bounds, true))
    }

    fn submit(
        &mut self,
        context: &Context,
        queue: &Queue,
        target_id: TextureId,
        background: UiColor,
        render_areas: &[PixelBounds],
        frame: &LoweredFrame<'_>,
    ) -> Result<()> {
        let table = Rc::clone(&self.table);
        let target = table
            .texture_ref(target_id)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let vertex_buffer = table
            .buffer_ref(self.vertex_buffer)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let sampler = table
            .sampler_ref(self.sampler)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let solid_pipeline = table
            .render_pipeline_ref(self.solid_pipeline)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let texture_pipeline = table
            .render_pipeline_ref(self.texture_pipeline)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let glyph_pipeline = table
            .render_pipeline_ref(self.glyph_pipeline)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let clear_color = ir_color(ui_color(background, 1.0)?)?;
        let transform = pixel_transform(self.width, self.height)?;

        let mut first_submission = true;
        for render_area in render_areas {
            let area = PixelRect::new(
                render_area.x,
                render_area.y,
                render_area.width,
                render_area.height,
            )
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            let mut draw_index = 0usize;
            let mut draw_offset = 0u32;
            let mut first_pass = true;
            while draw_index < frame.draws.len() {
                // Keep each render pass in its own command buffer and account
                // for vertex plus per-draw state against the backend's opaque
                // byte limit. Separate submissions retain paint order through
                // LoadOp::Load.
                let mut encoder = CommandEncoder::new(&table);
                if first_submission {
                    encoder
                        .write_buffer(vertex_buffer, 0, &frame.vertex_bytes)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    for upload in &frame.uploads {
                        let texture = table
                            .texture_ref(upload.texture)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        let destination = PixelRect::new(
                            upload.x,
                            upload.y,
                            upload.width,
                            upload.height,
                        )
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        let write = TextureWrite::new(
                            destination,
                            upload.bytes_per_row,
                            upload.bytes.as_slice(),
                        )
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        encoder
                            .write_texture(texture, write)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    }
                }
                let load = if first_pass {
                    LoadOp::Clear(clear_color)
                } else {
                    LoadOp::Load
                };
                let descriptor = RenderPassDesc::new(&table, target, area, load, StoreOp::Store)
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                let mut pass = encoder
                    .begin_render_pass(descriptor)
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                let mut pass_vertices = 0u32;
                let mut estimated_bytes = PASS_FIXED_COMMAND_BYTES;

                while draw_index < frame.draws.len() && pass_vertices < MAX_PASS_VERTICES {
                    let draw = frame.draws[draw_index];
                    let Some(scissor) = intersect_bounds(draw.geometry.scissor, *render_area) else {
                        draw_index += 1;
                        draw_offset = 0;
                        continue;
                    };
                    let remaining = draw.geometry.vertex_count.saturating_sub(draw_offset);
                    let draw_bytes = match draw.source {
                        DrawSource::Solid => SOLID_DRAW_COMMAND_BYTES,
                        DrawSource::Texture(_) | DrawSource::Glyph(_) => {
                            TEXTURED_DRAW_COMMAND_BYTES
                        }
                    };
                    let available_bytes = MAX_OPAQUE_COMMAND_BYTES
                        .saturating_sub(estimated_bytes)
                        .saturating_sub(draw_bytes);
                    let available = (MAX_PASS_VERTICES - pass_vertices)
                        .min(available_bytes / CANONICAL_VERTEX_BYTES);
                    let chunk = remaining.min(available) / 3 * 3;
                    if chunk == 0 {
                        break;
                    }
                    let (pipeline, texture) = match draw.source {
                        DrawSource::Solid => (solid_pipeline, None),
                        DrawSource::Texture(texture) => (texture_pipeline, Some(texture)),
                        DrawSource::Glyph(texture) => (glyph_pipeline, Some(texture)),
                    };
                    pass.set_pipeline(pipeline)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    pass.set_vertex_buffer(vertex_buffer, 0)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    if let Some(texture) = texture {
                        let texture = table
                            .texture_ref(texture)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        pass.set_texture(texture)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        pass.set_sampler(sampler)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    }
                    pass.set_uniforms(DrawUniforms::new(transform, ir_color(draw.color)?))
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    let scissor =
                        PixelRect::new(scissor.x, scissor.y, scissor.width, scissor.height)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    pass.set_scissor(Some(scissor))
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    pass.draw(
                        chunk,
                        draw.geometry.first_vertex.saturating_add(draw_offset),
                    )
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    pass_vertices += chunk;
                    estimated_bytes = estimated_bytes
                        .saturating_add(draw_bytes)
                        .saturating_add(chunk.saturating_mul(CANONICAL_VERTEX_BYTES));
                    draw_offset += chunk;
                    if draw_offset == draw.geometry.vertex_count {
                        draw_index += 1;
                        draw_offset = 0;
                    }
                }
                if pass_vertices == 0 {
                    return Err(Error::FrameTooComplex);
                }
                pass.end()
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                let commands = encoder
                    .finish()
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                queue
                    .submit_ir(context, &mut self.cache, &commands)
                    .map_err(|_| Error::sgfx(Stage::SubmitCommands))?;
                first_submission = false;
                first_pass = false;
            }
        }
        Ok(())
    }
}

fn bounding_area(areas: &[PixelBounds]) -> Option<PixelBounds> {
    let mut bounds: Option<PixelBounds> = None;
    for area in areas {
        bounds = Some(match bounds {
            None => *area,
            Some(current) => {
                let left = current.x.min(area.x);
                let top = current.y.min(area.y);
                let right = current
                    .x
                    .saturating_add(current.width)
                    .max(area.x.saturating_add(area.width));
                let bottom = current
                    .y
                    .saturating_add(current.height)
                    .max(area.y.saturating_add(area.height));
                PixelBounds {
                    x: left,
                    y: top,
                    width: right.saturating_sub(left),
                    height: bottom.saturating_sub(top),
                }
            }
        });
    }
    bounds
}

fn intersect_bounds(left: PixelBounds, right: PixelBounds) -> Option<PixelBounds> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let x2 = left
        .x
        .saturating_add(left.width)
        .min(right.x.saturating_add(right.width));
    let y2 = left
        .y
        .saturating_add(left.height)
        .min(right.y.saturating_add(right.height));
    if x2 <= x || y2 <= y {
        None
    } else {
        Some(PixelBounds {
            x,
            y,
            width: x2 - x,
            height: y2 - y,
        })
    }
}

fn define_pipeline(
    table: &ResourceTable,
    fragment: FragmentProgram,
) -> Result<sgfx::ir::RenderPipelineRef<'_>> {
    let layout = VertexBufferLayout::new(
        VERTEX_STRIDE,
        alloc::vec![
            VertexAttribute::new(0, VertexFormat::Float32x2, 0),
            VertexAttribute::new(1, VertexFormat::Float32x2, 8),
        ],
    )
    .map_err(|_| Error::sgfx(Stage::DefineResources))?;
    let descriptor = RenderPipelineDesc::new(
        TextureFormat::Bgra8Unorm,
        PrimitiveTopology::TriangleList,
        layout,
        fragment,
        BlendState::SOURCE_OVER_STRAIGHT_ALPHA,
        RasterState::new(sgfx::ir::CullMode::None, FrontFace::CounterClockwise),
    )
    .map_err(|_| Error::sgfx(Stage::DefineResources))?;
    table
        .define_render_pipeline(descriptor)
        .map_err(|_| Error::sgfx(Stage::DefineResources))
}

fn define_canvas_pipeline(
    table: &ResourceTable,
) -> Result<sgfx::ir::RenderPipelineRef<'_>> {
    let layout = VertexBufferLayout::new(
        CANVAS_VERTEX_STRIDE,
        alloc::vec![
            VertexAttribute::new(0, VertexFormat::Float32x4, 0),
            VertexAttribute::new(1, VertexFormat::Float32x4, 16),
        ],
    )
    .map_err(|_| Error::sgfx(Stage::DefineResources))?;
    let descriptor = RenderPipelineDesc::new(
        TextureFormat::Bgra8Unorm,
        PrimitiveTopology::TriangleList,
        layout,
        FragmentProgram::VertexColor,
        BlendState::SOURCE_OVER_STRAIGHT_ALPHA,
        RasterState::new(sgfx::ir::CullMode::Back, FrontFace::CounterClockwise),
    )
    .map_err(|_| Error::sgfx(Stage::DefineResources))?;
    table
        .define_render_pipeline(descriptor)
        .map_err(|_| Error::sgfx(Stage::DefineResources))
}

fn define_canvas_texture_pipeline(
    table: &ResourceTable,
) -> Result<sgfx::ir::RenderPipelineRef<'_>> {
    let layout = VertexBufferLayout::new(
        CANVAS_VERTEX_STRIDE,
        alloc::vec![
            VertexAttribute::new(0, VertexFormat::Float32x4, 0),
            VertexAttribute::new(1, VertexFormat::Float32x2, 32),
        ],
    )
    .map_err(|_| Error::sgfx(Stage::DefineResources))?;
    let descriptor = RenderPipelineDesc::new(
        TextureFormat::Bgra8Unorm,
        PrimitiveTopology::TriangleList,
        layout,
        FragmentProgram::Texture(TextureSampleMode::Rgba),
        BlendState::SOURCE_OVER_STRAIGHT_ALPHA,
        RasterState::new(sgfx::ir::CullMode::Back, FrontFace::CounterClockwise),
    )
    .map_err(|_| Error::sgfx(Stage::DefineResources))?;
    table
        .define_render_pipeline(descriptor)
        .map_err(|_| Error::sgfx(Stage::DefineResources))
}

fn define_sampled_texture(
    table: &ResourceTable,
    format: TextureFormat,
    width: u32,
    height: u32,
) -> Result<TextureId> {
    let extent = Extent2D::new(width, height).map_err(|_| Error::InvalidFrame)?;
    let descriptor = TextureDesc::new(
        format,
        extent,
        TextureUsage::SAMPLED | TextureUsage::COPY_DST,
    )
    .map_err(|_| Error::sgfx(Stage::DefineResources))?;
    table
        .define_texture(descriptor)
        .map(|texture| texture.id())
        .map_err(|_| Error::FrameTooComplex)
}

fn push_draw(
    draws: &mut Vec<Draw>,
    geometry: GeometryRange,
    color: [f32; 4],
    source: DrawSource,
) -> Result<()> {
    if let Some(previous) = draws.last_mut() {
        let previous_end = previous
            .geometry
            .first_vertex
            .checked_add(previous.geometry.vertex_count);
        if previous.source == source
            && previous.color == color
            && previous.geometry.scissor == geometry.scissor
            && previous_end == Some(geometry.first_vertex)
        {
            previous.geometry.vertex_count = previous
                .geometry
                .vertex_count
                .checked_add(geometry.vertex_count)
                .ok_or(Error::FrameTooComplex)?;
            return Ok(());
        }
    }
    draws
        .try_reserve(1)
        .map_err(|_| Error::FrameTooComplex)?;
    draws.push(Draw {
        geometry,
        color,
        source,
    });
    Ok(())
}

fn atlas_tex_coords(bounds: PixelBounds) -> [[f32; 2]; 4] {
    let inverse_size = 1.0 / GLYPH_ATLAS_SIZE as f32;
    let left = bounds.x as f32 * inverse_size;
    let top = bounds.y as f32 * inverse_size;
    let right = bounds.x.saturating_add(bounds.width) as f32 * inverse_size;
    let bottom = bounds.y.saturating_add(bounds.height) as f32 * inverse_size;
    [
        [left, top],
        [right, top],
        [right, bottom],
        [left, bottom],
    ]
}

fn encode_vertices(vertices: &[Vertex]) -> Result<Vec<u8>> {
    let capacity = vertices
        .len()
        .checked_mul(VERTEX_STRIDE as usize)
        .ok_or(Error::FrameTooComplex)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| Error::FrameTooComplex)?;
    for vertex in vertices {
        bytes.extend_from_slice(&vertex.position[0].to_le_bytes());
        bytes.extend_from_slice(&vertex.position[1].to_le_bytes());
        bytes.extend_from_slice(&vertex.tex_coord[0].to_le_bytes());
        bytes.extend_from_slice(&vertex.tex_coord[1].to_le_bytes());
    }
    Ok(bytes)
}

fn encode_canvas_vertices(mesh: &SgfxMesh) -> Result<Vec<u8>> {
    let capacity = mesh
        .vertices
        .len()
        .checked_mul(CANVAS_VERTEX_STRIDE as usize)
        .ok_or(Error::FrameTooComplex)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| Error::FrameTooComplex)?;
    for vertex in mesh.vertices.iter() {
        for component in vertex.position {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        for component in vertex.color {
            bytes.extend_from_slice(&component.clamp(0.0, 1.0).to_le_bytes());
        }
        for component in vertex.tex_coord {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    Ok(bytes)
}

fn physical_canvas_extent(logical: f32, scale: f32) -> Result<u32> {
    if !logical.is_finite() || logical <= 0.0 || !scale.is_finite() || scale <= 0.0 {
        return Err(Error::InvalidFrame);
    }
    let physical = libm::ceilf(logical * scale);
    if !physical.is_finite() || physical < 1.0 || physical > u32::MAX as f32 {
        return Err(Error::InvalidFrame);
    }
    Ok(physical as u32)
}

fn ui_color(color: UiColor, opacity: f32) -> Result<[f32; 4]> {
    if ![color.r, color.g, color.b, color.a, opacity]
        .iter()
        .all(|component| component.is_finite())
    {
        return Err(Error::InvalidFrame);
    }
    Ok([
        color.r.clamp(0.0, 1.0),
        color.g.clamp(0.0, 1.0),
        color.b.clamp(0.0, 1.0),
        color.a.clamp(0.0, 1.0) * opacity.clamp(0.0, 1.0),
    ])
}

fn ir_color(components: [f32; 4]) -> Result<Color> {
    Color::rgba(
        components[0],
        components[1],
        components[2],
        components[3],
    )
    .map_err(|_| Error::InvalidFrame)
}

fn finite_unit(value: f32) -> Result<f32> {
    if value.is_finite() {
        Ok(value.clamp(0.0, 1.0))
    } else {
        Err(Error::InvalidFrame)
    }
}

fn pixel_transform(width: u32, height: u32) -> Result<Transform> {
    if width == 0 || height == 0 {
        return Err(Error::InvalidFrame);
    }
    Transform::from_columns([
        2.0 / width as f32,
        0.0,
        0.0,
        0.0,
        0.0,
        -2.0 / height as f32,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
        -1.0,
        1.0,
        0.0,
        1.0,
    ])
    .map_err(|_| Error::InvalidFrame)
}

fn scale_text_origin(value: f32, scale_milli: u32) -> i32 {
    let logical = value as i32;
    ((logical as i64).saturating_mul(scale_milli.max(1) as i64) / 1000) as i32
}

fn truncated_scaled(value: f32, scale: f32) -> f32 {
    truncated(value * scale)
}

fn truncated(value: f32) -> f32 {
    (value as i32) as f32
}
