//! Paint-command to persistent SGFX IR lowering.
mod retained;

use alloc::rc::Rc;
use alloc::sync::Arc;
use alloc::vec::Vec;

use scarlet_ui_core::buffer::Buffer;
use scarlet_ui_core::color::Color as UiColor;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::graphics::{GlyphRasterKey, rasterize_text_cached};
use scarlet_ui_core::icon::{IconMaskKey, rasterize_icon};
use scarlet_ui_core::renderer::{BufferHandle, PaintCommand, PaintContext, PaintExtension};
use sgfx::backend::CommandExecutor;
use sgfx::ir::{
    AddressMode, BlendState, BufferDesc, BufferId, BufferUsage, Color, CommandEncoder,
    CompareFunction, DepthLoadOp, DepthState, DrawUniforms, Extent2D, FilterMode, FragmentProgram,
    FrontFace, LoadOp, MAX_COMMANDS, PixelRect, PrimitiveTopology, RasterState, RenderPassDesc,
    RenderPipelineDesc, RenderPipelineId, ResourceTable, SamplerDesc, SamplerId, StoreOp,
    TextureDesc, TextureFormat, TextureId, TextureSampleMode, TextureUsage, TextureWrite,
    Transform, VertexAttribute, VertexBufferLayout, VertexFormat, Viewport,
};

use crate::canvas::{SgfxCanvasFrame, SgfxCanvasPaint, SgfxCanvasVertex, SgfxMesh, SgfxTexture};
use crate::error::{Error, FrameError, Result, Stage};
use crate::external_surface::ExternalGpuSurfacePaint;
use crate::geometry::{
    FloatRect, GeometryRange, MAX_FRAME_VERTICES, PixelBounds, Tessellator, Vertex,
};

const PAINT_VERTEX_STRIDE: u32 = 40;
const PASS_COMMANDS: usize = 2;
const CANVAS_PASS_COMMANDS: usize = PASS_COMMANDS + 2; // viewport and scissor
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
    PixelTexture(TextureId),
    Glyph(TextureId),
    IconGlyph(TextureId),
}

#[derive(Clone, Copy)]
struct Draw {
    geometry: GeometryRange,
    source: DrawSource,
    vertex_buffer: Option<BufferId>,
    offset: [f32; 2],
    snap_phase: [f32; 2],
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
    capacity_width: u32,
    capacity_height: u32,
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
    generation: u64,
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
            generation: 0,
        }
    }

    fn reset(&mut self, frame_serial: u64) {
        self.generation = self.generation.wrapping_add(1);
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
    capacity_width: u32,
    capacity_height: u32,
    used_in_frame: bool,
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
            (previous.mesh.handle == draw.mesh.handle
                && previous.mesh.revision != draw.mesh.revision)
                || match (&previous.texture, &draw.texture) {
                    (Some(previous), Some(current)) => {
                        previous.handle == current.handle && previous.revision != current.revision
                    }
                    _ => false,
                }
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
    let mut command_count = prefix_commands.saturating_add(CANVAS_PASS_COMMANDS);
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
    handle_id: u64,
    revision: u64,
    texture: TextureId,
    source: Arc<SgfxTexture>,
    uploaded: bool,
    external_bound: bool,
}

/// One logical texture awaiting a platform image import.
#[derive(Clone, Debug)]
pub struct SgfxExternalTextureBinding {
    texture: TextureId,
    source: Arc<dyn PaintExtension>,
}

impl SgfxExternalTextureBinding {
    /// Logical sampled texture that must be mapped by the active SGFX session.
    pub const fn texture(&self) -> TextureId {
        self.texture
    }

    /// Type-erased platform image owner associated with this texture.
    pub fn source(&self) -> &dyn PaintExtension {
        self.source.as_ref()
    }
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
    pixel_sampler: SamplerId,
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
    free_external_textures: Vec<(TextureId, TextureFormat, u32, u32)>,
    frame_serial: u64,
    width: u32,
    height: u32,
    supports_depth: bool,
    retained_meshes: Vec<retained::RetainedMesh>,
    recording_mesh: bool,
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
        let pixel_sampler = table
            .define_sampler(SamplerDesc::new(
                FilterMode::Nearest,
                FilterMode::Nearest,
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
            retained_meshes: Vec::new(),
            recording_mesh: false,
            table,
            targets,
            vertex_buffer,
            solid_pipeline,
            texture_pipeline,
            glyph_pipeline,
            sampler,
            pixel_sampler,
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
            free_external_textures: Vec::new(),
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

    /// External slots no longer referenced by this frame. A bound slot must
    /// be detached only after accepted GPU work retires, before recycling it.
    pub fn unused_external_textures(&self, paint: &PaintContext<'_>) -> Vec<(TextureId, bool)> {
        if paint
            .commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::DrawDisplayList { .. }))
        {
            return self.unused_external_textures(&extension_context(paint));
        }
        let mut active = Vec::new();
        for command in paint.commands() {
            let PaintCommand::Extension { payload, .. } = command else {
                continue;
            };
            if let Some(surface) = payload
                .as_ref()
                .as_any()
                .downcast_ref::<ExternalGpuSurfacePaint>()
            {
                active.push(surface.texture.handle.id());
            } else if let Some(canvas) = payload.as_ref().as_any().downcast_ref::<SgfxCanvasPaint>()
            {
                for draw in &canvas.frame.draws {
                    if let Some(texture) = &draw.texture {
                        active.push(texture.handle.id());
                    }
                }
            }
        }
        self.canvas_textures
            .iter()
            .filter(|t| t.source.external_source().is_some() && !active.contains(&t.handle_id))
            .map(|t| (t.texture, t.external_bound))
            .collect()
    }

    /// Recycle an external logical slot after the platform has retired its reads
    /// and detached the old image. Descriptor-compatible slots avoid exhausting
    /// the immutable resource table during indefinite video playback.
    pub fn retire_external_texture(&mut self, texture: TextureId) -> Result<()> {
        let index = self
            .canvas_textures
            .iter()
            .position(|t| t.texture == texture && t.source.external_source().is_some())
            .ok_or(Error::InvalidFrame)?;
        let old = self.canvas_textures.remove(index);
        let format = if old.source.is_nv12() {
            TextureFormat::Nv12
        } else {
            TextureFormat::Bgra8Unorm
        };
        self.free_external_textures
            .push((texture, format, old.source.width, old.source.height));
        Ok(())
    }

