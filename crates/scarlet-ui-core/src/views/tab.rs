//! TabView - tabbed content container.
//!
//! `TabView` renders a tab strip and builds only the selected tab content as a
//! child element. Non-selected tab pages are not present in the element tree.

use crate::color::{Color, ColorPalette};
use crate::element::{
    ComponentElement, Element, ElementRenderObject, LayoutConstraints, RenderElement,
};
use crate::event::{Event, MouseButton, MouseEvent, Phase, TouchGesture, TouchPhase};
use crate::geometry::{EdgeInsets, Point, Rect, Size};
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

/// Colors and optional selected-indicator geometry for a [`TabView`] tab bar.
///
/// Defaults use inset rounded selections without separator or indicator lines.
/// Override these values to match an application's surfaces without replacing
/// the tab bar's interaction behavior.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabStyle {
    /// Background of the full tab bar.
    pub background_color: Color,
    /// Background of the selected tab.
    pub selected_color: Color,
    /// Background of a hovered or pressed tab.
    pub hover_color: Color,
    /// Optional divider between the tab bar and its content. Transparent by default.
    pub border_color: Color,
    /// Label color of unselected tabs.
    pub text_color: Color,
    /// Label color of the selected tab.
    pub selected_text_color: Color,
    /// Color of the selected-tab indicator on the content edge.
    pub indicator_color: Color,
    /// Indicator thickness in logical pixels. Zero hides the indicator.
    ///
    /// Negative or non-finite values are treated as zero, and the thickness is
    /// limited to the tab-bar height when painting.
    pub indicator_height: f32,
}

impl Default for TabStyle {
    fn default() -> Self {
        let palette = ColorPalette::default();
        Self {
            background_color: style::surface_color(&palette, style::SurfaceRole::Structural),
            selected_color: palette.surface(),
            hover_color: palette.header_button_hover(),
            border_color: Color::TRANSPARENT,
            text_color: palette.text_secondary(),
            selected_text_color: palette.primary(),
            indicator_color: palette.primary(),
            indicator_height: style::metrics().tab_indicator_height,
        }
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
    style: TabStyle,
    material: Option<style::GlassMaterial>,
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
        Self {
            tabs,
            selected_index,
            tab_bar_height: style::metrics().tab_bar_height,
            tab_bar_placement: TabBarPlacement::Automatic,
            tab_padding: 16.0,
            font_size: 14.0,
            style: TabStyle::default(),
            material: None,
        }
    }

    /// Set the colors and selected indicator of the tab bar.
    ///
    /// # Arguments
    ///
    /// * `style` - Tab-bar appearance. Geometry and selection behavior are unchanged.
    ///
    /// # Returns
    ///
    /// Updated tab view.
    pub fn style(mut self, style: TabStyle) -> Self {
        self.style = style;
        self
    }

    /// Use a shared backdrop material while retaining the tab-strip structure.
    ///
    /// The full-height selected tab meets the content edge, with a functional
    /// scarlet selection indicator. The surrounding strip and selected tab
    /// sample the real backdrop instead of using opaque fills. A later
    /// [`Self::style`] call can override the colors or hide the indicator.
    /// Content layout and selection behavior are unchanged.
    pub fn material(mut self, material: style::GlassMaterial) -> Self {
        let palette = ColorPalette::default();
        self.material = Some(material);
        self.style.background_color = Color::TRANSPARENT;
        self.style.selected_color = Color::TRANSPARENT;
        self.style.hover_color = palette.text().with_opacity(0.045);
        self.style.border_color = Color::TRANSPARENT;
        self.style.indicator_height = 2.0;
        self
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
    /// regular-width windows and moves it to the bottom in compact windows.
    /// Use an explicit placement when the layout requires a fixed edge.
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
    let mut render_object = TabViewRenderObject::with_placement(
        view.tabs.labels(),
        view.tabs.selected_index_state().clone(),
        view.tabs.tab_bar_height,
        view.tabs.tab_bar_placement,
        view.tabs.tab_padding,
        view.tabs.font_size,
        view.tabs.style.background_color,
        view.tabs.style.selected_color,
        view.tabs.style.hover_color,
        view.tabs.style.border_color,
        view.tabs.style.text_color,
        view.tabs.style.selected_text_color,
    );
    render_object.material = view.tabs.material;
    render_object.indicator_color = view.tabs.style.indicator_color;
    render_object.indicator_height = view.tabs.style.indicator_height;
    render_object
}

/// Render object for [`TabView`].
pub struct TabViewRenderObject {
    labels: Vec<String>,
    selected_index: State<usize>,
    hovered_index: Option<usize>,
    pressed_index: Option<usize>,
    touch_pressed: Option<(u64, usize)>,
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
    indicator_color: Color,
    indicator_height: f32,
    material: Option<style::GlassMaterial>,
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
            touch_pressed: None,
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
            indicator_color: ColorPalette::default().primary(),
            indicator_height: style::metrics().tab_indicator_height,
            material: None,
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
        self.touch_pressed
            .map(|(_, index)| index)
            .or(self.pressed_index)
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
        if !Rect::new(Point::ZERO, self.size).contains(point) {
            return None;
        }
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

