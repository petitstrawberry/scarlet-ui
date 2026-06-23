//! RenderingPipeline - Integration of PipelineOwner, ElementTree, and Compositor
//!
//! RenderingPipeline is the main entry point for the rendering system.
//! It orchestrates all phases of the rendering pipeline.

#![allow(deprecated)]

use crate::buffer::Buffer;
use crate::color::Color;
use crate::compositor::DamageRect;
use crate::element::{Element, ElementId, ElementTree, LayoutConstraints, ScrollLayerInfo};
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
const PRESENT_DAMAGE_FULL_AREA_NUMERATOR: u64 = 3;
const PRESENT_DAMAGE_FULL_AREA_DENOMINATOR: u64 = 5;
const SCROLL_LAYER_TILE_SIZE: f32 = 512.0;
const MAX_SCROLL_LAYER_TILES: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ScrollTileKey {
    x: i32,
    y: i32,
}

struct ScrollTile {
    rect: Rect,
    buffer: Arc<Buffer>,
    last_used: u64,
}

struct ScrollLayerCache {
    scale_milli: u32,
    viewport_size: Size,
    content_size: Size,
    tile_size: f32,
    tiles: BTreeMap<ScrollTileKey, ScrollTile>,
}

impl ScrollLayerCache {
    fn new(info: ScrollLayerInfo, scale_milli: u32) -> Self {
        Self {
            scale_milli,
            viewport_size: info.viewport_size,
            content_size: info.content_size,
            tile_size: SCROLL_LAYER_TILE_SIZE,
            tiles: BTreeMap::new(),
        }
    }

