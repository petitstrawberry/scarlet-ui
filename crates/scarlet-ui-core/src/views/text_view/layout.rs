//! Layout computation for the multi-line text view.

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::{BORDER_WIDTH, TextDocument, TextPosition, TextViewScroll, WrapMode};
use crate::color::Color;
use crate::geometry::{Point, Rect, Size};
use crate::graphics;
use crate::renderer::PaintContext;

const GUTTER_PADDING: f32 = 12.0;
const MIN_LINE_NUMBER_DIGITS: usize = 4;
const DEFAULT_TAB_WIDTH: usize = 4;

/// Computed layout for a TextView render object.
#[derive(Clone, Debug)]
pub struct TextViewLayout {
    /// Height of one visual text row in logical pixels.
    pub line_height: f32,
    /// Width of the scrollable text content in logical pixels.
    pub content_width: f32,
    /// Height of the scrollable text content in logical pixels.
    pub content_height: f32,
    /// Widest measured visual line.
    pub max_line_width: f32,
    /// Width reserved for the line-number gutter.
    pub gutter_width: f32,
    /// Range of visual lines intersecting the viewport.
    pub visible_lines: Range<usize>,
    /// Visual line fragments used by paint and hit testing.
    pub visual_lines: Vec<VisualLine>,
    pub(crate) font_size: f32,
    pub(crate) padding: f32,
    pub(crate) viewport_width: f32,
    pub(crate) viewport_height: f32,
    pub(crate) text_origin_x: f32,
    pub(crate) scroll: TextViewScroll,
    document: TextDocument,
    wrap_mode: WrapMode,
    size: Size,
    show_line_numbers: bool,
    font_revision: usize,
    font_stack: Option<Arc<graphics::FontStack>>,
}

/// A measured visual line fragment.
#[derive(Clone, Debug)]
pub struct VisualLine {
    /// Zero-based logical document line index.
    pub logical_line: usize,
    /// Absolute UTF-8 byte range covered by this visual fragment in the original document.
    ///
    /// The stored display text expands tabs to spaces for measurement and rendering, so this range
    /// can differ from `text.len()` when the source document contains tab characters.
    pub text_range: Range<usize>,
    /// X origin in widget-local coordinates after scroll is applied.
    pub x: f32,
    /// Y origin in widget-local coordinates after scroll is applied.
    pub y: f32,
    /// Measured text width in logical pixels.
    pub width: f32,
    text: String,
    document_to_display_offsets: Vec<usize>,
    prefix_widths: Vec<f32>,
    caret_stops: Vec<(usize, f32)>,
}

impl TextViewLayout {
    /// Compute all visual line metrics for a text view.
    ///
    /// # Arguments
    ///
    /// * `document` - Text document to measure.
    /// * `font_size` - Text font size in logical pixels.
    /// * `padding` - Uniform widget padding.
    /// * `wrap_mode` - Wrapping mode used for visual lines.
    /// * `size` - Laid-out widget size.
    /// * `scroll` - Current scroll offset.
    /// * `show_line_numbers` - Whether to reserve gutter space.
    ///
    /// # Returns
    ///
    /// Computed text view layout data.
    pub fn compute(
        document: &TextDocument,
        font_size: f32,
        padding: f32,
        wrap_mode: WrapMode,
        size: Size,
        scroll: TextViewScroll,
        show_line_numbers: bool,
    ) -> Self {
        Self::compute_with_font_stack(
            document,
            font_size,
            padding,
            wrap_mode,
            size,
            scroll,
            show_line_numbers,
            None,
        )
    }

    pub(crate) fn compute_with_font_stack(
        document: &TextDocument,
        font_size: f32,
        padding: f32,
        wrap_mode: WrapMode,
        size: Size,
        scroll: TextViewScroll,
        show_line_numbers: bool,
        font_stack: Option<&Arc<graphics::FontStack>>,
    ) -> Self {
        let line_height = match font_stack {
            Some(stack) => graphics::line_height_sized_with_font_stack(font_size, stack),
            None => graphics::line_height_sized(font_size),
        }
        .max(1) as f32;
        let gutter_width = if show_line_numbers {
            let digits = document
                .line_count()
                .max(1)
                .to_string()
                .len()
                .max(MIN_LINE_NUMBER_DIGITS);
            // Reserve a stable column before editing begins. Measuring the
            // current line count also jitters with proportional digits (11/88).
            let digit_width = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]
                .into_iter()
                .map(|digit| match font_stack {
                    Some(stack) => {
                        graphics::measure_text_sized_with_font_stack(digit, font_size, stack).0
                    }
                    None => graphics::measure_text_sized(digit, font_size).0,
                })
                .max()
                .unwrap_or(0);
            digit_width as f32 * digits as f32 + GUTTER_PADDING
        } else {
            0.0
        };
        // Gutter measurement can discover fallback fonts too. Capture the
        // complete stack after that work so glyph and cached caret widths agree.
        let measure = match font_stack {
            Some(stack) => graphics::TextMeasure::with_font_stack(font_size, stack),
            None => graphics::TextMeasure::new(&document.as_str(), font_size),
        };
        let text_origin_x = padding + gutter_width;
        let viewport_width = (size.width - padding * 2.0 - gutter_width).max(1.0);
        let viewport_height = (size.height - padding * 2.0).max(1.0);
        let mut visual_lines = Vec::new();

