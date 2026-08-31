//! TabView - tabbed content container.
//!
//! `TabView` renders a tab strip and builds only the selected tab content as a
//! child element. Non-selected tab pages are not present in the element tree.

use crate::color::{Color, ColorPalette};
use crate::element::{
    ComponentElement, Element, ElementRenderObject, LayoutConstraints, RenderElement,
};
use crate::event::{Event, MouseButton, MouseEvent, Phase};
use crate::geometry::{Point, Rect, Size};
use crate::graphics;
use crate::input_environment::InteractionMode;
use crate::renderer::PaintContext;
use crate::state::{Listenable, State};
use crate::view::View;
use crate::views::style;
use crate::views::{HorizontalSizeClass, WindowSizeClass};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;

/// Policy that determines where a [`TabView`] places its tab bar.
///
/// `Automatic` follows the current window width: compact windows use a bottom
/// tab bar, while regular and expanded windows keep tabs at the top. Explicit
/// placements always win over adaptive layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabBarPlacement {
    /// Resolve the placement from the current window size.
    #[default]
    Automatic,
    /// Place the tab bar above the selected content.
    Top,
    /// Place the tab bar below the selected content.
    Bottom,
}

/// Concrete tab-bar edge after resolving a [`TabBarPlacement`] policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabBarPosition {
    /// The tab bar occupies the top edge of the view.
    Top,
    /// The tab bar occupies the bottom edge of the view.
    Bottom,
}

impl TabBarPlacement {
    /// Resolve this placement policy for an interaction mode.
    ///
    /// # Arguments
    ///
    /// * `interaction_mode` - Retained for source compatibility and ignored.
    ///
    /// # Returns
    ///
    /// The concrete top or bottom tab-bar position.
    pub const fn resolve(self, _interaction_mode: InteractionMode) -> TabBarPosition {
        match self {
            Self::Automatic => TabBarPosition::Top,
            Self::Top => TabBarPosition::Top,
            Self::Bottom => TabBarPosition::Bottom,
        }
    }

    /// Resolve this placement from one window's actual logical bounds.
    ///
    /// # Arguments
    ///
    /// * `available_size` - Logical size available to the tab view.
    ///
    /// # Returns
    ///
    /// Bottom placement for compact windows and top placement otherwise.
    pub const fn resolve_for_size(self, available_size: Size) -> TabBarPosition {
        match self {
            Self::Automatic => {
                if matches!(
                    WindowSizeClass::for_size(available_size).horizontal,
                    HorizontalSizeClass::Compact
                ) {
                    TabBarPosition::Bottom
                } else {
                    TabBarPosition::Top
                }
            }
            Self::Top => TabBarPosition::Top,
            Self::Bottom => TabBarPosition::Bottom,
        }
    }
}

/// A single tab item used by [`TabView`].
#[derive(Clone)]
pub struct TabItem {
    label: String,
    content_builder: Rc<dyn Fn() -> Box<dyn View>>,
}

impl TabItem {
    /// Create a tab item.
    ///
    /// # Arguments
    ///
    /// * `label` - Text shown in the tab strip.
    /// * `content_builder` - Closure that builds this tab's content view.
    ///
    /// # Returns
    ///
    /// New tab item.
    pub fn new<V>(label: impl Into<String>, content_builder: impl Fn() -> V + 'static) -> Self
    where
        V: View + 'static,
    {
        let builder = move || -> Box<dyn View> { Box::new(content_builder()) };
        Self {
            label: label.into(),
            content_builder: Rc::new(builder),
        }
    }

    /// Return the tab label.
    ///
    /// # Returns
    ///
    /// Label text.
    pub fn label(&self) -> &str {
        &self.label
    }

    fn build_content(&self) -> Box<dyn View> {
        (self.content_builder)()
    }
}

/// Tabbed content view.
#[derive(Clone)]
pub struct TabView {
    tabs: Vec<TabItem>,
    selected_index: State<usize>,
    tab_bar_height: f32,
    tab_bar_placement: TabBarPlacement,
    tab_padding: f32,
    font_size: f32,
    background_color: Color,
    selected_color: Color,
    hover_color: Color,
    border_color: Color,
    text_color: Color,
    selected_text_color: Color,
}

impl TabView {
    /// Create a tab view with internal selected-index state.
    ///
    /// # Arguments
    ///
    /// * `tabs` - Tab items.
    ///
    /// # Returns
    ///
    /// New tab view.
    pub fn new(tabs: Vec<TabItem>) -> Self {
        Self::with_selected_index(tabs, State::initial(crate::state::generate_state_id()))
    }

