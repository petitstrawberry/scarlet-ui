//! Keyboard, IME, and focus editing support for [`TextView`].

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use core::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::{BORDER_WIDTH, TabMode, TextPosition, TextSelection, TextView, TextViewRenderObject};
use crate::event::{Event, FocusEvent, KeyCode, KeyEvent, MouseButton, MouseEvent};
use crate::geometry::Point;

enum CaretMove {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    LineStart,
    LineEnd,
    DocumentStart,
    DocumentEnd,
}

/// Handle a keyboard event for a focused text view.
///
/// # Arguments
///
/// * `view` - Source text view containing bindings and callbacks.
/// * `render_object` - Mutable render object holding current document state.
/// * `event` - Keyboard event to process.
///
/// # Returns
///
/// `true` when the event was handled by text editing.
pub(crate) fn handle_text_view_keyboard(
    view: &TextView,
    render_object: &mut TextViewRenderObject,
    event: KeyEvent,
) -> bool {
    if !render_object.is_focused() {
        return false;
    }

    match event {
        KeyEvent::Char { c } if !c.is_control() => {
            render_object.clear_preedit();
            let mut text = String::new();
            text.push(c);
            insert_text(view, render_object, &text);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Char(key),
            modifiers,
        } if modifiers.primary() => handle_primary_shortcut(view, render_object, key),
        KeyEvent::Pressed {
            keycode: KeyCode::Backspace,
            ..
        } => {
            render_object.clear_preedit();
            delete_backward(view, render_object);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Delete,
            ..
        } => {
            render_object.clear_preedit();
            delete_forward(view, render_object);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Enter,
            ..
        } => {
            render_object.clear_preedit();
            insert_text(view, render_object, "\n");
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Tab,
            ..
        } => {
            render_object.clear_preedit();
            let text = match render_object.tab_mode {
                TabMode::Tab => Cow::Borrowed("\t"),
                TabMode::Spaces(count) => {
                    let mut spaces = String::new();
                    for _ in 0..count {
                        spaces.push(' ');
                    }
                    Cow::Owned(spaces)
                }
            };
            insert_text(view, render_object, &text);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Space,
            ..
        } => {
            render_object.clear_preedit();
            insert_text(view, render_object, " ");
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Left,
            modifiers,
        } => {
            move_caret(render_object, CaretMove::Left, modifiers.shift);
            finish_caret_change(view, render_object);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Right,
            modifiers,
        } => {
            move_caret(render_object, CaretMove::Right, modifiers.shift);
            finish_caret_change(view, render_object);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Up,
            modifiers,
        } => {
            move_caret(render_object, CaretMove::Up, modifiers.shift);
            finish_caret_change(view, render_object);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Down,
            modifiers,
        } => {
            move_caret(render_object, CaretMove::Down, modifiers.shift);
            finish_caret_change(view, render_object);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Home,
            modifiers,
        } => {
            let movement = if modifiers.primary() {
                CaretMove::DocumentStart
            } else {
                CaretMove::LineStart
            };
            move_caret(render_object, movement, modifiers.shift);
            finish_caret_change(view, render_object);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::End,
            modifiers,
        } => {
            let movement = if modifiers.primary() {
                CaretMove::DocumentEnd
            } else {
                CaretMove::LineEnd
            };
            move_caret(render_object, movement, modifiers.shift);
            finish_caret_change(view, render_object);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::PageUp,
            modifiers,
        } => {
            move_caret(render_object, CaretMove::PageUp, modifiers.shift);
            finish_caret_change(view, render_object);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::PageDown,
            modifiers,
        } => {
            move_caret(render_object, CaretMove::PageDown, modifiers.shift);
            finish_caret_change(view, render_object);
            true
        }
        _ => false,
    }
}

