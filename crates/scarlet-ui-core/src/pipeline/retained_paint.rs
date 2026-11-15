//! Immutable paint recordings, separate from the CPU raster layer cache.
//! Topology and positions are reconciled before a frame is replayed. Records
//! never borrow mutable elements; removed virtual rows release their resources.
use super::rendering::RenderingPipeline;
use crate::{
    element::{Element, ElementId, ElementTree},
    geometry::{Point, Rect},
    renderer::{DisplayList, PaintContext},
};
use alloc::{sync::Arc, vec::Vec};
use hashbrown::{HashMap, HashSet};

struct Node {
    generation: u64,
    parent: Option<ElementId>,
    subtree_stamp: u64,
    // A picture is in local coordinates. Moving it changes its parent's
    // contents, but does not invalidate this node's own retained picture.
    content_stamp: u64,
    position: Point,
    bounds: Rect,
    // Local aggregate ink bounds, including descendant overflow and clips.
    subtree_bounds: Rect,
    clip: Option<(Rect, f32)>,
    opacity: f32,
    children: Vec<ElementId>,
    paint: Option<Arc<DisplayList>>,
    overlay: Option<Arc<DisplayList>>,
    group: Option<(u64, Arc<DisplayList>)>,
    reads_backdrop: bool,
    deferred_paint: bool,
}

#[derive(Default)]
pub(super) struct RetainedPaint {
    root: Option<ElementId>,
    generation: u64,
    // Lookup order is irrelevant: replay always follows the explicit child order.
    nodes: HashMap<ElementId, Node>,
    deferred_paint: bool,
    paint_scratch: HashSet<ElementId>,
    self_scratch: HashSet<ElementId>,
    structural_scratch: HashSet<ElementId>,
    path_scratch: Vec<ElementId>,
    #[cfg(test)]
    pub(super) sync_visits: usize,
    #[cfg(test)]
    pub(super) indexed_syncs: usize,
    #[cfg(test)]
    pub(super) bounds_refreshes: usize,
}
impl RetainedPaint {
    pub fn clear(&mut self) {
        self.nodes = HashMap::new();
        self.root = None;
        self.deferred_paint = false;
        self.paint_scratch = HashSet::new();
        self.self_scratch = HashSet::new();
        self.structural_scratch = HashSet::new();
        self.path_scratch = Vec::new();
    }
    pub fn has_deferred_paint(&self) -> bool {
        self.deferred_paint
    }
    pub fn sync_tree(
        &mut self,
        root: &dyn Element,
        paint_ids: &[ElementId],
        self_ids: &[ElementId],
        structural_ids: &[ElementId],
        full: bool,
        defer_outside: Option<Rect>,
    ) {
        self.root = Some(root.id());
        self.deferred_paint = false;
        self.generation = self.generation.wrapping_add(1);
        // A membership query happens for every element. Index the dirty IDs
        // once instead of linearly scanning a dense dirty list at each node.
        let mut paint_index = core::mem::take(&mut self.paint_scratch);
        let mut self_index = core::mem::take(&mut self.self_scratch);
        let mut structural_index = core::mem::take(&mut self.structural_scratch);
        paint_index.clear();
        self_index.clear();
        structural_index.clear();
        paint_index.extend(paint_ids.iter().copied());
        self_index.extend(self_ids.iter().copied());
        structural_index.extend(structural_ids.iter().copied());
        #[cfg(test)]
        {
            self.indexed_syncs += 1;
        }
        self.sync(
            root,
            &paint_index,
            &self_index,
            &structural_index,
            full,
            false,
            None,
            Point::ZERO,
            defer_outside,
        );
        self.nodes
            .retain(|_, node| node.generation == self.generation);
        // Virtualization and root replacement must release a formerly large
        // table. Keep some slack to avoid reallocating during ordinary scrolling.
        if self.nodes.capacity() > self.nodes.len().saturating_mul(4).max(256) {
            self.nodes.shrink_to(self.nodes.len().saturating_mul(2));
        }
        self.paint_scratch = paint_index;
        self.self_scratch = self_index;
        self.structural_scratch = structural_index;
    }

    fn path_into(&self, id: ElementId, path: &mut Vec<ElementId>) -> bool {
        path.clear();
        let mut current = Some(id);
        while let Some(id) = current {
            let Some(node) = self.nodes.get(&id) else {
                return false;
            };
            path.push(id);
            if path.len() > self.nodes.len() {
                return false;
            }
            current = node.parent;
        }
        path.reverse();
        path.first().copied() == self.root
    }

