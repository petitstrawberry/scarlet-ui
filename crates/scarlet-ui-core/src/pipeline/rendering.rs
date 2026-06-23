//! RenderingPipeline - Integration of PipelineOwner, ElementTree, and Compositor
//!
//! RenderingPipeline is the main entry point for the rendering system.
//! It orchestrates all phases of the rendering pipeline.

#![allow(deprecated)]

use crate::buffer::Buffer;
use crate::compositor::DamageRect;
use crate::element::{Element, ElementId, ElementTree, LayoutConstraints};
use crate::event::EventDispatcher;
use crate::geometry::{Point, Rect, Size};
use crate::pipeline::{PipelineId, PipelineOwner};
use crate::renderer::{CpuPaintRenderer, CpuRenderer, FrameSize, PaintContext};
use crate::views::WindowInfo;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use alloc::vec::Vec;

const MAX_PRESENT_DAMAGE_RECTS: usize = 4;
const MAX_REPAINT_BOUNDARY_CACHE_PIXELS: u64 = 16_000_000;

struct PaintCache {
    buffer: Arc<Buffer>,
    logical_size: Size,
    scale_milli: u32,
    valid: bool,
    invalidated_by: Option<ElementId>,
}

/// RenderingPipeline integrates all components of the rendering system
pub struct RenderingPipeline {
    element_tree: ElementTree,
    pipeline_owner: PipelineOwner,
    renderer: Option<Box<dyn crate::renderer::Renderer>>,
    window_size: Size,
    scale_milli: u32,
    event_dispatcher: EventDispatcher,
    paint_renderer: Option<CpuPaintRenderer>,
    last_paint_bounds: BTreeMap<ElementId, Rect>,
    paint_damage: Option<Vec<DamageRect>>,
    paint_needs_full: bool,
    paint_background_color: Option<crate::color::Color>,
    paint_enabled: bool,
    paint_caches: BTreeMap<ElementId, PaintCache>,
}

impl RenderingPipeline {
    /// Create a new RenderingPipeline
    pub fn new() -> Self {
        Self::with_pipeline_id(PipelineId::generate())
    }

    /// Create a new RenderingPipeline with a stable owner ID.
    pub fn with_pipeline_id(pipeline_id: PipelineId) -> Self {
        Self {
            element_tree: ElementTree::with_pipeline_id(pipeline_id),
            pipeline_owner: PipelineOwner::with_pipeline_id(pipeline_id),
            renderer: None,
            window_size: Size::new(800.0, 600.0),
            scale_milli: 1000,
            event_dispatcher: EventDispatcher::new(),
            paint_renderer: None,
            last_paint_bounds: BTreeMap::new(),
            paint_damage: None,
            paint_needs_full: true,
            paint_background_color: None,
            paint_enabled: true,
            paint_caches: BTreeMap::new(),
        }
    }

    pub fn set_paint_enabled(&mut self, enabled: bool) {
        self.paint_enabled = enabled;
    }

    /// Return this pipeline's owner ID.
    pub const fn pipeline_id(&self) -> PipelineId {
        self.element_tree.pipeline_id()
    }

    /// Unmount the element tree and discard pending global dirty work.
    pub fn teardown(&mut self) {
        self.element_tree.clear_root();
        crate::pipeline::clear_global_dirty(self.pipeline_id());
        self.renderer = None;
        self.paint_renderer = None;
        self.last_paint_bounds.clear();
        self.paint_damage = None;
        self.paint_needs_full = true;
        self.paint_background_color = None;
        self.paint_caches.clear();
    }

    /// Set the output scale in milli-units.
    pub fn set_scale_milli(&mut self, scale_milli: u32) {
        self.scale_milli = scale_milli.max(1);
        crate::graphics::set_current_scale_milli(self.scale_milli);
        if let Some(ref mut renderer) = self.renderer {
            renderer.resize(FrameSize {
                width: self.window_size.width,
                height: self.window_size.height,
                scale_milli: self.scale_milli,
            });
        }
        if let Some(ref mut paint_renderer) = self.paint_renderer {
            paint_renderer.resize(self.window_size, self.scale_milli);
        }
        self.last_paint_bounds.clear();
        self.paint_damage = None;
        self.paint_needs_full = true;
        self.paint_caches.clear();
        if let Some(root) = self.element_tree.root_mut() {
            root.clear_buffers();
        }
        if let Some(root) = self.element_tree.root() {
            self.pipeline_owner.mark_needs_layout(root.id());
        }
    }

    /// Return the current output scale in milli-units.
    pub fn scale_milli(&self) -> u32 {
        self.scale_milli
    }

    /// Set the root Element
    pub fn set_root(&mut self, root_element: Box<dyn Element>) {
        self.element_tree.set_root(root_element);
        self.paint_caches.clear();
        if let Some(root) = self.element_tree.root() {
            self.event_dispatcher.set_root(root.id());
        }
    }

