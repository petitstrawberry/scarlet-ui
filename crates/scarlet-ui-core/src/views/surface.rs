//! Semantic surface container.
//!
//! `Surface` separates containment tone from elevation. A section can stay
//! flat, a card can opt into a restrained raised treatment, and transient or
//! modal content can use the stronger floating and overlay shadow stacks.

use alloc::boxed::Box;
use alloc::vec;
use core::any::Any;
use core::marker::PhantomData;

use crate::color::{Color, ColorPalette};
use crate::element::{
    Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult,
};
use crate::geometry::{EdgeInsets, Point, Rect, Size};
use crate::renderer::PaintContext;
use crate::view::View;

use super::style;
pub use super::style::{ChromeStyle, ElevationRole, GlassMaterial, SurfaceRole};

/// Reusable surface appearance, independent of geometry and layout.
///
/// Applying a style preserves the surface's radius, clipping, role and padding.
/// Bubble Glass uses the renderer's shared directional rim and refraction;
/// a separate elevation adds a cast shadow when needed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceStyle {
    material: GlassMaterial,
    elevation: ElevationRole,
}

impl SurfaceStyle {
    /// Bubble Glass appearance without imposing rounded corners or a shadow.
    pub fn bubble_glass() -> Self {
        Self {
            material: GlassMaterial::regular(),
            elevation: ElevationRole::Flat,
        }
    }

    /// Customize the material independently from the cast shadow.
    pub fn material(mut self, material: GlassMaterial) -> Self {
        self.material = material;
        self
    }

    /// Set a shared cast-shadow preset without affecting geometry.
    pub fn elevation(mut self, elevation: ElevationRole) -> Self {
        self.elevation = elevation;
        self
    }
}

/// A semantic background surface that does not add padding or move its child.
///
/// Containment and elevation are deliberately independent. `Surface::new`
/// starts flat; use [`Self::elevation`] only for content that must visually sit
/// above its parent plane.
#[derive(Clone)]
pub struct Surface<V: View> {
    inner: V,
    role: SurfaceRole,
    elevation: ElevationRole,
    corner_radius: f32,
    fill: Option<Color>,
    border_color: Option<Color>,
    bordered: bool,
    clip_content: bool,
    material: Option<GlassMaterial>,
}

impl<V: View> Surface<V> {
    /// Apply a reusable appearance while preserving this surface's shape.
    pub fn surface_style(mut self, style: SurfaceStyle) -> Self {
        self.elevation = style.elevation;
        self.material(style.material)
    }

    /// Create a flat semantic surface around `inner`.
    pub fn new(inner: V, role: SurfaceRole) -> Self {
        Self {
            inner,
            role,
            elevation: ElevationRole::Flat,
            corner_radius: style::surface_radius(role),
            fill: None,
            border_color: None,
            bordered: matches!(role, SurfaceRole::Floating | SurfaceRole::Overlay),
            clip_content: matches!(
                role,
                SurfaceRole::Section | SurfaceRole::Floating | SurfaceRole::Overlay
            ),
            material: None,
        }
    }

    /// Create an ordinary flat section surface.
    pub fn section(inner: V) -> Self {
        Self::new(inner, SurfaceRole::Section)
    }

    /// Create a menu/popover-style floating surface.
    pub fn floating(inner: V) -> Self {
        Self::new(inner, SurfaceRole::Floating)
            .elevation(ElevationRole::Floating)
            .material(GlassMaterial::floating())
    }

    /// Create a dialog/sheet-style overlay surface.
    pub fn overlay(inner: V) -> Self {
        Self::new(inner, SurfaceRole::Overlay)
            .elevation(ElevationRole::Overlay)
            .material(GlassMaterial::floating())
    }

    /// Set visual elevation without changing the surface's tonal role.
    pub fn elevation(mut self, elevation: ElevationRole) -> Self {
        self.elevation = elevation;
        if elevation == ElevationRole::Raised && self.fill.is_none() && self.material.is_none() {
            self.material = Some(GlassMaterial::regular());
        }
        self
    }

    /// Use the restrained raised-card treatment.
    pub fn raised(self) -> Self {
        self.elevation(ElevationRole::Raised)
    }

    /// Override the semantic fill color.
    pub fn fill(mut self, color: Color) -> Self {
        self.fill = Some(color);
        self.material = None;
        self
    }

