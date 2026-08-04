//! Background View Modifier
//!
//! Adds a background color behind a child view.

use crate::color::Color;
use crate::element::LayoutConstraints;
use crate::element::{Element, ElementRenderObject, RenderElement};
use crate::geometry::{Point, Rect, Size};
use crate::renderer::PaintContext;
use crate::view::View;
use alloc::boxed::Box;
use alloc::vec;
use core::any::Any;

/// Background view modifier - adds a background color
#[derive(Clone)]
pub struct Background<V: View> {
    inner: V,
    color: Color,
}

impl<V: View> Background<V> {
    /// Create a new Background modifier
    pub fn new(inner: V, color: Color) -> Self {
        Self { inner, color }
    }

    /// Get the inner view
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Get the background color
    pub fn background_color(&self) -> Color {
        self.color
    }
}

impl<V: View + Clone> View for Background<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children(
            self.clone(),
            |view| BackgroundRenderObject::new(view.color),
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Background RenderObject
pub struct BackgroundRenderObject {
    color: Color,
    size: Size,
}

impl BackgroundRenderObject {
    /// Create a new BackgroundRenderObject
    pub fn new(color: Color) -> Self {
        Self {
            color,
            size: Size::ZERO,
        }
    }

    /// Get the background color
    pub fn get_color(&self) -> Color {
        self.color
    }
}

impl ElementRenderObject for BackgroundRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        // Background takes at least the minimum size
        let width = constraints.min_width.max(1.0);
        let height = constraints.min_height.max(1.0);

        self.size = Size { width, height };
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let mut child_size = Size::ZERO;
        for child in children {
            let size = child.layout(constraints);
            child_size.width = child_size.width.max(size.width);
            child_size.height = child_size.height.max(size.height);
            child.set_position(Point::ZERO);
        }

        // A child can report zero on one unconstrained axis while retaining a
        // meaningful size on the other axis. Keep that meaningful dimension:
        // replacing the whole size with the fallback would, for example,
        // collapse a fixed-height fill-width view to one pixel during a stack's
        // measurement pass.
        self.size = constraints.constrain(Size {
            width: child_size.width.max(1.0),
            height: child_size.height.max(1.0),
        });
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: Point) -> bool {
        let bounds = crate::geometry::Rect {
            origin: Point::ZERO,
            size: self.size,
        };
        bounds.contains(point)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        // Modifier doesn't directly render - child handles its own rendering
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        ctx.fill_rect(Rect::new(origin, self.size), self.color);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::ViewExt;
    use crate::views::Spacer;

    #[test]
    fn preserves_the_nonzero_child_axis_during_loose_measurement() {
        let mut element = Background::new(Spacer::new().frame(f32::INFINITY, 450.0), Color::WHITE)
            .create_element();

        let size = element.layout(LayoutConstraints::new(0.0, f32::INFINITY, 0.0, 564.0));

        assert_eq!(size, Size::new(1.0, 450.0));
    }
}
