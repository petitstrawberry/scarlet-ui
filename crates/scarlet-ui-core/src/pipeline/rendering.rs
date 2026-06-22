//! RenderingPipeline - Integration of PipelineOwner, ElementTree, and Compositor
//!
//! RenderingPipeline is the main entry point for the rendering system.
//! It orchestrates all phases of the rendering pipeline.

#![allow(deprecated)]

use crate::buffer::Buffer;
use crate::compositor::DamageRect;
use crate::element::{Element, ElementId, ElementTree, LayoutConstraints, ScrollPaintState};
use crate::event::EventDispatcher;
use crate::geometry::{Point, Rect, Size};
use crate::pipeline::{PipelineId, PipelineOwner};
use crate::renderer::{CpuPaintRenderer, CpuRenderer, FrameSize, PaintContext};
use crate::views::WindowInfo;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

const MAX_PRESENT_DAMAGE_RECTS: usize = 4;
const PRESENT_DAMAGE_FULL_AREA_NUMERATOR: u64 = 3;
const PRESENT_DAMAGE_FULL_AREA_DENOMINATOR: u64 = 5;

#[derive(Clone, Debug)]
struct ScrollPaintSnapshot {
    viewport: Rect,
    offset_x_physical: i32,
    offset_y_physical: i32,
    overlay_rects: Vec<Rect>,
}

#[derive(Clone, Debug)]
struct ScrollShift {
    id: ElementId,
    physical_rect: DamageRect,
    dx_physical: i32,
    dy_physical: i32,
    damage_rects: Vec<Rect>,
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
    last_scroll_snapshots: BTreeMap<ElementId, ScrollPaintSnapshot>,
    paint_damage: Option<Vec<DamageRect>>,
    paint_needs_full: bool,
    paint_background_color: Option<crate::color::Color>,
    paint_enabled: bool,
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
            last_scroll_snapshots: BTreeMap::new(),
            paint_damage: None,
            paint_needs_full: true,
            paint_background_color: None,
            paint_enabled: true,
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
        self.last_scroll_snapshots.clear();
        self.paint_damage = None;
        self.paint_needs_full = true;
        self.paint_background_color = None;
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
        self.last_scroll_snapshots.clear();
        self.paint_damage = None;
        self.paint_needs_full = true;
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
        self.last_scroll_snapshots.clear();
        self.paint_damage = None;
        self.paint_needs_full = true;

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
        let current_scroll_snapshots = self.current_scroll_snapshots();
        let scroll_shifts = if force_full {
            Vec::new()
        } else {
            self.scroll_shifts(dirty_ids, &current_scroll_snapshots)
        };
        let mut dirty_rects = if force_full {
            None
        } else {
            Some(self.paint_dirty_rects(dirty_ids, &scroll_shifts))
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

        let mut ctx = PaintContext::new();
        let damage_clip = dirty_rects.as_deref();
        let any_painted = if let Some(root) = self.element_tree.root() {
            let base_painted = Self::walk_and_paint(&mut ctx, root, Point::ZERO, damage_clip);
            let overlay_painted =
                Self::paint_select_overlays(&mut ctx, root, Point::ZERO, damage_clip);
            base_painted || overlay_painted
        } else {
            false
        };

        if force_full || any_painted {
            let pr = self.paint_renderer.as_mut().unwrap();
            pr.set_background_color(background_color);
            if damage_clip.is_some() {
                for shift in scroll_shifts.iter() {
                    pr.shift_physical_rect(
                        shift.physical_rect,
                        shift.dx_physical,
                        shift.dy_physical,
                    );
                }
            }
            pr.execute_with_damage(&ctx, damage_clip);
        }

        self.paint_needs_full = false;
        self.paint_background_color = Some(background_color);
        self.last_paint_bounds.clear();
        if let Some(root) = self.element_tree.root() {
            Self::collect_paint_bounds(root, Point::ZERO, &mut self.last_paint_bounds);
        }
        self.last_scroll_snapshots = current_scroll_snapshots;

        let pr = self.paint_renderer.as_ref().unwrap();
        Some(pr.buffer())
    }

    fn walk_and_paint<'a>(
        ctx: &mut PaintContext<'a>,
        element: &'a dyn Element,
        origin: Point,
        damage_rects: Option<&[Rect]>,
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

        if let Some((rect, radius)) = clip {
            ctx.push_rounded_clip(rect, radius);
        }

        for child in element.children() {
            if Self::walk_and_paint(ctx, child.as_ref(), abs, damage_rects) {
                painted = true;
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

    fn paint_dirty_rects(
        &self,
        dirty_ids: &[ElementId],
        scroll_shifts: &[ScrollShift],
    ) -> Vec<Rect> {
        if dirty_ids.is_empty() {
            return Vec::new();
        }

        let Some(root) = self.element_tree.root() else {
            return Vec::new();
        };

        let dirty_set: BTreeSet<ElementId> = dirty_ids.iter().copied().collect();
        let scroll_shift_map: BTreeMap<ElementId, &[Rect]> = scroll_shifts
            .iter()
            .map(|shift| (shift.id, shift.damage_rects.as_slice()))
            .collect();
        let mut rects = Vec::new();
        self.collect_dirty_rects(root, Point::ZERO, &dirty_set, &scroll_shift_map, &mut rects);
        rects
    }

    fn collect_dirty_rects(
        &self,
        element: &dyn Element,
        origin: Point,
        dirty_ids: &BTreeSet<ElementId>,
        scroll_shift_map: &BTreeMap<ElementId, &[Rect]>,
        rects: &mut Vec<Rect>,
    ) {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );

        if dirty_ids.contains(&element.id()) {
            if let Some(shift_rects) = scroll_shift_map.get(&element.id()) {
                rects.extend_from_slice(shift_rects);
            } else {
                rects.push(Self::element_paint_bounds(element, abs));
                if let Some(old_bounds) = self.last_paint_bounds.get(&element.id()) {
                    rects.push(*old_bounds);
                }
            }
        }

        for child in element.children() {
            self.collect_dirty_rects(child.as_ref(), abs, dirty_ids, scroll_shift_map, rects);
        }
    }

    fn current_scroll_snapshots(&self) -> BTreeMap<ElementId, ScrollPaintSnapshot> {
        let mut snapshots = BTreeMap::new();
        if let Some(root) = self.element_tree.root() {
            self.collect_scroll_snapshots(root, Point::ZERO, &mut snapshots);
        }
        snapshots
    }

    fn collect_scroll_snapshots(
        &self,
        element: &dyn Element,
        origin: Point,
        snapshots: &mut BTreeMap<ElementId, ScrollPaintSnapshot>,
    ) {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );

        if let Some(state) = element
            .render_object()
            .and_then(|render_object| render_object.scroll_paint_state())
        {
            snapshots.insert(element.id(), self.scroll_snapshot(abs, state));
        }

        for child in element.children() {
            self.collect_scroll_snapshots(child.as_ref(), abs, snapshots);
        }
    }

