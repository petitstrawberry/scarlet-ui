//! Paint-command to persistent SGFX IR lowering.

use alloc::rc::Rc;
use alloc::sync::Arc;
use alloc::vec::Vec;

use scarlet_ui_core::buffer::Buffer;
use scarlet_ui_core::color::Color as UiColor;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::graphics::{GlyphRasterKey, rasterize_text};
use scarlet_ui_core::icon::{IconMaskKey, rasterize_icon};
use scarlet_ui_core::renderer::{BufferHandle, PaintCommand, PaintContext};
use sgfx::backend::CommandExecutor;
use sgfx::ir::{
    AddressMode, BlendState, BufferDesc, BufferId, BufferUsage, Color, CommandEncoder,
    CompareFunction, DepthLoadOp, DepthState, DrawUniforms, Extent2D, FilterMode, FragmentProgram,
    FrontFace, LoadOp, MAX_COMMANDS, PixelRect, PrimitiveTopology, RasterState, RenderPassDesc,
    RenderPipelineDesc, RenderPipelineId, ResourceTable, SamplerDesc, SamplerId, StoreOp,
    TextureDesc, TextureFormat, TextureId, TextureSampleMode, TextureUsage, TextureWrite,
    Transform, VertexAttribute, VertexBufferLayout, VertexFormat,
};

use crate::canvas::{SgfxCanvasFrame, SgfxCanvasPaint, SgfxCanvasVertex, SgfxMesh, SgfxTexture};
use crate::error::{Error, FrameError, Result, Stage};
use crate::geometry::{
    FloatRect, GeometryRange, MAX_FRAME_VERTICES, PixelBounds, Tessellator, Vertex,
};

const PAINT_VERTEX_STRIDE: u32 = 40;
const PASS_COMMANDS: usize = 2;
const MAX_PAINT_DRAW_COMMANDS: usize = 7;
const MAX_CANVAS_DRAW_COMMANDS: usize = 6;
const GLYPH_ATLAS_SIZE: u32 = 2_048;
const GLYPH_ATLAS_PADDING: u32 = 1;
const MAX_GLYPH_ENTRIES: usize = 1_024;
const MAX_ICON_ENTRIES: usize = 256;
const MAX_GLYPH_ATLASES: usize = 2;
const MAX_BUFFER_TEXTURES: usize = 128;
const CANVAS_VERTEX_STRIDE: u32 = 40;
const MAX_CANVASES: usize = 32;
const MAX_CANVAS_MESHES: usize = 256;
const MAX_CANVAS_TEXTURES: usize = 128;
const MAX_CANVAS_DRAWS: usize = 240;
const GRADIENT_BAND_COUNT: usize = 8;
const SHADOW_LAYER_COUNT: usize = 8;

const SHADOW_LAYER_WEIGHTS: [f32; SHADOW_LAYER_COUNT] =
    [0.02, 0.03, 0.05, 0.08, 0.12, 0.17, 0.23, 0.30];

const CANVAS_TARGET_TEX_COORDS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawSource {
    Solid,
    Texture(TextureId),
    Glyph(TextureId),
}