    fn matches(&self, info: ScrollLayerInfo, scale_milli: u32) -> bool {
        self.scale_milli == scale_milli
            && self.viewport_size == info.viewport_size
            && self.content_size == info.content_size
            && (self.tile_size - SCROLL_LAYER_TILE_SIZE).abs() < 0.001
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PaintFrameStats {
    elements_visited: usize,
    elements_painted: usize,
    command_count: usize,
    scroll_layers: usize,
    scroll_tile_hits: usize,
    scroll_tile_misses: usize,
    scroll_tiles_invalidated: usize,
    scroll_tiles_evicted: usize,
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
    scroll_layers: BTreeMap<ElementId, ScrollLayerCache>,
    paint_frame_seq: u64,
    last_paint_stats: PaintFrameStats,
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
            scroll_layers: BTreeMap::new(),
            paint_frame_seq: 0,
            last_paint_stats: PaintFrameStats::default(),
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
        self.scroll_layers.clear();
        self.paint_frame_seq = 0;
        self.last_paint_stats = PaintFrameStats::default();
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
        self.scroll_layers.clear();
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
        self.scroll_layers.clear();

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

        let dirty_ids = self.pipeline_owner.last_paint_ids();
        let mut dirty_rects = if force_full {
            None
        } else {
            Some(self.paint_dirty_rects(dirty_ids))
        };

        let present_damage = match dirty_rects.as_mut() {
            Some(rects) if rects.is_empty() => Some(Vec::new()),
            Some(rects) => {
                Self::merge_overlapping_rects(rects);
                let damage = Self::present_damage_rects(rects, size, scale);
                if damage.is_none() {
                    dirty_rects = None;
                }
                damage
            }
            None => None,
        };

        self.paint_damage = present_damage;

        self.paint_frame_seq = self.paint_frame_seq.wrapping_add(1);

        let mut ctx = PaintContext::new();
        let damage_clip = dirty_rects.as_deref();
        let dirty_set: BTreeSet<ElementId> = dirty_ids.iter().copied().collect();
        let mut stats = PaintFrameStats::default();
        let any_painted = if let Some(root) = self.element_tree.root() {
            let mut walker = PaintWalker {
                scroll_layers: &mut self.scroll_layers,
                last_paint_bounds: &self.last_paint_bounds,
                dirty_set: &dirty_set,
                frame_seq: self.paint_frame_seq,
                scale_milli: self.scale_milli,
                stats: &mut stats,
            };
            let base_painted =
                walker.walk_and_paint(&mut ctx, root, Point::ZERO, damage_clip, false);
            let overlay_painted =
                Self::paint_select_overlays(&mut ctx, root, Point::ZERO, damage_clip);
            base_painted || overlay_painted
        } else {
            false
        };
        stats.command_count = ctx.commands().len();

        if crate::debug::is_enabled() {
            crate::logln!(
                "[Paint] visited={} painted={} commands={} scroll_layers={} tile_hits={} tile_misses={} invalidated={} evicted={}",
                stats.elements_visited,
                stats.elements_painted,
                stats.command_count,
                stats.scroll_layers,
                stats.scroll_tile_hits,
                stats.scroll_tile_misses,
                stats.scroll_tiles_invalidated,
                stats.scroll_tiles_evicted,
            );
        }
        self.last_paint_stats = stats;

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
        if damage_area.saturating_mul(PRESENT_DAMAGE_FULL_AREA_DENOMINATOR)
            >= window_area.saturating_mul(PRESENT_DAMAGE_FULL_AREA_NUMERATOR)
        {
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

struct PaintWalker<'a> {
    scroll_layers: &'a mut BTreeMap<ElementId, ScrollLayerCache>,
    last_paint_bounds: &'a BTreeMap<ElementId, Rect>,
    dirty_set: &'a BTreeSet<ElementId>,
    frame_seq: u64,
    scale_milli: u32,
    stats: &'a mut PaintFrameStats,
}

impl<'a> PaintWalker<'a> {
    fn walk_and_paint<'p>(
        &mut self,
        ctx: &mut PaintContext<'p>,
        element: &'p dyn Element,
        origin: Point,
        damage_rects: Option<&[Rect]>,
        ancestor_dirty: bool,
    ) -> bool {
        self.stats.elements_visited += 1;

        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );
        let paint_bounds = RenderingPipeline::element_paint_bounds(element, abs);
        let should_paint_self = damage_rects
            .map(|rects| RenderingPipeline::overlaps_any(paint_bounds, rects))
            .unwrap_or(true);
        let element_dirty = self.dirty_set.contains(&element.id());

        if let Some(info) = element
            .render_object()
            .and_then(|render_object| render_object.scroll_layer_info())
            && !element.children().is_empty()
        {
            if !should_paint_self {
                return false;
            }
            return self.paint_scroll_layer(ctx, element, abs, info, damage_rects, ancestor_dirty);
        }

        let mut painted =
            should_paint_self && RenderingPipeline::paint_element_self(ctx, element, abs);
        let clip = RenderingPipeline::clip_for_element(element, abs);

        if let Some((rect, radius)) = clip {
            ctx.push_rounded_clip(rect, radius);
        }

        for child in element.children() {
            if self.walk_and_paint(
                ctx,
                child.as_ref(),
                abs,
                damage_rects,
                ancestor_dirty || element_dirty,
            ) {
                painted = true;
            }
        }

        if RenderingPipeline::paint_element_overlay(ctx, element, abs) {
            painted = true;
        }

        if clip.is_some() {
            ctx.pop_clip();
        }

        if painted {
            self.stats.elements_painted += 1;
        }
        painted
    }

    fn paint_scroll_layer<'p>(
        &mut self,
        ctx: &mut PaintContext<'p>,
        element: &'p dyn Element,
        abs: Point,
        info: ScrollLayerInfo,
        damage_rects: Option<&[Rect]>,
        ancestor_dirty: bool,
    ) -> bool {
        self.stats.scroll_layers += 1;
        self.reset_scroll_layer_if_needed(element.id(), info);
        self.invalidate_scroll_layer_tiles(element, abs, info, ancestor_dirty);

        let mut painted = RenderingPipeline::paint_element_self(ctx, element, abs);
        let clip = RenderingPipeline::clip_for_element(element, abs);
        if let Some((rect, radius)) = clip {
            ctx.push_rounded_clip(rect, radius);
        }

        if self.paint_scroll_tiles(ctx, element, abs, info, damage_rects, ancestor_dirty) {
            painted = true;
        }

        if RenderingPipeline::paint_element_overlay(ctx, element, abs) {
            painted = true;
        }

        if clip.is_some() {
            ctx.pop_clip();
        }

        if painted {
            self.stats.elements_painted += 1;
        }
        painted
    }