        if document.line_count() == 0 {
            visual_lines.push(VisualLine::new(
                0,
                0..0,
                String::new(),
                text_origin_x - scroll.x,
                padding,
                0.0,
                alloc::vec![0],
                &measure,
            ));
        } else {
            for logical_line in 0..document.line_count() {
                let range = document.line_range(logical_line).unwrap_or(0..0);
                let line_text = document
                    .line_text(logical_line)
                    .unwrap_or(Cow::Borrowed(""));
                let display = strip_line_ending(&line_text);
                match wrap_mode {
                    WrapMode::None => push_visual_line(
                        &mut visual_lines,
                        logical_line,
                        range.start..range.start + display.len(),
                        display.to_string(),
                        text_origin_x - scroll.x,
                        padding,
                        &measure,
                    ),
                    WrapMode::Soft => push_wrapped_lines(
                        &mut visual_lines,
                        logical_line,
                        range.start,
                        display,
                        text_origin_x - scroll.x,
                        padding,
                        viewport_width,
                        &measure,
                    ),
                }
            }
        }

        for (index, line) in visual_lines.iter_mut().enumerate() {
            line.y = padding + index as f32 * line_height - scroll.y;
        }

        let max_line_width = visual_lines
            .iter()
            .map(|line| line.width)
            .fold(0.0, f32::max);
        let content_width = match wrap_mode {
            WrapMode::None => max_line_width,
            WrapMode::Soft => viewport_width,
        };
        let content_height = visual_lines.len() as f32 * line_height;
        let clip_scroll_y = scroll.y - padding + BORDER_WIDTH;
        let clip_height = (size.height - BORDER_WIDTH * 2.0).max(1.0);
        let visible_lines =
            visible_range(clip_scroll_y, clip_height, line_height, visual_lines.len());

