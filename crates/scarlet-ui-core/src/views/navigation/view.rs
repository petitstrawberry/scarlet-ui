//! NavigationView - adaptive navigation with dynamic content switching
//!
//! NavigationView provides sidebar and bottom-bar presentations where users can
//! select different items to display different content views.

use crate::os::Mutex;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;

use crate::color::Color;
use crate::icon::{IconFill, IconStyle, IconWeight};
use crate::input_environment::InteractionMode;
use crate::state::State;
use crate::view::View;
use crate::views::navigation::tuple::NavigationLinkTuple;

static NAVIGATION_SELECTED_REGISTRY: Mutex<BTreeMap<usize, State<usize>>> =
    Mutex::new(BTreeMap::new());

fn navigation_selected_state(key: usize) -> State<usize> {
    let mut registry = NAVIGATION_SELECTED_REGISTRY.lock();
    if let Some(state) = registry.get(&key) {
        return state.clone();
    }

    let state = State::new(crate::state::generate_state_id(), 0);
    registry.insert(key, state.clone());
    state
}

const MINIMUM_AUTOMATIC_CONTENT_WIDTH: f32 = 320.0;

/// Policy controlling how a navigation view presents its destinations.
///
/// Automatic presentation responds to both the live interaction mode and the
/// available width. Direct-touch environments use a bottom bar. Pointer and
/// hybrid environments retain a sidebar while at least 320 logical pixels
/// remain for content, then fall back to a bottom bar at narrower widths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NavigationPresentation {
    /// Select a sidebar or bottom bar from the interaction mode and width.
    #[default]
    Automatic,
    /// Always place navigation destinations in a leading sidebar.
    Sidebar,
    /// Always place navigation destinations in a bottom bar.
    BottomBar,
}

impl NavigationPresentation {
    /// Resolve this policy to a concrete presentation.
    ///
    /// # Arguments
    ///
    /// * `interaction_mode` - Current pointer, touch, or hybrid input mode.
    /// * `available_width` - Total width available to the navigation view.
    /// * `sidebar_width` - Width reserved by the sidebar presentation.
    ///
    /// # Returns
    ///
    /// [`Self::Sidebar`] or [`Self::BottomBar`]. Forced policies are returned
    /// unchanged; automatic presentation uses the input mode and a stable,
    /// content-first width threshold.
    pub fn resolve(
        self,
        interaction_mode: InteractionMode,
        available_width: f32,
        sidebar_width: f32,
    ) -> Self {
        match self {
            Self::Sidebar | Self::BottomBar => self,
            Self::Automatic => {
                let has_usable_content =
                    available_width >= sidebar_width.max(0.0) + MINIMUM_AUTOMATIC_CONTENT_WIDTH;
                if interaction_mode == InteractionMode::Touch || !has_usable_content {
                    Self::BottomBar
                } else {
                    Self::Sidebar
                }
            }
        }
    }
}