    /// Paint-only leaf changes cannot move descendants or remove nodes. The
    /// caller must exclude layout, topology, composite and full invalidations.
    /// Validate every cached route before changing any recording; otherwise the
    /// ordinary tree reconciliation remains the authority.
    pub fn try_sync_leaf_paint(&mut self, tree: &ElementTree, ids: &[ElementId]) -> bool {
        if self.deferred_paint || tree.root().map(|r| r.id()) != self.root {
            return false;
        }
        let mut path = core::mem::take(&mut self.path_scratch);
        let valid = ids.iter().all(|id| {
            if !self.path_into(*id, &mut path) {
                return false;
            }
            let Some((element, _)) = tree.element_and_absolute_origin_for_path(&path) else {
                return false;
            };
            #[cfg(test)]
            {
                self.sync_visits += path.len();
            }
            let node = &self.nodes[id];
            node.children.is_empty()
                && element.children().is_empty()
                && node.position == element.position()
                && node.reads_backdrop
                    == element.render_object().is_some_and(|r| r.reads_backdrop())
        });
        if valid {
            self.generation = self.generation.wrapping_add(1);
            for id in ids {
                self.path_into(*id, &mut path);
                let (element, _) = tree.element_and_absolute_origin_for_path(&path).unwrap();
                let node = self.nodes.get_mut(id).unwrap();
                let bounds = RenderingPipeline::element_paint_bounds(element, Point::ZERO);
                let clip = RenderingPipeline::clip_for_element(element, Point::ZERO);
                let opacity = element.render_object().map_or(1., |r| r.subtree_opacity());
                let mut bounds_changed = node.bounds != bounds;
                let mut changed = bounds_changed || node.clip != clip || node.opacity != opacity;
                node.bounds = bounds;
                node.clip = clip;
                node.opacity = opacity;
                let mut paint = PaintContext::new();
                RenderingPipeline::paint_element_self(&mut paint, element, Point::ZERO);
                let same = node
                    .paint
                    .as_ref()
                    .map_or_else(|| paint.is_empty(), |old| paint.same_recording(old));
                if !same {
                    node.paint = (!paint.is_empty()).then(|| paint.into_display_list());
                    changed = true;
                }
                let mut overlay = PaintContext::new();
                RenderingPipeline::paint_element_overlay(&mut overlay, element, Point::ZERO);
                let same = node
                    .overlay
                    .as_ref()
                    .map_or_else(|| overlay.is_empty(), |old| overlay.same_recording(old));
                if !same {
                    node.overlay = (!overlay.is_empty()).then(|| overlay.into_display_list());
                    changed = true;
                }
                if changed {
                    // Refresh enclosing retained pictures bottom-up. Unchanged
                    // sibling records remain shared, including nested groups.
                    for ancestor in path.iter().rev() {
                        if bounds_changed {
                            bounds_changed = self.refresh_bounds(*ancestor);
                        }
                        let node = self.nodes.get_mut(ancestor).unwrap();
                        node.content_stamp = self.generation;
                        node.subtree_stamp = self.generation;
                        if node.group.is_some() {
                            self.cache_static_subtrees(*ancestor);
                        }
                    }
                }
            }
        }
        self.path_scratch = path;
        valid
    }
    fn sync(
        &mut self,
        element: &dyn Element,
        paint_ids: &HashSet<ElementId>,
        self_ids: &HashSet<ElementId>,
        structural_ids: &HashSet<ElementId>,
        inherited: bool,
        group: bool,
        parent: Option<ElementId>,
        origin: Point,
        defer_outside: Option<Rect>,
    ) -> (u64, bool) {
        #[cfg(test)]
        {
            self.sync_visits += 1;
        }
        let id = element.id();
        let subtree_dirty = inherited
            || (paint_ids.contains(&id)
                && !self_ids.contains(&id)
                && !structural_ids.contains(&id));
        let own_dirty = subtree_dirty
            || (paint_ids.contains(&id) && !structural_ids.contains(&id))
            || !self.nodes.contains_key(&id);
        let node = self.nodes.entry(id).or_insert_with(|| Node {
            generation: 0,
            parent,
            subtree_stamp: 0,
            content_stamp: 0,
            position: Point::ZERO,
            bounds: Rect::default(),
            subtree_bounds: Rect::default(),
            clip: None,
            opacity: 1.0,
            children: Vec::new(),
            paint: None,
            overlay: None,
            group: None,
            reads_backdrop: false,
            deferred_paint: false,
        });
        let own_dirty = own_dirty || node.deferred_paint;
        let position = element.position();
        let abs = Point::new(origin.x + position.x, origin.y + position.y);
        let bounds = RenderingPipeline::element_paint_bounds(element, Point::ZERO);
        let clip = RenderingPipeline::clip_for_element(element, Point::ZERO);
        let opacity = element.render_object().map_or(1.0, |r| r.subtree_opacity());
        let moved = node.position != position;
        let children_changed = !node
            .children
            .iter()
            .copied()
            .eq(element.children().iter().map(|child| child.id()));
        let changed = node.bounds != bounds
            || node.clip != clip
            || node.opacity != opacity
            || children_changed;
        let old_paint = node.paint.as_ref().map_or(0, |list| list.identity());
        let old_overlay = node.overlay.as_ref().map_or(0, |list| list.identity());
        node.generation = self.generation;
        node.parent = parent;
        node.position = position;
        node.bounds = bounds;
        node.clip = clip;
        node.opacity = opacity;
        if children_changed {
            node.children.clear();
            node.children
                .extend(element.children().iter().map(|child| child.id()));
        }
        // Only explicitly safe objects can defer. Retained groups must stay
        // complete because a parent can replay their offscreen source pixels.
        let defer_outside = if group || node.group.is_some() {
            None
        } else {
            defer_outside
        };
        let defer = own_dirty
            && !inherited
            && defer_outside.is_some_and(|viewport| {
                let absolute_bounds = Rect::new(
                    Point::new(abs.x + bounds.left(), abs.y + bounds.top()),
                    bounds.size,
                );
                intersect(absolute_bounds, viewport).is_none()
                    && element
                        .render_object()
                        .is_some_and(|r| r.can_defer_offscreen_paint())
                    && node
                        .paint
                        .as_ref()
                        .is_none_or(|list| list.is_self_contained() && !list.has_extensions())
                    && node
                        .overlay
                        .as_ref()
                        .is_none_or(|list| list.is_self_contained() && !list.has_extensions())
            });
        node.deferred_paint = defer;
        self.deferred_paint |= defer;
        if own_dirty && !defer {
            let mut context = PaintContext::new();
            RenderingPipeline::paint_element_self(&mut context, element, Point::ZERO);
            if node
                .paint
                .as_ref()
                .is_none_or(|old| !context.same_recording(old))
            {
                node.paint = (!context.is_empty()).then(|| context.into_display_list());
            }
        }
        // Overlays include scrollbar thumbs and can change on a composite-only frame.
        if (own_dirty && !defer) || structural_ids.contains(&id) {
            let mut context = PaintContext::new();
            RenderingPipeline::paint_element_overlay(&mut context, element, Point::ZERO);
            if node
                .overlay
                .as_ref()
                .is_none_or(|old| !context.same_recording(old))
            {
                node.overlay = (!context.is_empty()).then(|| context.into_display_list());
            }
        }
        if changed
            || old_paint != node.paint.as_ref().map_or(0, |list| list.identity())
            || old_overlay != node.overlay.as_ref().map_or(0, |list| list.identity())
        {
            node.content_stamp = self.generation;
        }
        if moved {
            node.subtree_stamp = self.generation;
        }
        // Carry child summaries back up the traversal. Looking each child up
        // again in the node map adds several tree searches per element.
        let mut stamp = node.content_stamp;
        let mut reads_backdrop = element.render_object().is_some_and(|r| r.reads_backdrop());
        let group_children = element.retains_virtual_items()
            || element
                .render_object()
                .is_some_and(|r| r.owns_child_viewport());
        for child in element.children() {
            let (child_stamp, child_reads_backdrop) = self.sync(
                child.as_ref(),
                paint_ids,
                self_ids,
                structural_ids,
                subtree_dirty,
                group_children,
                Some(id),
                abs,
                defer_outside,
            );
            stamp = stamp.max(child_stamp);
            reads_backdrop |= child_reads_backdrop;
        }
        let node = self.nodes.get_mut(&id).unwrap();
        node.content_stamp = stamp;
        node.subtree_stamp = node.subtree_stamp.max(stamp);
        node.reads_backdrop = reads_backdrop;
        let subtree_stamp = node.subtree_stamp;
        let refresh_group = group || node.group.is_some();
        // Children have already refreshed their local summaries. No group
        // rebuild needs to recursively recompute unchanged descendant bounds.
        self.refresh_bounds(id);
        // Backdrop-dependent groups can install independently retained groups
        // below the viewport's immediate child. Refresh those existing groups
        // bottom-up too, before an ancestor records references to their lists.
        if refresh_group {
            self.cache_static_subtrees(id);
        }
        (subtree_stamp, reads_backdrop)
    }
    fn cache_static_subtrees(&mut self, id: ElementId) {
        self.refresh_group(id);
        if self.nodes[&id].reads_backdrop
            && self.nodes[&id]
                .group
                .as_ref()
                .is_some_and(|(_, list)| list.raster_cache_bounds().is_none())
        {
            self.nodes.get_mut(&id).unwrap().group = None;
            let children = self.nodes[&id].children.clone();
            for child in children {
                self.cache_static_subtrees(child);
            }
        }
    }
    fn refresh_group(&mut self, id: ElementId) {
        let node = &self.nodes[&id];
        let stamp = node.content_stamp;
        if node.group.as_ref().is_none_or(|(old, _)| *old != stamp) {
            let position = node.position;
            let mut context = PaintContext::new();
            self.replay_node(
                id,
                &mut context,
                Point::new(-position.x, -position.y),
                None,
                None,
                false,
            );
            let list = if context.commands().len() > 1 {
                if let Some((_, previous)) = &self.nodes[&id].group {
                    context.into_cached_layer_revision(self.group_bounds(id), previous)
                } else {
                    context.into_cached_layer(self.group_bounds(id))
                }
            } else {
                context.into_display_list()
            };
            self.nodes.get_mut(&id).unwrap().group = Some((stamp, list));
        }
    }
    fn group_bounds(&self, id: ElementId) -> Rect {
        self.nodes[&id].subtree_bounds
    }
    fn refresh_bounds(&mut self, id: ElementId) -> bool {
        #[cfg(test)]
        {
            self.bounds_refreshes += 1;
        }
        let node = &self.nodes[&id];
        let mut bounds = node.bounds;
        for child in &node.children {
            let child_node = &self.nodes[child];
            let local = child_node.subtree_bounds;
            let mut child_bounds = Rect::new(
                Point::new(
                    local.origin.x + child_node.position.x,
                    local.origin.y + child_node.position.y,
                ),
                local.size,
            );
            if let Some((clip, _)) = node.clip {
                let Some(clipped) = intersect(child_bounds, clip) else {
                    continue;
                };
                child_bounds = clipped;
            }
            let left = bounds.left().min(child_bounds.left());
            let top = bounds.top().min(child_bounds.top());
            bounds = Rect::from_xywh(
                left,
                top,
                bounds.right().max(child_bounds.right()) - left,
                bounds.bottom().max(child_bounds.bottom()) - top,
            );
        }
        let changed = node.subtree_bounds != bounds;
        self.nodes.get_mut(&id).unwrap().subtree_bounds = bounds;
        changed
    }
    /// Sync only the moving subtree. Geometry recordings are reused, even when
    /// virtualization mounts more children. This never visits sibling panes.
    pub fn sync_composite(&mut self, element: &dyn Element) {
        self.generation = self.generation.wrapping_add(1);
        let Some(node) = self.nodes.get_mut(&element.id()) else {
            return;
        };
        let moved = node.position != element.position();
        let bounds = RenderingPipeline::element_paint_bounds(element, Point::ZERO);
        let clip = RenderingPipeline::clip_for_element(element, Point::ZERO);
        let opacity = element.render_object().map_or(1.0, |r| r.subtree_opacity());
        let mut changed = node.bounds != bounds || node.clip != clip || node.opacity != opacity;
        node.position = element.position();
        node.bounds = bounds;
        node.clip = clip;
        node.opacity = opacity;
        let mut context = PaintContext::new();
        RenderingPipeline::paint_element_overlay(&mut context, element, Point::ZERO);
        let same_overlay = node
            .overlay
            .as_ref()
            .map_or_else(|| context.is_empty(), |old| context.same_recording(old));
        if !same_overlay {
            node.overlay = (!context.is_empty()).then(|| context.into_display_list());
            changed = true;
        }
        for child in element.children() {
            if let Some(node) = self.nodes.get_mut(&child.id()) {
                if node.position != child.position() {
                    node.position = child.position();
                    node.subtree_stamp = self.generation;
                    changed = true;
                }
            }
        }
        // Ending a gesture can notify composition without changing pixels.
        if !moved && !changed {
            return;
        }
        if changed {
            self.nodes.get_mut(&element.id()).unwrap().content_stamp = self.generation;
        }
        // Update enclosing pictures without walking unrelated panes. The
        // moving node's own picture changes only when its local contents do.
        let mut current = Some(element.id());
        let mut bounds_changed = true;
        while let Some(id) = current {
            if bounds_changed {
                bounds_changed = self.refresh_bounds(id) || (id == element.id() && moved);
            }
            let Some(node) = self.nodes.get_mut(&id) else {
                break;
            };
            node.subtree_stamp = self.generation;
            if id != element.id() {
                node.content_stamp = self.generation;
            }
            current = node.parent;
            if node.group.is_some() {
                self.refresh_group(id);
            }
        }
    }