    fn reset_scroll_layer_if_needed(&mut self, id: ElementId, info: ScrollLayerInfo) {
        let needs_reset = match self.scroll_layers.get(&id) {
            Some(cache) => !cache.matches(info, self.scale_milli),
            None => true,
        };
        if !needs_reset {
            return;
        }
        if let Some(cache) = self.scroll_layers.get(&id) {
            self.stats.scroll_tiles_invalidated += cache.tiles.len();
        }
        self.scroll_layers
            .insert(id, ScrollLayerCache::new(info, self.scale_milli));
    }

    fn invalidate_scroll_layer_tiles(
        &mut self,
        element: &dyn Element,
        abs: Point,
        info: ScrollLayerInfo,
        ancestor_dirty: bool,
    ) {
        if ancestor_dirty {
            let Some(cache) = self.scroll_layers.get_mut(&element.id()) else {
                return;
            };
            self.stats.scroll_tiles_invalidated += cache.tiles.len();
            cache.tiles.clear();
            return;
        }

        let mut dirty_content_rects = Vec::new();
        for child in element.children() {
            self.collect_dirty_content_rects(
                child.as_ref(),
                abs,
                abs,
                info.offset,
                &mut dirty_content_rects,
            );
        }
        if dirty_content_rects.is_empty() {
            return;
        }

        let Some(cache) = self.scroll_layers.get_mut(&element.id()) else {
            return;
        };
        let remove_keys: Vec<ScrollTileKey> = cache
            .tiles
            .iter()
            .filter_map(|(key, tile)| {
                dirty_content_rects
                    .iter()
                    .any(|dirty| tile.rect.overlaps(dirty))
                    .then_some(*key)
            })
            .collect();
        self.stats.scroll_tiles_invalidated += remove_keys.len();
        for key in remove_keys {
            cache.tiles.remove(&key);
        }
    }

    fn collect_dirty_content_rects(
        &self,
        element: &dyn Element,
        origin: Point,
        scroll_abs: Point,
        scroll_offset: Point,
        rects: &mut Vec<Rect>,
    ) {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );

        if self.dirty_set.contains(&element.id()) {
            let current = RenderingPipeline::element_paint_bounds(element, abs);
            rects.push(Self::window_rect_to_content(
                current,
                scroll_abs,
                scroll_offset,
            ));
            if let Some(old) = self.last_paint_bounds.get(&element.id()) {
                rects.push(Self::window_rect_to_content(
                    *old,
                    scroll_abs,
                    scroll_offset,
                ));
            }
        }