    fn scroll_snapshot(&self, abs: Point, state: ScrollPaintState) -> ScrollPaintSnapshot {
        let scale = self.scale_milli.max(1) as f32 / 1000.0;
        let overlay_rects = state
            .overlay_rects
            .into_iter()
            .map(|rect| {
                Rect::from_xywh(
                    abs.x + rect.origin.x,
                    abs.y + rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                )
            })
            .collect();
        ScrollPaintSnapshot {
            viewport: Rect::new(abs, state.viewport_size),
            offset_x_physical: libm::floorf(state.offset.x * scale) as i32,
            offset_y_physical: libm::floorf(state.offset.y * scale) as i32,
            overlay_rects,
        }
    }

    fn scroll_shifts(
        &self,
        dirty_ids: &[ElementId],
        current: &BTreeMap<ElementId, ScrollPaintSnapshot>,
    ) -> Vec<ScrollShift> {
        let dirty_set: BTreeSet<ElementId> = dirty_ids.iter().copied().collect();
        current
            .iter()
            .filter_map(|(id, snapshot)| {
                if !dirty_set.contains(id) {
                    return None;
                }
                let previous = self.last_scroll_snapshots.get(id)?;
                self.scroll_shift(*id, previous, snapshot)
            })
            .collect()
    }

    fn scroll_shift(
        &self,
        id: ElementId,
        previous: &ScrollPaintSnapshot,
        current: &ScrollPaintSnapshot,
    ) -> Option<ScrollShift> {
        if previous.viewport.size != current.viewport.size {
            return None;
        }

        let dx_physical = previous
            .offset_x_physical
            .saturating_sub(current.offset_x_physical);
        let dy_physical = previous
            .offset_y_physical
            .saturating_sub(current.offset_y_physical);
        if dx_physical == 0 && dy_physical == 0 {
            return None;
        }

        let physical_rect = Self::rect_to_damage(
            current.viewport,
            self.scale_milli,
            Self::scale_len(self.window_size.width as u32, self.scale_milli),
            Self::scale_len(self.window_size.height as u32, self.scale_milli),
        );
        if physical_rect.2 == 0 || physical_rect.3 == 0 {
            return None;
        }
        if dx_physical.unsigned_abs() >= physical_rect.2
            || dy_physical.unsigned_abs() >= physical_rect.3
        {
            return None;
        }

        let mut damage_rects = Self::scroll_exposed_rects(
            current.viewport,
            dx_physical,
            dy_physical,
            self.scale_milli,
        );
        let dx_logical = Self::physical_len_to_logical(dx_physical, self.scale_milli);
        let dy_logical = Self::physical_len_to_logical(dy_physical, self.scale_milli);
        for rect in previous.overlay_rects.iter().copied() {
            damage_rects.push(rect);
            damage_rects.push(Rect::from_xywh(
                rect.origin.x + dx_logical,
                rect.origin.y + dy_logical,
                rect.size.width,
                rect.size.height,
            ));
        }
        damage_rects.extend(current.overlay_rects.iter().copied());

        Some(ScrollShift {
            id,
            physical_rect,
            dx_physical,
            dy_physical,
            damage_rects,
        })
    }

    fn scroll_exposed_rects(
        viewport: Rect,
        dx_physical: i32,
        dy_physical: i32,
        scale_milli: u32,
    ) -> Vec<Rect> {
        let mut rects = Vec::new();
        let dx = Self::physical_len_to_logical(dx_physical, scale_milli).abs();
        let dy = Self::physical_len_to_logical(dy_physical, scale_milli).abs();

        if dx_physical > 0 {
            rects.push(Rect::from_xywh(
                viewport.left(),
                viewport.top(),
                dx,
                viewport.height(),
            ));
        } else if dx_physical < 0 {
            rects.push(Rect::from_xywh(
                viewport.right() - dx,
                viewport.top(),
                dx,
                viewport.height(),
            ));
        }

        if dy_physical > 0 {
            rects.push(Rect::from_xywh(
                viewport.left(),
                viewport.top(),
                viewport.width(),
                dy,
            ));
        } else if dy_physical < 0 {
            rects.push(Rect::from_xywh(
                viewport.left(),
                viewport.bottom() - dy,
                viewport.width(),
                dy,
            ));
        }

        rects
    }

    fn physical_len_to_logical(value: i32, scale_milli: u32) -> f32 {
        value as f32 * 1000.0 / scale_milli.max(1) as f32
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