/// Handle an IME text-input event for a focused text view.
///
/// # Arguments
///
/// * `view` - Source text view containing bindings and callbacks.
/// * `render_object` - Mutable render object holding current document state.
/// * `event` - Text-input event to process.
///
/// # Returns
///
/// `true` when the event was handled by text editing.
pub(crate) fn handle_text_view_text_input(
    view: &TextView,
    render_object: &mut TextViewRenderObject,
    event: &Event,
) -> bool {
    if !render_object.is_focused() {
        return false;
    }

    match event {
        Event::TextInputCommit { text, .. } => {
            render_object.clear_preedit();
            insert_text(view, render_object, text);
            true
        }
        Event::TextInputPreedit {
            cursor_byte,
            anchor_byte,
            text,
            spans,
            ..
        } => {
            render_object.set_preedit_state(text, *cursor_byte, *anchor_byte, spans);
            true
        }
        Event::TextInputDeleteSurroundingText {
            before_bytes,
            after_bytes,
            ..
        } => {
            render_object.clear_preedit();
            delete_surrounding_text(view, render_object, *before_bytes, *after_bytes);
            true
        }
        Event::TextInputDone { .. } => true,
        _ => false,
    }
}

/// Handle a mouse event for a text view.
///
/// # Arguments
///
/// * `view` - Source text view containing bindings and scroll state.
/// * `render_object` - Mutable render object holding selection and layout state.
/// * `event` - Mouse event to process.
///
/// # Returns
///
/// `true` when the event was handled by text selection or scrolling.
pub(crate) fn handle_text_view_mouse(
    view: &TextView,
    render_object: &mut TextViewRenderObject,
    event: &MouseEvent,
) -> bool {
    match *event {
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count,
        } => handle_primary_click(view, render_object, x, y, click_count),
        MouseEvent::Moved { x, y } => handle_drag_selection(view, render_object, x, y),
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            ..
        } => {
            render_object.dragging = false;
            true
        }
        MouseEvent::Wheel { delta_x, delta_y } => {
            scroll_view(view, render_object, delta_x, delta_y)
        }
        MouseEvent::Entered { .. } | MouseEvent::Exited { .. } => false,
        MouseEvent::ButtonPressed { .. } | MouseEvent::ButtonReleased { .. } => false,
    }
}

/// Handle focus changes for a text view.
///
/// # Arguments
///
/// * `render_object` - Mutable render object to update.
/// * `event` - Focus event to process.
///
/// # Returns
///
/// Always returns `true` for focus events.
pub(crate) fn handle_text_view_focus(
    render_object: &mut TextViewRenderObject,
    event: FocusEvent,
) -> bool {
    match event {
        FocusEvent::Gained => render_object.set_focused(true),
        FocusEvent::Lost => render_object.set_focused(false),
    }
    true
}

fn handle_primary_shortcut(
    view: &TextView,
    render_object: &mut TextViewRenderObject,
    key: char,
) -> bool {
    match key {
        'a' | 'A' => {
            select_all(render_object);
            view.selection_state().set(render_object.selection);
            true
        }
        'c' | 'C' => {
            copy_selection(view, render_object);
            true
        }
        'v' | 'V' => {
            if let Some(callback) = view.on_paste.as_ref()
                && let Some(text) = callback()
            {
                render_object.clear_preedit();
                insert_text(view, render_object, &text);
            }
            true
        }
        'x' | 'X' => {
            if view.on_copy.is_some() && copy_selection(view, render_object) {
                render_object.clear_preedit();
                delete_selection(view, render_object);
            }
            true
        }
        'z' | 'Z' | 'y' | 'Y' => false,
        _ => false,
    }
}

fn handle_primary_click(
    view: &TextView,
    render_object: &mut TextViewRenderObject,
    x: i32,
    y: i32,
    click_count: u8,
) -> bool {
    render_object.set_focused(true);
    let Some(position) = hit_test_mouse(render_object, x, y) else {
        render_object.dragging = false;
        return false;
    };

    render_object.clear_preedit();
    match click_count {
        0 | 1 => {
            render_object.selection = TextSelection {
                anchor: position,
                caret: position,
            };
            render_object.dragging = true;
        }
        2 => {
            render_object.selection = word_selection(render_object, position.byte);
            render_object.dragging = false;
        }
        _ => {
            render_object.selection = line_selection(render_object, position.byte);
            render_object.dragging = false;
        }
    }
    update_desired_x(render_object);
    view.selection_state().set(render_object.selection);
    true
}

fn handle_drag_selection(
    view: &TextView,
    render_object: &mut TextViewRenderObject,
    x: i32,
    y: i32,
) -> bool {
    if !render_object.dragging {
        return false;
    }
    if let Some(position) = hit_test_mouse(render_object, x, y) {
        render_object.selection.caret = position;
        update_desired_x(render_object);
        view.selection_state().set(render_object.selection);
    }
    true
}

