//! MenuItem - Individual menu bar item
//!
//! MenuItem represents a single menu item in the menu bar (e.g., "File", "Edit").
//! When clicked, it can show a dropdown menu.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, RenderElement};
use crate::geometry::{Point, Rect, Size};
use crate::graphics;
use crate::icon::{Icon, IconSize, IconStyle};
use crate::renderer::PaintContext;
use crate::view::View;
use crate::views::style;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::any::Any;

/// Menu item click callback type
pub type MenuItemCallback = Box<dyn Fn() + 'static>;

/// MenuItem View - displays a clickable menu bar item
#[derive(Clone)]
pub struct MenuItem {
    label: String,
    icon: Option<Icon>,
    icon_size: IconSize,
    on_click: Option<Arc<dyn Fn() + 'static>>,
    on_hover: Option<Arc<dyn Fn() + 'static>>,
    font_size: f32,
    padding: f32,
    selected: bool,
}

impl MenuItem {
    /// Create a new MenuItem with the given label
    pub fn new(label: impl Into<String>) -> Self {
        let label_str = label.into();
        Self {
            label: label_str,
            icon: None,
            icon_size: IconSize::Small,
            on_click: None,
            on_hover: None,
            font_size: 14.0,
            padding: 6.0,
            selected: false,
        }
    }

    /// Add a standard ScarletUI icon to this menu item.
    ///
    /// # Arguments
    ///
    /// * `icon` - Tabler icon rendered before the label, or centered when the
    ///   label is empty.
    ///
    /// # Returns
    ///
    /// The updated menu item.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set the standard logical size of this menu item's icon.
    ///
    /// # Arguments
    ///
    /// * `size` - Standard or explicit icon size.
    ///
    /// # Returns
    ///
    /// The updated menu item.
    pub fn icon_size(mut self, size: IconSize) -> Self {
        self.icon_size = size;
        self
    }

    /// Set the click callback
    pub fn on_click(mut self, callback: impl Fn() + 'static) -> Self {
        self.on_click = Some(Arc::new(callback));
        self
    }

    /// Set the hover callback
    pub fn on_hover(mut self, callback: impl Fn() + 'static) -> Self {
        self.on_hover = Some(Arc::new(callback));
        self
    }

    /// Set the font size
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Set the padding
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Set the selected state
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Get the menu item label
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Get the optional icon displayed by this menu item.
    ///
    /// # Returns
    ///
    /// The configured icon, or `None` for a text-only item.
    pub fn get_icon(&self) -> Option<Icon> {
        self.icon
    }

    /// Get the configured logical icon size.
    ///
    /// # Returns
    ///
    /// The standard or explicit icon size used during layout and painting.
    pub fn get_icon_size(&self) -> IconSize {
        self.icon_size
    }

    /// Get the font size
    pub fn get_font_size(&self) -> f32 {
        self.font_size
    }

    /// Get the padding
    pub fn get_padding(&self) -> f32 {
        self.padding
    }

    /// Invoke the click callback if present
    pub fn invoke_on_click(&self) {
        if let Some(callback) = self.on_click.as_ref() {
            callback();
        }
    }

    /// Invoke the hover callback if present
    pub fn invoke_on_hover(&self) {
        if let Some(callback) = self.on_hover.as_ref() {
            callback();
        }
    }

    /// Get selected state
    pub fn is_selected(&self) -> bool {
        self.selected
    }
}

impl View for MenuItem {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            MenuItemRenderObject::new(
                self.label.clone(),
                self.font_size,
                self.padding,
                self.selected,
            )
            .with_icon(self.icon, self.icon_size),
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        alloc::vec::Vec::new()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// MenuItem RenderObject - handles menu item rendering and interaction
pub struct MenuItemRenderObject {
    label: String,
    icon: Option<Icon>,
    icon_size: IconSize,
    font_size: f32,
    padding: f32,
    selected: bool,
    hovered: bool,
    pressed: bool,
    size: Size,
    buffer: Option<Buffer>,
}

impl MenuItemRenderObject {
    /// Create a new MenuItemRenderObject
    pub fn new(label: String, font_size: f32, padding: f32, selected: bool) -> Self {
        Self {
            label,
            icon: None,
            icon_size: IconSize::Small,
            font_size,
            padding,
            selected,
            hovered: false,
            pressed: false,
            size: Size::ZERO,
            buffer: None,
        }
    }

    fn with_icon(mut self, icon: Option<Icon>, icon_size: IconSize) -> Self {
        self.icon = icon;
        self.icon_size = icon_size;
        self
    }

    /// Estimate menu item size based on label
    fn estimate_size(&self) -> Size {
        let (text_w, text_h) = if self.label.is_empty() {
            (0, 0)
        } else {
            graphics::measure_text_sized(&self.label, self.font_size)
        };
        let icon_size = self
            .icon
            .map(|_| self.icon_size.logical_pixels() as f32)
            .unwrap_or(0.0);
        let spacing = if self.icon.is_some() && !self.label.is_empty() {
            4.0
        } else {
            0.0
        };
        let width = text_w as f32 + icon_size + spacing + self.padding * 2.0;
        let height = (text_h as f32).max(icon_size) + self.padding * 2.0;

        Size { width, height }
    }

    pub fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered;
    }

    pub fn set_pressed(&mut self, pressed: bool) {
        self.pressed = pressed;
    }