    /// Get the ElementTree
    pub fn element_tree(&self) -> &ElementTree {
        &self.element_tree
    }

    /// Get mutable reference to the ElementTree
    pub fn element_tree_mut(&mut self) -> &mut ElementTree {
        &mut self.element_tree
    }

    /// Get the PipelineOwner
    pub fn pipeline_owner(&self) -> &PipelineOwner {
        &self.pipeline_owner
    }

    /// Get mutable reference to the PipelineOwner
    pub fn pipeline_owner_mut(&mut self) -> &mut PipelineOwner {
        &mut self.pipeline_owner
    }

    /// Get the StateRegistry
    pub fn state_registry(&self) -> &crate::pipeline::StateRegistry {
        self.pipeline_owner.state_registry()
    }

    /// Get mutable reference to the StateRegistry
    pub fn state_registry_mut(&mut self) -> &mut crate::pipeline::StateRegistry {
        self.pipeline_owner.state_registry_mut()
    }

    /// Has any dirty elements?
    pub fn has_dirty(&self) -> bool {
        self.pipeline_owner.has_dirty()
    }

    /// Extract window information from the element tree
    ///
    /// This searches the element tree for a Window View and extracts
    /// the app_id, title, size, window type, background, and policies from it.
    ///
    /// Returns window information or defaults if no Window is found.
    fn extract_window_info(&self) -> WindowInfo {
        // Default values
        let default_info = WindowInfo::new(
            alloc::string::String::from("com.example.scarletui"),
            alloc::string::String::from("ScarletUI Application"),
            Size::new(800.0, 600.0),
            0,
            None,
            true,
            true,
            crate::color::ColorPalette::light().window_background(),
            true,
        );

        // Try to find a Window View in the element tree
        if let Some(root) = self.element_tree.root() {
            if let Some(window_info) = self.find_window_view(root) {
                return window_info;
            }
        }

        default_info
    }

    /// Recursively search for a Window View in the element tree
    fn find_window_view(&self, element: &dyn Element) -> Option<WindowInfo> {
        // Check if this element provides window info
        if let Some(info) = element.get_window_info() {
            return Some(info);
        }

        // Check children recursively
        for child in element.children() {
            if let Some(info) = self.find_window_view(child.as_ref()) {
                return Some(info);
            }
        }

        None
    }

    /// Perform initial layout
    ///
    /// This should be called once after setting the root element
    /// to determine the window size and create the compositor.
    ///
    /// Returns window information extracted from the Window View.
    pub fn layout_initial(&mut self) -> WindowInfo {
        // Extract window info first
        let window_info = self.extract_window_info();

        // Use the preferred size from Window as the actual window size
        let window_size = window_info.size;

        // Perform initial layout with tight constraints matching the window size
        let constraints = LayoutConstraints::tight(window_size.width, window_size.height);
        let _layout_size = self.element_tree.layout(constraints);

        // Create renderer with the window size
        crate::graphics::set_current_scale_milli(self.scale_milli);
        self.renderer = Some(Box::new(CpuRenderer::new(
            window_size,
            self.scale_milli,
            window_info.background_color,
        )));
        self.window_size = window_size;
        self.paint_needs_full = true;

        // Mark root as dirty for initial paint
        if let Some(root) = self.element_tree.root() {
            self.pipeline_owner.mark_needs_paint(root.id());
        }

        window_info
    }

    /// Set window size and resize compositor
    pub fn resize(&mut self, new_size: Size) {
        self.window_size = new_size;
        crate::graphics::set_current_scale_milli(self.scale_milli);
        if let Some(ref mut renderer) = self.renderer {
            renderer.resize(FrameSize {
                width: new_size.width,
                height: new_size.height,
                scale_milli: self.scale_milli,
            });
        }
        if let Some(ref mut paint_renderer) = self.paint_renderer {
            paint_renderer.resize(new_size, self.scale_milli);
        }
        self.last_paint_bounds.clear();
        self.paint_damage = None;
        self.paint_needs_full = true;
        self.paint_caches.clear();

        if let Some(root) = self.element_tree.root_mut() {
            root.clear_buffers();
        }

        // Mark entire tree for relayout
        // Note: In a full implementation, we would mark specific elements
        if let Some(root) = self.element_tree.root() {
            self.pipeline_owner.mark_needs_layout(root.id());
        }
    }

