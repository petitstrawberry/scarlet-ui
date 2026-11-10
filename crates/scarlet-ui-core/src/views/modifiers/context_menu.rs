//! A layout-transparent context menu provider.
use crate::element::{
    Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult,
};
use crate::geometry::Size;
use crate::view::View;
use crate::views::Menu;
use alloc::{boxed::Box, vec};
use core::any::Any;

/// Wrap a view in a right-click menu. Prefer [`crate::ViewExt::context_menu`].
#[derive(Clone)]
pub struct ContextMenu<V: View> {
    inner: V,
    menu: Menu,
}
impl<V: View> ContextMenu<V> {
    pub fn new(inner: V, menu: Menu) -> Self {
        Self { inner, menu }
    }
}
impl<V: View + Clone> View for ContextMenu<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| ContextMenuRenderObject {
                menu: view.menu.clone(),
                size: Size::ZERO,
            },
            |object, view| {
                object.menu = view.menu.clone();
                UpdateResult::Updated
            },
            |view| vec![view.inner.clone_view()],
        ))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct ContextMenuRenderObject {
    menu: Menu,
    size: Size,
}
impl ElementRenderObject for ContextMenuRenderObject {
    fn context_menu(&self) -> Option<Menu> {
        Some(self.menu.clone())
    }
    fn layout(&mut self, _: LayoutConstraints) -> Size {
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
        self.size
    }
    fn size(&self) -> Size {
        self.size
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn render(&mut self) {}
}
