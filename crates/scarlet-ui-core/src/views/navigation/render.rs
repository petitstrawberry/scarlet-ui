//! NavigationViewRenderObject - Runtime rendering and event handling for NavigationView
//!
//! This module provides the RenderObject for NavigationView which handles:
//! - Sidebar rendering with selection highlights
use crate::buffer::Buffer;
use crate::color::Color;
use crate::color::ColorPalette;
use crate::element::{Element, ElementRenderObject, LayoutConstraints};
use crate::geometry::{Point, Rect, Size};
use crate::graphics;
use crate::icon::{Icon, IconStyle};
use crate::renderer::PaintContext;
use crate::state::State;
/// - Layout of sidebar and content areas
/// - Mouse event handling for item selection and hover
use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::Cell;
use libm;

/// NavigationView RenderObject - handles rendering and layout
///
/// This render object manages the navigation sidebar and content area layout.
pub struct NavigationViewRenderObject {
    /// Number of navigation links
    link_count: usize,
    /// Labels for each link
    labels: Vec<String>,
    /// Icons for each link
    icons: Vec<Option<Icon>>,
    /// Optional callbacks invoked when a sidebar item becomes selected.
    selection_callbacks: Vec<Option<Rc<dyn Fn()>>>,
    /// Currently selected link index
    selected_index: State<usize>,
    /// Width of the sidebar (fixed)
    sidebar_width: f32,
    /// Whether icons are rendered next to sidebar labels.
    shows_icons: bool,
    /// Shared rendering style for sidebar icons.
    icon_style: IconStyle,
    /// Optional shared tint override for sidebar icons.
    icon_color: Option<Color>,
    /// Height reserved for the optional content header.
    header_height: f32,
    /// Currently hovered link index (if any)
    hovered_index: Option<usize>,
    /// Hover state represented by the most recently emitted paint commands.
    last_painted_hovered_index: Cell<Option<usize>>,
    /// Height of each navigation item
    item_height: f32,
    /// Total size of the NavigationView
    size: Size,
    /// Buffer for rendering the sidebar
    buffer: Option<Buffer>,
    /// Font size for labels
    font_size: f32,
    /// Icon size
    icon_size: u32,
    /// Padding for items
    item_padding: f32,
}

impl NavigationViewRenderObject {
    /// Create a navigation render object.
    ///
    /// # Arguments
    ///
    /// * `labels` - Sidebar labels in display order.
    /// * `icons` - Optional typed icon for each label.
    /// * `selected_index` - Reactive selected-item index.
    /// * `sidebar_width` - Fixed sidebar width.
    /// * `shows_icons` - Whether configured icons are painted.
    /// * `icon_style` - Shared vector and stroke style for configured icons.
    /// * `icon_color` - Optional shared icon-only tint override.
    /// * `header_height` - Optional content header height.
    /// * `selection_callbacks` - Optional callbacks corresponding to links.
    ///
    /// # Returns
    ///
    /// A navigation render object ready for layout.
    pub fn new(
        labels: Vec<String>,
        icons: Vec<Option<Icon>>,
        selected_index: State<usize>,
        sidebar_width: f32,
        shows_icons: bool,
        icon_style: IconStyle,
        icon_color: Option<Color>,
        header_height: f32,
        selection_callbacks: Vec<Option<Rc<dyn Fn()>>>,
    ) -> Self {
        let link_count = labels.len();
        Self {
            link_count,
            labels,
            icons,
            selection_callbacks,
            selected_index,
            sidebar_width,
            shows_icons,
            icon_style,
            icon_color,
            header_height: header_height.max(0.0),
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

    /// Update declarative navigation configuration without resetting hover state.
    pub(crate) fn update_configuration(
        &mut self,
        labels: Vec<String>,
        icons: Vec<Option<Icon>>,
        selected_index: State<usize>,
        sidebar_width: f32,
        shows_icons: bool,
        icon_style: IconStyle,
        icon_color: Option<Color>,
        header_height: f32,
        selection_callbacks: Vec<Option<Rc<dyn Fn()>>>,
    ) {
        self.link_count = labels.len();
        self.labels = labels;
        self.icons = icons;
        self.selection_callbacks = selection_callbacks;
        self.selected_index = selected_index;
        self.sidebar_width = sidebar_width;
        self.shows_icons = shows_icons;
        self.icon_style = icon_style;
        self.icon_color = icon_color;
        self.header_height = header_height.max(0.0);
        self.hovered_index = self.hovered_index.filter(|index| *index < self.link_count);
    }

    /// Get the currently hovered index
    pub fn hovered_index(&self) -> Option<usize> {
        self.hovered_index
    }

    /// Set the hovered index
    pub fn set_hovered_index(&mut self, index: Option<usize>) {
        self.hovered_index = index;
    }

    /// Get the selected index state reference
    pub fn selected_index(&self) -> &State<usize> {
        &self.selected_index
    }

    /// Get the sidebar width
    pub fn sidebar_width(&self) -> f32 {
        self.sidebar_width
    }

    /// Calculate the Y position for a given item index
    pub fn item_y(&self, index: usize) -> f32 {
        index as f32 * self.item_height
    }

    /// Get the index for a given Y position
    pub fn index_at_y(&self, y: f32) -> Option<usize> {
        if y >= 0.0 && y < self.link_count as f32 * self.item_height {
            Some((y / self.item_height) as usize)
        } else {
            None
        }
    }

    /// Get the number of links
    pub fn link_count(&self) -> usize {
        self.link_count
    }

    /// Clone the callback associated with a navigation item, if any.
    pub fn selection_callback(&self, index: usize) -> Option<Rc<dyn Fn()>> {
        self.selection_callbacks.get(index).cloned().flatten()
    }

    /// Render a single navigation item
    #[allow(dead_code)]
    fn render_item(
        &self,
        canvas: &mut graphics::Canvas,
        y: i32,
        label: &str,
        is_selected: bool,
        is_hovered: bool,
    ) {
        let width = self.sidebar_width as i32;
        let height = self.item_height as i32;

        // Background
        let background_color = if is_selected {
            Color {
                r: 0.2,
                g: 0.4,
                b: 0.8,
                a: 1.0,
            } // Blue for selected
        } else if is_hovered {
            Color {
                r: 0.9,
                g: 0.9,
                b: 0.9,
                a: 1.0,
            } // Light gray for hover
        } else {
            Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            } // White for normal
        };

        canvas.fill_rect(0, y, width as u32, height as u32, background_color);

        // Label
        let text_color = if is_selected {
            Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            }
        } else {
            Color {
                r: 0.2,
                g: 0.2,
                b: 0.2,
                a: 1.0,
            }
        };

        let text_x = (self.item_padding) as i32 + 8;
        let text_y = y + (height - (self.font_size * 1.2) as i32) / 2;
        canvas.draw_text_sized(text_x, text_y, label, text_color, self.font_size);

        // Bottom border
        let border_color = Color {
            r: 0.85,
            g: 0.85,
            b: 0.85,
            a: 1.0,
        };
        canvas.draw_line(0, y + height - 1, width, y + height - 1, border_color);
    }
}