    /// Handle a render frame
    ///
    /// This flushes all dirty phases and renders to the window buffer.
    pub fn render(&mut self) -> Option<&Buffer> {
        if crate::debug::is_enabled() {
            crate::logln!("[RenderingPipeline] render() starting...");
        }
        // Flush all dirty phases (build, layout, paint)
        crate::graphics::set_current_scale_milli(self.scale_milli);
        self.pipeline_owner.flush_with_legacy_paint(
            &mut self.element_tree,
            self.window_size,
            !self.paint_enabled,
        );
        if crate::debug::is_enabled() {
            crate::logln!("[RenderingPipeline] flush() completed");
        }

        let background_color = self.extract_window_info().background_color;

        if self.paint_enabled {
            return self.render_paint_path(background_color);
        }

        if let Some(ref mut renderer) = self.renderer {
            renderer.set_background_color(background_color);
            if let Some(root) = self.element_tree.root() {
                let dirty_ids = self.pipeline_owner.last_paint_ids();
                renderer.composite(root, dirty_ids);
            }

            Some(renderer.buffer())
        } else {
            None
        }
    }

    fn render_paint_path(&mut self, background_color: crate::color::Color) -> Option<&Buffer> {
        let size = self.window_size;
        let scale = self.scale_milli;
        let creating_renderer = self.paint_renderer.is_none();

        if self.paint_renderer.is_none() {
            self.paint_renderer = Some(CpuPaintRenderer::new(size, scale, background_color));
        }

        let force_full = self.paint_needs_full
            || creating_renderer
            || self.paint_background_color != Some(background_color);

        let dirty_ids = self.pipeline_owner.last_paint_ids().to_vec();
        let mut dirty_rects = if force_full {
            None
        } else {
            Some(self.paint_dirty_rects(&dirty_ids))
        };

        let present_damage = match dirty_rects.as_mut() {
            Some(rects) if rects.is_empty() => Some(Vec::new()),
            Some(rects) => {
                Self::merge_overlapping_rects(rects);
                Self::present_damage_rects(rects, size, scale)
            }
            None => None,
        };

        self.paint_damage = present_damage;
        self.invalidate_repaint_boundary_caches(&dirty_ids);

        let mut ctx = PaintContext::new();
        let damage_clip = dirty_rects.as_deref();
        let any_painted = if let Some(root) = self.element_tree.root() {
            let base_painted = Self::walk_and_paint(
                &mut ctx,
                root,
                Point::ZERO,
                damage_clip,
                &mut self.paint_caches,
                self.scale_milli,
            );
            let overlay_painted =
                Self::paint_select_overlays(&mut ctx, root, Point::ZERO, damage_clip);
            base_painted || overlay_painted
        } else {
            false
        };

        if force_full || any_painted {
            let pr = self.paint_renderer.as_mut().unwrap();
            pr.set_background_color(background_color);
            pr.execute_with_damage(&ctx, damage_clip);
        }

        self.paint_needs_full = false;
        self.paint_background_color = Some(background_color);
        self.last_paint_bounds.clear();
        if let Some(root) = self.element_tree.root() {
            Self::collect_paint_bounds(root, Point::ZERO, &mut self.last_paint_bounds);
        }

        let pr = self.paint_renderer.as_ref().unwrap();
        Some(pr.buffer())
    }

