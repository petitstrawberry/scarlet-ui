//! Configuration for a replaceable navigation header slot.
use crate::view::View;
use alloc::{boxed::Box, rc::Rc};

/// A custom header, including any extension rows, owned by navigation layout.
///
/// Navigation supplies a bounded width. The supplied view owns its appearance
/// and internal spacing; no extra material, padding or corner radius is added.
/// The measured or fixed height is reserved once in the content safe area.
#[derive(Clone)]
pub struct NavigationHeader {
    pub(crate) builder: Option<Rc<dyn Fn() -> Box<dyn View>>>,
    pub(crate) height: Option<f32>,
}

impl NavigationHeader {
    /// Build a custom header with the compatibility height of 48 logical pixels.
    pub fn new<V: View + 'static>(builder: impl Fn() -> V + 'static) -> Self {
        Self {
            builder: Some(Rc::new(move || Box::new(builder()))),
            height: Some(48.),
        }
    }

    /// Explicitly suppress the shared header for a destination.
    pub fn hidden() -> Self {
        Self {
            builder: None,
            height: Some(0.),
        }
    }

    /// Reserve a fixed height. Non-finite or non-positive values disable it.
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(if height.is_finite() {
            height.max(0.)
        } else {
            0.
        });
        self
    }

    /// Measure the supplied view at the available width on every layout.
    ///
    /// Use a view with a finite preferred height (for example HeaderBar or a
    /// VStack of fixed-height bars). Live extension rows may grow or disappear
    /// without rebuilding the navigation or losing the content's scroll state.
    pub fn automatic_height(mut self) -> Self {
        self.height = None;
        self
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.builder.is_some() && self.height.is_none_or(|height| height > 0.)
    }
}
