//! Paint helpers for the multi-line text view.

use alloc::string::ToString;

use super::layout::{TextViewLayout, VisualLine};
use super::{TextDocument, TextPosition, TextSelection};
use crate::color::Color;
use crate::geometry::{Point, Rect, Size};
use crate::graphics;
use crate::renderer::PaintContext;

const PREEDIT_STYLE_HIGHLIGHT: u32 = 1 << 2;
const PREEDIT_STYLE_SELECTED: u32 = 1 << 3;
const PREEDIT_STYLE_TARGET_CONVERTING: u32 = 1 << 5;

/// Paint a laid-out text view into a paint context.
///
/// # Arguments
///
/// * `ctx` - Paint command context to append commands to.
/// * `origin` - Absolute origin of the text view.
/// * `size` - Laid-out widget size.
/// * `layout` - Precomputed layout data.
/// * `document` - Text document to draw.
/// * `selection` - Current caret and selection state.
/// * `focused` - Whether the view has keyboard focus.
/// * `font_size` - Text font size in logical pixels.
/// * `padding` - Uniform widget padding.
/// * `background_color` - Widget background color.
/// * `text_color` - Main text color.
/// * `placeholder_color` - Placeholder and line number color.
/// * `selection_color` - Selection highlight color.
/// * `current_line_color` - Current-line highlight color.
/// * `border_color` - Border color used while unfocused.
/// * `focused_border_color` - Border color used while focused.
/// * `placeholder` - Placeholder text drawn for an empty document.
/// * `preedit` - Active IME preedit text.
/// * `preedit_cursor_byte` - Cursor byte within the preedit text.
/// * `preedit_spans` - Platform preedit style spans.
/// * `show_line_numbers` - Whether to draw the gutter and line numbers.
/// * `highlight_current_line` - Whether to highlight the caret logical line.
pub fn paint_text_view(
    ctx: &mut PaintContext,
    origin: Point,
    size: Size,
    layout: &TextViewLayout,
    document: &TextDocument,
    selection: TextSelection,
    focused: bool,
    font_size: f32,
    padding: f32,
    background_color: Color,
    text_color: Color,
    placeholder_color: Color,
    selection_color: Color,
    current_line_color: Color,
    border_color: Color,
    focused_border_color: Color,
    placeholder: &str,
    preedit: &str,
    preedit_cursor_byte: u32,
    preedit_spans: &[u8],
    show_line_numbers: bool,
    highlight_current_line: bool,
) {
    let bounds = Rect::new(origin, size);
    ctx.fill_rect(bounds, background_color);

    let content_rect = Rect::from_xywh(
        origin.x + padding,
        origin.y + padding,
        (size.width - padding * 2.0).max(0.0),
        (size.height - padding * 2.0).max(0.0),
    );
    ctx.push_clip(content_rect);

    if highlight_current_line && focused {
        paint_current_line(ctx, origin, size, layout, selection, current_line_color);
    }
    if let Some(range) = selection.normalized_range() {
        paint_selection(ctx, origin, layout, range, selection_color);
    }
    if show_line_numbers {
        paint_line_numbers(
            ctx,
            origin,
            layout,
            font_size,
            padding,
            placeholder_color,
            background_color,
        );
    }
    paint_visible_text(
        ctx,
        origin,
        layout,
        document,
        placeholder,
        text_color,
        placeholder_color,
        font_size,
    );
    if focused && !preedit.is_empty() {
        paint_preedit(
            ctx,
            origin,
            layout,
            document,
            selection.caret,
            preedit,
            preedit_spans,
            font_size,
            text_color,
            focused_border_color,
        );
    }
    if focused {
        let mut caret = layout.cursor_rect(selection.caret, document);
        if !preedit.is_empty() {
            let prefix = preedit_prefix(preedit, preedit_cursor_byte);
            caret.origin.x += graphics::measure_text_sized(prefix, font_size).0 as f32;
        }
        caret.origin.x += origin.x;
        caret.origin.y += origin.y;
        ctx.fill_rect(caret, text_color);
    }

    ctx.pop_clip();
    let border = if focused {
        focused_border_color
    } else {
        border_color
    };
    ctx.stroke_rect(bounds, 1.0, border);
}

fn paint_current_line(
    ctx: &mut PaintContext,
    origin: Point,
    size: Size,
    layout: &TextViewLayout,
    selection: TextSelection,
    color: Color,
) {
    let logical_line = layout.line_index_at_byte(selection.caret.byte);
    for line in layout
        .visual_lines
        .iter()
        .filter(|line| line.logical_line == logical_line)
    {
        ctx.fill_rect(
            Rect::from_xywh(origin.x, origin.y + line.y, size.width, layout.line_height),
            color,
        );
    }
}

fn paint_selection(
    ctx: &mut PaintContext,
    origin: Point,
    layout: &TextViewLayout,
    range: core::ops::Range<usize>,
    color: Color,
) {
    for line in visible_lines(layout) {
        let start = range.start.max(line.text_range.start);
        let end = range.end.min(line.text_range.end);
        if start >= end {
            continue;
        }
        let start_x = line.x + width_to_byte(line, start, layout.font_size);
        let end_x = line.x + width_to_byte(line, end, layout.font_size);
        ctx.fill_rect(
            Rect::from_xywh(
                origin.x + start_x,
                origin.y + line.y,
                (end_x - start_x).max(1.0),
                layout.line_height,
            ),
            color,
        );
    }
}