    /// Define external canvas textures referenced by this paint list and return
    /// every image that the platform session has not imported yet.
    pub fn prepare_external_textures(
        &mut self,
        paint: &PaintContext<'_>,
    ) -> Result<Vec<SgfxExternalTextureBinding>> {
        if paint
            .commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::DrawDisplayList { .. }))
        {
            return self.prepare_external_textures(&extension_context(paint));
        }
        let mut pending = Vec::new();
        for command in paint.commands() {
            let PaintCommand::Extension { payload, .. } = command else {
                continue;
            };
            if let Some(surface) = payload
                .as_ref()
                .as_any()
                .downcast_ref::<ExternalGpuSurfacePaint>()
            {
                self.prepare_external_texture(&surface.texture, &mut pending)?;
            } else if let Some(canvas) = payload.as_ref().as_any().downcast_ref::<SgfxCanvasPaint>()
            {
                for draw in &canvas.frame.draws {
                    if let Some(texture) = draw.texture.as_ref() {
                        self.prepare_external_texture(texture, &mut pending)?;
                    }
                }
            }
        }
        Ok(pending)
    }

    fn prepare_external_texture(
        &mut self,
        texture: &Arc<SgfxTexture>,
        pending: &mut Vec<SgfxExternalTextureBinding>,
    ) -> Result<()> {
        let Some(source) = texture.external_source() else {
            return Ok(());
        };
        let source = Arc::clone(source);
        let index = self.canvas_texture(texture)?;
        let cached = &self.canvas_textures[index];
        if cached.external_bound
            || pending
                .iter()
                .any(|binding| binding.texture == cached.texture)
        {
            return Ok(());
        }
        pending.push(SgfxExternalTextureBinding {
            texture: cached.texture,
            source,
        });
        Ok(())
    }

    /// Record that the active backend session imported an external texture.
    pub fn mark_external_texture_bound(&mut self, texture: TextureId) -> Result<()> {
        let cached = self
            .canvas_textures
            .iter_mut()
            .find(|cached| cached.texture == texture)
            .ok_or(Error::InvalidFrame)?;
        if cached.source.external_source().is_none() {
            return Err(Error::InvalidFrame);
        }
        cached.external_bound = true;
        Ok(())
    }

    /// Invalidate cached canvas contents after discarding a partially encoded frame.
    ///
    /// # Returns
    ///
    /// Nothing. Call only after all accepted work retired successfully. A canvas
    /// may have been modified by an accepted pass before a later pass was rejected,
    /// so its previous revision no longer certifies its pixels. Accepted mesh and
    /// texture uploads remain valid and are not needlessly uploaded again. The
    /// next presentation target must also be fully repainted by the platform.
    pub fn discard_frame(&mut self) {
        for target in &mut self.canvas_targets {
            target.initialized = false;
        }
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

    /// Encode one ScarletUI frame and submit its ordered command buffers.
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
    /// Success after the executor accepts all ordered command buffers, a portable
    /// lowering error, or the executor's backend-owned error. Success alone does
    /// not establish GPU completion. When using [`crate::FrameExecutor`], require
    /// `CompletionStatus::Complete` from [`crate::FrameExecutor::wait`] before
    /// handing the target to presentation.
    #[expect(
        clippy::too_many_arguments,
        reason = "Keep the existing public frame-encoding signature source compatible"
    )]
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
        #[cfg(feature = "std")]
        let started = scarlet_ui_core::debug::frame_log_enabled().then(std::time::Instant::now);
        let render_areas = self.validate_render_areas(render_areas)?;
        let target = *self.targets.get(slot).ok_or(Error::InvalidFrame)?;
        if let Some(source_slot) = copy_from {
            self.copy_target(executor, source_slot, slot)?;
        }
        let render_bounds = bounding_area(&render_areas).ok_or(Error::InvalidFrame)?;
        self.advance_frame_serial();
        let retained = paint
            .commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::DrawDisplayList { .. }));
        if retained {
            // Canvas payloads keep their existing preparation and import path.
            self.prepare_canvases(executor, &extension_context(paint), scale_milli)?;
        } else {
            self.prepare_canvases(executor, paint, scale_milli)?;
        }
        let lowered = if retained {
            self.lower_retained(executor, paint, scale_milli, render_bounds)?
        } else {
            self.lower(paint, scale_milli, render_bounds)?
        };
        #[cfg(feature = "std")]
        let lower_us = started.map(|start| start.elapsed().as_micros());
        let result = self.submit(executor, target, background, &render_areas, &lowered);
        #[cfg(feature = "std")]
        if let Some(start) = started {
            eprintln!(
                "[ScrollTiming] lower_us={} encode_us={} retained_draws={} frame_vertex_bytes={}",
                lower_us.unwrap_or_default(),
                start.elapsed().as_micros(),
                lowered
                    .draws
                    .iter()
                    .filter(|draw| draw.vertex_buffer.is_some())
                    .count(),
                lowered.vertex_bytes.len()
            );
        }
        result
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
                PaintCommand::DrawDisplayList { .. } => return Err(Error::InvalidFrame),
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
                    let glyphs = rasterize_text_cached(text, *font_size_px, scale_milli);
                    for glyph in glyphs.iter() {
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
                                bytes: UploadBytes::Shared(Arc::clone(&glyph.mask)),
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
                            push_draw_phase(
                                &mut draws,
                                &mut tessellator,
                                geometry,
                                color,
                                DrawSource::Glyph(texture),
                                if self.recording_mesh {
                                    [fractional(position.x), fractional(position.y)]
                                } else {
                                    [0., 0.]
                                },
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
                        push_draw_phase(
                            &mut draws,
                            &mut tessellator,
                            geometry,
                            ui_color(*color, opacity)?,
                            if self.recording_mesh {
                                DrawSource::IconGlyph(texture)
                            } else {
                                DrawSource::Glyph(texture)
                            },
                            if self.recording_mesh {
                                [fractional(rect.origin.x), fractional(rect.origin.y)]
                            } else {
                                [0., 0.]
                            },
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
                        uploads.extend(upload);
                        push_draw_phase(
                            &mut draws,
                            &mut tessellator,
                            geometry,
                            [1.0, 1.0, 1.0, opacity],
                            DrawSource::Texture(texture),
                            if self.recording_mesh {
                                [fractional(dst.origin.x), fractional(dst.origin.y)]
                            } else {
                                [0., 0.]
                            },
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
                    let scaled_source =
                        FloatRect::from_logical(*src, buffer.scale_milli() as f32 / 1000.0);
                    let source = FloatRect::new(
                        truncated(scaled_source.x),
                        truncated(scaled_source.y),
                        truncated(scaled_source.width),
                        truncated(scaled_source.height),
                    );
                    let destination = FloatRect::new(
                        truncated_scaled(dst.origin.x, scale),
                        truncated_scaled(dst.origin.y, scale),
                        truncated_scaled(dst.size.width, scale),
                        truncated_scaled(dst.size.height, scale),
                    );
                    if let Some((geometry, texture, upload)) = self.lower_buffer(
                        &mut tessellator,
                        &mut buffer_mappings,
                        buffer,
                        source,
                        destination,
                    )? {
                        uploads.extend(upload);
                        let combined_opacity = finite_unit(*command_opacity)? * opacity;
                        push_draw_phase(
                            &mut draws,
                            &mut tessellator,
                            geometry,
                            [1.0, 1.0, 1.0, combined_opacity],
                            DrawSource::Texture(texture),
                            if self.recording_mesh {
                                [fractional(dst.origin.x), fractional(dst.origin.y)]
                            } else {
                                [0., 0.]
                            },
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
                    if let Some(image) = payload
                        .as_ref()
                        .as_any()
                        .downcast_ref::<crate::scaled_buffer::ScaledBufferPaint>()
                    {
                        let buffer = &image.buffer;
                        if let Some((geometry, texture, upload)) = self.lower_buffer(
                            &mut tessellator,
                            &mut buffer_mappings,
                            buffer,
                            FloatRect::new(0.0, 0.0, buffer.width() as f32, buffer.height() as f32),
                            FloatRect::new(
                                truncated_scaled(rect.origin.x, scale),
                                truncated_scaled(rect.origin.y, scale),
                                truncated_scaled(rect.size.width, scale),
                                truncated_scaled(rect.size.height, scale),
                            ),
                        )? {
                            uploads.extend(upload);
                            push_draw(
                                &mut draws,
                                &mut tessellator,
                                geometry,
                                [1.0, 1.0, 1.0, opacity],
                                DrawSource::PixelTexture(texture),
                            )?;
                        }
                        continue;
                    }
                    if let Some(surface) = payload
                        .as_ref()
                        .as_any()
                        .downcast_ref::<ExternalGpuSurfacePaint>()
                    {
                        if surface.texture.external_source().is_none() {
                            return Err(Error::InvalidFrame);
                        }
                        let texture_index = self.canvas_texture(&surface.texture)?;
                        let texture = &self.canvas_textures[texture_index];
                        if !texture.external_bound {
                            return Err(Error::ExternalTextureUnbound);
                        }
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
                        continue;
                    }
                    let Some(canvas) = payload.as_ref().as_any().downcast_ref::<SgfxCanvasPaint>()
                    else {
                        continue;
                    };
                    let canvas_width = rasterized_canvas_extent(
                        rect.size.width,
                        scale,
                        canvas.frame.raster_scale,
                    )?;
                    let canvas_height = rasterized_canvas_extent(
                        rect.size.height,
                        scale,
                        canvas.frame.raster_scale,
                    )?;
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
                    let u = texture.width as f32 / texture.capacity_width as f32;
                    let v = texture.height as f32 / texture.capacity_height as f32;
                    if let Some(geometry) = tessellator
                        .textured_rect(destination, [[0., 0.], [u, 0.], [u, v], [0., v]])?
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
        for target in &mut self.canvas_targets {
            target.used_in_frame = false;
        }
        for command in paint.commands() {
            let PaintCommand::Extension { rect, payload } = command else {
                continue;
            };
            let Some(canvas) = payload.as_ref().as_any().downcast_ref::<SgfxCanvasPaint>() else {
                continue;
            };
            let width =
                rasterized_canvas_extent(rect.size.width, scale, canvas.frame.raster_scale)?;
            let height =
                rasterized_canvas_extent(rect.size.height, scale, canvas.frame.raster_scale)?;
            let target_index =
                self.canvas_target(canvas.handle.id(), width, height, canvas.frame.depth_test)?;
            self.canvas_targets[target_index].used_in_frame = true;
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
        if width == 0 || height == 0 {
            return Err(Error::InvalidFrame);
        }
        let matches_handle = |target: &CanvasTarget| {
            target.handle_id == handle_id && target.depth.is_some() == depth_test
        };
        // A handle may appear at multiple sizes in one paint list. Preserve
        // targets already used by this frame, but recycle prior-frame sizes.
        let existing = self
            .canvas_targets
            .iter()
            .position(|target| {
                matches_handle(target) && target.width == width && target.height == height
            })
            .or_else(|| {
                self.canvas_targets
                    .iter()
                    .position(|target| matches_handle(target) && !target.used_in_frame)
            });
        if let Some(index) = existing {
            let target = &mut self.canvas_targets[index];
            if width <= target.capacity_width && height <= target.capacity_height {
                if target.width != width || target.height != height {
                    target.width = width;
                    target.height = height;
                    target.initialized = false;
                }
                return Ok(index);
            }
        }
        validate_depth_support(depth_test, self.supports_depth)?;
        if existing.is_none() && self.canvas_targets.len() >= MAX_CANVASES {
            return Err(Error::FrameTooComplex);
        }
        // Keep one cache entry per concurrent canvas size. Capacity growth bounds
        // immutable texture definitions during arbitrarily many resize steps.
        let old_capacity = existing
            .map(|i| {
                (
                    self.canvas_targets[i].capacity_width,
                    self.canvas_targets[i].capacity_height,
                )
            })
            .unwrap_or((0, 0));
        let capacity_width = width
            .max(old_capacity.0)
            .checked_next_power_of_two()
            .ok_or(Error::InvalidFrame)?;
        let capacity_height = height
            .max(old_capacity.1)
            .checked_next_power_of_two()
            .ok_or(Error::InvalidFrame)?;
        let extent =
            Extent2D::new(capacity_width, capacity_height).map_err(|_| Error::InvalidFrame)?;
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
        let target = CanvasTarget {
            handle_id,
            texture,
            depth,
            width,
            height,
            capacity_width,
            capacity_height,
            used_in_frame: false,
            revision: 0,
            initialized: false,
        };
        if let Some(index) = existing {
            self.canvas_targets[index] = target;
            Ok(index)
        } else {
            self.canvas_targets.push(target);
            Ok(self.canvas_targets.len() - 1)
        }
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
        if texture.width == 0 || texture.height == 0 {
            return Err(Error::InvalidFrame);
        }
        let (format, external_bound) = if let Some(pixels) = texture.rgba8_pixels() {
            let expected_len = usize::try_from(texture.width)
                .ok()
                .and_then(|width| {
                    usize::try_from(texture.height)
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or(Error::FrameTooComplex)?;
            if pixels.len() != expected_len {
                return Err(Error::InvalidFrame);
            }
            (TextureFormat::Rgba8Unorm, true)
        } else {
            (
                if texture.is_nv12() {
                    TextureFormat::Nv12
                } else {
                    TextureFormat::Bgra8Unorm
                },
                false,
            )
        };
        if let Some(index) = self
            .canvas_textures
            .iter()
            .position(|cached| cached.handle_id == texture.handle.id())
        {
            let cached = &mut self.canvas_textures[index];
            let cached_format = if cached.source.rgba8_pixels().is_some() {
                TextureFormat::Rgba8Unorm
            } else if cached.source.is_nv12() {
                TextureFormat::Nv12
            } else {
                TextureFormat::Bgra8Unorm
            };
            if cached.source.width != texture.width
                || cached.source.height != texture.height
                || cached_format != format
            {
                return Err(Error::InvalidFrame);
            }
            if cached.revision == texture.revision {
                return Ok(index);
            }
            if texture.rgba8_pixels().is_none() {
                return Err(Error::InvalidFrame);
            }
            cached.revision = texture.revision;
            cached.source = Arc::clone(texture);
            cached.uploaded = false;
            return Ok(index);
        }
        if self.canvas_textures.len() >= MAX_CANVAS_TEXTURES {
            return Err(Error::InvalidFrame);
        }
        let reusable = self
            .free_external_textures
            .iter()
            .position(|&(_, f, w, h)| {
                !external_bound && f == format && w == texture.width && h == texture.height
            });
        let texture_id = if let Some(index) = reusable {
            self.free_external_textures.swap_remove(index).0
        } else {
            define_sampled_texture(&self.table, format, texture.width, texture.height)?
        };
        self.canvas_textures.push(CanvasTexture {
            handle_id: texture.handle.id(),
            revision: texture.revision,
            texture: texture_id,
            source: Arc::clone(texture),
            uploaded: false,
            external_bound,
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
        // Clear the entire attachment, including padding. Partial depth clears
        // are unsupported by WGPU; the viewport/scissor below bound the draws.
        let area = PixelRect::new(
            0,
            0,
            target_state.capacity_width,
            target_state.capacity_height,
        )
        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
        let viewport = Viewport::new(
            0.,
            0.,
            target_state.width as f32,
            target_state.height as f32,
            0.,
            1.,
        )
        .map_err(|_| Error::InvalidFrame)?;
        let scissor = PixelRect::new(0, 0, target_state.width, target_state.height)
            .map_err(|_| Error::InvalidFrame)?;
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
            let cached = &self.canvas_textures[texture_index];
            if cached.source.external_source().is_some() {
                if !cached.external_bound {
                    return Err(Error::ExternalTextureUnbound.into());
                }
                continue;
            }
            if cached.uploaded || texture_uploads.contains(&texture_index) {
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
            pass.set_viewport(viewport)
                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
            pass.set_scissor(Some(scissor))
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
                        let pixels = cached
                            .source
                            .rgba8_pixels()
                            .ok_or(Error::ExternalTextureUnbound)?;
                        let write = TextureWrite::new(destination, bytes_per_row, pixels)
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
                pass.set_viewport(viewport)
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                pass.set_scissor(Some(scissor))
                    .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                let mut command_count = prefix_commands.saturating_add(CANVAS_PASS_COMMANDS);

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
                if command_count == prefix_commands.saturating_add(CANVAS_PASS_COMMANDS) {
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
    ) -> Result<Option<(GeometryRange, TextureId, Vec<TextureUpload<'frame>>)>> {
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
        let scale_x = destination.width / source.width;
        let scale_y = destination.height / source.height;
        let clipped_destination = FloatRect::new(
            destination.x + (source_left - source.x) * scale_x,
            destination.y + (source_top - source.y) * scale_y,
            (source_right - source_left) * scale_x,
            (source_bottom - source_top) * scale_y,
        );
        // Determine allocation dimensions without consuming a slot for a
        // completely clipped image. The same slot remains available below.
        let (capacity_width, capacity_height) = match self.buffer_texture_slot(buffer) {
            Some(index) => {
                let texture = &self.buffer_textures[index];
                (texture.capacity_width, texture.capacity_height)
            }
            None => buffer_texture_capacity(buffer.width(), buffer.height())?,
        };
        let inverse_width = 1.0 / capacity_width as f32;
        let inverse_height = 1.0 / capacity_height as f32;
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
            return Ok(Some((geometry, *texture, Vec::new())));
        }
        let (texture, upload_required) = self.buffer_texture(buffer)?;
        mappings
            .try_reserve(1)
            .map_err(|_| Error::FrameTooComplex)?;
        mappings.push((buffer_identity, texture));
        if !upload_required {
            return Ok(Some((geometry, texture, Vec::new())));
        }
        let bytes_per_row = buffer
            .width()
            .checked_mul(4)
            .ok_or(Error::FrameTooComplex)?;
        let mut uploads = Vec::new();
        uploads.try_reserve(3).map_err(|_| Error::FrameTooComplex)?;
        uploads.push(TextureUpload {
            texture,
            x: 0,
            y: 0,
            width: buffer.width(),
            height: buffer.height(),
            bytes_per_row,
            bytes: UploadBytes::Borrowed(buffer.data()),
        });
        // A linear sampler must see the current image's edge, never unused
        // padding left by an older image. One duplicate column/row suffices;
        // UVs never address the remaining capacity. Full-capacity edges use
        // the sampler's existing ClampToEdge behavior.
        let row_bytes = usize::try_from(bytes_per_row).map_err(|_| Error::FrameTooComplex)?;
        let padded_right = buffer.width() < capacity_width;
        if padded_right {
            let mut edge = Vec::new();
            edge.try_reserve_exact(
                usize::try_from(buffer.height())
                    .map_err(|_| Error::FrameTooComplex)?
                    .checked_mul(4)
                    .ok_or(Error::FrameTooComplex)?,
            )
            .map_err(|_| Error::FrameTooComplex)?;
            for row in buffer.data().chunks_exact(row_bytes) {
                edge.extend_from_slice(&row[row_bytes - 4..]);
            }
            uploads.push(TextureUpload {
                texture,
                x: buffer.width(),
                y: 0,
                width: 1,
                height: buffer.height(),
                bytes_per_row: 4,
                bytes: UploadBytes::Shared(Arc::from(edge)),
            });
        }
        if buffer.height() < capacity_height {
            let bottom_width = buffer.width() + u32::from(padded_right);
            let bottom_bytes = bottom_width.checked_mul(4).ok_or(Error::FrameTooComplex)?;
            let mut edge = Vec::new();
            edge.try_reserve_exact(
                usize::try_from(bottom_bytes).map_err(|_| Error::FrameTooComplex)?,
            )
            .map_err(|_| Error::FrameTooComplex)?;
            let last_row = &buffer.data()[buffer.data().len() - row_bytes..];
            edge.extend_from_slice(last_row);
            if padded_right {
                edge.extend_from_slice(&last_row[row_bytes - 4..]);
            }
            uploads.push(TextureUpload {
                texture,
                x: 0,
                y: buffer.height(),
                width: bottom_width,
                height: 1,
                bytes_per_row: bottom_bytes,
                bytes: UploadBytes::Shared(Arc::from(edge)),
            });
        }
        Ok(Some((geometry, texture, uploads)))
    }

    fn buffer_texture_slot(&self, buffer: &Buffer) -> Option<usize> {
        self.buffer_textures
            .iter()
            .position(|texture| {
                texture.buffer_identity == buffer.identity()
                    && texture.width == buffer.width()
                    && texture.height == buffer.height()
            })
            .or_else(|| {
                self.buffer_textures
                    .iter()
                    .enumerate()
                    .filter(|(_, texture)| {
                        texture.used_frame != self.frame_serial
                            && texture.capacity_width >= buffer.width()
                            && texture.capacity_height >= buffer.height()
                    })
                    .min_by_key(|(_, texture)| {
                        u64::from(texture.capacity_width) * u64::from(texture.capacity_height)
                    })
                    .map(|(index, _)| index)
            })
    }

    fn buffer_texture(&mut self, buffer: &Buffer) -> Result<(TextureId, bool)> {
        let buffer_identity = buffer.identity();
        let revision = buffer.revision();
        let width = buffer.width();
        let height = buffer.height();
        if let Some(index) = self.buffer_texture_slot(buffer) {
            let texture = &mut self.buffer_textures[index];
            let upload_required = texture.buffer_identity != buffer_identity
                || texture.revision != revision
                || texture.upload_state == TextureUploadState::Pending;
            if upload_required {
                texture.upload_state = TextureUploadState::Pending;
            }
            texture.buffer_identity = buffer_identity;
            texture.revision = revision;
            texture.width = width;
            texture.height = height;
            texture.used_frame = self.frame_serial;
            return Ok((texture.texture, upload_required));
        }
        if self.buffer_textures.len() >= MAX_BUFFER_TEXTURES {
            return Err(Error::FrameTooComplex);
        }
        let (capacity_width, capacity_height) = buffer_texture_capacity(width, height)?;
        let texture = define_sampled_texture(
            &self.table,
            TextureFormat::Bgra8Unorm,
            capacity_width,
            capacity_height,
        )?;
        self.buffer_textures.push(BufferTexture {
            texture,
            buffer_identity,
            revision,
            width,
            height,
            capacity_width,
            capacity_height,
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
                if first_submission && !frame.vertex_bytes.is_empty() {
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
                        DrawSource::Texture(texture) | DrawSource::PixelTexture(texture) => {
                            (texture_pipeline, Some(texture))
                        }
                        DrawSource::Glyph(texture) | DrawSource::IconGlyph(texture) => {
                            (glyph_pipeline, Some(texture))
                        }
                    };
                    pass.set_pipeline(pipeline)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    let selected_buffer = match draw.vertex_buffer {
                        Some(id) => table
                            .buffer_ref(id)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?,
                        None => vertex_buffer,
                    };
                    pass.set_vertex_buffer(selected_buffer, 0)
                        .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    if let Some(texture) = texture {
                        let texture = table
                            .texture_ref(texture)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        pass.set_texture(texture)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                        let selected_sampler = if matches!(draw.source, DrawSource::PixelTexture(_))
                        {
                            table
                                .sampler_ref(self.pixel_sampler)
                                .map_err(|_| Error::sgfx(Stage::EncodeCommands))?
                        } else {
                            sampler
                        };
                        pass.set_sampler(selected_sampler)
                            .map_err(|_| Error::sgfx(Stage::EncodeCommands))?;
                    }
                    pass.set_uniforms(DrawUniforms::new(
                        if draw.offset == [0., 0.] {
                            transform
                        } else {
                            translated_pixel_transform(self.width, self.height, draw.offset)?
                        },
                        white,
                    ))
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
        // A frame may update many independent cached pictures. Keep each
        // texture's complete updates in its own logical submission instead of
        // aggregating the whole frame into one potentially oversized stream.
        // Native splitting of an individual upload remains backend-owned.
        for uploads in frame.uploads.chunk_by(|a, b| a.texture == b.texture) {
            let mut encoder = CommandEncoder::new(&table);
            for upload in uploads {
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
        }
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

fn extension_context(paint: &PaintContext<'_>) -> PaintContext<'static> {
    let mut extensions = PaintContext::new();
    paint.visit_commands(&mut |command, origin| {
        if let PaintCommand::Extension { rect, payload } = command {
            extensions.draw_extension(
                scarlet_ui_core::geometry::Rect::new(
                    scarlet_ui_core::geometry::Point::new(
                        rect.origin.x + origin.x,
                        rect.origin.y + origin.y,
                    ),
                    rect.size,
                ),
                Arc::clone(payload),
            );
        }
    });
    extensions
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
        // Canvas meshes are also used for 2D tessellation, where contour
        // winding can legitimately differ between shapes and fonts.
        RasterState::new(sgfx::ir::CullMode::None, FrontFace::CounterClockwise),
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
        RasterState::new(sgfx::ir::CullMode::None, FrontFace::CounterClockwise),
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
        if format == TextureFormat::Nv12 {
            TextureUsage::SAMPLED
        } else {
            TextureUsage::SAMPLED | TextureUsage::COPY_DST
        },
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
    push_draw_phase(draws, tessellator, geometry, color, source, [0., 0.])
}

fn push_draw_phase(
    draws: &mut Vec<Draw>,
    tessellator: &mut Tessellator,
    geometry: GeometryRange,
    color: [f32; 4],
    source: DrawSource,
    snap_phase: [f32; 2],
) -> Result<()> {
    tessellator.color_geometry(geometry, color)?;
    if let Some(previous) = draws.last_mut() {
        let previous_end = previous
            .geometry
            .first_vertex
            .checked_add(previous.geometry.vertex_count);
        if previous.source == source
            && previous.snap_phase == snap_phase
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
    draws.push(Draw {
        geometry,
        source,
        vertex_buffer: None,
        offset: [0., 0.],
        snap_phase,
    });
    Ok(())
}

fn buffer_texture_capacity(width: u32, height: u32) -> Result<(u32, u32)> {
    if width == 0 || height == 0 {
        return Err(Error::InvalidFrame);
    }
    Ok((
        width
            .checked_next_power_of_two()
            .ok_or(Error::FrameTooComplex)?,
        height
            .checked_next_power_of_two()
            .ok_or(Error::FrameTooComplex)?,
    ))
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

fn rasterized_canvas_extent(logical: f32, scale: f32, raster_scale: f32) -> Result<u32> {
    if !raster_scale.is_finite() || raster_scale < 1.0 {
        return Err(Error::InvalidFrame);
    }
    physical_canvas_extent(logical, scale * raster_scale)
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

fn translated_pixel_transform(width: u32, height: u32, offset: [f32; 2]) -> Result<Transform> {
    if width == 0 || height == 0 {
        return Err(Error::InvalidFrame);
    }
    Transform::from_columns([
        2. / width as f32,
        0.,
        0.,
        0.,
        0.,
        -2. / height as f32,
        0.,
        0.,
        0.,
        0.,
        1.,
        0.,
        -1. + 2. * offset[0] / width as f32,
        1. - 2. * offset[1] / height as f32,
        0.,
        1.,
    ])
    .map_err(|_| Error::InvalidFrame)
}

fn scale_text_origin(value: f32, scale_milli: u32) -> i32 {
    let logical = value as i32;
    ((logical as i64).saturating_mul(scale_milli.max(1) as i64) / 1000) as i32
}

fn fractional(value: f32) -> f32 {
    value - libm::truncf(value)
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
    use crate::canvas::{SgfxCanvasDraw, SgfxCanvasVertex, SgfxMeshHandle, SgfxTextureHandle};
    use scarlet_ui_core::geometry::{Offset, Point, Rect, Size};
    use scarlet_ui_core::icon::{ALL_ICONS, IconStyle};
    use sgfx::ir::{Command, CommandBuffer};

    #[derive(Default)]
    struct RecordingExecutor {
        command_kinds: Vec<Vec<&'static str>>,
        draw_vertices: Vec<u32>,
        buffer_write_sizes: Vec<usize>,
        viewports: Vec<[f32; 6]>,
        scissors: Vec<Option<PixelRect>>,
        render_areas: Vec<PixelRect>,
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
                    Command::CopyBufferToBuffer { .. } => "copy-buffer",
                    Command::SetProgrammablePipeline(_) => "set-programmable-pipeline",
                    Command::SetBindGroup { .. } => "set-bind-group",
                    Command::BeginComputePass => "begin-compute-pass",
                    Command::EndComputePass => "end-compute-pass",
                    Command::SetComputePipeline(_) => "set-compute-pipeline",
                    Command::Dispatch { .. } => "dispatch",
                    Command::ResourceBarrier(_) => "resource-barrier",
                    Command::WriteBuffer { data, .. } => {
                        self.buffer_write_sizes.push(data.len());
                        "write-buffer"
                    }
                    Command::WriteTexture { .. } => "write-texture",
                    Command::CopyTextureToTexture { .. } => "copy",
                    Command::BlitTexture { .. } => "blit-texture",
                    Command::BeginRenderPass(desc) => {
                        self.render_areas.push(desc.area());
                        "begin-pass"
                    }
                    Command::EndRenderPass => "end-pass",
                    Command::SetPipeline(_) => "set-pipeline",
                    Command::SetVertexBuffer { .. } => "set-vertex-buffer",
                    Command::SetVertexBufferSlot { .. } => "set-vertex-buffer-slot",
                    Command::SetIndexBuffer { .. } => "set-index-buffer",
                    Command::SetTexture(_) => "set-texture",
                    Command::SetSampler(_) => "set-sampler",
                    Command::SetUniforms(_) => "set-uniforms",
                    Command::SetScissor(rect) => {
                        self.scissors.push(*rect);
                        "set-scissor"
                    }
                    Command::SetViewport(viewport) => {
                        self.viewports.push(viewport.components());
                        "set-viewport"
                    }
                    Command::SetPushConstants { .. } => "set-push-constants",
                    Command::Draw { vertex_count, .. } => {
                        self.draw_vertices.push(*vertex_count);
                        "draw"
                    }
                    Command::DrawInstanced { .. } => "draw-instanced",
                    Command::DrawIndexed { .. } => "draw-indexed",
                    Command::DrawIndexedInstanced { .. } => "draw-indexed-instanced",
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
    fn changing_raster_buffer_dimensions_do_not_exhaust_historical_texture_slots() {
        let mut encoder = SgfxPaintEncoder::new(256, 256, false).unwrap();
        for height in 1..=256 {
            // Gallery images and retained layer buffers may change dimensions
            // without changing the window. Every frame here owns just one image.
            let image = Buffer::from_dimensions(256, height);
            let mut paint = PaintContext::new();
            paint.draw_buffer_ref(Rect::from_xywh(0., 0., 256., 256.), &image);
            let mut executor = RecordingExecutor::default();
            encoder
                .encode_frame(
                    &mut executor,
                    0,
                    None,
                    &paint,
                    UiColor::BLACK,
                    1_000,
                    &[(0, 0, 256, 256)],
                )
                .unwrap_or_else(|error| {
                    panic!("A frame with one {height}-pixel image failed: {error:?}")
                });
            assert!(encoder.buffer_textures.len() <= MAX_BUFFER_TEXTURES);
            assert_eq!(executor.draw_vertices, [6, 3]);
        }
    }

    #[test]
    fn buffer_rect_scales_destination_independently_from_source_dpi_and_reuses_upload() {
        let image = Buffer::from_dimensions(320, 240);
        let mut paint = PaintContext::new();
        paint.draw_buffer_rect_ref(
            Rect::from_xywh(10.0, 10.0, 100.0, 80.0),
            Rect::from_xywh(40.0, 0.0, 240.0, 240.0),
            &image,
            1.0,
        );
        let mut encoder = SgfxPaintEncoder::new(800, 600, false).unwrap();
        let bounds = PixelBounds {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };
        let frame = encoder.lower_once(&paint, 2000, bounds).unwrap();
        let floats: Vec<_> = frame
            .vertex_bytes
            .chunks_exact(PAINT_VERTEX_STRIDE as usize)
            .take(6)
            .map(|vertex| {
                (
                    f32::from_le_bytes(vertex[0..4].try_into().unwrap()),
                    f32::from_le_bytes(vertex[4..8].try_into().unwrap()),
                    f32::from_le_bytes(vertex[32..36].try_into().unwrap()),
                    f32::from_le_bytes(vertex[36..40].try_into().unwrap()),
                )
            })
            .collect();
        assert_eq!(
            floats.iter().map(|v| v.0).fold(f32::INFINITY, f32::min),
            20.0
        );
        assert_eq!(
            floats.iter().map(|v| v.0).fold(f32::NEG_INFINITY, f32::max),
            220.0
        );
        assert_eq!(
            floats.iter().map(|v| v.1).fold(f32::NEG_INFINITY, f32::max),
            180.0
        );
        let texture = &encoder.buffer_textures[0];
        let expected_u = 280.0 / texture.capacity_width as f32;
        let expected_v = 240.0 / texture.capacity_height as f32;
        assert!((floats.iter().map(|v| v.2).fold(0.0, f32::max) - expected_u).abs() < 0.0001);
        assert!((floats.iter().map(|v| v.3).fold(0.0, f32::max) - expected_v).abs() < 0.0001);
        encoder.commit_texture_uploads(&frame);
        let warm = encoder.lower_once(&paint, 2000, bounds).unwrap();
        assert!(
            warm.uploads.is_empty(),
            "Unchanged source was uploaded again"
        );
    }

    #[test]
    fn buffer_texture_pool_accepts_128_live_images_and_rejects_129() {
        let images = (0..MAX_BUFFER_TEXTURES + 1)
            .map(|_| Buffer::from_dimensions(4, 4))
            .collect::<Vec<_>>();
        let mut encoder = SgfxPaintEncoder::new(8, 8, false).unwrap();
        let mut paint = PaintContext::new();
        for image in &images[..MAX_BUFFER_TEXTURES] {
            paint.draw_buffer_ref(Rect::from_xywh(0., 0., 4., 4.), image);
        }
        let mut accepted = RecordingExecutor::default();
        encoder
            .encode_frame(
                &mut accepted,
                0,
                None,
                &paint,
                UiColor::BLACK,
                1_000,
                &[(0, 0, 8, 8)],
            )
            .unwrap();
        assert_eq!(encoder.buffer_textures.len(), MAX_BUFFER_TEXTURES);
        assert_eq!(
            accepted
                .draw_vertices
                .iter()
                .filter(|&&count| count == 6)
                .count(),
            MAX_BUFFER_TEXTURES
        );

        paint.draw_buffer_ref(
            Rect::from_xywh(0., 0., 4., 4.),
            &images[MAX_BUFFER_TEXTURES],
        );
        let mut rejected = RecordingExecutor::default();
        assert!(matches!(
            encoder.encode_frame(
                &mut rejected,
                0,
                None,
                &paint,
                UiColor::BLACK,
                1_000,
                &[(0, 0, 8, 8)],
            ),
            Err(FrameError::Lowering(Error::FrameTooComplex))
        ));
        assert!(rejected.command_kinds.is_empty());
        assert_eq!(encoder.buffer_textures.len(), MAX_BUFFER_TEXTURES);
    }

    #[test]
    fn buffer_texture_reuse_selects_the_smallest_unused_capacity() {
        let small = Buffer::from_dimensions(17, 17);
        let large = Buffer::from_dimensions(33, 33);
        let mut encoder = SgfxPaintEncoder::new(64, 64, false).unwrap();
        encoder.advance_frame_serial();
        let small_texture = encoder.buffer_texture(&small).unwrap().0;
        let large_texture = encoder.buffer_texture(&large).unwrap().0;
        assert_ne!(small_texture, large_texture);
        encoder.advance_frame_serial();
        let replacement = Buffer::from_dimensions(12, 12);
        assert_eq!(
            encoder.buffer_texture(&replacement).unwrap().0,
            small_texture
        );
        // That slot is now occupied in this frame; another image uses the
        // larger capacity instead of overwriting a draw already being encoded.
        let second = Buffer::from_dimensions(12, 12);
        assert_eq!(encoder.buffer_texture(&second).unwrap().0, large_texture);
        assert_eq!(encoder.buffer_textures.len(), 2);
    }

    #[test]
    fn reused_buffer_uvs_and_linear_edges_ignore_stale_capacity_pixels() {
        let mut encoder = SgfxPaintEncoder::new(32, 32, false).unwrap();
        encoder.advance_frame_serial();
        let old = Buffer::from_dimensions(8, 8);
        let old_texture = encoder.buffer_texture(&old).unwrap().0;
        encoder.advance_frame_serial();
        let mut image = Buffer::from_dimensions(3, 2);
        for (index, pixel) in [
            0xff010203, 0xff111213, 0xff212223, 0xff313233, 0xff414243, 0xff515253,
        ]
        .into_iter()
        .enumerate()
        {
            image.set_pixel(index as u32 % 3, index as u32 / 3, pixel);
        }
        let mut tessellator =
            Tessellator::new(1_000, 32, 32, FloatRect::new(0., 0., 32., 32.)).unwrap();
        let mut mappings = Vec::new();
        let (geometry, texture, uploads) = encoder
            .lower_buffer(
                &mut tessellator,
                &mut mappings,
                &image,
                FloatRect::new(1., 0., 2., 2.),
                FloatRect::new(0., 0., 20., 20.),
            )
            .unwrap()
            .unwrap();
        assert_eq!(texture, old_texture);
        assert_eq!(encoder.buffer_textures.len(), 1);
        let vertices = &tessellator.vertices()[geometry.first_vertex as usize..]
            [..geometry.vertex_count as usize];
        let min_u = vertices
            .iter()
            .map(|v| v.tex_coord[0])
            .fold(f32::INFINITY, f32::min);
        let max_u = vertices.iter().map(|v| v.tex_coord[0]).fold(0., f32::max);
        let max_v = vertices.iter().map(|v| v.tex_coord[1]).fold(0., f32::max);
        assert_eq!((min_u, max_u, max_v), (1. / 8., 3. / 8., 2. / 8.));
        assert_eq!(uploads.len(), 3);
        assert_eq!((uploads[0].width, uploads[0].height), (3, 2));
        let right = &uploads[1];
        assert_eq!((right.x, right.y, right.width, right.height), (3, 0, 1, 2));
        assert_eq!(
            right.bytes.as_slice(),
            &[0x23, 0x22, 0x21, 0xff, 0x53, 0x52, 0x51, 0xff]
        );
        let bottom = &uploads[2];
        assert_eq!(
            (bottom.x, bottom.y, bottom.width, bottom.height),
            (0, 2, 4, 1)
        );
        assert_eq!(
            bottom.bytes.as_slice(),
            &[
                0x33, 0x32, 0x31, 0xff, 0x43, 0x42, 0x41, 0xff, 0x53, 0x52, 0x51, 0xff, 0x53, 0x52,
                0x51, 0xff
            ]
        );
        // At the normalized right/bottom UV, bilinear sampling touches the
        // original corner and its three duplicates, all from this new image.
        assert_eq!(
            &right.bytes.as_slice()[4..],
            &bottom.bytes.as_slice()[8..12]
        );
        assert_eq!(
            &bottom.bytes.as_slice()[8..12],
            &bottom.bytes.as_slice()[12..]
        );
        let (_, repeated_texture, repeated_uploads) = encoder
            .lower_buffer(
                &mut tessellator,
                &mut mappings,
                &image,
                FloatRect::new(0., 0., 3., 2.),
                FloatRect::new(0., 0., 3., 2.),
            )
            .unwrap()
            .unwrap();
        assert_eq!(repeated_texture, texture);
        assert!(repeated_uploads.is_empty());
    }

    #[test]
    fn failed_padded_buffer_upload_retries_image_and_edge_guards() {
        let buffer = Buffer::from_dimensions(3, 3);
        let mut paint = PaintContext::new();
        paint.draw_buffer_ref(Rect::from_xywh(0., 0., 3., 3.), &buffer);
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
        for _ in 0..2 {
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
        }
        assert_eq!(executor.texture_write_counts, [3, 3, 0, 0]);
    }

    #[test]
    fn frame_vertex_limit_reserves_the_mandatory_clear_draw() {
        let mut paint = PaintContext::new();
        let content_vertices = ((MAX_FRAME_VERTICES - 3) / 6) * 6;
        for _ in 0..content_vertices / 6 {
            paint.fill_rect(Rect::from_xywh(0., 0., 8., 8.), UiColor::WHITE);
        }
        let mut encoder = SgfxPaintEncoder::new(8, 8, false).unwrap();
        let mut executor = RecordingExecutor::default();
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
        assert_eq!(executor.draw_vertices, [(content_vertices + 3) as u32]);

        paint.fill_rect(Rect::from_xywh(0., 0., 8., 8.), UiColor::WHITE);
        let mut rejected = RecordingExecutor::default();
        assert!(matches!(
            encoder.encode_frame(
                &mut rejected,
                0,
                None,
                &paint,
                UiColor::BLACK,
                1_000,
                &[(0, 0, 8, 8)],
            ),
            Err(FrameError::Lowering(Error::FrameTooComplex))
        ));
        assert!(rejected.command_kinds.is_empty());
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
    fn streaming_nv12_recycles_external_slot_and_releases_old_source() {
        #[derive(Debug)]
        struct Image;
        let mut encoder = SgfxPaintEncoder::new(16, 16, false).unwrap();
        let mut first = None;
        let mut previous = None;
        for _ in 0..2048 {
            let source = Arc::new(Image);
            let texture = SgfxTexture::external_nv12(16, 16, source.clone());
            let mut paint = PaintContext::new();
            texture.paint(&mut paint, Rect::from_xywh(0.0, 0.0, 16.0, 16.0));
            for (slot, bound) in encoder.unused_external_textures(&paint) {
                assert!(bound);
                // Platform retirement is a precondition of this cache operation.
                encoder.retire_external_texture(slot).unwrap();
            }
            if let Some(old) = previous.take() {
                assert_eq!(Arc::strong_count(&old), 1);
            }
            let pending = encoder.prepare_external_textures(&paint).unwrap();
            assert_eq!(pending.len(), 1);
            let slot = pending[0].texture();
            if let Some(first) = first {
                assert_eq!(slot, first);
            } else {
                first = Some(slot);
            }
            encoder.mark_external_texture_bound(slot).unwrap();
            assert_eq!(encoder.canvas_textures.len(), 1);
            previous = Some(source);
        }
    }

    #[test]
    fn external_canvas_texture_requires_one_platform_import() {
        #[derive(Debug)]
        struct ExternalImage;

        let texture = SgfxTexture::external_bgra8(16, 16, Arc::new(ExternalImage));
        let mesh = SgfxMesh::new(alloc::vec![
            SgfxCanvasVertex::new([-1.0, -1.0, 0.0, 1.0], [1.0; 4]),
            SgfxCanvasVertex::new([1.0, -1.0, 0.0, 1.0], [1.0; 4]),
            SgfxCanvasVertex::new([0.0, 1.0, 0.0, 1.0], [1.0; 4]),
        ]);
        let frame = Arc::new(
            SgfxCanvasFrame::new(1, UiColor::BLACK)
                .draw(SgfxCanvasDraw::new(mesh, Transform::identity().columns()).texture(texture)),
        );
        let mut paint = PaintContext::new();
        paint.draw_extension(
            Rect::from_xywh(0.0, 0.0, 16.0, 16.0),
            Arc::new(SgfxCanvasPaint {
                handle: crate::canvas::SgfxCanvasHandle::new(),
                frame,
            }),
        );
        let mut encoder = SgfxPaintEncoder::new(16, 16, false).unwrap();

        let pending = encoder.prepare_external_textures(&paint).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].source().as_any().is::<ExternalImage>());
        encoder
            .mark_external_texture_bound(pending[0].texture())
            .unwrap();
        assert!(
            encoder
                .prepare_external_textures(&paint)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn canvas_raster_scale_expands_the_offscreen_target() {
        let frame = Arc::new(
            SgfxCanvasFrame::new(1, UiColor::BLACK)
                .raster_scale(2.0)
                .draw(SgfxCanvasDraw::new(
                    SgfxMesh::new(triangle(0.0)),
                    Transform::identity().columns(),
                )),
        );
        let mut paint = PaintContext::new();
        paint.draw_extension(
            Rect::from_xywh(0.0, 0.0, 16.0, 12.0),
            Arc::new(SgfxCanvasPaint {
                handle: crate::canvas::SgfxCanvasHandle::new(),
                frame,
            }),
        );
        let mut encoder = SgfxPaintEncoder::new(32, 24, false).unwrap();
        let mut executor = RecordingExecutor::default();

        encoder
            .prepare_canvases(&mut executor, &paint, 2_000)
            .unwrap();

        assert_eq!(encoder.canvas_targets.len(), 1);
        assert_eq!(encoder.canvas_targets[0].width, 64);
        assert_eq!(encoder.canvas_targets[0].height, 48);
    }

    #[test]
    fn external_gpu_surface_imports_once_and_skips_the_canvas_pass() {
        #[derive(Debug)]
        struct ExternalImage;

        let texture = SgfxTexture::external_bgra8(16, 16, Arc::new(ExternalImage));
        let mut paint = PaintContext::new();
        paint.draw_extension(
            Rect::from_xywh(2.0, 3.0, 12.0, 10.0),
            Arc::new(ExternalGpuSurfacePaint { texture }),
        );
        let mut encoder = SgfxPaintEncoder::new(16, 16, false).unwrap();

        let pending = encoder.prepare_external_textures(&paint).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].source().as_any().is::<ExternalImage>());
        encoder
            .mark_external_texture_bound(pending[0].texture())
            .unwrap();

        let mut executor = RecordingExecutor::default();
        encoder
            .encode_frame(
                &mut executor,
                0,
                None,
                &paint,
                UiColor::BLACK,
                1_000,
                &[(0, 0, 16, 16)],
            )
            .unwrap();

        assert!(encoder.canvas_targets.is_empty());
        assert_eq!(executor.command_kinds.len(), 1);
        assert_eq!(executor.draw_vertices, [6, 3]);
        assert!(
            encoder
                .prepare_external_textures(&paint)
                .unwrap()
                .is_empty()
        );
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
    fn repeated_canvas_resizes_do_not_exhaust_target_slots() {
        let handles = [
            crate::canvas::SgfxCanvasHandle::new(),
            crate::canvas::SgfxCanvasHandle::new(),
            crate::canvas::SgfxCanvasHandle::new(),
        ];
        let frame = Arc::new(SgfxCanvasFrame::new(1, UiColor::BLACK));
        let mut encoder = SgfxPaintEncoder::new(1024, 256, false).unwrap();
        let mut executor = RecordingExecutor::default();
        let mut textures = Vec::new();
        for step in 0..500 {
            let mut paint = PaintContext::new();
            for (row, handle) in handles.iter().copied().enumerate() {
                paint.draw_extension(
                    Rect::from_xywh(0.0, row as f32 * 84.0, 1000.0 - step as f32, 84.0),
                    Arc::new(SgfxCanvasPaint {
                        handle,
                        frame: Arc::clone(&frame),
                    }),
                );
            }
            encoder
                .encode_frame(
                    &mut executor,
                    0,
                    None,
                    &paint,
                    UiColor::BLACK,
                    1_000,
                    &[(0, 0, 1024, 256)],
                )
                .unwrap();
            assert_eq!(encoder.canvas_targets.len(), 3);
            let current: Vec<_> = encoder.canvas_targets.iter().map(|t| t.texture).collect();
            if step == 0 {
                textures = current;
            } else {
                assert_eq!(current, textures);
            }
            assert!(
                encoder
                    .canvas_targets
                    .iter()
                    .all(|t| t.width == 1000 - step)
            );
        }
    }

    #[test]
    fn canvas_capacity_grows_without_changing_existing_resource_definitions() {
        let mut encoder = SgfxPaintEncoder::new(128, 128, true).unwrap();
        let index = encoder.canvas_target(1, 33, 17, true).unwrap();
        let texture = encoder.canvas_targets[index].texture;
        let depth = encoder.canvas_targets[index].depth;
        assert_eq!(
            (
                encoder.canvas_targets[index].capacity_width,
                encoder.canvas_targets[index].capacity_height
            ),
            (64, 32)
        );
        let grown = encoder.canvas_target(1, 65, 10, true).unwrap();
        assert_eq!(grown, index);
        assert_eq!(encoder.canvas_targets.len(), 1);
        let target = &encoder.canvas_targets[grown];
        assert_eq!((target.capacity_width, target.capacity_height), (128, 32));
        assert_ne!(target.texture, texture);
        assert_ne!(target.depth, depth);
        // Prior submissions may still reference the immutable old definitions.
        assert!(encoder.table.texture_ref(texture).is_ok());
        assert!(encoder.table.texture_ref(depth.unwrap()).is_ok());
        assert!(matches!(
            encoder.canvas_target(1, 0, 10, true),
            Err(Error::InvalidFrame)
        ));
    }

    #[test]
    fn canvas_resize_invalidates_same_revision_and_crops_capacity_padding() {
        let handle = crate::canvas::SgfxCanvasHandle::new();
        let frame = Arc::new(SgfxCanvasFrame::new(1, UiColor::BLACK));
        let make_paint = |width, height| {
            let mut paint = PaintContext::new();
            paint.draw_extension(
                Rect::from_xywh(0.0, 0.0, width, height),
                Arc::new(SgfxCanvasPaint {
                    handle,
                    frame: Arc::clone(&frame),
                }),
            );
            paint
        };
        let mut encoder = SgfxPaintEncoder::new(64, 32, false).unwrap();
        let mut executor = RecordingExecutor::default();
        let first = make_paint(60.0, 30.0);
        encoder
            .prepare_canvases(&mut executor, &first, 1_000)
            .unwrap();
        let texture = encoder.canvas_targets[0].texture;
        assert_eq!(executor.command_kinds.len(), 1);
        encoder
            .prepare_canvases(&mut executor, &first, 1_000)
            .unwrap();
        assert_eq!(executor.command_kinds.len(), 1);
        let resized = make_paint(50.0, 25.0);
        encoder
            .prepare_canvases(&mut executor, &resized, 1_000)
            .unwrap();
        assert_eq!(executor.command_kinds.len(), 2);
        assert_eq!(encoder.canvas_targets[0].texture, texture);
        let lowered = encoder
            .lower_once(
                &resized,
                1_000,
                PixelBounds {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 32,
                },
            )
            .unwrap();
        let maximum_uv = |offset| {
            lowered
                .vertex_bytes
                .chunks_exact(40)
                .map(|vertex| f32::from_le_bytes(vertex[offset..offset + 4].try_into().unwrap()))
                .fold(0.0, f32::max)
        };
        assert_eq!(maximum_uv(32), 50.0 / 64.0);
        assert_eq!(maximum_uv(36), 25.0 / 32.0);
    }

    #[test]
    fn shared_canvas_handle_preserves_multiple_sizes_in_the_same_frame() {
        let handle = crate::canvas::SgfxCanvasHandle::new();
        let frame = Arc::new(SgfxCanvasFrame::new(1, UiColor::BLACK));
        let mut encoder = SgfxPaintEncoder::new(256, 64, false).unwrap();
        let mut executor = RecordingExecutor::default();
        for step in 0..100 {
            let mut paint = PaintContext::new();
            for width in [100.0 - (step % 50) as f32, 150.0 - (step % 50) as f32] {
                paint.draw_extension(
                    Rect::from_xywh(0.0, 0.0, width, 32.0),
                    Arc::new(SgfxCanvasPaint {
                        handle,
                        frame: Arc::clone(&frame),
                    }),
                );
            }
            encoder
                .prepare_canvases(&mut executor, &paint, 1_000)
                .unwrap();
            assert_eq!(encoder.canvas_targets.len(), 2);
            let lowered = encoder
                .lower_once(
                    &paint,
                    1_000,
                    PixelBounds {
                        x: 0,
                        y: 0,
                        width: 256,
                        height: 64,
                    },
                )
                .unwrap();
            assert_eq!(
                lowered
                    .draws
                    .iter()
                    .filter(|draw| matches!(draw.source, DrawSource::Texture(_)))
                    .count(),
                2
            );
        }
    }

    #[test]
    fn canvas_capacity_padding_does_not_expand_the_draw_viewport() {
        for scale in [1_000, 2_000] {
            for depth in [false, true] {
                for has_draws in [false, true] {
                    let handle = crate::canvas::SgfxCanvasHandle::new();
                    let mut encoder = SgfxPaintEncoder::new(1024, 256, depth).unwrap();
                    for width in [300., 250., 340.] {
                        let mut frame = SgfxCanvasFrame::new(1, UiColor::BLACK);
                        if depth {
                            frame = frame.depth_tested();
                        }
                        if has_draws {
                            frame = frame.draw(SgfxCanvasDraw::new(
                                SgfxMesh::new(triangle(0.0)),
                                Transform::identity().columns(),
                            ));
                        }
                        let mut paint = PaintContext::new();
                        paint.draw_extension(
                            Rect::from_xywh(0., 0., width, 84.),
                            Arc::new(SgfxCanvasPaint {
                                handle,
                                frame: Arc::new(frame),
                            }),
                        );
                        let mut executor = RecordingExecutor::default();
                        encoder
                            .prepare_canvases(&mut executor, &paint, scale)
                            .unwrap();
                        let target = &encoder.canvas_targets[0];
                        let factor = scale as f32 / 1_000.;
                        assert_eq!(
                            executor.viewports,
                            [[0., 0., width * factor, 84. * factor, 0., 1.]]
                        );
                        assert_eq!(
                            executor.scissors,
                            [Some(
                                PixelRect::new(0, 0, target.width, target.height).unwrap()
                            )]
                        );
                        // Full-attachment clear permits depth-tested canvases on WGPU.
                        assert_eq!(
                            executor.render_areas,
                            [
                                PixelRect::new(0, 0, target.capacity_width, target.capacity_height)
                                    .unwrap()
                            ]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn canvas_draw_limit_keeps_the_requested_viewport_and_scissor() {
        let mut encoder = SgfxPaintEncoder::new(512, 128, true).unwrap();
        let target = encoder.canvas_target(1, 300, 84, true).unwrap();
        let mesh = SgfxMesh::new(triangle(0.0));
        let mut frame = SgfxCanvasFrame::new(1, UiColor::BLACK).depth_tested();
        for _ in 0..MAX_CANVAS_DRAWS {
            frame = frame.draw(SgfxCanvasDraw::new(
                Arc::clone(&mesh),
                Transform::identity().columns(),
            ));
        }
        let mut executor = RecordingExecutor::default();
        encoder
            .render_canvas(&mut executor, target, &frame)
            .unwrap();
        assert!(!executor.render_areas.is_empty());
        assert_eq!(executor.viewports.len(), executor.render_areas.len());
        assert_eq!(executor.scissors.len(), executor.render_areas.len());
        assert!(
            executor
                .viewports
                .iter()
                .all(|v| *v == [0., 0., 300., 84., 0., 1.])
        );
        assert!(
            executor
                .scissors
                .iter()
                .all(|s| *s == Some(PixelRect::new(0, 0, 300, 84).unwrap()))
        );
        assert_eq!(
            executor.draw_vertices.iter().sum::<u32>(),
            (MAX_CANVAS_DRAWS * 3) as u32
        );
    }

    #[test]
    fn retained_canvas_pass_is_limited_only_by_ir_commands() {
        let max_draws = (MAX_COMMANDS - CANVAS_PASS_COMMANDS) / MAX_CANVAS_DRAW_COMMANDS;
        let meshes = vec![0; max_draws];
        assert!(canvas_pass_reaches_frame_end(&meshes, 0, 0));

        let too_many_meshes = vec![0; max_draws + 1];
        assert!(!canvas_pass_reaches_frame_end(&too_many_meshes, 0, 0));
    }

    #[test]
    fn large_canvas_mesh_remains_one_persistent_buffer_and_one_logical_draw() {
        let mut encoder = SgfxPaintEncoder::new(32, 32, false).unwrap();
        let mut executor = RecordingExecutor::default();
        let mesh = SgfxMesh::with_handle(SgfxMeshHandle::new(), 1, triangle(0.0).repeat(20_000));
        let frame = SgfxCanvasFrame::new(1, UiColor::BLACK)
            .draw(SgfxCanvasDraw::new(mesh, Transform::identity().columns()));
        let target = encoder.canvas_target(1, 32, 32, false).unwrap();
        encoder
            .render_canvas(&mut executor, target, &frame)
            .unwrap();
        assert_eq!(executor.command_kinds.len(), 1);
        assert_eq!(executor.buffer_write_sizes, [2_400_000]);
        assert_eq!(executor.draw_vertices, [60_000]);
        encoder
            .render_canvas(&mut executor, target, &frame)
            .unwrap();
        assert_eq!(executor.command_kinds.len(), 2);
        assert_eq!(executor.buffer_write_sizes, [2_400_000]);
        assert_eq!(executor.draw_vertices, [60_000, 60_000]);
    }

    #[test]
    fn dynamic_canvas_texture_reuploads_one_retained_gpu_texture() {
        let mut encoder = SgfxPaintEncoder::new(32, 32, false).unwrap();
        let mut executor = RecordingExecutor::default();
        let handle = SgfxTextureHandle::new();
        let mesh = SgfxMesh::new(triangle(0.0));
        let make_frame = |revision, value| {
            SgfxCanvasFrame::new(revision, UiColor::BLACK).draw(
                SgfxCanvasDraw::new(Arc::clone(&mesh), Transform::identity().columns()).texture(
                    SgfxTexture::rgba8_with_handle(handle, revision, 2, 2, vec![value; 16]),
                ),
            )
        };
        let target = encoder.canvas_target(1, 32, 32, false).unwrap();
        let first = make_frame(1, 0);
        encoder
            .render_canvas(&mut executor, target, &first)
            .unwrap();
        encoder
            .render_canvas(&mut executor, target, &first)
            .unwrap();
        let second = make_frame(2, 255);
        encoder
            .render_canvas(&mut executor, target, &second)
            .unwrap();

        assert_eq!(encoder.canvas_textures.len(), 1);
        assert_eq!(encoder.canvas_textures[0].revision, 2);
        assert_eq!(
            executor
                .command_kinds
                .iter()
                .flatten()
                .filter(|kind| **kind == "write-texture")
                .count(),
            2
        );
    }

    #[test]
    fn discarded_canvas_contents_are_invalidated_without_reuploading_retired_meshes() {
        let mut encoder = SgfxPaintEncoder::new(32, 32, false).unwrap();
        let mut executor = RecordingExecutor::default();
        let mesh = SgfxMesh::with_handle(SgfxMeshHandle::new(), 1, triangle(0.0));
        let frame = SgfxCanvasFrame::new(1, UiColor::BLACK)
            .draw(SgfxCanvasDraw::new(mesh, Transform::identity().columns()));
        let target = encoder.canvas_target(1, 32, 32, false).unwrap();
        encoder
            .render_canvas(&mut executor, target, &frame)
            .unwrap();
        encoder.canvas_targets[target].initialized = true;
        encoder.canvas_targets[target].revision = 1;
        encoder.discard_frame();
        assert!(!encoder.canvas_targets[target].initialized);
        assert!(encoder.canvas_meshes[0].uploaded);
        encoder
            .render_canvas(&mut executor, target, &frame)
            .unwrap();
        assert_eq!(executor.buffer_write_sizes, [120]);
        assert_eq!(executor.draw_vertices, [3, 3]);
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

    #[test]
    fn independent_pictures_submit_separately_without_splitting_their_uploads() {
        struct LimitedExecutor {
            writes: Vec<(u32, u32, usize)>,
            drew: bool,
            reject_second: bool,
            batches: usize,
        }
        impl CommandExecutor for LimitedExecutor {
            type Error = ();
            fn execute<'r, 'data>(
                &mut self,
                commands: &CommandBuffer<'r, 'data>,
            ) -> core::result::Result<(), ()> {
                let mut bytes = 0;
                for command in commands.commands() {
                    match command {
                        Command::WriteTexture { write, .. } => {
                            assert!(!self.drew);
                            let bounds = write.destination();
                            self.writes
                                .push((bounds.width(), bounds.height(), write.data().len()));
                            bytes += write.data().len();
                        }
                        Command::BeginRenderPass(_) => {
                            assert_eq!(self.writes.len(), 4);
                            self.drew = true;
                        }
                        _ => {}
                    }
                }
                assert!(
                    bytes <= 16 * 1024 * 1024,
                    "independent pictures must not be aggregated into an oversized stream"
                );
                if bytes > 0 {
                    self.batches += 1;
                }
                if self.reject_second && self.batches == 2 {
                    return Err(());
                }
                Ok(())
            }
        }
        let first = Buffer::from_dimensions(2048, 1100);
        let second = Buffer::from_dimensions(2048, 1100);
        let mut paint = PaintContext::new();
        paint.draw_buffer_ref(Rect::from_xywh(0.0, 0.0, 2048.0, 1100.0), &first);
        paint.draw_buffer_ref(Rect::from_xywh(32.0, 0.0, 2048.0, 1100.0), &second);
        let mut encoder = SgfxPaintEncoder::new(64, 32, false).unwrap();
        let mut rejected = LimitedExecutor {
            writes: Vec::new(),
            drew: false,
            reject_second: true,
            batches: 0,
        };
        assert!(matches!(
            encoder.encode_frame(
                &mut rejected,
                0,
                None,
                &paint,
                UiColor::BLACK,
                1000,
                &[(0, 0, 64, 32)]
            ),
            Err(FrameError::Execution(()))
        ));
        assert!(!rejected.drew);
        assert!(
            encoder
                .buffer_textures
                .iter()
                .all(|texture| texture.upload_state == TextureUploadState::Pending)
        );
        let mut executor = LimitedExecutor {
            writes: Vec::new(),
            drew: false,
            reject_second: false,
            batches: 0,
        };
        encoder
            .encode_frame(
                &mut executor,
                0,
                None,
                &paint,
                UiColor::BLACK,
                1000,
                &[(0, 0, 64, 32)],
            )
            .unwrap();
        assert_eq!(
            executor.writes,
            [
                (2048, 1100, first.data().len()),
                (2048, 1, 2048 * 4),
                (2048, 1100, second.data().len()),
                (2048, 1, 2048 * 4),
            ]
        );
        assert!(executor.drew);
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
    fn one_canvas_frame_rejects_mixed_texture_revisions_of_a_handle() {
        let handle = SgfxTextureHandle::new();
        let transform = Transform::identity().columns();
        let mesh = SgfxMesh::new(triangle(0.0));
        let texture =
            |revision| SgfxTexture::rgba8_with_handle(handle, revision, 1, 1, vec![255; 4]);
        let valid = SgfxCanvasFrame::new(1, UiColor::BLACK)
            .draw(SgfxCanvasDraw::new(Arc::clone(&mesh), transform).texture(texture(4)))
            .draw(SgfxCanvasDraw::new(Arc::clone(&mesh), transform).texture(texture(4)));
        assert!(!canvas_frame_has_revision_conflict(&valid));

        let invalid = valid.draw(SgfxCanvasDraw::new(mesh, transform).texture(texture(5)));
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