fn scroll_view(
    view: &TextView,
    render_object: &mut TextViewRenderObject,
    delta_x: i32,
    delta_y: i32,
) -> bool {
    let requested = super::TextViewScroll {
        x: render_object.scroll.x + delta_x as f32,
        y: render_object.scroll.y + delta_y as f32,
    };
    set_scroll(view, render_object, requested);
    true
}

fn finish_caret_change(view: &TextView, render_object: &mut TextViewRenderObject) {
    ensure_caret_visible(view, render_object);
    view.selection_state().set(render_object.selection);
}

fn ensure_caret_visible(view: &TextView, render_object: &mut TextViewRenderObject) {
    let caret = render_object.cursor_rect();
    let clip_left = BORDER_WIDTH;
    let clip_top = BORDER_WIDTH;
    let clip_right = (render_object.size.width - BORDER_WIDTH).max(clip_left);
    let clip_bottom = (render_object.size.height - BORDER_WIDTH).max(clip_top);
    let reveal_left = render_object
        .layout
        .text_origin_x
        .clamp(clip_left, clip_right);
    let reveal_top = render_object.padding.clamp(clip_top, clip_bottom);
    let reveal_right =
        (render_object.size.width - render_object.padding).clamp(reveal_left, clip_right);
    let reveal_bottom =
        (render_object.size.height - render_object.padding).clamp(reveal_top, clip_bottom);
    let mut requested = render_object.scroll;

    if caret.origin.x < reveal_left {
        requested.x -= reveal_left - caret.origin.x;
    } else if caret.origin.x + caret.size.width > reveal_right {
        requested.x += caret.origin.x + caret.size.width - reveal_right;
    }

    if caret.origin.y < reveal_top {
        requested.y -= reveal_top - caret.origin.y;
    } else if caret.origin.y + caret.size.height > reveal_bottom {
        requested.y += caret.origin.y + caret.size.height - reveal_bottom;
    }

    set_scroll(view, render_object, requested);
}

fn set_scroll(
    view: &TextView,
    render_object: &mut TextViewRenderObject,
    requested: super::TextViewScroll,
) {
    let clamped = render_object.layout.clamp_scroll(requested);
    if clamped == render_object.scroll {
        return;
    }
    render_object.scroll = clamped;
    render_object.compute_layout();
    if let Some(scroll) = view.scroll.as_ref() {
        scroll.set(clamped);
    }
}

fn hit_test_mouse(render_object: &TextViewRenderObject, x: i32, y: i32) -> Option<TextPosition> {
    let point = Point::new(x as f32, y as f32);
    render_object
        .hit_test(point)
        .map(|position| position.clamp_to_grapheme(&render_object.text_document.as_str()))
}

fn word_selection(render_object: &TextViewRenderObject, byte: usize) -> TextSelection {
    let text = render_object.text_document.as_str();
    if text.is_empty() {
        return TextSelection::collapsed(0);
    }

    let byte = clamp_byte_boundary(&text, byte.min(text.len()));
    let mut word_start = byte;
    let mut word_end = byte;

    for (start, grapheme) in text.grapheme_indices(true) {
        let end = start + grapheme.len();
        if byte >= start && byte <= end {
            if !is_word_grapheme(grapheme) {
                return TextSelection::collapsed(start);
            }
            word_start = start;
            word_end = end;
            break;
        }
    }

    while let Some(previous) = previous_grapheme(word_start, &text)
        && is_word_grapheme(previous.1)
    {
        word_start = previous.0;
    }
    while word_end < text.len() {
        let Some((_, grapheme)) = next_grapheme(word_end, &text) else {
            break;
        };
        if !is_word_grapheme(grapheme) {
            break;
        }
        word_end += grapheme.len();
    }

    TextSelection {
        anchor: TextPosition::new(word_start),
        caret: TextPosition::new(word_end),
    }
}

fn line_selection(render_object: &TextViewRenderObject, byte: usize) -> TextSelection {
    let logical_line = render_object.layout.line_index_at_byte(byte);
    let Some(range) = render_object.text_document.line_range(logical_line) else {
        return TextSelection::collapsed(byte);
    };
    TextSelection {
        anchor: TextPosition::new(range.start),
        caret: TextPosition::new(line_end_without_newline(
            &render_object.text_document.as_str(),
            range,
        )),
    }
}

