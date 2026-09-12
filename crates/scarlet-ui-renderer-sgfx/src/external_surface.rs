//! External GPU images placed directly in the ScarletUI paint order.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use scarlet_ui_core::color::Color;
use scarlet_ui_core::element::{Element, ElementRenderObject, LayoutConstraints, UpdateResult};
use scarlet_ui_core::geometry::{Point, Rect, Size};
use scarlet_ui_core::renderer::PaintContext;
use scarlet_ui_core::state::{Listenable, State};
use scarlet_ui_core::view::View;

use crate::SgfxTexture;

/// ScarletUI view that composites an imported GPU image directly.
///
/// This is the lightweight path for an image produced by Vulkan, a video
/// decoder, or another GPU API. Unlike [`crate::SgfxCanvas`], it does not
/// allocate or render an intermediate canvas target. The active platform
/// backend must know how to import the [`SgfxTexture`]'s external image.
#[derive(Clone, Debug)]
pub struct ExternalGpuSurface {
    size: Size,
    source: ExternalGpuSurfaceSource,
    placeholder: Color,
}

#[derive(Clone, Debug)]
enum ExternalGpuSurfaceSource {
    Snapshot(Arc<SgfxTexture>),
    State(State<Arc<SgfxTexture>>),
}

impl ExternalGpuSurfaceSource {
    fn texture(&self) -> Arc<SgfxTexture> {
        match self {
            Self::Snapshot(texture) => Arc::clone(texture),
            Self::State(texture) => texture.get(),
        }
    }
}

impl ExternalGpuSurface {
    /// Create a surface backed by one external texture.
    ///
    /// Use [`Self::from_state`] when the producer updates the contents or
    /// rotates through multiple images.
    pub fn new(width: f32, height: f32, texture: Arc<SgfxTexture>) -> Self {
        Self {
            size: Size::new(width, height),
            source: ExternalGpuSurfaceSource::Snapshot(texture),
            placeholder: Color::rgb(0.025, 0.035, 0.055),
        }
    }

    /// Create a reactive surface backed by an external texture state.
    ///
    /// Set the state after the producer has finished a new image. Setting the
    /// same texture again is valid and invalidates the surface; alternating
    /// texture objects supports image queues once the platform supplies the
    /// corresponding producer/consumer synchronization contract.
    pub fn from_state(width: f32, height: f32, texture: State<Arc<SgfxTexture>>) -> Self {
        Self {
            size: Size::new(width, height),
            source: ExternalGpuSurfaceSource::State(texture),
            placeholder: Color::rgb(0.025, 0.035, 0.055),
        }
    }

    /// Set the color used when the active paint backend cannot draw the image.
    pub fn placeholder(mut self, color: Color) -> Self {
        self.placeholder = color;
        self
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ExternalGpuSurfacePaint {
    pub(crate) texture: Arc<SgfxTexture>,
}

impl View for ExternalGpuSurface {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui_core::element::RenderElement::new(
            self.clone(),
            ExternalGpuSurfaceRenderObject {
                size: self.size,
                source: self.source.clone(),
                placeholder: self.placeholder,
            },
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        match &self.source {
            ExternalGpuSurfaceSource::Snapshot(_) => Vec::new(),
            ExternalGpuSurfaceSource::State(texture) => alloc::vec![texture as &dyn Listenable],
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object used by [`ExternalGpuSurface`].
pub struct ExternalGpuSurfaceRenderObject {
    size: Size,
    source: ExternalGpuSurfaceSource,
    placeholder: Color,
}

impl ElementRenderObject for ExternalGpuSurfaceRenderObject {
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
            Arc::new(ExternalGpuSurfacePaint {
                texture: self.source.texture(),
            }),
        );
        true
    }

    fn emits_paint_extension(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn update(&mut self, new_view: &dyn View) -> UpdateResult {
        let Some(surface) = new_view.as_any().downcast_ref::<ExternalGpuSurface>() else {
            return UpdateResult::Replaced;
        };
        self.size = surface.size;
        self.source = surface.source.clone();
        self.placeholder = surface.placeholder;
        UpdateResult::Updated
    }

    fn update_needs_layout(&self) -> bool {
        true
    }
}