        Self {
            line_height,
            content_width,
            content_height,
            max_line_width,
            gutter_width,
            visible_lines,
            visual_lines,
            font_size,
            padding,
            viewport_width,
            viewport_height,
            text_origin_x,
            scroll,
            document: document.clone(),
            wrap_mode,
            size,
            show_line_numbers,
            font_revision: measure.font_revision(),
            font_stack: font_stack.cloned(),
        }
    }

    pub(crate) fn measure_text(&self, text: &str) -> (u32, u32) {
        match &self.font_stack {
            Some(stack) => {
                graphics::measure_text_sized_with_font_stack(text, self.font_size, stack)
            }
            None => graphics::measure_text_sized(text, self.font_size),
        }
    }

    pub(crate) fn draw_text(
        &self,
        ctx: &mut PaintContext,
        position: Point,
        text: impl Into<String>,
        color: Color,
    ) {
        match &self.font_stack {
            Some(stack) => ctx.draw_text_with_font_stack(
                position,
                text,
                color,
                self.font_size,
                Arc::clone(stack),
            ),
            None => ctx.draw_text(position, text, color, self.font_size),
        }
    }

    /// Reuse measured text unless a layout input actually changed. Scrolling
    /// changes coordinates and the visible range, never glyph measurement.
    pub(crate) fn matches(
        &self,
        document: &TextDocument,
        font_size: f32,
        padding: f32,
        wrap_mode: WrapMode,
        size: Size,
        show_line_numbers: bool,
    ) -> bool {
        self.matches_with_font_stack(
            document,
            font_size,
            padding,
            wrap_mode,
            size,
            show_line_numbers,
            None,
        )
    }

    pub(crate) fn matches_with_font_stack(
        &self,
        document: &TextDocument,
        font_size: f32,
        padding: f32,
        wrap_mode: WrapMode,
        size: Size,
        show_line_numbers: bool,
        font_stack: Option<&Arc<graphics::FontStack>>,
    ) -> bool {
        self.document.same_revision(document)
            && self.font_size == font_size
            && self.padding == padding
            && self.wrap_mode == wrap_mode
            && (wrap_mode == WrapMode::None || self.size.width == size.width)
            && self.show_line_numbers == show_line_numbers
            && self.font_stack.is_some() == font_stack.is_some()
            && self.font_revision
                == font_stack.map_or_else(graphics::text_font_revision, |stack| stack.cache_id())
    }

    pub(crate) fn set_viewport(&mut self, size: Size) {
        if self.size == size {
            return;
        }
        self.size = size;
        self.viewport_width = (size.width - self.padding * 2.0 - self.gutter_width).max(1.0);
        self.viewport_height = (size.height - self.padding * 2.0).max(1.0);
        if self.wrap_mode == WrapMode::Soft {
            self.content_width = self.viewport_width;
        }
        self.visible_lines = visible_range(
            self.scroll.y - self.padding + BORDER_WIDTH,
            (size.height - BORDER_WIDTH * 2.0).max(1.0),
            self.line_height,
            self.visual_lines.len(),
        );
    }

    pub(crate) fn set_scroll(&mut self, requested: TextViewScroll) -> TextViewScroll {
        let scroll = self.clamp_scroll(requested);
        if scroll != self.scroll {
            for (index, line) in self.visual_lines.iter_mut().enumerate() {
                line.x = self.text_origin_x - scroll.x;
                line.y = self.padding + index as f32 * self.line_height - scroll.y;
            }
            self.scroll = scroll;
            self.visible_lines = visible_range(
                scroll.y - self.padding + BORDER_WIDTH,
                (self.size.height - BORDER_WIDTH * 2.0).max(1.0),
                self.line_height,
                self.visual_lines.len(),
            );
        }
        scroll
    }

    /// Convert a widget-local point to a text position.
    ///
    /// # Arguments
    ///
    /// * `point` - Point in widget-local coordinates.
    ///
    /// # Returns
    ///
    /// The closest UTF-8 byte text position, or `None` outside the widget content.
    pub fn hit_test(&self, point: Point) -> Option<TextPosition> {
        if point.x < self.text_origin_x || point.y < self.padding {
            return None;
        }
        let visual_index = ((point.y + self.scroll.y - self.padding) / self.line_height) as usize;
        let line = self.visual_lines.get(visual_index)?;
        let local_x = (point.x - line.x).max(0.0);
        Some(TextPosition::new(byte_for_x(line, local_x)))
    }

    /// Compute the caret rectangle for a text position.
    ///
    /// # Arguments
    ///
    /// * `position` - Text position to locate.
    /// * `document` - Source document used to clamp the byte offset.
    ///
    /// # Returns
    ///
    /// A widget-local caret rectangle.
    pub fn cursor_rect(&self, position: TextPosition, document: &TextDocument) -> Rect {
        let byte = position.byte.min(document.len());
        let line_index = self
            .visual_lines
            .partition_point(|line| line.text_range.end < byte)
            .min(self.visual_lines.len().saturating_sub(1));
        let line = &self.visual_lines[line_index];
        let x = line.x + line.width_to_byte(byte);
        Rect::from_xywh(x, line.y, 1.0, self.line_height)
    }

    /// Return which logical line contains a byte offset.
    ///
    /// # Arguments
    ///
    /// * `byte_offset` - Absolute document byte offset.
    ///
    /// # Returns
    ///
    /// Zero-based logical line index.
    pub fn line_index_at_byte(&self, byte_offset: usize) -> usize {
        let index = self
            .visual_lines
            .partition_point(|line| line.text_range.end < byte_offset)
            .min(self.visual_lines.len().saturating_sub(1));
        self.visual_lines
            .get(index)
            .map(|line| line.logical_line)
            .unwrap_or(0)
    }

    /// Resolve a byte offset on a specific visual line at a given x position.
    ///
    /// Unlike [`hit_test`](Self::hit_test), this bypasses y-to-index
    /// conversion entirely, avoiding floating-point precision errors at line
    /// boundaries.
    ///
    /// # Arguments
    ///
    /// * `line_index` - Visual line index.
    /// * `x` - Widget-local x coordinate.
    ///
    /// # Returns
    ///
    /// The closest document byte offset, or `None` if the index is out of
    /// bounds.
    pub(crate) fn byte_at_line_x(&self, line_index: usize, x: f32) -> Option<usize> {
        let line = self.visual_lines.get(line_index)?;
        let local_x = (x - line.x).max(0.0);
        Some(byte_for_x(line, local_x))
    }

    /// Clamp a scroll offset to the measured content bounds.
    ///
    /// # Arguments
    ///
    /// * `scroll` - Requested scroll offset.
    ///
    /// # Returns
    ///
    /// Scroll offset constrained to valid horizontal and vertical ranges.
    pub fn clamp_scroll(&self, scroll: TextViewScroll) -> TextViewScroll {
        TextViewScroll {
            x: scroll
                .x
                .clamp(0.0, (self.content_width - self.viewport_width).max(0.0)),
            y: scroll
                .y
                .clamp(0.0, (self.content_height - self.viewport_height).max(0.0)),
        }
    }
}

