//! Observation of values derived from a view's laid-out geometry.

use alloc::boxed::Box;
use alloc::vec;
use core::any::Any;
use core::marker::PhantomData;

use crate::element::{
    Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult,
};
use crate::geometry::{GeometryProxy, Size};
use crate::state::Listenable;
use crate::view::View;

/// A view modifier that reports changes to a projected geometry value.
#[derive(Clone)]
pub struct OnGeometryChange<V, T, P, A> {
    inner: V,
    project: P,
    action: A,
    value: PhantomData<fn() -> T>,
}

impl<V, T, P, A> OnGeometryChange<V, T, P, A> {
    pub(crate) fn new(inner: V, project: P, action: A) -> Self {
        Self {
            inner,
            project,
            action,
            value: PhantomData,
        }
    }
}

impl<V, T, P, A> View for OnGeometryChange<V, T, P, A>
where
    V: View + Clone,
    T: PartialEq + Clone + 'static,
    P: Fn(GeometryProxy) -> T + Clone + 'static,
    A: Fn(T) + Clone + 'static,
{
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| OnGeometryChangeRenderObject::new(view.project.clone(), view.action.clone()),
            |render, view| {
                render.project = view.project.clone();
                render.action = view.action.clone();
                UpdateResult::NoChange
            },
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object that records geometry during layout and notifies afterward.
pub struct OnGeometryChangeRenderObject<T, P, A> {
    project: P,
    action: A,
    size: Size,
    last_notified: Option<T>,
    pending: Option<T>,
}

impl<T, P, A> OnGeometryChangeRenderObject<T, P, A> {
    fn new(project: P, action: A) -> Self {
        Self {
            project,
            action,
            size: Size::ZERO,
            last_notified: None,
            pending: None,
        }
    }
}

impl<T, P, A> ElementRenderObject for OnGeometryChangeRenderObject<T, P, A>
where
    T: PartialEq + Clone + 'static,
    P: Fn(GeometryProxy) -> T + 'static,
    A: Fn(T) + 'static,
{
    fn layout(&mut self, _constraints: LayoutConstraints) -> Size {
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        self.size = children
            .first_mut()
            .map_or(Size::ZERO, |child| child.layout(constraints));
        let value = (self.project)(GeometryProxy::new(self.size));
        self.pending = (self.last_notified.as_ref() != Some(&value)).then_some(value);
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn flush_layout_notifications(&mut self) {
        if let Some(value) = self.pending.take() {
            self.last_notified = Some(value.clone());
            (self.action)(value);
        }
    }

    fn render(&mut self) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::rc::Rc;
    use core::cell::{Cell, RefCell};

    use crate::element::ElementTree;
    use crate::pipeline::PipelineOwner;
    use crate::view::ViewExt;
    use crate::views::{CanvasView, Text};

    #[test]
    fn projects_geometry_after_layout_and_only_notifies_when_projection_changes() {
        let widths = Rc::new(RefCell::new(alloc::vec::Vec::new()));
        let observed = Rc::clone(&widths);
        let view = CanvasView::new(10.0, 10.0, Rc::new(|_, _, _| {})).on_geometry_change(
            |geometry| geometry.size().width,
            move |width| observed.borrow_mut().push(width),
        );
        let mut tree = ElementTree::new();
        tree.set_root(view.create_element());

        assert!(widths.borrow().is_empty());
        tree.layout(LayoutConstraints::tight(320.0, 240.0));
        assert_eq!(&*widths.borrow(), &[320.0]);

        tree.layout(LayoutConstraints::tight(320.0, 300.0));
        assert_eq!(&*widths.borrow(), &[320.0]);

        tree.layout(LayoutConstraints::tight(420.0, 300.0));
        assert_eq!(&*widths.borrow(), &[320.0, 420.0]);
    }

    #[test]
    fn callbacks_run_after_the_parent_finishes_layout() {
        let order = Rc::new(RefCell::new(alloc::vec::Vec::new()));
        let child_order = Rc::clone(&order);
        let parent_order = Rc::clone(&order);
        let view = CanvasView::new(10.0, 10.0, Rc::new(|_, _, _| {}))
            .on_geometry_change(
                |geometry| geometry.size(),
                move |_| child_order.borrow_mut().push("child"),
            )
            .on_geometry_change(
                |geometry| geometry.size(),
                move |_| parent_order.borrow_mut().push("parent"),
            );
        let mut tree = ElementTree::new();
        tree.set_root(view.create_element());

        tree.layout(LayoutConstraints::tight(320.0, 240.0));
        assert_eq!(&*order.borrow(), &["parent", "child"]);
    }

    #[test]
    fn partial_layout_delivers_observer_changes() {
        let projected = Rc::new(Cell::new(0u8));
        let project_value = Rc::clone(&projected);
        let values = Rc::new(RefCell::new(alloc::vec::Vec::new()));
        let observed = Rc::clone(&values);
        let view = Text::new("text")
            .on_geometry_change(
                move |_| project_value.get(),
                move |value| observed.borrow_mut().push(value),
            )
            .frame(100.0, 100.0);
        let mut tree = ElementTree::new();
        tree.set_root(view.create_element());
        tree.layout(LayoutConstraints::tight(100.0, 100.0));
        assert_eq!(&*values.borrow(), &[0]);

        let observer_id = tree.root().unwrap().children()[0].id();
        projected.set(1);
        let mut owner = PipelineOwner::with_pipeline_id(tree.pipeline_id());
        owner.mark_needs_layout(observer_id);
        owner.flush_with_legacy_paint(&mut tree, Size::new(100.0, 100.0), false);
        assert_eq!(&*values.borrow(), &[0, 1]);
    }
}
