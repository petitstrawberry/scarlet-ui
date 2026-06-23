//! LazyVStack - fixed-height virtualized vertical stack.
//!
//! `LazyVStack` is intended for large scrollable lists. It keeps element
//! instances only for the items near the current viewport hint provided by
//! `ScrollView`.

use crate::element::{Element, ElementId, LayoutConstraints, UpdateResult};
use crate::geometry::{Point, Rect, Size};
use crate::pipeline::MountContext;
use crate::view::View;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;

const DEFAULT_CACHE_EXTENT: f32 = 512.0;

type LazyItemBuilder = Rc<dyn Fn(usize) -> Box<dyn View>>;

/// Virtualized vertical stack with a fixed item extent.
#[derive(Clone)]
pub struct LazyVStack {
    item_count: usize,
    item_height: f32,
    spacing: f32,
    cache_extent: f32,
    builder: LazyItemBuilder,
}

impl LazyVStack {
    /// Create a lazy vertical stack.
    ///
    /// `builder` is called only for items that are near the current viewport.
    pub fn new<V>(
        item_count: usize,
        item_height: f32,
        builder: impl Fn(usize) -> V + 'static,
    ) -> Self
    where
        V: View + 'static,
    {
        Self {
            item_count,
            item_height: item_height.max(1.0),
            spacing: 0.0,
            cache_extent: DEFAULT_CACHE_EXTENT,
            builder: Rc::new(move |index| Box::new(builder(index))),
        }
    }

    /// Set spacing between items.
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing.max(0.0);
        self
    }

    /// Set extra logical pixels to materialize above and below the viewport.
    pub fn cache_extent(mut self, extent: f32) -> Self {
        self.cache_extent = extent.max(0.0);
        self
    }

    /// Return the item count.
    pub fn item_count(&self) -> usize {
        self.item_count
    }

    /// Return the fixed item height.
    pub fn item_height(&self) -> f32 {
        self.item_height
    }

    fn stride(&self) -> f32 {
        self.item_height + self.spacing
    }

    fn total_height(&self) -> f32 {
        if self.item_count == 0 {
            0.0
        } else {
            self.item_count as f32 * self.item_height
                + self.item_count.saturating_sub(1) as f32 * self.spacing
        }
    }

    fn build_item(&self, index: usize) -> Box<dyn Element> {
        (self.builder)(index).create_element()
    }
}

impl View for LazyVStack {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(LazyVStackElement::new(self.clone()))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LazyVStackElement {
    id: ElementId,
    view: LazyVStack,
    children: Vec<Box<dyn Element>>,
    child_indices: Vec<usize>,
    position: Point,
    size: Size,
    viewport_hint: Option<Rect>,
    last_constraints: Option<LayoutConstraints>,
    mount_context: Option<MountContext>,
}

impl LazyVStackElement {
    fn new(view: LazyVStack) -> Self {
        Self {
            id: ElementId::generate(),
            view,
            children: Vec::new(),
            child_indices: Vec::new(),
            position: Point::ZERO,
            size: Size::ZERO,
            viewport_hint: None,
            last_constraints: None,
            mount_context: None,
        }
    }

    fn visible_range(&self) -> core::ops::Range<usize> {
        if self.view.item_count == 0 {
            return 0..0;
        }

        let viewport = self.viewport_hint.unwrap_or_else(|| {
            Rect::from_xywh(
                0.0,
                0.0,
                self.size.width,
                self.view.cache_extent.max(self.view.item_height),
            )
        });
        let stride = self.view.stride().max(1.0);
        let start_y = (viewport.top() - self.view.cache_extent).max(0.0);
        let end_y = (viewport.bottom() + self.view.cache_extent).max(start_y);
        let start = (libm::floorf(start_y / stride) as usize).min(self.view.item_count);
        let end = (libm::ceilf(end_y / stride) as usize + 1).min(self.view.item_count);
        start..end.max(start)
    }

