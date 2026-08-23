//! Render objects used by `NavigationView`.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{
    Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult,
};
use crate::geometry::{Point, Rect, Size};
use crate::graphics;
use crate::icon::{Icon, IconStyle};
use crate::renderer::PaintContext;
use crate::state::State;
use crate::view::View;
use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::Cell;

/// Layout render object for the complete navigation view.
///
/// Sidebar painting and pointer interaction belong to the first child render
/// object. Keeping that region as a real element lets the event dispatcher's
/// hover-path diff generate `Entered` and `Exited` at the sidebar boundary.
pub struct NavigationViewRenderObject {
    sidebar_width: f32,
    header_height: f32,
    size: Size,
}

impl NavigationViewRenderObject {
    /// Create a navigation layout render object.
    ///
    /// # Arguments
    ///
    /// * `sidebar_width` - Fixed width reserved for the sidebar child.
    /// * `header_height` - Height reserved above the content child.
    ///
    /// # Returns
    ///
    /// A render object ready to lay out navigation children.
    pub fn new(sidebar_width: f32, header_height: f32) -> Self {
        Self {
            sidebar_width,
            header_height: header_height.max(0.0),
            size: Size::ZERO,
        }
    }

    /// Update navigation layout configuration.
    pub(crate) fn update_configuration(&mut self, sidebar_width: f32, header_height: f32) {
        self.sidebar_width = sidebar_width;
        self.header_height = header_height.max(0.0);
    }
}

impl ElementRenderObject for NavigationViewRenderObject {
    fn update_needs_layout(&self) -> bool {
        true
    }

    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = Size::new(
            constraints.min_width.min(constraints.max_width),
            constraints.min_height.min(constraints.max_height),
        );
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let has_header = self.header_height > 0.0;
        let content_index = if has_header { 2 } else { 1 };
        if children.len() != content_index + 1 {
            crate::logln!(
                "[NavigationViewRenderObject::layout] WARNING: expected {} children, got {}",
                content_index + 1,
                children.len()
            );
        }

        let height = constraints.max_height.max(0.0);
        if let Some(sidebar) = children.get_mut(0) {
            sidebar.layout(LayoutConstraints::tight(self.sidebar_width, height));
            sidebar.set_position(Point::ZERO);
        }

        let content_width = (constraints.max_width - self.sidebar_width).max(0.0);
        let header_height = if has_header {
            self.header_height.min(height)
        } else {
            0.0
        };
        if has_header && let Some(header) = children.get_mut(1) {
            header.layout(LayoutConstraints::tight(content_width, header_height));
            header.set_position(Point::new(self.sidebar_width, 0.0));
        }

        let content_height = (height - header_height).max(0.0);
        if let Some(content) = children.get_mut(content_index) {
            content.layout(LayoutConstraints::new(
                content_width,
                content_width,
                0.0,
                content_height,
            ));
            content.set_position(Point::new(self.sidebar_width, header_height));
        }