fn previous_grapheme(byte: usize, text: &str) -> Option<(usize, &str)> {
    text[..byte]
        .grapheme_indices(true)
        .map(|(offset, grapheme)| (offset, grapheme))
        .last()
}

fn next_grapheme(byte: usize, text: &str) -> Option<(usize, &str)> {
    text[byte..]
        .grapheme_indices(true)
        .map(|(offset, grapheme)| (byte + offset, grapheme))
        .next()
}

fn is_word_grapheme(grapheme: &str) -> bool {
    grapheme
        .chars()
        .any(|character| character.is_alphanumeric() || character == '_')
}

fn insert_text(view: &TextView, render_object: &mut TextViewRenderObject, text: &str) {
    let range = render_object
        .selection
        .normalized_range()
        .unwrap_or(render_object.selection.caret.byte..render_object.selection.caret.byte);
    apply_replace(view, render_object, range, text);
}

fn delete_selection(view: &TextView, render_object: &mut TextViewRenderObject) -> bool {
    let Some(range) = render_object.selection.normalized_range() else {
        return false;
    };
    apply_replace(view, render_object, range, "");
    true
}

fn delete_backward(view: &TextView, render_object: &mut TextViewRenderObject) -> bool {
    if delete_selection(view, render_object) {
        return true;
    }

    let text = render_object.text_document.as_str();
    let caret = render_object.selection.caret.byte.min(text.len());
    let Some(previous) = previous_grapheme_boundary(&text, caret) else {
        return false;
    };
    apply_replace(view, render_object, previous..caret, "");
    true
}

fn delete_forward(view: &TextView, render_object: &mut TextViewRenderObject) -> bool {
    if delete_selection(view, render_object) {
        return true;
    }

    let text = render_object.text_document.as_str();
    let caret = render_object.selection.caret.byte.min(text.len());
    let Some(next) = next_grapheme_boundary(&text, caret) else {
        return false;
    };
    apply_replace(view, render_object, caret..next, "");
    true
}

fn delete_surrounding_text(
    view: &TextView,
    render_object: &mut TextViewRenderObject,
    before_bytes: u32,
    after_bytes: u32,
) -> bool {
    let text = render_object.text_document.as_str();
    let caret = render_object.selection.caret.byte.min(text.len());
    let start = clamp_byte_boundary(&text, caret.saturating_sub(before_bytes as usize));
    let end = clamp_byte_boundary(&text, caret.saturating_add(after_bytes as usize));
    if start == end {
        return false;
    }
    apply_replace(view, render_object, start..end, "");
    true
}

fn apply_replace(
    view: &TextView,
    render_object: &mut TextViewRenderObject,
    range: Range<usize>,
    text: &str,
) {
    let (new_document, delta) = render_object.text_document.replace(range.clone(), text);
    let caret = delta.replaced_range.start + text.len();
    render_object.text_document = new_document.clone();
    render_object.selection = TextSelection::collapsed(caret);
    render_object.desired_x = None;
    render_object.compute_layout();
    ensure_caret_visible(view, render_object);

    if let Some(text_state) = view.text_state() {
        text_state.set(new_document.as_str().into_owned());
    }
    if let Some(document_state) = view.document_state() {
        document_state.set(new_document);
    }
    view.selection_state().set(render_object.selection);
    if let Some(callback) = view.on_text_change.as_ref() {
        callback(&delta);
    }
}

fn move_caret(
    render_object: &mut TextViewRenderObject,
    movement: CaretMove,
    extend_selection: bool,
) {
    let text = render_object.text_document.as_str();
    let current = render_object.selection.caret.byte.min(text.len());
    let next = match movement {
        CaretMove::Left => previous_grapheme_boundary(&text, current).unwrap_or(0),
        CaretMove::Right => next_grapheme_boundary(&text, current).unwrap_or(text.len()),
        CaretMove::Up => vertical_position(render_object, -1),
        CaretMove::Down => vertical_position(render_object, 1),
        CaretMove::PageUp => page_position(render_object, -1),
        CaretMove::PageDown => page_position(render_object, 1),
        CaretMove::LineStart => line_boundary(render_object, current, true),
        CaretMove::LineEnd => line_boundary(render_object, current, false),
        CaretMove::DocumentStart => 0,
        CaretMove::DocumentEnd => text.len(),
    };
    set_caret(render_object, next, extend_selection);
    if matches!(movement, CaretMove::Left | CaretMove::Right) {
        update_desired_x(render_object);
    }
}

