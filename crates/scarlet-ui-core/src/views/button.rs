//! Button View - Interactive button with callback
//!
//! Button displays a label and triggers an action when clicked.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, RenderElement};
use crate::geometry::{EdgeInsets, Point, Rect, Size};
use crate::graphics;
use crate::icon::{Icon, IconFill, IconSize, IconStyle, IconWeight};
use crate::renderer::PaintContext;
use crate::view::View;
use crate::views::style;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::any::Any;
use core::time::Duration;

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
    press_repeat: Option<(Duration, Duration)>,
    appearance: ButtonAppearance,
    material: Option<style::GlassMaterial>,
    enabled: bool,
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
            border_color: palette.border(),
            text_color: palette.text_primary(),
            font_size: style::CONTROL_FONT_SIZE,
            padding: style::CONTROL_PADDING,
            press_repeat: None,
            appearance: ButtonAppearance::Raised,
            material: Some(style::GlassMaterial::button()),
            enabled: true,
        }
    }

    /// Use the shared backdrop material on a raised button, without changing geometry or input.
    /// Header-style buttons retain their compact, transparent treatment.
    pub fn material(mut self, material: style::GlassMaterial) -> Self {
        self.material = Some(material);
        self
    }

    /// Enable interaction, or display a non-interactive disabled control.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
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
            border_color: palette.border(),
            text_color: palette.text_primary(),
            font_size: style::CONTROL_FONT_SIZE,
            padding: 7.0,
            press_repeat: None,
            appearance: ButtonAppearance::Raised,
            material: Some(style::GlassMaterial::button()),
            enabled: true,
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

    /// Set a custom background color, opting out of the default glass material.
    /// Call `material` afterward to restore a backdrop material.
    pub fn background_color(mut self, color: Color) -> Self {
        self.background_color = color;
        self.material = None;
        self
    }

    /// Set the text color
    pub fn text_color(mut self, color: Color) -> Self {
        self.text_color = color;
        self
    }

    /// Set a custom border color, opting out of the default glass material.
    /// Call `material` afterward to restore a backdrop material.
    pub fn border_color(mut self, color: Color) -> Self {
        self.border_color = color;
        self.material = None;
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

    /// Opt in to press activation and repeat while a touch remains held.
    ///
    /// Touch input invokes the normal click action immediately, waits for
    /// `initial_delay`, then invokes it at `interval` until release or gesture
    /// cancellation. The final tap is suppressed so a press never activates
    /// twice on release.
    /// Mouse and keyboard input retain their normal click behavior.
    pub fn repeat_while_pressed(mut self, initial_delay: Duration, interval: Duration) -> Self {
        self.press_repeat = Some((initial_delay, interval.max(Duration::from_millis(1))));
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

    pub(crate) fn has_click_callback(&self) -> bool {
        self.on_click.is_some()
    }

    /// Invoke the click callback if present
    pub fn invoke_on_click(&self) {
        if !self.enabled {
            return;
        }
        if let Some(callback) = self.on_click.as_ref() {
            callback();
        }
    }

    pub(crate) fn press_repeat_timing(&self) -> Option<(Duration, Duration)> {
        if !self.enabled {
            return None;
        }
        self.on_click.as_ref()?;
        self.press_repeat
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
        render_object.material = self.material;
        render_object.enabled = self.enabled;
        render_object.repeats_on_press = self.press_repeat.is_some();
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
    material: Option<style::GlassMaterial>,
    enabled: bool,
    hovered: bool,
    focused: bool,
    keyboard_pressed: bool,
    keyboard_key: Option<crate::event::KeyCode>,
    repeats_on_press: bool,
    pressed: crate::views::press::PressSources,
    size: Size,
    buffer: Option<Buffer>,
}

impl ButtonRenderObject {
    pub fn handle_keyboard(&mut self, event: crate::event::KeyEvent) -> Option<bool> {
        use crate::event::{KeyCode, KeyEvent};
        if !self.enabled || !self.focused {
            return None;
        }
        match event {
            KeyEvent::Pressed {
                keycode: key @ (KeyCode::Enter | KeyCode::Space),
                modifiers,
            } if !modifiers.control && !modifiers.alt && !modifiers.super_key => {
                if self.keyboard_key.is_none() {
                    self.keyboard_key = Some(key);
                    self.keyboard_pressed = true;
                }
                Some(false)
            }
            KeyEvent::Released {
                keycode: key @ (KeyCode::Enter | KeyCode::Space),
                ..
            } => {
                let activate = self.keyboard_key == Some(key);
                if activate {
                    self.keyboard_key = None;
                    self.keyboard_pressed = false;
                    self.keyboard_key = None;
                }
                Some(activate)
            }
            KeyEvent::Char {
                c: ' ' | '\r' | '\n',
            } => Some(false),
            _ => None,
        }
    }

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
            material: None,
            enabled: true,
            hovered: false,
            focused: false,
            keyboard_pressed: false,
            keyboard_key: None,
            repeats_on_press: false,
            pressed: crate::views::press::PressSources::default(),
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
        let (text_width, _) = graphics::measure_text_sized(&self.label, self.font_size);
        let icon_width = if self.icon.is_some() {
            self.icon_size
                .map(|size| size.logical_pixels() as f32)
                .unwrap_or(16.0)
                + style::CONTROL_GAP
        } else {
            0.0
        };
        let width = text_width as f32 + icon_width + self.padding * 2.0;
        let height = if self.font_size == style::CONTROL_FONT_SIZE
            && self.padding == style::CONTROL_PADDING
        {
            style::CONTROL_HEIGHT
        } else {
            graphics::line_height_sized(self.font_size) as f32 + self.padding * 2.0
        };

        Size { width, height }
    }

    pub fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered && self.enabled;
    }

    pub fn set_pressed(&mut self, pressed: bool) {
        self.pressed.set_mouse(pressed && self.enabled);
    }

    pub fn set_touch_pressed(&mut self, id: u64, pressed: bool) {
        self.pressed.set_touch(id, pressed && self.enabled);
    }

    pub fn is_pressed(&self) -> bool {
        self.pressed.is_pressed() || self.keyboard_pressed
    }

    pub fn is_mouse_pressed(&self) -> bool {
        self.pressed.is_mouse_pressed()
    }

    pub(crate) fn is_focused(&self) -> bool {
        self.focused && self.enabled
    }

    pub(crate) fn set_focused(&mut self, focused: bool) {
        self.focused = focused && self.enabled;
        if !self.focused {
            self.keyboard_pressed = false;
            self.keyboard_key = None;
            self.pressed = Default::default();
        }
    }

    pub(crate) fn set_keyboard_pressed(&mut self, pressed: bool) {
        self.keyboard_pressed = pressed && self.enabled;
    }

    pub(crate) fn is_keyboard_pressed(&self) -> bool {
        self.keyboard_pressed
    }

    fn current_text(&self) -> Color {
        if self.enabled {
            self.text_color
        } else {
            ColorPalette::default().text_tertiary()
        }
    }

    fn current_background(&self) -> Color {
        if !self.enabled {
            return ColorPalette::default().background_secondary();
        }
        if self.is_pressed() {
            self.pressed_background_color
                .unwrap_or_else(|| self.background_color.darken(0.08))
        } else if self.hovered {
            self.hover_background_color
                .unwrap_or_else(|| self.background_color.darken(0.035))
        } else {
            self.background_color
        }
    }

    fn current_border(&self) -> Color {
        if !self.enabled {
            return ColorPalette::default().divider();
        }
        if self.is_pressed() {
            self.border_color.darken(0.08)
        } else if self.hovered {
            self.border_color.darken(0.04)
        } else {
            self.border_color
        }
    }
}

