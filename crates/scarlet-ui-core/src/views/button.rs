//! Button View - Interactive button with callback
//!
//! Button displays a label and triggers an action when clicked.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, RenderElement};
use crate::geometry::{Point, Rect, Size};
use crate::graphics;
use crate::icon::{Icon, IconFill, IconStyle, IconWeight};
use crate::renderer::PaintContext;
use crate::view::View;
use crate::views::style;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::any::Any;

/// Button click callback type
pub type ButtonCallback = Box<dyn Fn() + 'static>;

/// Button View - displays a clickable button
#[derive(Clone)]
pub struct Button {
    label: String,
    icon: Option<Icon>,
    icon_style: IconStyle,
    icon_color: Option<Color>,
    on_click: Option<Arc<dyn Fn() + 'static>>,
    background_color: Color,
    border_color: Color,
    text_color: Color,
    font_size: f32,
    padding: f32,
}

impl Button {
    /// Create a new Button with the given label
    pub fn new(label: impl Into<String>) -> Self {
        let label_str = label.into();
        let palette = ColorPalette::default();
        Self {
            label: label_str,
            icon: None,
            icon_style: IconStyle::default(),
            icon_color: None,
            on_click: None,
            background_color: palette.button_background(),
            border_color: Color::CLEAR,
            text_color: palette.text_primary(),
            font_size: 15.0,
            padding: 4.0,
        }
    }

    /// Create a compact icon-only button.
    ///
    /// # Arguments
    ///
    /// * `icon` - Icon painted inside the button.
    ///
    /// # Returns
    ///
    /// A button sized for application toolbars and header bars.
    pub fn icon_only(icon: Icon) -> Self {
        let palette = ColorPalette::default();
        Self {
            label: String::new(),
            icon: Some(icon),
            icon_style: IconStyle::default(),
            icon_color: None,
            on_click: None,
            background_color: palette.button_background(),
            border_color: Color::CLEAR,
            text_color: palette.text_primary(),
            font_size: 15.0,
            padding: 7.0,
        }
    }

    /// Add an icon to this button.
    ///
    /// # Arguments
    ///
    /// * `icon` - Icon painted before the label.
    ///
    /// # Returns
    ///
    /// The updated button.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set the outline style used by this button's icon.
    ///
    /// # Arguments
    ///
    /// * `style` - Icon stroke style.
    ///
    /// # Returns
    ///
    /// The updated button.
    pub fn icon_style(mut self, style: IconStyle) -> Self {
        self.icon_style = style;
        self
    }

    /// Set the stroke width used by this button's icon.
    ///
    /// # Arguments
    ///
    /// * `width` - Stroke width in Tabler view-box units.
    ///
    /// # Returns
    ///
    /// The updated button.
    pub fn icon_stroke_width(mut self, width: f32) -> Self {
        self.icon_style = self.icon_style.stroke_width(width);
        self
    }

    /// Set a semantic weight for this button's icon.
    ///
    /// # Arguments
    ///
    /// * `weight` - Thin, normal, or bold stroke weight.
    ///
    /// # Returns
    ///
    /// The updated button.
    pub fn icon_weight(mut self, weight: IconWeight) -> Self {
        self.icon_style = self.icon_style.weight(weight);
        self
    }

    /// Select outline or filled treatment for this button's icon.
    ///
    /// # Arguments
    ///
    /// * `fill` - Requested vector treatment.
    ///
    /// # Returns
    ///
    /// The updated button.
    pub fn icon_fill(mut self, fill: IconFill) -> Self {
        self.icon_style = self.icon_style.fill(fill);
        self
    }

    /// Use the official filled variant for this button's icon when available.
    ///
    /// # Returns
    ///
    /// The updated button.
    pub fn icon_filled(self) -> Self {
        self.icon_fill(IconFill::Filled)
    }

    /// Override the icon tint independently from the label color.
    ///
    /// # Arguments
    ///
    /// * `color` - Explicit icon tint.
    ///
    /// # Returns
    ///
    /// The updated button.
    pub fn icon_color(mut self, color: Color) -> Self {
        self.icon_color = Some(color);
        self
    }

    /// Apply the compact flat appearance used by header bars.
    ///
    /// # Returns
    ///
    /// The updated button with transparent surface chrome and compact padding.
    pub fn header_style(mut self) -> Self {
        let palette = ColorPalette::default();
        self.background_color = Color::CLEAR;
        self.border_color = Color::CLEAR;
        self.text_color = palette.text();
        self.padding = 6.0;
        self
    }

