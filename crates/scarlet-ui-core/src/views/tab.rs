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
use crate::renderer::PaintContext;
use crate::state::{Listenable, State};
use crate::view::View;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;

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
            tab_bar_height: 30.0,
            tab_padding: 14.0,
            font_size: 13.0,
            background_color: palette.background_secondary(),
            selected_color: palette.surface(),
            hover_color: palette.menu_hover(),
            border_color: palette.border(),
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
        Box::new(ComponentElement::new(TabViewContent { tabs: self.clone() }))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![self.selected_index_state() as &dyn Listenable]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
struct TabViewContent {
    tabs: TabView,
}

impl View for TabViewContent {
    fn create_element(&self) -> Box<dyn Element> {
        let child = self.tabs.active_content();
        let render_object = TabViewRenderObject::new(
            self.tabs.labels(),
            self.tabs.selected_index_state().clone(),
            self.tabs.tab_bar_height,
            self.tabs.tab_padding,
            self.tabs.font_size,
            self.tabs.background_color,
            self.tabs.selected_color,
            self.tabs.hover_color,
            self.tabs.border_color,
            self.tabs.text_color,
            self.tabs.selected_text_color,
        );

        Box::new(RenderElement::with_children(
            self.clone(),
            render_object,
            vec![child.create_element()],
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![self.tabs.selected_index_state() as &dyn Listenable]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object for [`TabView`].
pub struct TabViewRenderObject {
    labels: Vec<String>,
    selected_index: State<usize>,
    hovered_index: Option<usize>,
    tab_bar_height: f32,
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
    /// Create a tab view render object.
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
        Self {
            labels,
            selected_index,
            hovered_index: None,
            tab_bar_height,
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
        Rect::from_xywh(x, 0.0, width, self.tab_bar_height)
    }

    fn tab_index_at(&self, point: Point) -> Option<usize> {
        if point.y < 0.0 || point.y >= self.tab_bar_height {
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
            child.set_position(Point::new(0.0, self.tab_bar_height));
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
                let changed = self.hovered_index.is_some();
                self.hovered_index = None;
                changed
            }
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x,
                y,
                ..
            } => {
                if let Some(index) = self.tab_index_at(Point::new(x as f32, y as f32)) {
                    if self.selected_index.get() != index {
                        self.selected_index.set(index);
                    }
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        ctx.fill_rect(
            Rect::from_xywh(origin.x, origin.y, self.size.width, self.tab_bar_height),
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
            if index == selected {
                ctx.fill_rect(rect, self.selected_color);
            } else if self.hovered_index == Some(index) {
                ctx.fill_rect(rect, self.hover_color);
            }
            ctx.fill_rect(
                Rect::from_xywh(rect.origin.x, rect.bottom() - 1.0, rect.size.width, 1.0),
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
                origin.y + self.tab_bar_height - 1.0,
                self.size.width,
                1.0,
            ),
            self.border_color,
        );
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
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
}
