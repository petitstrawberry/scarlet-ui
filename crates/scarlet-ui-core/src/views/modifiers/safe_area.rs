//! Scoped consumption of platform safe-area layout reservations.
use super::PaddingRenderObject;
use crate::{
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    geometry::{EdgeInsets, SafeAreaEdges, Size},
    view::View,
};
use alloc::{boxed::Box, vec, vec::Vec};
use core::any::Any;

/// Pad content by the unconsumed platform safe area, removing handled edges
/// from its descendants. Place this at the boundary that owns safe layout.
/// Unlike ordinary padding, nested SafeArea containers do not add an edge twice.
/// Use `Window::content_safe_area(SafeAreaEdges::NONE)` when this container,
/// rather than Window, should own the reservation.
#[derive(Clone)]
pub struct SafeArea<V: View> {
    inner: V,
    edges: SafeAreaEdges,
}
impl<V: View> SafeArea<V> {
    pub fn new(inner: V) -> Self {
        Self {
            inner,
            edges: SafeAreaEdges::ALL,
        }
    }
    pub fn edges(mut self, edges: SafeAreaEdges) -> Self {
        self.edges = edges;
        self
    }
}
impl<V: View + Clone> View for SafeArea<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| SafeAreaRenderObject {
                edges: view.edges,
                padding: PaddingRenderObject::new(EdgeInsets::ZERO),
            },
            |render, view| {
                if render.edges == view.edges {
                    return UpdateResult::NoChange;
                }
                render.edges = view.edges;
                UpdateResult::Updated
            },
            |view| vec![view.inner.clone_view()],
        ))
    }
    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        self.inner.listenables()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct SafeAreaRenderObject {
    edges: SafeAreaEdges,
    padding: PaddingRenderObject,
}
impl ElementRenderObject for SafeAreaRenderObject {
    fn set_safe_area_insets(&mut self, insets: EdgeInsets) {
        self.padding.set_insets(self.edges.select(insets));
    }
    fn child_safe_area_insets(&self, insets: EdgeInsets) -> EdgeInsets {
        self.edges.remaining(insets)
    }
    fn update_needs_layout(&self) -> bool {
        true
    }
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.padding.layout(constraints)
    }
    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        self.padding.layout_with_children(constraints, children)
    }
    fn size(&self) -> Size {
        self.padding.size()
    }
    fn render(&mut self) {}
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