    /// Set the click callback
    pub fn on_click(mut self, callback: impl Fn() + 'static) -> Self {
        self.on_click = Some(Arc::new(callback));
        self
    }

    /// Set the background color
    pub fn background_color(mut self, color: Color) -> Self {
        self.background_color = color;
        self
    }

    /// Set the text color
    pub fn text_color(mut self, color: Color) -> Self {
        self.text_color = color;
        self
    }

    /// Set the border color
    pub fn border_color(mut self, color: Color) -> Self {
        self.border_color = color;
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

    /// Get the button label
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Get the background color
    pub fn get_background_color(&self) -> Color {
        self.background_color
    }

    /// Get the text color
    pub fn get_text_color(&self) -> Color {
        self.text_color
    }

    /// Get the border color
    pub fn get_border_color(&self) -> Color {
        self.border_color
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
}

impl View for Button {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            ButtonRenderObject::new(
                self.label.clone(),
                self.icon,
                self.icon_style,
                self.icon_color,
                self.background_color,
                self.border_color,
                self.text_color,
                self.font_size,
                self.padding,
            ),
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        alloc::vec::Vec::new()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Button RenderObject - handles button rendering and interaction
pub struct ButtonRenderObject {
    label: String,
    icon: Option<Icon>,
    icon_style: IconStyle,
    icon_color: Option<Color>,
    background_color: Color,
    border_color: Color,
    text_color: Color,
    font_size: f32,
    padding: f32,
    hovered: bool,
    pressed: bool,
    size: Size,
    buffer: Option<Buffer>,
}

impl ButtonRenderObject {
    /// Create a button render object.
    ///
    /// # Arguments
    ///
    /// * `label` - Button label.
    /// * `icon` - Optional typed icon.
    /// * `icon_style` - Icon vector and stroke style.
    /// * `icon_color` - Optional icon-only tint override.
    /// * `background_color` - Normal background color.
    /// * `border_color` - Normal border color.
    /// * `text_color` - Text and icon foreground color.
    /// * `font_size` - Label font size.
    /// * `padding` - Interior padding.
    ///
    /// # Returns
    ///
    /// A render object initialized in its normal interaction state.
    pub fn new(
        label: String,
        icon: Option<Icon>,
        icon_style: IconStyle,
        icon_color: Option<Color>,
        background_color: Color,
        border_color: Color,
        text_color: Color,
        font_size: f32,
        padding: f32,
    ) -> Self {
        Self {
            label,
            icon,
            icon_style,
            icon_color,
            background_color,
            border_color,
            text_color,
            font_size,
            padding,
            hovered: false,
            pressed: false,
            size: Size::ZERO,
            buffer: None,
        }
    }

    /// Estimate button size based on label
    fn estimate_size(&self) -> Size {
        if self.icon.is_some() && self.label.is_empty() {
            let side = self.font_size * 1.35 + self.padding * 2.0;
            return Size::new(side, side);
        }
        let char_width = self.font_size * 0.6;

        let text_width = self.label.len() as f32 * char_width;
        let icon_width = if self.icon.is_some() {
            self.font_size * 1.1 + 6.0
        } else {
            0.0
        };
        let width = text_width + icon_width + self.padding * 2.0;
        let height = self.font_size * 1.2 + self.padding * 2.0;

        Size { width, height }
    }

    pub fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered;
    }

    pub fn set_pressed(&mut self, pressed: bool) {
        self.pressed = pressed;
    }

    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    fn shade_color(color: Color, factor: f32) -> Color {
        let clamp = |v: f32| v.clamp(0.0, 1.0);
        Color {
            r: clamp(color.r * factor),
            g: clamp(color.g * factor),
            b: clamp(color.b * factor),
            a: color.a,
        }
    }

    fn current_background(&self) -> Color {
        if self.pressed {
            Self::shade_color(self.background_color, 0.92)
        } else if self.hovered {
            Self::shade_color(self.background_color, 0.97)
        } else {
            self.background_color
        }
    }

    fn current_border(&self) -> Color {
        if self.pressed {
            Self::shade_color(self.border_color, 0.90)
        } else if self.hovered {
            Self::shade_color(self.border_color, 0.96)
        } else {
            self.border_color
        }
    }
}

impl ElementRenderObject for ButtonRenderObject {
    fn layout(&mut self, constraints: crate::element::LayoutConstraints) -> Size {
        let intrinsic = self.estimate_size();

        if crate::debug::is_enabled() {
            crate::logln!(
                "[ButtonRenderObject] layout: label='{}' intrinsic={:?}, constraints={:?}",
                self.label,
                intrinsic,
                constraints
            );
        }

        // For buttons, use the intrinsic size, but constrain within bounds
        // Buttons should NOT expand to fill min_width/min_height
        let mut width = intrinsic.width;
        let mut height = intrinsic
            .height
            .max(style::metrics().minimum_control_height);

        // Apply max constraints (don't exceed maximum)
        if constraints.max_width.is_finite() && constraints.max_width > 0.0 {
            width = width.min(constraints.max_width);
        }
        if constraints.max_height.is_finite() && constraints.max_height > 0.0 {
            height = height.min(constraints.max_height);
        }

        // Don't expand to min - buttons should stay at their intrinsic size

        self.size = Size { width, height };

        // Create buffer for this button
        let w = libm::ceilf(width) as u32;
        let h = libm::ceilf(height) as u32;

        if crate::debug::is_enabled() {
            crate::logln!(
                "[ButtonRenderObject] layout: final size={}x{}, buffer needed={} bytes",
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
        // Render button to buffer
        if crate::debug::is_enabled() {
            crate::logln!(
                "[ButtonRenderObject] render: label='{}', buffer={}",
                self.label,
                self.buffer.is_some()
            );
        }
        let background = self.current_background();
        let border = self.current_border();
        if let Some(ref mut buffer) = self.buffer {
            let mut canvas = graphics::Canvas::for_buffer(buffer);
            let width = canvas.width();
            let height = canvas.height();

            // Fill background
            canvas.fill_rect(0, 0, width, height, background);

            // Border
            canvas.draw_rect(0, 0, width, height, border);

            // Draw text centered
            let (text_w, _text_h) = graphics::measure_text_sized(&self.label, self.font_size);
            let x = ((width as i32) - (text_w as i32)) / 2;
            let y = ((height as i32) - (self.font_size as i32 * 6 / 5)) / 2;

            canvas.draw_text_sized(
                x.max(0),
                y.max(0),
                &self.label,
                self.text_color,
                self.font_size,
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
        let rect = Rect::new(origin, self.size);
        let background = self.current_background();
        let border = self.current_border();

        style::control_surface(ctx, rect, background, border);

        if let Some(icon) = self.icon {
            let icon_size = if self.label.is_empty() {
                (self.size.width.min(self.size.height) - self.padding).max(1.0)
            } else {
                (self.size.height - self.padding).max(1.0)
            };
            let (text_w, _text_h) = graphics::measure_text_sized(&self.label, self.font_size);
            let group_width = if self.label.is_empty() {
                icon_size
            } else {
                icon_size + 6.0 + text_w as f32
            };
            let group_x = origin.x + ((self.size.width - group_width) / 2.0).max(0.0);
            crate::views::icon::paint_icon(
                ctx,
                Point::new(group_x, origin.y + (self.size.height - icon_size) * 0.5),
                icon_size,
                icon,
                self.icon_style,
                self.icon_color.unwrap_or(self.text_color),
            );
            if !self.label.is_empty() {
                ctx.draw_text(
                    Point::new(
                        group_x + icon_size + 6.0,
                        origin.y + ((self.size.height - self.font_size * 1.2) / 2.0).max(0.0),
                    ),
                    self.label.clone(),
                    self.text_color,
                    self.font_size,
                );
            }
        } else {
            let (text_w, _text_h) = graphics::measure_text_sized(&self.label, self.font_size);
            let x = origin.x + ((self.size.width - text_w as f32) / 2.0).max(0.0);
            let y = origin.y + ((self.size.height - self.font_size * 1.2) / 2.0).max(0.0);
            ctx.draw_text(
                Point::new(x, y),
                self.label.clone(),
                self.text_color,
                self.font_size,
            );
        }
        true
    }

    fn update(&mut self, new_view: &dyn View) -> crate::element::UpdateResult {
        let Some(button) = new_view.as_any().downcast_ref::<Button>() else {
            return crate::element::UpdateResult::Replaced;
        };
        self.label = button.label.clone();
        self.icon = button.icon;
        self.icon_style = button.icon_style;
        self.icon_color = button.icon_color;
        self.background_color = button.background_color;
        self.border_color = button.border_color;
        self.text_color = button.text_color;
        self.font_size = button.font_size;
        self.padding = button.padding;
        crate::element::UpdateResult::Updated
    }

    fn update_needs_layout(&self) -> bool {
        true
    }
}