    /// Create a tab view with caller-owned selected-index state.
    ///
    /// # Arguments
    ///
    /// * `tabs` - Tab items.
    /// * `selected_index` - State storing the selected tab index.
    ///
    /// # Returns
    ///
    /// New tab view bound to `selected_index`.
    pub fn with_selected_index(tabs: Vec<TabItem>, selected_index: State<usize>) -> Self {
        let palette = ColorPalette::default();
        Self {
            tabs,
            selected_index,
            tab_bar_height: style::metrics().tab_bar_height,
            tab_bar_placement: TabBarPlacement::Automatic,
            tab_padding: 14.0,
            font_size: 13.0,
            background_color: style::surface_color(&palette, style::SurfaceRole::Structural),
            selected_color: style::surface_color(&palette, style::SurfaceRole::Canvas),
            hover_color: palette.menu_hover(),
            border_color: palette.divider(),
            text_color: palette.text_secondary(),
            selected_text_color: palette.text(),
        }
    }

    /// Set tab bar height.
    ///
    /// # Arguments
    ///
    /// * `height` - Tab bar height in logical pixels.
    ///
    /// # Returns
    ///
    /// Updated tab view.
    pub fn tab_bar_height(mut self, height: f32) -> Self {
        self.tab_bar_height = height.max(1.0);
        self
    }

    /// Set the policy used to place the tab bar.
    ///
    /// The default [`TabBarPlacement::Automatic`] keeps the bar at the top in
    /// laptop mode and moves it to the bottom in tablet mode. Use an explicit
    /// placement when the surrounding layout requires a fixed edge.
    ///
    /// # Arguments
    ///
    /// * `placement` - Automatic or explicitly fixed tab-bar placement.
    ///
    /// # Returns
    ///
    /// Updated tab view.
    pub fn tab_bar_placement(mut self, placement: TabBarPlacement) -> Self {
        self.tab_bar_placement = placement;
        self
    }

    /// Set horizontal tab label padding.
    ///
    /// # Arguments
    ///
    /// * `padding` - Horizontal padding in logical pixels.
    ///
    /// # Returns
    ///
    /// Updated tab view.
    pub fn tab_padding(mut self, padding: f32) -> Self {
        self.tab_padding = padding.max(0.0);
        self
    }

    /// Set tab label font size.
    ///
    /// # Arguments
    ///
    /// * `font_size` - Font size in logical pixels.
    ///
    /// # Returns
    ///
    /// Updated tab view.
    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size.max(1.0);
        self
    }

    /// Return the selected-index state.
    ///
    /// # Returns
    ///
    /// State storing selected tab index.
    pub fn selected_index_state(&self) -> &State<usize> {
        &self.selected_index
    }

    /// Return the number of tabs.
    ///
    /// # Returns
    ///
    /// Tab count.
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Return the configured tab-bar placement policy.
    ///
    /// # Returns
    ///
    /// The automatic or explicit placement policy.
    pub fn configured_tab_bar_placement(&self) -> TabBarPlacement {
        self.tab_bar_placement
    }

    fn selected_tab_index(&self) -> usize {
        if self.tabs.is_empty() {
            0
        } else {
            self.selected_index.get().min(self.tabs.len() - 1)
        }
    }

    fn labels(&self) -> Vec<String> {
        self.tabs
            .iter()
            .map(|tab| tab.label().to_string())
            .collect()
    }

