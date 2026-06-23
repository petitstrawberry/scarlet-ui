//! Repaint boundary view modifier.
//!
//! A repaint boundary asks the paint pipeline to cache the child subtree into
//! an offscreen buffer. Ancestor-only repaints can then composite that buffer
//! instead of walking and rasterizing the subtree again.

use crate::element::{Element, ElementRenderObject, LayoutConstraints, RenderElement};
use crate::geometry::{Point, Size};
use crate::view::View;
use alloc::boxed::Box;
use alloc::vec;
use core::any::Any;

/// Repaint boundary view modifier.
#[derive(Clone)]
pub struct RepaintBoundary<V: View> {
    inner: V,
    max_cache_pixels: Option<u64>,
    cache_nested_boundaries: bool,
}

impl<V: View> RepaintBoundary<V> {
    pub fn new(inner: V) -> Self {
        Self {
            inner,
            max_cache_pixels: None,
            cache_nested_boundaries: true,
        }
    }

    pub fn max_cache_pixels(mut self, pixels: u64) -> Self {
        self.max_cache_pixels = Some(pixels);
        self
    }

    pub fn cache_nested_boundaries(mut self, enabled: bool) -> Self {
        self.cache_nested_boundaries = enabled;
        self
    }
}

impl<V: View + Clone> View for RepaintBoundary<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_children(
            self.clone(),
            RepaintBoundaryRenderObject::new(self.max_cache_pixels, self.cache_nested_boundaries),
            vec![self.inner.create_element()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object for [`RepaintBoundary`].
pub struct RepaintBoundaryRenderObject {
    size: Size,
    max_cache_pixels: Option<u64>,
    cache_nested_boundaries: bool,
}

impl RepaintBoundaryRenderObject {
    pub fn new(max_cache_pixels: Option<u64>, cache_nested_boundaries: bool) -> Self {
        Self {
            size: Size::ZERO,
            max_cache_pixels,
            cache_nested_boundaries,
        }
    }
}

impl Default for RepaintBoundaryRenderObject {
    fn default() -> Self {
        Self::new(None, true)
    }
}

impl ElementRenderObject for RepaintBoundaryRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = constraints.constrain(Size::ZERO);
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        if let Some(child) = children.first_mut() {
            self.size = child.layout(constraints);
            child.set_position(Point::ZERO);
        } else {
            self.size = Size::ZERO;
        }
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn repaint_boundary_size(&self) -> Option<Size> {
        Some(self.size)
    }

    fn repaint_boundary_max_cache_pixels(&self) -> Option<u64> {
        self.max_cache_pixels
    }

    fn repaint_boundary_cache_nested_boundaries(&self) -> bool {
        self.cache_nested_boundaries
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {}
}