    /// Use a real in-app backdrop blur and uniform tint instead of a solid fill.
    /// No decorative border is added; callers can opt into one afterward.
    pub fn material(mut self, material: GlassMaterial) -> Self {
        self.material = Some(material);
        self.bordered = false;
        self
    }

    /// Set whether a one-pixel semantic divider border is drawn.
    pub fn bordered(mut self, bordered: bool) -> Self {
        self.bordered = bordered;
        self
    }

    /// Override the semantic border color and enable the border.
    pub fn border_color(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self.bordered = true;
        self
    }

    /// Override the role's default corner radius.
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius.max(0.0);
        self
    }

    /// Set whether descendants are clipped to the surface shape.
    pub fn clip_content(mut self, clip: bool) -> Self {
        self.clip_content = clip;
        self
    }

    /// Return the contained view.
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Return the tonal surface role.
    pub fn role(&self) -> SurfaceRole {
        self.role
    }

    /// Return the independent elevation role.
    pub fn elevation_role(&self) -> ElevationRole {
        self.elevation
    }
}

impl<V: View + Clone> View for Surface<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children(
            self.clone(),
            SurfaceRenderObject::<V>::from_view,
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

struct SurfaceRenderObject<V: View> {
    role: SurfaceRole,
    elevation: ElevationRole,
    corner_radius: f32,
    fill: Option<Color>,
    border_color: Option<Color>,
    bordered: bool,
    clip_content: bool,
    material: Option<GlassMaterial>,
    size: Size,
    repaint_subtree: bool,
    marker: PhantomData<fn() -> V>,
}

impl<V: View> SurfaceRenderObject<V> {
    fn from_view(view: &Surface<V>) -> Self {
        Self {
            role: view.role,
            elevation: view.elevation,
            corner_radius: view.corner_radius,
            fill: view.fill,
            border_color: view.border_color,
            bordered: view.bordered,
            clip_content: view.clip_content,
            material: view.material,
            size: Size::ZERO,
            repaint_subtree: true,
            marker: PhantomData,
        }
    }

    fn resolved_fill(&self, palette: &ColorPalette) -> Color {
        self.fill
            .unwrap_or_else(|| style::surface_color(palette, self.role))
    }

    fn resolved_border(&self, palette: &ColorPalette) -> Color {
        self.border_color.unwrap_or_else(|| palette.divider())
    }

    fn rect(&self, origin: Point) -> Rect {
        Rect::new(origin, self.size)
    }
}

impl<V: View + Clone> ElementRenderObject for SurfaceRenderObject<V> {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = constraints.constrain(Size::new(
            constraints.min_width.max(0.0),
            constraints.min_height.max(0.0),
        ));
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
        self.size = constraints.constrain(child_size);
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: Point) -> bool {
        Rect::new(Point::ZERO, self.size).contains(point)
    }

    fn render(&mut self) {}

    fn reads_backdrop(&self) -> bool {
        self.material.is_some()
    }

    fn paint(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
        let rect = self.rect(origin);
        if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
            return false;
        }
        let palette = ColorPalette::default();
        let radius = style::radius_for(rect, self.corner_radius);
        let fill = self.resolved_fill(&palette);
        style::elevation_shadow(ctx, rect, radius, palette.shadow(), self.elevation);
        if let Some(material) = self.material {
            style::paint_glass_material(ctx, rect, radius, material);
        } else if self.elevation == ElevationRole::Raised {
            style::fill_raised_surface(ctx, rect, radius, fill, false);
        } else {
            ctx.fill_rounded_rect(rect, radius, fill);
        }
        true
    }

    fn paint_overlay(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
        if !self.bordered || self.size.width <= 0.0 || self.size.height <= 0.0 {
            return false;
        }
        let palette = ColorPalette::default();
        let half = style::metrics().border_width * 0.5;
        let rect = self.rect(origin).inset(EdgeInsets::all(half));
        ctx.stroke_rounded_rect(
            rect,
            style::radius_for(rect, (self.corner_radius - half).max(0.0)),
            style::metrics().border_width,
            self.resolved_border(&palette),
        );
        true
    }

    fn paint_outsets(&self) -> EdgeInsets {
        style::elevation_outsets(self.elevation)
    }

    fn clip_bounds(&self, origin: Point) -> Option<(Rect, f32)> {
        self.clip_content.then(|| {
            let rect = self.rect(origin);
            (rect, style::radius_for(rect, self.corner_radius))
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn can_defer_offscreen_paint(&self) -> bool {
        self.material.is_none()
    }

    fn update_needs_subtree_paint(&self) -> bool {
        // Fill and border colors affect only this surface. Keep its children's
        // retained text/geometry; clipping/material changes remain conservative.
        self.repaint_subtree
    }

    fn update(&mut self, new_view: &dyn View) -> UpdateResult {
        let Some(surface) = new_view.as_any().downcast_ref::<Surface<V>>() else {
            return UpdateResult::Replaced;
        };
        self.repaint_subtree = self.role != surface.role
            || self.elevation != surface.elevation
            || self.corner_radius != surface.corner_radius
            || self.bordered != surface.bordered
            || self.clip_content != surface.clip_content
            || self.material != surface.material;
        let changed = self.repaint_subtree
            || self.fill != surface.fill
            || self.border_color != surface.border_color;
        self.role = surface.role;
        self.elevation = surface.elevation;
        self.corner_radius = surface.corner_radius;
        self.fill = surface.fill;
        self.border_color = surface.border_color;
        self.bordered = surface.bordered;
        self.clip_content = surface.clip_content;
        self.material = surface.material;
        if changed {
            UpdateResult::Updated
        } else {
            UpdateResult::NoChange
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::{PaintCommand, PaintContext};
    use crate::view::ViewExt;
    use crate::views::Spacer;

    fn laid_out<V: View + Clone>(surface: &Surface<V>) -> SurfaceRenderObject<V> {
        let mut render = SurfaceRenderObject::from_view(surface);
        render.layout(LayoutConstraints::tight(160.0, 90.0));
        render
    }

    #[test]
    fn color_updates_only_repaint_self_but_clip_changes_repaint_children() {
        let old = Surface::section(Spacer::new())
            .fill(Color::RED)
            .border_color(Color::BLUE);
        let mut render = laid_out(&old);
        let color = old.clone().fill(Color::GREEN).border_color(Color::RED);
        assert!(matches!(render.update(&color), UpdateResult::Updated));
        assert!(!render.update_needs_subtree_paint());
        let shape = color.clone().corner_radius(3.0);
        assert!(matches!(render.update(&shape), UpdateResult::Updated));
        assert!(render.update_needs_subtree_paint());
        assert!(matches!(render.update(&shape), UpdateResult::NoChange));
    }

    #[test]
    fn section_is_flat_by_default() {
        let surface = Surface::section(Spacer::new());
        let render = laid_out(&surface);
        let mut ctx = PaintContext::new();

        assert!(render.paint(&mut ctx, Point::ZERO));
        assert!(matches!(
            ctx.commands(),
            [PaintCommand::FillRoundedRect { .. }]
        ));
        assert_eq!(render.paint_outsets(), EdgeInsets::ZERO);
    }

    #[test]
    fn reusable_appearance_preserves_shape_and_layout() {
        let material = GlassMaterial::regular().blur_radius(10.0).opacity(0.35);
        let style = SurfaceStyle::bubble_glass().material(material);
        let surface = Surface::section(Spacer::new().frame(120.0, 40.0))
            .corner_radius(3.0)
            .surface_style(style);
        let render = laid_out(&surface);
        assert_eq!(render.corner_radius, 3.0);
        assert!(render.clip_content);
        assert_eq!(render.material, Some(material));
        assert_eq!(render.elevation, ElevationRole::Flat);
        assert_eq!(
            surface
                .create_element()
                .layout(LayoutConstraints::new(0.0, 200.0, 0.0, 100.0)),
            Size::new(120.0, 40.0)
        );

        let square = Spacer::new().surface_style(style);
        assert_eq!(square.corner_radius, 0.0);
        assert!(!square.clip_content);
        let shadow = Spacer::new()
            .surface_shadow(ElevationRole::Raised)
            .corner_radius(3.0);
        assert_eq!(shadow.material, None);
        assert_eq!(shadow.fill, Some(Color::CLEAR));
        assert_eq!(shadow.elevation, ElevationRole::Raised);
    }

    #[test]
    fn material_controls_update_existing_render_objects() {
        let old = Spacer::new().glass(GlassMaterial::regular());
        let new = Spacer::new().glass(
            GlassMaterial::regular()
                .tint(Color::RED)
                .opacity(0.25)
                .blur_radius(4.0),
        );
        let mut render = laid_out(&old);
        assert!(matches!(render.update(&new), UpdateResult::Updated));
        assert_eq!(render.material.unwrap().blur_radius, 4.0);
        assert_eq!(render.material.unwrap().tint, Color::RED.with_opacity(0.25));
        assert!(matches!(render.update(&new), UpdateResult::NoChange));
        let bounded = GlassMaterial::regular().opacity(2.0).blur_radius(-1.0);
        assert_eq!(bounded.tint.a, 1.0);
        assert_eq!(bounded.blur_radius, 0.0);
        assert_eq!(GlassMaterial::regular().opacity(f32::NAN).tint.a, 0.0);
        assert_eq!(
            GlassMaterial::regular()
                .blur_radius(f32::INFINITY)
                .blur_radius,
            0.0
        );
    }

    #[test]
    fn raised_card_adds_soft_shadow_and_shared_material() {
        let surface = Surface::section(Spacer::new()).raised().bordered(true);
        let render = laid_out(&surface);
        let mut ctx = PaintContext::new();

        render.paint(&mut ctx, Point::ZERO);
        assert!(matches!(
            ctx.commands(),
            [
                PaintCommand::DrawRoundedRectShadow { .. },
                PaintCommand::BackdropFilterRoundedRect { .. }
            ]
        ));
        assert_eq!(
            render.paint_outsets(),
            style::elevation_outsets(ElevationRole::Raised)
        );
    }

    #[test]
    fn floating_and_overlay_use_distinct_depth_stacks() {
        let floating = laid_out(&Surface::floating(Spacer::new()));
        let overlay = laid_out(&Surface::overlay(Spacer::new()));
        let mut floating_ctx = PaintContext::new();
        let mut overlay_ctx = PaintContext::new();

        floating.paint(&mut floating_ctx, Point::ZERO);
        overlay.paint(&mut overlay_ctx, Point::ZERO);

        assert_eq!(floating_ctx.commands().len(), 3);
        assert_eq!(overlay_ctx.commands().len(), 3);
        assert_ne!(floating.paint_outsets(), overlay.paint_outsets());
        assert!(overlay.paint_outsets().bottom > floating.paint_outsets().bottom);
    }

    #[test]
    fn floating_surfaces_share_material_without_changing_section_defaults() {
        for surface in [
            Surface::floating(Spacer::new()),
            Surface::overlay(Spacer::new()),
        ] {
            let render = laid_out(&surface);
            let mut ctx = PaintContext::new();
            render.paint(&mut ctx, Point::ZERO);
            assert!(render.reads_backdrop());
            assert!(ctx.commands().iter().any(|command| matches!(command,
                PaintCommand::BackdropFilterRoundedRect { tint, .. }
                    if *tint == GlassMaterial::floating().tint)));
            assert!(!ctx.commands().iter().any(|command| matches!(
                command,
                PaintCommand::StrokeRoundedRect { .. }
                    | PaintCommand::FillVerticalGradientRoundedRect { .. }
            )));
        }
        assert!(!laid_out(&Surface::section(Spacer::new())).reads_backdrop());
        assert!(!laid_out(&Surface::floating(Spacer::new()).fill(Color::WHITE)).reads_backdrop());
    }

    #[test]
    fn surface_does_not_add_layout_padding() {
        let mut element =
            Surface::section(Spacer::new().frame(f32::INFINITY, 45.0)).create_element();

        let size = element.layout(LayoutConstraints::new(0.0, f32::INFINITY, 0.0, 100.0));

        assert_eq!(size, Size::new(0.0, 45.0));
    }

    #[test]
    fn section_clips_children_to_its_shape() {
        let surface = Surface::section(Spacer::new());
        let render = laid_out(&surface);
        let (rect, radius) = render.clip_bounds(Point::new(10.0, 20.0)).unwrap();

        assert_eq!(rect, Rect::from_xywh(10.0, 20.0, 160.0, 90.0));
        assert_eq!(radius, style::surface_radius(SurfaceRole::Section));
    }
}
