//! Retained SGFX content embedded in the ScarletUI paint order.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;
use core::sync::atomic::{AtomicU64, Ordering};

use scarlet_ui_core::color::Color;
use scarlet_ui_core::element::{Element, ElementRenderObject, LayoutConstraints, UpdateResult};
use scarlet_ui_core::geometry::{Point, Rect, Size};
use scarlet_ui_core::renderer::PaintContext;
use scarlet_ui_core::state::{Listenable, State};
use scarlet_ui_core::view::View;

static NEXT_CANVAS_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_MESH_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_TEXTURE_ID: AtomicU64 = AtomicU64::new(1);

/// Stable identity for retained resources owned by one SGFX canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SgfxCanvasHandle(u64);

impl SgfxCanvasHandle {
    /// Allocate a stable canvas identity.
    ///
    /// # Returns
    ///
    /// A handle suitable for reuse across ScarletUI view rebuilds.
    pub fn new() -> Self {
        let id = NEXT_CANVAS_ID.fetch_add(1, Ordering::Relaxed);
        Self(id.max(1))
    }

    pub(crate) const fn id(self) -> u64 {
        self.0
    }
}

impl Default for SgfxCanvasHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// One vertex consumed by the SGFX canvas vertex-color pipeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SgfxCanvasVertex {
    pub(crate) position: [f32; 4],
    pub(crate) color: [f32; 4],
    pub(crate) tex_coord: [f32; 2],
}

impl SgfxCanvasVertex {
    /// Construct a homogeneous-position, RGBA vertex.
    ///
    /// # Arguments
    ///
    /// * `position` - Homogeneous object-space position `[x, y, z, w]`.
    /// * `color` - Straight-alpha RGBA vertex color.
    ///
    /// # Returns
    ///
    /// A canvas vertex. Invalid non-finite values are rejected when rendered.
    pub const fn new(position: [f32; 4], color: [f32; 4]) -> Self {
        Self {
            position,
            color,
            tex_coord: [0.0, 0.0],
        }
    }

    /// Set the normalized texture coordinate.
    ///
    /// # Arguments
    ///
    /// * `tex_coord` - Normalized `[u, v]` coordinate.
    ///
    /// # Returns
    ///
    /// This vertex with the requested texture coordinate.
    pub const fn with_tex_coord(mut self, tex_coord: [f32; 2]) -> Self {
        self.tex_coord = tex_coord;
        self
    }
}

/// Immutable straight-alpha RGBA texture retained by the SGFX renderer.
#[derive(Debug)]
pub struct SgfxTexture {
    pub(crate) id: u64,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Arc<[u8]>,
}

impl SgfxTexture {
    /// Create an immutable RGBA8 texture.
    ///
    /// # Arguments
    ///
    /// * `width` - Texture width in pixels.
    /// * `height` - Texture height in pixels.
    /// * `pixels` - Tightly packed RGBA8 texels.
    ///
    /// # Returns
    ///
    /// A retained texture. Dimensions and byte length are validated when used.
    pub fn rgba8(width: u32, height: u32, pixels: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            id: NEXT_TEXTURE_ID.fetch_add(1, Ordering::Relaxed).max(1),
            width,
            height,
            pixels: pixels.into(),
        })
    }

    /// Return the texture width.
    ///
    /// # Returns
    ///
    /// Width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Return the texture height.
    ///
    /// # Returns
    ///
    /// Height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }
}

/// Immutable triangle-list mesh retained by the SGFX renderer.
#[derive(Debug)]
pub struct SgfxMesh {
    pub(crate) id: u64,
    pub(crate) vertices: Arc<[SgfxCanvasVertex]>,
}

impl SgfxMesh {
    /// Create an immutable triangle-list mesh.
    ///
    /// # Arguments
    ///
    /// * `vertices` - Vertices whose length must be a multiple of three.
    ///
    /// # Returns
    ///
    /// A retained mesh. Structural and numeric validation occurs when rendered.
    pub fn new(vertices: Vec<SgfxCanvasVertex>) -> Arc<Self> {
        Arc::new(Self {
            id: NEXT_MESH_ID.fetch_add(1, Ordering::Relaxed).max(1),
            vertices: vertices.into(),
        })
    }

