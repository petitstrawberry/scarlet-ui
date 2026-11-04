//! Text View - Displays text with styling options
//!
//! Text view renders text with configurable font size, color, and other styling.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::LayoutConstraints;
use crate::element::{Element, ElementRenderObject, RenderElement, UpdateResult};
use crate::geometry::{Point, Size};
use crate::graphics;
use crate::renderer::PaintContext;
use crate::view::View;
use alloc::boxed::Box;
use alloc::string::String;
use core::any::Any;

/// Semantic type sizes for titles, content and supporting labels.
///
/// These roles only choose a logical size. They do not change the font family,
/// weight, rasterization, or the current text color. [`Text::font_size`] remains
/// available for custom sizes, and the last size-setting modifier wins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextStyle {
    /// Page or section title, 24 logical pixels.
    Title,
    /// A short heading above body content, 18 logical pixels.
    Headline,
    /// Ordinary reading text, 16 logical pixels. The default for [`Text`].
    #[default]
    Body,
    /// Compact descriptive text or control-adjacent labels, 14 logical pixels.
    Callout,
    /// Short supporting metadata, 12 logical pixels.
    Caption,
}

impl TextStyle {
    /// Return the logical font size associated with this role.
    pub const fn font_size(self) -> f32 {
        match self {
            Self::Title => 24.0,
            Self::Headline => 18.0,
            Self::Body => 16.0,
            Self::Callout => 14.0,
            Self::Caption => 12.0,
        }
    }
}

/// Semantic emphasis independent from a text view's size.
///
/// Use secondary text for supporting explanations. Tertiary text is intended
/// only for disabled or nonessential content, not primary reading text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextTone {
    /// The strongest foreground color for essential content.
    #[default]
    Primary,
    /// Supporting information subordinate to the primary content.
    Secondary,
    /// Disabled or nonessential information.
    Tertiary,
}

impl TextTone {
    /// Resolve this tone using an explicit light or dark palette.
    pub fn color(self, palette: &ColorPalette) -> Color {
        match self {
            Self::Primary => palette.text_primary(),
            Self::Secondary => palette.text_secondary(),
            Self::Tertiary => palette.text_tertiary(),
        }
    }
}

/// Text View - displays a string of text
#[derive(Clone)]
pub struct Text {
    content: String,
    content_state: Option<crate::state::State<String>>,
    font_size: f32,
    color: Color,
}

impl Text {
    /// Create a new Text view with the given content
    pub fn new(content: impl Into<String>) -> Self {
        let content_str = content.into();
        let palette = ColorPalette::default();
        Self {
            content: content_str,
            content_state: None,
            font_size: TextStyle::Body.font_size(),
            color: palette.text_primary(),
        }
    }

    /// Create text whose content follows a reactive string state.
    ///
    /// # Arguments
    ///
    /// * `content` - State containing the displayed string.
    ///
    /// # Returns
    ///
    /// A text view that repaints and relayouts when the state changes.
    pub fn from_state(content: crate::state::State<String>) -> Self {
        let mut text = Self::new(content.get());
        text.content_state = Some(content);
        text
    }

    /// Apply a semantic type size without changing the text color or font.
    ///
    /// A later [`Self::font_size`] call overrides this size. This method also
    /// works with [`Self::from_state`] and preserves its reactive content.
    pub fn text_style(mut self, style: TextStyle) -> Self {
        self.font_size = style.font_size();
        self
    }

    /// Apply a semantic foreground tone from the current default palette.
    ///
    /// The text size is unchanged. A later [`Self::color`] call overrides this
    /// tone. Use [`TextTone::color`] for an explicitly chosen palette.
    pub fn tone(mut self, tone: TextTone) -> Self {
        self.color = tone.color(&ColorPalette::default());
        self
    }

    /// Set the font size in points
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Set the text color
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Get the text content
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Get the font size
    pub fn font_size_value(&self) -> f32 {
        self.font_size
    }

