//! Immutable paint recordings, separate from the CPU raster layer cache.
//! Topology and positions are reconciled before a frame is replayed. Records
//! never borrow mutable elements; removed virtual rows release their resources.
use super::rendering::RenderingPipeline;
use crate::{
    element::{Element, ElementId},
    geometry::{Point, Rect},
    renderer::{DisplayList, PaintContext},
};
use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};

struct Node {
    generation: u64,
    parent: Option<ElementId>,
    subtree_stamp: u64,
    // A picture is in local coordinates. Moving it changes its parent's
    // contents, but does not invalidate this node's own retained picture.
    content_stamp: u64,
    position: Point,
    bounds: Rect,
    clip: Option<(Rect, f32)>,
    children: Vec<ElementId>,
    paint: Option<Arc<DisplayList>>,
    overlay: Option<Arc<DisplayList>>,
    group: Option<(u64, Arc<DisplayList>)>,
    reads_backdrop: bool,
}

#[derive(Default)]
pub(super) struct RetainedPaint {
    root: Option<ElementId>,
    generation: u64,
    nodes: BTreeMap<ElementId, Node>,
}
impl RetainedPaint {
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = None;
    }
    pub fn sync_tree(
        &mut self,
        root: &dyn Element,
        paint_ids: &[ElementId],
        self_ids: &[ElementId],
        structural_ids: &[ElementId],
        full: bool,
    ) {
        self.root = Some(root.id());
        self.generation = self.generation.wrapping_add(1);
        self.sync(root, paint_ids, self_ids, structural_ids, full, false, None);
        self.nodes
            .retain(|_, node| node.generation == self.generation);
    }
    fn sync(
        &mut self,
        element: &dyn Element,
        paint_ids: &[ElementId],
        self_ids: &[ElementId],
        structural_ids: &[ElementId],
        inherited: bool,
        group: bool,
        parent: Option<ElementId>,
    ) {
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
            clip: None,
            children: Vec::new(),
            paint: None,
            overlay: None,
            group: None,
            reads_backdrop: false,
        });
        let position = element.position();
        let bounds = RenderingPipeline::element_paint_bounds(element, Point::ZERO);
        let clip = RenderingPipeline::clip_for_element(element, Point::ZERO);
        let moved = node.position != position;
        let changed = node.bounds != bounds
            || node.clip != clip
            || !node
                .children
                .iter()
                .copied()
                .eq(element.children().iter().map(|child| child.id()));
        let old_paint = node.paint.as_ref().map_or(0, |list| list.identity());
        let old_overlay = node.overlay.as_ref().map_or(0, |list| list.identity());
        node.generation = self.generation;
        node.parent = parent;
        node.position = position;
        node.bounds = bounds;
        node.clip = clip;
        node.children.clear();
        node.children
            .extend(element.children().iter().map(|child| child.id()));
        if own_dirty {
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
        if own_dirty || structural_ids.contains(&id) {
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
        for child in element.children() {
            self.sync(
                child.as_ref(),
                paint_ids,
                self_ids,
                structural_ids,
                subtree_dirty,
                element.retains_virtual_items()
                    || element
                        .render_object()
                        .is_some_and(|r| r.owns_child_viewport()),
                Some(id),
            );
        }
        let stamp = self.nodes[&id]
            .children
            .iter()
            .fold(self.nodes[&id].content_stamp, |stamp, child| {
                stamp.max(self.nodes[child].subtree_stamp)
            });
        let node = self.nodes.get_mut(&id).unwrap();
        node.content_stamp = stamp;
        node.subtree_stamp = node.subtree_stamp.max(stamp);
        let reads_backdrop = element.render_object().is_some_and(|r| r.reads_backdrop())
            || self.nodes[&id]
                .children
                .iter()
                .any(|child| self.nodes[child].reads_backdrop);
        self.nodes.get_mut(&id).unwrap().reads_backdrop = reads_backdrop;
        if group {
            self.cache_static_subtrees(id);
        }
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
        let node = &self.nodes[&id];
        let mut bounds = node.bounds;
        for child in &node.children {
            let child_node = &self.nodes[child];
            let local = self.group_bounds(*child);
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
        bounds
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
        let mut changed = node.bounds != bounds || node.clip != clip;
        node.position = element.position();
        node.bounds = bounds;
        node.clip = clip;
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
        while let Some(id) = current {
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
        if use_groups {
            if let Some((_, list)) = &node.group {
                if active_clip.is_none_or(|clip| intersect(bounds, clip).is_some())
                    && damage.is_none_or(|rects| RenderingPipeline::overlaps_any(bounds, rects))
                {
                    context.draw_display_list(abs, Arc::clone(list));
                }
                return;
            }
        }
        if active_clip.is_none_or(|clip| intersect(bounds, clip).is_some())
            && damage.is_none_or(|rects| RenderingPipeline::overlaps_any(bounds, rects))
        {
            if let Some(list) = &node.paint {
                context.draw_display_list(abs, Arc::clone(list));
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
        scene.sync_tree(element.as_ref(), &[], &[], &[], true);
        let weak = Arc::downgrade(
            scene
                .nodes
                .values()
                .find_map(|node| node.paint.as_ref())
                .unwrap(),
        );
        for y in [10_000., 20_000., 30_000., 40_000.] {
            element.set_viewport_hint(Rect::from_xywh(0., y, 200., 100.));
            scene.sync_tree(element.as_ref(), &[], &[], &[element.id()], false);
            assert!(
                scene.nodes.len() < 200,
                "the scene must retain mounted virtual rows, not the full model"
            );
        }
        assert!(
            weak.upgrade().is_none(),
            "removed row recordings must release their resources"
        );
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