    /// Return the triangle count.
    ///
    /// # Returns
    ///
    /// The number of complete triangles in this mesh.
    pub fn triangle_count(&self) -> usize {
        self.vertices.len() / 3
    }
}

/// One retained mesh draw in an SGFX canvas frame.
#[derive(Clone, Debug)]
pub struct SgfxCanvasDraw {
    pub(crate) mesh: Arc<SgfxMesh>,
    pub(crate) transform: [f32; 16],
    pub(crate) tint: Color,
    pub(crate) texture: Option<Arc<SgfxTexture>>,
}

impl SgfxCanvasDraw {
    /// Draw a mesh with a column-major transform.
    ///
    /// # Arguments
    ///
    /// * `mesh` - Immutable mesh to draw.
    /// * `transform` - Column-major 4×4 object-to-clip transform.
    ///
    /// # Returns
    ///
    /// A white-tinted mesh draw.
    pub fn new(mesh: Arc<SgfxMesh>, transform: [f32; 16]) -> Self {
        Self {
            mesh,
            transform,
            tint: Color::WHITE,
            texture: None,
        }
    }

    /// Multiply the mesh colors by a tint.
    ///
    /// # Arguments
    ///
    /// * `tint` - Straight-alpha RGBA multiplier.
    ///
    /// # Returns
    ///
    /// This draw with the requested tint.
    pub fn tint(mut self, tint: Color) -> Self {
        self.tint = tint;
        self
    }

    /// Sample an RGBA texture using the mesh texture coordinates.
    ///
    /// # Arguments
    ///
    /// * `texture` - Immutable retained RGBA8 texture.
    ///
    /// # Returns
    ///
    /// This draw configured for textured rendering.
    pub fn texture(mut self, texture: Arc<SgfxTexture>) -> Self {
        self.texture = Some(texture);
        self
    }
}

/// Immutable snapshot of one SGFX canvas frame.
#[derive(Clone, Debug)]
pub struct SgfxCanvasFrame {
    pub(crate) revision: u64,
    pub(crate) clear_color: Color,
    pub(crate) draws: Vec<SgfxCanvasDraw>,
}

impl SgfxCanvasFrame {
    /// Create an empty frame snapshot.
    ///
    /// # Arguments
    ///
    /// * `revision` - Application-controlled revision that changes when content changes.
    /// * `clear_color` - Color used to clear the offscreen target.
    ///
    /// # Returns
    ///
    /// An empty frame ready to receive retained draws.
    pub fn new(revision: u64, clear_color: Color) -> Self {
        Self {
            revision,
            clear_color,
            draws: Vec::new(),
        }
    }

    /// Append a retained mesh draw.
    ///
    /// # Arguments
    ///
    /// * `draw` - Mesh draw to append in paint order.
    ///
    /// # Returns
    ///
    /// This frame with the draw appended.
    pub fn draw(mut self, draw: SgfxCanvasDraw) -> Self {
        self.draws.push(draw);
        self
    }

    /// Return the number of mesh draws.
    ///
    /// # Returns
    ///
    /// The draw count encoded by this snapshot.
    pub fn draw_count(&self) -> usize {
        self.draws.len()
    }
}

/// ScarletUI view that embeds a retained SGFX frame.
#[derive(Clone, Debug)]
pub struct SgfxCanvas {
    handle: SgfxCanvasHandle,
    size: Size,
    source: SgfxCanvasSource,
    placeholder: Color,
}

#[derive(Clone, Debug)]
enum SgfxCanvasSource {
    Snapshot(Arc<SgfxCanvasFrame>),
    State(State<Arc<SgfxCanvasFrame>>),
}

impl SgfxCanvasSource {
    fn frame(&self) -> Arc<SgfxCanvasFrame> {
        match self {
            Self::Snapshot(frame) => Arc::clone(frame),
            Self::State(frame) => frame.get(),
        }
    }
}

