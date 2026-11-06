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
use crate::views::style::{self, ChromeStyle, GlassMaterial, SurfaceRole};
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
    material: Option<GlassMaterial>,
    corner_radius: f32,
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
    /// A Bubble Glass header. Appearance adds no padding or safe-area insets.
    pub fn new(content: C) -> Self {
        let palette = ColorPalette::default();
        Self {
            content,
            height: 48.0,
            background: style::surface_color(&palette, SurfaceRole::Structural),
            border: Color::CLEAR,
            material: Some(GlassMaterial::regular()),
            corner_radius: 16.0,
        }
    }

    /// Select Bubble Glass (default) or the original solid header and divider.
    /// This resets appearance colors, but preserves the configured shape/height.
    pub fn chrome_style(mut self, appearance: ChromeStyle) -> Self {
        let palette = ColorPalette::default();
        self.background = style::surface_color(&palette, SurfaceRole::Structural);
        self.material = match appearance {
            ChromeStyle::BubbleGlass => Some(GlassMaterial::regular()),
            ChromeStyle::Simple => None,
        };
        self.border = if self.material.is_some() {
            Color::CLEAR
        } else {
            palette.divider()
        };
        self
    }

    /// Customize the glass material independently from the header's shape.
    pub fn material(mut self, material: GlassMaterial) -> Self {
        self.material = Some(material);
        self.border = Color::CLEAR;
        self
    }

    /// Set the glass outline's radius without adding layout padding.
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = if radius.is_finite() {
            radius.max(0.0)
        } else {
            0.0
        };
        self
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
        // Explicit fills, including CLEAR for externally styled headers, must
        // not retain a hidden second backdrop layer.
        self.material = None;
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
            |view| {
                let mut render =
                    HeaderBarRenderObject::new(view.height, view.background, view.border);
                render.material = view.material;
                render.corner_radius = view.corner_radius;
                render
            },
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
    material: Option<GlassMaterial>,
    corner_radius: f32,
    size: Size,
}

impl HeaderBarRenderObject {
    /// Create a header bar render object.
    pub fn new(height: f32, background: Color, border: Color) -> Self {
        Self {
            height: height.max(0.0),
            background,
            border,
            material: None,
            corner_radius: 0.0,
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

    fn reads_backdrop(&self) -> bool {
        self.material.is_some()
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let rect = crate::geometry::Rect::new(origin, self.size);
        if let Some(material) = self.material {
            style::paint_glass_material(ctx, rect, self.corner_radius, material);
        } else {
            style::chrome_surface(ctx, rect, self.background);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::renderer::PaintCommand;
    use crate::views::Spacer;

    #[test]
    fn default_glass_and_simple_headers_keep_the_same_content_bounds() {
        for appearance in [ChromeStyle::BubbleGlass, ChromeStyle::Simple] {
            let view = HeaderBar::new(crate::views::Rectangle::new())
                .chrome_style(appearance)
                .height(48.)
                .corner_radius(5.);
            let mut tree = crate::element::ElementTree::new();
            tree.set_root(view.create_element());
            tree.layout(LayoutConstraints::tight(300., 48.));
            let header = tree.root().unwrap();
            assert_eq!(header.bounds().size, Size::new(300., 48.));
            assert_eq!(
                header.children()[0].bounds(),
                Rect::from_xywh(0., 0., 300., 48.)
            );
            let render = header.render_object().unwrap();
            assert_eq!(
                render.reads_backdrop(),
                appearance == ChromeStyle::BubbleGlass
            );
            let mut paint = PaintContext::new();
            render.paint(&mut paint, Point::ZERO);
            assert_eq!(
                paint
                    .commands()
                    .iter()
                    .filter(|c| matches!(c, PaintCommand::BackdropFilterRoundedRect { .. }))
                    .count(),
                if appearance == ChromeStyle::BubbleGlass {
                    1
                } else {
                    0
                }
            );
            if appearance == ChromeStyle::BubbleGlass {
                assert!(paint.commands().iter().any(|c| matches!(c, PaintCommand::BackdropFilterRoundedRect { corner_radius, .. } if *corner_radius == 5.)));
            }
        }
        let default = HeaderBar::new(Spacer::new()).create_element();
        assert!(default.render_object().unwrap().reads_backdrop());
    }

    #[test]
    fn explicit_header_fills_do_not_keep_a_second_glass_layer() {
        let explicit = HeaderBar::new(Spacer::new())
            .surface(Color::CLEAR)
            .separator(Color::CLEAR);
        assert!(
            !explicit
                .create_element()
                .render_object()
                .unwrap()
                .reads_backdrop()
        );
        let material = GlassMaterial::regular().opacity(0.3).blur_radius(4.);
        assert_eq!(explicit.material(material).material, Some(material));
    }
}
