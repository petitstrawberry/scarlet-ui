//! Text View - Displays text with styling options
//!
//! Text view renders text with configurable font size, color, and other styling.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::LayoutConstraints;
use crate::element::{Element, ElementRenderObject, RenderElement, UpdateResult};
use crate::geometry::{Point, Rect, Size};
use crate::graphics;
use crate::renderer::PaintContext;
use crate::view::View;
use super::text_fit::{self, Truncation};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
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

/// Text View - displays a string of text.
///
/// Text takes its natural width when it can. Given less, it shortens the line
/// with an ellipsis ([`Self::truncation`], at the end by default), or wraps
/// onto more lines when [`Self::line_limit`] allows them.
#[derive(Clone)]
pub struct Text {
    content: String,
    content_state: Option<crate::state::State<String>>,
    font_size: f32,
    color: Color,
    truncation: Truncation,
    line_limit: Option<usize>,
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
            truncation: Truncation::Tail,
            line_limit: Some(1),
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

    /// Where a line that does not fit is shortened with an ellipsis: at the
    /// end (the default), the start, or the middle (good for paths).
    pub fn truncation(mut self, truncation: Truncation) -> Self {
        self.truncation = truncation;
        self
    }

    /// How many lines the text may wrap onto when its width is limited:
    /// `Some(1)` (the default) keeps one line, `None` allows as many as it
    /// needs. Text left over after the last line ends in an ellipsis.
    pub fn line_limit(mut self, lines: Option<usize>) -> Self {
        self.line_limit = lines;
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
        let mut render = TextRenderObject::new(self.content_for_render(), self.font_size, self.color);
        render.truncation = self.truncation;
        render.line_limit = self.line_limit;
        Box::new(RenderElement::new(self.clone(), render))
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
    truncation: Truncation,
    line_limit: Option<usize>,
    /// The lines drawn, fitted to the last width; the content when it fits.
    lines: Vec<String>,
    line_height: f32,
    /// The width and font revision `lines` were fitted for, and the size
    /// they take.
    fitted: Option<(u32, usize, Size)>,
    size: Size,
    buffer: Option<Buffer>,
    // Intrinsic measurement is independent of parent constraints. Keep it with
    // the label rather than depending on a small shared cache during relayout.
    intrinsic_measurement: Option<(usize, (u32, u32))>,
    #[cfg(test)]
    measurement_count: usize,
}

impl TextRenderObject {
    /// Create a new TextRenderObject
    pub fn new(content: String, font_size: f32, color: Color) -> Self {
        Self {
            lines: alloc::vec![content.clone()],
            content,
            font_size,
            color,
            truncation: Truncation::Tail,
            line_limit: Some(1),
            line_height: 0.0,
            fitted: None,
            size: Size::ZERO,
            buffer: None,
            intrinsic_measurement: None,
            #[cfg(test)]
            measurement_count: 0,
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
        let revision = graphics::text_font_revision();
        let (measured_width, measured_height) = match self.intrinsic_measurement {
            Some((cached_revision, size)) if cached_revision == revision => size,
            _ => {
                let measure = graphics::TextMeasure::new(&self.content, self.font_size);
                let size = measure.size(&self.content);
                // Capture the revision of the actual font stack used, including
                // any fallback discovered while resolving this label.
                self.intrinsic_measurement = Some((measure.font_revision(), size));
                #[cfg(test)]
                {
                    self.measurement_count += 1;
                }
                size
            }
        };

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

        let limit = constraints.max_width.max(0.0);
        if constraints.max_width.is_finite() && width > limit {
            // Too wide: shorten with an ellipsis, or wrap onto more lines.
            let revision = self.intrinsic_measurement.map_or(0, |(revision, _)| revision);
            let fitted = match self.fitted {
                Some((bits, fitted_revision, size))
                    if bits == limit.to_bits() && fitted_revision == revision =>
                {
                    size
                }
                _ => {
                    let measure = graphics::TextMeasure::new(&self.content, self.font_size);
                    self.lines = text_fit::fit(
                        &self.content,
                        &measure,
                        limit,
                        self.line_limit,
                        self.truncation,
                    );
                    self.line_height = measure.line_height();
                    let widest = self
                        .lines
                        .iter()
                        .map(|line| measure.size(line).0)
                        .max()
                        .unwrap_or(0) as f32;
                    let extra = self.lines.len().saturating_sub(1) as f32 * self.line_height;
                    let size = Size::new(widest.min(limit), height + extra);
                    self.fitted = Some((limit.to_bits(), revision, size));
                    size
                }
            };
            width = fitted.width;
            height = fitted.height;
        } else if self.fitted.take().is_some() {
            self.lines = alloc::vec![self.content.clone()];
        }
        if constraints.max_height.is_finite() {
            height = height.min(constraints.max_height.max(0.0));
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
            for (index, line) in self.lines.iter().enumerate() {
                let y = libm::roundf(index as f32 * self.line_height) as i32;
                canvas.draw_text_sized(0, y, line, self.color, self.font_size);
            }
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
        if self.size.width <= 0.0 || self.size.height <= 0.0 {
            return true;
        }
        // Match the bounds of the CPU text buffer. Ancestor clips (including
        // edge-to-edge scroll viewports) can be wider than this text's layout.
        ctx.push_clip(Rect::new(origin, self.size));
        for (index, line) in self.lines.iter().enumerate() {
            let point = Point::new(origin.x, origin.y + index as f32 * self.line_height);
            ctx.draw_text(point, line.clone(), self.color, self.font_size);
        }
        ctx.pop_clip();
        true
    }

    fn update(&mut self, new_view: &dyn View) -> UpdateResult {
        let Some(text) = new_view.as_any().downcast_ref::<Text>() else {
            return UpdateResult::Replaced;
        };
        let content = text.content_for_render();
        if self.content == content
            && self.font_size == text.font_size
            && self.color == text.color
            && self.truncation == text.truncation
            && self.line_limit == text.line_limit
        {
            return UpdateResult::NoChange;
        }
        if self.content != content || self.font_size != text.font_size {
            self.intrinsic_measurement = None;
        }
        // Fit again at the next layout.
        self.fitted = None;
        self.lines = alloc::vec![content.clone()];
        self.truncation = text.truncation;
        self.line_limit = text.line_limit;
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
    use crate::renderer::{CpuPaintRenderer, PaintCommand};
    use crate::state::{State, generate_state_id};

    #[test]
    fn dense_label_relayout_reuses_intrinsics_and_invalidates_content_size_and_font_revision() {
        // The first ellipsis may load a fallback face, which moves the font
        // revision once; settle it before counting measurements.
        graphics::measure_text_sized(text_fit::ELLIPSIS, 13.);
        let mut labels: alloc::vec::Vec<_> = (0..1024)
            .map(|i| TextRenderObject::new(alloc::format!("Label {i:04}"), 13., Color::BLACK))
            .collect();
        let loose = LayoutConstraints::loose(1000., 1000.);
        let natural: alloc::vec::Vec<_> = labels.iter_mut().map(|l| l.layout(loose)).collect();
        for (label, expected) in labels.iter_mut().zip(natural) {
            // Too narrow for even the ellipsis: nothing is drawn.
            let small = label.layout(LayoutConstraints::loose(2., 3.));
            assert_eq!(small, Size::new(0., expected.height.min(3.)));
            assert_eq!(label.layout(loose), expected);
            assert_eq!(
                label.measurement_count, 1,
                "shared cache eviction must not remeasure retained labels"
            );
        }
        let label = &mut labels[0];
        let text = Text::new("a different label").font_size(13.);
        label.update(&text);
        label.layout(loose);
        assert_eq!(label.measurement_count, 2);
        label.update(&text.clone().color(Color::RED));
        label.layout(loose);
        assert_eq!(label.measurement_count, 2);
        label.update(&text.font_size(25.));
        label.layout(loose);
        assert_eq!(label.measurement_count, 3);
        // A cached revision must never survive a different current font stack.
        label.intrinsic_measurement = Some((usize::MAX, (1, 1)));
        let refreshed = label.layout(loose);
        assert_eq!(label.measurement_count, 4);
        assert_eq!(refreshed, {
            let (w, h) = graphics::measure_text_sized("a different label", 25.);
            Size::new(w as f32, h as f32)
        });
    }

    #[test]
    fn constrained_text_pixels_stay_inside_layout_and_parent_clip() {
        let mut text = TextRenderObject::new(
            String::from("Controls description extends beyond its allocated width"),
            20.0,
            Color::BLACK,
        );
        let natural = text.layout(LayoutConstraints::loose(f32::INFINITY, f32::INFINITY));
        assert!(natural.width > 40.0);
        for (width, height) in [(40.0, natural.height), (40.0, 8.0)] {
            // Shortened with an ellipsis to fit, never wider than given.
            let size = text.layout(LayoutConstraints::loose(width, height));
            assert!(size.width > 20.0 && size.width <= width, "{size:?}");
            assert_eq!(size.height, height);
            let origin = Point::new(12.0, 10.0);
            let mut ctx = PaintContext::new();
            // The parent clips the left edge; Text must also clip its right
            // and bottom edges, without leaking or replacing the parent clip.
            ctx.push_clip(Rect::from_xywh(16.0, 0.0, 144.0, 60.0));
            text.paint(&mut ctx, origin);
            ctx.fill_rect(Rect::from_xywh(0.0, 50.0, 160.0, 4.0), Color::BLACK);
            ctx.pop_clip();
            for scale in [1000, 2000] {
                let mut renderer =
                    CpuPaintRenderer::new(Size::new(160.0, 60.0), scale, Color::WHITE);
                renderer.execute(&ctx);
                let factor = scale / 1000;
                let mut ink = 0;
                for y in 0..50 * factor {
                    for x in 0..160 * factor {
                        let pixel = renderer.buffer().get_pixel(x, y).unwrap();
                        if x >= 16 * factor
                            && x < 52 * factor
                            && y >= 10 * factor
                            && y < (10 + height as u32) * factor
                        {
                            ink += usize::from(pixel != Color::WHITE.to_bgra());
                        } else {
                            assert_eq!(pixel, Color::WHITE.to_bgra(), "overflow at {x},{y}");
                        }
                    }
                }
                assert!(ink > 0, "text must actually render");
                assert_eq!(
                    renderer
                        .buffer()
                        .get_pixel(80 * factor, 51 * factor)
                        .unwrap(),
                    Color::BLACK.to_bgra()
                );
                assert_eq!(
                    renderer
                        .buffer()
                        .get_pixel(4 * factor, 51 * factor)
                        .unwrap(),
                    Color::WHITE.to_bgra()
                );
            }
        }
    }

    #[test]
    fn narrow_text_draws_an_ellipsis_or_wraps_and_widens_back() {
        let content = "Staged changes in a folder with a long name";
        let natural = graphics::measure_text_sized(content, 13.).0 as f32;
        let mut text = TextRenderObject::new(String::from(content), 13., Color::BLACK);
        let one_line = text.layout(LayoutConstraints::loose(natural, 100.));
        let narrow = text.layout(LayoutConstraints::loose(natural / 2., 100.));
        assert!(narrow.width <= natural / 2. && narrow.height == one_line.height);
        assert_eq!(text.lines.len(), 1);
        assert!(text.lines[0].ends_with(text_fit::ELLIPSIS), "{:?}", text.lines);
        let mut ctx = PaintContext::new();
        text.paint(&mut ctx, Point::ZERO);
        let drawn: alloc::vec::Vec<_> = ctx
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCommand::DrawText { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(drawn, text.lines);
        // Given its width back, the whole label returns.
        assert_eq!(text.layout(LayoutConstraints::loose(natural, 100.)), one_line);
        assert_eq!(text.lines, [content]);

        text.line_limit = None;
        text.fitted = None;
        let wrapped = text.layout(LayoutConstraints::loose(natural / 2., 100.));
        assert!(text.lines.len() >= 2 && !text.lines.concat().contains(text_fit::ELLIPSIS));
        assert!(wrapped.height > one_line.height && wrapped.width <= natural / 2.);
    }

    #[test]
    fn zero_extent_text_does_not_paint() {
        for (width, height) in [(0.0, 30.0), (80.0, 0.0)] {
            let mut text = TextRenderObject::new(String::from("Hidden"), 16.0, Color::BLACK);
            let size = text.layout(LayoutConstraints::loose(width, height));
            assert!(size.width == 0.0 || size.height == 0.0);
            let mut ctx = PaintContext::new();
            assert!(text.paint(&mut ctx, Point::ZERO));
            assert!(ctx.commands().is_empty());
        }
    }

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
        render.layout(LayoutConstraints::loose(200.0, 40.0));
        let mut ctx = PaintContext::new();
        render.paint(&mut ctx, Point::new(8.0, 12.0));
        let [
            PaintCommand::PushClip { .. },
            PaintCommand::DrawText {
                text: content,
                color,
                font_size_px,
                ..
            },
            PaintCommand::PopClip,
        ] = ctx.commands()
        else {
            panic!("semantic text should use a bounded text paint command");
        };
        assert_eq!(content, "Second");
        assert_eq!(*color, ColorPalette::default().text_secondary());
        assert_eq!(*font_size_px, 18.0);
    }
}