impl VisualLine {
    fn new(
        logical_line: usize,
        text_range: Range<usize>,
        text: String,
        x: f32,
        y: f32,
        width: f32,
        document_to_display_offsets: Vec<usize>,
        measure: &graphics::TextMeasure,
    ) -> Self {
        let mut prefix_widths = alloc::vec![0.0; text.len() + 1];
        let mut advance = 0.0;
        for (byte, ch) in text.char_indices() {
            let start_width = graphics::TextMeasure::rounded_width(advance);
            prefix_widths[byte..byte + ch.len_utf8()].fill(start_width);
            advance += measure.advance(ch);
            prefix_widths[byte + ch.len_utf8()] = graphics::TextMeasure::rounded_width(advance);
        }
        let mut line = Self {
            logical_line,
            text_range,
            x,
            y,
            width,
            text,
            document_to_display_offsets,
            prefix_widths,
            caret_stops: Vec::new(),
        };
        line.width = *line.prefix_widths.last().unwrap_or(&0.0);
        line.caret_stops.push((line.text_range.start, 0.0));
        for (offset, grapheme) in line.text.grapheme_indices(true) {
            let end = offset + grapheme.len();
            line.caret_stops.push((
                line.document_byte_for_display_byte(end),
                line.prefix_widths[end],
            ));
        }
        line
    }

    pub(crate) fn width_to_byte(&self, byte: usize) -> f32 {
        self.prefix_widths[self.display_byte_for_document_byte(byte)]
    }

    pub(crate) fn width_to_display_byte(&self, byte: usize) -> f32 {
        self.prefix_widths[byte]
    }