    fn active_content(&self) -> Box<dyn View> {
        if self.tabs.is_empty() {
            Box::new(crate::views::Spacer::new())
        } else {
            self.tabs[self.selected_tab_index()].build_content()
        }
    }
}

impl View for TabView {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_tab_view_content,
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![self.selected_index_state() as &dyn Listenable]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn build_tab_view_content(view: &TabView) -> Box<dyn View> {
    Box::new(TabViewContent { tabs: view.clone() })
}

#[derive(Clone)]
struct TabViewContent {
    tabs: TabView,
}

impl View for TabViewContent {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children(
            self.clone(),
            tab_render_object,
            |view| vec![view.tabs.active_content()],
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![self.tabs.selected_index_state() as &dyn Listenable]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn tab_render_object(view: &TabViewContent) -> TabViewRenderObject {
    TabViewRenderObject::with_placement(
        view.tabs.labels(),
        view.tabs.selected_index_state().clone(),
        view.tabs.tab_bar_height,
        view.tabs.tab_bar_placement,
        view.tabs.tab_padding,
        view.tabs.font_size,
        view.tabs.background_color,
        view.tabs.selected_color,
        view.tabs.hover_color,
        view.tabs.border_color,
        view.tabs.text_color,
        view.tabs.selected_text_color,
    )
}

/// Render object for [`TabView`].
pub struct TabViewRenderObject {
    labels: Vec<String>,
    selected_index: State<usize>,
    hovered_index: Option<usize>,
    pressed_index: Option<usize>,
    tab_bar_height: f32,
    tab_bar_placement: TabBarPlacement,
    tab_bar_position: TabBarPosition,
    tab_padding: f32,
    font_size: f32,
    background_color: Color,
    selected_color: Color,
    hover_color: Color,
    border_color: Color,
    text_color: Color,
    selected_text_color: Color,
    size: Size,
}

impl TabViewRenderObject {
    /// Create a tab view render object with fixed top placement.
    ///
    /// This compatibility constructor preserves the legacy top-tab behavior.
    /// Use [`TabViewRenderObject::with_placement`] to select an automatic or
    /// explicitly fixed policy.
    ///
    /// # Arguments
    ///
    /// * `labels` - Labels displayed in the tab bar.
    /// * `selected_index` - State storing the selected tab index.
    /// * `tab_bar_height` - Height of the tab bar in logical pixels.
    /// * `tab_padding` - Horizontal padding around each label.
    /// * `font_size` - Label font size in logical pixels.
    /// * `background_color` - Tab-bar background color.
    /// * `selected_color` - Selected-tab background color.
    /// * `hover_color` - Hovered or pressed-tab background color.
    /// * `border_color` - Divider color between content and the tab bar.
    /// * `text_color` - Text color for unselected tabs.
    /// * `selected_text_color` - Text color for the selected tab.
    ///
    /// # Returns
    ///
    /// A render object ready for layout.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        labels: Vec<String>,
        selected_index: State<usize>,
        tab_bar_height: f32,
        tab_padding: f32,
        font_size: f32,
        background_color: Color,
        selected_color: Color,
        hover_color: Color,
        border_color: Color,
        text_color: Color,
        selected_text_color: Color,
    ) -> Self {
        Self::with_placement(
            labels,
            selected_index,
            tab_bar_height,
            TabBarPlacement::Top,
            tab_padding,
            font_size,
            background_color,
            selected_color,
            hover_color,
            border_color,
            text_color,
            selected_text_color,
        )
    }

    /// Create a tab view render object with a placement policy.
    ///
    /// # Arguments
    ///
    /// * `labels` - Labels displayed in the tab bar.
    /// * `selected_index` - State storing the selected tab index.
    /// * `tab_bar_height` - Height of the tab bar in logical pixels.
    /// * `tab_bar_placement` - Policy that selects the tab-bar edge.
    /// * `tab_padding` - Horizontal padding around each label.
    /// * `font_size` - Label font size in logical pixels.
    /// * `background_color` - Tab-bar background color.
    /// * `selected_color` - Selected-tab background color.
    /// * `hover_color` - Hovered or pressed-tab background color.
    /// * `border_color` - Divider color between content and the tab bar.
    /// * `text_color` - Text color for unselected tabs.
    /// * `selected_text_color` - Text color for the selected tab.
    ///
    /// # Returns
    ///
    /// A render object ready for layout.
    #[allow(clippy::too_many_arguments)]
    pub fn with_placement(
        labels: Vec<String>,
        selected_index: State<usize>,
        tab_bar_height: f32,
        tab_bar_placement: TabBarPlacement,
        tab_padding: f32,
        font_size: f32,
        background_color: Color,
        selected_color: Color,
        hover_color: Color,
        border_color: Color,
        text_color: Color,
        selected_text_color: Color,
    ) -> Self {
        Self {
            labels,
            selected_index,
            hovered_index: None,
            pressed_index: None,
            tab_bar_height,
            tab_bar_placement,
            tab_bar_position: tab_bar_placement.resolve(InteractionMode::Pointer),
            tab_padding,
            font_size,
            background_color,
            selected_color,
            hover_color,
            border_color,
            text_color,
            selected_text_color,
            size: Size::ZERO,
        }
    }

    /// Return the hovered tab index.
    ///
    /// # Returns
    ///
    /// Hovered index if the pointer is over a tab.
    pub fn hovered_index(&self) -> Option<usize> {
        self.hovered_index
    }

    /// Return the pressed tab index.
    ///
    /// # Returns
    ///
    /// Pressed index if a tab is currently held down.
    pub fn pressed_index(&self) -> Option<usize> {
        self.pressed_index
    }

    /// Return the concrete tab-bar position from the latest layout.
    ///
    /// # Returns
    ///
    /// The top or bottom edge currently occupied by the tab bar.
    pub fn tab_bar_position(&self) -> TabBarPosition {
        self.tab_bar_position
    }

    fn resolve_tab_bar_position(&mut self) {
        self.tab_bar_position = self.tab_bar_placement.resolve_for_size(self.size);
    }

    fn tab_bar_origin_y(&self) -> f32 {
        match self.tab_bar_position {
            TabBarPosition::Top => 0.0,
            TabBarPosition::Bottom => (self.size.height - self.tab_bar_height).max(0.0),
        }
    }

    fn tab_width(&self, label: &str) -> f32 {
        let (text_width, _) = graphics::measure_text_sized(label, self.font_size);
        text_width as f32 + self.tab_padding * 2.0
    }

    fn tab_rect(&self, index: usize) -> Rect {
        let x = self
            .labels
            .iter()
            .take(index)
            .map(|label| self.tab_width(label))
            .sum();
        let width = self
            .labels
            .get(index)
            .map_or(0.0, |label| self.tab_width(label));
        Rect::from_xywh(x, self.tab_bar_origin_y(), width, self.tab_bar_height)
    }

    fn tab_index_at(&self, point: Point) -> Option<usize> {
        for index in 0..self.labels.len() {
            if self.tab_rect(index).contains(point) {
                return Some(index);
            }
        }
        None
    }
}

impl ElementRenderObject for TabViewRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = Size::new(
            finite_tab_axis(constraints.min_width, constraints.max_width),
            finite_tab_axis(constraints.min_height, constraints.max_height),
        );
        self.resolve_tab_bar_position();
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        self.layout(constraints);
        let content_height = (self.size.height - self.tab_bar_height).max(0.0);
        if let Some(child) = children.first_mut() {
            child.layout(LayoutConstraints::tight(self.size.width, content_height));
            let content_y = match self.tab_bar_position {
                TabBarPosition::Top => self.tab_bar_height,
                TabBarPosition::Bottom => 0.0,
            };
            child.set_position(Point::new(0.0, content_y));
        }
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: Point) -> bool {
        self.tab_index_at(point).is_some()
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }

