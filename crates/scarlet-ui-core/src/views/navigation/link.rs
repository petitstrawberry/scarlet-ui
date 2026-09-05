//! NavigationLink - Data structure for navigation items
//!
//! NavigationLink represents a single navigation item in a NavigationView.
//! It is NOT a View - it stores a label, optional typed icon, and a closure
//! that builds the selected content view.

use crate::icon::Icon;
use crate::view::View;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;

/// Navigation link data structure (NOT a View)
///
/// NavigationLink holds the information needed for a single navigation item:
/// - A display label
/// - An optional icon
/// - A closure (wrapped in Rc) that builds the content view when this link is selected
///
/// The closure is wrapped in Rc to allow NavigationLink (and thus NavigationView) to be Clone-able.
/// This is necessary to work with ScarletUI's RenderElement architecture.
///
/// # Content View
///
/// * `V` - View type inferred by [`NavigationLink::new`] from the closure's return value
///
/// # Examples
///
/// ```rust
/// use scarlet_ui_core::{Icon, NavigationLink, Text};
///
/// let link = NavigationLink::new("Home", || Text::new("Welcome to Home"))
///     .icon(Icon::Home);
/// ```
pub struct NavigationLink {
    label: String,
    icon: Option<Icon>,
    /// The closure that builds the content view, wrapped in Rc for Clone-ability
    pub(crate) content_builder: Rc<dyn Fn() -> Box<dyn View>>,
    /// Optional callback invoked when the link becomes selected.
    on_select: Option<Rc<dyn Fn()>>,
}

impl Clone for NavigationLink {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            icon: self.icon,
            content_builder: Rc::clone(&self.content_builder),
            on_select: self.on_select.clone(),
        }
    }
}

impl NavigationLink {
    /// Create a new NavigationLink
    ///
    /// # Arguments
    ///
    /// * `label` - Display text for the navigation item
    /// * `content_builder` - Closure that builds the content view when selected (boxed internally)
    ///
    /// # Returns
    ///
    /// A text-only navigation link. Call [`Self::icon`] to associate an icon.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use scarlet_ui_core::{Icon, NavigationLink, Text};
    /// # struct SettingsView;
    /// # impl SettingsView { fn new() -> Text { Text::new("Settings") } }
    ///
    /// NavigationLink::new("Settings", || SettingsView::new()).icon(Icon::Settings);
    /// ```
    pub fn new<V>(label: impl Into<String>, content_builder: impl Fn() -> V + 'static) -> Self
    where
        V: View + 'static,
    {
        let builder = move || -> Box<dyn View> { Box::new(content_builder()) };
        Self {
            label: label.into(),
            icon: None,
            content_builder: Rc::new(builder),
            on_select: None,
        }
    }

    /// Add an icon to this navigation link.
    ///
    /// Navigation links are text-only unless an icon is explicitly supplied
    /// and their containing NavigationView enables icon display.
    ///
    /// # Arguments
    ///
    /// * `icon` - Typed icon associated with this link.
    ///
    /// # Returns
    ///
    /// The updated navigation link.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Register a callback invoked when this link is selected.
    ///
    /// The callback lets a navigation target update application state without
    /// coupling that state change to page construction.
    pub fn on_select(mut self, callback: impl Fn() + 'static) -> Self {
        self.on_select = Some(Rc::new(callback));
        self
    }

    /// Get the label text
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Get the optional icon.
    ///
    /// # Returns
    ///
    /// The explicitly configured icon, if any.
    pub fn get_icon(&self) -> Option<Icon> {
        self.icon
    }

    /// Build the content view
    ///
    /// This invokes the stored closure to create the content view.
    pub fn build_content(&self) -> Box<dyn View> {
        // Call the Fn through Rc
        (self.content_builder)()
    }

    /// Clone the optional selection callback for navigation dispatch.
    pub(crate) fn selection_callback(&self) -> Option<Rc<dyn Fn()>> {
        self.on_select.clone()
    }
}