impl ElementRenderObject for ButtonRenderObject {
    fn keeps_touch_press_within_bounds(&self) -> bool {
        self.enabled && !self.repeats_on_press
    }

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

        // Honor tight sizing from a Frame or parent so the button's layout,
        // painted bounds, and hit area agree. Loose sizing remains intrinsic.
        let mut width = intrinsic.width;
        let mut height = intrinsic
            .height
            .max(if self.appearance == ButtonAppearance::Header {
                style::metrics().minimum_control_height
            } else {
                style::CONTROL_HEIGHT
            });

        // Apply max constraints (don't exceed maximum)
        if constraints.max_width.is_finite() && constraints.max_width > 0.0 {
            width = width.min(constraints.max_width);
        }
        if constraints.max_height.is_finite() && constraints.max_height > 0.0 {
            height = height.min(constraints.max_height);
        }

        if constraints.is_tight_width() {
            width = constraints.max_width;
        }
        if constraints.is_tight_height() {
            height = constraints.max_height;
        }

        self.size = Size { width, height };

        // Explicit-color controls retain their shadow outside the layout/hit box.
        let outsets = self.paint_outsets();
        let w = libm::ceilf(width + outsets.left + outsets.right) as u32;
        let h = libm::ceilf(height + outsets.top + outsets.bottom) as u32;

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