/// NavigationView - Adaptive navigation with dynamic content switching
///
/// NavigationView provides a SwiftUI-style navigation interface with:
/// - An adaptive sidebar or bottom bar containing navigation items
/// - A content area that displays the selected item's view
/// - Visual feedback for selection and hover states
///
/// # Type Parameters
///
/// * `T` - Tuple of NavigationLink items
///
/// # Important Notes
///
/// - NavigationView does NOT implement Clone (closures don't support Clone)
/// - When selected_index changes, the entire view tree is rebuilt
/// - The `navigation!` macro preserves selected item state across rebuilds
/// - For page state preservation, use `State<T>` passed to link closures
///
/// # Examples
///
/// ```ignore
/// // Basic usage with macro (recommended)
/// let nav = navigation! {
///     NavigationLink::new("Home", || Text::new("Home View")),
///     NavigationLink::new("Settings", || Text::new("Settings View")),
/// };
///
/// // With state preservation
/// let home_state = State::new(StateId::new(1), HomeData::default());
/// let nav = navigation! {
///     NavigationLink::new("Home", || HomeView::new(home_state.clone())),
/// };
///
/// // With modifiers
/// let nav = navigation! {
///     NavigationLink::new("Home", || Text::new("Home")),
/// }
/// .sidebar_width(250.0)
/// .padding(20.0);
/// ```
pub struct NavigationView<T>
where
    T: NavigationLinkTuple,
{
    /// Tuple of navigation links (stack-only, no heap allocation)
    links: T,
    /// Currently selected link index (tracked via State for reactivity)
    selected_index: State<usize>,
    /// Width reserved when using sidebar presentation.
    sidebar_width: f32,
    /// Requested adaptive presentation policy.
    presentation: NavigationPresentation,
    /// Whether icons are rendered next to sidebar labels.
    shows_icons: bool,
    /// Shared rendering style for sidebar icons.
    icon_style: IconStyle,
    /// Optional shared tint override for sidebar icons.
    icon_color: Option<Color>,
    /// Optional builder for the content header.
    header_builder: Option<Rc<dyn Fn() -> Box<dyn View>>>,
    /// Height reserved for the content header.
    header_height: f32,
}

