//! ElementTree - manages the element tree
//!
//! ElementTree owns the root Element and manages the element lifecycle.
//! StateRegistry is now managed by PipelineOwner to ensure there's only
//! one registry per application.

#![allow(deprecated)]

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::element::{Element, ElementId, TextInputElementState};
use crate::geometry::{Point, Rect, Size};
use crate::pipeline::{MountContext, PipelineId};

/// Global counter for generating unique Element IDs
static ELEMENT_ID_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Generate a new unique Element ID
pub fn generate_element_id() -> u32 {
    ELEMENT_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// ElementTree - owns and manages the element tree
///
/// ElementTree is responsible for:
/// - Owning the root Element
/// - Managing the element lifecycle
/// - Providing access to elements by ID
pub struct ElementTree {
    root: Option<Box<dyn Element>>,
    pipeline_id: PipelineId,
}

impl ElementTree {
    /// Create a new empty ElementTree
    pub fn new() -> Self {
        Self::with_pipeline_id(PipelineId::generate())
    }

    /// Create a new empty ElementTree owned by a rendering pipeline.
    pub fn with_pipeline_id(pipeline_id: PipelineId) -> Self {
        Self {
            root: None,
            pipeline_id,
        }
    }

    /// Set the root Element
    ///
    /// This will unmount the previous root (if any) and mount the new root.
    pub fn set_root(&mut self, root: Box<dyn Element>) {
        // Unmount the old root
        if let Some(ref mut old_root) = self.root {
            old_root.unmount();
        }

        // Set and mount the new root
        self.root = Some(root);
        if let Some(ref mut new_root) = self.root {
            let ctx = MountContext::new(self.pipeline_id);
            new_root.mount(&ctx);
        }
    }

    /// Return the owner pipeline ID.
    pub const fn pipeline_id(&self) -> PipelineId {
        self.pipeline_id
    }

    /// Unmount and remove the root element.
    pub fn clear_root(&mut self) {
        if let Some(ref mut root) = self.root {
            root.unmount();
        }
        self.root = None;
    }

    /// Get the root Element
    pub fn root(&self) -> Option<&(dyn Element + '_)> {
        self.root.as_deref()
    }

    /// Get mutable reference to the root Element
    ///
    /// Note: This is a simplified version. For full mutable access,
    /// use the layout method or work with the tree directly.
    pub fn root_mut(&mut self) -> Option<&mut Box<dyn Element>> {
        self.root.as_mut()
    }

    /// Layout the entire tree
    ///
    /// This performs a layout pass starting from the root.
    pub fn layout(&mut self, constraints: crate::element::LayoutConstraints) -> Size {
        if let Some(ref mut root) = self.root {
            let size = root.layout(constraints);
            self.flush_layout_notifications();
            size
        } else {
            Size::ZERO
        }
    }

    /// Deliver geometry changes after all elements have finished layout.
    pub(crate) fn flush_layout_notifications(&mut self) {
        if let Some(root) = self.root.as_deref_mut() {
            Self::flush_layout_notifications_in(root);
        }
    }

    /// Deliver pending changes only in a subtree that was laid out.
    pub(crate) fn flush_layout_notifications_for(&mut self, id: ElementId) {
        if let Some(element) = self.find_element_mut(id) {
            Self::flush_layout_notifications_in(element.as_mut());
        }
    }

    fn flush_layout_notifications_in(element: &mut dyn Element) {
        if let Some(render_object) = element.render_object_mut() {
            render_object.flush_layout_notifications();
        }
        for child in element.children_mut() {
            Self::flush_layout_notifications_in(child.as_mut());
        }
    }

    /// Find the path of Element IDs from root to the target element.
    pub fn find_path_ids(&self, target: ElementId) -> Option<Vec<ElementId>> {
        let root = self.root.as_deref()?;
        let mut path = Vec::new();
        if Self::find_path_recursive(root, target, &mut path) {
            Some(path)
        } else {
            None
        }
    }

    /// Fill `out` with the path of Element IDs from root to `target`.
    ///
    /// The caller owns the vector so repeated frame-path queries can reuse its
    /// allocation. `out` is always cleared before the search starts.
    pub fn find_path_ids_into(&self, target: ElementId, out: &mut Vec<ElementId>) -> bool {
        out.clear();
        let Some(root) = self.root.as_deref() else {
            return false;
        };
        Self::find_path_recursive(root, target, out)
    }

    /// Find an element by its ID.
    pub fn find_element(&self, id: ElementId) -> Option<&dyn Element> {
        Self::find_element_recursive(self.root.as_deref()?, id)
    }

    /// Resolve a root-to-element path and return the target plus absolute origin.
    ///
    /// This follows only the supplied path, so callers that already have a dirty
    /// element path can avoid walking unrelated subtrees just to compute bounds.
    pub(crate) fn element_and_absolute_origin_for_path(
        &self,
        path: &[ElementId],
    ) -> Option<(&dyn Element, Point)> {
        let mut element = self.root.as_deref()?;
        let mut absolute_origin = Point::ZERO;
        for (index, id) in path.iter().copied().enumerate() {
            if element.id() != id {
                return None;
            }
            absolute_origin = Point::new(
                absolute_origin.x + element.position().x,
                absolute_origin.y + element.position().y,
            );
            if index + 1 == path.len() {
                return Some((element, absolute_origin));
            }
            let next_id = path[index + 1];
            element = element
                .children()
                .iter()
                .find(|child| child.id() == next_id)?
                .as_ref();
        }
        None
    }

    /// Return the deepest repaint boundary in a root-to-target path.
    pub fn find_nearest_repaint_boundary_in_path(&self, path: &[ElementId]) -> Option<ElementId> {
        path.iter().rev().copied().find(|id| {
            self.find_element(*id)
                .and_then(|element| element.render_object())
                .and_then(|render_object| render_object.repaint_boundary_size())
                .is_some()
        })
    }

    /// Fill `path` and return the deepest repaint-boundary ancestor of `target`.
    pub fn find_nearest_repaint_boundary_ancestor_into(
        &self,
        target: ElementId,
        path: &mut Vec<ElementId>,
    ) -> Option<ElementId> {
        self.find_path_ids_into(target, path)
            .then(|| self.find_nearest_repaint_boundary_in_path(path))
            .flatten()
    }

    /// Find the path to the element that currently wants keyboard focus.
    pub fn find_keyboard_focus_path_ids(&self) -> Option<Vec<ElementId>> {
        let root = self.root.as_deref()?;
        let mut path = Vec::new();
        if Self::find_keyboard_focus_recursive(root, &mut path) {
            Some(path)
        } else {
            None
        }
    }

    /// Return text-input state for the focused editable element, if any.
    pub fn focused_text_input_state(&self) -> Option<TextInputElementState> {
        let root = self.root.as_deref()?;
        Self::focused_text_input_state_recursive(root, Point::ZERO, None)
    }

    /// Perform a hit test to find the element at a point
    pub fn hit_test(&self, point: crate::geometry::Point) -> Option<&dyn Element> {
        let root = self.root.as_deref()?;
        self.hit_test_select_overlay_recursive(root, point)
            .or_else(|| self.hit_test_recursive(root, point))
    }

    /// Find an element by its ID
    pub fn find_element_mut(&mut self, id: ElementId) -> Option<&mut Box<dyn Element>> {
        // Use a helper function that doesn't take self
        Self::find_element_recursive_helper(self.root.as_mut()?, id)
    }

    /// Find the nearest focusable element in a path, preferring the deepest one.
    pub fn nearest_focusable_in_path(&mut self, path: &[ElementId]) -> Option<ElementId> {
        for id in path.iter().rev() {
            if self
                .find_element_mut(*id)
                .is_some_and(|element| element.accepts_keyboard_focus())
            {
                return Some(*id);
            }
        }
        None
    }

    /// Return whether an element currently wants keyboard focus.
    pub fn element_wants_keyboard_focus(&mut self, id: ElementId) -> bool {
        self.find_element_mut(id)
            .is_some_and(|element| element.wants_keyboard_focus())
    }

    fn find_element_recursive_helper(
        element: &mut Box<dyn Element>,
        target_id: ElementId,
    ) -> Option<&mut Box<dyn Element>> {
        // Check this element
        if element.id() == target_id {
            return Some(element);
        }

        // Search children
        for child in element.children_mut().iter_mut() {
            if let Some(found) = Self::find_element_recursive_helper(child, target_id) {
                return Some(found);
            }
        }

        None
    }

    fn find_element_recursive(element: &dyn Element, target_id: ElementId) -> Option<&dyn Element> {
        if element.id() == target_id {
            return Some(element);
        }

        for child in element.children() {
            if let Some(found) = Self::find_element_recursive(child.as_ref(), target_id) {
                return Some(found);
            }
        }

        None
    }

    fn hit_test_recursive<'a>(
        &'a self,
        element: &'a dyn Element,
        point: crate::geometry::Point,
    ) -> Option<&'a dyn Element> {
        if !Self::point_inside_element_clip(element, point) {
            return None;
        }

        let local_point = crate::geometry::Point {
            x: point.x - element.position().x,
            y: point.y - element.position().y,
        };
        // Check children first (reverse order for z-index)
        for child in element.children().iter().rev() {
            if let Some(found) = self.hit_test_recursive(child.as_ref(), local_point) {
                return Some(found);
            }
        }

        // Check this element
        if element.hit_test(point) {
            return Some(element);
        }

        None
    }

    fn hit_test_select_overlay_recursive<'a>(
        &'a self,
        element: &'a dyn Element,
        point: crate::geometry::Point,
    ) -> Option<&'a dyn Element> {
        let local_point = crate::geometry::Point {
            x: point.x - element.position().x,
            y: point.y - element.position().y,
        };

        for child in element.children().iter().rev() {
            if let Some(found) = self.hit_test_select_overlay_recursive(child.as_ref(), local_point)
            {
                return Some(found);
            }
        }

        let is_expanded_select = element
            .render_object()
            .and_then(|render_object| {
                render_object
                    .as_any()
                    .downcast_ref::<crate::views::SelectRenderObject>()
            })
            .is_some_and(|select| select.is_expanded());
        if is_expanded_select && element.hit_test(point) {
            return Some(element);
        }

        None
    }

    fn point_inside_element_clip(element: &dyn Element, point: crate::geometry::Point) -> bool {
        let Some(render_object) = element.render_object() else {
            return true;
        };
        let Some((clip_rect, _radius)) = render_object.clip_bounds(element.position()) else {
            return true;
        };
        clip_rect.contains(point)
    }

    fn find_path_recursive(
        element: &dyn Element,
        target: ElementId,
        path: &mut Vec<ElementId>,
    ) -> bool {
        path.push(element.id());
        if element.id() == target {
            return true;
        }

        for child in element.children() {
            if Self::find_path_recursive(child.as_ref(), target, path) {
                return true;
            }
        }

        path.pop();
        false
    }

    fn find_keyboard_focus_recursive(element: &dyn Element, path: &mut Vec<ElementId>) -> bool {
        path.push(element.id());
        if element.wants_keyboard_focus() {
            return true;
        }

        for child in element.children() {
            if Self::find_keyboard_focus_recursive(child.as_ref(), path) {
                return true;
            }
        }

        path.pop();
        false
    }

    fn focused_text_input_state_recursive(
        element: &dyn Element,
        parent_origin: Point,
        ancestor_clip: Option<Rect>,
    ) -> Option<TextInputElementState> {
        let bounds = element.bounds();
        let absolute_origin = Point {
            x: parent_origin.x + bounds.origin.x,
            y: parent_origin.y + bounds.origin.y,
        };

        let intersect = |a: Rect, b: Rect| {
            let left = a.left().max(b.left());
            let top = a.top().max(b.top());
            Rect::from_xywh(
                left,
                top,
                (a.right().min(b.right()) - left).max(0.0),
                (a.bottom().min(b.bottom()) - top).max(0.0),
            )
        };
        let local_clip = element
            .render_object()
            .and_then(|render| render.clip_bounds(absolute_origin))
            .map(|(rect, _)| rect);
        let clip = match (ancestor_clip, local_clip) {
            (Some(a), Some(b)) => Some(intersect(a, b)),
            (a, b) => a.or(b),
        };
        if element.wants_keyboard_focus()
            && let Some(mut state) = element.text_input_state()
        {
            state.editor_id = Some(element.id());
            state.input_rect.origin.x += absolute_origin.x;
            state.input_rect.origin.y += absolute_origin.y;
            if let Some(clip) = clip {
                state.input_rect = intersect(state.input_rect, clip);
            }
            // Keep the caret un-clipped so platform keyboard avoidance can
            // still reveal an editor that has moved outside its viewport.
            state.cursor_rect.origin.x += absolute_origin.x;
            state.cursor_rect.origin.y += absolute_origin.y;
            return Some(state);
        }

        for child in element.children() {
            if let Some(state) =
                Self::focused_text_input_state_recursive(child.as_ref(), absolute_origin, clip)
            {
                return Some(state);
            }
        }

        None
    }

    /// Dump the element tree structure for debugging
    pub fn dump(&self) {
        if !crate::debug::is_enabled() {
            return;
        }
        crate::logln!("[ElementTree] Dumping element tree:");
        if let Some(root) = self.root.as_deref() {
            self.dump_element(root, 0);
        } else {
            crate::logln!("  (empty)");
        }
        crate::logln!("[ElementTree] End of tree dump");
    }

    fn dump_element(&self, element: &dyn Element, depth: usize) {
        // Build indent string
        let mut indent = alloc::string::String::new();
        for _ in 0..depth {
            indent.push_str("  ");
        }

        let type_name = element.type_name_debug();
        let id = element.id().get();
        let children = element.children();
        let has_buffer = element.get_buffer().is_some();

        crate::logln!(
            "{}[{}] id={} (children={}, buffer={})",
            indent,
            type_name,
            id,
            children.len(),
            has_buffer
        );

        for child in children {
            self.dump_element(child.as_ref(), depth + 1);
        }
    }
}