impl SgfxCanvas {
    /// Create an SGFX canvas view.
    ///
    /// # Arguments
    ///
    /// * `handle` - Stable identity reused across view rebuilds.
    /// * `width` - Preferred logical width.
    /// * `height` - Preferred logical height.
    /// * `frame` - Immutable retained frame snapshot.
    ///
    /// # Returns
    ///
    /// A canvas view embedded in normal ScarletUI layout and paint order.
    pub fn new(
        handle: SgfxCanvasHandle,
        width: f32,
        height: f32,
        frame: Arc<SgfxCanvasFrame>,
    ) -> Self {
        Self {
            handle,
            size: Size::new(width, height),
            source: SgfxCanvasSource::Snapshot(frame),
            placeholder: Color::rgb(0.025, 0.035, 0.055),
        }
    }

    /// Create a state-driven SGFX canvas view.
    ///
    /// Changes to the state invalidate this canvas while preserving its handle
    /// and retained mesh resources.
    ///
    /// # Arguments
    ///
    /// * `handle` - Stable identity reused across view rebuilds.
    /// * `width` - Preferred logical width.
    /// * `height` - Preferred logical height.
    /// * `frame` - Reactive immutable frame snapshot.
    ///
    /// # Returns
    ///
    /// A reactive canvas view embedded in normal ScarletUI paint order.
    pub fn from_state(
        handle: SgfxCanvasHandle,
        width: f32,
        height: f32,
        frame: State<Arc<SgfxCanvasFrame>>,
    ) -> Self {
        Self {
            handle,
            size: Size::new(width, height),
            source: SgfxCanvasSource::State(frame),
            placeholder: Color::rgb(0.025, 0.035, 0.055),
        }
    }

    /// Set the color visible when the active paint backend is not SGFX.
    ///
    /// # Arguments
    ///
    /// * `color` - Placeholder fill color.
    ///
    /// # Returns
    ///
    /// This canvas with the requested placeholder.
    pub fn placeholder(mut self, color: Color) -> Self {
        self.placeholder = color;
        self
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SgfxCanvasPaint {
    pub(crate) handle: SgfxCanvasHandle,
    pub(crate) frame: Arc<SgfxCanvasFrame>,
}

impl View for SgfxCanvas {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui_core::element::RenderElement::new(
            self.clone(),
            SgfxCanvasRenderObject {
                handle: self.handle,
                size: self.size,
                source: self.source.clone(),
                placeholder: self.placeholder,
            },
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        match &self.source {
            SgfxCanvasSource::Snapshot(_) => Vec::new(),
            SgfxCanvasSource::State(frame) => alloc::vec![frame as &dyn Listenable],
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object used by [`SgfxCanvas`].
pub struct SgfxCanvasRenderObject {
    handle: SgfxCanvasHandle,
    size: Size,
    source: SgfxCanvasSource,
    placeholder: Color,
}

impl ElementRenderObject for SgfxCanvasRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        let width = self
            .size
            .width
            .clamp(constraints.min_width, constraints.max_width);
        let height = self
            .size
            .height
            .clamp(constraints.min_height, constraints.max_height);
        self.size = Size::new(width, height);
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn render(&mut self) {}

    fn paint<'a>(&'a self, ctx: &mut PaintContext<'a>, origin: Point) -> bool {
        let rect = Rect::new(origin, self.size);
        ctx.fill_rect(rect, self.placeholder);
        ctx.draw_extension(
            rect,
            Arc::new(SgfxCanvasPaint {
                handle: self.handle,
                frame: self.source.frame(),
            }),
        );
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn update(&mut self, new_view: &dyn View) -> UpdateResult {
        let Some(canvas) = new_view.as_any().downcast_ref::<SgfxCanvas>() else {
            return UpdateResult::Replaced;
        };
        if canvas.handle != self.handle {
            return UpdateResult::Replaced;
        }
        self.size = canvas.size;
        self.source = canvas.source.clone();
        self.placeholder = canvas.placeholder;
        UpdateResult::Updated
    }

    fn update_needs_layout(&self) -> bool {
        true
    }
}