impl<T> NavigationView<T>
where
    T: NavigationLinkTuple,
{
    /// Create a new NavigationView with the given tuple of links
    ///
    /// # Parameters
    ///
    /// * `links` - Tuple of NavigationLink items (stack-only, no Vec)
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let nav = NavigationView::new((
    ///     NavigationLink::new("Home", || Text::new("Home")),
    ///     NavigationLink::new("Settings", || Text::new("Settings")),
    /// ));
    /// ```
    pub fn new(links: T) -> Self {
        let state_id = crate::state::generate_state_id();

        Self {
            links,
            selected_index: State::new(state_id, 0),
            sidebar_width: 200.0,
            presentation: NavigationPresentation::Automatic,
            shows_icons: false,
            icon_style: IconStyle::default(),
            icon_color: None,
            header_builder: None,
            header_height: 0.0,
        }
    }

    /// Create a new NavigationView with an internal state key.
    ///
    /// This is used by the `navigation!` macro so a NavigationView can preserve
    /// its selected item across view rebuilds without requiring application code
    /// to own the navigation state.
    ///
    /// # Arguments
    ///
    /// * `links` - Tuple of NavigationLink items
    /// * `state_key` - Stable key for this NavigationView call site
    ///
    /// # Returns
    ///
    /// A NavigationView whose selected item state is stored internally.
    pub fn new_with_state_key(links: T, state_key: usize) -> Self {
        Self {
            links,
            selected_index: navigation_selected_state(state_key),
            sidebar_width: 200.0,
            presentation: NavigationPresentation::Automatic,
            shows_icons: false,
            icon_style: IconStyle::default(),
            icon_color: None,
            header_builder: None,
            header_height: 0.0,
        }
    }

    /// Set the width reserved by sidebar presentation.
    ///
    /// # Parameters
    ///
    /// * `width` - Width of the sidebar in points
    pub fn sidebar_width(mut self, width: f32) -> Self {
        self.sidebar_width = width;
        self
    }

    /// Set the navigation presentation policy.
    ///
    /// The default is [`NavigationPresentation::Automatic`], which responds to
    /// the live input environment and available width. Explicit sidebar and
    /// bottom-bar policies disable automatic selection.
    ///
    /// # Arguments
    ///
    /// * `presentation` - Adaptive or forced presentation policy.
    ///
    /// # Returns
    ///
    /// Navigation view with the requested presentation policy.
    pub fn presentation(mut self, presentation: NavigationPresentation) -> Self {
        self.presentation = presentation;
        self
    }

    /// Configure whether icons are shown next to sidebar labels.
    ///
    /// Icons are hidden by default. This keeps navigation links text-only unless
    /// an application explicitly opts into the icon treatment.
    ///
    /// # Arguments
    ///
    /// * `shows_icons` - `true` to render each link's icon.
    ///
    /// # Returns
    ///
    /// Navigation view with the requested icon visibility.
    pub fn shows_icons(mut self, shows_icons: bool) -> Self {
        self.shows_icons = shows_icons;
        self
    }

    /// Set the shared rendering style for sidebar icons.
    ///
    /// # Arguments
    ///
    /// * `style` - Outline style applied to configured link icons.
    ///
    /// # Returns
    ///
    /// Navigation view with the requested icon style.
    pub fn icon_style(mut self, style: IconStyle) -> Self {
        self.icon_style = style;
        self
    }

    /// Set the stroke width for sidebar icons.
    ///
    /// # Arguments
    ///
    /// * `width` - Stroke width in Tabler view-box units.
    ///
    /// # Returns
    ///
    /// Navigation view with the requested icon stroke width.
    pub fn icon_stroke_width(mut self, width: f32) -> Self {
        self.icon_style = self.icon_style.stroke_width(width);
        self
    }

    /// Set a semantic weight for sidebar icons.
    ///
    /// # Arguments
    ///
    /// * `weight` - Thin, normal, or bold stroke weight.
    ///
    /// # Returns
    ///
    /// Navigation view with the requested icon weight.
    pub fn icon_weight(mut self, weight: IconWeight) -> Self {
        self.icon_style = self.icon_style.weight(weight);
        self
    }

    /// Select outline or filled treatment for sidebar icons.
    ///
    /// # Arguments
    ///
    /// * `fill` - Requested vector treatment.
    ///
    /// # Returns
    ///
    /// Navigation view with the requested vector treatment.
    pub fn icon_fill(mut self, fill: IconFill) -> Self {
        self.icon_style = self.icon_style.fill(fill);
        self
    }

    /// Use official filled sidebar icons when available.
    ///
    /// # Returns
    ///
    /// Navigation view using the filled treatment.
    pub fn icons_filled(self) -> Self {
        self.icon_fill(IconFill::Filled)
    }

    /// Override the sidebar icon tint independently from label colors.
    ///
    /// # Arguments
    ///
    /// * `color` - Explicit icon tint.
    ///
    /// # Returns
    ///
    /// Navigation view with the requested icon tint.
    pub fn icon_color(mut self, color: Color) -> Self {
        self.icon_color = Some(color);
        self
    }

    /// Add a header above the selected content view.
    ///
    /// The builder is evaluated whenever the navigation content is rebuilt,
    /// which makes it possible to use stateful controls such as a search field
    /// or a toolbar button in the header. The header is laid out at the full
    /// content width and defaults to 48 logical pixels high.
    ///
    /// # Arguments
    ///
    /// * `builder` - Closure returning the view rendered in the header.
    ///
    /// # Returns
    ///
    /// Navigation view with the supplied content header.
    pub fn header<V>(mut self, builder: impl Fn() -> V + 'static) -> Self
    where
        V: View + 'static,
    {
        self.header_builder = Some(Rc::new(move || Box::new(builder())));
        if self.header_height <= 0.0 {
            self.header_height = 48.0;
        }
        self
    }

    /// Set the height of the content header.
    ///
    /// A non-positive height disables the header. Call [`Self::header`] first
    /// to install a header view.
    ///
    /// # Arguments
    ///
    /// * `height` - Header height in logical pixels.
    ///
    /// # Returns
    ///
    /// Navigation view with the requested header height.
    pub fn header_height(mut self, height: f32) -> Self {
        self.header_height = height.max(0.0);
        self
    }

    /// Get the sidebar width
    pub fn get_sidebar_width(&self) -> f32 {
        self.sidebar_width
    }

    /// Return the configured navigation presentation policy.
    ///
    /// # Returns
    ///
    /// The adaptive or forced presentation selected for this navigation view.
    pub fn get_presentation(&self) -> NavigationPresentation {
        self.presentation
    }

    /// Return whether sidebar link icons are shown.
    ///
    /// # Returns
    ///
    /// `true` when sidebar icons are enabled.
    pub fn get_shows_icons(&self) -> bool {
        self.shows_icons
    }

    pub(crate) fn get_icon_style(&self) -> IconStyle {
        self.icon_style
    }

    pub(crate) fn get_icon_color(&self) -> Option<Color> {
        self.icon_color
    }

    /// Get the number of navigation links
    pub fn link_count(&self) -> usize {
        self.links.count()
    }

    /// Get the label for a link at the given index
    pub fn get_label(&self, index: usize) -> &str {
        self.links.get_label(index)
    }

    /// Get the optional icon for a link at the given index.
    ///
    /// # Arguments
    ///
    /// * `index` - Link index.
    ///
    /// # Returns
    ///
    /// The explicitly configured icon, if any.
    pub fn get_icon(&self, index: usize) -> Option<crate::icon::Icon> {
        self.links.get_icon(index)
    }

    /// Get the selected index State
    pub fn selected_index_state(&self) -> &State<usize> {
        &self.selected_index
    }

    /// Get the links tuple
    pub fn links(&self) -> &T {
        &self.links
    }

    pub(crate) fn header_builder(&self) -> Option<Rc<dyn Fn() -> Box<dyn View>>> {
        self.header_builder.clone()
    }

    pub(crate) fn get_header_height(&self) -> f32 {
        if self.header_builder.is_some() {
            self.header_height
        } else {
            0.0
        }
    }
}

// Clone implementation for NavigationView
// NavigationLink is Clone-able because closures are wrapped in Rc
impl<T> Clone for NavigationView<T>
where
    T: NavigationLinkTuple + Clone,
{
    fn clone(&self) -> Self {
        Self {
            links: self.links.clone(),
            selected_index: self.selected_index.clone(),
            sidebar_width: self.sidebar_width,
            presentation: self.presentation,
            shows_icons: self.shows_icons,
            icon_style: self.icon_style,
            icon_color: self.icon_color,
            header_builder: self.header_builder.clone(),
            header_height: self.header_height,
        }
    }
}

// View trait implementations are in view_impl.rs for each tuple size

#[cfg(test)]
mod tests {
    use super::{NavigationPresentation, NavigationView};
    use crate::views::{NavigationLink, Text};
    use crate::{Icon, InteractionMode};

    #[test]
    fn sidebar_icons_are_hidden_by_default_and_can_be_enabled() {
        let navigation =
            NavigationView::new((
                NavigationLink::new("Overview", || Text::new("Overview")).icon(Icon::Home),
            ));

        assert!(!navigation.get_shows_icons());
        assert!(navigation.shows_icons(true).get_shows_icons());
    }

    #[test]
    fn automatic_presentation_uses_input_mode_and_usable_content_width() {
        assert_eq!(
            NavigationPresentation::Automatic.resolve(InteractionMode::Touch, 1200.0, 200.0),
            NavigationPresentation::BottomBar
        );
        assert_eq!(
            NavigationPresentation::Automatic.resolve(InteractionMode::Pointer, 520.0, 200.0),
            NavigationPresentation::Sidebar
        );
        assert_eq!(
            NavigationPresentation::Automatic.resolve(InteractionMode::Hybrid, 519.0, 200.0),
            NavigationPresentation::BottomBar
        );
    }

    #[test]
    fn explicit_presentation_is_not_changed_by_environment_or_width() {
        assert_eq!(
            NavigationPresentation::Sidebar.resolve(InteractionMode::Touch, 100.0, 200.0),
            NavigationPresentation::Sidebar
        );
        assert_eq!(
            NavigationPresentation::BottomBar.resolve(InteractionMode::Pointer, 1200.0, 200.0),
            NavigationPresentation::BottomBar
        );
    }
}
