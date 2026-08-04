//! ComponentElement - wraps Views and manages their lifecycle
//!
//! ComponentElement is the bridge between Views (immutable descriptions) and
//! the element tree (mutable runtime objects).

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use crate::element::{Element, ElementId, LayoutConstraints, UpdateResult};
use crate::geometry::{Point, Size};
use crate::pipeline::{MountContext, PipelineId};
use crate::state::{InvalidationKind, SubscriptionId};
use crate::view::View;

/// Element that wraps a View and manages its lifecycle
///
/// ComponentElement is responsible for:
/// - Owning the View instance
/// - Tracking State subscriptions for rebuilds
/// - Reconciling the child View description with a retained child Element
pub struct ComponentElement<V: View + Clone> {
    id: ElementId,
    view: V,
    build_child: fn(&V) -> Box<dyn View>,
    child: Option<Box<dyn Element>>,
    size: Size,
    position: Point,
    last_constraints: Option<LayoutConstraints>,
    subscriptions: Vec<SubscriptionId>,
    pipeline_id: PipelineId,
    mounted: bool,
}

impl<V: View + Clone> ComponentElement<V> {
    /// Create a new ComponentElement with a View
    pub fn new(view: V) -> Self {
        Self::new_with_builder(view, default_component_child::<V>)
    }

    /// Create a new ComponentElement with an explicit child builder.
    ///
    /// # Arguments
    ///
    /// * `view` - View value owned by this component.
    /// * `build_child` - Function that builds the child View description for `view`.
    ///
    /// # Returns
    ///
    /// Component element that subscribes to `view` listenables and rebuilds
    /// its child through `build_child`.
    pub fn new_with_builder(view: V, build_child: fn(&V) -> Box<dyn View>) -> Self {
        let id = ElementId::generate();
        let child_view = build_child(&view);
        let child = child_view.create_element();
        Self {
            id,
            view,
            build_child,
            child: Some(child),
            size: Size::ZERO,
            position: Point::ZERO,
            last_constraints: None,
            subscriptions: Vec::new(),
            pipeline_id: PipelineId::default(),
            mounted: false,
        }
    }

    /// Get the View
    pub fn view(&self) -> &V {
        &self.view
    }

    /// Get mutable reference to the View
    pub fn view_mut(&mut self) -> &mut V {
        &mut self.view
    }

    fn subscribe_view_listenables(&mut self) {
        let listenables = self.view.listenables();
        for listenable in listenables {
            let element_id = self.id;
            let pipeline_id = self.pipeline_id;
            let invalidation_kind = listenable.invalidation_kind();
            let callback = Arc::new(move || match invalidation_kind {
                InvalidationKind::Build => {
                    crate::pipeline::mark_element_dirty(pipeline_id, element_id)
                }
                InvalidationKind::Paint => {
                    crate::pipeline::mark_element_needs_paint(pipeline_id, element_id)
                }
            });
            self.subscriptions.push(listenable.subscribe_any(callback));
        }
    }

    fn unsubscribe_view_listenables(&mut self) {
        if self.subscriptions.is_empty() {
            return;
        }

        let listenables = self.view.listenables();
        for (listenable, subscription_id) in listenables.iter().zip(self.subscriptions.iter()) {
            listenable.unsubscribe(*subscription_id);
        }
        self.subscriptions.clear();
    }

    fn reconcile_built_child(&mut self) -> UpdateResult {
        let child_view = (self.build_child)(&self.view);
        let mount_context = self.mounted.then(|| MountContext::new(self.pipeline_id));
        crate::element::update_child(&mut self.child, Some(child_view.as_ref()), mount_context)
    }
}

fn default_component_child<V: View + Clone>(view: &V) -> Box<dyn View> {
    view.clone_view()
}

impl<V: View + Clone> Element for ComponentElement<V> {
    fn id(&self) -> ElementId {
        self.id
    }

    fn type_name(&self) -> &str {
        "ComponentElement"
    }

    fn view_key(&self) -> Option<&crate::view::ViewKey> {
        self.view.key()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn children(&self) -> &[Box<dyn Element>] {
        match &self.child {
            Some(child) => core::slice::from_ref(child),
            None => &[],
        }
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Element>] {
        match &mut self.child {
            Some(child) => core::slice::from_mut(child),
            None => &mut [],
        }
    }

    fn update(&mut self, new_view: &dyn View) -> UpdateResult {
        let Some(new_typed_view) = new_view.as_any().downcast_ref::<V>() else {
            return UpdateResult::Replaced;
        };

        if self.mounted {
            self.unsubscribe_view_listenables();
        }
        self.view = new_typed_view.clone();
        let result = self.reconcile_built_child();
        if self.mounted {
            self.subscribe_view_listenables();
        }
        result
    }

    fn rebuild(&mut self) -> UpdateResult {
        if self.mounted {
            self.unsubscribe_view_listenables();
        }
        let result = self.reconcile_built_child();
        if self.mounted {
            self.subscribe_view_listenables();
        }
        result
    }

    fn mount(&mut self, ctx: &MountContext) {
        self.pipeline_id = ctx.pipeline_id();
        self.mounted = true;
        self.subscribe_view_listenables();

        // Mount the child
        if let Some(ref mut child) = self.child {
            child.mount(ctx);
        }
    }

    fn unmount(&mut self) {
        // Unmount the child first
        if let Some(ref mut child) = self.child {
            child.unmount();
        }

        self.unsubscribe_view_listenables();
        self.mounted = false;
    }

    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.last_constraints = Some(constraints);
        // Delegate layout to the child
        if let Some(ref mut child) = self.child {
            self.size = child.layout(constraints);
        } else {
            self.size = Size::ZERO;
        }
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

    fn bounds(&self) -> crate::geometry::Rect {
        crate::geometry::Rect {
            origin: self.position,
            size: self.size,
        }
    }

    fn hit_test(&self, point: Point) -> bool {
        // `point` is expressed in the parent's coordinate space. Component
        // positions are intentionally not copied into their child Elements,
        // so translate once before delegating into the component-local tree.
        let local_point = Point {
            x: point.x - self.position.x,
            y: point.y - self.position.y,
        };
        if let Some(ref child) = self.child {
            child.hit_test(local_point)
        } else {
            self.bounds().contains(point)
        }
    }

    fn handle_event(&mut self, event: &crate::event::Event, phase: crate::event::Phase) -> bool {
        // Delegate to child
        if let Some(ref mut child) = self.child {
            child.handle_event(event, phase)
        } else {
            false
        }
    }

    fn take_window_action(&mut self) -> Option<crate::event::WindowEvent> {
        self.child
            .as_mut()
            .and_then(|child| child.take_window_action())
    }

    fn fill_width(&self) -> bool {
        self.child
            .as_ref()
            .map(|child| child.fill_width())
            .unwrap_or(false)
    }

    fn fill_height(&self) -> bool {
        self.child
            .as_ref()
            .map(|child| child.fill_height())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::LayoutConstraints;
    use crate::views::Text;

    #[test]
    fn component_position_does_not_propagate_to_child() {
        let mut element = ComponentElement::new(Text::new("child"));

        element.layout(LayoutConstraints::loose(200.0, 100.0));
        element.set_position(Point::new(10.0, 32.0));

        assert_eq!(element.position(), Point::new(10.0, 32.0));
        assert_eq!(element.children()[0].position(), Point::ZERO);
        assert!(!element.hit_test(Point::new(1.0, 1.0)));
        assert!(element.hit_test(Point::new(11.0, 33.0)));
    }
}