    pub fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }

    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    fn current_background(&self) -> Color {
        let palette = ColorPalette::default();
        if self.pressed || self.selected {
            palette.menu_active()
        } else if self.hovered {
            palette.menu_hover()
        } else {
            Color::rgba(0.0, 0.0, 0.0, 0.0) // Transparent
        }
    }
}

impl ElementRenderObject for MenuItemRenderObject {
    fn layout(&mut self, constraints: crate::element::LayoutConstraints) -> Size {
        let intrinsic = self.estimate_size();

        if crate::debug::is_enabled() {
            crate::logln!(
                "[MenuItemRenderObject] layout: label='{}' intrinsic={:?}, constraints={:?}",
                self.label,
                intrinsic,
                constraints
            );
        }

        // For menu items, use the intrinsic size, but constrain within bounds
        let mut width = intrinsic.width;
        let mut height = intrinsic
            .height
            .max(style::metrics().minimum_control_height);

        // Apply min/max constraints
        if constraints.min_width.is_finite() && constraints.min_width > 0.0 {
            width = width.max(constraints.min_width);
        }
        if constraints.min_height.is_finite() && constraints.min_height > 0.0 {
            height = height.max(constraints.min_height);
        }
        if constraints.max_width.is_finite() && constraints.max_width > 0.0 {
            width = width.min(constraints.max_width);
        }
        if constraints.max_height.is_finite() && constraints.max_height > 0.0 {
            height = height.min(constraints.max_height);
        }

        self.size = Size { width, height };

        // Create buffer for this menu item
        let w = libm::ceilf(width) as u32;
        let h = libm::ceilf(height) as u32;

        if crate::debug::is_enabled() {
            crate::logln!(
                "[MenuItemRenderObject] layout: final size={}x{}, buffer needed={} bytes",
                w,
                h,
                w * h * 4
            );
        }

        let needs_resize = self
            .buffer
            .as_ref()
            .map_or(true, |b| b.logical_width() != w || b.logical_height() != h);
        if needs_resize {
            self.buffer = Some(Buffer::from_logical_dimensions(w, h));
        }

        self.size
    }

    fn update(&mut self, new_view: &dyn crate::view::View) -> crate::element::UpdateResult {
        if let Some(view) = new_view.as_any().downcast_ref::<MenuItem>() {
            self.label = view.label.clone();
            self.icon = view.icon;
            self.icon_size = view.icon_size;
            self.font_size = view.font_size;
            self.padding = view.padding;
            self.selected = view.selected;
            crate::element::UpdateResult::Updated
        } else {
            crate::element::UpdateResult::Replaced
        }
    }

    fn size(&self) -> Size {
        self.size
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        // Render menu item to buffer
        if crate::debug::is_enabled() {
            crate::logln!(
                "[MenuItemRenderObject] render: label='{}', buffer={}",
                self.label,
                self.buffer.is_some()
            );
        }
        let background = self.current_background();
        let palette = ColorPalette::default();
        let text_color = palette.text_primary();

        if let Some(ref mut buffer) = self.buffer {
            let mut canvas = graphics::Canvas::for_buffer(buffer);
            let width = canvas.width();
            let height = canvas.height();

            // Clear to avoid blending text on top of previous frames.
            canvas.fill_rect(0, 0, width, height, Color::TRANSPARENT);

            // Fill background (only if hovered or pressed)
            if self.hovered || self.pressed || self.selected {
                canvas.fill_rect(0, 0, width, height, background);
            }

            // Draw text centered
            let (text_w, _text_h) = graphics::measure_text_sized(&self.label, self.font_size);
            let x = ((width as i32) - (text_w as i32)) / 2;
            let y = ((height as i32) - (self.font_size as i32 * 6 / 5)) / 2;

            canvas.draw_text_sized(x.max(0), y.max(0), &self.label, text_color, self.font_size);
        }
    }

    fn get_buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    fn clear_buffer(&mut self) {
        self.buffer = None;
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let background = self.current_background();
        let palette = ColorPalette::default();
        let text_color = palette.text_primary();

        if self.hovered || self.pressed || self.selected {
            style::item_highlight(
                ctx,
                Rect::from_xywh(
                    origin.x + 2.0,
                    origin.y + 2.0,
                    (self.size.width - 4.0).max(0.0),
                    (self.size.height - 4.0).max(0.0),
                ),
                background,
            );
        }

        let (text_w, _text_h) = if self.label.is_empty() {
            (0, 0)
        } else {
            graphics::measure_text_sized(&self.label, self.font_size)
        };
        let icon_size = self
            .icon
            .map(|_| self.icon_size.logical_pixels() as f32)
            .unwrap_or(0.0);
        let spacing = if self.icon.is_some() && !self.label.is_empty() {
            4.0
        } else {
            0.0
        };
        let group_width = icon_size + spacing + text_w as f32;
        let group_x = origin.x + ((self.size.width - group_width) / 2.0).max(0.0);
        if let Some(icon) = self.icon {
            crate::views::icon::paint_icon(
                ctx,
                Point::new(
                    group_x,
                    origin.y + ((self.size.height - icon_size) / 2.0).max(0.0),
                ),
                icon_size,
                icon,
                IconStyle::default(),
                text_color,
            );
        }
        if !self.label.is_empty() {
            let text_x = group_x + icon_size + spacing;
            let text_y =
                origin.y + ((self.size.height - self.font_size * 1.2) / 2.0).max(0.0);
            ctx.draw_text(
                Point::new(text_x, text_y),
                self.label.clone(),
                text_color,
                self.font_size,
            );
        }
        true
    }
}