    fn paint_outsets(&self) -> EdgeInsets {
        if self.appearance == ButtonAppearance::Raised && self.material.is_none() {
            style::elevation_outsets(style::ElevationRole::Raised)
        } else {
            EdgeInsets::ZERO
        }
    }

    fn buffer_origin(&self) -> Point {
        let outsets = self.paint_outsets();
        Point::new(-outsets.left, -outsets.top)
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
        // Preserve the straight-alpha contract of compact legacy header buffers.
        if self.appearance == ButtonAppearance::Header {
            let background = self.current_background();
            let text_color = self.current_text();
            if let Some(buffer) = self.buffer.as_mut() {
                let mut canvas = graphics::Canvas::for_buffer(buffer);
                let width = canvas.width();
                let height = canvas.height();
                canvas.fill_rect(0, 0, width, height, Color::CLEAR);
                canvas_fill_rounded_rect(
                    &mut canvas,
                    width,
                    height,
                    style::metrics().control_radius,
                    background,
                );
                let (text_width, _) = graphics::measure_text_sized(&self.label, self.font_size);
                canvas.draw_text_sized(
                    ((width as i32 - text_width as i32) / 2).max(0),
                    style::control_text_y(height as f32, self.font_size) as i32,
                    &self.label,
                    text_color,
                    self.font_size,
                );
            }
            return;
        }
        // Keep the deprecated buffer route visually identical to retained paint.
        let Some(mut buffer) = self.buffer.take() else {
            return;
        };
        {
            let mut ctx = PaintContext::new();
            let origin = self.buffer_origin();
            self.paint(&mut ctx, Point::new(-origin.x, -origin.y));
            #[allow(deprecated)]
            crate::renderer::CpuPaintRenderer::execute_into_buffer(
                &mut buffer,
                Color::CLEAR,
                &ctx,
                None,
            );
        }
        self.buffer = Some(buffer);
    }

    fn get_buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    fn clear_buffer(&mut self) {
        self.buffer = None;
    }

