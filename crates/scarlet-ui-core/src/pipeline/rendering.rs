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
use crate::pipeline::layers::{
    LayerChild, LayerClip, LayerId, LayerPrimitive, LayerPrimitiveKind, LayerStore, PictureChunk,
};
use crate::pipeline::{PipelineId, PipelineOwner};
use crate::renderer::{
    BackendFrame, CpuPaintBackend, CpuPaintRenderer, CpuRenderer, FrameSize, PaintBackend,
    PaintContext, PresentedFrame,
};
use crate::views::{WindowBackdrop, WindowInfo, WindowRenderObject};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
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

struct WindowBackdropCache {
    backdrop: WindowBackdrop,
    origin: Point,
    scale_milli: u32,
    buffer: Arc<Buffer>,
}

#[derive(Default)]
struct DirtyScratch {
    ids: Vec<ElementId>,
    path: Vec<ElementId>,
    rects: Vec<Rect>,
    damage: Vec<DamageRect>,
}

impl DirtyScratch {
    fn clear_for_frame(&mut self) {
        self.ids.clear();
        self.path.clear();
        self.rects.clear();
        self.damage.clear();
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PaintTestCounters {
    pub(crate) paint_context_news: usize,
    pub(crate) walk_and_paint_calls: usize,
    pub(crate) boundary_rebuilds: usize,
    pub(crate) retained_composites: usize,
    pub(crate) retained_sync_visits: usize,
    pub(crate) localized_retained_syncs: usize,
    pub(crate) retained_sync_fallbacks: usize,
    pub(crate) retained_primitive_slot_syncs: usize,
    pub(crate) retained_primitive_scan_syncs: usize,
    pub(crate) window_backdrop_rebuilds: usize,
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
    paint_backend: Box<dyn PaintBackend>,
    last_paint_bounds: BTreeMap<ElementId, Rect>,
    paint_damage: Option<Vec<DamageRect>>,
    paint_needs_full: bool,
    paint_background_color: Option<crate::color::Color>,
    paint_enabled: bool,
    paint_caches: BTreeMap<ElementId, PaintCache>,
    window_backdrop_cache: Option<WindowBackdropCache>,
    layer_store: LayerStore,
    retained_ctx: PaintContext<'static>,
    dirty_scratch: DirtyScratch,
    #[cfg(test)]
    paint_test_counters: PaintTestCounters,
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
            paint_backend: Box::new(CpuPaintBackend::new(
                Size::new(800.0, 600.0),
                1000,
                crate::color::Color::TRANSPARENT,
            )),
            last_paint_bounds: BTreeMap::new(),
            paint_damage: None,
            paint_needs_full: true,
            paint_background_color: None,
            paint_enabled: true,
            paint_caches: BTreeMap::new(),
            window_backdrop_cache: None,
            layer_store: LayerStore::new(),
            retained_ctx: PaintContext::new(),
            dirty_scratch: DirtyScratch::default(),
            #[cfg(test)]
            paint_test_counters: PaintTestCounters::default(),
        }
    }

    pub fn set_paint_enabled(&mut self, enabled: bool) {
        self.paint_enabled = enabled;
    }

    /// Install a paint backend.
    ///
    /// The backend is resized to the pipeline's current target before its first
    /// frame. Installing a backend invalidates retained output state so the
    /// first frame redraws the complete window.
    ///
    /// # Arguments
    ///
    /// * `paint_backend` - Backend used to execute paint commands.
    ///
    /// # Returns
    ///
    /// Nothing.
    pub fn set_paint_backend(&mut self, mut paint_backend: Box<dyn PaintBackend>) {
        paint_backend.resize(self.window_size, self.scale_milli);
        self.paint_backend = paint_backend;
        self.last_paint_bounds.clear();
        self.paint_damage = None;
        self.paint_needs_full = true;
        self.paint_background_color = None;
        self.paint_caches.clear();
        self.window_backdrop_cache = None;
        self.layer_store.clear();
        self.retained_ctx.clear();
    }

    /// Return this pipeline's owner ID.
    pub const fn pipeline_id(&self) -> PipelineId {
        self.element_tree.pipeline_id()
    }

    #[cfg(test)]
    pub(crate) fn reset_paint_test_counters(&mut self) {
        self.paint_test_counters = PaintTestCounters::default();
    }

    #[cfg(test)]
    pub(crate) fn paint_test_counters(&self) -> PaintTestCounters {
        self.paint_test_counters
    }

    #[cfg(test)]
    pub(crate) fn retained_boundary_scratch_capacity_for_test(
        &self,
    ) -> Option<crate::renderer::RasterScratchCapacity> {
        self.paint_renderer
            .as_ref()
            .map(CpuPaintRenderer::retained_boundary_scratch_capacity_for_test)
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
        self.window_backdrop_cache = None;
        self.layer_store.clear();
        self.retained_ctx.clear();
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
        self.paint_backend
            .resize(self.window_size, self.scale_milli);
        self.last_paint_bounds.clear();
        self.paint_damage = None;
        self.paint_needs_full = true;
        self.paint_caches.clear();
        self.window_backdrop_cache = None;
        self.layer_store.clear();
        self.retained_ctx.clear();
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
        self.window_backdrop_cache = None;
        self.layer_store.clear();
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

    /// Invalidate the complete pipeline after a runtime input-environment change.
    ///
    /// This schedules build, layout, and paint so device-dependent metrics are
    /// reflected by both geometry and rendering on the next frame.
    pub fn invalidate_input_environment(&mut self) {
        self.last_paint_bounds.clear();
        self.paint_damage = None;
        self.paint_needs_full = true;
        self.paint_caches.clear();
        self.window_backdrop_cache = None;
        self.layer_store.clear();
        self.retained_ctx.clear();
        if let Some(root) = self.element_tree.root_mut() {
            root.clear_buffers();
        }
        if let Some(root) = self.element_tree.root() {
            let id = root.id();
            self.pipeline_owner.mark_needs_build(id);
            self.pipeline_owner.mark_needs_layout(id);
            self.pipeline_owner.mark_needs_paint(id);
        }
    }

    /// Extract window information from the element tree
    ///
    /// This searches the element tree for a Window View and extracts
    /// the app_id, title, size, window type, background, placement, and
    /// policies from it.
    ///
    /// Returns window information or defaults if no Window is found.
    fn extract_window_info(&self) -> WindowInfo {
        // Try to find a Window View in the element tree
        if let Some(root) = self.element_tree.root() {
            if let Some(window_info) = self.find_window_view(root) {
                return window_info;
            }
        }

        // Default values
        WindowInfo::new(
            alloc::string::String::from("com.example.scarletui"),
            alloc::string::String::from("ScarletUI Application"),
            Size::new(800.0, 600.0),
            0,
            None,
            true,
            true,
            crate::color::ColorPalette::light().window_background(),
            true,
            crate::platform::WindowPlacement::Default,
        )
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

    fn find_deferred_window_backdrop(
        element: &dyn Element,
        origin: Point,
    ) -> Option<(WindowBackdrop, Point)> {
        let absolute_origin = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );
        if let Some(backdrop) = element
            .render_object()
            .and_then(|render_object| render_object.as_any().downcast_ref::<WindowRenderObject>())
            .and_then(WindowRenderObject::deferred_backdrop)
        {
            return Some((backdrop, absolute_origin));
        }
        element
            .children()
            .iter()
            .find_map(|child| Self::find_deferred_window_backdrop(child.as_ref(), absolute_origin))
    }

    fn paint_deferred_window_backdrop<'a>(
        ctx: &mut PaintContext<'a>,
        deferred_backdrop: Option<(WindowBackdrop, Point)>,
        damage_clip: Option<&[Rect]>,
        cache: &mut Option<WindowBackdropCache>,
        paint_renderer: &mut CpuPaintRenderer,
        scale_milli: u32,
        #[cfg(test)] paint_test_counters: &mut PaintTestCounters,
    ) -> bool {
        let Some((backdrop, origin)) = deferred_backdrop else {
            *cache = None;
            return false;
        };
        let size = backdrop.size();
        let cacheable = Self::repaint_boundary_physical_size(size, scale_milli)
            .is_some_and(|(_, _, pixels)| pixels <= MAX_REPAINT_BOUNDARY_CACHE_PIXELS);
        if !cacheable {
            *cache = None;
            backdrop.paint(ctx, origin);
            return true;
        }

        let needs_rebuild = cache.as_ref().is_none_or(|cached| {
            cached.backdrop != backdrop
                || cached.origin != origin
                || cached.scale_milli != scale_milli
        });
        if needs_rebuild {
            let mut buffer = Buffer::from_logical_dimensions_with_scale(
                libm::ceilf(size.width.max(1.0)) as u32,
                libm::ceilf(size.height.max(1.0)) as u32,
                scale_milli,
            );
            let mut backdrop_ctx = PaintContext::new();
            backdrop.paint_cached_shadow(&mut backdrop_ctx, Point::ZERO);
            let shadow_regions = backdrop.shadow_visible_regions();
            let mut shadow_damage = Vec::new();
            shadow_damage.extend(
                shadow_regions
                    .iter()
                    .copied()
                    .filter(|rect| !Self::paint_bounds_is_empty(*rect)),
            );
            paint_renderer.execute_into_external_buffer(
                &mut buffer,
                crate::color::Color::TRANSPARENT,
                &backdrop_ctx,
                Some(&shadow_damage),
            );
            *cache = Some(WindowBackdropCache {
                backdrop,
                origin,
                scale_milli,
                buffer: Arc::new(buffer),
            });
            #[cfg(test)]
            {
                paint_test_counters.window_backdrop_rebuilds += 1;
            }
        }

        let Some(cached) = cache.as_ref() else {
            return false;
        };
        Self::paint_cached_window_backdrop(ctx, cached, damage_clip);
        true
    }