impl ElementRenderObject for NavigationViewRenderObject {
    fn update_needs_layout(&self) -> bool {
        true
    }

    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = Size {
            width: constraints.min_width.min(constraints.max_width),
            height: constraints.min_height.min(constraints.max_height),
        };
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        if crate::debug::is_enabled() {
            crate::logln!(
                "[NavigationViewRenderObject::layout] constraints=({:?}, {:?}) -> ({:?}, {:?})",
                constraints.min_width,
                constraints.min_height,
                constraints.max_width,
                constraints.max_height
            );
        }

        let has_header = self.header_height > 0.0;
        let content_index = if has_header { 2 } else { 1 };

        // Children are sidebar, optional header, and content.
        if children.len() != content_index + 1 {
            crate::logln!(
                "[NavigationViewRenderObject::layout] WARNING: Expected {} children, got {}",
                content_index + 1,
                children.len()
            );
        }

        // Layout sidebar (child 0) with fixed width
        let sidebar_constraints =
            LayoutConstraints::tight(self.sidebar_width, constraints.max_height);
        let sidebar_height = if let Some(sidebar) = children.get_mut(0) {
            sidebar.layout(sidebar_constraints)
        } else {
            Size::new(self.sidebar_width, constraints.max_height)
        };

        let content_width = (constraints.max_width - self.sidebar_width).max(0.0);
        let header_height = if has_header {
            self.header_height.min(constraints.max_height.max(0.0))
        } else {
            0.0
        };

        if has_header {
            if let Some(header) = children.get_mut(1) {
                header.layout(LayoutConstraints::tight(content_width, header_height));
                header.set_position(Point::new(self.sidebar_width, 0.0));
            }
        }

        // Layout content with the remaining height below the header.
        let content_height = (constraints.max_height - header_height).max(0.0);
        let content_constraints =
            LayoutConstraints::new(content_width, content_width, 0.0, content_height);
        let _content_height = if let Some(content) = children.get_mut(content_index) {
            content.layout(content_constraints)
        } else {
            Size::new(content_width, content_height)
        };

        // Position sidebar at (0, 0)
        if let Some(sidebar) = children.get_mut(0) {
            sidebar.set_position(Point::ZERO);
        }

        // Position content below the optional header.
        if let Some(content) = children.get_mut(content_index) {
            content.set_position(Point::new(self.sidebar_width, header_height));
        }

        // Total size is the full constraint
        self.size = Size::new(constraints.max_width, constraints.max_height);