    fn walk_and_paint<'a>(
        ctx: &mut PaintContext<'a>,
        element: &'a dyn Element,
        origin: Point,
        damage_rects: Option<&[Rect]>,
        paint_caches: &mut BTreeMap<ElementId, PaintCache>,
        scale_milli: u32,
    ) -> bool {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );
        let paint_bounds = Self::element_paint_bounds(element, abs);
        let should_paint_self = damage_rects
            .map(|rects| Self::overlaps_any(paint_bounds, rects))
            .unwrap_or(true);
        let mut painted = should_paint_self && Self::paint_element_self(ctx, element, abs);
        let clip = Self::clip_for_element(element, abs);
        let mut painted_boundary = false;

        if let Some((rect, radius)) = clip {
            ctx.push_rounded_clip(rect, radius);
        }

        if should_paint_self {
            painted_boundary =
                Self::paint_repaint_boundary(ctx, element, abs, paint_caches, scale_milli);
            if painted_boundary {
                painted = true;
            }
        }

        if !painted_boundary {
            for child in element.children() {
                if Self::walk_and_paint(
                    ctx,
                    child.as_ref(),
                    abs,
                    damage_rects,
                    paint_caches,
                    scale_milli,
                ) {
                    painted = true;
                }
            }
        }

        if Self::paint_element_overlay(ctx, element, abs) {
            painted = true;
        }

        if clip.is_some() {
            ctx.pop_clip();
        }

        painted
    }

    fn paint_repaint_boundary<'a>(
        ctx: &mut PaintContext<'a>,
        element: &'a dyn Element,
        abs: Point,
        paint_caches: &mut BTreeMap<ElementId, PaintCache>,
        scale_milli: u32,
    ) -> bool {
        let Some(render_object) = element.render_object() else {
            return false;
        };
        let Some(size) = render_object.repaint_boundary_size() else {
            return false;
        };
        let max_cache_pixels = render_object
            .repaint_boundary_max_cache_pixels()
            .unwrap_or(MAX_REPAINT_BOUNDARY_CACHE_PIXELS);

        if !render_object.repaint_boundary_cache_nested_boundaries()
            && Self::has_descendant_repaint_boundary(element)
        {
            if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!(
                    "[RepaintBoundary] skip id={} reason=nested-boundary logical={}x{}",
                    element.id().get(),
                    size.width,
                    size.height
                );
            }
            paint_caches.remove(&element.id());
            return false;
        }

        let Some((physical_width, physical_height, physical_pixels)) =
            Self::repaint_boundary_physical_size(size, scale_milli)
        else {
            if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!(
                    "[RepaintBoundary] skip id={} reason=invalid-size logical={}x{}",
                    element.id().get(),
                    size.width,
                    size.height
                );
            }
            paint_caches.remove(&element.id());
            return false;
        };

        if physical_pixels > max_cache_pixels {
            if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!(
                    "[RepaintBoundary] skip id={} reason=too-large logical={}x{} physical={}x{} pixels={} max={}",
                    element.id().get(),
                    size.width,
                    size.height,
                    physical_width,
                    physical_height,
                    physical_pixels,
                    max_cache_pixels
                );
            }
            paint_caches.remove(&element.id());
            return false;
        }

        let rebuild_reason = match paint_caches.get(&element.id()) {
            None => Some("miss"),
            Some(cache) if cache.logical_size != size => Some("size-changed"),
            Some(cache) if cache.scale_milli != scale_milli => Some("scale-changed"),
            Some(cache) if !cache.valid => Some("dirty"),
            Some(_) => None,
        };

        if let Some(reason) = rebuild_reason {
            if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!(
                    "[RepaintBoundary] rebuild id={} reason={} logical={}x{} physical={}x{} pixels={}",
                    element.id().get(),
                    reason,
                    size.width,
                    size.height,
                    physical_width,
                    physical_height,
                    physical_pixels
                );
            }
            let mut cache_ctx = PaintContext::new();
            let painted = Self::build_repaint_boundary_context(
                &mut cache_ctx,
                element,
                paint_caches,
                scale_milli,
            );
            if !painted {
                if crate::debug::repaint_boundary_log_enabled() {
                    crate::logln!(
                        "[RepaintBoundary] skip id={} reason=empty",
                        element.id().get()
                    );
                }
                paint_caches.remove(&element.id());
                return false;
            };

            let reused = if let Some(cache) = paint_caches.get_mut(&element.id()) {
                if let Some(buffer) = Arc::get_mut(&mut cache.buffer) {
                    if cache.logical_size != size || cache.scale_milli != scale_milli {
                        buffer.resize_logical_dimensions_with_scale(
                            libm::ceilf(size.width) as u32,
                            libm::ceilf(size.height) as u32,
                            scale_milli,
                        );
                    }
                    CpuPaintRenderer::execute_into_buffer(
                        buffer,
                        crate::color::Color::TRANSPARENT,
                        &cache_ctx,
                        None,
                    );
                    cache.logical_size = size;
                    cache.scale_milli = scale_milli;
                    cache.valid = true;
                    cache.invalidated_by = None;
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if !reused {
                let mut buffer = Buffer::from_logical_dimensions_with_scale(
                    libm::ceilf(size.width) as u32,
                    libm::ceilf(size.height) as u32,
                    scale_milli,
                );
                CpuPaintRenderer::execute_into_buffer(
                    &mut buffer,
                    crate::color::Color::TRANSPARENT,
                    &cache_ctx,
                    None,
                );
                paint_caches.insert(
                    element.id(),
                    PaintCache {
                        buffer: Arc::new(buffer),
                        logical_size: size,
                        scale_milli,
                        valid: true,
                        invalidated_by: None,
                    },
                );
            }
        } else if crate::debug::repaint_boundary_log_enabled() {
            crate::logln!(
                "[RepaintBoundary] hit id={} logical={}x{} physical={}x{} pixels={}",
                element.id().get(),
                size.width,
                size.height,
                physical_width,
                physical_height,
                physical_pixels
            );
        }

        let Some(cache) = paint_caches.get(&element.id()) else {
            return false;
        };
        ctx.draw_buffer_rect_shared(
            Rect::new(abs, size),
            Rect::new(Point::ZERO, size),
            cache.buffer.clone(),
            1.0,
        );
        true
    }

    fn repaint_boundary_physical_size(size: Size, scale_milli: u32) -> Option<(u32, u32, u64)> {
        if size.width <= 0.0
            || size.height <= 0.0
            || !size.width.is_finite()
            || !size.height.is_finite()
        {
            return None;
        }

        let width = Self::scale_len(libm::ceilf(size.width) as u32, scale_milli);
        let height = Self::scale_len(libm::ceilf(size.height) as u32, scale_milli);
        let pixels = u64::from(width).saturating_mul(u64::from(height));
        Some((width, height, pixels))
    }

    fn build_repaint_boundary_context<'a>(
        ctx: &mut PaintContext<'a>,
        element: &'a dyn Element,
        paint_caches: &mut BTreeMap<ElementId, PaintCache>,
        scale_milli: u32,
    ) -> bool {
        let mut painted = false;
        for child in element.children() {
            if Self::walk_and_paint(
                ctx,
                child.as_ref(),
                Point::ZERO,
                None,
                paint_caches,
                scale_milli,
            ) {
                painted = true;
            }
        }
        painted
    }

    fn has_descendant_repaint_boundary(element: &dyn Element) -> bool {
        element.children().iter().any(|child| {
            child
                .render_object()
                .and_then(|render_object| render_object.repaint_boundary_size())
                .is_some()
                || Self::has_descendant_repaint_boundary(child.as_ref())
        })
    }

    fn paint_select_overlays<'a>(
        ctx: &mut PaintContext<'a>,
        element: &'a dyn Element,
        origin: Point,
        damage_rects: Option<&[Rect]>,
    ) -> bool {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );
        let mut painted = false;
        let clip = Self::clip_for_element(element, abs);

        if let Some((rect, radius)) = clip {
            ctx.push_rounded_clip(rect, radius);
        }

        for child in element.children() {
            if Self::paint_select_overlays(ctx, child.as_ref(), abs, damage_rects) {
                painted = true;
            }
        }

        let paint_bounds = Self::element_paint_bounds(element, abs);
        let should_paint_self = damage_rects
            .map(|rects| Self::overlaps_any(paint_bounds, rects))
            .unwrap_or(true);
        if should_paint_self
            && Self::is_expanded_select(element)
            && Self::paint_element_self(ctx, element, abs)
        {
            painted = true;
        }

        if clip.is_some() {
            ctx.pop_clip();
        }

        painted
    }

    fn clip_for_element(element: &dyn Element, abs: Point) -> Option<(Rect, f32)> {
        element
            .render_object()
            .and_then(|render_object| render_object.clip_bounds(abs))
    }

    fn is_expanded_select(element: &dyn Element) -> bool {
        element
            .render_object()
            .and_then(|render_object| {
                render_object
                    .as_any()
                    .downcast_ref::<crate::views::SelectRenderObject>()
            })
            .is_some_and(|select| select.is_expanded())
    }

    fn paint_element_self<'a>(
        ctx: &mut PaintContext<'a>,
        element: &'a dyn Element,
        abs: Point,
    ) -> bool {
        let Some(ro) = element.render_object() else {
            return false;
        };

        let before = ctx.commands().len();
        if ro.paint(ctx, abs) || ctx.commands().len() > before {
            return true;
        }

        if let Some(buf) = element.get_buffer() {
            let rect = crate::geometry::Rect::new(
                abs,
                Size::new(buf.logical_width() as f32, buf.logical_height() as f32),
            );
            ctx.draw_buffer_ref(rect, buf);
            return true;
        }

        false
    }

    fn paint_element_overlay<'a>(
        ctx: &mut PaintContext<'a>,
        element: &'a dyn Element,
        abs: Point,
    ) -> bool {
        let Some(ro) = element.render_object() else {
            return false;
        };

        let before = ctx.commands().len();
        ro.paint_overlay(ctx, abs) || ctx.commands().len() > before
    }

    fn invalidate_repaint_boundary_caches(&mut self, dirty_ids: &[ElementId]) {
        if dirty_ids.is_empty() || self.paint_caches.is_empty() {
            return;
        }

        let cached_ids: Vec<ElementId> = self.paint_caches.keys().copied().collect();
        let mut invalidated = Vec::new();
        for boundary_id in cached_ids {
            if let Some(dirty_id) = dirty_ids
                .iter()
                .copied()
                .find(|dirty_id| self.dirty_id_invalidates_repaint_boundary(boundary_id, *dirty_id))
            {
                invalidated.push((boundary_id, dirty_id));
            }
        }

        for (id, dirty_id) in invalidated {
            if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!(
                    "[RepaintBoundary] invalidate id={} dirty_id={}",
                    id.get(),
                    dirty_id.get()
                );
            }
            if let Some(cache) = self.paint_caches.get_mut(&id) {
                cache.valid = false;
                cache.invalidated_by = Some(dirty_id);
            }
        }
    }

    fn dirty_id_invalidates_repaint_boundary(
        &self,
        boundary_id: ElementId,
        dirty_id: ElementId,
    ) -> bool {
        self.element_tree
            .find_path_ids(dirty_id)
            .map(|path| path.contains(&boundary_id))
            .unwrap_or(true)
    }

    fn element_paint_bounds(element: &dyn Element, absolute_origin: Point) -> Rect {
        let bounds = element.bounds();
        let mut width = bounds.size.width;
        let mut height = bounds.size.height;
        if let Some(select) = element.render_object().and_then(|render_object| {
            render_object
                .as_any()
                .downcast_ref::<crate::views::SelectRenderObject>()
        }) {
            height = height.max(select.paint_height());
        }
        if let Some(buffer) = element.get_buffer() {
            width = width.max(buffer.logical_width() as f32);
            height = height.max(buffer.logical_height() as f32);
        }
        Rect::from_xywh(absolute_origin.x, absolute_origin.y, width, height)
    }

    fn paint_dirty_rects(&self, dirty_ids: &[ElementId]) -> Vec<Rect> {
        if dirty_ids.is_empty() {
            return Vec::new();
        }

        let Some(root) = self.element_tree.root() else {
            return Vec::new();
        };

        let dirty_set: BTreeSet<ElementId> = dirty_ids.iter().copied().collect();
        let mut rects = Vec::new();
        self.collect_dirty_rects(root, Point::ZERO, &dirty_set, &mut rects);
        rects
    }

    fn collect_dirty_rects(
        &self,
        element: &dyn Element,
        origin: Point,
        dirty_ids: &BTreeSet<ElementId>,
        rects: &mut Vec<Rect>,
    ) {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );

        if dirty_ids.contains(&element.id()) {
            rects.push(Self::element_paint_bounds(element, abs));
            if let Some(old_bounds) = self.last_paint_bounds.get(&element.id()) {
                rects.push(*old_bounds);
            }
        }

        for child in element.children() {
            self.collect_dirty_rects(child.as_ref(), abs, dirty_ids, rects);
        }
    }

    fn collect_paint_bounds(
        element: &dyn Element,
        origin: Point,
        bounds: &mut BTreeMap<ElementId, Rect>,
    ) {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );
        bounds.insert(element.id(), Self::element_paint_bounds(element, abs));

        for child in element.children() {
            Self::collect_paint_bounds(child.as_ref(), abs, bounds);
        }
    }

    fn overlaps_any(rect: Rect, rects: &[Rect]) -> bool {
        rects.iter().any(|r| rect.overlaps(r))
    }

    fn merge_overlapping_rects(rects: &mut Vec<Rect>) {
        let mut merged: Vec<Rect> = Vec::new();
        'outer: for rect in rects.drain(..) {
            for existing in merged.iter_mut() {
                if existing.overlaps(&rect) {
                    let left = existing.left().min(rect.left());
                    let top = existing.top().min(rect.top());
                    let right = existing.right().max(rect.right());
                    let bottom = existing.bottom().max(rect.bottom());
                    *existing = Rect::from_xywh(left, top, right - left, bottom - top);
                    continue 'outer;
                }
            }
            merged.push(rect);
        }
        *rects = merged;
    }

    fn present_damage_rects(
        rects: &[Rect],
        window_size: Size,
        scale_milli: u32,
    ) -> Option<Vec<DamageRect>> {
        let physical_width = Self::scale_len(window_size.width as u32, scale_milli);
        let physical_height = Self::scale_len(window_size.height as u32, scale_milli);
        let mut damage: Vec<DamageRect> = rects
            .iter()
            .map(|rect| Self::rect_to_damage(*rect, scale_milli, physical_width, physical_height))
            .filter(|(_, _, width, height)| *width > 0 && *height > 0)
            .collect();

        Self::coalesce_damage_rects(&mut damage);

        let damage_area = Self::damage_rects_area(&damage);
        let window_area = (physical_width as u64).saturating_mul(physical_height as u64);
        if damage_area >= window_area {
            return None;
        }

        Some(damage)
    }

    fn scale_len(value: u32, scale_milli: u32) -> u32 {
        ((value as u64)
            .saturating_mul(scale_milli.max(1) as u64)
            .saturating_add(999)
            / 1000)
            .max(1) as u32
    }

    fn rect_to_damage(
        rect: Rect,
        scale_milli: u32,
        physical_width: u32,
        physical_height: u32,
    ) -> DamageRect {
        let scale = scale_milli.max(1) as f32 / 1000.0;
        let x0 = libm::floorf(rect.origin.x * scale).max(0.0);
        let y0 = libm::floorf(rect.origin.y * scale).max(0.0);
        let x1 = libm::ceilf((rect.origin.x + rect.size.width) * scale).min(physical_width as f32);
        let y1 =
            libm::ceilf((rect.origin.y + rect.size.height) * scale).min(physical_height as f32);
        (
            x0 as u32,
            y0 as u32,
            (x1 - x0).max(0.0) as u32,
            (y1 - y0).max(0.0) as u32,
        )
    }

    fn coalesce_damage_rects(rects: &mut Vec<DamageRect>) {
        rects.retain(|(_, _, width, height)| *width > 0 && *height > 0);

        let mut index = 0usize;
        while index < rects.len() {
            let mut merged = false;
            let mut other = index + 1;
            while other < rects.len() {
                if Self::damage_rects_touch_or_overlap(rects[index], rects[other]) {
                    rects[index] = Self::union_damage_rect(rects[index], rects[other]);
                    rects.remove(other);
                    merged = true;
                } else {
                    other += 1;
                }
            }
            if !merged {
                index += 1;
            }
        }

        while rects.len() > MAX_PRESENT_DAMAGE_RECTS {
            let mut best_pair = (0usize, 1usize);
            let mut best_extra = u64::MAX;

            for i in 0..rects.len() {
                for j in (i + 1)..rects.len() {
                    let union = Self::union_damage_rect(rects[i], rects[j]);
                    let extra = Self::damage_rect_area(union)
                        .saturating_sub(Self::damage_rect_area(rects[i]))
                        .saturating_sub(Self::damage_rect_area(rects[j]));
                    if extra < best_extra {
                        best_extra = extra;
                        best_pair = (i, j);
                    }
                }
            }

            let (i, j) = best_pair;
            rects[i] = Self::union_damage_rect(rects[i], rects[j]);
            rects.remove(j);
        }
    }

    fn damage_rect_area(rect: DamageRect) -> u64 {
        u64::from(rect.2).saturating_mul(u64::from(rect.3))
    }

    fn damage_rects_area(rects: &[DamageRect]) -> u64 {
        rects.iter().fold(0u64, |area, rect| {
            area.saturating_add(Self::damage_rect_area(*rect))
        })
    }

    fn union_damage_rect(a: DamageRect, b: DamageRect) -> DamageRect {
        let left = a.0.min(b.0);
        let top = a.1.min(b.1);
        let right = a.0.saturating_add(a.2).max(b.0.saturating_add(b.2));
        let bottom = a.1.saturating_add(a.3).max(b.1.saturating_add(b.3));
        (
            left,
            top,
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        )
    }

    fn damage_rects_touch_or_overlap(a: DamageRect, b: DamageRect) -> bool {
        let a_right = a.0.saturating_add(a.2);
        let a_bottom = a.1.saturating_add(a.3);
        let b_right = b.0.saturating_add(b.2);
        let b_bottom = b.1.saturating_add(b.3);
        a.0 <= b_right && a_right >= b.0 && a.1 <= b_bottom && a_bottom >= b.1
    }

    /// Handle a render frame and return the buffer with physical damage rectangles.
    ///
    /// The damage is `None` when the whole window should be presented.
    pub fn render_with_damage(&mut self) -> Option<(&Buffer, Option<&[DamageRect]>)> {
        if self.paint_enabled {
            crate::graphics::set_current_scale_milli(self.scale_milli);
            self.pipeline_owner.flush_with_legacy_paint(
                &mut self.element_tree,
                self.window_size,
                false,
            );
            self.render_paint_path(self.extract_window_info().background_color)?;
            let pr = self.paint_renderer.as_ref().unwrap();
            return Some((pr.buffer(), self.paint_damage.as_deref()));
        }
        self.render()?;
        let renderer = self.renderer.as_ref()?;
        Some((renderer.buffer(), renderer.damage()))
    }

    pub fn window_buffer(&self) -> Option<&Buffer> {
        self.renderer.as_ref().map(|r| r.buffer())
    }

    pub fn window_buffer_mut(&mut self) -> Option<&mut Buffer> {
        self.renderer.as_mut().map(|r| r.buffer_mut())
    }

    /// Get the current window size
    pub fn window_size(&self) -> Size {
        self.window_size
    }

    /// Handle an event
    ///
    /// In a full implementation, this would route events through the
    /// EventDispatcher to the target elements.
    pub fn handle_event(&mut self, _event: &crate::event::Event) -> bool {
        self.event_dispatcher
            .dispatch(&mut self.element_tree, _event)
    }

    /// Take emitted events from the event dispatcher
    pub fn take_emitted_events(&mut self) -> Vec<crate::event::Event> {
        self.event_dispatcher.take_emitted_events()
    }

    pub fn focused_text_input_state(&self) -> Option<crate::element::TextInputElementState> {
        self.element_tree.focused_text_input_state()
    }
}

