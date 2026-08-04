//! Header bar view for application content areas.
//!
//! `HeaderBar` is intentionally content-agnostic: callers compose the title,
//! navigation buttons, search field, and toolbar actions themselves and pass
//! the resulting view to this wrapper. `NavigationView::header` uses the same
//! pattern for a header above its selected page.

use crate::color::{Color, ColorPalette};
use crate::element::{
    Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult,
};
use crate::geometry::{Point, Size};
use crate::renderer::PaintContext;
use crate::view::View;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;

/// A reusable application header containing an arbitrary composed view.
///
/// The content is laid out at the full available width and a fixed logical
/// height. This makes the component suitable for window headers, file
/// manager toolbars, and breadcrumb/navigation rows.
#[derive(Clone)]
pub struct HeaderBar<C: View + Clone> {
    content: C,
    height: f32,
    background: Color,
    border: Color,
}

impl<C: View + Clone> HeaderBar<C> {
    /// Create a header bar around `content`.
    ///
    /// # Arguments
    ///
    /// * `content` - View rendered inside the header.
    ///
    /// # Returns
    ///
    /// A header bar with the platform secondary background and border colors.
    pub fn new(content: C) -> Self {
        let palette = ColorPalette::default();
        Self {
            content,
            height: 48.0,
            background: palette.background_secondary(),
            border: palette.border(),
        }
    }

    /// Set the header height in logical pixels.
    ///
    /// # Arguments
    ///
    /// * `height` - Desired header height.
    ///
    /// # Returns
    ///
    /// The updated header bar.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(0.0);
        self
    }

    /// Set the header background color.
    ///
    /// # Arguments
    ///
    /// * `color` - Background color used behind the content.
    ///
    /// # Returns
    ///
    /// The updated header bar.
    pub fn surface(mut self, color: Color) -> Self {
        self.background = color;
        self
    }

    /// Set the one-pixel separator color.
    ///
    /// # Arguments
    ///
    /// * `color` - Color of the lower separator line.
    ///
    /// # Returns
    ///
    /// The updated header bar.
    pub fn separator(mut self, color: Color) -> Self {
        self.border = color;
        self
    }

    /// Return the configured header height.
    pub fn header_height(&self) -> f32 {
        self.height
    }
}

impl<C: View + Clone + 'static> View for HeaderBar<C> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children(
            self.clone(),
            |view| HeaderBarRenderObject::new(view.height, view.background, view.border),
            |view| vec![view.content.clone_view()],
        ))
    }

    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        self.content.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object used by [`HeaderBar`].
pub struct HeaderBarRenderObject {
    height: f32,
    background: Color,
    border: Color,
    size: Size,
}

impl HeaderBarRenderObject {
    /// Create a header bar render object.
    pub fn new(height: f32, background: Color, border: Color) -> Self {
        Self {
            height: height.max(0.0),
            background,
            border,
            size: Size::ZERO,
        }
    }
}

impl ElementRenderObject for HeaderBarRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        let width = if constraints.max_width.is_finite() {
            constraints.max_width.max(constraints.min_width)
        } else {
            constraints.min_width
        };
        let height = self
            .height
            .clamp(constraints.min_height, constraints.max_height);
        self.size = Size::new(width, height);
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let size = self.layout(constraints);
        if let Some(child) = children.get_mut(0) {
            child.layout(LayoutConstraints::tight(size.width, size.height));
            child.set_position(Point::ZERO);
        }
        size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn render(&mut self) {}

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        ctx.fill_rect(
            crate::geometry::Rect::from_xywh(origin.x, origin.y, self.size.width, self.size.height),
            self.background,
        );
        if self.size.height > 0.0 {
            ctx.fill_rect(
                crate::geometry::Rect::from_xywh(
                    origin.x,
                    origin.y + self.size.height - 1.0,
                    self.size.width,
                    1.0,
                ),
                self.border,
            );
        }
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn update(&mut self, _new_view: &dyn View) -> UpdateResult {
        // HeaderBar's content type is erased from this render-object type. The
        // owning RenderElement recreates only this render object while keeping
        // the child Element subtree intact.
        UpdateResult::Replaced
    }
}