    fn paint_cached_window_backdrop<'a>(
        ctx: &mut PaintContext<'a>,
        cached: &WindowBackdropCache,
        damage_clip: Option<&[Rect]>,
    ) {
        let backdrop = cached.backdrop;
        let origin = cached.origin;
        for source in backdrop.shadow_visible_regions() {
            if Self::paint_bounds_is_empty(source) {
                continue;
            }
            let destination = Rect::new(
                Point::new(origin.x + source.origin.x, origin.y + source.origin.y),
                source.size,
            );
            if damage_clip.is_some_and(|damage| !Self::overlaps_any(destination, damage)) {
                continue;
            }
            ctx.draw_buffer_rect_shared(destination, source, cached.buffer.clone(), 1.0);
        }
        backdrop.paint_body(ctx, origin);
    }

    fn extract_background_color(&self) -> crate::color::Color {
        if let Some(root) = self.element_tree.root()
            && let Some(window_info) = self.find_window_view(root)
        {
            return window_info.surface_clear_color();
        }
        crate::color::ColorPalette::light().window_background()
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
            window_info.surface_clear_color(),
        )));
        self.window_size = window_size;
        self.paint_backend.resize(window_size, self.scale_milli);
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
        self.paint_backend.resize(new_size, self.scale_milli);
        self.last_paint_bounds.clear();
        self.paint_damage = None;
        self.paint_needs_full = true;
        self.paint_caches.clear();
        self.window_backdrop_cache = None;
        self.layer_store.clear();
        self.retained_ctx.clear();

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

        let background_color = self.extract_background_color();

        if self.paint_enabled {
            return match self.render_paint_path(background_color).ok()? {
                PresentedFrame::Cpu { buffer, .. } => Some(buffer),
                PresentedFrame::External | PresentedFrame::Idle => None,
            };
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

    fn render_paint_path(
        &mut self,
        background_color: crate::color::Color,
    ) -> crate::error::Result<PresentedFrame<'_>> {
        let size = self.window_size;
        let scale = self.scale_milli;
        let creating_renderer = self.paint_renderer.is_none();
        let has_paint_extensions = self
            .element_tree
            .root()
            .is_some_and(Self::subtree_emits_paint_extension);
        let deferred_window_backdrop = self
            .element_tree
            .root()
            .and_then(|root| Self::find_deferred_window_backdrop(root, Point::ZERO));
        let window_backdrop_retained =
            deferred_window_backdrop.is_some_and(|(backdrop, origin)| {
                self.window_backdrop_cache.as_ref().is_some_and(|cached| {
                    cached.backdrop == backdrop
                        && cached.origin == origin
                        && cached.scale_milli == self.scale_milli
                })
            });
        let repaint_composite_damage = has_paint_extensions
            && self.pipeline_owner.last_paint_ids().is_empty()
            && !self.pipeline_owner.last_composite_ids().is_empty();
        let transparent_surface_changed = background_color.a < 1.0
            && (!self.pipeline_owner.last_paint_ids().is_empty()
                || !self.pipeline_owner.last_composite_ids().is_empty());

        if self.paint_renderer.is_none() {
            self.paint_renderer = Some(CpuPaintRenderer::new(size, scale, background_color));
        }

        let mut force_full = self.paint_needs_full
            || creating_renderer
            || self.paint_background_color != Some(background_color)
            // A transparent top-level surface cannot clear and rebuild only a
            // retained descendant: transparent pixels in that descendant need
            // the window background beneath them, while the rounded exterior
            // must stay transparent. Recompose from the root until transparent
            // damage has its own ancestor-aware clear path.
            || (transparent_surface_changed && !window_backdrop_retained)
            || self.last_paint_ids_require_full_refresh();

        if !has_paint_extensions
            && !force_full
            && self.pipeline_owner.last_paint_ids().is_empty()
            && !self.pipeline_owner.last_composite_ids().is_empty()
        {
            if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!(
                    "[RetainedComposite] candidate composite_ids={} root_valid={} paint_needs_full={}",
                    self.pipeline_owner.last_composite_ids().len(),
                    self.layer_store.root_graph_valid_for(size, scale),
                    self.paint_needs_full,
                );
            }
            if self.layer_store.root_graph_valid_for(size, scale) {
                if self.prepare_retained_composite_path(background_color, window_backdrop_retained)
                {
                    return self.render_prepared_retained_composite(background_color);
                }
                if crate::debug::repaint_boundary_log_enabled() {
                    crate::logln!(
                        "[RetainedComposite] fallback reason=retained-path-failed composite_ids={}",
                        self.pipeline_owner.last_composite_ids().len(),
                    );
                }
            } else if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!(
                    "[RetainedComposite] fallback reason=root-invalid composite_ids={}",
                    self.pipeline_owner.last_composite_ids().len(),
                );
            }
            force_full = true;
            self.paint_needs_full = true;
        }

        self.dirty_scratch.clear_for_frame();
        self.dirty_scratch
            .ids
            .extend_from_slice(self.pipeline_owner.last_paint_ids());
        if repaint_composite_damage {
            self.dirty_scratch
                .ids
                .extend_from_slice(self.pipeline_owner.last_composite_ids());
        }
        self.dirty_scratch.ids.sort_unstable();
        self.dirty_scratch.ids.dedup();

        let has_dirty_rects = !force_full;
        if has_dirty_rects {
            Self::paint_dirty_rects_into(
                &self.element_tree,
                &self.last_paint_bounds,
                &self.dirty_scratch.ids,
                self.pipeline_owner.last_self_paint_ids(),
                &mut self.dirty_scratch.path,
                &mut self.dirty_scratch.rects,
            );
            if window_backdrop_retained && let Some((backdrop, origin)) = deferred_window_backdrop {
                let body = backdrop.window_geometry_rect(origin);
                self.dirty_scratch.rects.retain_mut(|rect| {
                    let Some(intersection) = Self::intersect_paint_bounds(*rect, body) else {
                        return false;
                    };
                    *rect = intersection;
                    true
                });
            }
            if !self.dirty_scratch.rects.is_empty() {
                Self::merge_overlapping_rects(&mut self.dirty_scratch.rects);
                let partial = Self::present_damage_rects_into(
                    &self.dirty_scratch.rects,
                    size,
                    scale,
                    &mut self.dirty_scratch.damage,
                );
                self.store_paint_damage(partial);
            } else {
                self.dirty_scratch.damage.clear();
                self.store_paint_damage(true);
            }
        } else {
            self.paint_damage = None;
        }

        self.invalidate_repaint_boundary_caches();
        let layer_generation = self.layer_store.begin_rebuild();

        #[cfg(test)]
        {
            self.paint_test_counters.paint_context_news += 1;
        }
        let mut ctx = PaintContext::new();
        let damage_clip = has_dirty_rects.then_some(self.dirty_scratch.rects.as_slice());
        let any_painted = if let Some(root) = self.element_tree.root() {
            let Some(paint_renderer) = self.paint_renderer.as_mut() else {
                return Err(crate::error::Error::RenderError);
            };
            let backdrop_painted = Self::paint_deferred_window_backdrop(
                &mut ctx,
                deferred_window_backdrop,
                damage_clip,
                &mut self.window_backdrop_cache,
                paint_renderer,
                self.scale_milli,
                #[cfg(test)]
                &mut self.paint_test_counters,
            );
            let base_painted = Self::walk_and_paint(
                &mut ctx,
                root,
                Point::ZERO,
                damage_clip,
                &mut self.paint_caches,
                &mut self.layer_store,
                paint_renderer,
                self.scale_milli,
                layer_generation,
                #[cfg(test)]
                &mut self.paint_test_counters,
            );
            let overlay_painted =
                Self::paint_select_overlays(&mut ctx, root, Point::ZERO, damage_clip);
            backdrop_painted || base_painted || overlay_painted
        } else {
            false
        };
        if damage_clip.is_none() {
            if let Some(root) = self.element_tree.root() {
                Self::rebuild_root_layer_refs(
                    root,
                    self.window_size,
                    self.scale_milli,
                    layer_generation,
                    &mut self.layer_store,
                );
            }
            self.layer_store.prune_unmarked(layer_generation);
        }

        let should_render = force_full || any_painted;

        self.paint_needs_full = false;
        self.paint_background_color = Some(background_color);
        self.last_paint_bounds.clear();
        if let Some(root) = self.element_tree.root() {
            Self::collect_paint_bounds(root, Point::ZERO, None, &mut self.last_paint_bounds);
        }

        if !should_render {
            return Ok(PresentedFrame::Idle);
        }

        let physical_damage = self.paint_damage.as_deref();
        let backend_frame =
            match self
                .paint_backend
                .render(&ctx, background_color, damage_clip, physical_damage)
            {
                Ok(frame) => frame,
                Err(error) => {
                    self.paint_needs_full = true;
                    return Err(error);
                }
            };
        match backend_frame {
            BackendFrame::Cpu { buffer } => Ok(PresentedFrame::Cpu {
                buffer,
                damage: physical_damage,
            }),
            BackendFrame::External => Ok(PresentedFrame::External),
        }
    }

    fn prepare_retained_composite_path(
        &mut self,
        background_color: crate::color::Color,
        include_window_backdrop: bool,
    ) -> bool {
        self.dirty_scratch.clear_for_frame();
        self.dirty_scratch
            .ids
            .extend_from_slice(self.pipeline_owner.last_composite_ids());
        self.dirty_scratch.ids.sort_unstable();
        self.dirty_scratch.ids.dedup();

        let Some(root) = self.element_tree.root() else {
            if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!("[RetainedComposite] fail reason=no-root");
            }
            return false;
        };
        self.dirty_scratch.rects.clear();
        for index in 0..self.dirty_scratch.ids.len() {
            self.dirty_scratch.path.clear();
            let dirty_id = self.dirty_scratch.ids[index];
            if !self
                .element_tree
                .find_path_ids_into(dirty_id, &mut self.dirty_scratch.path)
            {
                if crate::debug::repaint_boundary_log_enabled() {
                    crate::logln!(
                        "[RetainedComposite] fail reason=path-not-found dirty_id={}",
                        dirty_id.get(),
                    );
                }
                return false;
            }
            if self
                .element_tree
                .element_and_absolute_origin_for_path(&self.dirty_scratch.path)
                .is_none()
            {
                if crate::debug::repaint_boundary_log_enabled() {
                    crate::logln!(
                        "[RetainedComposite] fail reason=origin-resolution dirty_id={}",
                        dirty_id.get(),
                    );
                }
                return false;
            }
            if let Some(current_bounds) =
                Self::element_paint_bounds_for_path(&self.element_tree, &self.dirty_scratch.path)
            {
                self.dirty_scratch.rects.push(current_bounds);
            }
            if let Some(old_bounds) = self.last_paint_bounds.get(&dirty_id)
                && !Self::paint_bounds_is_empty(*old_bounds)
            {
                self.dirty_scratch.rects.push(*old_bounds);
            }
            if Self::sync_retained_layer_offsets_for_path_target(
                root,
                &self.dirty_scratch.path,
                &mut self.layer_store,
                #[cfg(test)]
                &mut self.paint_test_counters,
            ) {
                continue;
            }
            #[cfg(test)]
            {
                self.paint_test_counters.retained_sync_fallbacks += 1;
            }
            if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!(
                    "[RetainedComposite] fallback reason=localized-sync-failed dirty_id={}",
                    dirty_id.get(),
                );
            }
            if !Self::sync_retained_layer_offsets_along_path(
                root,
                Point::ZERO,
                None,
                LayerId::Root,
                &self.dirty_scratch.path,
                0,
                &mut self.layer_store,
                #[cfg(test)]
                &mut self.paint_test_counters,
            ) {
                if crate::debug::repaint_boundary_log_enabled() {
                    crate::logln!(
                        "[RetainedComposite] fail reason=recursive-sync dirty_id={}",
                        dirty_id.get(),
                    );
                }
                return false;
            }
        }
        if self.dirty_scratch.rects.is_empty() {
            if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!("[RetainedComposite] fail reason=empty-dirty-rects");
            }
            return false;
        }
        Self::merge_overlapping_rects(&mut self.dirty_scratch.rects);
        let partial = Self::present_damage_rects_into(
            &self.dirty_scratch.rects,
            self.window_size,
            self.scale_milli,
            &mut self.dirty_scratch.damage,
        );
        self.store_paint_damage(partial);

        self.retained_ctx.clear();
        if include_window_backdrop {
            let Some(cached_backdrop) = self.window_backdrop_cache.as_ref() else {
                return false;
            };
            Self::paint_cached_window_backdrop(
                &mut self.retained_ctx,
                cached_backdrop,
                Some(self.dirty_scratch.rects.as_slice()),
            );
        }
        if !Self::composite_layer_container(
            &mut self.retained_ctx,
            &self.layer_store,
            LayerId::Root,
            Point::ZERO,
        ) {
            if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!("[RetainedComposite] fail reason=command-composite");
            }
            return false;
        }

        #[cfg(test)]
        {
            self.paint_test_counters.retained_composites += 1;
        }

        if crate::debug::repaint_boundary_log_enabled() {
            crate::logln!(
                "[RetainedComposite] success composite_ids={} dirty_rects={} partial_damage={}",
                self.dirty_scratch.ids.len(),
                self.dirty_scratch.rects.len(),
                partial,
            );
        }

        self.paint_needs_full = false;
        self.paint_background_color = Some(background_color);
        Self::update_paint_bounds_for_ids(
            &self.element_tree,
            &self.dirty_scratch.ids,
            &mut self.dirty_scratch.path,
            &mut self.last_paint_bounds,
        );

        true
    }

    fn render_prepared_retained_composite(
        &mut self,
        background_color: crate::color::Color,
    ) -> crate::error::Result<PresentedFrame<'_>> {
        let partial = self.paint_damage.is_some();
        let damage_clip = partial.then_some(self.dirty_scratch.rects.as_slice());
        let physical_damage = self.paint_damage.as_deref();
        let backend_frame = match self.paint_backend.render(
            &self.retained_ctx,
            background_color,
            damage_clip,
            physical_damage,
        ) {
            Ok(frame) => frame,
            Err(error) => {
                self.retained_ctx.clear();
                self.paint_needs_full = true;
                return Err(error);
            }
        };
        self.retained_ctx.clear();
        let frame = match backend_frame {
            BackendFrame::Cpu { buffer } => PresentedFrame::Cpu {
                buffer,
                damage: physical_damage,
            },
            BackendFrame::External => PresentedFrame::External,
        };
        Ok(frame)
    }

    fn last_paint_ids_require_full_refresh(&self) -> bool {
        self.pipeline_owner.last_paint_ids().iter().any(|id| {
            let Some(element) = self.element_tree.find_element(*id) else {
                return true;
            };
            !self.last_paint_bounds.contains_key(&element.id())
                || Self::subtree_has_untracked_retained_boundary(element, &self.last_paint_bounds)
        })
    }

    fn subtree_has_untracked_retained_boundary(
        element: &dyn Element,
        last_paint_bounds: &BTreeMap<ElementId, Rect>,
    ) -> bool {
        element.children().iter().any(|child| {
            let child = child.as_ref();
            let is_retained_boundary = child
                .render_object()
                .and_then(|render_object| render_object.repaint_boundary_size())
                .is_some();
            (is_retained_boundary && !last_paint_bounds.contains_key(&child.id()))
                || Self::subtree_has_untracked_retained_boundary(child, last_paint_bounds)
        })
    }

    fn walk_and_paint<'a>(
        ctx: &mut PaintContext<'a>,
        element: &'a dyn Element,
        origin: Point,
        damage_rects: Option<&[Rect]>,
        paint_caches: &mut BTreeMap<ElementId, PaintCache>,
        layer_store: &mut LayerStore,
        paint_renderer: &mut CpuPaintRenderer,
        scale_milli: u32,
        layer_generation: u64,
        #[cfg(test)] paint_test_counters: &mut PaintTestCounters,
    ) -> bool {
        #[cfg(test)]
        {
            paint_test_counters.walk_and_paint_calls += 1;
        }
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
            painted_boundary = Self::paint_repaint_boundary(
                ctx,
                element,
                abs,
                paint_caches,
                layer_store,
                paint_renderer,
                scale_milli,
                layer_generation,
                #[cfg(test)]
                paint_test_counters,
            );
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
                    layer_store,
                    paint_renderer,
                    scale_milli,
                    layer_generation,
                    #[cfg(test)]
                    paint_test_counters,
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
        layer_store: &mut LayerStore,
        paint_renderer: &mut CpuPaintRenderer,
        scale_milli: u32,
        layer_generation: u64,
        #[cfg(test)] paint_test_counters: &mut PaintTestCounters,
    ) -> bool {
        if Self::subtree_emits_paint_extension(element) {
            paint_caches.remove(&element.id());
            return false;
        }
        let Some(render_object) = element.render_object() else {
            return false;
        };
        let Some(size) = render_object.repaint_boundary_size() else {
            return false;
        };
        let max_cache_pixels = render_object
            .repaint_boundary_max_cache_pixels()
            .unwrap_or(MAX_REPAINT_BOUNDARY_CACHE_PIXELS);

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

        let layer_id = LayerId::Boundary(element.id());
        let rebuild_reason = if !layer_store.is_valid_for(layer_id, size, scale_milli) {
            match layer_store.container(layer_id) {
                None => Some("miss"),
                Some(container) if container.logical_size != size => Some("size-changed"),
                Some(container) if container.scale_milli != scale_milli => Some("scale-changed"),
                Some(container) if !container.valid => Some("dirty"),
                Some(_) => Some("chunk-dirty"),
            }
        } else {
            None
        };

        if let Some(reason) = rebuild_reason {
            #[cfg(test)]
            {
                paint_test_counters.boundary_rebuilds += 1;
            }
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
            if !Self::rebuild_boundary_layer(
                element,
                size,
                layer_store,
                paint_renderer,
                scale_milli,
                layer_generation,
                #[cfg(test)]
                paint_test_counters,
            ) {
                if crate::debug::repaint_boundary_log_enabled() {
                    crate::logln!(
                        "[RepaintBoundary] skip id={} reason=empty",
                        element.id().get()
                    );
                }
                paint_caches.remove(&element.id());
                return false;
            }
        } else {
            // Keep composite_layer_container as the single z-order compositor for
            // this retained subtree. On a parent cache hit, child boundary layers
            // still need an independent cache-check/rebuild pass, but they must
            // not emit their own draw commands here or they would be composited
            // twice. Non-boundary descendants remain part of this boundary's
            // retained picture chunks and are intentionally skipped.
            Self::ensure_descendant_boundary_layers(
                element,
                layer_store,
                paint_renderer,
                scale_milli,
                layer_generation,
                #[cfg(test)]
                paint_test_counters,
            );
            layer_store.mark_container_subtree(layer_id, layer_generation);
            if crate::debug::repaint_boundary_log_enabled() {
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
        }

        Self::composite_layer_container(ctx, layer_store, layer_id, abs)
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

    fn rebuild_boundary_layer<'a>(
        element: &'a dyn Element,
        size: Size,
        layer_store: &mut LayerStore,
        paint_renderer: &mut CpuPaintRenderer,
        scale_milli: u32,
        layer_generation: u64,
        #[cfg(test)] paint_test_counters: &mut PaintTestCounters,
    ) -> bool {
        let owner = element.id();
        let container_id = LayerId::Boundary(owner);
        layer_store.begin_container_rebuild(
            container_id,
            Some(owner),
            size,
            scale_milli,
            layer_generation,
        );
        let mut chunk_ctx = PaintContext::new();
        #[cfg(test)]
        {
            paint_test_counters.paint_context_news += 1;
        }
        let mut next_ordinal = 0u16;
        let mut painted = false;

        for child in element.children() {
            if Self::build_boundary_walk(
                &mut chunk_ctx,
                child.as_ref(),
                Point::ZERO,
                None,
                owner,
                container_id,
                size,
                layer_store,
                paint_renderer,
                scale_milli,
                layer_generation,
                &mut next_ordinal,
                #[cfg(test)]
                paint_test_counters,
            ) {
                painted = true;
            }
        }

        if Self::flush_picture_chunk(
            &mut chunk_ctx,
            owner,
            container_id,
            size,
            layer_store,
            paint_renderer,
            scale_milli,
            layer_generation,
            &mut next_ordinal,
        ) {
            painted = true;
        }

        if painted {
            layer_store.finish_container_rebuild(container_id);
            layer_store.mark_container_subtree(container_id, layer_generation);
        }
        painted
    }

    fn build_boundary_walk<'a>(
        chunk_ctx: &mut PaintContext<'a>,
        element: &'a dyn Element,
        origin: Point,
        active_clip: Option<LayerClip>,
        owner: ElementId,
        container_id: LayerId,
        container_size: Size,
        layer_store: &mut LayerStore,
        paint_renderer: &mut CpuPaintRenderer,
        scale_milli: u32,
        layer_generation: u64,
        next_ordinal: &mut u16,
        #[cfg(test)] paint_test_counters: &mut PaintTestCounters,
    ) -> bool {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );

        if element
            .render_object()
            .and_then(|render_object| render_object.repaint_boundary_size())
            .is_some()
        {
            let flushed = Self::flush_picture_chunk(
                chunk_ctx,
                owner,
                container_id,
                container_size,
                layer_store,
                paint_renderer,
                scale_milli,
                layer_generation,
                next_ordinal,
            );
            let nested_painted = Self::ensure_boundary_layer(
                element,
                layer_store,
                paint_renderer,
                scale_milli,
                layer_generation,
                #[cfg(test)]
                paint_test_counters,
            );
            if nested_painted {
                let nested_id = LayerId::Boundary(element.id());
                layer_store.append_child(
                    container_id,
                    LayerChild::Boundary {
                        id: nested_id,
                        offset: abs,
                        clip: active_clip,
                    },
                );
            }
            return flushed || nested_painted;
        }

        let mut painted = Self::paint_element_self(chunk_ctx, element, abs);
        let clip = Self::clip_for_element(element, abs);
        let mut child_clip = active_clip;
        if let Some((rect, radius)) = clip {
            chunk_ctx.push_rounded_clip(rect, radius);
            child_clip = Some(LayerClip {
                rect,
                corner_radius: radius,
            });
        }

        for child in element.children() {
            if Self::build_boundary_walk(
                chunk_ctx,
                child.as_ref(),
                abs,
                child_clip,
                owner,
                container_id,
                container_size,
                layer_store,
                paint_renderer,
                scale_milli,
                layer_generation,
                next_ordinal,
                #[cfg(test)]
                paint_test_counters,
            ) {
                painted = true;
            }
        }

        if Self::append_retained_overlay_primitives(
            element,
            abs,
            child_clip,
            container_id,
            layer_store,
        ) {
            painted = true;
        } else if Self::paint_element_overlay(chunk_ctx, element, abs) {
            painted = true;
        }
        if clip.is_some() {
            chunk_ctx.pop_clip();
        }
        painted
    }

    fn ensure_boundary_layer<'a>(
        element: &'a dyn Element,
        layer_store: &mut LayerStore,
        paint_renderer: &mut CpuPaintRenderer,
        scale_milli: u32,
        layer_generation: u64,
        #[cfg(test)] paint_test_counters: &mut PaintTestCounters,
    ) -> bool {
        let Some(render_object) = element.render_object() else {
            return false;
        };
        let Some(size) = render_object.repaint_boundary_size() else {
            return false;
        };
        let layer_id = LayerId::Boundary(element.id());
        if layer_store.is_valid_for(layer_id, size, scale_milli) {
            layer_store.mark_container_subtree(layer_id, layer_generation);
            return true;
        }
        #[cfg(test)]
        {
            paint_test_counters.boundary_rebuilds += 1;
        }
        Self::rebuild_boundary_layer(
            element,
            size,
            layer_store,
            paint_renderer,
            scale_milli,
            layer_generation,
            #[cfg(test)]
            paint_test_counters,
        )
    }

    fn ensure_descendant_boundary_layers<'a>(
        element: &'a dyn Element,
        layer_store: &mut LayerStore,
        paint_renderer: &mut CpuPaintRenderer,
        scale_milli: u32,
        layer_generation: u64,
        #[cfg(test)] paint_test_counters: &mut PaintTestCounters,
    ) -> bool {
        let mut rebuilt = false;
        for child in element.children() {
            let is_boundary = child
                .render_object()
                .and_then(|render_object| render_object.repaint_boundary_size())
                .is_some();
            if is_boundary {
                if Self::ensure_boundary_layer(
                    child.as_ref(),
                    layer_store,
                    paint_renderer,
                    scale_milli,
                    layer_generation,
                    #[cfg(test)]
                    paint_test_counters,
                ) {
                    rebuilt = true;
                }
            }
            if Self::ensure_descendant_boundary_layers(
                child.as_ref(),
                layer_store,
                paint_renderer,
                scale_milli,
                layer_generation,
                #[cfg(test)]
                paint_test_counters,
            ) {
                rebuilt = true;
            }
        }
        rebuilt
    }

    fn flush_picture_chunk(
        chunk_ctx: &mut PaintContext<'_>,
        owner: ElementId,
        container_id: LayerId,
        container_size: Size,
        layer_store: &mut LayerStore,
        paint_renderer: &mut CpuPaintRenderer,
        scale_milli: u32,
        layer_generation: u64,
        next_ordinal: &mut u16,
    ) -> bool {
        if chunk_ctx.is_empty() {
            return false;
        }
        let ordinal = *next_ordinal;
        *next_ordinal = (*next_ordinal).saturating_add(1);
        let chunk_id = LayerId::Chunk { owner, ordinal };
        let logical_bounds = Rect::new(Point::ZERO, container_size);
        if let Some(chunk) = layer_store.chunk_mut(chunk_id)
            && let Some(buffer) = Arc::get_mut(&mut chunk.buffer)
        {
            if chunk.logical_bounds.size != container_size || buffer.scale_milli() != scale_milli {
                buffer.resize_logical_dimensions_with_scale(
                    libm::ceilf(container_size.width) as u32,
                    libm::ceilf(container_size.height) as u32,
                    scale_milli,
                );
            }
            paint_renderer.execute_into_external_buffer(
                buffer,
                crate::color::Color::TRANSPARENT,
                chunk_ctx,
                None,
            );
            chunk.logical_bounds = logical_bounds;
            chunk.generation = layer_generation;
            layer_store.mark_chunk(chunk_id, layer_generation);
            layer_store.finish_chunk_rebuild(chunk_id);
            layer_store.append_child(
                container_id,
                LayerChild::Chunk {
                    id: chunk_id,
                    offset: Point::ZERO,
                    clip: None,
                },
            );
            chunk_ctx.clear();
            return true;
        }

        let mut buffer = Buffer::from_logical_dimensions_with_scale(
            libm::ceilf(container_size.width) as u32,
            libm::ceilf(container_size.height) as u32,
            scale_milli,
        );
        paint_renderer.execute_into_external_buffer(
            &mut buffer,
            crate::color::Color::TRANSPARENT,
            chunk_ctx,
            None,
        );
        if let Some(chunk) = layer_store.chunk_mut(chunk_id) {
            chunk.buffer = Arc::new(buffer);
            chunk.logical_bounds = logical_bounds;
            chunk.generation = layer_generation;
            layer_store.finish_chunk_rebuild(chunk_id);
        } else {
            layer_store.insert_chunk(PictureChunk::new(
                owner,
                ordinal,
                logical_bounds,
                buffer,
                layer_generation,
            ));
        }
        layer_store.mark_chunk(chunk_id, layer_generation);
        layer_store.append_child(
            container_id,
            LayerChild::Chunk {
                id: chunk_id,
                offset: Point::ZERO,
                clip: None,
            },
        );
        chunk_ctx.clear();
        true
    }

    fn composite_layer_container<'a>(
        ctx: &mut PaintContext<'a>,
        layer_store: &LayerStore,
        container_id: LayerId,
        origin: Point,
    ) -> bool {
        let Some(container) = layer_store.container(container_id) else {
            return false;
        };
        let mut painted = false;
        for child in &container.children {
            match *child {
                LayerChild::Chunk { id, offset, .. } => {
                    if let Some(chunk) = layer_store.chunk(id) {
                        let dst = Rect::new(
                            Point::new(
                                origin.x + offset.x + chunk.logical_bounds.origin.x,
                                origin.y + offset.y + chunk.logical_bounds.origin.y,
                            ),
                            chunk.logical_bounds.size,
                        );
                        ctx.draw_buffer_rect_shared(
                            dst,
                            chunk.logical_bounds,
                            chunk.buffer.clone(),
                            1.0,
                        );
                        painted = true;
                    }
                }
                LayerChild::Boundary { id, offset, clip } => {
                    if let Some(clip) = clip {
                        ctx.push_rounded_clip(
                            Rect::new(
                                Point::new(
                                    origin.x + clip.rect.origin.x,
                                    origin.y + clip.rect.origin.y,
                                ),
                                clip.rect.size,
                            ),
                            clip.corner_radius,
                        );
                    }
                    if Self::composite_layer_container(
                        ctx,
                        layer_store,
                        id,
                        Point::new(origin.x + offset.x, origin.y + offset.y),
                    ) {
                        painted = true;
                    }
                    if clip.is_some() {
                        ctx.pop_clip();
                    }
                }
                LayerChild::Primitive(primitive) => {
                    Self::paint_layer_primitive(ctx, primitive, origin);
                    painted = true;
                }
            }
        }
        painted
    }

    fn rebuild_root_layer_refs(
        root: &dyn Element,
        window_size: Size,
        scale_milli: u32,
        layer_generation: u64,
        layer_store: &mut LayerStore,
    ) {
        layer_store.begin_container_rebuild(
            LayerId::Root,
            None,
            window_size,
            scale_milli,
            layer_generation,
        );
        Self::append_root_layer_refs(root, Point::ZERO, None, layer_store);
        layer_store.finish_container_rebuild(LayerId::Root);
    }

    fn append_root_layer_refs(
        element: &dyn Element,
        origin: Point,
        active_clip: Option<LayerClip>,
        layer_store: &mut LayerStore,
    ) {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );
        if element
            .render_object()
            .and_then(|render_object| render_object.repaint_boundary_size())
            .is_some()
        {
            let id = LayerId::Boundary(element.id());
            if layer_store.container(id).is_some() {
                layer_store.append_child(
                    LayerId::Root,
                    LayerChild::Boundary {
                        id,
                        offset: abs,
                        clip: active_clip,
                    },
                );
            }
            return;
        }

        let mut child_clip = active_clip;
        if let Some((rect, radius)) = Self::clip_for_element(element, abs) {
            child_clip = Some(LayerClip {
                rect,
                corner_radius: radius,
            });
        }
        for child in element.children() {
            Self::append_root_layer_refs(child.as_ref(), abs, child_clip, layer_store);
        }
        Self::append_retained_overlay_primitives(
            element,
            abs,
            child_clip,
            LayerId::Root,
            layer_store,
        );
    }

    fn sync_retained_layer_offsets_along_path(
        element: &dyn Element,
        origin: Point,
        active_clip: Option<LayerClip>,
        parent_container: LayerId,
        path: &[ElementId],
        path_index: usize,
        layer_store: &mut LayerStore,
        #[cfg(test)] paint_test_counters: &mut PaintTestCounters,
    ) -> bool {
        #[cfg(test)]
        {
            paint_test_counters.retained_sync_visits += 1;
        }
        if path.get(path_index).copied() != Some(element.id()) {
            return false;
        }

        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );
        let is_boundary = element
            .render_object()
            .and_then(|render_object| render_object.repaint_boundary_size())
            .is_some();
        if is_boundary {
            if !Self::sync_retained_boundary_ref(
                element.id(),
                abs,
                active_clip,
                parent_container,
                layer_store,
                false,
            ) {
                return false;
            }
            let id = LayerId::Boundary(element.id());
            if let Some(next_id) = path.get(path_index + 1).copied() {
                let Some(child) = element
                    .children()
                    .iter()
                    .find(|child| child.id() == next_id)
                else {
                    return false;
                };
                return Self::sync_retained_layer_offsets_along_path(
                    child.as_ref(),
                    Point::ZERO,
                    None,
                    id,
                    path,
                    path_index + 1,
                    layer_store,
                    #[cfg(test)]
                    paint_test_counters,
                );
            }
            return true;
        }

        let mut child_clip = active_clip;
        if let Some((rect, radius)) = Self::clip_for_element(element, abs) {
            child_clip = Some(LayerClip {
                rect,
                corner_radius: radius,
            });
        }
        if let Some(next_id) = path.get(path_index + 1).copied() {
            let Some(child) = element
                .children()
                .iter()
                .find(|child| child.id() == next_id)
            else {
                return false;
            };
            return Self::sync_retained_layer_offsets_along_path(
                child.as_ref(),
                abs,
                child_clip,
                parent_container,
                path,
                path_index + 1,
                layer_store,
                #[cfg(test)]
                paint_test_counters,
            );
        }

        for child in element.children() {
            if child
                .render_object()
                .and_then(|render_object| render_object.repaint_boundary_size())
                .is_some()
            {
                if !Self::sync_retained_boundary_child(
                    child.as_ref(),
                    abs,
                    child_clip,
                    parent_container,
                    layer_store,
                ) {
                    return false;
                }
            }
        }
        Self::sync_retained_overlay_primitives(
            element,
            abs,
            child_clip,
            parent_container,
            layer_store,
            #[cfg(test)]
            paint_test_counters,
        );
        true
    }

    fn sync_retained_layer_offsets_for_path_target(
        mut element: &dyn Element,
        path: &[ElementId],
        layer_store: &mut LayerStore,
        #[cfg(test)] paint_test_counters: &mut PaintTestCounters,
    ) -> bool {
        let mut origin = Point::ZERO;
        let mut active_clip = None;
        let mut parent_container = LayerId::Root;
        let mut path_index = 0usize;

        loop {
            if path.get(path_index).copied() != Some(element.id()) {
                return false;
            }

            let abs = Point::new(
                origin.x + element.position().x,
                origin.y + element.position().y,
            );
            let is_boundary = element
                .render_object()
                .and_then(|render_object| render_object.repaint_boundary_size())
                .is_some();

            if is_boundary {
                if !Self::sync_retained_boundary_ref(
                    element.id(),
                    abs,
                    active_clip,
                    parent_container,
                    layer_store,
                    false,
                ) {
                    return false;
                }
                if path_index + 1 == path.len() {
                    #[cfg(test)]
                    {
                        paint_test_counters.localized_retained_syncs += 1;
                    }
                    return true;
                }
                let Some(next_id) = path.get(path_index + 1).copied() else {
                    return false;
                };
                let Some(child) = element
                    .children()
                    .iter()
                    .find(|child| child.id() == next_id)
                else {
                    return false;
                };
                element = child.as_ref();
                origin = Point::ZERO;
                active_clip = None;
                parent_container = LayerId::Boundary(path[path_index]);
                path_index += 1;
                continue;
            }

            let mut child_clip = active_clip;
            if let Some((rect, radius)) = Self::clip_for_element(element, abs) {
                child_clip = Some(LayerClip {
                    rect,
                    corner_radius: radius,
                });
            }

            if path_index + 1 == path.len() {
                for child in element.children() {
                    if child
                        .render_object()
                        .and_then(|render_object| render_object.repaint_boundary_size())
                        .is_some()
                        && !Self::sync_retained_boundary_child(
                            child.as_ref(),
                            abs,
                            child_clip,
                            parent_container,
                            layer_store,
                        )
                    {
                        return false;
                    }
                }
                Self::sync_retained_overlay_primitives(
                    element,
                    abs,
                    child_clip,
                    parent_container,
                    layer_store,
                    #[cfg(test)]
                    paint_test_counters,
                );
                #[cfg(test)]
                {
                    paint_test_counters.localized_retained_syncs += 1;
                }
                return true;
            }

            let Some(next_id) = path.get(path_index + 1).copied() else {
                return false;
            };
            let Some(child) = element
                .children()
                .iter()
                .find(|child| child.id() == next_id)
            else {
                return false;
            };
            element = child.as_ref();
            origin = abs;
            active_clip = child_clip;
            path_index += 1;
        }
    }

    fn sync_retained_boundary_ref(
        element_id: ElementId,
        offset: Point,
        active_clip: Option<LayerClip>,
        parent_container: LayerId,
        layer_store: &mut LayerStore,
        allow_missing_non_retained: bool,
    ) -> bool {
        let id = LayerId::Boundary(element_id);
        let retained_container_exists = layer_store.container(id).is_some();
        let Some(container) = layer_store.container_mut(parent_container) else {
            return false;
        };
        if let Some(child) = container.children.iter_mut().find(
            |child| matches!(child, LayerChild::Boundary { id: child_id, .. } if *child_id == id),
        ) {
            if !retained_container_exists {
                return false;
            }
            *child = LayerChild::Boundary {
                id,
                offset,
                clip: active_clip,
            };
            return true;
        }
        allow_missing_non_retained && !retained_container_exists
    }

    fn sync_retained_boundary_child(
        element: &dyn Element,
        origin: Point,
        active_clip: Option<LayerClip>,
        parent_container: LayerId,
        layer_store: &mut LayerStore,
    ) -> bool {
        let offset = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );
        Self::sync_retained_boundary_ref(
            element.id(),
            offset,
            active_clip,
            parent_container,
            layer_store,
            true,
        )
    }

    fn append_retained_overlay_primitives(
        element: &dyn Element,
        origin: Point,
        active_clip: Option<LayerClip>,
        container_id: LayerId,
        layer_store: &mut LayerStore,
    ) -> bool {
        let Some(render_object) = element.render_object() else {
            return false;
        };
        let mut primitives = [None, None];
        render_object.retained_overlay_primitives(
            element.id(),
            origin,
            active_clip,
            &mut primitives,
        );
        let mut appended = false;
        for primitive in primitives.into_iter().flatten() {
            layer_store.append_child(container_id, LayerChild::Primitive(primitive));
            appended = true;
        }
        appended
    }

    fn sync_retained_overlay_primitives(
        element: &dyn Element,
        origin: Point,
        active_clip: Option<LayerClip>,
        container_id: LayerId,
        layer_store: &mut LayerStore,
        #[cfg(test)] paint_test_counters: &mut PaintTestCounters,
    ) {
        let Some(render_object) = element.render_object() else {
            return;
        };
        let mut primitives = [None, None];
        render_object.retained_overlay_primitives(
            element.id(),
            origin,
            active_clip,
            &mut primitives,
        );
        if layer_store.replace_stable_primitive_range(container_id, element.id(), &primitives) {
            #[cfg(test)]
            {
                paint_test_counters.retained_primitive_slot_syncs += 1;
            }
            return;
        }
        #[cfg(test)]
        {
            paint_test_counters.retained_primitive_scan_syncs += 1;
        }
        let Some(container) = layer_store.container_mut(container_id) else {
            return;
        };
        let mut primitive_index = 0usize;
        let mut child_index = 0usize;
        while child_index < container.children.len() {
            let matches_owner = matches!(
                container.children[child_index],
                LayerChild::Primitive(LayerPrimitive { owner, .. }) if owner == element.id()
            );
            if matches_owner {
                if let Some(primitive) = primitives.get_mut(primitive_index).and_then(Option::take)
                {
                    container.children[child_index] = LayerChild::Primitive(primitive);
                    primitive_index += 1;
                    child_index += 1;
                } else {
                    container.children.remove(child_index);
                }
            } else {
                child_index += 1;
            }
        }
        for primitive in primitives.into_iter().flatten() {
            container.children.push(LayerChild::Primitive(primitive));
        }
        layer_store.rebuild_primitive_ranges(container_id);
    }

    fn paint_layer_primitive(ctx: &mut PaintContext<'_>, primitive: LayerPrimitive, origin: Point) {
        match primitive.kind {
            LayerPrimitiveKind::RoundedRect {
                mut rect,
                corner_radius,
            } => {
                rect.origin.x += origin.x;
                rect.origin.y += origin.y;
                if let Some(clip) = primitive.clip {
                    ctx.push_rounded_clip(
                        Rect::new(
                            Point::new(
                                origin.x + clip.rect.origin.x,
                                origin.y + clip.rect.origin.y,
                            ),
                            clip.rect.size,
                        ),
                        clip.corner_radius,
                    );
                    ctx.fill_rounded_rect(rect, corner_radius, primitive.color);
                    ctx.pop_clip();
                } else {
                    ctx.fill_rounded_rect(rect, corner_radius, primitive.color);
                }
            }
        }
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

    fn subtree_emits_paint_extension(element: &dyn Element) -> bool {
        element
            .render_object()
            .is_some_and(|render_object| render_object.emits_paint_extension())
            || element
                .children()
                .iter()
                .any(|child| Self::subtree_emits_paint_extension(child.as_ref()))
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

    fn invalidate_repaint_boundary_caches(&mut self) {
        if self.dirty_scratch.ids.is_empty() {
            return;
        }

        for index in 0..self.dirty_scratch.ids.len() {
            let dirty_id = self.dirty_scratch.ids[index];
            self.dirty_scratch.path.clear();
            if !self
                .element_tree
                .find_path_ids_into(dirty_id, &mut self.dirty_scratch.path)
            {
                continue;
            }

            let layer_id = self
                .element_tree
                .find_nearest_repaint_boundary_in_path(&self.dirty_scratch.path)
                .map(LayerId::Boundary)
                .unwrap_or(LayerId::Root);

            let legacy_id = match layer_id {
                LayerId::Boundary(id) => Some(id),
                LayerId::Root | LayerId::Chunk { .. } => None,
            };
            if crate::debug::repaint_boundary_log_enabled() {
                crate::logln!(
                    "[RepaintBoundary] invalidate id={} dirty_id={} retained_layer={:?} path_len={}",
                    legacy_id.map_or(0, ElementId::get),
                    dirty_id.get(),
                    layer_id,
                    self.dirty_scratch.path.len(),
                );
            }

            // Phase 4 stores nested repaint boundaries as child layers, not as
            // pixels flattened into the ancestor picture chunk. Therefore only
            // the nearest owning boundary must be invalidated for descendant
            // content changes. Ancestor overlay/chrome retention (for example,
            // scrollbars painted over child boundaries) remains a Phase 7 item.
            self.layer_store.invalidate_layer(layer_id, dirty_id);
            if let Some(id) = legacy_id
                && let Some(cache) = self.paint_caches.get_mut(&id)
            {
                cache.valid = false;
                cache.invalidated_by = Some(dirty_id);
            }
        }
    }

    fn element_paint_bounds(element: &dyn Element, absolute_origin: Point) -> Rect {
        let bounds = element.bounds();
        let mut width = bounds.size.width;
        let mut height = bounds.size.height;
        let render_object = element.render_object();
        if let Some(select) = render_object.and_then(|render_object| {
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
        let bounds = Rect::from_xywh(absolute_origin.x, absolute_origin.y, width, height);
        render_object.map_or(bounds, |render_object| {
            Self::apply_paint_outsets(bounds, render_object.paint_outsets())
        })
    }

    fn apply_paint_outsets(bounds: Rect, outsets: crate::geometry::EdgeInsets) -> Rect {
        let finite_positive = |value: f32| {
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            }
        };
        let left = finite_positive(outsets.left);
        let top = finite_positive(outsets.top);
        let right = finite_positive(outsets.right);
        let bottom = finite_positive(outsets.bottom);
        Rect::from_xywh(
            bounds.origin.x - left,
            bounds.origin.y - top,
            bounds.size.width + left + right,
            bounds.size.height + top + bottom,
        )
    }

    fn paint_dirty_rects_into(
        element_tree: &crate::element::ElementTree,
        last_paint_bounds: &BTreeMap<ElementId, Rect>,
        dirty_ids: &[ElementId],
        self_paint_ids: &[ElementId],
        path_scratch: &mut Vec<ElementId>,
        rects: &mut Vec<Rect>,
    ) {
        rects.clear();
        if dirty_ids.is_empty() {
            return;
        }

        for dirty_id in dirty_ids {
            path_scratch.clear();
            if !element_tree.find_path_ids_into(*dirty_id, path_scratch) {
                continue;
            }
            let self_paint_only = self_paint_ids.binary_search(dirty_id).is_ok();
            let current_bounds = if self_paint_only {
                Self::element_self_paint_bounds_for_path(element_tree, path_scratch)
            } else {
                Self::element_paint_bounds_for_path(element_tree, path_scratch)
            };
            if let Some(current_bounds) = current_bounds {
                rects.push(current_bounds);
            }
            if !self_paint_only
                && let Some(old_bounds) = last_paint_bounds.get(dirty_id)
                && !Self::paint_bounds_is_empty(*old_bounds)
            {
                rects.push(*old_bounds);
            }
        }
    }

    fn collect_paint_bounds(
        element: &dyn Element,
        origin: Point,
        active_clip: Option<Rect>,
        bounds: &mut BTreeMap<ElementId, Rect>,
    ) {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );
        let paint_bounds = Self::element_paint_bounds(element, abs);
        let visible_bounds = active_clip
            .and_then(|clip| Self::intersect_paint_bounds(paint_bounds, clip))
            .unwrap_or_else(|| {
                if active_clip.is_some() {
                    Rect::from_xywh(abs.x, abs.y, 0.0, 0.0)
                } else {
                    paint_bounds
                }
            });
        bounds.insert(element.id(), visible_bounds);

        let child_clip = match (active_clip, Self::clip_for_element(element, abs)) {
            (Some(active), Some((clip, _))) => Some(
                Self::intersect_paint_bounds(active, clip)
                    .unwrap_or_else(|| Rect::from_xywh(abs.x, abs.y, 0.0, 0.0)),
            ),
            (Some(active), None) => Some(active),
            (None, Some((clip, _))) => Some(clip),
            (None, None) => None,
        };

        for child in element.children() {
            Self::collect_paint_bounds(child.as_ref(), abs, child_clip, bounds);
        }
    }

    fn element_paint_bounds_for_path(
        element_tree: &crate::element::ElementTree,
        path: &[ElementId],
    ) -> Option<Rect> {
        let mut element = element_tree.root()?;
        let mut origin = Point::ZERO;
        let mut active_clip = None;

        for (index, id) in path.iter().copied().enumerate() {
            if element.id() != id {
                return None;
            }
            let abs = Point::new(
                origin.x + element.position().x,
                origin.y + element.position().y,
            );
            if index + 1 == path.len() {
                let paint_bounds = Self::element_paint_bounds(element, abs);
                return match active_clip {
                    Some(clip) => Self::intersect_paint_bounds(paint_bounds, clip),
                    None => Some(paint_bounds),
                };
            }

            if let Some((clip, _)) = Self::clip_for_element(element, abs) {
                active_clip = match active_clip {
                    Some(active) => Self::intersect_paint_bounds(active, clip),
                    None => Some(clip),
                };
                active_clip?;
            }
            let next_id = path[index + 1];
            element = element
                .children()
                .iter()
                .find(|child| child.id() == next_id)?
                .as_ref();
            origin = abs;
        }
        None
    }

    fn element_self_paint_bounds_for_path(
        element_tree: &crate::element::ElementTree,
        path: &[ElementId],
    ) -> Option<Rect> {
        let mut element = element_tree.root()?;
        let mut origin = Point::ZERO;
        let mut active_clip = None;

        for (index, id) in path.iter().copied().enumerate() {
            if element.id() != id {
                return None;
            }
            let abs = Point::new(
                origin.x + element.position().x,
                origin.y + element.position().y,
            );
            if index + 1 == path.len() {
                let paint_bounds = element
                    .render_object()
                    .and_then(|render_object| render_object.self_paint_bounds(abs))
                    .unwrap_or_else(|| Self::element_paint_bounds(element, abs));
                return match active_clip {
                    Some(clip) => Self::intersect_paint_bounds(paint_bounds, clip),
                    None => Some(paint_bounds),
                };
            }

            if let Some((clip, _)) = Self::clip_for_element(element, abs) {
                active_clip = match active_clip {
                    Some(active) => Self::intersect_paint_bounds(active, clip),
                    None => Some(clip),
                };
                active_clip?;
            }
            let next_id = path[index + 1];
            element = element
                .children()
                .iter()
                .find(|child| child.id() == next_id)?
                .as_ref();
            origin = abs;
        }
        None
    }

    fn intersect_paint_bounds(left: Rect, right: Rect) -> Option<Rect> {
        let x = left.left().max(right.left());
        let y = left.top().max(right.top());
        let right_edge = left.right().min(right.right());
        let bottom = left.bottom().min(right.bottom());
        (right_edge > x && bottom > y).then(|| Rect::from_xywh(x, y, right_edge - x, bottom - y))
    }

    fn paint_bounds_is_empty(bounds: Rect) -> bool {
        bounds.size.width <= 0.0 || bounds.size.height <= 0.0
    }

    fn update_paint_bounds_for_ids(
        element_tree: &crate::element::ElementTree,
        ids: &[ElementId],
        path_scratch: &mut Vec<ElementId>,
        bounds: &mut BTreeMap<ElementId, Rect>,
    ) {
        for id in ids {
            path_scratch.clear();
            if !element_tree.find_path_ids_into(*id, path_scratch) {
                continue;
            }
            let visible_bounds = Self::element_paint_bounds_for_path(element_tree, path_scratch)
                .or_else(|| {
                    element_tree
                        .element_and_absolute_origin_for_path(path_scratch)
                        .map(|(_, origin)| Rect::from_xywh(origin.x, origin.y, 0.0, 0.0))
                });
            if let Some(visible_bounds) = visible_bounds {
                bounds.insert(*id, visible_bounds);
            }
        }
    }

    fn overlaps_any(rect: Rect, rects: &[Rect]) -> bool {
        rects.iter().any(|r| rect.overlaps(r))
    }

    fn merge_overlapping_rects(rects: &mut Vec<Rect>) {
        let mut index = 0;
        while index < rects.len() {
            let mut other = index + 1;
            while other < rects.len() {
                if rects[index].overlaps(&rects[other]) {
                    let left = rects[index].left().min(rects[other].left());
                    let top = rects[index].top().min(rects[other].top());
                    let right = rects[index].right().max(rects[other].right());
                    let bottom = rects[index].bottom().max(rects[other].bottom());
                    rects[index] = Rect::from_xywh(left, top, right - left, bottom - top);
                    rects.remove(other);
                } else {
                    other += 1;
                }
            }
            index += 1;
        }
    }

    fn present_damage_rects(
        rects: &[Rect],
        window_size: Size,
        scale_milli: u32,
    ) -> Option<Vec<DamageRect>> {
        let mut damage = Vec::new();
        if Self::present_damage_rects_into(rects, window_size, scale_milli, &mut damage) {
            Some(damage)
        } else {
            None
        }
    }

    fn present_damage_rects_into(
        rects: &[Rect],
        window_size: Size,
        scale_milli: u32,
        damage: &mut Vec<DamageRect>,
    ) -> bool {
        damage.clear();
        let physical_width = Self::scale_len(window_size.width as u32, scale_milli);
        let physical_height = Self::scale_len(window_size.height as u32, scale_milli);
        for rect in rects {
            let damage_rect =
                Self::rect_to_damage(*rect, scale_milli, physical_width, physical_height);
            if damage_rect.2 > 0 && damage_rect.3 > 0 {
                damage.push(damage_rect);
            }
        }

        Self::coalesce_damage_rects(damage);

        let damage_area = Self::damage_rects_area(damage);
        let window_area = (physical_width as u64).saturating_mul(physical_height as u64);
        if damage_area >= window_area {
            damage.clear();
            return false;
        }

        true
    }

    fn store_paint_damage(&mut self, partial: bool) {
        if partial {
            let damage = self.paint_damage.get_or_insert_with(Vec::new);
            damage.clear();
            damage.extend_from_slice(&self.dirty_scratch.damage);
        } else {
            self.paint_damage = None;
        }
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
        match self.render_for_present().ok()? {
            PresentedFrame::Cpu { buffer, damage } => Some((buffer, damage)),
            PresentedFrame::External | PresentedFrame::Idle => None,
        }
    }

    /// Render a frame using the configured paint backend.
    ///
    /// CPU frames still need to be passed to the platform window. External
    /// frames have already been presented by their backend.
    ///
    /// # Returns
    ///
    /// The frame presentation state, or a backend rendering error.
    pub fn render_for_present(&mut self) -> crate::error::Result<PresentedFrame<'_>> {
        if !self.paint_enabled {
            let _ = self.render();
            let Some(renderer) = self.renderer.as_ref() else {
                return Ok(PresentedFrame::Idle);
            };
            return Ok(PresentedFrame::Cpu {
                buffer: renderer.buffer(),
                damage: renderer.damage(),
            });
        }

        crate::graphics::set_current_scale_milli(self.scale_milli);
        self.pipeline_owner.flush_with_legacy_paint(
            &mut self.element_tree,
            self.window_size,
            false,
        );
        let background_color = self.extract_background_color();
        self.render_paint_path(background_color)
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
    use crate::element::ComponentElement;
    use crate::event::{
        Event, KeyCode, KeyEvent, MouseButton, MouseEvent, ScrollSource, WheelPhase,
    };
    use crate::state::{State, StateId};
    use crate::testing::alloc_counter::{
        allocation_snapshot, measure_allocations, reset_allocation_counts,
    };
    use crate::view::{View, ViewExt};
    use crate::views::{
        Button, Either, GridView, LazyVStack, MenuItem, NavigationLink, NavigationView, Rectangle,
        ScrollView, ScrollbarVisibility, Select, Text, TextField, Toggle, Window,
    };
    use core::cell::Cell;
    use std::rc::Rc;

    #[derive(Clone)]
    struct PaintExtensionProbe;

    #[test]
    fn paint_outsets_expand_damage_bounds_conservatively() {
        let bounds = Rect::from_xywh(20.0, 30.0, 80.0, 40.0);
        let expanded = RenderingPipeline::apply_paint_outsets(
            bounds,
            crate::geometry::EdgeInsets::new(6.0, 4.0, 8.0, 10.0),
        );

        assert_eq!(expanded, Rect::from_xywh(14.0, 26.0, 94.0, 54.0));
    }

    #[test]
    fn runtime_input_environment_transition_invalidates_layout_and_paint() {
        let desktop = crate::InputEnvironment::desktop();
        let _environment_guard = crate::input_environment::install_test_input_environment(desktop);
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(Rectangle::new().frame(100.0, 100.0).create_element());
        pipeline.layout_initial();
        let _ = pipeline.render_with_damage();
        assert!(!pipeline.has_dirty());

        crate::input_environment::install_input_environment(crate::InputEnvironment::new(
            1,
            Some(true),
            None,
            true,
            false,
            false,
            false,
        ));
        pipeline.invalidate_input_environment();
        assert!(pipeline.has_dirty());
        let _ = pipeline.render_with_damage();
        assert!(!pipeline.has_dirty());
    }

    impl View for PaintExtensionProbe {
        fn create_element(&self) -> Box<dyn Element> {
            Box::new(crate::element::RenderElement::new(
                self.clone(),
                PaintExtensionProbeRenderObject { size: Size::ZERO },
            ))
        }

        fn as_any(&self) -> &dyn core::any::Any {
            self
        }
    }

    struct PaintExtensionProbeRenderObject {
        size: Size,
    }

    impl crate::element::ElementRenderObject for PaintExtensionProbeRenderObject {
        fn layout(&mut self, constraints: LayoutConstraints) -> Size {
            self.size = constraints.constrain(Size::new(1.0, 1.0));
            self.size
        }

        fn size(&self) -> Size {
            self.size
        }

        fn render(&mut self) {}

        fn paint<'a>(&'a self, ctx: &mut PaintContext<'a>, origin: Point) -> bool {
            ctx.draw_extension(Rect::new(origin, self.size), Arc::new(0u8));
            true
        }

        fn emits_paint_extension(&self) -> bool {
            true
        }

        fn as_any(&self) -> &dyn core::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
            self
        }
    }

    #[derive(Clone)]
    struct PaintOnlyComponentHarness {
        color: State<crate::color::Color>,
    }

    fn build_paint_only_patch(view: &PaintOnlyComponentHarness) -> Box<dyn View> {
        Box::new(PaintOnlyPatch {
            color: view.color.get(),
        })
    }

    impl View for PaintOnlyComponentHarness {
        fn create_element(&self) -> Box<dyn Element> {
            Box::new(ComponentElement::new_with_builder(
                self.clone(),
                build_paint_only_patch,
            ))
        }

        fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
            alloc::vec![&self.color]
        }

        fn as_any(&self) -> &dyn core::any::Any {
            self
        }
    }

    #[derive(Clone)]
    struct PaintOnlyPatch {
        color: crate::color::Color,
    }

    impl View for PaintOnlyPatch {
        fn create_element(&self) -> Box<dyn Element> {
            Box::new(crate::element::RenderElement::new(
                self.clone(),
                PaintOnlyPatchRenderObject {
                    color: self.color,
                    size: Size::ZERO,
                },
            ))
        }

        fn as_any(&self) -> &dyn core::any::Any {
            self
        }
    }

    struct PaintOnlyPatchRenderObject {
        color: crate::color::Color,
        size: Size,
    }

    impl crate::element::ElementRenderObject for PaintOnlyPatchRenderObject {
        fn layout(&mut self, _constraints: LayoutConstraints) -> Size {
            self.size = Size::new(40.0, 40.0);
            self.size
        }

        fn size(&self) -> Size {
            self.size
        }

        fn render(&mut self) {}

        fn paint<'a>(&'a self, ctx: &mut PaintContext<'a>, origin: Point) -> bool {
            ctx.fill_rect(Rect::new(origin, self.size), self.color);
            true
        }

        fn update(&mut self, new_view: &dyn View) -> crate::element::UpdateResult {
            let Some(patch) = new_view.as_any().downcast_ref::<PaintOnlyPatch>() else {
                return crate::element::UpdateResult::Replaced;
            };
            if self.color == patch.color {
                return crate::element::UpdateResult::NoChange;
            }
            self.color = patch.color;
            crate::element::UpdateResult::Updated
        }

        fn as_any(&self) -> &dyn core::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
            self
        }
    }

    #[derive(Clone)]
    struct LauncherInputHarness {
        query: State<alloc::string::String>,
        focus_requested: State<bool>,
        selection_moves: State<usize>,
    }

    fn build_launcher_input(view: &LauncherInputHarness) -> Box<dyn View> {
        let query = view.query.clone();
        let focus_requested = view.focus_requested.clone();
        let empty_focus = focus_requested.clone();
        let selection_moves = view.selection_moves.clone();
        Box::new(
            TextField::new(view.query.clone())
                .autofocus(view.focus_requested.get())
                .on_empty(move || empty_focus.set(false))
                .on_key(move |event| match event {
                    KeyEvent::Char { c } if !c.is_control() => {
                        focus_requested.set(true);
                        query.update(|value| value.push(c));
                        true
                    }
                    KeyEvent::Pressed {
                        keycode: KeyCode::Up,
                        ..
                    } => {
                        selection_moves.update(|moves| *moves += 1);
                        true
                    }
                    _ => false,
                }),
        )
    }

    impl View for LauncherInputHarness {
        fn create_element(&self) -> Box<dyn Element> {
            Box::new(ComponentElement::new_with_builder(
                self.clone(),
                build_launcher_input,
            ))
        }

        fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
            alloc::vec![&self.query, &self.focus_requested]
        }

        fn as_any(&self) -> &dyn core::any::Any {
            self
        }
    }

    #[test]
    fn paint_only_reconciliation_does_not_damage_full_ancestor() {
        let color = State::new(
            crate::state::generate_state_id(),
            crate::color::Color::rgb(220, 40, 40),
        );
        let root = PaintOnlyComponentHarness {
            color: color.clone(),
        }
        .frame(800.0, 600.0)
        .create_element();
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();

        color.set(crate::color::Color::rgb(40, 80, 220));
        let (_, damage) = pipeline
            .render_with_damage()
            .expect("paint-only state update should render a frame");
        let damage = damage.expect("paint-only state update should remain partial");

        assert!(!damage.is_empty());
        assert!(damage.iter().all(|&(x, y, width, height)| {
            x.saturating_add(width) <= 40 && y.saturating_add(height) <= 40
        }));
    }

    #[test]
    fn navigation_hover_damages_only_changed_sidebar_rows() {
        let root = NavigationView::new((
            NavigationLink::new("First", || Text::new("first page")),
            NavigationLink::new("Second", || Text::new("second page")),
        ))
        .sidebar_width(150.0)
        .create_element();
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();

        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 10, y: 10 })));
        let (_, damage) = pipeline
            .render_with_damage()
            .expect("navigation hover should render a frame");
        let damage = damage.expect("navigation hover should remain partial");

        assert_eq!(damage, &[(0, 0, 150, 40)]);

        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 10, y: 50 })));
        let (_, damage) = pipeline
            .render_with_damage()
            .expect("moving hover between rows should render a frame");
        let damage = damage.expect("moving hover between rows should remain partial");

        assert_eq!(damage, &[(0, 0, 150, 80)]);

        pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 200, y: 50 }));
        let (_, damage) = pipeline
            .render_with_damage()
            .expect("leaving the sidebar should clear its hover");
        let damage = damage.expect("clearing sidebar hover should remain partial");

        assert_eq!(damage, &[(0, 40, 150, 40)]);
    }

    #[test]
    fn moving_between_children_does_not_exit_shared_hover_ancestor() {
        let entered = Rc::new(Cell::new(0u32));
        let exited = Rc::new(Cell::new(0u32));
        let entered_callback = entered.clone();
        let exited_callback = exited.clone();
        let target = crate::hstack! {
            Rectangle::new().frame(40.0, 40.0),
            Rectangle::new().frame(40.0, 40.0),
        }
        .spacing(0.0)
        .on_hover(move || entered_callback.set(entered_callback.get() + 1))
        .on_exit(move || exited_callback.set(exited_callback.get() + 1));

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(target.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 10, y: 10 }));
        assert_eq!(entered.get(), 1);
        assert_eq!(exited.get(), 0);

        pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 50, y: 10 }));
        assert_eq!(entered.get(), 1);
        assert_eq!(exited.get(), 0);

        pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 900, y: 700 }));
        assert_eq!(entered.get(), 1);
        assert_eq!(exited.get(), 1);
    }

    #[test]
    fn button_release_outside_capture_cancels_click() {
        let clicked = State::new(crate::state::generate_state_id(), false);
        let clicked_callback = clicked.clone();
        let button = Button::new("Task").on_click(move || clicked_callback.set(true));

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(button.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 10, y: 10 }));
        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 10,
                y: 10,
                click_count: 1,
            }))
        );
        pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 200, y: 200 }));
        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 200,
                y: 200,
                click_count: 1,
            }))
        );

        assert!(!clicked.get());
    }

    #[test]
    fn touch_terminal_events_clear_hover_and_only_release_activates() {
        let click_count = Rc::new(Cell::new(0));
        let click_count_for_button = click_count.clone();
        let hover_enter_count = Rc::new(Cell::new(0));
        let hover_enter_for_button = hover_enter_count.clone();
        let hover_exit_count = Rc::new(Cell::new(0));
        let hover_exit_for_button = hover_exit_count.clone();
        let button = Button::new("Task")
            .on_click(move || click_count_for_button.set(click_count_for_button.get() + 1))
            .on_hover(move || hover_enter_for_button.set(hover_enter_for_button.get() + 1))
            .on_exit(move || hover_exit_for_button.set(hover_exit_for_button.get() + 1));

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(button.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 10, y: 10 }));
        assert_eq!(hover_enter_count.get(), 1);
        pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 10,
            y: 10,
            click_count: 1,
        }));
        pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonCancelled {
            button: MouseButton::Left,
            x: 10,
            y: 10,
        }));
        assert_eq!(click_count.get(), 0);
        pipeline.handle_event(&Event::Mouse(MouseEvent::Exited { x: 10, y: 10 }));
        assert_eq!(hover_exit_count.get(), 1);

        pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 10, y: 10 }));
        assert_eq!(hover_enter_count.get(), 2);
        pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 10,
            y: 10,
            click_count: 1,
        }));
        pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: 10,
            y: 10,
            click_count: 1,
        }));
        assert_eq!(click_count.get(), 1);
        pipeline.handle_event(&Event::Mouse(MouseEvent::Exited { x: 10, y: 10 }));
        assert_eq!(hover_exit_count.get(), 2);
    }

    #[test]
    fn cancelled_pointer_does_not_select_navigation_or_call_its_callback() {
        let callback_count = Rc::new(Cell::new(0));
        let callback_count_for_link = callback_count.clone();
        let navigation = NavigationView::new((
            NavigationLink::new("First", || Text::new("first page")),
            NavigationLink::new("Second", || Text::new("second page")).on_select(move || {
                callback_count_for_link.set(callback_count_for_link.get() + 1);
            }),
        ))
        .sidebar_width(150.0);
        let selected_index = navigation.selected_index_state().clone();

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(navigation.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 10,
            y: 50,
            click_count: 1,
        }));
        assert!(
            !pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                x: 10,
                y: 50,
            }))
        );
        assert_eq!(selected_index.get(), 0);
        assert_eq!(callback_count.get(), 0);
    }

    #[test]
    fn cancelled_pointer_does_not_open_or_change_select() {
        let selected_index = State::new(crate::state::generate_state_id(), 0);
        let change_count = Rc::new(Cell::new(0));
        let change_count_for_select = change_count.clone();
        let select = Select::new(
            alloc::vec![String::from("First"), String::from("Second")],
            selected_index.clone(),
        )
        .on_change(move |_| change_count_for_select.set(change_count_for_select.get() + 1));

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(select.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 10,
            y: 10,
            click_count: 1,
        }));
        assert!(
            !pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                x: 10,
                y: 10,
            }))
        );
        assert_eq!(selected_index.get(), 0);
        assert_eq!(change_count.get(), 0);
    }

    #[test]
    fn taskbar_menu_item_clears_hover_and_press_when_pointer_leaves() {
        let clicked = Rc::new(Cell::new(false));
        let clicked_callback = clicked.clone();
        let item = MenuItem::new("Terminal").on_click(move || clicked_callback.set(true));

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(item.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 10, y: 10 }));
        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 10,
                y: 10,
                click_count: 1,
            }))
        );
        pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 900, y: 700 }));
        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 900,
                y: 700,
                click_count: 1,
            }))
        );

        assert!(!clicked.get());
        let render_object = pipeline
            .element_tree
            .root()
            .and_then(|root| root.render_object())
            .and_then(|render_object| {
                render_object
                    .as_any()
                    .downcast_ref::<crate::views::menu::MenuItemRenderObject>()
            })
            .expect("taskbar item should retain its menu-item render object");
        assert!(!render_object.is_hovered());
        assert!(!render_object.is_pressed());
    }

    /// Phase 1 warm-scroll allocation baseline measured on this machine.
    /// This is the Phase 2 max budget and Phase 7's target is zero.
    const PHASE_1_WARM_SCROLL_ALLOCS: usize = 22;
    /// Phase 2 retained-scratch warm-scroll allocation baseline measured on this machine.
    const PHASE_2_WARM_SCROLL_ALLOCS: usize = 21;
    /// Phase 3 shadow LayerStore warm-scroll allocation baseline measured on this machine.
    const PHASE_3_WARM_SCROLL_ALLOCS: usize = 21;
    /// Phase 4 nested retained-boundary warm-scroll allocation baseline measured on this machine.
    const PHASE_4_WARM_SCROLL_ALLOCS: usize = 21;
    /// Phase 5 nearest-invalidation warm-scroll allocation baseline measured on this machine.
    const PHASE_5_WARM_SCROLL_ALLOCS: usize = 13;
    /// Phase 6a cascade-fix warm-scroll allocation baseline measured on this machine.
    const PHASE_6A_WARM_SCROLL_ALLOCS: usize = 13;
    /// Phase 6b retained-composite fast-path warm-scroll allocation baseline.
    const PHASE_6B_WARM_SCROLL_ALLOCS: usize = 0;
    /// Phase 7 retained-scrollbar visible warm-scroll allocation baseline.
    const PHASE_7_WARM_SCROLL_ALLOCS: usize = 0;

    fn warm_scroll_event() -> Event {
        Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: -40,
            x: 10,
            y: 10,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        })
    }

    fn build_warm_scroll_pipeline() -> RenderingPipeline {
        build_warm_scroll_pipeline_with_visibility(ScrollbarVisibility::Never)
    }

    fn build_warm_scroll_pipeline_with_visibility(
        visibility: ScrollbarVisibility,
    ) -> RenderingPipeline {
        let mut pipeline = RenderingPipeline::new();
        let root = ScrollView::new(
            Rectangle::new()
                .fill(crate::color::Color::rgb(220, 40, 40))
                .frame(100.0, 2_000.0),
        )
        .content_size(100.0, 2_000.0)
        .wheel_sensitivity(1.0)
        .scrollbar_visibility(visibility)
        .frame(100.0, 100.0)
        .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();

        for _ in 0..4 {
            drive_warm_scroll_frame(&mut pipeline);
        }

        pipeline
    }

    fn drive_warm_scroll_frame(pipeline: &mut RenderingPipeline) {
        assert!(pipeline.handle_event(&warm_scroll_event()));
        pipeline
            .render_with_damage()
            .expect("warm scroll frame should render");
    }

    #[test]
    fn transparent_window_scroll_recomposes_the_window_background() {
        let scroll = ScrollView::new(
            Rectangle::new()
                .fill(crate::color::Color::rgb(220, 40, 40))
                .frame(100.0, 2_000.0),
        )
        .content_size(100.0, 2_000.0)
        .wheel_sensitivity(1.0)
        .scrollbar_visibility(ScrollbarVisibility::Never)
        .frame(100.0, 100.0);
        let window = Window::new("Transparent scroll", scroll).size(Size::new(120.0, 160.0));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(window.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: -40,
            x: 10,
            y: 40,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        })));
        let (buffer, damage) = pipeline
            .render_with_damage()
            .expect("transparent scroll frame should render");

        assert!(damage.is_none(), "transparent damage must include the root");
        assert_eq!(
            buffer.get_pixel(10, 40),
            Some(crate::color::Color::rgb(220, 40, 40).to_bgra())
        );
        assert_eq!(
            buffer.get_pixel(0, 0),
            Some(crate::color::Color::TRANSPARENT.to_bgra())
        );
    }

    #[test]
    fn transparent_window_warm_scroll_preserves_unpainted_background() {
        let background = crate::color::Color::rgb(38, 48, 64);
        let scroll = ScrollView::new(
            Rectangle::new()
                .fill(crate::color::Color::TRANSPARENT)
                .frame(100.0, 600.0),
        )
        .content_size(100.0, 600.0)
        .wheel_sensitivity(1.0)
        .scrollbar_visibility(ScrollbarVisibility::Never)
        .frame(100.0, 100.0);
        let window = Window::new("Transparent scroll gap", scroll)
            .background_color(background)
            .shadow(true)
            .size(Size::new(120.0, 160.0));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(window.create_element());
        pipeline.layout_initial();
        pipeline
            .render_with_damage()
            .expect("initial transparent window frame should render");
        pipeline.reset_paint_test_counters();

        for _ in 0..4 {
            assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: -40,
                x: 60,
                y: 80,
                phase: WheelPhase::Moved,
                source: ScrollSource::Wheel,
            })));
            let (buffer, damage) = pipeline
                .render_with_damage()
                .expect("warm transparent scroll frame should render");
            assert!(damage_contains_rect(
                damage,
                Rect::from_xywh(60.0, 80.0, 1.0, 1.0)
            ));
            assert_eq!(buffer.get_pixel(60, 80), Some(background.to_bgra()));
        }
        assert!(pipeline.paint_test_counters().retained_composites > 0);
    }

    #[test]
    fn shadowed_window_reuses_backdrop_for_descendant_paint() {
        let color = State::new(
            crate::state::generate_state_id(),
            crate::color::Color::rgb(220, 40, 40),
        );
        let content = PaintOnlyComponentHarness {
            color: color.clone(),
        }
        .frame(160.0, 100.0);
        let window = Window::new("Video-like paint", content)
            .shadow(true)
            .size(Size::new(180.0, 140.0));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(window.create_element());
        pipeline.layout_initial();
        pipeline
            .render_with_damage()
            .expect("initial shadowed frame should render");

        assert_eq!(pipeline.paint_test_counters().window_backdrop_rebuilds, 1);
        let initial_backdrop = pipeline
            .window_backdrop_cache
            .as_ref()
            .expect("shadowed window should retain its backdrop")
            .buffer
            .clone();
        let shadow_alpha_at = |x, y| {
            crate::color::Color::from_bgra(
                initial_backdrop
                    .get_pixel(x, y)
                    .expect("shadow sample should be inside the cached surface"),
            )
            .a
        };
        let top_alpha = shadow_alpha_at(101, 4);
        let left_alpha = shadow_alpha_at(5, 76);
        let right_alpha = shadow_alpha_at(196, 76);
        let bottom_alpha = shadow_alpha_at(101, 156);
        assert!(top_alpha > 0.0);
        assert!(left_alpha > 0.0);
        assert!(right_alpha > 0.0);
        assert!(bottom_alpha > top_alpha);
        assert!(bottom_alpha < 1.0);

        pipeline.reset_paint_test_counters();
        color.set(crate::color::Color::rgb(40, 80, 220));
        let (_, damage) = pipeline
            .render_with_damage()
            .expect("descendant paint should render");
        let damage = damage
            .expect("retained window backdrop should preserve partial damage")
            .to_vec();

        assert_eq!(pipeline.paint_test_counters().window_backdrop_rebuilds, 0);
        assert!(damage.iter().all(|&(x, y, width, height)| {
            x >= 11 && y >= 6 && x.saturating_add(width) <= 191 && y.saturating_add(height) <= 146
        }));
        let retained_backdrop = &pipeline
            .window_backdrop_cache
            .as_ref()
            .expect("descendant paint should preserve the backdrop")
            .buffer;
        assert!(Arc::ptr_eq(&initial_backdrop, retained_backdrop));
    }

    #[test]
    fn flat_window_does_not_allocate_a_shadow_backdrop() {
        let window = Window::new("Flat", Text::new("Content"))
            .shadow(false)
            .size(Size::new(180.0, 140.0));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(window.create_element());
        pipeline.layout_initial();
        pipeline
            .render_with_damage()
            .expect("flat window should render");

        assert!(pipeline.window_backdrop_cache.is_none());
        assert_eq!(pipeline.paint_test_counters().window_backdrop_rebuilds, 0);
    }

    fn first_any_scrollbar_primitive_rect(pipeline: &RenderingPipeline) -> Option<Rect> {
        fn in_container(pipeline: &RenderingPipeline, container_id: LayerId) -> Option<Rect> {
            let container = pipeline.layer_store.container(container_id)?;
            for child in &container.children {
                match *child {
                    LayerChild::Primitive(primitive) => {
                        let LayerPrimitiveKind::RoundedRect { rect, .. } = primitive.kind;
                        return Some(rect);
                    }
                    LayerChild::Boundary { id, .. } => {
                        if let Some(rect) = in_container(pipeline, id) {
                            return Some(rect);
                        }
                    }
                    LayerChild::Chunk { .. } => {}
                }
            }
            None
        }
        in_container(pipeline, LayerId::Root)
    }

    fn damage_contains_rect(damage: Option<&[DamageRect]>, rect: Rect) -> bool {
        let Some(damage) = damage else {
            return true;
        };
        let target = RenderingPipeline::rect_to_damage(rect, 1000, 800, 600);
        damage.iter().any(|damage| {
            damage.0 <= target.0
                && damage.1 <= target.1
                && damage.0.saturating_add(damage.2) >= target.0.saturating_add(target.2)
                && damage.1.saturating_add(damage.3) >= target.1.saturating_add(target.3)
        })
    }

    #[test]
    fn warm_scroll_allocation_baseline_is_measured() {
        let mut pipeline = build_warm_scroll_pipeline();

        let snapshot = measure_allocations(|| {
            drive_warm_scroll_frame(&mut pipeline);
        });

        println!(
            "PHASE_6B_WARM_SCROLL_ALLOCS={} PHASE_6A_WARM_SCROLL_ALLOCS={} PHASE_5_WARM_SCROLL_ALLOCS={} PHASE_4_WARM_SCROLL_ALLOCS={} PHASE_3_WARM_SCROLL_ALLOCS={} PHASE_2_WARM_SCROLL_ALLOCS={} PHASE_1_WARM_SCROLL_ALLOCS={} allocated_bytes={} deallocations={}",
            snapshot.allocations,
            PHASE_6A_WARM_SCROLL_ALLOCS,
            PHASE_5_WARM_SCROLL_ALLOCS,
            PHASE_4_WARM_SCROLL_ALLOCS,
            PHASE_3_WARM_SCROLL_ALLOCS,
            PHASE_2_WARM_SCROLL_ALLOCS,
            PHASE_1_WARM_SCROLL_ALLOCS,
            snapshot.allocated_bytes,
            snapshot.deallocations
        );
        assert!(snapshot.allocations <= PHASE_1_WARM_SCROLL_ALLOCS);
        assert!(snapshot.allocations <= PHASE_2_WARM_SCROLL_ALLOCS);
        assert!(snapshot.allocations <= PHASE_3_WARM_SCROLL_ALLOCS);
        assert!(snapshot.allocations <= PHASE_4_WARM_SCROLL_ALLOCS);
        assert!(snapshot.allocations <= PHASE_5_WARM_SCROLL_ALLOCS);
        assert!(snapshot.allocations <= PHASE_6A_WARM_SCROLL_ALLOCS);
        assert_eq!(snapshot.allocations, PHASE_6B_WARM_SCROLL_ALLOCS);
        assert_eq!(snapshot.deallocations, 0);
        assert_eq!(snapshot.allocated_bytes, 0);
    }

    #[test]
    fn warm_scroll_without_dynamic_overlay_allocates_zero_after_warmup() {
        let mut pipeline = build_warm_scroll_pipeline();

        let snapshot = measure_allocations(|| {
            drive_warm_scroll_frame(&mut pipeline);
        });

        assert_eq!(snapshot.allocations, PHASE_6B_WARM_SCROLL_ALLOCS);
        assert_eq!(snapshot.deallocations, 0);
        assert_eq!(snapshot.allocated_bytes, 0);
    }

    #[test]
    fn default_scroll_view_warm_scroll_allocates_zero_after_warmup() {
        let mut pipeline =
            build_warm_scroll_pipeline_with_visibility(ScrollbarVisibility::Automatic);

        let snapshot = measure_allocations(|| {
            drive_warm_scroll_frame(&mut pipeline);
        });

        assert_eq!(snapshot.allocations, PHASE_7_WARM_SCROLL_ALLOCS);
        assert_eq!(snapshot.deallocations, 0);
        assert_eq!(snapshot.allocated_bytes, 0);
    }

    #[test]
    fn warm_scroll_path_counter_baseline_is_measured() {
        let mut pipeline = build_warm_scroll_pipeline();
        pipeline.reset_paint_test_counters();

        drive_warm_scroll_frame(&mut pipeline);

        let counters = pipeline.paint_test_counters();
        assert_eq!(counters.paint_context_news, 0);
        assert_eq!(counters.walk_and_paint_calls, 0);
        assert_eq!(counters.boundary_rebuilds, 0);
        assert_eq!(counters.retained_composites, 1);
        assert_eq!(counters.retained_sync_visits, 0);
        assert_eq!(counters.localized_retained_syncs, 1);
        assert_eq!(counters.retained_sync_fallbacks, 0);
    }

    #[test]
    fn default_scroll_view_warm_scroll_skips_paint_context_and_walk() {
        let mut pipeline =
            build_warm_scroll_pipeline_with_visibility(ScrollbarVisibility::Automatic);
        pipeline.reset_paint_test_counters();

        drive_warm_scroll_frame(&mut pipeline);

        let counters = pipeline.paint_test_counters();
        assert_eq!(counters.paint_context_news, 0);
        assert_eq!(counters.walk_and_paint_calls, 0);
        assert_eq!(counters.boundary_rebuilds, 0);
        assert_eq!(counters.retained_composites, 1);
        assert_eq!(counters.retained_sync_visits, 0);
        assert_eq!(counters.localized_retained_syncs, 1);
        assert_eq!(counters.retained_sync_fallbacks, 0);
        assert_eq!(counters.retained_primitive_slot_syncs, 1);
        assert_eq!(counters.retained_primitive_scan_syncs, 0);
    }

    #[test]
    fn scrollbar_overlay_retained_composite_updates_position_on_scroll() {
        let mut pipeline = build_warm_scroll_pipeline_with_visibility(ScrollbarVisibility::Always);
        let before = first_any_scrollbar_primitive_rect(&pipeline)
            .expect("always-visible scrollbar should be retained");
        pipeline.reset_paint_test_counters();

        drive_warm_scroll_frame(&mut pipeline);

        let after = first_any_scrollbar_primitive_rect(&pipeline)
            .expect("scrollbar primitive should remain retained");
        assert!(after.origin.y > before.origin.y);
        let counters = pipeline.paint_test_counters();
        assert_eq!(counters.paint_context_news, 0);
        assert_eq!(counters.walk_and_paint_calls, 0);
        assert_eq!(counters.boundary_rebuilds, 0);
        assert_eq!(counters.retained_composites, 1);
        assert_eq!(counters.retained_sync_visits, 0);
        assert_eq!(counters.localized_retained_syncs, 1);
        assert_eq!(counters.retained_sync_fallbacks, 0);
        assert_eq!(counters.retained_primitive_slot_syncs, 1);
        assert_eq!(counters.retained_primitive_scan_syncs, 0);
    }

    #[test]
    fn scrollbar_overlay_damage_includes_old_and_new_thumb() {
        let mut pipeline = build_warm_scroll_pipeline_with_visibility(ScrollbarVisibility::Always);
        let old_thumb = first_any_scrollbar_primitive_rect(&pipeline)
            .expect("always-visible scrollbar should be retained");

        assert!(pipeline.handle_event(&warm_scroll_event()));
        let (_, damage) = pipeline
            .render_with_damage()
            .expect("scroll frame should render with damage");
        let damage = damage.map(|damage| damage.to_vec());
        let new_thumb = first_any_scrollbar_primitive_rect(&pipeline)
            .expect("scrollbar primitive should update");

        assert!(damage_contains_rect(damage.as_deref(), old_thumb));
        assert!(damage_contains_rect(damage.as_deref(), new_thumb));
    }

    #[test]
    fn overlay_chunk_order_is_after_child_layers() {
        let pipeline = build_warm_scroll_pipeline_with_visibility(ScrollbarVisibility::Always);
        let root = pipeline.layer_store.root();
        let boundary_index = root
            .children
            .iter()
            .position(|child| matches!(child, LayerChild::Boundary { .. }))
            .expect("root should contain retained child content");
        let primitive_index = root
            .children
            .iter()
            .position(|child| matches!(child, LayerChild::Primitive(_)))
            .expect("root should contain retained scrollbar overlay");

        assert!(primitive_index > boundary_index);
    }

    #[test]
    fn real_app_like_nested_default_and_always_scroll_stays_retained() {
        let inner = ScrollView::new(
            Rectangle::new()
                .fill(crate::color::Color::rgb(80, 140, 220))
                .frame(520.0, 360.0),
        )
        .both_axes()
        .content_size(520.0, 360.0)
        .scrollbar_visibility(ScrollbarVisibility::Always)
        .frame(320.0, 160.0);
        let mut pipeline = RenderingPipeline::new();
        let root = ScrollView::new(inner)
            .vertical()
            .content_size(320.0, 520.0)
            .frame(320.0, 180.0)
            .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();
        for _ in 0..4 {
            drive_warm_scroll_frame(&mut pipeline);
        }
        pipeline.reset_paint_test_counters();

        drive_warm_scroll_frame(&mut pipeline);

        let counters = pipeline.paint_test_counters();
        assert_eq!(counters.paint_context_news, 0);
        assert_eq!(counters.walk_and_paint_calls, 0);
        assert_eq!(counters.boundary_rebuilds, 0);
        assert_eq!(counters.retained_composites, 1);
        assert_eq!(counters.retained_sync_visits, 0);
        assert_eq!(counters.localized_retained_syncs, 1);
        assert_eq!(counters.retained_sync_fallbacks, 0);
    }

    #[test]
    fn nested_scroll_content_stays_clipped_when_outer_scrolls() {
        let red = crate::color::Color::rgb(255, 0, 0);
        let inner = ScrollView::new(Rectangle::new().fill(red).frame(200.0, 240.0))
            .content_size(200.0, 240.0)
            .scrollbar_visibility(ScrollbarVisibility::Always)
            .frame(200.0, 80.0);
        let content = crate::vstack! {
            inner,
            crate::views::Spacer::new().frame(200.0, 220.0),
            PaintExtensionProbe.frame(1.0, 1.0),
        }
        .spacing(0.0);
        let mut pipeline = RenderingPipeline::new();
        let root = ScrollView::new(content)
            .content_size(200.0, 300.0)
            .wheel_sensitivity(1.0)
            .scrollbar_visibility(ScrollbarVisibility::Always)
            .frame(200.0, 150.0)
            .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();

        let initial_outside = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(100, 90))
            .expect("initial pixel outside the inner viewport should exist");
        assert_ne!(initial_outside, red.to_bgra());

        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: -40,
            x: 100,
            y: 120,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        })));
        let after_outer_scroll = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(100, 50))
            .expect("pixel below the moved inner viewport should exist");
        assert_ne!(after_outer_scroll, red.to_bgra());
    }

    #[test]
    fn retained_composite_sync_does_not_walk_cached_scroll_content() {
        let mut pipeline = RenderingPipeline::new();
        let root = ScrollView::new(LazyVStack::new(1_000, 20.0, |index| {
            Text::new(format!("Item {index}"))
        }))
        .wheel_sensitivity(1.0)
        .scrollbar_visibility(ScrollbarVisibility::Always)
        .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();
        for _ in 0..4 {
            drive_warm_scroll_frame(&mut pipeline);
        }
        pipeline.reset_paint_test_counters();

        drive_warm_scroll_frame(&mut pipeline);

        let counters = pipeline.paint_test_counters();
        assert_eq!(counters.paint_context_news, 0);
        assert_eq!(counters.walk_and_paint_calls, 0);
        assert_eq!(counters.boundary_rebuilds, 0);
        assert_eq!(counters.retained_composites, 1);
        assert_eq!(counters.retained_sync_visits, 0);
        assert_eq!(counters.localized_retained_syncs, 1);
        assert_eq!(counters.retained_sync_fallbacks, 0);
    }

    #[test]
    fn navigation_page_switch_then_scroll_visibly_updates_without_hover_damage() {
        let even_color = crate::color::Color::rgb(20, 180, 80);
        let odd_color = crate::color::Color::rgb(40, 80, 220);
        let page = move || {
            ScrollView::new(LazyVStack::new(30, 30.0, move |index| {
                let color = if index % 2 == 0 {
                    even_color
                } else {
                    odd_color
                };
                Rectangle::new().fill(color).frame(600.0, 30.0)
            }))
            .content_size(600.0, 900.0)
            .wheel_sensitivity(1.0)
            .scrollbar_visibility(ScrollbarVisibility::Always)
        };
        let mut pipeline = RenderingPipeline::new();
        let root = NavigationView::new((
            NavigationLink::new("First", || Text::new("first page")),
            NavigationLink::new("Second", page),
        ))
        .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();

        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 10,
                y: 45,
                click_count: 1,
            }))
        );
        let before_scroll = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(220, 10))
            .expect("content pixel should be available after switch");
        assert_eq!(before_scroll, even_color.to_bgra());
        pipeline.reset_paint_test_counters();

        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: -40,
            x: 220,
            y: 10,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        })));
        let after_scroll = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(220, 10))
            .expect("content pixel should be available after scroll");
        assert_eq!(after_scroll, odd_color.to_bgra());
        assert_ne!(before_scroll, after_scroll);
    }

    #[test]
    fn window_component_replacement_relayouts_new_grid_content() {
        let show_grid = State::new(StateId::new(24_001), false);
        let page_state = show_grid.clone();
        let colors = [
            crate::color::Color::rgb(220, 40, 40),
            crate::color::Color::rgb(40, 180, 80),
            crate::color::Color::rgb(40, 80, 220),
            crate::color::Color::rgb(220, 180, 40),
            crate::color::Color::rgb(180, 40, 220),
            crate::color::Color::rgb(220, 100, 40),
            crate::color::Color::rgb(40, 180, 180),
            crate::color::Color::rgb(100, 40, 220),
            crate::color::Color::rgb(180, 180, 40),
            crate::color::Color::rgb(40, 120, 180),
        ];
        let window = Window::new(
            "Files",
            NavigationView::new((NavigationLink::new("Files", move || {
                if page_state.get() {
                    let items = State::new(StateId::new(24_002), (0usize..10).collect());
                    let selected = State::new(StateId::new(24_003), None);
                    Either::B(
                        GridView::new(items, selected, 5, 120.0, move |index, _, _| {
                            Rectangle::new()
                                .fill(colors[index])
                                .frame(f32::INFINITY, 100.0)
                        })
                        .spacing(10.0),
                    )
                } else {
                    Either::A(Text::new("Loading"))
                }
            }),))
            .sidebar_width(170.0),
        )
        .size(Size::new(960.0, 640.0));

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(window.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        show_grid.set(true);
        let root = pipeline
            .element_tree_mut()
            .root_mut()
            .expect("window root should exist");
        root.update(&window);

        let buffer = pipeline
            .render_with_damage()
            .map(|(buffer, _)| buffer)
            .expect("replacement grid should render");
        for (index, color) in colors.into_iter().enumerate() {
            let row = (index / 5) as u32;
            let column = (index % 5) as u32;
            let left = 170 + column * 160;
            let right = left + 150;
            let top = 32 + row * 120;
            let bottom = top + 100;
            assert!(
                (left..right.min(buffer.width())).any(|x| {
                    (top..bottom.min(buffer.height()))
                        .any(|y| buffer.get_pixel(x, y) == Some(color.to_bgra()))
                }),
                "replacement grid item {index} should be painted in row {row}, column {column}"
            );
        }
    }

    #[test]
    fn navigation_factory_like_page_switch_refreshes_retained_state_before_scroll() {
        let top_color = crate::color::Color::rgb(210, 40, 40);
        let inner_color = crate::color::Color::rgb(40, 80, 220);
        let page = move || {
            let inner_scroll = ScrollView::new(LazyVStack::new(30, 30.0, move |_| {
                Rectangle::new().fill(inner_color).frame(520.0, 30.0)
            }))
            .both_axes()
            .content_size(520.0, 900.0)
            .scrollbar_visibility(ScrollbarVisibility::Always)
            .frame(320.0, 160.0);
            let content = crate::vstack! {
                Rectangle::new().fill(top_color).frame(600.0, 30.0),
                inner_scroll,
                Rectangle::new().fill(top_color).frame(600.0, 30.0),
                Rectangle::new().fill(top_color).frame(600.0, 700.0),
            }
            .spacing(0.0);
            ScrollView::new(content)
                .vertical()
                .content_size(600.0, 920.0)
                .wheel_sensitivity(1.0)
        };
        let mut pipeline = RenderingPipeline::new();
        let root = NavigationView::new((
            NavigationLink::new("First", || Text::new("first page")),
            NavigationLink::new("Factory", page),
        ))
        .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();

        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 10,
                y: 45,
                click_count: 1,
            }))
        );
        pipeline.reset_paint_test_counters();
        let before_scroll = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(220, 10))
            .expect("content pixel should be available after switch");
        let switch_counters = pipeline.paint_test_counters();
        assert!(switch_counters.paint_context_news > 0);
        assert_eq!(switch_counters.retained_composites, 0);
        assert_eq!(before_scroll, top_color.to_bgra());
        pipeline.reset_paint_test_counters();

        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: -40,
            x: 220,
            y: 10,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        })));
        let after_scroll = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(220, 10))
            .expect("content pixel should be available after scroll");
        assert_ne!(before_scroll, after_scroll);
        let scroll_counters = pipeline.paint_test_counters();
        assert_eq!(scroll_counters.paint_context_news, 0);
        assert_eq!(scroll_counters.walk_and_paint_calls, 0);
        assert_eq!(scroll_counters.boundary_rebuilds, 0);
        assert_eq!(scroll_counters.retained_composites, 1);
    }

    #[test]
    fn navigation_widget_factory_like_invalid_retained_graph_repaints_on_scroll() {
        let top_color = crate::color::Color::rgb(210, 40, 40);
        let next_color = crate::color::Color::rgb(40, 80, 220);
        let inner_color = crate::color::Color::rgb(80, 160, 90);
        let overview = move || {
            let inner_scroll = ScrollView::new(
                crate::vstack! {
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                }
                .spacing(8.0)
                .padding(12.0),
            )
            .both_axes()
            .content_size(520.0, 360.0)
            .scrollbar_visibility(ScrollbarVisibility::Always)
            .frame(320.0, 160.0);
            let content = crate::vstack! {
                Rectangle::new().fill(top_color).frame(600.0, 30.0),
                inner_scroll,
                Rectangle::new().fill(next_color).frame(600.0, 30.0),
                Rectangle::new().fill(next_color).frame(600.0, 700.0),
            }
            .spacing(16.0)
            .padding(24.0);
            ScrollView::new(content).vertical().content_size(0.0, 760.0)
        };
        let controls = || {
            ScrollView::new(
                crate::vstack! {
                    Text::new("Controls").font_size(24.0),
                    Rectangle::new().fill(crate::color::Color::rgb(230, 230, 230)).frame(600.0, 320.0),
                }
                .spacing(16.0)
                .padding(24.0),
            )
            .vertical()
            .content_size(0.0, 360.0)
        };
        let inputs = || {
            ScrollView::new(
                crate::vstack! {
                    Text::new("Inputs").font_size(24.0),
                    Rectangle::new().fill(crate::color::Color::rgb(230, 230, 230)).frame(600.0, 520.0),
                }
                .spacing(16.0)
                .padding(24.0),
            )
            .vertical()
            .content_size(0.0, 560.0)
        };
        let display = move || {
            let both_scroll = ScrollView::new(
                crate::vstack! {
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                    Rectangle::new().fill(inner_color).frame(520.0, 30.0),
                }
                .spacing(8.0)
                .padding(12.0),
            )
            .both_axes()
            .content_size(520.0, 360.0)
            .scrollbar_visibility(ScrollbarVisibility::Always)
            .frame(320.0, 160.0);
            let x_scroll = ScrollView::new(
                crate::hstack! {
                    Rectangle::new().fill(inner_color).frame(120.0, 40.0),
                    Rectangle::new().fill(inner_color).frame(120.0, 40.0),
                    Rectangle::new().fill(inner_color).frame(120.0, 40.0),
                    Rectangle::new().fill(inner_color).frame(120.0, 40.0),
                }
                .spacing(8.0)
                .padding(8.0),
            )
            .horizontal()
            .content_size(560.0, 64.0)
            .frame(240.0, 64.0);
            let y_scroll = ScrollView::new(
                crate::vstack! {
                    Rectangle::new().fill(inner_color).frame(160.0, 24.0),
                    Rectangle::new().fill(inner_color).frame(160.0, 24.0),
                    Rectangle::new().fill(inner_color).frame(160.0, 24.0),
                    Rectangle::new().fill(inner_color).frame(160.0, 24.0),
                    Rectangle::new().fill(inner_color).frame(160.0, 24.0),
                    Rectangle::new().fill(inner_color).frame(160.0, 24.0),
                    Rectangle::new().fill(inner_color).frame(160.0, 24.0),
                    Rectangle::new().fill(inner_color).frame(160.0, 24.0),
                }
                .spacing(8.0)
                .padding(8.0),
            )
            .vertical()
            .content_size(160.0, 240.0)
            .scrollbar_visibility(ScrollbarVisibility::Always)
            .frame(160.0, 96.0);
            let content = crate::vstack! {
                Rectangle::new().fill(top_color).frame(600.0, 30.0),
                both_scroll,
                x_scroll,
                y_scroll,
                Rectangle::new().fill(next_color).frame(600.0, 700.0),
            }
            .spacing(16.0)
            .padding(24.0);
            ScrollView::new(content).vertical().content_size(0.0, 940.0)
        };
        let mut pipeline = RenderingPipeline::new();
        let root = crate::navigation! {
            NavigationLink::new("Overview", overview),
            NavigationLink::new("Controls", controls),
            NavigationLink::new("Inputs", inputs),
            NavigationLink::new("Display", display),
        }
        .sidebar_width(190.0)
        .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();

        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 10,
                y: 125,
                click_count: 1,
            }))
        );
        let before_scroll = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(220, 30))
            .expect("display page pixel should be available");
        assert_eq!(before_scroll, top_color.to_bgra());
        let invalidated_by = pipeline
            .element_tree
            .root()
            .map(|root| root.id())
            .expect("root should exist");
        pipeline
            .layer_store
            .invalidate_layer(LayerId::Root, invalidated_by);
        pipeline.reset_paint_test_counters();

        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: -240,
            x: 220,
            y: 30,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        })));
        let after_scroll = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(220, 30))
            .expect("display page pixel should be available after scroll");
        assert_ne!(before_scroll, after_scroll);
        let counters = pipeline.paint_test_counters();
        assert!(counters.paint_context_news > 0);
        assert_eq!(counters.retained_composites, 0);
    }

    #[test]
    fn allocation_counter_measurement_does_not_allocate_while_reading_counts() {
        reset_allocation_counts();
        let _ = allocation_snapshot();

        let snapshot = measure_allocations(|| {
            let _ = allocation_snapshot();
            let _ = allocation_snapshot();
        });

        assert_eq!(snapshot.allocations, 0);
        assert_eq!(snapshot.deallocations, 0);
        assert_eq!(snapshot.allocated_bytes, 0);
    }

    fn scroll_content_boundary_id(pipeline: &RenderingPipeline) -> ElementId {
        pipeline
            .layer_store
            .boundary_ids()
            .into_iter()
            .next()
            .expect("scroll content should be a retained repaint boundary")
    }

    fn nested_scroll_boundary_ids(pipeline: &RenderingPipeline) -> (ElementId, ElementId) {
        let mut outer = None;
        let mut inner = None;
        for id in pipeline.layer_store.boundary_ids() {
            let container = pipeline
                .layer_store
                .container(LayerId::Boundary(id))
                .expect("boundary id should resolve to a container");
            if container
                .children
                .iter()
                .any(|child| matches!(child, LayerChild::Boundary { .. }))
            {
                outer = Some(id);
            } else {
                inner = Some(id);
            }
        }
        (
            outer.expect("outer scroll content should be retained"),
            inner.expect("inner scroll content should be retained"),
        )
    }

    fn first_chunk_id(pipeline: &RenderingPipeline, boundary_id: ElementId) -> LayerId {
        pipeline
            .layer_store
            .container(LayerId::Boundary(boundary_id))
            .and_then(|container| {
                container.children.iter().find_map(|child| match child {
                    LayerChild::Chunk { id, .. } => Some(*id),
                    LayerChild::Boundary { .. } | LayerChild::Primitive(_) => None,
                })
            })
            .expect("retained boundary should contain a picture chunk")
    }

    fn first_child_id(pipeline: &RenderingPipeline, id: ElementId) -> ElementId {
        pipeline
            .element_tree()
            .find_element(id)
            .and_then(|element| element.children().first())
            .map(|child| child.id())
            .expect("element should have a child")
    }

    fn invalidate_for_test(pipeline: &mut RenderingPipeline, dirty_id: ElementId) {
        pipeline.dirty_scratch.clear_for_frame();
        pipeline.dirty_scratch.ids.push(dirty_id);
        pipeline.invalidate_repaint_boundary_caches();
    }

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
            delta_y: -900,
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

        let boundary_id = scroll_content_boundary_id(&pipeline);
        let layer_id = LayerId::Boundary(boundary_id);
        let chunk_id = first_chunk_id(&pipeline, boundary_id);
        let before = Arc::as_ptr(&pipeline.layer_store.chunk(chunk_id).unwrap().buffer);

        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: -40,
            x: 10,
            y: 10,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        })));
        pipeline.render_with_damage();

        assert!(pipeline.layer_store.container(layer_id).unwrap().valid);
        let after = Arc::as_ptr(&pipeline.layer_store.chunk(chunk_id).unwrap().buffer);
        assert_eq!(before, after);
    }

    #[test]
    fn boundary_rebuild_uses_pipeline_renderer_scratch() {
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
            .expect("scroll content should be a repaint boundary");
        let descendant_id = boundary
            .children()
            .first()
            .expect("boundary should have a child")
            .id();
        let first_capacity = pipeline
            .retained_boundary_scratch_capacity_for_test()
            .expect("paint renderer should exist after initial render");
        assert!(first_capacity.scaled_points > 0);
        assert!(first_capacity.crossings > 0);

        pipeline.pipeline_owner.mark_needs_paint(descendant_id);
        pipeline.render_with_damage();
        let second_capacity = pipeline
            .retained_boundary_scratch_capacity_for_test()
            .expect("paint renderer should be retained after rebuild");

        assert_eq!(first_capacity.scaled_points, second_capacity.scaled_points);
        assert_eq!(first_capacity.crossings, second_capacity.crossings);
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
            .expect("scroll content should be a repaint boundary");
        let descendant_id = boundary
            .children()
            .first()
            .expect("boundary should have a child")
            .id();
        let boundary_id = scroll_content_boundary_id(&pipeline);
        let layer_id = LayerId::Boundary(boundary_id);
        let chunk_id = first_chunk_id(&pipeline, boundary_id);
        let before = Arc::as_ptr(&pipeline.layer_store.chunk(chunk_id).unwrap().buffer);

        invalidate_for_test(&mut pipeline, descendant_id);
        let container = pipeline.layer_store.container(layer_id).unwrap();
        assert!(!container.valid);
        assert_eq!(container.invalidated_by, Some(descendant_id));

        pipeline.pipeline_owner.mark_needs_paint(descendant_id);
        pipeline.render_with_damage();
        let after = Arc::as_ptr(&pipeline.layer_store.chunk(chunk_id).unwrap().buffer);
        assert_eq!(before, after);
        assert!(pipeline.layer_store.container(layer_id).unwrap().valid);
    }

    #[test]
    fn stateful_leaf_inside_scroll_repaints_without_unrelated_input() {
        let is_on = State::new(crate::state::generate_state_id(), false);
        let mut pipeline = RenderingPipeline::new();
        let root = ScrollView::new(Toggle::new(is_on.clone()))
            .content_size(100.0, 100.0)
            .frame(100.0, 100.0)
            .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();

        let before = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(10, 15))
            .expect("initial toggle pixel should be available");

        is_on.set(true);
        assert!(pipeline.has_dirty(), "toggle State must schedule a frame");
        let after = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(10, 15))
            .expect("toggle-only update should render a frame");

        assert_ne!(before, after, "the scroll boundary must repaint the toggle");
    }

    #[test]
    fn launcher_style_keyboard_routing_focuses_and_edits_text_field() {
        let query = State::new(
            crate::state::generate_state_id(),
            alloc::string::String::new(),
        );
        let focus_requested = State::new(crate::state::generate_state_id(), false);
        let selection_moves = State::new(crate::state::generate_state_id(), 0usize);
        let harness = LauncherInputHarness {
            query: query.clone(),
            focus_requested: focus_requested.clone(),
            selection_moves: selection_moves.clone(),
        };
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(harness.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: 'a' })));
        pipeline
            .render_with_damage()
            .expect("the first character should focus and repaint the field");
        assert_eq!(query.get(), "a");
        assert!(focus_requested.get());
        assert!(pipeline.focused_text_input_state().is_some());

        assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: 'b' })));
        pipeline
            .render_with_damage()
            .expect("focused text entry should repaint");
        assert_eq!(query.get(), "ab");

        assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Left,
            modifiers: crate::event::KeyModifiers::default(),
        })));
        pipeline.render_with_damage();
        assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Backspace,
            modifiers: crate::event::KeyModifiers::default(),
        })));
        pipeline
            .render_with_damage()
            .expect("backspace should repaint the edited text");
        assert_eq!(query.get(), "b");

        assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Up,
            modifiers: crate::event::KeyModifiers::default(),
        })));
        pipeline.render_with_damage();
        assert_eq!(selection_moves.get(), 1);
    }

    #[test]
    fn stateful_leaf_on_lazily_selected_navigation_page_schedules_a_frame() {
        let is_on = State::new(crate::state::generate_state_id(), false);
        let controls_state = is_on.clone();
        let root = NavigationView::new((
            NavigationLink::new("Overview", || Text::new("overview")),
            NavigationLink::new("Controls", move || {
                ScrollView::new(crate::vstack! {
                    Text::new("Controls").font_size(24.0),
                    Toggle::new(controls_state.clone()),
                })
                .content_size(0.0, 360.0)
            }),
        ))
        .create_element();
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();

        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 10,
                y: 45,
                click_count: 1,
            }))
        );
        pipeline
            .render_with_damage()
            .expect("selecting the controls page should render");

        fn find_toggle_id(element: &dyn Element) -> Option<ElementId> {
            if element.render_object().is_some_and(|render_object| {
                render_object
                    .as_any()
                    .downcast_ref::<crate::views::ToggleRenderObject>()
                    .is_some()
            }) {
                return Some(element.id());
            }
            element
                .children()
                .iter()
                .find_map(|child| find_toggle_id(child.as_ref()))
        }

        let toggle_id = find_toggle_id(
            pipeline
                .element_tree()
                .root()
                .expect("navigation tree should have a root"),
        )
        .expect("controls page should contain a toggle");
        let toggle_path = pipeline
            .element_tree()
            .find_path_ids(toggle_id)
            .expect("toggle should remain in the element tree");
        let (toggle_element, toggle_origin) = pipeline
            .element_tree()
            .element_and_absolute_origin_for_path(&toggle_path)
            .expect("toggle origin should resolve");
        let toggle_size = toggle_element.bounds().size;
        let click_x = (toggle_origin.x + toggle_size.width * 0.5) as i32;
        let click_y = (toggle_origin.y + toggle_size.height * 0.5) as i32;

        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: click_x,
                y: click_y,
                click_count: 1,
            }))
        );
        assert!(is_on.get(), "clicking the visible toggle must update State");
        assert!(
            pipeline.has_dirty(),
            "a control mounted after navigation must retain its subscription"
        );
        pipeline
            .render_with_damage()
            .expect("the toggle-only update should present a frame");
    }

    #[test]
    fn nested_repaint_boundary_dirty_descendant_invalidates_inner_only() {
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

        let (outer_id, inner_id) = nested_scroll_boundary_ids(&pipeline);
        let dirty_id = first_child_id(&pipeline, inner_id);

        invalidate_for_test(&mut pipeline, dirty_id);

        let outer = pipeline
            .layer_store
            .container(LayerId::Boundary(outer_id))
            .expect("outer boundary should exist");
        let inner = pipeline
            .layer_store
            .container(LayerId::Boundary(inner_id))
            .expect("inner boundary should exist");
        assert!(outer.valid);
        assert_eq!(outer.invalidated_by, None);
        assert!(!inner.valid);
        assert_eq!(inner.invalidated_by, Some(dirty_id));
    }

    #[test]
    fn nested_repaint_boundary_inner_dirty_does_not_rebuild_outer() {
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

        let (outer_id, inner_id) = nested_scroll_boundary_ids(&pipeline);
        let dirty_id = first_child_id(&pipeline, inner_id);

        pipeline.reset_paint_test_counters();
        pipeline.pipeline_owner.mark_needs_paint(dirty_id);
        pipeline.render_with_damage();

        let counters = pipeline.paint_test_counters();
        assert_eq!(counters.boundary_rebuilds, 1);
        assert!(
            pipeline
                .layer_store
                .container(LayerId::Boundary(outer_id))
                .expect("outer boundary should still exist")
                .valid
        );
        assert!(
            pipeline
                .layer_store
                .container(LayerId::Boundary(inner_id))
                .expect("inner boundary should still exist")
                .valid
        );
    }

    #[test]
    fn dirty_root_invalidates_root_layer_only() {
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

        let root_id = pipeline.element_tree().root().unwrap().id();
        let boundary_id = scroll_content_boundary_id(&pipeline);

        invalidate_for_test(&mut pipeline, root_id);

        assert_eq!(pipeline.layer_store.root().invalidated_by, Some(root_id));
        assert!(
            pipeline
                .layer_store
                .container(LayerId::Boundary(boundary_id))
                .unwrap()
                .valid
        );
    }

    #[test]
    fn find_path_ids_into_reuses_capacity() {
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

        let boundary_id = scroll_content_boundary_id(&pipeline);
        let dirty_id = first_child_id(&pipeline, boundary_id);
        let mut path = Vec::new();
        assert!(
            pipeline
                .element_tree()
                .find_path_ids_into(dirty_id, &mut path)
        );
        let warmed_capacity = path.capacity();
        assert!(
            pipeline
                .element_tree()
                .find_path_ids_into(boundary_id, &mut path)
        );
        assert!(path.capacity() <= warmed_capacity);
        assert_eq!(
            pipeline
                .element_tree()
                .find_nearest_repaint_boundary_in_path(&path),
            Some(boundary_id)
        );
    }

    #[test]
    fn paint_dirty_rects_does_not_allocate_after_capacity_warmup() {
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

        let boundary_id = scroll_content_boundary_id(&pipeline);
        let dirty_id = first_child_id(&pipeline, boundary_id);
        pipeline.dirty_scratch.ids.clear();
        pipeline.dirty_scratch.ids.push(dirty_id);
        RenderingPipeline::paint_dirty_rects_into(
            &pipeline.element_tree,
            &pipeline.last_paint_bounds,
            &pipeline.dirty_scratch.ids,
            &[],
            &mut pipeline.dirty_scratch.path,
            &mut pipeline.dirty_scratch.rects,
        );

        let snapshot = measure_allocations(|| {
            RenderingPipeline::paint_dirty_rects_into(
                &pipeline.element_tree,
                &pipeline.last_paint_bounds,
                &pipeline.dirty_scratch.ids,
                &[],
                &mut pipeline.dirty_scratch.path,
                &mut pipeline.dirty_scratch.rects,
            );
        });

        assert_eq!(snapshot.allocations, 0);
        assert_eq!(snapshot.deallocations, 0);
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
        let boundary_id = scroll_content_boundary_id(&pipeline);
        assert!(
            pipeline
                .layer_store
                .container(LayerId::Boundary(boundary_id))
                .is_some()
        );
        assert_eq!(inside, Some(red.to_bgra()));
        assert_ne!(outside, Some(red.to_bgra()));
    }

    #[test]
    fn damage_bounds_respect_scroll_ancestor_clip() {
        let mut pipeline = RenderingPipeline::new();
        let root = crate::vstack! {
            Rectangle::new()
                .fill(crate::color::Color::rgb(40, 80, 180))
                .frame(200.0, 40.0),
            ScrollView::new(
                Rectangle::new()
                    .fill(crate::color::Color::rgb(220, 40, 40))
                    .frame(200.0, 300.0),
            )
            .content_size(200.0, 300.0)
            .frame(200.0, 150.0),
        }
        .spacing(0.0)
        .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline.render_with_damage();

        let boundary_id = scroll_content_boundary_id(&pipeline);
        let mut path = Vec::new();
        assert!(
            pipeline
                .element_tree
                .find_path_ids_into(boundary_id, &mut path)
        );
        let bounds =
            RenderingPipeline::element_paint_bounds_for_path(&pipeline.element_tree, &path)
                .expect("scroll content should intersect its viewport");

        assert_eq!(bounds, Rect::from_xywh(300.0, 40.0, 200.0, 150.0));
    }

    #[test]
    fn nested_repaint_boundary_rebuild_preserves_parent_and_child_layers() {
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

        let (outer_id, inner_id) = nested_scroll_boundary_ids(&pipeline);
        assert!(
            pipeline
                .layer_store
                .container(LayerId::Boundary(outer_id))
                .is_some()
        );
        assert!(
            pipeline
                .layer_store
                .container(LayerId::Boundary(inner_id))
                .is_some()
        );
    }

    #[test]
    fn nested_repaint_boundary_child_is_composited_through_parent_order() {
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

        let (outer_id, inner_id) = nested_scroll_boundary_ids(&pipeline);
        let outer = pipeline
            .layer_store
            .container(LayerId::Boundary(outer_id))
            .expect("outer boundary should be retained");
        assert!(matches!(
            outer.children.first(),
            Some(LayerChild::Chunk { .. })
        ));
        assert!(outer.children.iter().any(|child| matches!(
            child,
            LayerChild::Boundary { id, .. } if *id == LayerId::Boundary(inner_id)
        )));
        assert!(matches!(
            outer.children.last(),
            Some(LayerChild::Chunk { .. })
        ));
    }

    #[test]
    fn scroll_view_auto_boundary_retains_nested_boundaries() {
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

        let (outer_id, inner_id) = nested_scroll_boundary_ids(&pipeline);
        let outer = pipeline
            .layer_store
            .container(LayerId::Boundary(outer_id))
            .expect("outer boundary should be retained");
        let inner = pipeline
            .layer_store
            .container(LayerId::Boundary(inner_id))
            .expect("inner boundary should be retained");
        assert_eq!(outer.logical_size, Size::new(100.0, 100.0));
        assert_eq!(inner.logical_size, Size::new(100.0, 300.0));
    }
}
