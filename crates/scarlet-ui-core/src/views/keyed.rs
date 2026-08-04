//! Explicit sibling identity for declarative views.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;

use crate::element::{ComponentElement, Element};
use crate::state::Listenable;
use crate::view::{View, ViewKey};

/// A View wrapper carrying stable identity among sibling Views.
#[derive(Clone)]
pub struct Keyed<V: View + Clone> {
    content: V,
    key: ViewKey,
}

impl<V: View + Clone> Keyed<V> {
    /// Wrap a View with a stable sibling key.
    ///
    /// # Arguments
    ///
    /// * `content` - View whose runtime identity should be retained.
    /// * `key` - Identity unique among the View's siblings.
    pub fn new(content: V, key: ViewKey) -> Self {
        Self { content, key }
    }

    /// Return the wrapped View.
    pub fn content(&self) -> &V {
        &self.content
    }
}

fn build_keyed_child<V>(view: &Keyed<V>) -> Box<dyn View>
where
    V: View + Clone + 'static,
{
    view.content.clone_view()
}

impl<V> View for Keyed<V>
where
    V: View + Clone + 'static,
{
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_keyed_child::<V>,
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        // The wrapped View or its descendants own these subscriptions. Keeping
        // this identity-only wrapper passive avoids duplicate rebuilds.
        Vec::new()
    }

    fn key(&self) -> Option<&ViewKey> {
        Some(&self.key)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