        let Event::Mouse(mouse_event) = event else {
            return false;
        };

        match *mouse_event {
            MouseEvent::Moved { x, y } | MouseEvent::Entered { x, y } => {
                let hovered = self.tab_index_at(Point::new(x as f32, y as f32));
                let changed = hovered != self.hovered_index;
                self.hovered_index = hovered;
                changed
            }
            MouseEvent::Exited { .. } => {
                let changed = self.hovered_index.is_some() || self.pressed_index.is_some();
                self.hovered_index = None;
                self.pressed_index = None;
                changed
            }
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                ..
            } => {
                let pressed = self.tab_index_at(Point::new(x as f32, y as f32));
                let changed = pressed != self.pressed_index;
                self.pressed_index = pressed;
                changed
            }
            MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                ..
            } => self.pressed_index.take().is_some(),
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x,
                y,
                ..
            } => {
                let was_pressed = self.pressed_index.take();
                if let Some(index) = self.tab_index_at(Point::new(x as f32, y as f32)) {
                    if self.selected_index.get() != index {
                        self.selected_index.set(index);
                    }
                    return true;
                }
                was_pressed.is_some()
            }
            _ => false,
        }
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let metrics = style::metrics();
        let tab_bar_y = origin.y + self.tab_bar_origin_y();
        ctx.fill_rect(
            Rect::from_xywh(origin.x, tab_bar_y, self.size.width, self.tab_bar_height),
            self.background_color,
        );

        let selected = self.selected_index.get();
        for (index, label) in self.labels.iter().enumerate() {
            let rect = self.tab_rect(index);
            let rect = Rect::from_xywh(
                origin.x + rect.origin.x,
                origin.y + rect.origin.y,
                rect.size.width,
                rect.size.height,
            );
            if self.pressed_index == Some(index) {
                ctx.fill_rect(rect, self.hover_color);
            } else if index == selected {
                ctx.fill_rect(rect, self.selected_color);
            } else if self.hovered_index == Some(index) {
                ctx.fill_rect(rect, self.hover_color);
            }
            let border_y = match self.tab_bar_position {
                TabBarPosition::Top => rect.bottom() - 1.0,
                TabBarPosition::Bottom => rect.origin.y,
            };
            ctx.fill_rect(
                Rect::from_xywh(rect.origin.x, border_y, rect.size.width, 1.0),
                self.border_color,
            );
            let text_color = if index == selected {
                self.selected_text_color
            } else {
                self.text_color
            };
            let text_y = rect.origin.y + (self.tab_bar_height - self.font_size * 1.2) / 2.0;
            ctx.draw_text(
                Point::new(rect.origin.x + self.tab_padding, text_y),
                label.clone(),
                text_color,
                self.font_size,
            );
        }

        ctx.fill_rect(
            Rect::from_xywh(
                origin.x,
                match self.tab_bar_position {
                    TabBarPosition::Top => tab_bar_y + self.tab_bar_height - 1.0,
                    TabBarPosition::Bottom => tab_bar_y,
                },
                self.size.width,
                1.0,
            ),
            self.border_color,
        );
        if selected < self.labels.len() {
            let selected_rect = self.tab_rect(selected);
            ctx.fill_rect(
                Rect::from_xywh(
                    origin.x + selected_rect.origin.x,
                    match self.tab_bar_position {
                        TabBarPosition::Top => {
                            tab_bar_y + self.tab_bar_height - metrics.tab_indicator_height
                        }
                        TabBarPosition::Bottom => tab_bar_y,
                    },
                    selected_rect.size.width,
                    metrics.tab_indicator_height,
                ),
                ColorPalette::default().primary(),
            );
        }
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn update(&mut self, new_view: &dyn View) -> crate::element::UpdateResult {
        let Some(content) = new_view.as_any().downcast_ref::<TabViewContent>() else {
            return crate::element::UpdateResult::Replaced;
        };

        self.labels = content.tabs.labels();
        self.selected_index = content.tabs.selected_index_state().clone();
        self.tab_bar_height = content.tabs.tab_bar_height;
        self.tab_bar_placement = content.tabs.tab_bar_placement;
        self.tab_padding = content.tabs.tab_padding;
        self.font_size = content.tabs.font_size;
        self.background_color = content.tabs.background_color;
        self.selected_color = content.tabs.selected_color;
        self.hover_color = content.tabs.hover_color;
        self.border_color = content.tabs.border_color;
        self.text_color = content.tabs.text_color;
        self.selected_text_color = content.tabs.selected_text_color;
        self.hovered_index = self
            .hovered_index
            .filter(|index| *index < self.labels.len());
        self.pressed_index = self
            .pressed_index
            .filter(|index| *index < self.labels.len());
        crate::element::UpdateResult::Updated
    }

    fn update_needs_layout(&self) -> bool {
        true
    }

    fn render(&mut self) {
        // PaintCommand path handles tab strip drawing.
    }
}