    fn materialize_visible_children(&mut self) -> bool {
        let range = self.visible_range();
        if self.child_indices == range.clone().collect::<Vec<_>>() {
            return false;
        }

        let mut old_children = core::mem::take(&mut self.children);
        let mut old_indices = core::mem::take(&mut self.child_indices);
        let mut new_children = Vec::new();
        let mut new_indices = Vec::new();
        let mut changed = false;

        for index in range {
            if let Some(position) = old_indices.iter().position(|old| *old == index) {
                new_children.push(old_children.remove(position));
                old_indices.remove(position);
            } else {
                let mut child = self.view.build_item(index);
                if let Some(ctx) = self.mount_context {
                    child.mount(&ctx);
                }
                new_children.push(child);
                changed = true;
            }
            new_indices.push(index);
        }

        for mut child in old_children {
            child.unmount();
            changed = true;
        }

        self.children = new_children;
        self.child_indices = new_indices;
        changed
    }

    fn layout_visible_children(&mut self) {
        let width = self.size.width.max(0.0);
        for (child, index) in self
            .children
            .iter_mut()
            .zip(self.child_indices.iter().copied())
        {
            child.layout(LayoutConstraints::tight(width, self.view.item_height));
            child.set_position(Point::new(0.0, index as f32 * self.view.stride()));
        }
    }
}

impl Element for LazyVStackElement {
    fn id(&self) -> ElementId {
        self.id
    }

    fn type_name(&self) -> &str {
        "LazyVStackElement"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn children(&self) -> &[Box<dyn Element>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Element>] {
        &mut self.children
    }

    fn update(&mut self, new_view: &dyn View) -> UpdateResult {
        let Some(new_view) = new_view.as_any().downcast_ref::<LazyVStack>() else {
            return UpdateResult::Replaced;
        };
        self.view = new_view.clone();
        self.materialize_visible_children();
        if let Some(constraints) = self.last_constraints {
            self.layout(constraints);
        }
        UpdateResult::Updated
    }

    fn rebuild(&mut self) -> UpdateResult {
        UpdateResult::NoChange
    }

    fn mount(&mut self, ctx: &MountContext) {
        self.mount_context = Some(*ctx);
        for child in self.children.iter_mut() {
            child.mount(ctx);
        }
    }

    fn unmount(&mut self) {
        for child in self.children.iter_mut() {
            child.unmount();
        }
        self.mount_context = None;
    }

    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.last_constraints = Some(constraints);
        let width = if constraints.max_width.is_finite() {
            constraints.max_width.max(constraints.min_width)
        } else {
            constraints.min_width.max(0.0)
        };
        self.size = Size::new(width, self.view.total_height());
        self.materialize_visible_children();
        self.layout_visible_children();
        self.size
    }

    fn last_layout_constraints(&self) -> Option<LayoutConstraints> {
        self.last_constraints
    }

    fn set_last_layout_constraints(&mut self, constraints: LayoutConstraints) {
        self.last_constraints = Some(constraints);
    }

    fn position(&self) -> Point {
        self.position
    }

    fn set_position(&mut self, position: Point) {
        self.position = position;
    }

    fn set_viewport_hint(&mut self, viewport: Rect) -> bool {
        if self.viewport_hint == Some(viewport) {
            return false;
        }
        self.viewport_hint = Some(viewport);
        let changed = self.materialize_visible_children();
        if changed {
            self.layout_visible_children();
        }
        changed
    }

    fn bounds(&self) -> Rect {
        Rect::new(self.position, self.size)
    }

    fn hit_test(&self, point: Point) -> bool {
        self.bounds().contains(point)
    }

    fn clear_buffers(&mut self) {
        for child in self.children.iter_mut() {
            child.clear_buffers();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::Text;

    #[test]
    fn materializes_only_items_near_viewport() {
        let view = LazyVStack::new(1_000, 20.0, |index| Text::new(format!("Item {index}")));
        let mut element = LazyVStackElement::new(view);

        element.set_viewport_hint(Rect::from_xywh(0.0, 400.0, 200.0, 100.0));
        element.layout(LayoutConstraints::tight(200.0, 20_000.0));

        assert!(element.children().len() < 80);
        assert!(element.child_indices.first().copied().unwrap_or(0) < 20);
        assert!(element.child_indices.last().copied().unwrap_or(0) > 20);
    }
}