fn select_all(render_object: &mut TextViewRenderObject) {
    render_object.selection = TextSelection {
        anchor: TextPosition::new(0),
        caret: TextPosition::new(render_object.text_document.len()),
    };
    render_object.desired_x = None;
}

fn copy_selection(view: &TextView, render_object: &TextViewRenderObject) -> bool {
    let Some(callback) = view.on_copy.as_ref() else {
        return false;
    };
    let Some(range) = render_object.selection.normalized_range() else {
        return false;
    };
    let text = render_object.text_document.as_str();
    callback(Cow::Owned(text[range].to_string()));
    true
}

fn set_caret(render_object: &mut TextViewRenderObject, byte: usize, extend_selection: bool) {
    let text = render_object.text_document.as_str();
    let position = TextPosition::new(byte.min(text.len())).clamp_to_grapheme(&text);
    if extend_selection {
        render_object.selection.caret = position;
    } else {
        render_object.selection = TextSelection {
            anchor: position,
            caret: position,
        };
    }
}

fn vertical_position(render_object: &mut TextViewRenderObject, delta_lines: isize) -> usize {
    let current_byte = render_object.selection.caret.byte;
    let current_index = visual_line_index(render_object, current_byte);
    let target_index = current_index
        .saturating_add_signed(delta_lines)
        .min(render_object.layout.visual_lines.len().saturating_sub(1));
    byte_at_visual_line_x(render_object, target_index)
}

fn page_position(render_object: &mut TextViewRenderObject, direction: isize) -> usize {
    let visible_count =
        (libm::ceilf(render_object.layout.viewport_height / render_object.layout.line_height)
            .max(1.0)) as isize;
    vertical_position(render_object, direction.saturating_mul(visible_count))
}

fn byte_at_visual_line_x(render_object: &mut TextViewRenderObject, target_index: usize) -> usize {
    let current_rect = render_object.cursor_rect();
    let desired_x = render_object.desired_x.unwrap_or(current_rect.origin.x);
    render_object.desired_x = Some(desired_x);
    let x = desired_x.max(render_object.layout.text_origin_x);
    render_object
        .layout
        .byte_at_line_x(target_index, x)
        .unwrap_or_else(|| {
            render_object
                .layout
                .visual_lines
                .get(target_index)
                .map(|line| line.text_range.end)
                .unwrap_or(render_object.selection.caret.byte)
        })
}

fn line_boundary(
    render_object: &TextViewRenderObject,
    current_byte: usize,
    line_start: bool,
) -> usize {
    let logical_line = render_object.layout.line_index_at_byte(current_byte);
    let Some(range) = render_object.text_document.line_range(logical_line) else {
        return current_byte;
    };
    if line_start {
        range.start
    } else {
        line_end_without_newline(&render_object.text_document.as_str(), range)
    }
}

fn visual_line_index(render_object: &TextViewRenderObject, byte: usize) -> usize {
    render_object
        .layout
        .visual_lines
        .iter()
        .position(|line| line.contains_byte(byte))
        .unwrap_or_else(|| render_object.layout.visual_lines.len().saturating_sub(1))
}

fn update_desired_x(render_object: &mut TextViewRenderObject) {
    render_object.desired_x = Some(render_object.cursor_rect().origin.x);
}

fn previous_grapheme_boundary(text: &str, byte: usize) -> Option<usize> {
    let byte = clamp_byte_boundary(text, byte);
    text[..byte]
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .last()
}

fn next_grapheme_boundary(text: &str, byte: usize) -> Option<usize> {
    let byte = clamp_byte_boundary(text, byte);
    text[byte..]
        .grapheme_indices(true)
        .map(|(offset, grapheme)| byte + offset + grapheme.len())
        .next()
}

fn line_end_without_newline(text: &str, range: Range<usize>) -> usize {
    let line = &text[range.clone()];
    if line.ends_with("\r\n") {
        range.end.saturating_sub(2)
    } else if line.ends_with('\n') {
        range.end.saturating_sub(1)
    } else {
        range.end
    }
}

fn clamp_byte_boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}