        self.size = Size::new(constraints.max_width, constraints.max_height);
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn render(&mut self) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Declarative child view for the interactive navigation sidebar.
#[derive(Clone)]
pub(crate) struct NavigationSidebar {
    labels: Vec<String>,
    icons: Vec<Option<Icon>>,
    selection_callbacks: Vec<Option<Rc<dyn Fn()>>>,
    selected_index: State<usize>,
    shows_icons: bool,
    icon_style: IconStyle,
    icon_color: Option<Color>,
}

impl NavigationSidebar {
    pub(crate) fn new(
        labels: Vec<String>,
        icons: Vec<Option<Icon>>,
        selection_callbacks: Vec<Option<Rc<dyn Fn()>>>,
        selected_index: State<usize>,
        shows_icons: bool,
        icon_style: IconStyle,
        icon_color: Option<Color>,
    ) -> Self {
        Self {
            labels,
            icons,
            selection_callbacks,
            selected_index,
            shows_icons,
            icon_style,
            icon_color,
        }
    }
}

impl View for NavigationSidebar {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new_with_builder(
            self.clone(),
            NavigationSidebarRenderObject::from_view,
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Paint and interaction render object for the sidebar region.
pub(crate) struct NavigationSidebarRenderObject {
    labels: Vec<String>,
    icons: Vec<Option<Icon>>,
    selection_callbacks: Vec<Option<Rc<dyn Fn()>>>,
    selected_index: State<usize>,
    shows_icons: bool,
    icon_style: IconStyle,
    icon_color: Option<Color>,
    hovered_index: Option<usize>,
    last_painted_hovered_index: Cell<Option<usize>>,
    item_height: f32,
    size: Size,
    buffer: Option<Buffer>,
    font_size: f32,
    icon_size: u32,
    item_padding: f32,
}

impl NavigationSidebarRenderObject {
    fn from_view(view: &NavigationSidebar) -> Self {
        Self {
            labels: view.labels.clone(),
            icons: view.icons.clone(),
            selection_callbacks: view.selection_callbacks.clone(),
            selected_index: view.selected_index.clone(),
            shows_icons: view.shows_icons,
            icon_style: view.icon_style,
            icon_color: view.icon_color,
            hovered_index: None,
            last_painted_hovered_index: Cell::new(None),
            item_height: 40.0,
            size: Size::ZERO,
            buffer: None,
            font_size: 14.0,
            icon_size: 16,
            item_padding: 8.0,
        }
    }

    pub(crate) fn hovered_index(&self) -> Option<usize> {
        self.hovered_index
    }

    pub(crate) fn set_hovered_index(&mut self, index: Option<usize>) {
        self.hovered_index = index;
    }

    pub(crate) fn index_at_point(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x >= self.size.width || y < 0.0 {
            return None;
        }
        let index = (y / self.item_height) as usize;
        (index < self.labels.len()).then_some(index)
    }

    pub(crate) fn selected_index(&self) -> &State<usize> {
        &self.selected_index
    }

    pub(crate) fn selection_callback(&self, index: usize) -> Option<Rc<dyn Fn()>> {
        self.selection_callbacks.get(index).cloned().flatten()
    }

    fn paint_commands(&self, ctx: &mut PaintContext<'_>, origin: Point) {
        let palette = ColorPalette::default();
        let width = self.size.width.max(0.0);
        let height = self.size.height.max(0.0);
        ctx.fill_rect(
            Rect::from_xywh(origin.x, origin.y, width, height),
            palette.background_secondary(),
        );

        let selected = self.selected_index.get();
        for index in 0..self.labels.len() {
            let y = index as f32 * self.item_height;
            let is_selected = selected == index;
            if self.hovered_index == Some(index) {
                ctx.fill_rect(
                    Rect::from_xywh(origin.x, origin.y + y, width, self.item_height),
                    palette.menu_hover(),
                );
            }
            if is_selected {
                ctx.fill_rect(
                    Rect::from_xywh(origin.x, origin.y + y, 3.0, self.item_height),
                    palette.primary(),
                );
            }

            let text_color = if is_selected {
                palette.primary()
            } else {
                palette.text()
            };
            let text_x = if self.shows_icons {
                if let Some(icon) = self.icons.get(index).copied().flatten() {
                    let icon_size = self.icon_size as f32;
                    crate::views::icon::paint_icon(
                        ctx,
                        Point::new(
                            origin.x + self.item_padding + 4.0,
                            origin.y + y + (self.item_height - icon_size) * 0.5,
                        ),
                        icon_size,
                        icon,
                        self.icon_style,
                        self.icon_color.unwrap_or(text_color),
                    );
                    origin.x + self.item_padding + icon_size + 10.0
                } else {
                    origin.x + self.item_padding + 8.0
                }
            } else {
                origin.x + self.item_padding + 8.0
            };
            let text_y = origin.y + y + (self.item_height - self.font_size * 1.2) / 2.0;
            ctx.draw_text(
                Point::new(text_x, text_y),
                self.labels[index].to_owned(),
                text_color,
                self.font_size,
            );
        }

        if width > 0.0 {
            ctx.fill_rect(
                Rect::from_xywh(origin.x + width - 1.0, origin.y, 1.0, height),
                palette.border(),
            );
        }
    }
}

impl ElementRenderObject for NavigationSidebarRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = Size::new(constraints.max_width, constraints.max_height);
        let width = libm::ceilf(self.size.width.max(0.0)) as u32;
        let height = libm::ceilf(self.size.height.max(0.0)) as u32;
        let needs_resize = self.buffer.as_ref().is_none_or(|buffer| {
            buffer.logical_width() != width || buffer.logical_height() != height
        });
        if needs_resize {
            self.buffer = Some(Buffer::from_logical_dimensions(width, height));
        }
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn self_paint_bounds(&self, origin: Point) -> Option<Rect> {
        let previous = self.last_painted_hovered_index.get();
        let current = self.hovered_index;
        let (first, last) = match (previous, current) {
            (Some(previous), Some(current)) => (previous.min(current), previous.max(current)),
            (Some(index), None) | (None, Some(index)) => (index, index),
            (None, None) => return Some(Rect::new(origin, self.size)),
        };
        Some(Rect::from_xywh(
            origin.x,
            origin.y + first as f32 * self.item_height,
            self.size.width.max(0.0),
            (last - first + 1) as f32 * self.item_height,
        ))
    }

    fn update(&mut self, new_view: &dyn View) -> UpdateResult {
        let Some(view) = new_view.as_any().downcast_ref::<NavigationSidebar>() else {
            return UpdateResult::Replaced;
        };
        self.labels = view.labels.clone();
        self.icons = view.icons.clone();
        self.selection_callbacks = view.selection_callbacks.clone();
        self.selected_index = view.selected_index.clone();
        self.shows_icons = view.shows_icons;
        self.icon_style = view.icon_style;
        self.icon_color = view.icon_color;
        self.hovered_index = self
            .hovered_index
            .filter(|index| *index < self.labels.len());
        UpdateResult::Updated
    }

    fn render(&mut self) {
        if let Some(buffer) = self.buffer.as_mut() {
            let mut canvas = graphics::Canvas::for_buffer(buffer);
            let palette = ColorPalette::default();
            let width = canvas.width();
            let height = canvas.height();
            canvas.fill_rect(0, 0, width, height, palette.background_secondary());
            let selected = self.selected_index.get();
            for index in 0..self.labels.len() {
                let y = (index as f32 * self.item_height) as i32;
                if self.hovered_index == Some(index) {
                    canvas.fill_rect(0, y, width, self.item_height as u32, palette.menu_hover());
                }
                if selected == index {
                    canvas.fill_rect(0, y, 3, self.item_height as u32, palette.primary());
                }
                let color = if selected == index {
                    palette.primary()
                } else {
                    palette.text()
                };
                canvas.draw_text_sized(
                    self.item_padding as i32 + 8,
                    y + (self.item_height as i32 - (self.font_size * 1.2) as i32) / 2,
                    self.labels.get(index).map(String::as_str).unwrap_or("Item"),
                    color,
                    self.font_size,
                );
            }
            if width > 0 {
                canvas.draw_line(
                    width as i32 - 1,
                    0,
                    width as i32 - 1,
                    height as i32,
                    palette.border(),
                );
            }
        }
        self.last_painted_hovered_index.set(self.hovered_index);
    }

    fn paint(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
        self.paint_commands(ctx, origin);
        self.last_painted_hovered_index.set(self.hovered_index);
        true
    }

    fn get_buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    fn clear_buffer(&mut self) {
        self.buffer = None;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