impl Default for ElementTree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod text_input_tests {
    use super::*;
    use crate::element::LayoutConstraints;
    use crate::event::{Event, EventDispatcher, MouseEvent, ScrollSource, WheelPhase};
    use crate::state::{State, generate_state_id};
    use crate::view::{View, ViewExt};
    use crate::views::{ScrollView, TextField};

    #[test]
    fn input_bounds_respect_scroll_clip_without_clipping_caret_or_losing_identity() {
        let text = State::new(generate_state_id(), "long text beyond viewport ".repeat(8));
        let root = ScrollView::new(
            TextField::new(text.clone())
                .autofocus(true)
                .frame(200.0, 60.0),
        )
        .both_axes()
        .content_size(200.0, 200.0)
        .frame(100.0, 40.0)
        .padding(10.0);
        let mut tree = ElementTree::new();
        tree.set_root(root.create_element());
        tree.layout(LayoutConstraints::tight(120.0, 60.0));
        let initial = tree.focused_text_input_state().unwrap();
        assert_eq!(initial.input_rect, Rect::from_xywh(10.0, 10.0, 100.0, 40.0));
        assert!(initial.cursor_rect.right() > initial.input_rect.right());
        assert_eq!(
            initial.editor_id,
            tree.find_keyboard_focus_path_ids().unwrap().last().copied()
        );
        tree.layout(LayoutConstraints::tight(120.0, 60.0));
        assert_eq!(
            tree.focused_text_input_state().unwrap().editor_id,
            initial.editor_id
        );

        let mut dispatcher = EventDispatcher::new();
        dispatcher.set_root(tree.root().unwrap().id());
        dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: -1000,
                x: 20,
                y: 20,
                phase: WheelPhase::Started,
                source: ScrollSource::Trackpad,
            }),
        );
        let scrolled = tree.focused_text_input_state().unwrap();
        assert_eq!(scrolled.editor_id, initial.editor_id);
        assert_eq!(scrolled.input_rect.size.height, 0.0);
        assert!(!scrolled.input_rect.contains(Point::new(20.0, 20.0)));
        assert!(scrolled.cursor_rect.origin.y < initial.cursor_rect.origin.y);

        tree.set_root(TextField::new(text).autofocus(true).create_element());
        tree.layout(LayoutConstraints::tight(120.0, 60.0));
        assert_ne!(
            tree.focused_text_input_state().unwrap().editor_id,
            initial.editor_id
        );
    }
}
