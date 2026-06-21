//! Layout computation for the multi-line text view.

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::{TextDocument, TextPosition, TextViewScroll, WrapMode};
use crate::geometry::{Point, Rect, Size};
use crate::graphics;

const GUTTER_PADDING: f32 = 12.0;
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
        let line_height = font_size * 1.2;
        let gutter_width = if show_line_numbers {
            let digits = document.line_count().max(1).to_string();
            graphics::measure_text_sized(&digits, font_size).0 as f32 + GUTTER_PADDING
        } else {
            0.0
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
                text_origin_x,
                padding,
                0.0,
                alloc::vec![0],
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
                        font_size,
                    ),
                    WrapMode::Soft => push_wrapped_lines(
                        &mut visual_lines,
                        logical_line,
                        range.start,
                        display,
                        text_origin_x,
                        padding,
                        viewport_width,
                        font_size,
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
        let visible_lines =
            visible_range(scroll.y, viewport_height, line_height, visual_lines.len());

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
        }
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
        Some(TextPosition::new(byte_for_x(line, local_x, self.font_size)))
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
            .iter()
            .position(|line| byte >= line.text_range.start && byte <= line.text_range.end)
            .unwrap_or_else(|| self.visual_lines.len().saturating_sub(1));
        let line = &self.visual_lines[line_index];
        let clamped = byte.clamp(line.text_range.start, line.text_range.end);
        let prefix_end = line.display_byte_for_document_byte(clamped);
        let prefix = clamp_str_prefix(&line.text, prefix_end);
        let x = line.x + graphics::measure_text_sized(prefix, self.font_size).0 as f32;
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
        self.visual_lines
            .iter()
            .find(|line| byte_offset >= line.text_range.start && byte_offset <= line.text_range.end)
            .map(|line| line.logical_line)
            .unwrap_or_else(|| {
                self.visual_lines
                    .last()
                    .map(|line| line.logical_line)
                    .unwrap_or(0)
            })
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
    ) -> Self {
        Self {
            logical_line,
            text_range,
            x,
            y,
            width,
            text,
            document_to_display_offsets,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
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

    fn document_byte_for_display_byte(&self, display_byte: usize) -> usize {
        let display_byte = display_byte.min(self.text.len());
        let local = self
            .document_to_display_offsets
            .iter()
            .position(|offset| *offset >= display_byte)
            .unwrap_or_else(|| self.text_range.end.saturating_sub(self.text_range.start));
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
    font_size: f32,
) {
    let (display_text, document_to_display_offsets) =
        expand_tabs_with_offsets(&text, DEFAULT_TAB_WIDTH);
    let width = graphics::measure_text_sized(&display_text, font_size).0 as f32;
    lines.push(VisualLine::new(
        logical_line,
        range,
        display_text,
        x,
        y,
        width,
        document_to_display_offsets,
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
    font_size: f32,
) {
    if text.is_empty() {
        push_visual_line(
            lines,
            logical_line,
            line_start..line_start,
            String::new(),
            x,
            y,
            font_size,
        );
        return;
    }

    let mut start = 0usize;
    while start < text.len() {
        let end = wrapped_line_end(&text[start..], max_width, font_size) + start;
        let end = if end == start {
            next_grapheme_end(text, start)
        } else {
            end
        };
        let fragment = text[start..end].to_string();
        push_visual_line(
            lines,
            logical_line,
            line_start + start..line_start + end,
            fragment,
            x,
            y,
            font_size,
        );
        start = end;
        while start < text.len() && text[start..].starts_with(' ') {
            start += 1;
        }
    }
}

fn wrapped_line_end(text: &str, max_width: f32, font_size: f32) -> usize {
    let mut last_fit = 0usize;
    let mut last_space_fit = 0usize;
    for (offset, grapheme) in text.grapheme_indices(true) {
        let end = offset + grapheme.len();
        let width = graphics::measure_text_sized(
            &expand_tabs(&text[..end], DEFAULT_TAB_WIDTH),
            font_size,
        )
        .0 as f32;
        if width > max_width && last_fit > 0 {
            return if last_space_fit > 0 {
                last_space_fit
            } else {
                last_fit
            };
        }
        last_fit = end;
        if grapheme == " " || grapheme == "\t" {
            last_space_fit = end;
        }
    }
    text.len()
}

fn next_grapheme_end(text: &str, start: usize) -> usize {
    text[start..]
        .grapheme_indices(true)
        .map(|(offset, grapheme)| start + offset + grapheme.len())
        .next()
        .unwrap_or(text.len())
}

fn byte_for_x(line: &VisualLine, x: f32, font_size: f32) -> usize {
    if x <= 0.0 {
        return line.text_range.start;
    }
    let mut previous_byte = line.text_range.start;
    let mut previous_width = 0.0;
    for (offset, grapheme) in line.text.grapheme_indices(true) {
        let end = offset + grapheme.len();
        let width = graphics::measure_text_sized(&line.text[..end], font_size).0 as f32;
        let midpoint = previous_width + (width - previous_width) * 0.5;
        if x < midpoint {
            return previous_byte;
        }
        // VisualLine.text contains expanded tabs, so map display byte offsets back to the
        // original document bytes. Hit testing inside an expanded tab resolves to the nearest
        // original tab boundary, which is an accepted MVP limitation.
        previous_byte = line.document_byte_for_display_byte(end);
        previous_width = width;
    }
    line.text_range.end
}

fn expand_tabs(text: &str, width: usize) -> String {
    let (display_text, _) = expand_tabs_with_offsets(text, width);
    display_text
}

fn expand_tabs_with_offsets(text: &str, width: usize) -> (String, Vec<usize>) {
    let mut display_text = String::new();
    let mut offsets = alloc::vec![0usize; text.len() + 1];

    for (byte, character) in text.char_indices() {
        offsets[byte] = display_text.len();
        if character == '\t' {
            for _ in 0..width {
                display_text.push(' ');
            }
        } else {
            display_text.push(character);
        }
        offsets[byte + character.len_utf8()] = display_text.len();
    }
    offsets[text.len()] = display_text.len();

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
    let first = libm::floorf(scroll_y / line_height).max(0.0) as usize;
    let visible_count = libm::ceilf(viewport_height / line_height).max(1.0) as usize;
    first.saturating_sub(1)..(first + visible_count + 2).min(line_count)
}

fn strip_line_ending(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text)
}

fn clamp_str_prefix(text: &str, byte: usize) -> &str {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    &text[..byte]
}