fn finite_tab_axis(min: f32, max: f32) -> f32 {
    if min.is_finite() && max.is_finite() && min == max {
        max.max(0.0)
    } else if max.is_finite() {
        max.max(min).max(0.0)
    } else if min.is_finite() {
        min.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ComponentElement;
    use crate::pipeline::RenderingPipeline;
    use crate::renderer::PaintCommand;
    use crate::state::State;
    use crate::view::ViewExt;
    use crate::views::{Rectangle, Spacer};

    #[derive(Clone)]
    struct UpdatingTabHarness {
        sample: State<u32>,
        selected_tab: State<usize>,
    }

    fn build_updating_tab_harness(view: &UpdatingTabHarness) -> Box<dyn View> {
        let content_color = if view.sample.get() == 0 {
            Color::rgb(20, 80, 180)
        } else {
            Color::rgb(20, 180, 80)
        };
        let selected_tab = view.selected_tab.clone();
        Box::new(
            crate::vstack! {
                Rectangle::new()
                    .fill(Color::rgb(180, 40, 40))
                    .frame(f32::INFINITY, 100.0),
                Spacer::new().frame_height(14.0),
                TabView::with_selected_index(
                    vec![TabItem::new("Overview", move || {
                        Rectangle::new()
                            .fill(content_color)
                            .frame(f32::INFINITY, f32::INFINITY)
                    })],
                    selected_tab,
                )
                .tab_bar_height(38.0)
                .frame(f32::INFINITY, 450.0)
                .background(Color::rgb(245, 245, 248))
                .clip_radius(12.0),
            }
            .padding(18.0)
            .frame(780.0, 600.0)
            .background(Color::WHITE),
        )
    }

    impl View for UpdatingTabHarness {
        fn create_element(&self) -> Box<dyn Element> {
            Box::new(ComponentElement::new_with_builder(
                self.clone(),
                build_updating_tab_harness,
            ))
        }

        fn listenables(&self) -> Vec<&dyn Listenable> {
            vec![&self.sample]
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn click_changes_selected_index() {
        let selected = State::initial(crate::state::generate_state_id());
        let mut render_object = TabViewRenderObject::new(
            vec![String::from("Mixer"), String::from("Editor")],
            selected.clone(),
            30.0,
            12.0,
            13.0,
            ColorPalette::default().background_secondary(),
            ColorPalette::default().surface(),
            ColorPalette::default().menu_hover(),
            ColorPalette::default().border(),
            ColorPalette::default().text_secondary(),
            ColorPalette::default().text(),
        );
        render_object.layout(LayoutConstraints::tight(300.0, 180.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 80,
                y: 12,
                click_count: 1,
            }),
            Phase::Target,
        ));
        assert_eq!(selected.get(), 1);
    }

    #[test]
    fn automatic_placement_resolves_from_window_width() {
        assert_eq!(
            TabBarPlacement::Automatic.resolve(InteractionMode::Pointer),
            TabBarPosition::Top
        );
        assert_eq!(
            TabBarPlacement::Automatic.resolve(InteractionMode::Touch),
            TabBarPosition::Top
        );
        assert_eq!(
            TabBarPlacement::Automatic.resolve_for_size(Size::new(390.0, 844.0)),
            TabBarPosition::Bottom
        );
        assert_eq!(
            TabBarPlacement::Automatic.resolve_for_size(Size::new(768.0, 540.0)),
            TabBarPosition::Top
        );
        assert_eq!(
            TabBarPlacement::Top.resolve(InteractionMode::Touch),
            TabBarPosition::Top
        );
    }

    #[test]
    fn legacy_render_constructor_keeps_tabs_at_the_top_in_touch_environment() {
        let _environment = crate::input_environment::install_test_input_environment(
            crate::InputEnvironment::new(1, Some(true), None, true, false, false, false),
        );
        let selected = State::initial(crate::state::generate_state_id());
        let mut render_object = TabViewRenderObject::new(
            vec![String::from("Mixer")],
            selected,
            30.0,
            12.0,
            13.0,
            ColorPalette::default().background_secondary(),
            ColorPalette::default().surface(),
            ColorPalette::default().menu_hover(),
            ColorPalette::default().border(),
            ColorPalette::default().text_secondary(),
            ColorPalette::default().text(),
        );

        render_object.layout(LayoutConstraints::tight(300.0, 180.0));

        assert_eq!(render_object.tab_bar_position(), TabBarPosition::Top);
        assert_eq!(render_object.tab_rect(0).origin.y, 0.0);
    }

    #[test]
    fn compact_window_places_automatic_tabs_at_the_bottom_for_layout_and_hits() {
        let selected = State::initial(crate::state::generate_state_id());
        let mut render_object = TabViewRenderObject::with_placement(
            vec![String::from("Mixer"), String::from("Editor")],
            selected.clone(),
            30.0,
            TabBarPlacement::Automatic,
            12.0,
            13.0,
            ColorPalette::default().background_secondary(),
            ColorPalette::default().surface(),
            ColorPalette::default().menu_hover(),
            ColorPalette::default().border(),
            ColorPalette::default().text_secondary(),
            ColorPalette::default().text(),
        );
        let mut children = vec![Spacer::new().create_element()];
        render_object.layout_with_children(LayoutConstraints::tight(300.0, 180.0), &mut children);

        assert_eq!(render_object.tab_bar_position(), TabBarPosition::Bottom);
        assert_eq!(render_object.tab_rect(0).origin.y, 150.0);
        assert_eq!(children[0].position(), Point::ZERO);
        assert!(render_object.hit_test(Point::new(80.0, 162.0)));
        assert!(!render_object.hit_test(Point::new(80.0, 12.0)));
        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 80,
                y: 162,
                click_count: 1,
            }),
            Phase::Target,
        ));
        assert_eq!(selected.get(), 1);
    }

    #[test]
    fn pressing_selected_first_tab_sets_pressed_index() {
        let selected = State::initial(crate::state::generate_state_id());
        let mut render_object = TabViewRenderObject::new(
            vec![String::from("Mixer"), String::from("Editor")],
            selected.clone(),
            30.0,
            12.0,
            13.0,
            ColorPalette::default().background_secondary(),
            ColorPalette::default().surface(),
            ColorPalette::default().menu_hover(),
            ColorPalette::default().border(),
            ColorPalette::default().text_secondary(),
            ColorPalette::default().text(),
        );
        render_object.layout(LayoutConstraints::tight(300.0, 180.0));

        assert_eq!(selected.get(), 0);
        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 12,
                y: 12,
                click_count: 1,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.pressed_index(), Some(0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 12,
                y: 12,
                click_count: 1,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.pressed_index(), None);
        assert_eq!(selected.get(), 0);
    }

    #[test]
    fn cancelled_press_clears_tab_feedback_without_selecting() {
        let selected = State::initial(crate::state::generate_state_id());
        let mut render_object = TabViewRenderObject::new(
            vec![String::from("Mixer"), String::from("Editor")],
            selected.clone(),
            30.0,
            12.0,
            13.0,
            ColorPalette::default().background_secondary(),
            ColorPalette::default().surface(),
            ColorPalette::default().menu_hover(),
            ColorPalette::default().border(),
            ColorPalette::default().text_secondary(),
            ColorPalette::default().text(),
        );
        render_object.layout(LayoutConstraints::tight(300.0, 180.0));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 80,
                y: 12,
                click_count: 1,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.pressed_index(), Some(1));

        assert!(render_object.handle_event(
            &Event::Mouse(MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                x: 80,
                y: 12,
            }),
            Phase::Target,
        ));
        assert_eq!(render_object.pressed_index(), None);
        assert_eq!(selected.get(), 0);
    }

    #[test]
    fn selected_tab_keeps_a_line_indicator() {
        let selected = State::initial(crate::state::generate_state_id());
        let mut render_object = TabViewRenderObject::new(
            vec![String::from("Mixer"), String::from("Editor")],
            selected,
            30.0,
            12.0,
            13.0,
            ColorPalette::default().background_secondary(),
            ColorPalette::default().surface(),
            ColorPalette::default().menu_hover(),
            ColorPalette::default().border(),
            ColorPalette::default().text_secondary(),
            ColorPalette::default().text(),
        );
        render_object.layout(LayoutConstraints::tight(300.0, 180.0));

        let mut ctx = PaintContext::new();
        render_object.paint(&mut ctx, Point::ZERO);
        let primary = ColorPalette::default().primary();
        let indicator = ctx.commands().iter().find_map(|command| match command {
            PaintCommand::FillPath { path, color } if *color == primary => Some(path),
            _ => None,
        });

        let indicator = indicator.expect("selected tab should emit a scarlet line indicator");
        assert_eq!(indicator.len(), 4);
        assert_eq!(indicator[0].y, 28.0);
        assert_eq!(indicator[2].y, 30.0);
    }

    #[test]
    fn bottom_tabs_paint_the_selected_indicator_on_the_content_edge() {
        let selected = State::initial(crate::state::generate_state_id());
        let mut render_object = TabViewRenderObject::with_placement(
            vec![String::from("Mixer"), String::from("Editor")],
            selected,
            30.0,
            TabBarPlacement::Bottom,
            12.0,
            13.0,
            ColorPalette::default().background_secondary(),
            ColorPalette::default().surface(),
            ColorPalette::default().menu_hover(),
            ColorPalette::default().border(),
            ColorPalette::default().text_secondary(),
            ColorPalette::default().text(),
        );
        render_object.layout(LayoutConstraints::tight(300.0, 180.0));

        let mut ctx = PaintContext::new();
        render_object.paint(&mut ctx, Point::ZERO);
        let primary = ColorPalette::default().primary();
        let indicator = ctx.commands().iter().find_map(|command| match command {
            PaintCommand::FillPath { path, color } if *color == primary => Some(path),
            _ => None,
        });

        let indicator = indicator.expect("bottom tabs should paint a selected indicator");
        assert_eq!(indicator[0].y, 150.0);
        assert_eq!(indicator[2].y, 152.0);
    }

    #[test]
    fn tab_strip_and_content_remain_visible_after_ancestor_state_update() {
        let sample = State::new(crate::state::generate_state_id(), 0u32);
        let harness = UpdatingTabHarness {
            sample: sample.clone(),
            selected_tab: State::new(crate::state::generate_state_id(), 0usize),
        };
        let mut pipeline = RenderingPipeline::new();
        pipeline.resize(Size::new(780.0, 600.0));
        pipeline.set_root(harness.create_element());
        pipeline.layout_initial();

        let first = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(30, 190));
        assert_eq!(first, Some(Color::rgb(20, 80, 180).to_bgra()));

        sample.set(1);
        let second = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(30, 190));
        assert_eq!(second, Some(Color::rgb(20, 180, 80).to_bgra()));
    }
}
