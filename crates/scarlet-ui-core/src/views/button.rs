//! Button View - Interactive button with callback
//!
//! Button displays a label and triggers an action when clicked.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, RenderElement};
use crate::geometry::{Point, Rect, Size};
use crate::graphics;
use crate::icon::{Icon, IconFill, IconSize, IconStyle, IconWeight};
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
    icon_size: Option<IconSize>,
    icon_style: IconStyle,
    icon_color: Option<Color>,
    on_click: Option<Arc<dyn Fn() + 'static>>,
    background_color: Color,
    hover_background_color: Option<Color>,
    pressed_background_color: Option<Color>,
    border_color: Color,
    text_color: Color,
    font_size: f32,
    padding: f32,
    appearance: ButtonAppearance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ButtonAppearance {
    Raised,
    Header,
}

impl Button {
    /// Create a new Button with the given label
    pub fn new(label: impl Into<String>) -> Self {
        let label_str = label.into();
        let palette = ColorPalette::default();
        Self {
            label: label_str,
            icon: None,
            icon_size: None,
            icon_style: IconStyle::default(),
            icon_color: None,
            on_click: None,
            background_color: palette.button_background(),
            hover_background_color: None,
            pressed_background_color: None,
            border_color: Color::CLEAR,
            text_color: palette.text_primary(),
            font_size: 15.0,
            padding: 4.0,
            appearance: ButtonAppearance::Raised,
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
            icon_size: None,
            icon_style: IconStyle::default(),
            icon_color: None,
            on_click: None,
            background_color: palette.button_background(),
            hover_background_color: None,
            pressed_background_color: None,
            border_color: Color::CLEAR,
            text_color: palette.text_primary(),
            font_size: 15.0,
            padding: 7.0,
            appearance: ButtonAppearance::Raised,
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

    /// Set the standard logical size used to paint this button's icon.
    ///
    /// # Arguments
    ///
    /// * `size` - Standard or explicit icon size.
    ///
    /// # Returns
    ///
    /// The updated button. The icon remains constrained by the button's
    /// available content area.
    pub fn icon_size(mut self, size: IconSize) -> Self {
        self.icon_size = Some(size);
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
    /// The updated button with transparent rest-state chrome, semantic rounded
    /// hover and pressed fills, and compact padding.
    pub fn header_style(mut self) -> Self {
        let palette = ColorPalette::default();
        self.background_color = Color::CLEAR;
        self.hover_background_color = Some(palette.header_button_hover());
        self.pressed_background_color = Some(palette.header_button_pressed());
        self.border_color = Color::CLEAR;
        self.text_color = palette.text();
        self.padding = 6.0;
        self.appearance = ButtonAppearance::Header;
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

    /// Get the optional explicit icon size.
    ///
    /// # Returns
    ///
    /// The configured size, or `None` when the button derives its legacy icon
    /// size from the available content area.
    pub fn get_icon_size(&self) -> Option<IconSize> {
        self.icon_size
    }

    /// Invoke the click callback if present
    pub fn invoke_on_click(&self) {
        if let Some(callback) = self.on_click.as_ref() {
            callback();
        }
    }

    fn build_render_object(&self) -> ButtonRenderObject {
        let mut render_object = ButtonRenderObject::new(
            self.label.clone(),
            self.icon,
            self.icon_style,
            self.icon_color,
            self.background_color,
            self.border_color,
            self.text_color,
            self.font_size,
            self.padding,
        );
        render_object.icon_size = self.icon_size;
        render_object.hover_background_color = self.hover_background_color;
        render_object.pressed_background_color = self.pressed_background_color;
        render_object.appearance = self.appearance;
        render_object
    }
}

impl View for Button {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(self.clone(), self.build_render_object()))
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
    icon_size: Option<IconSize>,
    icon_style: IconStyle,
    icon_color: Option<Color>,
    background_color: Color,
    hover_background_color: Option<Color>,
    pressed_background_color: Option<Color>,
    border_color: Color,
    text_color: Color,
    font_size: f32,
    padding: f32,
    appearance: ButtonAppearance,
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
            icon_size: None,
            icon_style,
            icon_color,
            background_color,
            hover_background_color: None,
            pressed_background_color: None,
            border_color,
            text_color,
            font_size,
            padding,
            appearance: ButtonAppearance::Raised,
            hovered: false,
            pressed: false,
            size: Size::ZERO,
            buffer: None,
        }
    }

    /// Estimate button size based on label
    fn estimate_size(&self) -> Size {
        if self.icon.is_some() && self.label.is_empty() {
            let icon_size = self
                .icon_size
                .map(|size| size.logical_pixels() as f32)
                .unwrap_or(self.font_size * 1.35);
            let side = icon_size + self.padding * 2.0;
            return Size::new(side, side);
        }
        let char_width = self.font_size * 0.6;

        let text_width = self.label.len() as f32 * char_width;
        let icon_width = if self.icon.is_some() {
            self.icon_size
                .map(|size| size.logical_pixels() as f32)
                .unwrap_or(self.font_size * 1.1)
                + 6.0
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
            self.pressed_background_color
                .unwrap_or_else(|| Self::shade_color(self.background_color, 0.92))
        } else if self.hovered {
            self.hover_background_color
                .unwrap_or_else(|| Self::shade_color(self.background_color, 0.97))
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

            match self.appearance {
                ButtonAppearance::Raised => {
                    // Fill background
                    canvas.fill_rect(0, 0, width, height, background);

                    // Border
                    canvas.draw_rect(0, 0, width, height, border);
                }
                ButtonAppearance::Header => {
                    canvas.fill_rect(0, 0, width, height, Color::CLEAR);
                    canvas_fill_rounded_rect(
                        &mut canvas,
                        width,
                        height,
                        style::radius_for(
                            Rect::from_xywh(0.0, 0.0, width as f32, height as f32),
                            style::metrics().control_radius,
                        ),
                        background,
                    );
                }
            }

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

        match self.appearance {
            ButtonAppearance::Raised => {
                style::raised_control_surface(ctx, rect, background, border, self.pressed);
            }
            ButtonAppearance::Header => style::fill_control(ctx, rect, background),
        }

        if let Some(icon) = self.icon {
            let available_icon_size = if self.label.is_empty() {
                (self.size.width.min(self.size.height) - self.padding).max(1.0)
            } else {
                (self.size.height - self.padding).max(1.0)
            };
            let icon_size = self
                .icon_size
                .map(|size| size.logical_pixels() as f32)
                .unwrap_or(available_icon_size)
                .min(available_icon_size);
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
        self.icon_size = button.icon_size;
        self.icon_style = button.icon_style;
        self.icon_color = button.icon_color;
        self.background_color = button.background_color;
        self.hover_background_color = button.hover_background_color;
        self.pressed_background_color = button.pressed_background_color;
        self.border_color = button.border_color;
        self.text_color = button.text_color;
        self.font_size = button.font_size;
        self.padding = button.padding;
        self.appearance = button.appearance;
        crate::element::UpdateResult::Updated
    }

    fn update_needs_layout(&self) -> bool {
        true
    }
}

fn canvas_fill_rounded_rect(
    canvas: &mut graphics::Canvas<'_>,
    width: u32,
    height: u32,
    radius: f32,
    color: Color,
) {
    if width == 0 || height == 0 || color.a <= 0.0 {
        return;
    }

    let radius = radius
        .max(0.0)
        .min(width as f32 * 0.5)
        .min(height as f32 * 0.5);
    for row in 0..height {
        for column in 0..width {
            let point_x = column as f32 + 0.5;
            let point_y = row as f32 + 0.5;
            let nearest_x = point_x.clamp(radius, width as f32 - radius);
            let nearest_y = point_y.clamp(radius, height as f32 - radius);
            let dx = point_x - nearest_x;
            let dy = point_y - nearest_y;
            if dx * dx + dy * dy <= radius * radius {
                canvas.put_pixel(column as i32, row as i32, color);
            }
        }
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::element::{ElementRenderObject, LayoutConstraints, UpdateResult};
    use crate::renderer::PaintCommand;

    fn laid_out(button: Button) -> ButtonRenderObject {
        let mut render_object = button.build_render_object();
        render_object.layout(LayoutConstraints::unconstrained());
        render_object
    }

    fn center_pixel(render_object: &ButtonRenderObject) -> u32 {
        let buffer = render_object
            .get_buffer()
            .expect("button buffer should exist");
        let x = buffer.width() / 2;
        let y = buffer.height() / 2;
        buffer.as_slice()[(y * buffer.width() + x) as usize]
    }

    #[test]
    fn header_style_uses_transparent_rest_and_distinct_semantic_state_fills() {
        let mut render_object = laid_out(Button::new("").header_style());
        let palette = ColorPalette::default();

        assert_eq!(render_object.current_background(), Color::CLEAR);
        render_object.render();
        assert_eq!(center_pixel(&render_object), Color::CLEAR.to_bgra());

        render_object.set_hovered(true);
        assert_eq!(
            render_object.current_background(),
            palette.header_button_hover()
        );
        assert!(render_object.current_background().a > 0.0);
        render_object.render();
        assert_eq!(
            center_pixel(&render_object),
            palette.header_button_hover().to_bgra()
        );

        render_object.set_pressed(true);
        assert_eq!(
            render_object.current_background(),
            palette.header_button_pressed()
        );
        assert_ne!(
            palette.header_button_hover(),
            palette.header_button_pressed()
        );
        assert!(palette.header_button_pressed().a > palette.header_button_hover().a);
        render_object.render();
        assert_eq!(
            center_pixel(&render_object),
            palette.header_button_pressed().to_bgra()
        );
    }

    #[test]
    fn raised_button_keeps_its_legacy_shaded_interaction_colors() {
        let mut render_object = laid_out(Button::new(""));
        let base = ColorPalette::default().button_background();

        assert_eq!(render_object.current_background(), base);
        render_object.set_hovered(true);
        assert_eq!(
            render_object.current_background(),
            ButtonRenderObject::shade_color(base, 0.97)
        );
        render_object.set_pressed(true);
        assert_eq!(
            render_object.current_background(),
            ButtonRenderObject::shade_color(base, 0.92)
        );
    }

    #[test]
    fn button_visual_geometry_stays_stable_across_posture_changes() {
        let _environment = crate::input_environment::install_test_input_environment(
            crate::InputEnvironment::new(1, Some(true), None, true, true, true, false),
        );
        let mut render_object = Button::new("Mode").build_render_object();
        let tablet_size = render_object.layout(LayoutConstraints::unconstrained());

        crate::input_environment::install_input_environment(crate::InputEnvironment::new(
            2,
            Some(false),
            None,
            true,
            true,
            true,
            false,
        ));
        let laptop_size = render_object.layout(LayoutConstraints::unconstrained());

        assert_eq!(tablet_size.height, 26.0);
        assert_eq!(laptop_size.height, tablet_size.height);
        assert_eq!(
            crate::current_input_environment().interaction_mode(),
            crate::InteractionMode::Pointer
        );
    }

    #[test]
    fn header_buffer_and_retained_paint_share_the_current_background() {
        let mut render_object = laid_out(Button::new("").header_style());
        render_object.set_hovered(true);
        let expected = render_object.current_background();
        render_object.render();

        assert_eq!(center_pixel(&render_object), expected.to_bgra());

        let mut context = PaintContext::new();
        assert!(render_object.paint(&mut context, Point::new(0.0, 0.0)));
        let Some(PaintCommand::FillRoundedRect {
            color,
            corner_radius,
            ..
        }) = context.commands().first()
        else {
            panic!("header button should paint a rounded background");
        };
        assert_eq!(*color, expected);
        assert!(*corner_radius > 0.0);
    }

    #[test]
    fn explicit_icon_size_uses_the_shared_icon_token_in_retained_paint() {
        let button = Button::icon_only(Icon::Settings)
            .header_style()
            .icon_size(IconSize::Medium);
        assert_eq!(button.get_icon_size(), Some(IconSize::Medium));

        let render_object = laid_out(button);
        let mut context = PaintContext::new();
        assert!(render_object.paint(&mut context, Point::ZERO));
        let Some(rect) = context.commands().iter().find_map(|command| match command {
            PaintCommand::DrawIcon { rect, .. } => Some(rect),
            _ => None,
        }) else {
            panic!("icon button should emit a retained icon command");
        };
        assert_eq!(rect.size, Size::new(20.0, 20.0));
    }

    #[test]
    fn update_preserves_header_appearance_and_semantic_state_colors() {
        let mut render_object = laid_out(Button::new(""));
        let replacement = Button::new("").header_style();

        assert!(matches!(
            render_object.update(&replacement),
            UpdateResult::Updated
        ));
        assert_eq!(render_object.appearance, ButtonAppearance::Header);
        assert_eq!(render_object.current_background(), Color::CLEAR);
        render_object.set_hovered(true);
        assert_eq!(
            render_object.current_background(),
            ColorPalette::default().header_button_hover()
        );
        render_object.set_pressed(true);
        assert_eq!(
            render_object.current_background(),
            ColorPalette::default().header_button_pressed()
        );
    }
}