    fn reads_backdrop(&self) -> bool {
        self.appearance == ButtonAppearance::Raised && self.material.is_some()
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let rect = Rect::new(origin, self.size);
        let background = self.current_background();
        let border = self.current_border();

        match self.appearance {
            ButtonAppearance::Raised => {
                // Glass controls share the thin editing surface. Feedback comes
                // from the material tint, not a raised face or contact shadow.
                if self.enabled && self.material.is_none() {
                    let shadow = ColorPalette::default().shadow();
                    style::elevation_shadow(
                        ctx,
                        rect,
                        style::metrics().control_radius,
                        shadow.with_opacity(shadow.a * if self.is_pressed() { 0.35 } else { 1.0 }),
                        style::ElevationRole::Raised,
                    );
                }
                if let Some(mut material) = self.material {
                    if self.is_pressed() {
                        material.tint = material
                            .tint
                            .darken(0.025)
                            .with_opacity((material.tint.a + 0.12).min(1.0));
                    } else if self.hovered {
                        material.tint = material
                            .tint
                            .lighten(0.016)
                            .with_opacity((material.tint.a + 0.06).min(1.0));
                    }
                    if !self.enabled {
                        material.tint = material.tint.with_opacity(material.tint.a * 0.55);
                    }
                    style::paint_glass_material(
                        ctx,
                        rect,
                        style::metrics().control_radius,
                        material,
                    );
                } else {
                    style::raised_control_surface(ctx, rect, background, border, self.is_pressed());
                }
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
                .unwrap_or(16.0)
                .min(available_icon_size);
            let (text_w, _text_h) = graphics::measure_text_sized(&self.label, self.font_size);
            let group_width = if self.label.is_empty() {
                icon_size
            } else {
                icon_size + style::CONTROL_GAP + text_w as f32
            };
            let group_x = origin.x + ((self.size.width - group_width) / 2.0).max(0.0);
            crate::views::icon::paint_icon(
                ctx,
                Point::new(group_x, origin.y + (self.size.height - icon_size) * 0.5),
                icon_size,
                icon,
                self.icon_style,
                if self.enabled {
                    self.icon_color.unwrap_or(self.text_color)
                } else {
                    self.current_text()
                },
            );
            if !self.label.is_empty() {
                ctx.draw_text(
                    Point::new(
                        group_x + icon_size + style::CONTROL_GAP,
                        origin.y + style::control_text_y(self.size.height, self.font_size),
                    ),
                    self.label.clone(),
                    self.current_text(),
                    self.font_size,
                );
            }
        } else {
            let (text_w, _text_h) = graphics::measure_text_sized(&self.label, self.font_size);
            let x = origin.x + ((self.size.width - text_w as f32) / 2.0).max(0.0);
            let y = origin.y + style::control_text_y(self.size.height, self.font_size);
            ctx.draw_text(
                Point::new(x, y),
                self.label.clone(),
                self.current_text(),
                self.font_size,
            );
        }
        true
    }

    fn update(&mut self, new_view: &dyn View) -> crate::element::UpdateResult {
        let Some(button) = new_view.as_any().downcast_ref::<Button>() else {
            return crate::element::UpdateResult::Replaced;
        };
        if self.label == button.label
            && self.icon == button.icon
            && self.icon_size == button.icon_size
            && self.icon_style == button.icon_style
            && self.icon_color == button.icon_color
            && self.background_color == button.background_color
            && self.hover_background_color == button.hover_background_color
            && self.pressed_background_color == button.pressed_background_color
            && self.border_color == button.border_color
            && self.text_color == button.text_color
            && self.font_size == button.font_size
            && self.padding == button.padding
            && self.appearance == button.appearance
            && self.material == button.material
            && self.enabled == button.enabled
        {
            return crate::element::UpdateResult::NoChange;
        }
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
        self.material = button.material;
        self.enabled = button.enabled;
        if !self.enabled {
            self.hovered = false;
            self.focused = false;
            self.keyboard_pressed = false;
            self.pressed = Default::default();
        }
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

    fn glass_tint(render_object: &ButtonRenderObject) -> Color {
        let mut context = PaintContext::new();
        render_object.paint(&mut context, Point::ZERO);
        assert!(!context.commands().iter().any(|command| matches!(
            command,
            PaintCommand::FillVerticalGradientRoundedRect { .. } | PaintCommand::FillPath { .. }
        )));
        let filter_index = context
            .commands()
            .iter()
            .position(|command| matches!(command, PaintCommand::BackdropFilterRoundedRect { .. }))
            .expect("stock raised button should filter its backdrop");
        assert!(
            context.commands()[filter_index + 1..]
                .iter()
                .any(|command| matches!(command, PaintCommand::DrawText { .. })),
            "foreground must be painted after backdrop filtering"
        );
        match &context.commands()[filter_index] {
            PaintCommand::BackdropFilterRoundedRect { tint, .. } => *tint,
            _ => unreachable!(),
        }
    }

    #[test]
    fn stock_buttons_use_glass_with_distinct_uniform_states() {
        let mut button = laid_out(Button::new("Save"));
        assert!(button.reads_backdrop());
        let rest = glass_tint(&button);
        button.set_hovered(true);
        let hover = glass_tint(&button);
        button.set_pressed(true);
        let pressed = glass_tint(&button);
        button.set_pressed(false);
        button.set_hovered(false);
        button.set_focused(true);
        let focused = glass_tint(&button);
        let disabled = laid_out(Button::new("Save").enabled(false));
        assert!(disabled.reads_backdrop());
        assert_ne!(rest, hover);
        assert_ne!(hover, pressed);
        assert_eq!(rest, focused, "keyboard focus must not tint the glass face");
        let mut focused_paint = PaintContext::new();
        button.paint(&mut focused_paint, Point::ZERO);
        assert!(
            !focused_paint
                .commands()
                .iter()
                .any(|command| matches!(command,
                    PaintCommand::StrokeRoundedRect { stroke_width, .. } if *stroke_width == 3.0
                ))
        );
        button.set_focused(false);
        let mut unfocused_paint = PaintContext::new();
        button.paint(&mut unfocused_paint, Point::ZERO);
        assert!(
            !unfocused_paint
                .commands()
                .iter()
                .any(|command| matches!(command, PaintCommand::StrokeRoundedRect { .. }))
        );
        assert_ne!(rest, glass_tint(&disabled));
        assert!(laid_out(Button::icon_only(Icon::Settings)).reads_backdrop());
    }

    #[test]
    fn custom_button_chrome_and_headers_preserve_material_opt_out() {
        let mut button =
            laid_out(Button::new("Save").background_color(Color::rgb(255u8, 0u8, 0u8)));
        assert!(!button.reads_backdrop());
        assert!(
            !laid_out(Button::new("Save").border_color(Color::rgb(255u8, 0u8, 0u8)))
                .reads_backdrop()
        );
        assert!(!laid_out(Button::new("Save").header_style()).reads_backdrop());
        button.update(
            &Button::new("Save")
                .background_color(Color::rgb(255u8, 0u8, 0u8))
                .material(style::GlassMaterial::regular()),
        );
        assert!(button.reads_backdrop());
        button.update(&Button::new("Save").header_style());
        assert!(!button.reads_backdrop());
    }

    #[test]
    fn glass_button_stays_thin_and_legible_without_a_contact_shadow() {
        let mut button = laid_out(Button::new(""));
        button.layout(LayoutConstraints::tight(100.0, 36.0));
        for (hovered, pressed) in [(false, false), (true, false), (true, true)] {
            button.set_hovered(hovered);
            button.set_pressed(pressed);
            let mut context = PaintContext::new();
            button.paint(&mut context, Point::new(12.0, 12.0));
            assert!(
                !context.commands().iter().any(|command| matches!(
                    command,
                    PaintCommand::DrawRoundedRectShadow { .. }
                        | PaintCommand::FillVerticalGradientRoundedRect { .. }
                )),
                "glass feedback must not add elevation or a bevel"
            );
            for scale in [1000, 2000] {
                let mut renderer = crate::renderer::CpuPaintRenderer::new(
                    Size::new(124.0, 64.0),
                    scale,
                    Color::WHITE,
                );
                renderer.execute(&context);
                let buffer = renderer.buffer();
                let factor = scale / 1000;
                let pixel = |x: u32, y: u32| {
                    buffer.as_slice()[(y * factor * buffer.width() + x * factor) as usize]
                        .to_le_bytes()[0]
                };
                assert!(
                    pixel(62, 28) < 250,
                    "tint keeps the control visible on white"
                );
                assert_eq!(pixel(62, 54), 255, "no shadow below the control");
            }
            assert_eq!(button.paint_outsets(), EdgeInsets::ZERO);
        }
        let explicit = laid_out(Button::new("").background_color(Color::WHITE));
        let mut context = PaintContext::new();
        explicit.paint(&mut context, Point::ZERO);
        assert!(
            context
                .commands()
                .iter()
                .any(|command| matches!(command, PaintCommand::DrawRoundedRectShadow { .. })),
            "explicit-color buttons retain their compatibility treatment"
        );
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
    fn standard_button_has_distinct_flat_interaction_colors() {
        let mut render_object = laid_out(Button::new(""));
        let base = ColorPalette::default().button_background();

        assert_eq!(render_object.current_background(), base);
        render_object.set_hovered(true);
        assert_eq!(render_object.current_background(), base.darken(0.035));
        render_object.set_pressed(true);
        assert_eq!(render_object.current_background(), base.darken(0.08));
    }

    #[test]
    fn explicit_frame_constraints_expand_the_button_and_hit_area() {
        let mut render_object = Button::new("q").build_render_object();
        let size = render_object.layout(LayoutConstraints::tight(64.0, 52.0));

        assert_eq!(size, Size::new(64.0, 52.0));
        assert!(render_object.hit_test(Point::new(63.0, 51.0)));
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

        assert_eq!(tablet_size.height, 36.0);
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