    /// Resolve the previous frame's bounds only for a dirty element. Scroll
    /// translations stay on ancestors; no absolute-bounds table is rebuilt for
    /// every descendant of an unchanged picture.
    pub fn bounds(&self, id: ElementId) -> Option<Rect> {
        let mut path = Vec::new();
        let mut current = Some(id);
        while let Some(id) = current {
            let node = self.nodes.get(&id)?;
            path.push(id);
            current = node.parent;
        }
        let mut origin = Point::ZERO;
        let mut clip = None;
        for ancestor in path.into_iter().rev() {
            let node = self.nodes.get(&ancestor)?;
            origin.x += node.position.x;
            origin.y += node.position.y;
            if ancestor == id {
                let bounds = Rect::new(
                    Point::new(origin.x + node.bounds.left(), origin.y + node.bounds.top()),
                    node.bounds.size,
                );
                return clip.map_or(Some(bounds), |clip| intersect(bounds, clip));
            }
            if let Some((local, _)) = node.clip {
                let bounds = Rect::new(
                    Point::new(origin.x + local.left(), origin.y + local.top()),
                    local.size,
                );
                clip = Some(clip.map_or(Some(bounds), |clip| intersect(bounds, clip))?);
            }
        }
        None
    }
    pub fn replay(&self, context: &mut PaintContext<'_>, damage: Option<&[Rect]>) {
        if let Some(root) = self.root {
            self.replay_node(root, context, Point::ZERO, None, damage, true);
        }
    }
    fn replay_node(
        &self,
        id: ElementId,
        context: &mut PaintContext<'_>,
        origin: Point,
        active_clip: Option<Rect>,
        damage: Option<&[Rect]>,
        use_groups: bool,
    ) {
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        let abs = Point::new(origin.x + node.position.x, origin.y + node.position.y);
        let bounds = Rect::new(
            Point::new(abs.x + node.bounds.origin.x, abs.y + node.bounds.origin.y),
            node.bounds.size,
        );
        if use_groups && context.subtree_opacity() == 1.0 && node.opacity == 1.0 {
            if let Some((_, list)) = &node.group {
                if active_clip.is_none_or(|clip| intersect(bounds, clip).is_some())
                    && damage.is_none_or(|rects| RenderingPipeline::overlaps_any(bounds, rects))
                {
                    context.draw_display_list(abs, Arc::clone(list));
                }
                return;
            }
        }
        let previous_opacity = context.push_subtree_opacity(node.opacity);
        if active_clip.is_none_or(|clip| intersect(bounds, clip).is_some())
            && damage.is_none_or(|rects| RenderingPipeline::overlaps_any(bounds, rects))
        {
            if let Some(list) = &node.paint {
                context.draw_display_list(abs, Arc::clone(list));
            }
        }
        // The node's own paint may extend outside its child clip (a shadow),
        // so only stop after emitting it. Children and overlays are both inside
        // this clip and cannot contribute when it misses an ancestor clip.
        if let (Some(active), Some((local, _))) = (active_clip, node.clip) {
            let clip = Rect::new(
                Point::new(abs.x + local.left(), abs.y + local.top()),
                local.size,
            );
            if intersect(active, clip).is_none()
                && node.overlay.as_ref().is_none_or(|list| {
                    list.is_self_contained() && !list.has_extensions() && !list.reads_backdrop()
                })
            {
                context.restore_subtree_opacity(previous_opacity);
                return;
            }
        }
        if let Some((clip, radius)) = node.clip {
            context.push_rounded_clip(
                Rect::new(
                    Point::new(abs.x + clip.origin.x, abs.y + clip.origin.y),
                    clip.size,
                ),
                radius,
            );
        }
        let next_clip = node.clip.map(|(clip, _)| {
            Rect::new(
                Point::new(abs.x + clip.origin.x, abs.y + clip.origin.y),
                clip.size,
            )
        });
        let next_clip = match (active_clip, next_clip) {
            (Some(a), Some(b)) => intersect(a, b),
            (a, b) => a.or(b),
        };
        if !(active_clip.is_some() && node.clip.is_some() && next_clip.is_none()) {
            for child in &node.children {
                // Suppress only the group being recorded, not independently
                // retained descendants such as nested scrolling content.
                self.replay_node(*child, context, abs, next_clip, damage, true);
            }
        }
        if let Some(list) = &node.overlay {
            context.draw_display_list(abs, Arc::clone(list));
        }
        if node.clip.is_some() {
            context.pop_clip();
        }
        context.restore_subtree_opacity(previous_opacity);
    }
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.origin.x.max(b.origin.x);
    let y = a.origin.y.max(b.origin.y);
    let right = (a.origin.x + a.size.width).min(b.origin.x + b.size.width);
    let bottom = (a.origin.y + a.size.height).min(b.origin.y + b.size.height);
    (right > x && bottom > y).then(|| Rect::from_xywh(x, y, right - x, bottom - y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        buffer::Buffer,
        event::{Event, MouseEvent, ScrollSource, WheelPhase},
        prelude::*,
        renderer::{BackendFrame, CpuPaintRenderer, PaintBackend},
    };
    use alloc::{boxed::Box, rc::Rc};
    use core::cell::RefCell;

    #[test]
    fn warm_leaf_sync_visits_only_its_path_without_allocating_dirty_indexes() {
        use crate::testing::alloc_counter::measure_allocations;
        let row = VStack::new((Spacer::new(), Spacer::new(), Spacer::new(), Spacer::new()));
        let mut tree = ElementTree::new();
        tree.set_root(
            VStack::new((Spacer::new(), row.clone(), row.clone(), row.clone(), row))
                .create_element(),
        );
        let root = tree.root().unwrap();
        let id = root.children()[0].id();
        let mut scene = RetainedPaint::default();
        scene.sync_tree(root, &[], &[], &[], true, None);
        assert!(scene.nodes.len() > 20);
        assert!(scene.try_sync_leaf_paint(&tree, &[id]));
        scene.sync_visits = 0;
        scene.indexed_syncs = 0;
        let allocations = measure_allocations(|| {
            for _ in 0..64 {
                assert!(scene.try_sync_leaf_paint(&tree, &[id]));
            }
        });
        assert_eq!(scene.sync_visits, 64 * 2);
        assert_eq!(scene.indexed_syncs, 0);
        assert_eq!(allocations.allocations, 0);
        assert_eq!(allocations.allocated_bytes, 0);
        // A subtree paint is deliberately handled by the full reconciler.
        assert!(!scene.try_sync_leaf_paint(&tree, &[root.id()]));
    }

    #[test]
    fn leaf_sync_refreshes_enclosing_groups_and_rejects_geometry_or_missing_routes() {
        fn rectangle(element: &dyn Element) -> Option<ElementId> {
            if element
                .render_object()
                .is_some_and(|r| r.as_any().is::<crate::views::RectangleRenderObject>())
            {
                return Some(element.id());
            }
            element
                .children()
                .iter()
                .find_map(|e| rectangle(e.as_ref()))
        }
        fn pixels(scene: &RetainedPaint) -> Vec<u8> {
            let mut ctx = PaintContext::new();
            scene.replay(&mut ctx, None);
            let mut renderer = CpuPaintRenderer::new(Size::new(40., 30.), 1000, Color::WHITE);
            renderer.execute_with_damage(&ctx, None);
            renderer.buffer().data().to_vec()
        }
        let mut tree = ElementTree::new();
        tree.set_root(
            ScrollView::new(VStack::new((
                Rectangle::new().fill(Color::RED).frame(30., 20.),
                Rectangle::new().fill(Color::GREEN).frame(30., 20.),
            )))
            .content_size(30., 40.)
            .frame(40., 30.)
            .create_element(),
        );
        tree.layout(crate::element::LayoutConstraints::tight(40., 30.));
        let id = rectangle(tree.root().unwrap()).unwrap();
        let mut scene = RetainedPaint::default();
        scene.sync_tree(tree.root().unwrap(), &[], &[], &[], true, None);
        assert!(scene.nodes.values().any(|n| n.group.is_some()));
        for color in [Color::BLUE, Color::CLEAR, Color::RED] {
            scene.bounds_refreshes = 0;
            tree.find_element_mut(id)
                .unwrap()
                .update(&Rectangle::new().fill(color));
            assert!(scene.try_sync_leaf_paint(&tree, &[id]));
            assert_eq!(
                scene.bounds_refreshes, 0,
                "paint-only revisions must reuse aggregate ink bounds"
            );
            let mut fresh = RetainedPaint::default();
            fresh.sync_tree(tree.root().unwrap(), &[], &[], &[], true, None);
            assert_eq!(pixels(&scene), pixels(&fresh));
        }
        tree.find_element_mut(id)
            .unwrap()
            .set_position(Point::new(3., 4.));
        assert!(!scene.try_sync_leaf_paint(&tree, &[id]));
        tree.clear_root();
        assert!(!scene.try_sync_leaf_paint(&tree, &[id]));
    }

    #[test]
    fn cached_ink_bounds_follow_paint_outsets_without_layout_and_child_motion() {
        use crate::element::{ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult};
        use crate::geometry::EdgeInsets;
        use core::any::Any;
        #[derive(Clone)]
        struct Ink {
            pad: f32,
            size: Size,
        }
        impl View for Ink {
            fn create_element(&self) -> Box<dyn Element> {
                Box::new(RenderElement::new(self.clone(), self.clone()))
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
        impl ElementRenderObject for Ink {
            fn layout(&mut self, _: LayoutConstraints) -> Size {
                self.size
            }
            fn size(&self) -> Size {
                self.size
            }
            fn render(&mut self) {}
            fn paint_outsets(&self) -> EdgeInsets {
                EdgeInsets::all(self.pad)
            }
            fn update_needs_layout(&self) -> bool {
                false
            }
            fn update(&mut self, view: &dyn View) -> UpdateResult {
                let Some(view) = view.as_any().downcast_ref::<Self>() else {
                    return UpdateResult::Replaced;
                };
                self.pad = view.pad;
                UpdateResult::Updated
            }
            fn paint<'a>(&'a self, ctx: &mut PaintContext<'a>, origin: Point) -> bool {
                ctx.fill_rect(
                    Rect::from_xywh(
                        origin.x - self.pad,
                        origin.y - self.pad,
                        self.size.width + self.pad * 2.,
                        self.size.height + self.pad * 2.,
                    ),
                    Color::BLUE,
                );
                true
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn Any {
                self
            }
        }
        let ink = |pad| Ink {
            pad,
            size: Size::new(20., 10.),
        };
        let mut tree = ElementTree::new();
        tree.set_root(
            VStack::new((ink(0.), Spacer::new().frame(20., 10.)))
                .spacing(0.)
                .create_element(),
        );
        tree.layout(LayoutConstraints::tight(20., 20.));
        let root = tree.root().unwrap().id();
        let id = tree.root().unwrap().children()[0].id();
        let mut scene = RetainedPaint::default();
        scene.sync_tree(tree.root().unwrap(), &[], &[], &[], true, None);
        let baseline = scene.group_bounds(root);
        for pad in [12., 3., 0.] {
            tree.find_element_mut(id).unwrap().update(&ink(pad));
            assert!(scene.try_sync_leaf_paint(&tree, &[id]));
            let mut fresh = RetainedPaint::default();
            fresh.sync_tree(tree.root().unwrap(), &[], &[], &[], true, None);
            assert_eq!(scene.group_bounds(root), fresh.group_bounds(root));
            assert_eq!(
                scene.group_bounds(id),
                Rect::from_xywh(-pad, -pad, 20. + pad * 2., 10. + pad * 2.)
            );
            if pad > 0. {
                assert!(scene.group_bounds(root).left() < baseline.left());
            }
        }
        tree.find_element_mut(id)
            .unwrap()
            .set_position(Point::new(35., 0.));
        scene.sync_composite(tree.root().unwrap());
        assert!(scene.group_bounds(root).right() >= 55.);
    }

    #[test]
    fn deferred_offscreen_surface_repaints_before_exposure_and_releases_on_clear() {
        use crate::element::LayoutConstraints;
        let view =
            |color| crate::views::Surface::section(Spacer::new().frame(30., 30.)).fill(color);
        let mut element = view(Color::RED).create_element();
        element.layout(LayoutConstraints::tight(30., 30.));
        element.set_position(Point::new(200., 10.));
        let id = element.id();
        let mut scene = RetainedPaint::default();
        scene.sync_tree(element.as_ref(), &[], &[], &[], true, None);
        let original = scene.nodes[&id].paint.as_ref().unwrap().identity();
        for color in [Color::BLUE, Color::GREEN] {
            element.update(&view(color));
            scene.sync_tree(
                element.as_ref(),
                &[id],
                &[id],
                &[],
                false,
                Some(Rect::from_xywh(0., 0., 100., 100.)),
            );
            assert!(scene.has_deferred_paint());
            assert_eq!(
                scene.nodes[&id].paint.as_ref().unwrap().identity(),
                original
            );
        }
        // Geometry/composite changes flush pending paint even without a dirty ID.
        element.set_position(Point::new(20., 10.));
        scene.sync_tree(element.as_ref(), &[], &[], &[], false, None);
        assert!(!scene.has_deferred_paint());
        assert_ne!(
            scene.nodes[&id].paint.as_ref().unwrap().identity(),
            original
        );
        let mut fresh = RetainedPaint::default();
        fresh.sync_tree(element.as_ref(), &[], &[], &[], true, None);
        let pixels = |scene: &RetainedPaint| {
            let mut paint = PaintContext::new();
            scene.replay(&mut paint, None);
            let mut renderer = CpuPaintRenderer::new(Size::new(100., 100.), 1000, Color::WHITE);
            renderer.execute_with_damage(&paint, None);
            renderer.buffer().data().to_vec()
        };
        assert_eq!(pixels(&scene), pixels(&fresh));
        element.set_position(Point::new(200., 10.));
        element.update(&view(Color::RED));
        scene.sync_tree(
            element.as_ref(),
            &[id],
            &[id],
            &[],
            false,
            Some(Rect::from_xywh(0., 0., 100., 100.)),
        );
        assert!(scene.has_deferred_paint());
        scene.clear();
        assert!(!scene.has_deferred_paint());
        assert!(scene.nodes.is_empty());
    }

    #[test]
    fn offscreen_surface_shadow_reaching_viewport_is_recorded_eagerly() {
        use crate::element::LayoutConstraints;
        let view = |color| {
            crate::views::Surface::section(Spacer::new().frame(30., 30.))
                .elevation(crate::views::ElevationRole::Floating)
                .fill(color)
        };
        let mut element = view(Color::RED).create_element();
        element.layout(LayoutConstraints::tight(30., 30.));
        element.set_position(Point::new(101., 10.));
        let id = element.id();
        let mut scene = RetainedPaint::default();
        scene.sync_tree(element.as_ref(), &[], &[], &[], true, None);
        assert!(scene.nodes[&id].bounds.left() < -1.);
        let old = scene.nodes[&id].paint.as_ref().unwrap().identity();
        element.update(&view(Color::GREEN));
        scene.sync_tree(
            element.as_ref(),
            &[id],
            &[id],
            &[],
            false,
            Some(Rect::from_xywh(0., 0., 100., 100.)),
        );
        assert!(!scene.has_deferred_paint());
        assert_ne!(scene.nodes[&id].paint.as_ref().unwrap().identity(), old);
    }

    #[test]
    fn disjoint_child_clip_keeps_outset_paint_without_child_or_overlay_commands() {
        let picture = |rect, color| {
            let mut context = PaintContext::new();
            context.fill_rect(rect, color);
            context.into_display_list()
        };
        let outset = picture(Rect::from_xywh(-10., 0., 20., 20.), Color::RED);
        let hidden = picture(Rect::from_xywh(-100., 0., 200., 20.), Color::BLUE);
        let node = |position, clip, paint, overlay, children| Node {
            opacity: 1.0,
            generation: 1,
            parent: None,
            subtree_stamp: 1,
            content_stamp: 1,
            position,
            bounds: Rect::from_xywh(-10., 0., 20., 20.),
            subtree_bounds: Rect::from_xywh(-10., 0., 20., 20.),
            clip,
            children,
            paint,
            overlay,
            group: None,
            reads_backdrop: false,
            deferred_paint: false,
        };
        let root = ElementId::new(1);
        let child = ElementId::new(2);
        let descendant = ElementId::new(3);
        let mut scene = RetainedPaint::default();
        scene.root = Some(root);
        scene.nodes.insert(
            root,
            node(
                Point::ZERO,
                Some((Rect::from_xywh(0., 0., 100., 100.), 0.)),
                None,
                None,
                vec![child],
            ),
        );
        scene.nodes.insert(
            child,
            node(
                Point::new(105., 10.),
                Some((Rect::from_xywh(0., 0., 20., 20.), 0.)),
                Some(outset.clone()),
                Some(hidden.clone()),
                vec![descendant],
            ),
        );
        scene.nodes.insert(
            descendant,
            node(Point::ZERO, None, Some(hidden), None, vec![]),
        );
        let mut context = PaintContext::new();
        scene.replay(&mut context, None);
        let lists: Vec<_> = context
            .commands()
            .iter()
            .filter_map(|command| match command {
                crate::renderer::PaintCommand::DrawDisplayList { list, .. } => {
                    Some(list.identity())
                }
                _ => None,
            })
            .collect();
        assert_eq!(lists, vec![outset.identity()]);
        assert_eq!(
            context.commands().len(),
            3,
            "only the ancestor clip and visible outset remain"
        );
        let mut actual = CpuPaintRenderer::new(Size::new(100., 100.), 1000, Color::WHITE);
        actual.execute_with_damage(&context, None);
        let mut expected_context = PaintContext::new();
        expected_context.fill_rect(Rect::from_xywh(95., 10., 5., 20.), Color::RED);
        let mut expected = CpuPaintRenderer::new(Size::new(100., 100.), 1000, Color::WHITE);
        expected.execute_with_damage(&expected_context, None);
        assert_eq!(actual.buffer().data(), expected.buffer().data());

        // State changes can affect later siblings even inside an empty clip.
        // Retain their original position in the command stream.
        let mut opacity = PaintContext::new();
        opacity.set_opacity(0.5);
        let opacity = opacity.into_display_list();
        scene.nodes.get_mut(&child).unwrap().overlay = Some(opacity.clone());
        let mut stateful = PaintContext::new();
        scene.replay(&mut stateful, None);
        assert!(stateful.commands().iter().any(|command| matches!(command,
            crate::renderer::PaintCommand::DrawDisplayList { list, .. } if list.identity() == opacity.identity())));
    }
    struct Capture {
        retained: bool,
        renderer: CpuPaintRenderer,
        pixels: Rc<RefCell<Buffer>>,
        lists: Rc<RefCell<Vec<u64>>>,
    }
    impl PaintBackend for Capture {
        fn supports_retained_display_lists(&self) -> bool {
            self.retained
        }
        fn resize(&mut self, size: Size, scale: u32) {
            self.renderer.resize(size, scale);
        }
        fn render<'a>(
            &'a mut self,
            context: &PaintContext<'_>,
            background: Color,
            damage: Option<&[Rect]>,
            _: Option<&[crate::compositor::DamageRect]>,
        ) -> crate::Result<BackendFrame<'a>> {
            self.lists.borrow_mut().clear();
            for command in context.commands() {
                if let crate::renderer::PaintCommand::DrawDisplayList { list, .. } = command {
                    self.lists.borrow_mut().push(list.identity());
                }
            }
            self.renderer.set_background_color(background);
            self.renderer.execute_with_damage(context, damage);
            *self.pixels.borrow_mut() = self.renderer.buffer().clone();
            Ok(BackendFrame::External)
        }
    }
    fn pipeline(
        items: State<Vec<usize>>,
        retained: bool,
    ) -> (
        RenderingPipeline,
        Rc<RefCell<Buffer>>,
        Rc<RefCell<Vec<u64>>>,
    ) {
        let pixels = Rc::new(RefCell::new(Buffer::new(Size::new(200., 100.))));
        let lists = Rc::new(RefCell::new(Vec::new()));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_paint_backend(Box::new(Capture {
            retained,
            renderer: CpuPaintRenderer::new(Size::new(200., 100.), 1000, Color::WHITE),
            pixels: pixels.clone(),
            lists: lists.clone(),
        }));
        pipeline.set_root(
            Window::new(
                "Retained",
                ListView::new(
                    items,
                    State::new(crate::generate_state_id(), None),
                    20.,
                    |_, id, _| {
                        Rectangle::new()
                            .fill(Color::rgb(id as u8, 20, 50))
                            .frame(200., 20.)
                    },
                )
                .item_key(|id| (*id).into())
                .frame(200., 100.),
            )
            .decorated(false)
            .size(Size::new(200., 100.))
            .create_element(),
        );
        pipeline.resize(Size::new(200., 100.));
        pipeline.layout_initial();
        pipeline.render_for_present().unwrap();
        (pipeline, pixels, lists)
    }
    fn wheel(pipeline: &mut RenderingPipeline, delta: i32) {
        pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            x: 20,
            y: 20,
            delta_x: 0,
            delta_y: delta,
            phase: WheelPhase::Moved,
            source: ScrollSource::Trackpad,
        }));
    }
    #[test]
    fn previously_grouped_descendant_refreshes_before_ancestor_replay() {
        let view = |color| {
            VStack::new((
                Rectangle::new().fill(color).frame(80., 20.),
                Rectangle::new().fill(Color::BLUE).frame(80., 20.),
            ))
        };
        let mut element = view(Color::RED).create_element();
        let constraints = crate::element::LayoutConstraints::tight(80., 40.);
        element.layout(constraints);
        let mut scene = RetainedPaint::default();
        scene.sync_tree(element.as_ref(), &[], &[], &[], true, None);
        // Backdrop fallback can install a group below the immediate child of
        // a viewport. Such a node has no viewport-owning parent on the next
        // sync, but its recording still needs to follow content changes.
        let grouped = element.children()[0].id();
        scene.cache_static_subtrees(grouped);
        let original = scene.nodes[&grouped].group.as_ref().unwrap().1.identity();
        element.update(&view(Color::GREEN));
        element.layout(constraints);
        scene.sync_tree(element.as_ref(), &[element.id()], &[], &[], false, None);
        let node = &scene.nodes[&grouped];
        let (stamp, list) = node.group.as_ref().unwrap();
        assert_eq!(*stamp, node.content_stamp);
        assert_ne!(list.identity(), original);
    }

    #[test]
    fn composite_and_virtualization_reuse_recordings_and_match_direct_pixels() {
        let items = State::new(crate::generate_state_id(), (0..100).collect());
        let (mut retained, pixels, lists) = pipeline(items.clone(), true);
        let (mut direct, expected, _) = pipeline(items, false);
        let original = lists.borrow().clone();
        for delta in [-4, -4, -160, -160, -160, 80] {
            wheel(&mut retained, delta);
            wheel(&mut direct, delta);
            retained.render_for_present().unwrap();
            direct.render_for_present().unwrap();
            assert_eq!(pixels.borrow().data(), expected.borrow().data());
            assert!(
                lists.borrow().iter().any(|id| original.contains(id)),
                "Clean retained rows should survive scrolling and materialization"
            );
        }
    }
    #[test]
    fn content_update_scroll_and_shrink_are_committed_together() {
        let items = State::new(crate::generate_state_id(), (0..100).collect());
        let (mut retained, pixels, _) = pipeline(items.clone(), true);
        let (mut direct, expected, _) = pipeline(items.clone(), false);
        wheel(&mut retained, -400);
        wheel(&mut direct, -400);
        retained.render_for_present().unwrap();
        direct.render_for_present().unwrap();
        items.set((200..210).chain(0..100).collect());
        wheel(&mut retained, -16);
        wheel(&mut direct, -16);
        retained.render_for_present().unwrap();
        direct.render_for_present().unwrap();
        assert_eq!(pixels.borrow().data(), expected.borrow().data());
        items.set(vec![250, 251]);
        retained.render_for_present().unwrap();
        direct.render_for_present().unwrap();
        assert_eq!(pixels.borrow().data(), expected.borrow().data());
    }
    #[test]
    fn virtual_items_release_removed_recordings_and_keep_scene_bounded() {
        let view = LazyVStack::new(50_000, 20., |_| {
            Rectangle::new()
                .fill(Color::rgb(80, 20, 50))
                .frame(200., 20.)
        });
        let mut element = view.create_element();
        element.set_viewport_hint(Rect::from_xywh(0., 0., 200., 100.));
        element.layout(crate::element::LayoutConstraints::tight(200., 100.));
        let mut scene = RetainedPaint::default();
        scene.sync_tree(element.as_ref(), &[], &[], &[], true, None);
        // Simulate a formerly large mounted tree, then verify that scrolling
        // with a small live set releases spare table storage as well as nodes.
        scene.nodes.reserve(8192);
        let weak = Arc::downgrade(
            scene
                .nodes
                .values()
                .find_map(|node| node.paint.as_ref())
                .unwrap(),
        );
        for y in [10_000., 20_000., 30_000., 40_000.] {
            element.set_viewport_hint(Rect::from_xywh(0., y, 200., 100.));
            scene.sync_tree(element.as_ref(), &[], &[], &[element.id()], false, None);
            assert!(
                scene.nodes.len() < 200,
                "the scene must retain mounted virtual rows, not the full model"
            );
        }
        assert!(
            weak.upgrade().is_none(),
            "removed row recordings must release their resources"
        );
        assert!(scene.nodes.capacity() <= scene.nodes.len().saturating_mul(4).max(256));
        scene.clear();
        assert_eq!(scene.nodes.capacity(), 0);
    }
    #[test]
    fn nested_scroller_refreshes_its_grouped_row() {
        let mut pipelines = Vec::new();
        for retained in [true, false] {
            let pixels = Rc::new(RefCell::new(Buffer::new(Size::new(200., 100.))));
            let mut pipeline = RenderingPipeline::new();
            pipeline.set_paint_backend(Box::new(Capture {
                retained,
                renderer: CpuPaintRenderer::new(Size::new(200., 100.), 1000, Color::WHITE),
                pixels: pixels.clone(),
                lists: Rc::new(RefCell::new(Vec::new())),
            }));
            let rows = LazyVStack::new(2, 100., |_| {
                ScrollView::new(
                    VStack::new((
                        Rectangle::new().fill(Color::RED).frame(200., 80.),
                        Rectangle::new().fill(Color::BLUE).frame(200., 80.),
                        Rectangle::new().fill(Color::GREEN).frame(200., 80.),
                        Rectangle::new().fill(Color::WHITE).frame(200., 80.),
                        Rectangle::new().fill(Color::BLACK).frame(200., 80.),
                    ))
                    .spacing(0.),
                )
                .frame(200., 100.)
            });
            pipeline.set_root(
                Window::new("Nested", ScrollView::new(rows).frame(200., 100.))
                    .decorated(false)
                    .size(Size::new(200., 100.))
                    .create_element(),
            );
            pipeline.resize(Size::new(200., 100.));
            pipeline.layout_initial();
            pipeline.render_for_present().unwrap();
            pipelines.push((pipeline, pixels));
        }
        for delta in [-4, -4, -80, 20] {
            for (pipeline, _) in &mut pipelines {
                wheel(pipeline, delta);
                pipeline.render_for_present().unwrap();
            }
            assert_eq!(
                pipelines[0].1.borrow().data(),
                pipelines[1].1.borrow().data()
            );
        }
    }
    #[test]
    fn changing_border_preserves_child_recordings_and_matches_fresh_pixels() {
        let size = Size::new(200., 100.);
        let view = |color, width| {
            Window::new(
                "Border",
                Spacer::new()
                    .frame(80., 80.)
                    .background(Color::BLUE)
                    .clip_radius(10.)
                    .border_rounded(color, width, 10.),
            )
            .decorated(false)
            .size(size)
        };
        let pixels = Rc::new(RefCell::new(Buffer::new(size)));
        let lists = Rc::new(RefCell::new(Vec::new()));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_paint_backend(Box::new(Capture {
            retained: true,
            renderer: CpuPaintRenderer::new(size, 1000, Color::WHITE),
            pixels: pixels.clone(),
            lists: lists.clone(),
        }));
        pipeline.set_root(view(Color::RED, 1.).create_element());
        pipeline.resize(size);
        pipeline.layout_initial();
        pipeline.render_for_present().unwrap();
        let old = lists.borrow().clone();
        for (color, width) in [(Color::GREEN, 3.), (Color::RED, 1.)] {
            let updated = view(color, width);
            pipeline
                .element_tree_mut()
                .root_mut()
                .unwrap()
                .update(&updated);
            pipeline.render_for_present().unwrap();
            assert_eq!(pipeline.pipeline_owner().last_self_paint_ids().len(), 1);
            let shared = old.iter().filter(|id| lists.borrow().contains(id)).count();
            assert_eq!(
                shared,
                old.len() - 1,
                "only the border overlay recording may change"
            );
            let fresh_pixels = Rc::new(RefCell::new(Buffer::new(size)));
            let mut fresh = RenderingPipeline::new();
            fresh.set_paint_backend(Box::new(Capture {
                retained: false,
                renderer: CpuPaintRenderer::new(size, 1000, Color::WHITE),
                pixels: fresh_pixels.clone(),
                lists: Rc::new(RefCell::new(Vec::new())),
            }));
            fresh.set_root(updated.create_element());
            fresh.resize(size);
            fresh.layout_initial();
            fresh.render_for_present().unwrap();
            assert_eq!(pixels.borrow().data(), fresh_pixels.borrow().data());
            fresh.teardown();
        }
        pipeline.teardown();
    }

    #[test]
    fn changing_surface_colors_preserves_child_recordings_and_matches_fresh_pixels() {
        let size = Size::new(200., 100.);
        let view = |color, _width: f32| {
            Window::new(
                "Border",
                crate::views::Surface::section(Spacer::new().frame(80., 80.))
                    .corner_radius(10.)
                    .fill(color)
                    .border_color(Color::BLUE),
            )
            .decorated(false)
            .size(size)
        };
        let pixels = Rc::new(RefCell::new(Buffer::new(size)));
        let lists = Rc::new(RefCell::new(Vec::new()));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_paint_backend(Box::new(Capture {
            retained: true,
            renderer: CpuPaintRenderer::new(size, 1000, Color::WHITE),
            pixels: pixels.clone(),
            lists: lists.clone(),
        }));
        pipeline.set_root(view(Color::RED, 1.).create_element());
        pipeline.resize(size);
        pipeline.layout_initial();
        pipeline.render_for_present().unwrap();
        let old = lists.borrow().clone();
        for (color, width) in [(Color::GREEN, 3.), (Color::RED, 1.)] {
            let updated = view(color, width);
            pipeline
                .element_tree_mut()
                .root_mut()
                .unwrap()
                .update(&updated);
            pipeline.render_for_present().unwrap();
            assert_eq!(pipeline.pipeline_owner().last_self_paint_ids().len(), 1);
            let shared = old.iter().filter(|id| lists.borrow().contains(id)).count();
            assert_eq!(
                shared,
                old.len() - 1,
                "only the surface recording may change"
            );
            let fresh_pixels = Rc::new(RefCell::new(Buffer::new(size)));
            let mut fresh = RenderingPipeline::new();
            fresh.set_paint_backend(Box::new(Capture {
                retained: false,
                renderer: CpuPaintRenderer::new(size, 1000, Color::WHITE),
                pixels: fresh_pixels.clone(),
                lists: Rc::new(RefCell::new(Vec::new())),
            }));
            fresh.set_root(updated.create_element());
            fresh.resize(size);
            fresh.layout_initial();
            fresh.render_for_present().unwrap();
            assert_eq!(pixels.borrow().data(), fresh_pixels.borrow().data());
            fresh.teardown();
        }
        pipeline.teardown();
    }

    #[test]
    fn selection_reveal_reuses_recordings_and_handles_parent_resize() {
        let view = |target: f32, viewport: f32| {
            Window::new(
                "Reveal",
                ScrollView::new(
                    HStack::new((
                        Spacer::new().frame(100., 100.).background(Color::RED),
                        Spacer::new().frame(100., 100.).background(Color::GREEN),
                        Spacer::new().frame(100., 100.).background(Color::BLUE),
                        Spacer::new().frame(100., 100.).background(Color::YELLOW),
                    ))
                    .spacing(0.)
                    .frame(400., 100.),
                )
                .horizontal()
                .scrollbar_visibility(crate::views::ScrollbarVisibility::Never)
                .content_size(400., 100.)
                .scroll_to_horizontal_range(target, target + 50.)
                .frame(viewport, 100.),
            )
            .decorated(false)
            .size(Size::new(100., 100.))
        };
        let size = Size::new(100., 100.);
        let pixels = Rc::new(RefCell::new(Buffer::new(size)));
        let lists = Rc::new(RefCell::new(Vec::new()));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_paint_backend(Box::new(Capture {
            retained: true,
            renderer: CpuPaintRenderer::new(size, 1000, Color::WHITE),
            pixels: pixels.clone(),
            lists: lists.clone(),
        }));
        pipeline.set_root(view(0., 100.).create_element());
        pipeline.resize(size);
        pipeline.layout_initial();
        pipeline.render_for_present().unwrap();
        let old_lists = lists.borrow().clone();
        pipeline.reset_paint_test_counters();
        for (target, viewport) in [(310., 100.), (310., 60.), (0., 100.)] {
            let updated = view(target, viewport);
            pipeline
                .element_tree_mut()
                .root_mut()
                .unwrap()
                .update(&updated);
            pipeline.render_for_present().unwrap();
            if viewport == 100. && target == 310. {
                assert!(!pipeline.pipeline_owner().last_composite_ids().is_empty());
                assert!(pipeline.pipeline_owner().last_paint_ids().is_empty());
                assert_eq!(pipeline.paint_test_counters().boundary_rebuilds, 0);
                assert!(
                    old_lists.iter().any(|id| lists.borrow().contains(id)),
                    "unchanged visible recordings must survive selection movement"
                );
            }
            let fresh_pixels = Rc::new(RefCell::new(Buffer::new(size)));
            let mut fresh = RenderingPipeline::new();
            fresh.set_paint_backend(Box::new(Capture {
                retained: false,
                renderer: CpuPaintRenderer::new(size, 1000, Color::WHITE),
                pixels: fresh_pixels.clone(),
                lists: Rc::new(RefCell::new(Vec::new())),
            }));
            fresh.set_root(updated.create_element());
            fresh.resize(size);
            fresh.layout_initial();
            fresh.render_for_present().unwrap();
            let equal = pixels.borrow().data() == fresh_pixels.borrow().data();
            assert!(equal);

            fresh.teardown();
        }
        pipeline.teardown();
    }

    #[test]
    fn offscreen_content_commit_is_not_lost_on_an_idle_frame() {
        let content = State::new(crate::generate_state_id(), "AAA".to_string());
        let mut pipelines = Vec::new();
        for retained in [true, false] {
            let pixels = Rc::new(RefCell::new(Buffer::new(Size::new(200., 100.))));
            let mut pipeline = RenderingPipeline::new();
            pipeline.set_paint_backend(Box::new(Capture {
                retained,
                renderer: CpuPaintRenderer::new(Size::new(200., 100.), 1000, Color::WHITE),
                pixels: pixels.clone(),
                lists: Rc::new(RefCell::new(Vec::new())),
            }));
            let state = content.clone();
            let rows = LazyVStack::new(100, 20., move |index| {
                (if index == 25 {
                    Text::from_state(state.clone())
                } else {
                    Text::new(format!("Row {index}"))
                })
                .frame(200., 20.)
            });
            pipeline.set_root(
                Window::new("Offscreen", ScrollView::new(rows).frame(200., 100.))
                    .decorated(false)
                    .size(Size::new(200., 100.))
                    .create_element(),
            );
            pipeline.resize(Size::new(200., 100.));
            pipeline.layout_initial();
            pipeline.render_for_present().unwrap();
            pipelines.push((pipeline, pixels));
        }
        content.set("BBB".to_string());
        assert!(matches!(
            pipelines[0].0.render_for_present().unwrap(),
            crate::renderer::PresentedFrame::Idle
        ));
        pipelines[1].0.render_for_present().unwrap();
        for (pipeline, _) in &mut pipelines {
            wheel(pipeline, -2000);
            pipeline.render_for_present().unwrap();
        }
        assert_eq!(
            pipelines[0].1.borrow().data(),
            pipelines[1].1.borrow().data(),
            "scrolling must reveal the committed content, even if its update had no visible damage"
        );
    }
}
