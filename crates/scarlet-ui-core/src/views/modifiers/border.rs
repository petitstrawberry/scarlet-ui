//! Border View Modifier
//!
//! Draws a border outline over a child view. The border is painted on top of
//! the child and kept inside the view's bounds. Clipping and borders are
//! separate concerns: combine `.clip_radius(r)` with `.border_rounded(_, _, r)`
//! (same radius) for a rounded view with a matching border.

use crate::color::Color;
use crate::element::{Element, ElementRenderObject, LayoutConstraints, RenderElement};
use crate::geometry::{EdgeInsets, Point, Rect, Size};
use crate::renderer::PaintContext;
use crate::view::View;
use alloc::boxed::Box;
use alloc::vec;
use core::any::Any;

/// Border view modifier - draws an outline over a child view.
#[derive(Clone)]
pub struct Border<V: View> {
    inner: V,
    color: Color,
    width: f32,
    corner_radius: f32,
}

impl<V: View> Border<V> {
    /// Create a new sharp-cornered border.
    pub fn new(inner: V, color: Color, width: f32) -> Self {
        Self {
            inner,
            color,
            width,
            corner_radius: 0.0,
        }
    }

    /// Create a new border with rounded corners.
    pub fn with_corner_radius(inner: V, color: Color, width: f32, corner_radius: f32) -> Self {
        Self {
            inner,
            color,
            width,
            corner_radius,
        }
    }

    /// Get the inner view.
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Get the border color.
    pub fn color(&self) -> Color {
        self.color
    }

    /// Get the border width.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Get the corner radius.
    pub fn corner_radius(&self) -> f32 {
        self.corner_radius
    }
}

impl<V: View + Clone> View for Border<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_children(
            self.clone(),
            BorderRenderObject::new(self.color, self.width, self.corner_radius),
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

/// Border RenderObject
pub struct BorderRenderObject {
    color: Color,
    width: f32,
    corner_radius: f32,
    size: Size,
}

impl BorderRenderObject {
    /// Create a new BorderRenderObject.
    pub fn new(color: Color, width: f32, corner_radius: f32) -> Self {
        Self {
            color,
            width,
            corner_radius,
            size: Size::ZERO,
        }
    }
}

impl ElementRenderObject for BorderRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        let width = constraints.min_width.max(0.0);
        let height = constraints.min_height.max(0.0);
        self.size = Size { width, height };
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        // A border draws over its child without consuming space.
        let mut size = Size::ZERO;
        for child in children {
            let child_size = child.layout(constraints);
            size.width = size.width.max(child_size.width);
            size.height = size.height.max(child_size.height);
            child.set_position(Point::ZERO);
        }
        self.size = constraints.constrain(size);
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: Point) -> bool {
        Rect {
            origin: Point::ZERO,
            size: self.size,
        }
        .contains(point)
    }

    fn paint_overlay(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
        if self.width <= 0.0 {
            return false;
        }
        // Inset by half the stroke width so the border stays fully inside the
        // element bounds (SwiftUI's strokeBorder semantics).
        let half = self.width * 0.5;
        let rect = Rect::new(origin, self.size).inset(EdgeInsets::all(half));
        if self.corner_radius > 0.0 {
            ctx.stroke_rounded_rect(rect, self.corner_radius, self.width, self.color);
        } else {
            ctx.stroke_rect(rect, self.width, self.color);
        }
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::LayoutConstraints;
    use crate::geometry::Point;
    use crate::renderer::{PaintCommand, PaintContext};

    fn sized_border(width: f32, corner_radius: f32) -> BorderRenderObject {
        let mut ro = BorderRenderObject::new(Color::rgb(255, 0, 0), width, corner_radius);
        ro.layout(LayoutConstraints::tight(100.0, 60.0));
        ro
    }

    #[test]
    fn sharp_border_emits_stroke_rect() {
        let ro = sized_border(2.0, 0.0);
        let mut ctx = PaintContext::new();
        assert!(ro.paint_overlay(&mut ctx, Point::ZERO));
        let [cmd] = ctx.commands() else {
            panic!("expected exactly one command, got {:?}", ctx.commands());
        };
        match cmd {
            PaintCommand::StrokeRect {
                rect,
                stroke_width,
                color,
            } => {
                assert_eq!(*stroke_width, 2.0);
                assert_eq!(*color, Color::rgb(255, 0, 0));
                assert_eq!(rect.origin, Point::new(1.0, 1.0));
                assert_eq!(rect.size, Size::new(98.0, 58.0));
            }
            other => panic!("expected StrokeRect, got {:?}", other),
        }
    }

    #[test]
    fn rounded_border_emits_stroke_rounded_rect() {
        let ro = sized_border(2.0, 8.0);
        let mut ctx = PaintContext::new();
        assert!(ro.paint_overlay(&mut ctx, Point::ZERO));
        match ctx.commands().first() {
            Some(PaintCommand::StrokeRoundedRect {
                corner_radius,
                stroke_width,
                ..
            }) => {
                assert_eq!(*corner_radius, 8.0);
                assert_eq!(*stroke_width, 2.0);
            }
            other => panic!("expected StrokeRoundedRect, got {:?}", other),
        }
    }

    #[test]
    fn zero_width_border_paints_nothing() {
        let ro = sized_border(0.0, 0.0);
        let mut ctx = PaintContext::new();
        assert!(!ro.paint_overlay(&mut ctx, Point::ZERO));
        assert!(ctx.commands().is_empty());
    }

    #[test]
    fn border_paints_at_absolute_origin() {
        let ro = sized_border(2.0, 0.0);
        let mut ctx = PaintContext::new();
        ro.paint_overlay(&mut ctx, Point::new(10.0, 20.0));
        match ctx.commands().first() {
            Some(PaintCommand::StrokeRect { rect, .. }) => {
                assert_eq!(rect.origin, Point::new(11.0, 21.0));
            }
            other => panic!("expected StrokeRect, got {:?}", other),
        }
    }
}