        match event {
            Event::Touch(change) if phase == Phase::Target => {
                return match change.phase {
                    TouchPhase::Down => {
                        let pressed =
                            self.tab_index_at(Point::new(change.x as f32, change.y as f32));
                        let next = pressed.map(|index| (change.id, index));
                        if self.touch_pressed.is_none() {
                            self.touch_pressed = next;
                            next.is_some()
                        } else {
                            false
                        }
                    }
                    TouchPhase::Up | TouchPhase::Cancel => {
                        if self.touch_pressed.is_some_and(|(id, _)| id == change.id) {
                            self.touch_pressed = None;
                            true
                        } else {
                            false
                        }
                    }
                    TouchPhase::Move => false,
                };
            }
            Event::TouchGesture(TouchGesture::CancelPress { id }) => {
                if self
                    .touch_pressed
                    .is_some_and(|(pressed_id, _)| pressed_id == *id)
                {
                    self.touch_pressed = None;
                    return true;
                }
                return false;
            }
            Event::TouchGesture(TouchGesture::Tap { x, y, .. }) => {
                if let Some(index) = self.tab_index_at(Point::new(*x as f32, *y as f32)) {
                    if self.selected_index.get() != index {
                        self.selected_index.set(index);
                    }
                    return true;
                }
                return false;
            }
            _ => {}
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

    fn reads_backdrop(&self) -> bool {
        self.material.is_some() && !self.labels.is_empty()
    }

    fn paint_outsets(&self) -> EdgeInsets {
        if self.material.is_some() {
            style::elevation_outsets(style::ElevationRole::Raised)
        } else {
            EdgeInsets::ZERO
        }
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let indicator_height = if self.indicator_height.is_finite() {
            self.indicator_height.max(0.0).min(self.tab_bar_height)
        } else {
            0.0
        };
        let tab_bar_y = origin.y + self.tab_bar_origin_y();
        let bar = Rect::from_xywh(origin.x, tab_bar_y, self.size.width, self.tab_bar_height);
        let clip = Rect::from_xywh(
            origin.x,
            tab_bar_y,
            self.size.width,
            self.tab_bar_height.min(self.size.height),
        );
        if self.material.is_some() {
            let shadow = ColorPalette::default().shadow();
            style::elevation_shadow(
                ctx,
                bar,
                8.0,
                shadow.with_opacity(shadow.a * 1.75),
                style::ElevationRole::Raised,
            );
            ctx.push_rounded_clip(clip, style::radius_for(clip, 8.0));
        } else {
            ctx.push_clip(clip);
        }
        if let Some(mut material) = self.material {
            // A restrained structural tint keeps the strip legible even on a
            // white canvas. It is a uniform material tint, not a bevel.
            material.tint = ColorPalette::default()
                .text()
                .with_opacity(0.045)
                .blend_over(material.tint);
            material.tint = self.background_color.blend_over(material.tint);
            style::paint_glass_material(ctx, bar, style::radius_for(bar, 8.0), material);
        } else if self.background_color.a > 0.0 {
            ctx.fill_rect(bar, self.background_color);
        }

        // Keep visual selection aligned with active_content when external state
        // briefly points beyond a shortened tab collection.
        let selected = self
            .selected_index
            .get()
            .min(self.labels.len().saturating_sub(1));
        for (index, label) in self.labels.iter().enumerate() {
            let local_rect = self.tab_rect(index);
            let rect = Rect::from_xywh(
                origin.x + local_rect.origin.x,
                origin.y + local_rect.origin.y,
                local_rect.size.width,
                local_rect.size.height,
            );
            // Glass retains the original full-height tab geometry. Insets are
            // reserved for the existing non-material treatment.
            let inset = if self.material.is_some() {
                0.0_f32
            } else {
                3.0_f32
            };
            let inset_x = inset.min(rect.size.width * 0.5);
            let inset_y = inset.min(rect.size.height * 0.5);
            let face = Rect::from_xywh(
                rect.origin.x + inset_x,
                rect.origin.y + inset_y,
                (rect.size.width - inset_x * 2.0).max(0.0),
                (rect.size.height - inset_y * 2.0).max(0.0),
            );
            let radius = style::radius_for(face, if self.material.is_some() { 6.0 } else { 12.0 });
            // Only the exposed corners curve. Extending the face through the
            // content edge and clipping it back keeps a tab attached to its view.
            let attached_face = if self.material.is_some() {
                Rect::from_xywh(
                    face.origin.x,
                    face.origin.y
                        - if self.tab_bar_position == TabBarPosition::Bottom {
                            radius
                        } else {
                            0.0
                        },
                    face.size.width,
                    face.size.height + radius,
                )
            } else {
                face
            };
            let pressed = self.pressed_index() == Some(index);
            if index == selected {
                if let Some(mut material) = self.material {
                    let selection = self.selected_color.with_opacity(
                        (self.selected_color.a * if pressed { 1.5 } else { 1.0 }).min(1.0),
                    );
                    material.tint = selection.blend_over(material.tint);
                    if pressed {
                        material.tint = material
                            .tint
                            .with_opacity((material.tint.a + 0.08).min(1.0));
                    } else if self.hovered_index == Some(index) {
                        material.tint = material
                            .tint
                            .with_opacity((material.tint.a + 0.04).min(1.0));
                    }
                    ctx.push_clip(face);
                    style::paint_glass_material(ctx, attached_face, radius, material);
                    ctx.pop_clip();
                } else {
                    // Selection remains visible when held. Depth comes from the
                    // material and a soft shadow, never a fixed highlight line.
                    if !pressed && self.selected_color.a > 0.0 {
                        ctx.draw_rounded_rect_shadow(
                            face,
                            radius,
                            crate::geometry::Offset::new(0.0, 1.0),
                            2.0,
                            0.0,
                            Color::BLACK.with_opacity(0.07 * self.selected_color.a),
                        );
                    }
                    let fill = if pressed {
                        self.selected_color.darken(0.04)
                    } else {
                        self.selected_color
                    };
                    style::fill_raised_surface(ctx, face, radius, fill, pressed);
                }
            } else if pressed || self.hovered_index == Some(index) {
                let fill = if pressed {
                    self.hover_color
                        .darken(0.05)
                        .with_opacity((self.hover_color.a * 1.5).min(1.0))
                } else {
                    self.hover_color
                };
                ctx.push_clip(face);
                ctx.fill_rounded_rect(attached_face, radius, fill);
                ctx.pop_clip();
            }
            let text_color = if index == selected {
                self.selected_text_color
            } else {
                self.text_color
            };
            let text_y = rect.origin.y
                + ((self.tab_bar_height - graphics::line_height_sized(self.font_size) as f32)
                    * 0.5)
                    .max(0.0);
            ctx.draw_text(
                Point::new(rect.origin.x + self.tab_padding, text_y),
                label.clone(),
                text_color,
                self.font_size,
            );
        }

        // The selection indicator meets the content edge. Explicit custom
        // separators and indicators remain source-compatible.
        if self.border_color.a > 0.0 {
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
        }
        if selected < self.labels.len() && indicator_height > 0.0 && self.indicator_color.a > 0.0 {
            let selected_rect = self.tab_rect(selected);
            ctx.fill_rect(
                Rect::from_xywh(
                    origin.x + selected_rect.origin.x,
                    match self.tab_bar_position {
                        TabBarPosition::Top => tab_bar_y + self.tab_bar_height - indicator_height,
                        TabBarPosition::Bottom => tab_bar_y,
                    },
                    selected_rect.size.width,
                    indicator_height,
                ),
                self.indicator_color,
            );
        }
        ctx.pop_clip();
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
        self.background_color = content.tabs.style.background_color;
        self.selected_color = content.tabs.style.selected_color;
        self.hover_color = content.tabs.style.hover_color;
        self.border_color = content.tabs.style.border_color;
        self.text_color = content.tabs.style.text_color;
        self.selected_text_color = content.tabs.style.selected_text_color;
        self.indicator_color = content.tabs.style.indicator_color;
        self.indicator_height = content.tabs.style.indicator_height;
        self.material = content.tabs.material;
        self.hovered_index = self
            .hovered_index
            .filter(|index| *index < self.labels.len());
        self.pressed_index = self
            .pressed_index
            .filter(|index| *index < self.labels.len());
        self.touch_pressed = self
            .touch_pressed
            .filter(|(_, index)| *index < self.labels.len());
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
    fn touch_tab_selection_commits_on_tap_and_cancels_pressed_feedback() {
        let selected = State::initial(crate::state::generate_state_id());
        let palette = ColorPalette::default();
        let mut tabs = TabViewRenderObject::new(
            vec![String::from("Mixer"), String::from("Editor")],
            selected.clone(),
            30.0,
            12.0,
            13.0,
            palette.background_secondary(),
            palette.surface(),
            palette.menu_hover(),
            palette.border(),
            palette.text_secondary(),
            palette.text(),
        );
        tabs.layout(LayoutConstraints::tight(300.0, 180.0));
        let touch = |phase| {
            Event::Touch(crate::event::TouchChange {
                seat_id: 0,
                serial: 1,
                time_ns: 1,
                id: 1,
                phase,
                x: 80,
                y: 12,
                pressure: None,
                touch_major: None,
            })
        };
        tabs.handle_event(&touch(TouchPhase::Down), Phase::Target);
        assert_eq!(tabs.pressed_index(), Some(1));
        tabs.handle_event(
            &Event::TouchGesture(TouchGesture::CancelPress { id: 1 }),
            Phase::Target,
        );
        assert_eq!(tabs.pressed_index(), None);
        assert_eq!(selected.get(), 0);
        tabs.handle_event(&touch(TouchPhase::Down), Phase::Target);
        tabs.handle_event(&touch(TouchPhase::Up), Phase::Target);
        assert_eq!(selected.get(), 0);
        assert!(tabs.handle_event(
            &Event::TouchGesture(TouchGesture::Tap {
                id: 1,
                x: 80,
                y: 12,
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

    fn custom_style() -> TabStyle {
        TabStyle {
            background_color: Color::rgb(10, 11, 12),
            selected_color: Color::rgb(20, 21, 22),
            hover_color: Color::rgb(30, 31, 32),
            border_color: Color::rgb(40, 41, 42),
            text_color: Color::rgb(50, 51, 52),
            selected_text_color: Color::rgb(60, 61, 62),
            indicator_color: Color::rgb(70, 71, 72),
            indicator_height: 3.0,
        }
    }

    fn styled_tabs(style: TabStyle, placement: TabBarPlacement) -> TabViewContent {
        TabViewContent {
            tabs: TabView::new(vec![
                TabItem::new("Mixer", Spacer::new),
                TabItem::new("Editor", Spacer::new),
            ])
            .tab_bar_height(30.0)
            .tab_bar_placement(placement)
            .style(style),
        }
    }

    fn glass_tabs(placement: TabBarPlacement) -> TabViewContent {
        TabViewContent {
            tabs: styled_tabs(TabStyle::default(), placement)
                .tabs
                .material(style::GlassMaterial::regular()),
        }
    }

    #[test]
    fn glass_tabs_keep_the_original_strip_and_selected_content_edge() {
        for placement in [TabBarPlacement::Top, TabBarPlacement::Bottom] {
            let mut tabs = tab_render_object(&glass_tabs(placement));
            tabs.layout(LayoutConstraints::tight(300.0, 180.0));
            assert!(tabs.reads_backdrop());
            let mut ctx = PaintContext::new();
            tabs.paint(&mut ctx, Point::ZERO);
            let selected = tabs.tab_rect(0);
            let glass: Vec<_> = ctx
                .commands()
                .iter()
                .filter_map(|command| match command {
                    PaintCommand::BackdropFilterRoundedRect {
                        rect,
                        corner_radius,
                        blur_radius,
                        ..
                    } => Some((*rect, *corner_radius, *blur_radius)),
                    _ => None,
                })
                .collect();
            assert_eq!(glass.len(), 2, "strip and selected tab share real glass");
            assert_eq!(
                glass[0].0,
                Rect::from_xywh(0.0, tabs.tab_bar_origin_y(), 300.0, 30.0)
            );
            assert_eq!(glass[0].1, 8.0, "the strip has restrained outer corners");
            let selected_glass = Rect::from_xywh(
                selected.origin.x,
                selected.origin.y
                    - if placement == TabBarPlacement::Bottom {
                        6.0
                    } else {
                        0.0
                    },
                selected.size.width,
                selected.size.height + 6.0,
            );
            assert_eq!(glass[1], (selected_glass, 6.0, 18.0));
            assert!(
                ctx.commands().iter().any(|command| matches!(
                    command, PaintCommand::PushClip { rect, .. } if *rect == selected
                )),
                "selected tab remains full height and attached, not a chip"
            );
            let indicator = ctx
                .commands()
                .iter()
                .find_map(|command| match command {
                    PaintCommand::FillPath { path, color } if *color == tabs.indicator_color => {
                        Some(path)
                    }
                    _ => None,
                })
                .expect("functional selection indicator");
            let edge = if placement == TabBarPlacement::Top {
                28.0
            } else {
                150.0
            };
            assert_eq!(indicator[0].y, edge);
            assert_eq!(indicator[2].y, edge + 2.0);
            assert!(
                !ctx.commands().iter().any(|command| matches!(
                    command,
                    PaintCommand::StrokePath { .. }
                        | PaintCommand::StrokeRoundedRect { .. }
                        | PaintCommand::FillVerticalGradientRoundedRect { .. }
                )),
                "no bevel or fixed decorative rim"
            );
            assert!(
                matches!(
                    ctx.commands().first(),
                    Some(PaintCommand::DrawRoundedRectShadow { .. })
                ),
                "a single whole-strip shadow separates glass from white"
            );
            assert_ne!(tabs.paint_outsets(), EdgeInsets::ZERO);
        }
    }

    #[test]
    fn glass_tab_pressed_material_stays_translucent_and_more_distinct() {
        let mut tabs = tab_render_object(&glass_tabs(TabBarPlacement::Top));
        tabs.layout(LayoutConstraints::tight(300.0, 180.0));
        let material_tint = |tabs: &TabViewRenderObject| {
            let mut ctx = PaintContext::new();
            tabs.paint(&mut ctx, Point::ZERO);
            ctx.commands()
                .iter()
                .rev()
                .find_map(|command| match command {
                    PaintCommand::BackdropFilterRoundedRect { tint, .. } => Some(*tint),
                    _ => None,
                })
                .unwrap()
        };
        let rest = material_tint(&tabs);
        tabs.handle_event(
            &Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 12,
                y: 12,
                click_count: 1,
            }),
            Phase::Target,
        );
        let held = material_tint(&tabs);
        assert!(held.a > rest.a && held.a < 1.0);
        assert_eq!(tabs.selected_index.get(), 0);
        tabs.handle_event(
            &Event::Mouse(MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                x: 12,
                y: 12,
            }),
            Phase::Target,
        );
        assert_eq!(material_tint(&tabs), rest);
    }

    #[test]
    fn tab_material_updates_clear_backdrop_dependency_when_removed() {
        let mut tabs = tab_render_object(&glass_tabs(TabBarPlacement::Top));
        assert!(tabs.reads_backdrop());
        tabs.update(&styled_tabs(TabStyle::default(), TabBarPlacement::Top));
        assert!(!tabs.reads_backdrop());
        tabs.update(&glass_tabs(TabBarPlacement::Top));
        assert!(tabs.reads_backdrop());
        let empty = TabViewContent {
            tabs: TabView::new(vec![]).material(style::GlassMaterial::regular()),
        };
        tabs.update(&empty);
        assert!(!tabs.reads_backdrop());
    }

    #[test]
    fn default_style_uses_rounded_material_without_lines() {
        let palette = ColorPalette::default();
        let tabs = TabView::new(vec![]);
        assert_eq!(
            tabs.style.background_color,
            style::surface_color(&palette, style::SurfaceRole::Structural)
        );
        assert_eq!(tabs.style.selected_color, palette.surface());
        assert_eq!(tabs.style.hover_color, palette.header_button_hover());
        assert_eq!(tabs.style.border_color, Color::TRANSPARENT);
        assert_eq!(tabs.style.text_color, palette.text_secondary());
        assert_eq!(tabs.style.selected_text_color, palette.primary());
        assert_eq!(tabs.style.indicator_color, palette.primary());
        assert_eq!(
            tabs.style.indicator_height,
            style::metrics().tab_indicator_height
        );
    }

    #[test]
    fn custom_tab_style_paints_surfaces_labels_and_indicator_at_both_edges() {
        let style = custom_style();
        for (placement, expected_y) in [
            (TabBarPlacement::Top, 27.0),
            (TabBarPlacement::Bottom, 150.0),
        ] {
            let mut tabs = tab_render_object(&styled_tabs(style, placement));
            tabs.layout(LayoutConstraints::tight(300.0, 180.0));
            let editor = tabs.tab_rect(1);
            tabs.handle_event(
                &Event::Mouse(MouseEvent::Moved {
                    x: (editor.origin.x + 1.0) as i32,
                    y: (editor.origin.y + 1.0) as i32,
                }),
                Phase::Target,
            );
            let mut ctx = PaintContext::new();
            tabs.paint(&mut ctx, Point::ZERO);
            for expected in [style.background_color, style.border_color] {
                assert!(ctx.commands().iter().any(|command| matches!(command, PaintCommand::FillPath { color, .. } if *color == expected)));
            }
            assert!(ctx.commands().iter().any(|command| matches!(command,
                PaintCommand::FillRoundedRect { color, .. } if *color == style.selected_color)));
            assert!(ctx.commands().iter().any(|command| matches!(command,
                PaintCommand::FillRoundedRect { color, .. } if *color == style.hover_color)));
            for (label, expected) in [
                ("Mixer", style.selected_text_color),
                ("Editor", style.text_color),
            ] {
                assert!(ctx.commands().iter().any(|command| matches!(command, PaintCommand::DrawText { text, color, .. } if text == label && *color == expected)));
            }
            let indicator = ctx
                .commands()
                .iter()
                .find_map(|command| match command {
                    PaintCommand::FillPath { path, color } if *color == style.indicator_color => {
                        Some(path)
                    }
                    _ => None,
                })
                .expect("custom indicator");
            assert_eq!(indicator[0].y, expected_y);
            assert_eq!(indicator[2].y, expected_y + 3.0);
            assert_eq!(tabs.selected_index.get(), 0);
        }
    }

    #[test]
    fn style_update_reaches_existing_render_object() {
        let mut tabs = tab_render_object(&styled_tabs(TabStyle::default(), TabBarPlacement::Top));
        tabs.layout(LayoutConstraints::tight(300.0, 180.0));
        assert!(matches!(
            tabs.update(&styled_tabs(custom_style(), TabBarPlacement::Top)),
            crate::element::UpdateResult::Updated
        ));
        let mut ctx = PaintContext::new();
        tabs.paint(&mut ctx, Point::ZERO);
        assert!(ctx.commands().iter().any(|command| matches!(command, PaintCommand::FillPath { color, .. } if *color == custom_style().indicator_color)));
        assert!(ctx.commands().iter().any(|command| matches!(command, PaintCommand::DrawText { color, .. } if *color == custom_style().selected_text_color)));
    }

    #[test]
    fn indicator_height_is_bounded_and_can_be_hidden() {
        for height in [0.0, -1.0, f32::NAN, f32::INFINITY, 300.0] {
            let style = TabStyle {
                indicator_height: height,
                ..custom_style()
            };
            let mut tabs = tab_render_object(&styled_tabs(style, TabBarPlacement::Top));
            tabs.layout(LayoutConstraints::tight(300.0, 180.0));
            let mut ctx = PaintContext::new();
            tabs.paint(&mut ctx, Point::ZERO);
            let indicator = ctx.commands().iter().find_map(|command| match command {
                PaintCommand::FillPath { path, color } if *color == style.indicator_color => {
                    Some(path)
                }
                _ => None,
            });
            if height == 300.0 {
                let path = indicator.expect("indicator clamped to tab height");
                assert_eq!(path[0].y, 0.0);
                assert_eq!(path[2].y, 30.0);
            } else {
                assert!(indicator.is_none());
            }
        }
    }

    #[test]
    fn default_selection_is_an_inset_rounded_face_at_both_edges() {
        for placement in [TabBarPlacement::Top, TabBarPlacement::Bottom] {
            let mut tabs = tab_render_object(&styled_tabs(TabStyle::default(), placement));
            tabs.layout(LayoutConstraints::tight(300.0, 180.0));
            let mut ctx = PaintContext::new();
            tabs.paint(&mut ctx, Point::ZERO);
            let face = ctx
                .commands()
                .iter()
                .find_map(|command| match command {
                    PaintCommand::FillRoundedRect {
                        rect,
                        corner_radius,
                        ..
                    } => Some((*rect, *corner_radius)),
                    _ => None,
                })
                .expect("selected tab uses rounded material");
            let tab = tabs.tab_rect(0);
            assert_eq!(
                face.0.origin,
                Point::new(tab.origin.x + 3.0, tab.origin.y + 3.0)
            );
            assert_eq!(
                face.0.size,
                Size::new(tab.size.width - 6.0, tab.size.height - 6.0)
            );
            assert_eq!(face.1, 12.0);
            assert!(
                ctx.commands()
                    .iter()
                    .any(|command| matches!(command, PaintCommand::DrawRoundedRectShadow { .. }))
            );
            assert_eq!(
                ctx.commands()
                    .iter()
                    .filter(|command| matches!(command, PaintCommand::FillPath { .. }))
                    .count(),
                1,
                "only the bar background is rectangular"
            );
            assert!(!ctx.commands().iter().any(|command| matches!(
                command,
                PaintCommand::StrokeRoundedRect { .. } | PaintCommand::StrokePath { .. }
            )));
        }
    }

    #[test]
    fn selected_tab_keeps_its_material_and_text_when_pressed() {
        let mut tabs = tab_render_object(&styled_tabs(TabStyle::default(), TabBarPlacement::Top));
        tabs.layout(LayoutConstraints::tight(300.0, 180.0));
        tabs.handle_event(
            &Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 12,
                y: 12,
                click_count: 1,
            }),
            Phase::Target,
        );
        let mut ctx = PaintContext::new();
        tabs.paint(&mut ctx, Point::ZERO);
        assert!(ctx.commands().iter().any(|command| matches!(command,
            PaintCommand::FillRoundedRect { color, .. }
            if *color == tabs.selected_color.darken(0.04).darken(0.035))));
        assert!(ctx.commands().iter().any(|command| matches!(command,
            PaintCommand::DrawText { text, color, .. }
            if text == "Mixer" && *color == tabs.selected_text_color)));
        assert!(
            !ctx.commands()
                .iter()
                .any(|command| matches!(command, PaintCommand::DrawRoundedRectShadow { .. }))
        );
    }

    #[test]
    fn selected_face_clamps_to_the_same_tab_as_content() {
        let content = styled_tabs(TabStyle::default(), TabBarPlacement::Top);
        content.tabs.selected_index.set(20);
        let mut tabs = tab_render_object(&content);
        tabs.layout(LayoutConstraints::tight(300.0, 180.0));
        let mut ctx = PaintContext::new();
        tabs.paint(&mut ctx, Point::ZERO);
        assert!(ctx.commands().iter().any(|command| matches!(command,
            PaintCommand::DrawText { text, color, .. }
            if text == "Editor" && *color == tabs.selected_text_color)));
        assert_eq!(content.tabs.selected_tab_index(), 1);
    }

    #[test]
    fn label_vertical_alignment_uses_the_real_line_height() {
        let mut tabs = tab_render_object(&styled_tabs(TabStyle::default(), TabBarPlacement::Top));
        tabs.layout(LayoutConstraints::tight(300.0, 180.0));
        let mut ctx = PaintContext::new();
        tabs.paint(&mut ctx, Point::new(10.0, 20.0));
        let expected =
            20.0 + ((30.0 - graphics::line_height_sized(tabs.font_size) as f32) * 0.5).max(0.0);
        for command in ctx.commands() {
            if let PaintCommand::DrawText { position, .. } = command {
                assert_eq!(position.y, expected);
            }
        }
    }

    #[test]
    fn overflowing_tabs_are_not_hit_outside_the_view() {
        let mut tabs = tab_render_object(&styled_tabs(TabStyle::default(), TabBarPlacement::Top));
        tabs.layout(LayoutConstraints::tight(40.0, 180.0));
        assert!(tabs.hit_test(Point::new(20.0, 12.0)));
        assert!(!tabs.hit_test(Point::new(80.0, 12.0)));
        let mut ctx = PaintContext::new();
        tabs.paint(&mut ctx, Point::new(10.0, 20.0));
        assert!(
            matches!(ctx.commands().first(), Some(PaintCommand::PushClip { rect, .. })
            if *rect == Rect::from_xywh(10.0, 20.0, 40.0, 30.0))
        );
        assert!(matches!(ctx.commands().last(), Some(PaintCommand::PopClip)));
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
