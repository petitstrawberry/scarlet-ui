//! NavigationView implementation

use crate::element::{ComponentElement, Element, RenderElement};
use crate::state::Listenable;
use crate::view::View;
use crate::views::Spacer;
use crate::views::navigation::render::NavigationViewRenderObject;
use crate::views::navigation::tuple::NavigationLinkTuple;
use crate::views::navigation::view::NavigationView;
use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::any::Any;

// Internal View that actually creates the RenderElement
struct NavigationContent<T>
where
    T: NavigationLinkTuple + Clone,
{
    nav: NavigationView<T>,
}

impl<T> Clone for NavigationContent<T>
where
    T: NavigationLinkTuple + Clone,
{
    fn clone(&self) -> Self {
        Self {
            nav: self.nav.clone(),
        }
    }
}

impl<T> View for NavigationContent<T>
where
    T: NavigationLinkTuple + Clone + 'static,
{
    fn create_element(&self) -> Box<dyn Element> {
        let selected = self.nav.selected_index_state().get();
        let content_view = self.nav.links().build_content(selected);

        let sidebar_placeholder = Spacer::new();
        let mut children = Vec::new();
        children.push(sidebar_placeholder.create_element());
        if let Some(header_builder) = self.nav.header_builder() {
            children.push(header_builder().create_element());
        }
        children.push(content_view.create_element());

        // Collect labels and icons
        let mut labels = Vec::new();
        let mut icons = Vec::new();
        let mut selection_callbacks = Vec::new();
        for i in 0..self.nav.links().count() {
            labels.push(self.nav.links().get_label(i).to_string());
            icons.push(self.nav.links().get_icon(i));
            selection_callbacks.push(self.nav.links().get_on_select(i));
        }

        let render_object = NavigationViewRenderObject::new(
            labels,
            icons,
            self.nav.selected_index_state().clone(),
            self.nav.get_sidebar_width(),
            self.nav.get_shows_icons(),
            self.nav.get_icon_style(),
            self.nav.get_icon_color(),
            self.nav.get_header_height(),
            selection_callbacks,
        );

        Box::new(RenderElement::with_children(
            self.clone(),
            render_object,
            children,
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        let mut v = Vec::new();
        v.push(self.nav.selected_index_state() as &dyn Listenable);
        v
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// View implementation for NavigationView
impl<T> View for NavigationView<T>
where
    T: NavigationLinkTuple + Clone + 'static,
{
    fn create_element(&self) -> Box<dyn Element> {
        let content = NavigationContent { nav: self.clone() };
        Box::new(ComponentElement::new(content))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        let mut v = Vec::new();
        v.push(self.selected_index_state() as &dyn Listenable);
        v
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