fn paint_line_numbers(
    ctx: &mut PaintContext,
    origin: Point,
    layout: &TextViewLayout,
    font_size: f32,
    padding: f32,
    color: Color,
    background_color: Color,
) {
    ctx.fill_rect(
        Rect::from_xywh(
            origin.x + padding,
            origin.y + padding,
            layout.gutter_width,
            layout.viewport_height,
        ),
        background_color,
    );
    let mut last_logical = None;
    for line in visible_lines(layout) {
        if last_logical == Some(line.logical_line) {
            continue;
        }
        last_logical = Some(line.logical_line);
        let label = (line.logical_line + 1).to_string();
        let label_width = graphics::measure_text_sized(&label, font_size).0 as f32;
        let x = origin.x + padding + (layout.gutter_width - label_width - 6.0).max(0.0);
        ctx.draw_text(Point::new(x, origin.y + line.y), label, color, font_size);
    }
}

fn paint_visible_text(
    ctx: &mut PaintContext,
    origin: Point,
    layout: &TextViewLayout,
    document: &TextDocument,
    placeholder: &str,
    text_color: Color,
    placeholder_color: Color,
    font_size: f32,
) {
    if document.len() == 0 && !placeholder.is_empty() {
        if let Some(line) = layout.visual_lines.first() {
            ctx.draw_text(
                Point::new(origin.x + line.x, origin.y + line.y),
                placeholder,
                placeholder_color,
                font_size,
            );
        }
        return;
    }
    for line in visible_lines(layout) {
        ctx.draw_text(
            Point::new(origin.x + line.x, origin.y + line.y),
            line.text(),
            text_color,
            font_size,
        );
    }
}

fn paint_preedit(
    ctx: &mut PaintContext,
    origin: Point,
    layout: &TextViewLayout,
    document: &TextDocument,
    position: TextPosition,
    preedit: &str,
    spans: &[u8],
    font_size: f32,
    text_color: Color,
    active_color: Color,
) {
    let caret = layout.cursor_rect(position, document);
    let x = origin.x + caret.origin.x;
    let y = origin.y + caret.origin.y;
    paint_preedit_marks(ctx, x, y, preedit, spans, font_size, active_color);
    ctx.draw_text(Point::new(x, y), preedit, text_color, font_size);
}

fn paint_preedit_marks(
    ctx: &mut PaintContext,
    x: f32,
    y: f32,
    preedit: &str,
    spans: &[u8],
    font_size: f32,
    active_color: Color,
) {
    if spans.is_empty() {
        paint_preedit_mark_span(
            ctx,
            x,
            y,
            preedit,
            0,
            preedit.len(),
            false,
            font_size,
            active_color,
        );
        return;
    }

    let mut offset = 0usize;
    while offset + 12 <= spans.len() {
        let start = u32::from_le_bytes([
            spans[offset],
            spans[offset + 1],
            spans[offset + 2],
            spans[offset + 3],
        ]);
        let length = u32::from_le_bytes([
            spans[offset + 4],
            spans[offset + 5],
            spans[offset + 6],
            spans[offset + 7],
        ]);
        let style = u32::from_le_bytes([
            spans[offset + 8],
            spans[offset + 9],
            spans[offset + 10],
            spans[offset + 11],
        ]);
        let start = clamp_byte_boundary(preedit, start) as usize;
        let end =
            clamp_byte_boundary(preedit, start.saturating_add(length as usize) as u32) as usize;
        if start < end {
            let active = style
                & (PREEDIT_STYLE_HIGHLIGHT
                    | PREEDIT_STYLE_SELECTED
                    | PREEDIT_STYLE_TARGET_CONVERTING)
                != 0;
            paint_preedit_mark_span(
                ctx,
                x,
                y,
                preedit,
                start,
                end,
                active,
                font_size,
                active_color,
            );
        }
        offset += 12;
    }
}

fn paint_preedit_mark_span(
    ctx: &mut PaintContext,
    x: f32,
    y: f32,
    preedit: &str,
    start: usize,
    end: usize,
    active: bool,
    font_size: f32,
    active_color: Color,
) {
    let prefix_width = graphics::measure_text_sized(&preedit[..start], font_size).0 as f32;
    let span_width = graphics::measure_text_sized(&preedit[start..end], font_size).0 as f32;
    let underline_x = x + prefix_width;
    let underline_y = (y + font_size * 1.15).max(0.0);
    let thickness = if active { 3.0 } else { 1.0 };
    let color = if active {
        active_color
    } else {
        Color::rgb(150u8, 158u8, 170u8)
    };
    if active {
        ctx.fill_rect(
            Rect::from_xywh(
                underline_x,
                y - 1.0,
                span_width.max(1.0),
                (font_size * 1.25).max(1.0),
            ),
            Color::rgba(218u8, 232u8, 255u8, 0.95),
        );
    }
    ctx.fill_rect(
        Rect::from_xywh(underline_x, underline_y, span_width.max(1.0), thickness),
        color,
    );
}

fn visible_lines(layout: &TextViewLayout) -> impl Iterator<Item = &VisualLine> {
    layout.visual_lines[layout.visible_lines.clone()].iter()
}

fn width_to_byte(line: &VisualLine, byte: usize, font_size: f32) -> f32 {
    let display_byte = line.display_byte_for_document_byte(byte);
    let prefix = clamp_str_prefix(line.text(), display_byte);
    graphics::measure_text_sized(prefix, font_size).0 as f32
}

fn preedit_prefix(preedit: &str, byte: u32) -> &str {
    let byte = clamp_byte_boundary(preedit, byte) as usize;
    &preedit[..byte]
}

fn clamp_byte_boundary(text: &str, byte: u32) -> u32 {
    let mut byte = (byte as usize).min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte as u32
}

fn clamp_str_prefix(text: &str, byte: usize) -> &str {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    &text[..byte]
}