    /// Keep one neighboring scalar at each edge for glyph overhang. The clip
    /// removes its offscreen pixels; distant text never reaches the renderer.
    pub(crate) fn visible_display_range(&self, left: f32, right: f32) -> Range<usize> {
        let mut start = self
            .prefix_widths
            .partition_point(|width| *width < left)
            .saturating_sub(1)
            .min(self.text.len());
        while start > 0 && !self.text.is_char_boundary(start) {
            start -= 1;
        }
        if start > 0 {
            start -= 1;
            while !self.text.is_char_boundary(start) {
                start -= 1;
            }
        }
        let mut end = self
            .prefix_widths
            .partition_point(|width| *width <= right)
            .min(self.text.len());
        while end < self.text.len() && !self.text.is_char_boundary(end) {
            end += 1;
        }
        if end < self.text.len() {
            end += self.text[end..].chars().next().unwrap().len_utf8();
        }
        start..end
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn contains_byte(&self, byte: usize) -> bool {
        if self.text_range.is_empty() {
            byte == self.text_range.start
        } else {
            byte >= self.text_range.start && byte <= self.text_range.end
        }
    }

    pub(crate) fn display_byte_for_document_byte(&self, document_byte: usize) -> usize {
        let local = document_byte
            .saturating_sub(self.text_range.start)
            .min(self.text_range.end.saturating_sub(self.text_range.start));
        self.document_to_display_offsets
            .get(local)
            .copied()
            .unwrap_or(self.text.len())
            .min(self.text.len())
    }

    pub(crate) fn display_end_for_document_byte(&self, document_byte: usize) -> usize {
        let byte = self.display_byte_for_document_byte(document_byte);
        let local = document_byte
            .saturating_sub(self.text_range.start)
            .min(self.text_range.len());
        if local > 0
            && self.document_to_display_offsets[local - 1]
                == self.document_to_display_offsets[local]
            && byte < self.text.len()
        {
            byte + self.text[byte..].chars().next().unwrap().len_utf8()
        } else {
            byte
        }
    }

    fn document_byte_for_display_byte(&self, display_byte: usize) -> usize {
        let display_byte = display_byte.min(self.text.len());
        let local = self
            .document_to_display_offsets
            .partition_point(|offset| *offset < display_byte);
        self.text_range.start + local.min(self.text_range.end.saturating_sub(self.text_range.start))
    }
}

fn push_visual_line(
    lines: &mut Vec<VisualLine>,
    logical_line: usize,
    range: Range<usize>,
    text: String,
    x: f32,
    y: f32,
    measure: &graphics::TextMeasure,
) {
    let (display_text, offsets) = expand_tabs_with_offsets(&text, DEFAULT_TAB_WIDTH);
    lines.push(VisualLine::new(
        logical_line,
        range,
        display_text,
        x,
        y,
        0.0,
        offsets,
        measure,
    ));
}

fn push_wrapped_lines(
    lines: &mut Vec<VisualLine>,
    logical_line: usize,
    line_start: usize,
    text: &str,
    x: f32,
    y: f32,
    max_width: f32,
    measure: &graphics::TextMeasure,
) {
    if text.is_empty() {
        push_visual_line(
            lines,
            logical_line,
            line_start..line_start,
            String::new(),
            x,
            y,
            measure,
        );
        return;
    }
    // Measure scalars once. Each candidate row then visits advances, rather
    // than repeatedly expanding and remeasuring its growing text prefix.
    let mut advances = Vec::with_capacity(text.len());
    let graphemes: Vec<_> = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| {
            let advance_start = advances.len();
            for ch in grapheme.chars() {
                let (ch, repeats) = if ch == '\t' {
                    (' ', DEFAULT_TAB_WIDTH)
                } else {
                    (ch, 1)
                };
                advances.extend(core::iter::repeat_n(measure.advance(ch), repeats));
            }
            (
                start,
                start + grapheme.len(),
                grapheme == " " || grapheme == "\t",
                advance_start..advances.len(),
            )
        })
        .collect();
    let mut start = 0;
    while start < graphemes.len() {
        let mut end = start;
        let mut last_space = None;
        let mut advance = 0.0;
        while end < graphemes.len() {
            for width in &advances[graphemes[end].3.clone()] {
                advance += width;
            }
            if graphics::TextMeasure::rounded_width(advance) > max_width && end > start {
                end = last_space.unwrap_or(end);
                break;
            }
            if graphemes[end].2 {
                last_space = Some(end + 1);
            }
            end += 1;
        }
        let byte_start = graphemes[start].0;
        let byte_end = graphemes[end - 1].1;
        push_visual_line(
            lines,
            logical_line,
            line_start + byte_start..line_start + byte_end,
            text[byte_start..byte_end].to_string(),
            x,
            y,
            measure,
        );
        start = end;
    }
}

fn byte_for_x(line: &VisualLine, x: f32) -> usize {
    if x <= 0.0 {
        return line.text_range.start;
    }
    let mut left = 0;
    let mut right = line.caret_stops.len() - 1;
    while left < right {
        let middle = left + (right - left) / 2;
        let a = line.caret_stops[middle].1;
        let b = line.caret_stops[middle + 1].1;
        if x >= a + (b - a) * 0.5 {
            left = middle + 1;
        } else {
            right = middle;
        }
    }
    line.caret_stops[left].0
}

fn expand_tabs_with_offsets(text: &str, width: usize) -> (String, Vec<usize>) {
    let mut display_text = String::with_capacity(text.len());
    let mut offsets = alloc::vec![0usize; text.len() + 1];
    for (byte, character) in text.char_indices() {
        offsets[byte..byte + character.len_utf8()].fill(display_text.len());
        if character == '\t' {
            for _ in 0..width {
                display_text.push(' ');
            }
        } else {
            display_text.push(character);
        }
        offsets[byte + character.len_utf8()] = display_text.len();
    }
    (display_text, offsets)
}

fn visible_range(
    scroll_y: f32,
    viewport_height: f32,
    line_height: f32,
    line_count: usize,
) -> Range<usize> {
    if line_count == 0 {
        return 0..0;
    }
    let first = (libm::floorf(scroll_y / line_height).max(0.0) as usize).min(line_count);
    let end = libm::ceilf((scroll_y + viewport_height) / line_height)
        .max(first.saturating_add(1) as f32) as usize;
    first..end.min(line_count)
}

fn strip_line_ending(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text)
}