        for child in element.children() {
            self.collect_dirty_content_rects(child.as_ref(), abs, scroll_abs, scroll_offset, rects);
        }
    }

    fn window_rect_to_content(rect: Rect, scroll_abs: Point, scroll_offset: Point) -> Rect {
        Rect::from_xywh(
            rect.origin.x - scroll_abs.x + scroll_offset.x,
            rect.origin.y - scroll_abs.y + scroll_offset.y,
            rect.size.width,
            rect.size.height,
        )
    }

    fn paint_scroll_tiles<'p>(
        &mut self,
        ctx: &mut PaintContext<'p>,
        element: &'p dyn Element,
        abs: Point,
        info: ScrollLayerInfo,
        damage_rects: Option<&[Rect]>,
        ancestor_dirty: bool,
    ) -> bool {
        let Some(visible_content) = Self::visible_content_rect(info) else {
            return false;
        };

        let mut painted = false;
        for key in Self::visible_tile_keys(visible_content) {
            let Some(tile_rect) = Self::tile_rect(key, info.content_size) else {
                continue;
            };
            let Some(intersection) = Self::intersect_rects(tile_rect, visible_content) else {
                continue;
            };
            let dst = Rect::from_xywh(
                abs.x + intersection.origin.x - info.offset.x,
                abs.y + intersection.origin.y - info.offset.y,
                intersection.size.width,
                intersection.size.height,
            );
            if damage_rects.is_some_and(|rects| !RenderingPipeline::overlaps_any(dst, rects)) {
                continue;
            }

            let buffer = if let Some(buffer) = self.cached_scroll_tile(element.id(), key) {
                self.stats.scroll_tile_hits += 1;
                buffer
            } else {
                self.stats.scroll_tile_misses += 1;
                let rendered = self.render_scroll_tile(element, info, tile_rect, ancestor_dirty);
                self.insert_scroll_tile(element.id(), key, tile_rect, rendered.clone());
                rendered
            };

            let src = Rect::from_xywh(
                intersection.origin.x - tile_rect.origin.x,
                intersection.origin.y - tile_rect.origin.y,
                intersection.size.width,
                intersection.size.height,
            );
            ctx.draw_buffer_rect_shared(dst, src, buffer, 1.0);
            painted = true;
        }
        painted
    }

    fn visible_content_rect(info: ScrollLayerInfo) -> Option<Rect> {
        let width = info
            .viewport_size
            .width
            .min((info.content_size.width - info.offset.x).max(0.0));
        let height = info
            .viewport_size
            .height
            .min((info.content_size.height - info.offset.y).max(0.0));
        (width > 0.0 && height > 0.0).then(|| {
            Rect::from_xywh(
                info.offset.x.max(0.0),
                info.offset.y.max(0.0),
                width,
                height,
            )
        })
    }

    fn visible_tile_keys(rect: Rect) -> Vec<ScrollTileKey> {
        let start_x = libm::floorf(rect.left() / SCROLL_LAYER_TILE_SIZE) as i32;
        let start_y = libm::floorf(rect.top() / SCROLL_LAYER_TILE_SIZE) as i32;
        let end_x = libm::floorf((rect.right() - 0.001) / SCROLL_LAYER_TILE_SIZE) as i32;
        let end_y = libm::floorf((rect.bottom() - 0.001) / SCROLL_LAYER_TILE_SIZE) as i32;
        let mut keys = Vec::new();
        for y in start_y..=end_y {
            for x in start_x..=end_x {
                keys.push(ScrollTileKey { x, y });
            }
        }
        keys
    }

    fn tile_rect(key: ScrollTileKey, content_size: Size) -> Option<Rect> {
        let x = key.x as f32 * SCROLL_LAYER_TILE_SIZE;
        let y = key.y as f32 * SCROLL_LAYER_TILE_SIZE;
        let width = (content_size.width - x).min(SCROLL_LAYER_TILE_SIZE);
        let height = (content_size.height - y).min(SCROLL_LAYER_TILE_SIZE);
        (width > 0.0 && height > 0.0).then(|| Rect::from_xywh(x, y, width, height))
    }

    fn intersect_rects(a: Rect, b: Rect) -> Option<Rect> {
        let left = a.left().max(b.left());
        let top = a.top().max(b.top());
        let right = a.right().min(b.right());
        let bottom = a.bottom().min(b.bottom());
        (right > left && bottom > top)
            .then(|| Rect::from_xywh(left, top, right - left, bottom - top))
    }

    fn cached_scroll_tile(&mut self, id: ElementId, key: ScrollTileKey) -> Option<Arc<Buffer>> {
        let cache = self.scroll_layers.get_mut(&id)?;
        let tile = cache.tiles.get_mut(&key)?;
        tile.last_used = self.frame_seq;
        Some(tile.buffer.clone())
    }

    fn insert_scroll_tile(
        &mut self,
        id: ElementId,
        key: ScrollTileKey,
        rect: Rect,
        buffer: Arc<Buffer>,
    ) {
        let cache = self
            .scroll_layers
            .entry(id)
            .or_insert_with(|| ScrollLayerCache {
                scale_milli: self.scale_milli,
                viewport_size: Size::ZERO,
                content_size: Size::ZERO,
                tile_size: SCROLL_LAYER_TILE_SIZE,
                tiles: BTreeMap::new(),
            });
        cache.tiles.insert(
            key,
            ScrollTile {
                rect,
                buffer,
                last_used: self.frame_seq,
            },
        );
        self.evict_scroll_tiles(id);
    }

    fn evict_scroll_tiles(&mut self, id: ElementId) {
        let Some(cache) = self.scroll_layers.get_mut(&id) else {
            return;
        };
        while cache.tiles.len() > MAX_SCROLL_LAYER_TILES {
            let Some(remove_key) = cache
                .tiles
                .iter()
                .min_by_key(|(_, tile)| tile.last_used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            cache.tiles.remove(&remove_key);
            self.stats.scroll_tiles_evicted += 1;
        }
    }

    fn render_scroll_tile(
        &mut self,
        element: &dyn Element,
        info: ScrollLayerInfo,
        tile_rect: Rect,
        ancestor_dirty: bool,
    ) -> Arc<Buffer> {
        let mut tile_ctx = PaintContext::new();
        let tile_damage = [Rect::from_xywh(
            0.0,
            0.0,
            tile_rect.size.width,
            tile_rect.size.height,
        )];
        let child_origin = Point::new(
            info.offset.x - tile_rect.origin.x,
            info.offset.y - tile_rect.origin.y,
        );

        for child in element.children() {
            self.walk_and_paint(
                &mut tile_ctx,
                child.as_ref(),
                child_origin,
                Some(&tile_damage),
                ancestor_dirty,
            );
        }
        for child in element.children() {
            RenderingPipeline::paint_select_overlays(
                &mut tile_ctx,
                child.as_ref(),
                child_origin,
                Some(&tile_damage),
            );
        }

        let mut renderer =
            CpuPaintRenderer::new(tile_rect.size, self.scale_milli, Color::TRANSPARENT);
        renderer.execute(&tile_ctx);
        Arc::new(renderer.into_buffer())
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
impl RenderingPipeline {
    fn last_paint_stats(&self) -> PaintFrameStats {
        self.last_paint_stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, MouseEvent, ScrollSource, WheelPhase};
    use crate::view::View;
    use crate::views::{LazyVStack, ScrollView, Text};

    fn scroll_pipeline() -> RenderingPipeline {
        let mut pipeline = RenderingPipeline::new();
        let root = ScrollView::new(Text::new("content"))
            .content_size(800.0, 2_000.0)
            .wheel_sensitivity(1.0)
            .create_element();
        pipeline.set_root(root);
        pipeline.layout_initial();
        pipeline
    }

    #[test]
    fn scroll_view_reuses_cached_tiles_when_only_offset_changes() {
        let mut pipeline = scroll_pipeline();

        pipeline.render_with_damage();
        let first = pipeline.last_paint_stats();
        assert!(first.scroll_layers > 0);
        assert!(first.scroll_tile_misses > 0);

        assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: 20,
            x: 10,
            y: 10,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        })));
        pipeline.render_with_damage();
        let scrolled = pipeline.last_paint_stats();

        assert!(scrolled.scroll_tile_hits > 0);
        assert_eq!(scrolled.scroll_tile_misses, 0);
    }

    #[test]
    fn scroll_view_invalidates_tiles_for_dirty_descendants() {
        let mut pipeline = scroll_pipeline();
        pipeline.render_with_damage();

        let child_id = pipeline
            .element_tree()
            .root()
            .and_then(|root| root.children().first())
            .map(|child| child.id())
            .expect("scroll view should have a child");
        pipeline.pipeline_owner_mut().mark_needs_paint(child_id);
        pipeline.render_with_damage();
        let stats = pipeline.last_paint_stats();

        assert!(stats.scroll_tiles_invalidated > 0);
        assert!(stats.scroll_tile_misses > 0);
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
            .and_then(|lazy| lazy.children().first())
            .map(|item| item.position().y)
            .expect("lazy child should materialize visible items");
        assert!(first_materialized_y > 0.0);
    }
}