        // Create buffer for sidebar only
        let sidebar_height_px = libm::ceilf(sidebar_height.height) as u32;
        let sidebar_width_px = libm::ceilf(self.sidebar_width) as u32;

        if crate::debug::is_enabled() {
            crate::logln!(
                "[NavigationViewRenderObject::layout] sidebar size={}x{}, buffer needed={} bytes",
                sidebar_width_px,
                sidebar_height_px,
                sidebar_width_px * sidebar_height_px * 4
            );
        }

        let needs_resize = self.buffer.as_ref().map_or(true, |b| {
            b.logical_width() != sidebar_width_px || b.logical_height() != sidebar_height_px
        });

        if needs_resize {
            self.buffer = Some(Buffer::from_logical_dimensions(
                sidebar_width_px,
                sidebar_height_px,
            ));
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
            (None, None) => {
                return Some(Rect::from_xywh(
                    origin.x,
                    origin.y,
                    self.sidebar_width.max(0.0),
                    self.size.height.max(0.0),
                ));
            }
        };
        let y = first as f32 * self.item_height;
        let height = (last - first + 1) as f32 * self.item_height;
        Some(Rect::from_xywh(
            origin.x,
            origin.y + y,
            self.sidebar_width.max(0.0),
            height,
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        if crate::debug::is_enabled() {
            crate::logln!(
                "[NavigationViewRenderObject::render] buffer={}",
                self.buffer.is_some()
            );
        }

        if let Some(ref mut buffer) = self.buffer {
            let mut canvas = graphics::Canvas::for_buffer(buffer);
            let width = canvas.width();
            let height = canvas.height();

            // Fill background (lighter, semi-transparent look)
            let palette = ColorPalette::default();
            let bg_color = palette.background_secondary();
            canvas.fill_rect(0, 0, width, height, bg_color);

            // Clone values we need before the loop to avoid borrow issues
            let link_count = self.link_count;
            let selected = self.selected_index.get();
            let item_height = self.item_height;
            let font_size = self.font_size;
            let item_padding = self.item_padding;

            for i in 0..link_count {
                let y = (i as f32 * item_height) as i32;
                let is_selected = selected == i;

                // Get the label for this item.
                let label = self.labels.get(i).map(|s| s.as_str()).unwrap_or("Item");

                let height_px = item_height as i32;

                // Draw underline for selected state (left indicator)
                if is_selected {
                    let indicator_width = 3.0;
                    let indicator_x = 0;
                    let indicator_y = y;
                    let indicator_height = height_px;
                    canvas.fill_rect(
                        indicator_x,
                        indicator_y,
                        indicator_width as u32,
                        indicator_height as u32,
                        palette.primary(),
                    );
                }

                // Label
                let text_color = if is_selected {
                    palette.primary()
                } else {
                    palette.text()
                };

                let text_x = item_padding as i32 + 8;
                let text_y = y + (height_px - (font_size * 1.2) as i32) / 2;
                canvas.draw_text_sized(text_x, text_y, label, text_color, font_size);
            }

            // Draw separator line on the right edge
            let border_color = palette.border();
            canvas.draw_line(
                width as i32 - 1,
                0,
                width as i32 - 1,
                height as i32,
                border_color,
            );
        }
    }

    fn get_buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    fn clear_buffer(&mut self) {
        self.buffer = None;
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let palette = ColorPalette::default();
        let sidebar_height = self.size.height.max(0.0);
        let sidebar_width = self.sidebar_width.max(0.0);

        ctx.fill_rect(
            Rect::from_xywh(origin.x, origin.y, sidebar_width, sidebar_height),
            palette.background_secondary(),
        );

        let selected = self.selected_index.get();
        for i in 0..self.link_count {
            let y = i as f32 * self.item_height;
            let is_selected = selected == i;
            let is_hovered = self.hovered_index == Some(i);
            let label = self.labels.get(i).map(|s| s.as_str()).unwrap_or("Item");

            if is_hovered {
                ctx.fill_rect(
                    Rect::from_xywh(origin.x, origin.y + y, sidebar_width, self.item_height),
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
                if let Some(icon) = self.icons.get(i).copied().flatten() {
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
                label.to_owned(),
                text_color,
                self.font_size,
            );
        }

        ctx.fill_rect(
            Rect::from_xywh(
                origin.x + sidebar_width - 1.0,
                origin.y,
                1.0,
                sidebar_height,
            ),
            palette.border(),
        );
        self.last_painted_hovered_index.set(self.hovered_index);
        true
    }

    fn hit_test(&self, point: Point) -> bool {
        // Check if point is within sidebar bounds
        if point.x >= 0.0
            && point.x <= self.sidebar_width
            && point.y >= 0.0
            && point.y <= self.size.height
        {
            true
        } else {
            false
        }
    }
}
