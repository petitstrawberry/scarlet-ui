//! Configuration for a replaceable navigation header slot.
use crate::geometry::EdgeInsets;
use crate::view::{View, ViewExt};
use crate::{HeaderBar, Spacer, Text, hstack};
use alloc::string::String;
use alloc::{boxed::Box, rc::Rc};

/// A custom header, including any extension rows, owned by navigation layout.
///
/// Navigation supplies the surface width and places its outline. The root of
/// the supplied view must describe the body, without outer padding or shadows
/// in its layout size. Internal spacing, material and extension rows remain
/// customizable. Shadows use paint_outsets, never layout padding. Navigation
/// reserves the measured body height and the surrounding gap once.
#[derive(Clone)]
pub struct NavigationHeader {
    pub(crate) builder: Option<Rc<dyn Fn() -> Box<dyn View>>>,
    pub(crate) height: Option<f32>,
}

impl NavigationHeader {
    /// Build a custom header with the default height of 48 logical pixels.
    pub fn new<V: View + 'static>(builder: impl Fn() -> V + 'static) -> Self {
        Self {
            builder: Some(Rc::new(move || Box::new(builder()))),
            height: Some(48.),
        }
    }

    /// A standard navigation toolbar. Applications supply controls only;
    /// this helper supplies material, height and internal content padding.
    /// Navigation layout owns outer spacing.
    pub fn toolbar<V: View + Clone + 'static>(builder: impl Fn() -> V + 'static) -> Self {
        Self::new(move || {
            HeaderBar::new(builder().padding_insets(EdgeInsets::new(16., 8., 16., 8.))).height(48.)
        })
        .automatic_height()
    }

    /// A standard navigation title, with the same chrome as a toolbar.
    pub fn title(title: impl Into<String>) -> Self {
        let title = title.into();
        Self::toolbar(move || hstack! { Text::new(title.clone()).font_size(20.), Spacer::new() })
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