impl Drop for RenderingPipeline {
    fn drop(&mut self) {
        self.teardown();
    }
}

impl Default for RenderingPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, MouseEvent, ScrollSource, WheelPhase};
    use crate::view::{View, ViewExt};
    use crate::views::{LazyVStack, Rectangle, ScrollView, Text};

    #[test]
    fn present_damage_keeps_large_partial_region() {
        let damage = RenderingPipeline::present_damage_rects(
            &[Rect::from_xywh(0.0, 0.0, 800.0, 400.0)],
            Size::new(800.0, 600.0),
            1000,
        )
        .expect("partial damage should not force full present");

        assert_eq!(damage, vec![(0, 0, 800, 400)]);
    }

    #[test]
    fn present_damage_uses_full_for_whole_window() {
        let damage = RenderingPipeline::present_damage_rects(
            &[Rect::from_xywh(0.0, 0.0, 800.0, 600.0)],
            Size::new(800.0, 600.0),
            1000,
        );

        assert_eq!(damage, None);
    }

    #[test]
    fn scroll_view_updates_lazy_vstack_viewport_hint() {
        let mut pipeline = RenderingPipeline::new();
        let root = ScrollView::new(LazyVStack::new(1_000, 20.0, |index| {
            Text::new(format!("Item {index}"))
        }))
        .wheel_sensitivity(1.0)
        .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();

        let initial_child_count = pipeline
            .element_tree()
            .root()
            .and_then(|root| root.children().first())
            .and_then(|boundary| boundary.children().first())
            .map(|lazy| lazy.children().len())
            .expect("scroll view should have lazy child");
        assert!(initial_child_count < 100);

        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: 900,
            x: 10,
            y: 10,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        })));

        let first_materialized_y = pipeline
            .element_tree()
            .root()
            .and_then(|root| root.children().first())
            .and_then(|boundary| boundary.children().first())
            .and_then(|lazy| lazy.children().first())
            .map(|item| item.position().y)
            .expect("lazy child should materialize visible items");
        assert!(first_materialized_y > 0.0);
    }

    #[test]
    fn repaint_boundary_cache_survives_scroll_offset_repaint() {
        let mut pipeline = RenderingPipeline::new();
        let root = ScrollView::new(
            Rectangle::new()
                .fill(crate::color::Color::rgb(220, 40, 40))
                .frame(100.0, 300.0),
        )
        .content_size(100.0, 300.0)
        .wheel_sensitivity(1.0)
        .frame(100.0, 100.0)
        .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();

        assert_eq!(pipeline.paint_caches.len(), 1);
        let cache_id = *pipeline.paint_caches.keys().next().unwrap();
        let before = Arc::as_ptr(&pipeline.paint_caches[&cache_id].buffer);

        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: 40,
            x: 10,
            y: 10,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        })));
        pipeline.render_with_damage();

        let after = Arc::as_ptr(&pipeline.paint_caches[&cache_id].buffer);
        assert_eq!(before, after);
    }

    #[test]
    fn repaint_boundary_cache_is_invalidated_by_descendant_dirty() {
        let mut pipeline = RenderingPipeline::new();
        let root = ScrollView::new(
            Rectangle::new()
                .fill(crate::color::Color::rgb(40, 120, 220))
                .frame(100.0, 300.0),
        )
        .content_size(100.0, 300.0)
        .frame(100.0, 100.0)
        .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();

        let boundary = pipeline
            .element_tree()
            .root()
            .and_then(|root| root.children().first())
            .and_then(|scroll| scroll.children().first())
            .expect("scroll content should be a repaint boundary");
        let descendant_id = boundary
            .children()
            .first()
            .expect("boundary should have a child")
            .id();
        assert_eq!(pipeline.paint_caches.len(), 1);
        let cache_id = *pipeline.paint_caches.keys().next().unwrap();
        let before = Arc::as_ptr(&pipeline.paint_caches[&cache_id].buffer);

        pipeline.pipeline_owner.mark_needs_paint(descendant_id);
        pipeline.render_with_damage();
        assert_eq!(pipeline.paint_caches.len(), 1);
        let after = Arc::as_ptr(&pipeline.paint_caches[&cache_id].buffer);
        assert_eq!(before, after);
        assert!(pipeline.paint_caches[&cache_id].valid);
    }

    #[test]
    fn repaint_boundary_cache_respects_ancestor_clip() {
        let red = crate::color::Color::rgb(255, 0, 0);
        let mut pipeline = RenderingPipeline::new();
        let root = ScrollView::new(Rectangle::new().fill(red).frame(20.0, 20.0))
            .content_size(20.0, 20.0)
            .frame(10.0, 10.0)
            .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();

        let (inside, outside) = {
            let (buffer, _) = pipeline.render_with_damage().expect("frame should render");
            (buffer.get_pixel(5, 5), buffer.get_pixel(15, 5))
        };
        assert_eq!(pipeline.paint_caches.len(), 1);
        assert_eq!(inside, Some(red.to_bgra()));
        assert_ne!(outside, Some(red.to_bgra()));
    }

    #[test]
    fn scroll_view_auto_boundary_skips_nested_boundaries() {
        let mut pipeline = RenderingPipeline::new();
        let inner = ScrollView::new(
            Rectangle::new()
                .fill(crate::color::Color::rgb(80, 180, 120))
                .frame(100.0, 300.0),
        )
        .content_size(100.0, 300.0)
        .frame(100.0, 100.0);
        let root = ScrollView::new(inner)
            .content_size(100.0, 200.0)
            .frame(100.0, 100.0)
            .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();

        assert_eq!(pipeline.paint_caches.len(), 1);
        let cache = pipeline.paint_caches.values().next().unwrap();
        assert_eq!(cache.logical_size, Size::new(100.0, 300.0));
    }
}