#[derive(Clone, Copy)]
struct Draw {
    geometry: GeometryRange,
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

fn texture_upload_scheduled(
    uploads: &[TextureUpload<'_>],
    texture: TextureId,
    bounds: PixelBounds,
) -> bool {
    uploads.iter().any(|upload| {
        upload.texture == texture
            && upload.x == bounds.x
            && upload.y == bounds.y
            && upload.width == bounds.width
            && upload.height == bounds.height
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextureUploadState {
    Pending,
    Uploaded,
}

#[derive(Clone, Copy)]
struct BufferTexture {
    texture: TextureId,
    buffer_identity: u64,
    revision: u64,
    width: u32,
    height: u32,
    used_frame: u64,
    upload_state: TextureUploadState,
}

struct GlyphTexture {
    key: GlyphRasterKey,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    upload_state: TextureUploadState,
}

struct IconTexture {
    key: IconMaskKey,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    upload_state: TextureUploadState,
}

struct GlyphAtlas {
    texture: TextureId,
    entries: Vec<GlyphTexture>,
    icon_entries: Vec<IconTexture>,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    used_frame: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtlasEntryKind {
    Glyph,
    Icon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GlyphAtlasCacheAction {
    Append(usize),
    Recycle(usize),
    Create,
}

impl GlyphAtlas {
    fn new(texture: TextureId) -> Self {
        Self {
            texture,
            entries: Vec::new(),
            icon_entries: Vec::new(),
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
            used_frame: 0,
        }
    }

    fn reset(&mut self, frame_serial: u64) {
        self.entries.clear();
        self.icon_entries.clear();
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.row_height = 0;
        self.used_frame = frame_serial;
    }

    fn can_allocate(&self, kind: AtlasEntryKind, width: u32, height: u32) -> bool {
        let has_entry_capacity = match kind {
            AtlasEntryKind::Glyph => self.entries.len() < MAX_GLYPH_ENTRIES,
            AtlasEntryKind::Icon => self.icon_entries.len() < MAX_ICON_ENTRIES,
        };
        has_entry_capacity && self.next_slot(width, height).is_some()
    }

    fn allocate(&mut self, kind: AtlasEntryKind, width: u32, height: u32) -> Option<PixelBounds> {
        if !self.can_allocate(kind, width, height) {
            return None;
        }
        let (bounds, cursor_x, cursor_y, row_height) = self.next_slot(width, height)?;
        self.cursor_x = cursor_x;
        self.cursor_y = cursor_y;
        self.row_height = row_height;
        Some(bounds)
    }

    fn next_slot(&self, width: u32, height: u32) -> Option<(PixelBounds, u32, u32, u32)> {
        let padded_width = width.checked_add(GLYPH_ATLAS_PADDING)?;
        let padded_height = height.checked_add(GLYPH_ATLAS_PADDING)?;
        if padded_width > GLYPH_ATLAS_SIZE || padded_height > GLYPH_ATLAS_SIZE {
            return None;
        }

        let mut x = self.cursor_x;
        let mut y = self.cursor_y;
        let mut row_height = self.row_height;
        if x.checked_add(padded_width)
            .is_none_or(|right| right > GLYPH_ATLAS_SIZE)
        {
            x = 0;
            y = y.checked_add(row_height)?;
            row_height = 0;
        }
        if y.checked_add(padded_height)
            .is_none_or(|bottom| bottom > GLYPH_ATLAS_SIZE)
        {
            return None;
        }

        Some((
            PixelBounds {
                x,
                y,
                width,
                height,
            },
            x.checked_add(padded_width)?,
            y,
            row_height.max(padded_height),
        ))
    }

    fn empty_can_allocate(width: u32, height: u32) -> bool {
        width
            .checked_add(GLYPH_ATLAS_PADDING)
            .is_some_and(|padded| padded <= GLYPH_ATLAS_SIZE)
            && height
                .checked_add(GLYPH_ATLAS_PADDING)
                .is_some_and(|padded| padded <= GLYPH_ATLAS_SIZE)
    }
}

fn glyph_atlas_cache_action(
    atlases: &[GlyphAtlas],
    frame_serial: u64,
    kind: AtlasEntryKind,
    width: u32,
    height: u32,
    max_atlases: usize,
) -> Option<GlyphAtlasCacheAction> {
    if !GlyphAtlas::empty_can_allocate(width, height) {
        return None;
    }
    if let Some(index) = atlases.iter().position(|atlas| {
        atlas.used_frame == frame_serial && atlas.can_allocate(kind, width, height)
    }) {
        return Some(GlyphAtlasCacheAction::Append(index));
    }
    if let Some(index) = atlases
        .iter()
        .position(|atlas| atlas.used_frame != frame_serial)
    {
        return Some(GlyphAtlasCacheAction::Recycle(index));
    }
    if atlases.len() < max_atlases {
        Some(GlyphAtlasCacheAction::Create)
    } else {
        None
    }
}

struct CanvasTarget {
    handle_id: u64,
    texture: TextureId,
    depth: Option<TextureId>,
    width: u32,
    height: u32,
    revision: u64,
    initialized: bool,
}

struct CanvasMesh {
    handle_id: u64,
    revision: u64,
    buffer: BufferId,
    vertex_count: u32,
    capacity_vertices: u32,
    uploaded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CanvasMeshCacheAction {
    Reuse,
    Upload,
    Reallocate(u32),
}

fn canvas_mesh_cache_action(
    cached_revision: u64,
    capacity_vertices: u32,
    revision: u64,
    vertex_count: u32,
) -> Result<CanvasMeshCacheAction> {
    if cached_revision == revision {
        return Ok(CanvasMeshCacheAction::Reuse);
    }
    if vertex_count <= capacity_vertices {
        return Ok(CanvasMeshCacheAction::Upload);
    }
    vertex_count
        .checked_next_power_of_two()
        .map(CanvasMeshCacheAction::Reallocate)
        .ok_or(Error::FrameTooComplex)
}

fn canvas_frame_has_revision_conflict(frame: &SgfxCanvasFrame) -> bool {
    frame.draws.iter().enumerate().any(|(index, draw)| {
        frame.draws[..index].iter().any(|previous| {
            previous.mesh.handle == draw.mesh.handle && previous.mesh.revision != draw.mesh.revision
        })
    })
}

fn validate_depth_support(requested: bool, supported: bool) -> Result<()> {
    if requested && !supported {
        Err(Error::DepthUnsupported)
    } else {
        Ok(())
    }
}

fn canvas_pass_reaches_frame_end(
    mesh_indices: &[usize],
    mut draw_index: usize,
    prefix_commands: usize,
) -> bool {
    let mut command_count = prefix_commands.saturating_add(PASS_COMMANDS);
    while draw_index < mesh_indices.len() {
        if mesh_indices.get(draw_index).is_none() {
            return false;
        }
        if command_count.saturating_add(MAX_CANVAS_DRAW_COMMANDS) > MAX_COMMANDS {
            return false;
        }
        command_count = command_count.saturating_add(MAX_CANVAS_DRAW_COMMANDS);
        draw_index += 1;
    }
    draw_index == mesh_indices.len()
}

struct CanvasTexture {
    texture_id: u64,
    texture: TextureId,
    source: Arc<SgfxTexture>,
    uploaded: bool,
}

/// Persistent logical SGFX resources and retained ScarletUI paint caches.
///
/// Physical images and all execution state remain owned by SGFX backend
/// sessions composed around this encoder.
pub struct SgfxPaintEncoder {
    table: Rc<ResourceTable>,
    targets: Vec<TextureId>,
    vertex_buffer: BufferId,
    solid_pipeline: RenderPipelineId,
    texture_pipeline: RenderPipelineId,
    glyph_pipeline: RenderPipelineId,
    sampler: SamplerId,
    buffer_textures: Vec<BufferTexture>,
    glyph_atlases: Vec<GlyphAtlas>,
    glyph_atlas_rebuild_required: bool,
    canvas_pipeline: RenderPipelineId,
    canvas_texture_pipeline: RenderPipelineId,
    canvas_depth_pipeline: Option<RenderPipelineId>,
    canvas_depth_texture_pipeline: Option<RenderPipelineId>,
    canvas_dummy_buffer: BufferId,
    canvas_targets: Vec<CanvasTarget>,
    canvas_meshes: Vec<CanvasMesh>,
    canvas_textures: Vec<CanvasTexture>,
    frame_serial: u64,
    width: u32,
    height: u32,
    supports_depth: bool,
}

impl SgfxPaintEncoder {
    /// Define the persistent logical resources for a two-slot encoder.
    ///
    /// # Arguments
    ///
    /// * `width` - Physical target width in pixels.
    /// * `height` - Physical target height in pixels.
    /// * `supports_depth` - Whether retained canvases may request depth testing.
    ///
    /// # Returns
    ///
    /// A logical encoder, or a lowering error for invalid dimensions or SGFX
    /// resource-definition failure.
    pub fn new(width: u32, height: u32, supports_depth: bool) -> Result<Self> {
        Self::with_target_count(width, height, supports_depth, 2)
    }

    /// Define persistent logical resources with an explicit target count.
    ///
    /// Presentation integrations that retain the currently displayed image
    /// may need three targets so rendering can continue while one image is
    /// displayed and another is pending at the compositor.
    ///
    /// # Arguments
    ///
    /// * `width` - Physical target width in pixels.
    /// * `height` - Physical target height in pixels.
    /// * `supports_depth` - Whether retained canvases may request depth testing.
    /// * `target_count` - Number of logical presentation targets to allocate.
    ///
    /// # Returns
    ///
    /// A logical encoder, or a lowering error for invalid dimensions, an empty
    /// target set, or SGFX resource-definition failure.
    pub fn with_target_count(
        width: u32,
        height: u32,
        supports_depth: bool,
        target_count: usize,
    ) -> Result<Self> {
        if width == 0 || height == 0 || target_count == 0 {
            return Err(Error::InvalidFrame);
        }
        let table = Rc::new(ResourceTable::new());
        let extent =
            Extent2D::new(width, height).map_err(|_| Error::sgfx(Stage::DefineResources))?;
        let target_usage = TextureUsage::RENDER_ATTACHMENT
            | TextureUsage::COPY_SRC
            | TextureUsage::COPY_DST
            | TextureUsage::PRESENT;

        let mut targets = Vec::new();
        targets
            .try_reserve_exact(target_count)
            .map_err(|_| Error::FrameTooComplex)?;
        for _ in 0..target_count {
            let target = table
                .define_texture(
                    TextureDesc::new(TextureFormat::Bgra8Unorm, extent, target_usage)
                        .map_err(|_| Error::sgfx(Stage::DefineResources))?,
                )
                .map_err(|_| Error::sgfx(Stage::DefineResources))?
                .id();
            targets.push(target);
        }

        let vertex_bytes = u64::try_from(MAX_FRAME_VERTICES)
            .ok()
            .and_then(|count| count.checked_mul(u64::from(PAINT_VERTEX_STRIDE)))
            .ok_or(Error::FrameTooComplex)?;
        let vertex_buffer = table
            .define_buffer(
                BufferDesc::new(vertex_bytes, BufferUsage::VERTEX | BufferUsage::COPY_DST)
                    .map_err(|_| Error::sgfx(Stage::DefineResources))?,
            )
            .map_err(|_| Error::sgfx(Stage::DefineResources))?
            .id();

        let solid_pipeline = define_colored_pipeline(&table, FragmentProgram::VertexColor)?.id();
        let texture_pipeline = define_colored_pipeline(
            &table,
            FragmentProgram::TextureVertexColor(TextureSampleMode::Rgba),
        )?
        .id();
        let glyph_pipeline = define_colored_pipeline(
            &table,
            FragmentProgram::TextureVertexColor(TextureSampleMode::AlphaMask),
        )?
        .id();
        let sampler = table
            .define_sampler(SamplerDesc::new(
                // Keep text, icons, and retained surfaces smooth when a
                // logical frame is sampled at a non-integer output scale.
                FilterMode::Linear,
                FilterMode::Linear,
                AddressMode::ClampToEdge,
                AddressMode::ClampToEdge,
            ))
            .map_err(|_| Error::sgfx(Stage::DefineResources))?
            .id();
        let glyph_atlas = GlyphAtlas::new(define_sampled_texture(
            &table,
            TextureFormat::R8Unorm,
            GLYPH_ATLAS_SIZE,
            GLYPH_ATLAS_SIZE,
        )?);
        let mut glyph_atlases = Vec::new();
        glyph_atlases
            .try_reserve_exact(1)
            .map_err(|_| Error::FrameTooComplex)?;
        glyph_atlases.push(glyph_atlas);
        let canvas_pipeline = define_canvas_pipeline(&table, false)?.id();
        let canvas_texture_pipeline = define_canvas_texture_pipeline(&table, false)?.id();
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

        Ok(Self {
            table,
            targets,
            vertex_buffer,
            solid_pipeline,
            texture_pipeline,
            glyph_pipeline,
            sampler,
            buffer_textures: Vec::new(),
            glyph_atlases,
            glyph_atlas_rebuild_required: false,
            canvas_pipeline,
            canvas_texture_pipeline,
            canvas_depth_pipeline: None,
            canvas_depth_texture_pipeline: None,
            canvas_dummy_buffer,
            canvas_targets: Vec::new(),
            canvas_meshes: Vec::new(),
            canvas_textures: Vec::new(),
            frame_serial: 0,
            width,
            height,
            supports_depth,
        })
    }

    /// Clone the resource table shared with a backend executor.
    ///
    /// # Returns
    ///
    /// Shared ownership of this encoder's logical resource table.
    pub fn resource_table(&self) -> Rc<ResourceTable> {
        Rc::clone(&self.table)
    }

    /// Return the logical presentation texture for a target slot.
    ///
    /// # Arguments
    ///
    /// * `slot` - Logical target slot, in the range selected at construction.
    ///
    /// # Returns
    ///
    /// The slot's texture identifier, or `None` for an invalid slot.
    pub fn target_texture(&self, slot: usize) -> Option<TextureId> {
        self.targets.get(slot).copied()
    }

    /// Return the physical width encoded into logical target resources.
    ///
    /// # Returns
    ///
    /// Target width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Return the physical height encoded into logical target resources.
    ///
    /// # Returns
    ///
    /// Target height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Encode and synchronously execute one ScarletUI frame.
    ///
    /// # Arguments
    ///
    /// * `executor` - Backend-owned executor supplied by the composition root
    ///   and bound to this encoder's resources.
    /// * `slot` - Logical destination target slot.
    /// * `copy_from` - Optional distinct source slot copied before painting.
    /// * `paint` - Backend-neutral paint commands and borrowed buffer data.
    /// * `background` - Straight-alpha background clear color.
    /// * `scale_milli` - Physical scale in milli-units.
    /// * `render_areas` - Physical `(x, y, width, height)` regions to redraw.
    ///
    /// # Returns
    ///
    /// Success after all ordered command buffers execute, a portable lowering
    /// error, or the executor's backend-owned error.
    pub fn encode_frame<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        slot: usize,
        copy_from: Option<usize>,
        paint: &PaintContext<'_>,
        background: UiColor,
        scale_milli: u32,
        render_areas: &[DamageRect],
    ) -> core::result::Result<(), FrameError<E::Error>> {
        let render_areas = self.validate_render_areas(render_areas)?;
        let target = *self.targets.get(slot).ok_or(Error::InvalidFrame)?;
        if let Some(source_slot) = copy_from {
            self.copy_target(executor, source_slot, slot)?;
        }
        let render_bounds = bounding_area(&render_areas).ok_or(Error::InvalidFrame)?;
        self.advance_frame_serial();
        self.prepare_canvases(executor, paint, scale_milli)?;
        let lowered = self.lower(paint, scale_milli, render_bounds)?;
        self.submit(executor, target, background, &render_areas, &lowered)
    }

    fn copy_target<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        source_slot: usize,
        destination_slot: usize,
    ) -> core::result::Result<(), FrameError<E::Error>> {
        if source_slot == destination_slot {
            return Err(FrameError::Lowering(Error::InvalidFrame));
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
        executor.execute(&commands).map_err(FrameError::Execution)
    }

    fn advance_frame_serial(&mut self) {
        self.frame_serial = self.frame_serial.wrapping_add(1);
        if self.frame_serial == 0 {
            self.frame_serial = 1;
            for texture in &mut self.buffer_textures {
                texture.used_frame = 0;
            }
            for atlas in &mut self.glyph_atlases {
                atlas.used_frame = 0;
            }
        }
    }

    fn validate_render_areas(&self, areas: &[DamageRect]) -> Result<Vec<PixelBounds>> {
        let mut validated = Vec::new();
        validated
            .try_reserve_exact(areas.len())
            .map_err(|_| Error::FrameTooComplex)?;
        for &(x, y, width, height) in areas {
            if width == 0 || height == 0 || x >= self.width || y >= self.height {
                continue;
            }
            let right = x.saturating_add(width).min(self.width);
            let bottom = y.saturating_add(height).min(self.height);
            validated.push(PixelBounds {
                x,
                y,
                width: right - x,
                height: bottom - y,
            });
        }
        if validated.is_empty() {
            Err(Error::InvalidFrame)
        } else {
            Ok(validated)
        }
    }

    fn lower<'frame>(
        &mut self,
        paint: &'frame PaintContext<'_>,
        scale_milli: u32,
        render_area: PixelBounds,
    ) -> Result<LoweredFrame<'frame>> {
        let mut buffer_textures_before = Vec::new();
        buffer_textures_before
            .try_reserve_exact(self.buffer_textures.len())
            .map_err(|_| Error::FrameTooComplex)?;
        buffer_textures_before.extend(self.buffer_textures.iter().copied());
        self.glyph_atlas_rebuild_required = false;
        match self.lower_once(paint, scale_milli, render_area) {
            Err(Error::FrameTooComplex) if self.glyph_atlas_rebuild_required => {
                // Cached atlas pages may contain glyphs from many older frames. Rebuild
                // once before reporting a genuinely over-complex current frame.
                for atlas in &mut self.glyph_atlases {
                    atlas.reset(self.frame_serial);
                }
                for (texture, previous) in self
                    .buffer_textures
                    .iter_mut()
                    .zip(buffer_textures_before.iter().copied())
                {
                    *texture = previous;
                }
                for texture in self
                    .buffer_textures
                    .iter_mut()
                    .skip(buffer_textures_before.len())
                {
                    // The first lowering pass never submitted this new texture. Keep
                    // the resource reusable, but force the retry to upload its pixels.
                    texture.upload_state = TextureUploadState::Pending;
                    texture.used_frame = 0;
                }
                self.glyph_atlas_rebuild_required = false;
                self.lower_once(paint, scale_milli, render_area)
            }
            result => result,
        }
    }

    fn lower_once<'frame>(
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
        let mut opacity = 1.0f32;
        let scale = scale_milli.max(1) as f32 / 1000.0;

        for command in paint.commands() {
            match command {
                PaintCommand::FillPath { path, color } => {
                    if let Some(geometry) = tessellator.fill_path(path)? {
                        push_draw(
                            &mut draws,
                            &mut tessellator,
                            geometry,
                            ui_color(*color, opacity)?,
                            DrawSource::Solid,
                        )?;
                    }
                }
                PaintCommand::FillRoundedRect {
                    rect,
                    corner_radius,
                    color,
                } => {
                    if let Some(geometry) = tessellator.fill_rounded_rect(*rect, *corner_radius)? {
                        push_draw(
                            &mut draws,
                            &mut tessellator,
                            geometry,
                            ui_color(*color, opacity)?,
                            DrawSource::Solid,
                        )?;
                    }
                }
                PaintCommand::FillVerticalGradientRoundedRect {
                    rect,
                    corner_radius,
                    top_color,
                    bottom_color,
                } => {
                    if !rect.size.height.is_finite() || rect.size.height <= 0.0 {
                        continue;
                    }
                    tessellator.push_clip(*rect, *corner_radius)?;
                    let band_height = rect.size.height / GRADIENT_BAND_COUNT as f32;
                    for index in 0..GRADIENT_BAND_COUNT {
                        let top = rect.origin.y + band_height * index as f32;
                        let bottom = if index + 1 == GRADIENT_BAND_COUNT {
                            rect.origin.y + rect.size.height
                        } else {
                            rect.origin.y + band_height * (index + 1) as f32
                        };
                        let band = scarlet_ui_core::geometry::Rect::from_xywh(
                            rect.origin.x,
                            top,
                            rect.size.width,
                            (bottom - top).max(0.0),
                        );
                        if let Some(geometry) = tessellator.fill_rounded_rect(band, 0.0)? {
                            let amount = (index as f32 + 0.5) / GRADIENT_BAND_COUNT as f32;
                            push_draw(
                                &mut draws,
                                &mut tessellator,
                                geometry,
                                ui_color(
                                    interpolate_ui_color(*top_color, *bottom_color, amount),
                                    opacity,
                                )?,
                                DrawSource::Solid,
                            )?;
                        }
                    }
                    tessellator.pop_clip();
                }
                PaintCommand::DrawRoundedRectShadow {
                    rect,
                    corner_radius,
                    offset,
                    blur_radius,
                    spread_radius,
                    color,
                } => {
                    if ![
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                        *corner_radius,
                        offset.dx,
                        offset.dy,
                        *blur_radius,
                        *spread_radius,
                    ]
                    .iter()
                    .all(|value| value.is_finite())
                    {
                        return Err(Error::InvalidFrame);
                    }
                    let blur = blur_radius.max(0.0);
                    for (index, weight) in SHADOW_LAYER_WEIGHTS.iter().enumerate() {
                        let distance = if SHADOW_LAYER_COUNT > 1 {
                            (SHADOW_LAYER_COUNT - index - 1) as f32
                                / (SHADOW_LAYER_COUNT - 1) as f32
                        } else {
                            0.0
                        };
                        let expansion = *spread_radius + blur * distance;
                        let shadow_rect = scarlet_ui_core::geometry::Rect::from_xywh(
                            rect.origin.x + offset.dx - expansion,
                            rect.origin.y + offset.dy - expansion,
                            rect.size.width + expansion * 2.0,
                            rect.size.height + expansion * 2.0,
                        );
                        let radius = (*corner_radius + expansion).max(0.0);
                        if let Some(geometry) =
                            tessellator.fill_rounded_rect(shadow_rect, radius)?
                        {
                            let layer_color = color.with_opacity(color.a * *weight);
                            push_draw(
                                &mut draws,
                                &mut tessellator,
                                geometry,
                                ui_color(layer_color, opacity)?,
                                DrawSource::Solid,
                            )?;
                        }
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
                            &mut tessellator,
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
                    if let Some(geometry) = tessellator.stroke_rect(*rect, 0.0, *stroke_width)? {
                        push_draw(
                            &mut draws,
                            &mut tessellator,
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
                            &mut tessellator,
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
                        let (texture, atlas_bounds, upload_required) =
                            self.glyph_texture(glyph.key, glyph.width, glyph.height)?;
                        if upload_required
                            && !texture_upload_scheduled(&uploads, texture, atlas_bounds)
                        {
                            uploads.try_reserve(1).map_err(|_| Error::FrameTooComplex)?;
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
                        if let Some(geometry) = tessellator
                            .textured_rect(destination, atlas_tex_coords(atlas_bounds))?
                        {
                            push_draw(
                                &mut draws,
                                &mut tessellator,
                                geometry,
                                color,
                                DrawSource::Glyph(texture),
                            )?;
                        }
                    }
                }
                PaintCommand::DrawIcon {
                    rect,
                    icon,
                    style,
                    color,
                } => {
                    if !rect.origin.x.is_finite()
                        || !rect.origin.y.is_finite()
                        || !rect.size.width.is_finite()
                        || !rect.size.height.is_finite()
                    {
                        return Err(Error::InvalidFrame);
                    }
                    let pixel_size =
                        libm::ceilf(rect.size.width.min(rect.size.height).max(1.0) * scale)
                            .min(u16::MAX as f32) as u16;
                    let raster = rasterize_icon(*icon, pixel_size, *style);
                    let (texture, atlas_bounds, upload_required) =
                        self.icon_texture(raster.key, raster.width, raster.height)?;
                    if upload_required && !texture_upload_scheduled(&uploads, texture, atlas_bounds)
                    {
                        uploads.try_reserve(1).map_err(|_| Error::FrameTooComplex)?;
                        uploads.push(TextureUpload {
                            texture,
                            x: atlas_bounds.x,
                            y: atlas_bounds.y,
                            width: raster.width,
                            height: raster.height,
                            bytes_per_row: raster.width,
                            bytes: UploadBytes::Shared(raster.mask),
                        });
                    }
                    let destination = FloatRect::new(
                        truncated_scaled(rect.origin.x, scale),
                        truncated_scaled(rect.origin.y, scale),
                        raster.width as f32,
                        raster.height as f32,
                    );
                    if let Some(geometry) =
                        tessellator.textured_rect(destination, atlas_tex_coords(atlas_bounds))?
                    {
                        push_draw(
                            &mut draws,
                            &mut tessellator,
                            geometry,
                            ui_color(*color, opacity)?,
                            DrawSource::Glyph(texture),
                        )?;
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
                            &mut tessellator,
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
                            &mut tessellator,
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
                PaintCommand::SetOpacity {
                    opacity: next_opacity,
                } => {
                    opacity = finite_unit(*next_opacity)?;
                }
                PaintCommand::Extension { rect, payload } => {
                    let Some(canvas) = payload.as_ref().as_any().downcast_ref::<SgfxCanvasPaint>()
                    else {
                        continue;
                    };
                    let canvas_width = physical_canvas_extent(rect.size.width, scale)?;
                    let canvas_height = physical_canvas_extent(rect.size.height, scale)?;
                    let Some(texture) = self.canvas_targets.iter().find(|target| {
                        target.handle_id == canvas.handle.id()
                            && target.width == canvas_width
                            && target.height == canvas_height
                            && target.depth.is_some() == canvas.frame.depth_test
                            && target.initialized
                    }) else {
                        continue;
                    };
                    let destination = FloatRect::new(
                        truncated_scaled(rect.origin.x, scale),
                        truncated_scaled(rect.origin.y, scale),
                        truncated_scaled(rect.size.width, scale),
                        truncated_scaled(rect.size.height, scale),
                    );
                    if let Some(geometry) =
                        tessellator.textured_rect(destination, CANVAS_TARGET_TEX_COORDS)?
                    {
                        push_draw(
                            &mut draws,
                            &mut tessellator,
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
        push_draw(
            &mut draws,
            &mut tessellator,
            geometry,
            [0.0, 0.0, 0.0, 0.0],
            DrawSource::Solid,
        )?;
        let vertex_bytes = encode_paint_vertices(tessellator.vertices())?;
        Ok(LoweredFrame {
            vertex_bytes,
            draws,
            uploads,
        })
    }

    fn prepare_canvases<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        paint: &PaintContext<'_>,
        scale_milli: u32,
    ) -> core::result::Result<(), FrameError<E::Error>> {
        let scale = scale_milli.max(1) as f32 / 1000.0;
        for command in paint.commands() {
            let PaintCommand::Extension { rect, payload } = command else {
                continue;
            };
            let Some(canvas) = payload.as_ref().as_any().downcast_ref::<SgfxCanvasPaint>() else {
                continue;
            };
            let width = physical_canvas_extent(rect.size.width, scale)?;
            let height = physical_canvas_extent(rect.size.height, scale)?;
            let target_index =
                self.canvas_target(canvas.handle.id(), width, height, canvas.frame.depth_test)?;
            let unchanged = {
                let target = &self.canvas_targets[target_index];
                target.initialized && target.revision == canvas.frame.revision
            };
            if unchanged {
                continue;
            }
            self.render_canvas(executor, target_index, &canvas.frame)?;
            let target = &mut self.canvas_targets[target_index];
            target.revision = canvas.frame.revision;
            target.initialized = true;
        }
        Ok(())
    }

    fn canvas_target(
        &mut self,
        handle_id: u64,
        width: u32,
        height: u32,
        depth_test: bool,
    ) -> Result<usize> {
        if let Some(index) = self.canvas_targets.iter().position(|target| {
            target.handle_id == handle_id
                && target.width == width
                && target.height == height
                && target.depth.is_some() == depth_test
        }) {
            return Ok(index);
        }
        validate_depth_support(depth_test, self.supports_depth)?;
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
        let depth = if depth_test {
            Some(
                self.table
                    .define_texture(
                        TextureDesc::new(
                            TextureFormat::Depth32Float,
                            extent,
                            TextureUsage::RENDER_ATTACHMENT,
                        )
                        .map_err(|_| Error::sgfx(Stage::DefineResources))?,
                    )
                    .map_err(|_| Error::sgfx(Stage::DefineResources))?
                    .id(),
            )
        } else {
            None
        };
        self.canvas_targets.push(CanvasTarget {
            handle_id,
            texture,
            depth,
            width,
            height,
            revision: 0,
            initialized: false,
        });
        Ok(self.canvas_targets.len() - 1)
    }

    fn canvas_mesh(&mut self, mesh: &SgfxMesh) -> Result<usize> {
        if mesh.vertices.is_empty() || !mesh.vertices.len().is_multiple_of(3) {
            return Err(Error::FrameTooComplex);
        }
        if !mesh.vertices.iter().all(|vertex| {
            vertex.position.iter().all(|value| value.is_finite())
                && vertex.color.iter().all(|value| value.is_finite())
                && vertex.tex_coord.iter().all(|value| value.is_finite())
        }) {
            return Err(Error::InvalidFrame);
        }
        let vertex_count =
            u32::try_from(mesh.vertices.len()).map_err(|_| Error::FrameTooComplex)?;
        if let Some(index) = self
            .canvas_meshes
            .iter()
            .position(|cached| cached.handle_id == mesh.handle.id())
        {
            let action = canvas_mesh_cache_action(
                self.canvas_meshes[index].revision,
                self.canvas_meshes[index].capacity_vertices,
                mesh.revision,
                vertex_count,
            )?;
            if action == CanvasMeshCacheAction::Reuse {
                return Ok(index);
            }
            if let CanvasMeshCacheAction::Reallocate(capacity_vertices) = action {
                let byte_size = u64::from(capacity_vertices)
                    .checked_mul(u64::from(CANVAS_VERTEX_STRIDE))
                    .ok_or(Error::FrameTooComplex)?;
                self.canvas_meshes[index].buffer = self
                    .table
                    .define_buffer(
                        BufferDesc::new(byte_size, BufferUsage::VERTEX | BufferUsage::COPY_DST)
                            .map_err(|_| Error::sgfx(Stage::DefineResources))?,
                    )
                    .map_err(|_| Error::sgfx(Stage::DefineResources))?
                    .id();
                self.canvas_meshes[index].capacity_vertices = capacity_vertices;
            }
            self.canvas_meshes[index].revision = mesh.revision;
            self.canvas_meshes[index].vertex_count = vertex_count;
            self.canvas_meshes[index].uploaded = false;
            return Ok(index);
        }
        if self.canvas_meshes.len() >= MAX_CANVAS_MESHES {
            return Err(Error::FrameTooComplex);
        }
        let capacity_vertices = vertex_count;
        let byte_size = u64::from(capacity_vertices)
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
            handle_id: mesh.handle.id(),
            revision: mesh.revision,
            buffer,
            vertex_count,
            capacity_vertices,
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

    fn render_canvas<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        target_index: usize,
        frame: &SgfxCanvasFrame,
    ) -> core::result::Result<(), FrameError<E::Error>> {
        if frame.draws.len() > MAX_CANVAS_DRAWS {
            return Err(FrameError::Lowering(Error::FrameTooComplex));
        }
        if canvas_frame_has_revision_conflict(frame) {
            return Err(FrameError::Lowering(Error::InvalidFrame));
        }
        if frame.depth_test {
            self.ensure_canvas_depth_pipelines()?;
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
        let color_pipeline_id = if frame.depth_test {
            self.canvas_depth_pipeline.ok_or(Error::InvalidFrame)?
        } else {
            self.canvas_pipeline
        };
        let texture_pipeline_id = if frame.depth_test {
            self.canvas_depth_texture_pipeline
                .ok_or(Error::InvalidFrame)?
        } else {
            self.canvas_texture_pipeline
        };
        let color_pipeline = table
            .render_pipeline_ref(color_pipeline_id)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let texture_pipeline = table
            .render_pipeline_ref(texture_pipeline_id)
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let depth = target_state
            .depth
            .map(|depth| table.texture_ref(depth))
            .transpose()
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
            let dummy_vertices = canvas_dummy_vertices();
            let dummy_bytes = encode_canvas_vertex_slice(&dummy_vertices)?;
            let dummy = table
                .buffer_ref(self.canvas_dummy_buffer)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            encoder
                .write_buffer(dummy, 0, &dummy_bytes)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            let descriptor =
                RenderPassDesc::new(&table, target, area, LoadOp::Clear(clear), StoreOp::Store)
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            let descriptor = if let Some(depth) = depth {
                descriptor
                    .with_depth_attachment(
                        &table,
                        depth,
                        DepthLoadOp::Clear(1.0),
                        StoreOp::DontCare,
                    )
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?
            } else {
                descriptor
            };
            let mut pass = encoder
                .begin_render_pass(descriptor)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            pass.set_pipeline(color_pipeline)
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
            pass.end().map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            let commands = encoder
                .finish()
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            executor.execute(&commands).map_err(FrameError::Execution)?;
        } else {
            let mut draw_index = 0usize;
            let mut first_submission = true;
            while draw_index < frame.draws.len() {
                let mut encoder = CommandEncoder::new(&table);
                let prefix_commands = if first_submission {
                    uploads.len().saturating_add(texture_uploads.len())
                } else {
                    0
                };
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
                        let destination =
                            PixelRect::new(0, 0, cached.source.width, cached.source.height)
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
                let depth_store =
                    if canvas_pass_reaches_frame_end(&mesh_indices, draw_index, prefix_commands) {
                        StoreOp::DontCare
                    } else {
                        StoreOp::Store
                    };
                let descriptor = RenderPassDesc::new(&table, target, area, load, StoreOp::Store)
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                let descriptor = if let Some(depth) = depth {
                    let depth_load = if first_submission {
                        DepthLoadOp::Clear(1.0)
                    } else {
                        DepthLoadOp::Load
                    };
                    descriptor
                        .with_depth_attachment(&table, depth, depth_load, depth_store)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?
                } else {
                    descriptor
                };
                let mut pass = encoder
                    .begin_render_pass(descriptor)
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                let mut command_count = prefix_commands.saturating_add(PASS_COMMANDS);

                while draw_index < frame.draws.len() {
                    let draw = &frame.draws[draw_index];
                    let mesh_index = mesh_indices[draw_index];
                    let texture_index = texture_indices[draw_index];
                    let cached = &self.canvas_meshes[mesh_index];
                    if command_count.saturating_add(MAX_CANVAS_DRAW_COMMANDS) > MAX_COMMANDS {
                        break;
                    }

                    let buffer = table
                        .buffer_ref(cached.buffer)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    let transform = Transform::from_columns(canvas_transform(
                        draw.transform,
                        frame.reference_aspect,
                        target_state.width,
                        target_state.height,
                    )?)
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
                    pass.draw(cached.vertex_count, 0)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;

                    command_count = command_count.saturating_add(MAX_CANVAS_DRAW_COMMANDS);
                    draw_index += 1;
                }
                if command_count == prefix_commands.saturating_add(PASS_COMMANDS) {
                    return Err(Error::FrameTooComplex.into());
                }
                pass.end().map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                let commands = encoder
                    .finish()
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                executor.execute(&commands).map_err(FrameError::Execution)?;
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

    fn ensure_canvas_depth_pipelines(&mut self) -> Result<()> {
        validate_depth_support(true, self.supports_depth)?;
        if self.canvas_depth_pipeline.is_none() {
            self.canvas_depth_pipeline = Some(define_canvas_pipeline(&self.table, true)?.id());
        }
        if self.canvas_depth_texture_pipeline.is_none() {
            self.canvas_depth_texture_pipeline =
                Some(define_canvas_texture_pipeline(&self.table, true)?.id());
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
            let upload_required =
                texture.revision != revision || texture.upload_state == TextureUploadState::Pending;
            if texture.revision != revision {
                texture.upload_state = TextureUploadState::Pending;
            }
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
            texture.upload_state = TextureUploadState::Pending;
            return Ok((texture.texture, true));
        }
        if self.buffer_textures.len() >= MAX_BUFFER_TEXTURES {
            return Err(Error::FrameTooComplex);
        }
        let texture =
            define_sampled_texture(&self.table, TextureFormat::Bgra8Unorm, width, height)?;
        self.buffer_textures.push(BufferTexture {
            texture,
            buffer_identity,
            revision,
            width,
            height,
            used_frame: self.frame_serial,
            upload_state: TextureUploadState::Pending,
        });
        Ok((texture, true))
    }

    fn glyph_texture(
        &mut self,
        key: GlyphRasterKey,
        width: u32,
        height: u32,
    ) -> Result<(TextureId, PixelBounds, bool)> {
        for atlas in &mut self.glyph_atlases {
            if let Some(entry) = atlas
                .entries
                .iter()
                .find(|entry| entry.key == key && entry.width == width && entry.height == height)
            {
                let bounds = PixelBounds {
                    x: entry.x,
                    y: entry.y,
                    width: entry.width,
                    height: entry.height,
                };
                atlas.used_frame = self.frame_serial;
                return Ok((
                    atlas.texture,
                    bounds,
                    entry.upload_state == TextureUploadState::Pending,
                ));
            }
        }

        let atlas_index = self.glyph_atlas_for_insert(AtlasEntryKind::Glyph, width, height)?;
        let atlas = &mut self.glyph_atlases[atlas_index];
        atlas
            .entries
            .try_reserve(1)
            .map_err(|_| Error::FrameTooComplex)?;
        let Some(bounds) = atlas.allocate(AtlasEntryKind::Glyph, width, height) else {
            self.glyph_atlas_rebuild_required = true;
            return Err(Error::FrameTooComplex);
        };
        atlas.entries.push(GlyphTexture {
            key,
            x: bounds.x,
            y: bounds.y,
            width,
            height,
            upload_state: TextureUploadState::Pending,
        });
        Ok((atlas.texture, bounds, true))
    }

    fn icon_texture(
        &mut self,
        key: IconMaskKey,
        width: u32,
        height: u32,
    ) -> Result<(TextureId, PixelBounds, bool)> {
        for atlas in &mut self.glyph_atlases {
            if let Some(entry) = atlas
                .icon_entries
                .iter()
                .find(|entry| entry.key == key && entry.width == width && entry.height == height)
            {
                let bounds = PixelBounds {
                    x: entry.x,
                    y: entry.y,
                    width: entry.width,
                    height: entry.height,
                };
                atlas.used_frame = self.frame_serial;
                return Ok((
                    atlas.texture,
                    bounds,
                    entry.upload_state == TextureUploadState::Pending,
                ));
            }
        }

        let atlas_index = self.glyph_atlas_for_insert(AtlasEntryKind::Icon, width, height)?;
        let atlas = &mut self.glyph_atlases[atlas_index];
        atlas
            .icon_entries
            .try_reserve(1)
            .map_err(|_| Error::FrameTooComplex)?;
        let Some(bounds) = atlas.allocate(AtlasEntryKind::Icon, width, height) else {
            self.glyph_atlas_rebuild_required = true;
            return Err(Error::FrameTooComplex);
        };
        atlas.icon_entries.push(IconTexture {
            key,
            x: bounds.x,
            y: bounds.y,
            width,
            height,
            upload_state: TextureUploadState::Pending,
        });
        Ok((atlas.texture, bounds, true))
    }

    fn glyph_atlas_for_insert(
        &mut self,
        kind: AtlasEntryKind,
        width: u32,
        height: u32,
    ) -> Result<usize> {
        let action = glyph_atlas_cache_action(
            &self.glyph_atlases,
            self.frame_serial,
            kind,
            width,
            height,
            MAX_GLYPH_ATLASES,
        );
        let Some(action) = action else {
            self.glyph_atlas_rebuild_required = true;
            return Err(Error::FrameTooComplex);
        };
        match action {
            GlyphAtlasCacheAction::Append(index) => Ok(index),
            GlyphAtlasCacheAction::Recycle(index) => {
                self.glyph_atlases[index].reset(self.frame_serial);
                Ok(index)
            }
            GlyphAtlasCacheAction::Create => {
                self.glyph_atlases
                    .try_reserve(1)
                    .map_err(|_| Error::FrameTooComplex)?;
                let texture = define_sampled_texture(
                    &self.table,
                    TextureFormat::R8Unorm,
                    GLYPH_ATLAS_SIZE,
                    GLYPH_ATLAS_SIZE,
                )?;
                let mut atlas = GlyphAtlas::new(texture);
                atlas.reset(self.frame_serial);
                self.glyph_atlases.push(atlas);
                Ok(self.glyph_atlases.len() - 1)
            }
        }
    }

    fn submit<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        target_id: TextureId,
        background: UiColor,
        render_areas: &[PixelBounds],
        frame: &LoweredFrame<'_>,
    ) -> core::result::Result<(), FrameError<E::Error>> {
        if frame.draws.is_empty() {
            return Ok(());
        }
        self.submit_texture_uploads(executor, frame)?;

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
        let white = ir_color([1.0, 1.0, 1.0, 1.0])?;

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
            let mut first_pass = true;
            while draw_index < frame.draws.len() {
                // Each draw remains intact. Split only to respect SGFX IR's
                // fixed command capacity, preserving order with LoadOp::Load.
                let mut encoder = CommandEncoder::new(&table);
                if first_submission {
                    encoder
                        .write_buffer(vertex_buffer, 0, &frame.vertex_bytes)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
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
                let initial_command_count = PASS_COMMANDS + usize::from(first_submission);
                let mut command_count = initial_command_count;

                while draw_index < frame.draws.len() {
                    let draw = frame.draws[draw_index];
                    let Some(scissor) = intersect_bounds(draw.geometry.scissor, *render_area)
                    else {
                        draw_index += 1;
                        continue;
                    };
                    if command_count.saturating_add(MAX_PAINT_DRAW_COMMANDS) > MAX_COMMANDS {
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
                    pass.set_uniforms(DrawUniforms::new(transform, white))
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    let scissor =
                        PixelRect::new(scissor.x, scissor.y, scissor.width, scissor.height)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    pass.set_scissor(Some(scissor))
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    pass.draw(draw.geometry.vertex_count, draw.geometry.first_vertex)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    command_count = command_count.saturating_add(MAX_PAINT_DRAW_COMMANDS);
                    draw_index += 1;
                }
                if command_count == initial_command_count {
                    return Err(Error::FrameTooComplex.into());
                }
                pass.end().map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                let commands = encoder
                    .finish()
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                executor.execute(&commands).map_err(FrameError::Execution)?;
                first_submission = false;
                first_pass = false;
            }
        }
        Ok(())
    }

    fn submit_texture_uploads<E: CommandExecutor>(
        &mut self,
        executor: &mut E,
        frame: &LoweredFrame<'_>,
    ) -> core::result::Result<(), FrameError<E::Error>> {
        if frame.uploads.is_empty() {
            return Ok(());
        }
        let table = Rc::clone(&self.table);
        let mut encoder = CommandEncoder::new(&table);
        for upload in &frame.uploads {
            let texture = table
                .texture_ref(upload.texture)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            let destination = PixelRect::new(upload.x, upload.y, upload.width, upload.height)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            let write =
                TextureWrite::new(destination, upload.bytes_per_row, upload.bytes.as_slice())
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            encoder
                .write_texture(texture, write)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        }
        let commands = encoder
            .finish()
            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        executor.execute(&commands).map_err(FrameError::Execution)?;
        self.commit_texture_uploads(frame);
        Ok(())
    }

    fn commit_texture_uploads(&mut self, frame: &LoweredFrame<'_>) {
        for upload in &frame.uploads {
            for texture in &mut self.buffer_textures {
                if texture.texture == upload.texture
                    && upload.x == 0
                    && upload.y == 0
                    && texture.width == upload.width
                    && texture.height == upload.height
                {
                    texture.upload_state = TextureUploadState::Uploaded;
                }
            }
            for atlas in &mut self.glyph_atlases {
                if atlas.texture != upload.texture {
                    continue;
                }
                if let Some(entry) = atlas.entries.iter_mut().find(|entry| {
                    entry.x == upload.x
                        && entry.y == upload.y
                        && entry.width == upload.width
                        && entry.height == upload.height
                }) {
                    entry.upload_state = TextureUploadState::Uploaded;
                }
                if let Some(entry) = atlas.icon_entries.iter_mut().find(|entry| {
                    entry.x == upload.x
                        && entry.y == upload.y
                        && entry.width == upload.width
                        && entry.height == upload.height
                }) {
                    entry.upload_state = TextureUploadState::Uploaded;
                }
            }
        }
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

fn define_colored_pipeline(
    table: &ResourceTable,
    fragment: FragmentProgram,
) -> Result<sgfx::ir::RenderPipelineRef<'_>> {
    let attributes = match fragment {
        FragmentProgram::VertexColor => alloc::vec![
            VertexAttribute::new(0, VertexFormat::Float32x4, 0),
            VertexAttribute::new(1, VertexFormat::Float32x4, 16),
        ],
        FragmentProgram::TextureVertexColor(
            TextureSampleMode::Rgba | TextureSampleMode::AlphaMask,
        ) => alloc::vec![
            VertexAttribute::new(0, VertexFormat::Float32x4, 0),
            VertexAttribute::new(1, VertexFormat::Float32x4, 16),
            VertexAttribute::new(2, VertexFormat::Float32x2, 32),
        ],
        _ => return Err(Error::InvalidFrame),
    };
    let layout = VertexBufferLayout::new(PAINT_VERTEX_STRIDE, attributes)
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
    depth_test: bool,
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
    let descriptor = if depth_test {
        descriptor
            .with_depth_stencil(DepthState::new(
                TextureFormat::Depth32Float,
                CompareFunction::Less,
                true,
            ))
            .map_err(|_| Error::sgfx(Stage::DefineResources))?
    } else {
        descriptor
    };
    table
        .define_render_pipeline(descriptor)
        .map_err(|_| Error::sgfx(Stage::DefineResources))
}

fn define_canvas_texture_pipeline(
    table: &ResourceTable,
    depth_test: bool,
) -> Result<sgfx::ir::RenderPipelineRef<'_>> {
    let layout = VertexBufferLayout::new(
        CANVAS_VERTEX_STRIDE,
        alloc::vec![
            VertexAttribute::new(0, VertexFormat::Float32x4, 0),
            VertexAttribute::new(1, VertexFormat::Float32x4, 16),
            VertexAttribute::new(2, VertexFormat::Float32x2, 32),
        ],
    )
    .map_err(|_| Error::sgfx(Stage::DefineResources))?;
    let descriptor = RenderPipelineDesc::new(
        TextureFormat::Bgra8Unorm,
        PrimitiveTopology::TriangleList,
        layout,
        FragmentProgram::TextureVertexColor(TextureSampleMode::Rgba),
        BlendState::SOURCE_OVER_STRAIGHT_ALPHA,
        RasterState::new(sgfx::ir::CullMode::Back, FrontFace::CounterClockwise),
    )
    .map_err(|_| Error::sgfx(Stage::DefineResources))?;
    let descriptor = if depth_test {
        descriptor
            .with_depth_stencil(DepthState::new(
                TextureFormat::Depth32Float,
                CompareFunction::Less,
                true,
            ))
            .map_err(|_| Error::sgfx(Stage::DefineResources))?
    } else {
        descriptor
    };
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
    tessellator: &mut Tessellator,
    geometry: GeometryRange,
    color: [f32; 4],
    source: DrawSource,
) -> Result<()> {
    tessellator.color_geometry(geometry, color)?;
    if let Some(previous) = draws.last_mut() {
        let previous_end = previous
            .geometry
            .first_vertex
            .checked_add(previous.geometry.vertex_count);
        if previous.source == source
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
    draws.try_reserve(1).map_err(|_| Error::FrameTooComplex)?;
    draws.push(Draw { geometry, source });
    Ok(())
}

fn atlas_tex_coords(bounds: PixelBounds) -> [[f32; 2]; 4] {
    let inverse_size = 1.0 / GLYPH_ATLAS_SIZE as f32;
    let left = bounds.x as f32 * inverse_size;
    let top = bounds.y as f32 * inverse_size;
    let right = bounds.x.saturating_add(bounds.width) as f32 * inverse_size;
    let bottom = bounds.y.saturating_add(bounds.height) as f32 * inverse_size;
    [[left, top], [right, top], [right, bottom], [left, bottom]]
}

fn encode_paint_vertices(vertices: &[Vertex]) -> Result<Vec<u8>> {
    let capacity = vertices
        .len()
        .checked_mul(PAINT_VERTEX_STRIDE as usize)
        .ok_or(Error::FrameTooComplex)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| Error::FrameTooComplex)?;
    for vertex in vertices {
        bytes.extend_from_slice(&vertex.position[0].to_le_bytes());
        bytes.extend_from_slice(&vertex.position[1].to_le_bytes());
        bytes.extend_from_slice(&0.0f32.to_le_bytes());
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        for component in vertex.color {
            bytes.extend_from_slice(&component.clamp(0.0, 1.0).to_le_bytes());
        }
        bytes.extend_from_slice(&vertex.tex_coord[0].to_le_bytes());
        bytes.extend_from_slice(&vertex.tex_coord[1].to_le_bytes());
    }
    debug_assert_eq!(bytes.len(), capacity);
    Ok(bytes)
}

fn encode_canvas_vertices(mesh: &SgfxMesh) -> Result<Vec<u8>> {
    encode_canvas_vertex_slice(&mesh.vertices)
}

fn encode_canvas_vertex_slice(vertices: &[SgfxCanvasVertex]) -> Result<Vec<u8>> {
    let capacity = vertices
        .len()
        .checked_mul(CANVAS_VERTEX_STRIDE as usize)
        .ok_or(Error::FrameTooComplex)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| Error::FrameTooComplex)?;
    for vertex in vertices {
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

fn canvas_dummy_vertices() -> [SgfxCanvasVertex; 3] {
    [SgfxCanvasVertex::new([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 0.0]); 3]
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

fn canvas_transform(
    mut transform: [f32; 16],
    reference_aspect: f32,
    width: u32,
    height: u32,
) -> Result<[f32; 16]> {
    if !reference_aspect.is_finite() || reference_aspect <= 0.0 || width == 0 || height == 0 {
        return Err(Error::InvalidFrame);
    }
    let target_aspect = width as f32 / height as f32;
    let horizontal_scale = reference_aspect / target_aspect;
    if !horizontal_scale.is_finite() || horizontal_scale <= 0.0 {
        return Err(Error::InvalidFrame);
    }
    for index in [0usize, 4, 8, 12] {
        transform[index] *= horizontal_scale;
    }
    if !transform.iter().all(|component| component.is_finite()) {
        return Err(Error::InvalidFrame);
    }
    Ok(transform)
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

fn interpolate_ui_color(start: UiColor, end: UiColor, amount: f32) -> UiColor {
    let amount = amount.clamp(0.0, 1.0);
    UiColor {
        r: start.r + (end.r - start.r) * amount,
        g: start.g + (end.g - start.g) * amount,
        b: start.b + (end.b - start.b) * amount,
        a: start.a + (end.a - start.a) * amount,
    }
}

fn ir_color(components: [f32; 4]) -> Result<Color> {
    Color::rgba(components[0], components[1], components[2], components[3])
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{SgfxCanvasDraw, SgfxCanvasVertex, SgfxMeshHandle};
    use scarlet_ui_core::geometry::{Offset, Point, Rect, Size};
    use scarlet_ui_core::icon::{ALL_ICONS, IconStyle};
    use sgfx::ir::{Command, CommandBuffer};

    #[derive(Default)]
    struct RecordingExecutor {
        command_kinds: Vec<Vec<&'static str>>,
        draw_vertices: Vec<u32>,
    }

    impl CommandExecutor for RecordingExecutor {
        type Error = ();

        fn execute<'r, 'data>(
            &mut self,
            commands: &CommandBuffer<'r, 'data>,
        ) -> core::result::Result<(), Self::Error> {
            let mut kinds = Vec::new();
            for command in commands.commands() {
                let kind = match command {
                    Command::WriteBuffer { .. } => "write-buffer",
                    Command::WriteTexture { .. } => "write-texture",
                    Command::CopyTextureToTexture { .. } => "copy",
                    Command::BeginRenderPass(_) => "begin-pass",
                    Command::EndRenderPass => "end-pass",
                    Command::SetPipeline(_) => "set-pipeline",
                    Command::SetVertexBuffer { .. } => "set-vertex-buffer",
                    Command::SetIndexBuffer { .. } => "set-index-buffer",
                    Command::SetTexture(_) => "set-texture",
                    Command::SetSampler(_) => "set-sampler",
                    Command::SetUniforms(_) => "set-uniforms",
                    Command::SetScissor(_) => "set-scissor",
                    Command::Draw { vertex_count, .. } => {
                        self.draw_vertices.push(*vertex_count);
                        "draw"
                    }
                    Command::DrawIndexed { .. } => "draw-indexed",
                };
                kinds.push(kind);
            }
            self.command_kinds.push(kinds);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FailOnceExecutor {
        fail_next: bool,
        texture_write_counts: Vec<usize>,
    }

    impl CommandExecutor for FailOnceExecutor {
        type Error = &'static str;

        fn execute<'r, 'data>(
            &mut self,
            commands: &CommandBuffer<'r, 'data>,
        ) -> core::result::Result<(), Self::Error> {
            self.texture_write_counts.push(
                commands
                    .commands()
                    .iter()
                    .filter(|command| matches!(command, Command::WriteTexture { .. }))
                    .count(),
            );
            if self.fail_next {
                self.fail_next = false;
                Err("injected upload failure")
            } else {
                Ok(())
            }
        }
    }

    fn test_glyph_atlas(table: &ResourceTable, used_frame: u64) -> GlyphAtlas {
        let texture = define_sampled_texture(
            table,
            TextureFormat::R8Unorm,
            GLYPH_ATLAS_SIZE,
            GLYPH_ATLAS_SIZE,
        )
        .unwrap();
        let mut atlas = GlyphAtlas::new(texture);
        atlas.used_frame = used_frame;
        atlas
    }

    fn fill_atlas_shelf(atlas: &mut GlyphAtlas) {
        atlas.cursor_x = 0;
        atlas.cursor_y = GLYPH_ATLAS_SIZE;
        atlas.row_height = 0;
    }

    fn triangle(z: f32) -> Vec<SgfxCanvasVertex> {
        alloc::vec![
            SgfxCanvasVertex::new([0.0, 0.0, z, 1.0], [1.0; 4]),
            SgfxCanvasVertex::new([1.0, 0.0, z, 1.0], [1.0; 4]),
            SgfxCanvasVertex::new([0.0, 1.0, z, 1.0], [1.0; 4]),
        ]
    }

    fn encoded_paint_colors(bytes: &[u8]) -> Vec<[f32; 4]> {
        bytes
            .chunks_exact(PAINT_VERTEX_STRIDE as usize)
            .map(|vertex| {
                let component =
                    |offset| f32::from_le_bytes(vertex[offset..offset + 4].try_into().unwrap());
                [component(16), component(20), component(24), component(28)]
            })
            .collect()
    }

    #[test]
    fn empty_canvas_dummy_buffer_contains_three_valid_vertices() {
        let vertices = canvas_dummy_vertices();
        let bytes = encode_canvas_vertex_slice(&vertices).unwrap();

        assert_eq!(bytes.len(), CANVAS_VERTEX_STRIDE as usize * 3);
        assert!(vertices.iter().all(|vertex| {
            vertex.position.iter().all(|value| value.is_finite())
                && vertex
                    .color
                    .iter()
                    .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
                && vertex.tex_coord.iter().all(|value| value.is_finite())
        }));
    }

    #[test]
    fn mesh_cache_reuses_updates_and_grows_to_power_of_two() {
        assert_eq!(
            canvas_mesh_cache_action(3, 8, 3, 8).unwrap(),
            CanvasMeshCacheAction::Reuse
        );
        assert_eq!(
            canvas_mesh_cache_action(3, 8, 4, 3).unwrap(),
            CanvasMeshCacheAction::Upload
        );
        assert_eq!(
            canvas_mesh_cache_action(4, 8, 5, 9).unwrap(),
            CanvasMeshCacheAction::Reallocate(16)
        );
        assert_eq!(
            canvas_mesh_cache_action(5, 16, 6, 3).unwrap(),
            CanvasMeshCacheAction::Upload
        );
    }

    #[test]
    fn glyph_atlas_reset_discards_history_and_restarts_shelf() {
        let table = ResourceTable::new();
        let mut atlas = test_glyph_atlas(&table, 3);
        let first = atlas
            .allocate(
                AtlasEntryKind::Glyph,
                GLYPH_ATLAS_SIZE - GLYPH_ATLAS_PADDING,
                10,
            )
            .unwrap();
        let second = atlas.allocate(AtlasEntryKind::Glyph, 8, 6).unwrap();
        assert_eq!((first.x, first.y), (0, 0));
        assert_eq!((second.x, second.y), (0, 11));
        atlas.entries.push(GlyphTexture {
            key: GlyphRasterKey {
                codepoint: 'A' as u32,
                size_px: 16,
                font_stack_id: 1,
                font_slot: 0,
            },
            x: second.x,
            y: second.y,
            width: second.width,
            height: second.height,
            upload_state: TextureUploadState::Uploaded,
        });

        atlas.reset(4);

        assert!(atlas.entries.is_empty());
        assert!(atlas.icon_entries.is_empty());
        assert_eq!(atlas.cursor_x, 0);
        assert_eq!(atlas.cursor_y, 0);
        assert_eq!(atlas.row_height, 0);
        assert_eq!(atlas.used_frame, 4);
    }

    #[test]
    fn glyph_atlas_cache_prefers_current_page_with_room() {
        let table = ResourceTable::new();
        let active = test_glyph_atlas(&table, 7);
        let stale = test_glyph_atlas(&table, 6);

        assert_eq!(
            glyph_atlas_cache_action(&[active, stale], 7, AtlasEntryKind::Glyph, 16, 16, 2,),
            Some(GlyphAtlasCacheAction::Append(0))
        );
    }

    #[test]
    fn glyph_atlas_cache_recycles_only_a_page_unused_by_current_frame() {
        let table = ResourceTable::new();
        let mut active = test_glyph_atlas(&table, 7);
        let stale = test_glyph_atlas(&table, 6);
        fill_atlas_shelf(&mut active);

        assert_eq!(
            glyph_atlas_cache_action(&[active, stale], 7, AtlasEntryKind::Glyph, 16, 16, 2,),
            Some(GlyphAtlasCacheAction::Recycle(1))
        );
    }

    #[test]
    fn glyph_atlas_cache_creates_a_page_before_rejecting_current_frame() {
        let table = ResourceTable::new();
        let mut active = test_glyph_atlas(&table, 7);
        fill_atlas_shelf(&mut active);

        assert_eq!(
            glyph_atlas_cache_action(&[active], 7, AtlasEntryKind::Glyph, 16, 16, 2,),
            Some(GlyphAtlasCacheAction::Create)
        );
    }

    #[test]
    fn glyph_atlas_cache_rejects_only_when_all_current_pages_are_full() {
        let table = ResourceTable::new();
        let mut first = test_glyph_atlas(&table, 7);
        let mut second = test_glyph_atlas(&table, 7);
        fill_atlas_shelf(&mut first);
        fill_atlas_shelf(&mut second);

        assert_eq!(
            glyph_atlas_cache_action(&[first, second], 7, AtlasEntryKind::Glyph, 16, 16, 2,),
            None
        );
    }

    #[test]
    fn depth_support_is_required_only_for_opted_in_frames() {
        assert_eq!(validate_depth_support(false, false), Ok(()));
        assert_eq!(
            validate_depth_support(true, false),
            Err(Error::DepthUnsupported)
        );
        assert_eq!(validate_depth_support(true, true), Ok(()));
    }

    #[test]
    fn canvas_target_composite_preserves_vertical_orientation() {
        assert_eq!(CANVAS_TARGET_TEX_COORDS[0], [0.0, 0.0]);
        assert_eq!(CANVAS_TARGET_TEX_COORDS[1], [1.0, 0.0]);
        assert_eq!(CANVAS_TARGET_TEX_COORDS[2], [1.0, 1.0]);
        assert_eq!(CANVAS_TARGET_TEX_COORDS[3], [0.0, 1.0]);
    }

    #[test]
    fn presentation_target_count_is_configurable() {
        let encoder = SgfxPaintEncoder::with_target_count(64, 32, false, 3).unwrap();
        assert!(encoder.target_texture(0).is_some());
        assert!(encoder.target_texture(1).is_some());
        assert!(encoder.target_texture(2).is_some());
        assert!(encoder.target_texture(3).is_none());
        assert!(SgfxPaintEncoder::with_target_count(64, 32, false, 0).is_err());
    }

    #[test]
    fn vertical_gradient_lowers_to_banded_solid_draws() {
        let mut paint = PaintContext::new();
        paint.fill_vertical_gradient_rounded_rect(
            Rect::from_xywh(8.0, 8.0, 96.0, 32.0),
            6.0,
            UiColor::WHITE,
            UiColor::BLACK,
        );
        let mut encoder = SgfxPaintEncoder::new(128, 64, false).unwrap();
        let frame = encoder
            .lower_once(
                &paint,
                1_000,
                PixelBounds {
                    x: 0,
                    y: 0,
                    width: 128,
                    height: 64,
                },
            )
            .unwrap();

        assert_eq!(frame.draws.len(), 2);
        assert!(
            frame
                .draws
                .iter()
                .all(|draw| draw.source == DrawSource::Solid)
        );
        let colors = encoded_paint_colors(&frame.vertex_bytes);
        let opaque_red = colors
            .iter()
            .filter(|color| color[3] > 0.99)
            .map(|color| color[0])
            .collect::<Vec<_>>();
        assert!(opaque_red.first().unwrap() > opaque_red.last().unwrap());
    }

    #[test]
    fn rounded_shadow_lowers_to_weighted_blur_layers() {
        let mut paint = PaintContext::new();
        paint.draw_rounded_rect_shadow(
            Rect::from_xywh(24.0, 20.0, 72.0, 32.0),
            8.0,
            Offset::new(0.0, 3.0),
            10.0,
            0.0,
            UiColor::rgba(0, 0, 0, 48),
        );
        let mut encoder = SgfxPaintEncoder::new(128, 80, false).unwrap();
        let frame = encoder
            .lower_once(
                &paint,
                1_000,
                PixelBounds {
                    x: 0,
                    y: 0,
                    width: 128,
                    height: 80,
                },
            )
            .unwrap();

        assert_eq!(frame.draws.len(), 1);
        let alphas = encoded_paint_colors(&frame.vertex_bytes)
            .into_iter()
            .map(|color| color[3])
            .filter(|alpha| *alpha > 0.0)
            .collect::<Vec<_>>();
        let minimum = alphas.iter().copied().fold(f32::INFINITY, f32::min);
        let maximum = alphas.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(minimum < maximum);
    }

    #[test]
    fn differently_colored_masks_share_one_vertex_colored_draw() {
        let mut paint = PaintContext::new();
        paint.draw_icon(
            Rect::from_xywh(4.0, 4.0, 16.0, 16.0),
            ALL_ICONS[0],
            IconStyle::default(),
            UiColor::WHITE,
        );
        paint.draw_icon(
            Rect::from_xywh(24.0, 4.0, 16.0, 16.0),
            ALL_ICONS[0],
            IconStyle::default(),
            UiColor::BLACK,
        );
        let mut encoder = SgfxPaintEncoder::new(64, 32, false).unwrap();
        let frame = encoder
            .lower_once(
                &paint,
                1_000,
                PixelBounds {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 32,
                },
            )
            .unwrap();

        assert_eq!(
            frame
                .draws
                .iter()
                .filter(|draw| matches!(draw.source, DrawSource::Glyph(_)))
                .count(),
            1
        );
        let colors = encoded_paint_colors(&frame.vertex_bytes);
        assert!(colors.iter().any(|color| color == &[1.0, 1.0, 1.0, 1.0]));
        assert!(colors.iter().any(|color| color == &[0.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn mixed_paint_items_collapse_to_source_batches() {
        let image = Buffer::from_dimensions(2, 2);
        let mut paint = PaintContext::new();
        for index in 0..32 {
            let rect = Rect::from_xywh((index % 8) as f32 * 8.0, 0.0, 6.0, 6.0);
            let color = if index % 2 == 0 {
                UiColor::WHITE
            } else {
                UiColor::BLACK
            };
            paint.fill_rect(rect, color);
        }
        for index in 0..32 {
            let rect = Rect::from_xywh((index % 8) as f32 * 8.0, 8.0, 6.0, 6.0);
            let color = if index % 2 == 0 {
                UiColor::WHITE
            } else {
                UiColor::BLACK
            };
            paint.draw_icon(rect, ALL_ICONS[0], IconStyle::default(), color);
        }
        for index in 0..32 {
            let rect = Rect::from_xywh((index % 8) as f32 * 8.0, 16.0, 6.0, 6.0);
            paint.draw_buffer_ref(rect, &image);
        }

        let mut encoder = SgfxPaintEncoder::new(64, 32, false).unwrap();
        let frame = encoder
            .lower_once(
                &paint,
                1_000,
                PixelBounds {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 32,
                },
            )
            .unwrap();

        assert_eq!(frame.draws.len(), 4);
        assert!(frame.draws[0].source == DrawSource::Solid);
        assert!(matches!(frame.draws[1].source, DrawSource::Glyph(_)));
        assert!(matches!(frame.draws[2].source, DrawSource::Texture(_)));
        assert!(frame.draws[3].source == DrawSource::Solid);
    }

    #[test]
    fn retained_canvas_pass_is_limited_only_by_ir_commands() {
        let max_draws = (MAX_COMMANDS - PASS_COMMANDS) / MAX_CANVAS_DRAW_COMMANDS;
        let meshes = vec![0; max_draws];
        assert!(canvas_pass_reaches_frame_end(&meshes, 0, 0));

        let too_many_meshes = vec![0; max_draws + 1];
        assert!(!canvas_pass_reaches_frame_end(&too_many_meshes, 0, 0));
    }

    #[test]
    fn executor_receives_copy_then_unchunked_large_draw() {
        let mut paint = PaintContext::new();
        let mut polygon = Vec::new();
        for index in 0..600 {
            let angle = core::f32::consts::TAU * index as f32 / 600.0;
            polygon.push(Point::new(
                64.0 + libm::cosf(angle) * 60.0,
                64.0 + libm::sinf(angle) * 60.0,
            ));
        }
        paint.fill_path(polygon, UiColor::WHITE);

        let mut encoder = SgfxPaintEncoder::new(128, 128, false).unwrap();
        let mut executor = RecordingExecutor::default();
        encoder
            .encode_frame(
                &mut executor,
                1,
                Some(0),
                &paint,
                UiColor::BLACK,
                1_000,
                &[(0, 0, 128, 128)],
            )
            .unwrap();

        assert_eq!(executor.command_kinds.len(), 2);
        assert_eq!(executor.command_kinds[0], ["copy"]);
        assert_eq!(executor.command_kinds[1][0], "write-buffer");
        assert!(executor.command_kinds[1].contains(&"begin-pass"));
        assert_eq!(executor.command_kinds[1].last(), Some(&"end-pass"));
        assert!(executor.draw_vertices.iter().any(|count| *count > 1_440));
    }

    #[test]
    fn failed_buffer_upload_is_retried_then_committed() {
        let buffer = Buffer::from_dimensions(2, 2);
        let mut paint = PaintContext::new();
        paint.draw_buffer_ref(
            Rect::new(Point::new(0.0, 0.0), Size::new(2.0, 2.0)),
            &buffer,
        );
        let mut encoder = SgfxPaintEncoder::new(8, 8, false).unwrap();
        let mut executor = FailOnceExecutor {
            fail_next: true,
            texture_write_counts: Vec::new(),
        };

        assert!(matches!(
            encoder.encode_frame(
                &mut executor,
                0,
                None,
                &paint,
                UiColor::BLACK,
                1_000,
                &[(0, 0, 8, 8)],
            ),
            Err(FrameError::Execution("injected upload failure"))
        ));
        encoder
            .encode_frame(
                &mut executor,
                0,
                None,
                &paint,
                UiColor::BLACK,
                1_000,
                &[(0, 0, 8, 8)],
            )
            .unwrap();
        encoder
            .encode_frame(
                &mut executor,
                0,
                None,
                &paint,
                UiColor::BLACK,
                1_000,
                &[(0, 0, 8, 8)],
            )
            .unwrap();

        assert_eq!(executor.texture_write_counts, [1, 1, 0, 0]);
    }

    fn assert_encode_frame_texture_upload_retry(paint: &PaintContext<'_>) {
        let mut encoder = SgfxPaintEncoder::new(64, 64, false).unwrap();
        let mut executor = FailOnceExecutor {
            fail_next: true,
            texture_write_counts: Vec::new(),
        };
        assert!(matches!(
            encoder.encode_frame(
                &mut executor,
                0,
                None,
                paint,
                UiColor::BLACK,
                1_000,
                &[(0, 0, 64, 64)],
            ),
            Err(FrameError::Execution("injected upload failure"))
        ));
        encoder
            .encode_frame(
                &mut executor,
                0,
                None,
                paint,
                UiColor::BLACK,
                1_000,
                &[(0, 0, 64, 64)],
            )
            .unwrap();
        encoder
            .encode_frame(
                &mut executor,
                0,
                None,
                paint,
                UiColor::BLACK,
                1_000,
                &[(0, 0, 64, 64)],
            )
            .unwrap();

        let first_upload_count = executor.texture_write_counts[0];
        assert!(first_upload_count > 0);
        assert_eq!(
            executor.texture_write_counts,
            [first_upload_count, first_upload_count, 0, 0]
        );
    }

    #[test]
    fn failed_text_upload_is_retried_through_encode_frame() {
        let mut paint = PaintContext::new();
        paint.draw_text(Point::new(4.0, 24.0), "A", UiColor::WHITE, 16.0);
        assert_encode_frame_texture_upload_retry(&paint);
    }

    #[test]
    fn failed_icon_upload_is_retried_through_encode_frame() {
        let mut paint = PaintContext::new();
        paint.draw_icon(
            Rect::new(Point::new(4.0, 4.0), Size::new(20.0, 20.0)),
            ALL_ICONS[0],
            IconStyle::default(),
            UiColor::WHITE,
        );
        assert_encode_frame_texture_upload_retry(&paint);
    }

    #[test]
    fn glyph_and_icon_uploads_remain_pending_until_committed() {
        let mut encoder = SgfxPaintEncoder::new(32, 32, false).unwrap();
        let glyph_key = GlyphRasterKey {
            codepoint: 'A' as u32,
            size_px: 16,
            font_stack_id: 1,
            font_slot: 0,
        };
        let (glyph_texture, glyph_bounds, glyph_upload) =
            encoder.glyph_texture(glyph_key, 4, 5).unwrap();
        assert!(glyph_upload);
        assert!(encoder.glyph_texture(glyph_key, 4, 5).unwrap().2);

        let icon_key = IconMaskKey {
            icon: ALL_ICONS[0],
            pixel_size: 16,
            style: IconStyle::default(),
        };
        let (icon_texture, icon_bounds, icon_upload) =
            encoder.icon_texture(icon_key, 6, 7).unwrap();
        assert!(icon_upload);
        assert!(encoder.icon_texture(icon_key, 6, 7).unwrap().2);

        let glyph_bytes = Arc::<[u8]>::from(alloc::vec![0; 20]);
        let icon_bytes = Arc::<[u8]>::from(alloc::vec![0; 42]);
        let frame = LoweredFrame {
            vertex_bytes: Vec::new(),
            draws: Vec::new(),
            uploads: alloc::vec![
                TextureUpload {
                    texture: glyph_texture,
                    x: glyph_bounds.x,
                    y: glyph_bounds.y,
                    width: glyph_bounds.width,
                    height: glyph_bounds.height,
                    bytes_per_row: glyph_bounds.width,
                    bytes: UploadBytes::Shared(glyph_bytes),
                },
                TextureUpload {
                    texture: icon_texture,
                    x: icon_bounds.x,
                    y: icon_bounds.y,
                    width: icon_bounds.width,
                    height: icon_bounds.height,
                    bytes_per_row: icon_bounds.width,
                    bytes: UploadBytes::Shared(icon_bytes),
                },
            ],
        };
        let mut executor = FailOnceExecutor {
            fail_next: true,
            texture_write_counts: Vec::new(),
        };
        assert!(matches!(
            encoder.submit_texture_uploads(&mut executor, &frame),
            Err(FrameError::Execution("injected upload failure"))
        ));
        assert!(encoder.glyph_texture(glyph_key, 4, 5).unwrap().2);
        assert!(encoder.icon_texture(icon_key, 6, 7).unwrap().2);
        encoder
            .submit_texture_uploads(&mut executor, &frame)
            .unwrap();

        assert!(!encoder.glyph_texture(glyph_key, 4, 5).unwrap().2);
        assert!(!encoder.icon_texture(icon_key, 6, 7).unwrap().2);
        assert_eq!(executor.texture_write_counts, [2, 2]);
    }

    #[test]
    fn one_canvas_frame_rejects_mixed_revisions_of_a_handle() {
        let handle = SgfxMeshHandle::new();
        let transform = Transform::identity().columns();
        let valid = SgfxCanvasFrame::new(1, UiColor::BLACK)
            .draw(SgfxCanvasDraw::new(
                SgfxMesh::with_handle(handle, 4, triangle(0.0)),
                transform,
            ))
            .draw(SgfxCanvasDraw::new(
                SgfxMesh::with_handle(handle, 4, triangle(0.5)),
                transform,
            ));
        assert!(!canvas_frame_has_revision_conflict(&valid));

        let invalid = valid.draw(SgfxCanvasDraw::new(
            SgfxMesh::with_handle(handle, 5, triangle(1.0)),
            transform,
        ));
        assert!(canvas_frame_has_revision_conflict(&invalid));
    }

    #[test]
    fn depth_target_pipeline_and_pass_descriptor_are_valid() {
        let table = ResourceTable::new();
        let extent = Extent2D::new(64, 48).unwrap();
        let color = table
            .define_texture(
                TextureDesc::new(
                    TextureFormat::Bgra8Unorm,
                    extent,
                    TextureUsage::RENDER_ATTACHMENT,
                )
                .unwrap(),
            )
            .unwrap();
        let depth = table
            .define_texture(
                TextureDesc::new(
                    TextureFormat::Depth32Float,
                    extent,
                    TextureUsage::RENDER_ATTACHMENT,
                )
                .unwrap(),
            )
            .unwrap();
        let area = PixelRect::new(0, 0, 64, 48).unwrap();
        let pass = RenderPassDesc::new(&table, color, area, LoadOp::DontCare, StoreOp::Store)
            .unwrap()
            .with_depth_attachment(&table, depth, DepthLoadOp::Clear(1.0), StoreOp::DontCare)
            .unwrap();
        let attachment = pass.depth_attachment().unwrap();
        assert_eq!(attachment.load(), DepthLoadOp::Clear(1.0));
        assert_eq!(attachment.store(), StoreOp::DontCare);
        assert!(define_canvas_pipeline(&table, true).is_ok());
        assert!(define_canvas_texture_pipeline(&table, true).is_ok());
    }
}