    /// Get the text color
    pub fn text_color(&self) -> Color {
        self.color
    }

    fn content_for_render(&self) -> String {
        self.content_state
            .as_ref()
            .map(|state| state.get())
            .unwrap_or_else(|| self.content.clone())
    }
}

impl View for Text {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            TextRenderObject::new(self.content_for_render(), self.font_size, self.color),
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        match self.content_state.as_ref() {
            Some(state) => alloc::vec![state as &dyn crate::state::Listenable],
            None => alloc::vec::Vec::new(),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Text RenderObject - handles text rendering
pub struct TextRenderObject {
    content: String,
    font_size: f32,
    color: Color,
    size: Size,
    buffer: Option<Buffer>,
}

impl TextRenderObject {
    /// Create a new TextRenderObject
    pub fn new(content: String, font_size: f32, color: Color) -> Self {
        Self {
            content,
            font_size,
            color,
            size: Size::ZERO,
            buffer: None,
        }
    }

    /// Get the buffer
    pub fn buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    /// Get the buffer mutably
    pub fn buffer_mut(&mut self) -> Option<&mut Buffer> {
        self.buffer.as_mut()
    }
}

impl ElementRenderObject for TextRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        if crate::debug::is_enabled() {
            crate::logln!(
                "[TextRenderObject::layout] START: content='{}', constraints=({:?}, {:?}) -> ({:?}, {:?})",
                self.content,
                constraints.min_width,
                constraints.min_height,
                constraints.max_width,
                constraints.max_height
            );
        }
        // Use actual font measurement
        let (measured_width, measured_height) =
            graphics::measure_text_sized(&self.content, self.font_size);

        if crate::debug::is_enabled() {
            crate::logln!(
                "[TextRenderObject::layout] measured={}x{}",
                measured_width,
                measured_height
            );
        }

        // For text, use the measured size, but constrain within bounds
        // Text should NOT expand to fill min_width/min_height
        let mut width = measured_width as f32;
        let mut height = measured_height as f32;

        // Apply max constraints (don't exceed maximum)
        if constraints.max_width.is_finite() && constraints.max_width > 0.0 {
            width = width.min(constraints.max_width);
        }
        if constraints.max_height.is_finite() && constraints.max_height > 0.0 {
            height = height.min(constraints.max_height);
        }

        // Only apply min constraints if measured size is smaller
        // But for text, we generally don't want to expand it
        // So we'll just use the measured size even if it's smaller than min

        self.size = Size { width, height };

        // Create/update buffer for this text
        let w = libm::ceilf(width) as u32;
        let h = libm::ceilf(height) as u32;
        if crate::debug::is_enabled() {
            crate::logln!(
                "[TextRenderObject] layout: final size={}x{}, buffer needed={} bytes",
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

    fn hit_test(&self, point: Point) -> bool {
        let bounds = crate::geometry::Rect {
            origin: Point::ZERO,
            size: self.size,
        };
        bounds.contains(point)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        // Render text to buffer
        if let Some(ref mut buffer) = self.buffer {
            let mut canvas = graphics::Canvas::for_buffer(buffer);
            let width = canvas.width();
            let height = canvas.height();

            if crate::debug::is_enabled() {
                crate::logln!(
                    "[TextRenderObject] render: buffer {}x{}, data.len()={}",
                    width,
                    height,
                    width.saturating_mul(height).saturating_mul(4)
                );
            }

            // Clear to avoid blending text on top of previous frames.
            canvas.fill_rect(0, 0, width, height, Color::TRANSPARENT);

            // Draw text
            canvas.draw_text_sized(0, 0, &self.content, self.color, self.font_size);
        } else {
            if crate::debug::is_enabled() {
                crate::logln!("[TextRenderObject] render: NO BUFFER!");
            }
        }
    }

    fn get_buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    fn clear_buffer(&mut self) {
        self.buffer = None;
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        ctx.draw_text(origin, self.content.clone(), self.color, self.font_size);
        true
    }

    fn update(&mut self, new_view: &dyn View) -> UpdateResult {
        let Some(text) = new_view.as_any().downcast_ref::<Text>() else {
            return UpdateResult::Replaced;
        };
        let content = text.content_for_render();
        if self.content == content && self.font_size == text.font_size && self.color == text.color {
            return UpdateResult::NoChange;
        }
        self.content = content;
        self.font_size = text.font_size;
        self.color = text.color;
        UpdateResult::Updated
    }

    fn update_needs_layout(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::PaintCommand;
    use crate::state::{State, generate_state_id};

    #[test]
    fn default_text_preserves_existing_body_size_and_primary_color() {
        let text = Text::new("Body");
        assert_eq!(text.font_size_value(), 16.0);
        assert_eq!(text.text_color(), ColorPalette::default().text_primary());
    }

    #[test]
    fn semantic_type_scale_is_ordered_and_does_not_recolor_text() {
        let mut previous = 0.0;
        for role in [
            TextStyle::Caption,
            TextStyle::Callout,
            TextStyle::Body,
            TextStyle::Headline,
            TextStyle::Title,
        ] {
            let text = Text::new("Label")
                .color(Color::rgb(20, 80, 160))
                .text_style(role);
            assert!(text.font_size_value() > previous);
            assert_eq!(text.font_size_value(), role.font_size());
            assert_eq!(text.text_color(), Color::rgb(20, 80, 160));
            previous = role.font_size();
        }
    }

    #[test]
    fn tones_follow_both_palettes_without_changing_size() {
        for palette in [ColorPalette::light(), ColorPalette::dark()] {
            assert_eq!(TextTone::Primary.color(&palette), palette.text_primary());
            assert_eq!(
                TextTone::Secondary.color(&palette),
                palette.text_secondary()
            );
            assert_eq!(TextTone::Tertiary.color(&palette), palette.text_tertiary());
        }
        let text = Text::new("Details")
            .text_style(TextStyle::Callout)
            .tone(TextTone::Secondary);
        assert_eq!(text.font_size_value(), 14.0);
        assert_eq!(text.text_color(), ColorPalette::default().text_secondary());
    }

    #[test]
    fn explicit_modifiers_keep_last_call_precedence() {
        let text = Text::new("Custom")
            .text_style(TextStyle::Title)
            .font_size(20.0)
            .tone(TextTone::Secondary)
            .color(Color::BLACK);
        assert_eq!(text.font_size_value(), 20.0);
        assert_eq!(text.text_color(), Color::BLACK);
        let text = text.text_style(TextStyle::Caption).tone(TextTone::Tertiary);
        assert_eq!(text.font_size_value(), 12.0);
        assert_eq!(text.text_color(), ColorPalette::default().text_tertiary());
    }

    #[test]
    fn styled_reactive_text_keeps_content_updates_and_paint_path() {
        let content = State::new(generate_state_id(), String::from("First"));
        let text = Text::from_state(content.clone())
            .text_style(TextStyle::Headline)
            .tone(TextTone::Secondary);
        assert_eq!(text.listenables().len(), 1);
        let mut render =
            TextRenderObject::new(text.content_for_render(), text.font_size, text.color);
        content.set(String::from("Second"));
        assert!(matches!(render.update(&text), UpdateResult::Updated));
        let mut ctx = PaintContext::new();
        render.paint(&mut ctx, Point::new(8.0, 12.0));
        let [
            PaintCommand::DrawText {
                text: content,
                color,
                font_size_px,
                ..
            },
        ] = ctx.commands()
        else {
            panic!("semantic text should use the unchanged text paint command");
        };
        assert_eq!(content, "Second");
        assert_eq!(*color, ColorPalette::default().text_secondary());
        assert_eq!(*font_size_px, 18.0);
    }
}
