//! Text field View - editable single-line text input.
//!
//! TextField shares the TextView document, selection, layout, IME, and paint
//! primitives, while keeping single-line submit behavior.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::any::Any;

use unicode_segmentation::UnicodeSegmentation;

use crate::color::{Color, ColorPalette};
use crate::element::{
    Element, ElementRenderObject, LayoutConstraints, RenderElement, TextInputElementState,
};
use crate::event::{FocusEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent};
use crate::geometry::{Point, Size};
use crate::graphics;
use crate::renderer::PaintContext;
use crate::state::{Listenable, State};
use crate::view::View;

use super::text_view::paint;
use super::text_view::{
    TextDocument, TextPosition, TextSelection, TextViewLayout, TextViewScroll, WrapMode,
};

/// Single-line editable text input.
#[derive(Clone)]
pub struct TextField {
    text: State<String>,
    placeholder: String,
    on_submit: Option<Arc<dyn Fn() + 'static>>,
    blur_on_submit: bool,
    background_color: Color,
    border_color: Color,
    focused_border_color: Color,
    text_color: Color,
    placeholder_color: Color,
    font_size: f32,
    padding: f32,
}

impl TextField {
    /// Create a new text field bound to the supplied text state.
    pub fn new(text: State<String>) -> Self {
        let palette = ColorPalette::default();
        Self {
            text,
            placeholder: String::new(),
            on_submit: None,
            blur_on_submit: false,
            background_color: palette.background(),
            border_color: palette.background_tertiary(),
            focused_border_color: palette.primary_light().lighten(0.4),
            text_color: palette.text(),
            placeholder_color: palette.text_secondary(),
            font_size: 14.0,
            padding: 8.0,
        }
    }

    /// Set placeholder text shown while the value is empty.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Set whether Enter should remove focus after submitting.
    pub fn blur_on_submit(mut self, blur: bool) -> Self {
        self.blur_on_submit = blur;
        self
    }

    /// Set the callback invoked when Enter is pressed while focused.
    pub fn on_submit(mut self, callback: impl Fn() + 'static) -> Self {
        self.on_submit = Some(Arc::new(callback));
        self
    }

    /// Set the font size.
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Set the padding.
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Set the text field background color.
    ///
    /// # Arguments
    ///
    /// * `color` - Background color used to fill the text field.
    ///
    /// # Returns
    ///
    /// The updated text field.
    pub fn background_color(mut self, color: Color) -> Self {
        self.background_color = color;
        self
    }

    /// Set the unfocused border color.
    ///
    /// # Arguments
    ///
    /// * `color` - Border color used while the text field is not focused.
    ///
    /// # Returns
    ///
    /// The updated text field.
    pub fn border_color(mut self, color: Color) -> Self {
        self.border_color = color;
        self
    }

    /// Set the focused border color.
    ///
    /// # Arguments
    ///
    /// * `color` - Border color used while the text field has keyboard focus.
    ///
    /// # Returns
    ///
    /// The updated text field.
    pub fn focused_border_color(mut self, color: Color) -> Self {
        self.focused_border_color = color;
        self
    }

    /// Set the main text color.
    ///
    /// # Arguments
    ///
    /// * `color` - Color used for entered text and the caret marker.
    ///
    /// # Returns
    ///
    /// The updated text field.
    pub fn text_color(mut self, color: Color) -> Self {
        self.text_color = color;
        self
    }

    /// Return the bound text state.
    pub fn text_state(&self) -> &State<String> {
        &self.text
    }

    /// Invoke the submit callback if present.
    pub fn invoke_submit(&self) {
        if let Some(callback) = self.on_submit.as_ref() {
            callback();
        }
    }
}

impl View for TextField {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            TextFieldRenderObject::from_view(self),
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn Listenable> {
        alloc::vec![&self.text]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// TextField RenderObject.
pub struct TextFieldRenderObject {
    text_document: TextDocument,
    selection: TextSelection,
    layout: TextViewLayout,
    preedit: String,
    preedit_cursor_byte: u32,
    preedit_anchor_byte: u32,
    preedit_spans: alloc::vec::Vec<u8>,
    focused: bool,
    dragging: bool,
    placeholder: String,
    background_color: Color,
    border_color: Color,
    focused_border_color: Color,
    text_color: Color,
    placeholder_color: Color,
    font_size: f32,
    padding: f32,
    size: Size,
}

impl TextFieldRenderObject {
    /// Create a render object from a TextField view.
    pub fn from_view(view: &TextField) -> Self {
        let text_document = TextDocument::from_str(&view.text.get());
        let selection = TextSelection::collapsed(text_document.len());
        let layout = text_field_layout(&text_document, view.font_size, view.padding, Size::ZERO);
        Self {
            text_document,
            selection,
            layout,
            preedit: String::new(),
            preedit_cursor_byte: 0,
            preedit_anchor_byte: 0,
            preedit_spans: alloc::vec::Vec::new(),
            focused: false,
            dragging: false,
            placeholder: view.placeholder.clone(),
            background_color: view.background_color,
            border_color: view.border_color,
            focused_border_color: view.focused_border_color,
            text_color: view.text_color,
            placeholder_color: view.placeholder_color,
            font_size: view.font_size,
            padding: view.padding,
            size: Size::ZERO,
        }
    }

    fn compute_layout(&mut self) {
        self.layout =
            text_field_layout(&self.text_document, self.font_size, self.padding, self.size);
    }
}

impl ElementRenderObject for TextFieldRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        let document_text = self.text_document.as_str();
        let visible_text = if self.text_document.is_empty() && self.preedit.is_empty() {
            alloc::borrow::Cow::Borrowed(self.placeholder.as_str())
        } else {
            document_text
        };
        let preedit_width = if self.preedit.is_empty() {
            0.0
        } else {
            graphics::measure_text_sized(&self.preedit, self.font_size).0 as f32
        };
        let (measured_width, _) = graphics::measure_text_sized(&visible_text, self.font_size);
        let line_height = graphics::line_height_sized(self.font_size).max(1) as f32;
        let intrinsic = Size::new(
            measured_width as f32 + preedit_width + self.padding * 2.0,
            line_height + self.padding * 2.0,
        );
        self.size = constraints.constrain(intrinsic);
        self.compute_layout();
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: Point) -> bool {
        point.x >= 0.0 && point.y >= 0.0 && point.x < self.size.width && point.y < self.size.height
    }

    fn render(&mut self) {}

    fn get_buffer(&self) -> Option<&crate::buffer::Buffer> {
        None
    }

    fn clear_buffer(&mut self) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn update(&mut self, new_view: &dyn View) -> crate::element::UpdateResult {
        let Some(view) = new_view.as_any().downcast_ref::<TextField>() else {
            return crate::element::UpdateResult::Replaced;
        };
        let focused = self.focused;
        let selection = self.selection;
        let preedit = self.preedit.clone();
        let preedit_cursor_byte = self.preedit_cursor_byte;
        let preedit_anchor_byte = self.preedit_anchor_byte;
        let preedit_spans = self.preedit_spans.clone();
        let size = self.size;
        *self = TextFieldRenderObject::from_view(view);
        let len = self.text_document.len();
        self.selection = TextSelection {
            anchor: TextPosition::new(selection.anchor.byte.min(len)),
            caret: TextPosition::new(selection.caret.byte.min(len)),
        };
        self.focused = focused;
        self.preedit = preedit;
        self.preedit_cursor_byte = preedit_cursor_byte;
        self.preedit_anchor_byte = preedit_anchor_byte;
        self.preedit_spans = preedit_spans;
        self.size = size;
        self.compute_layout();
        crate::element::UpdateResult::Updated
    }

    fn update_needs_layout(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        paint::paint_text_view(
            ctx,
            origin,
            self.size,
            &self.layout,
            &self.text_document,
            self.selection,
            self.focused,
            self.font_size,
            self.padding,
            self.background_color,
            self.text_color,
            self.placeholder_color,
            self.selection_color(),
            Color::TRANSPARENT,
            self.border_color,
            self.focused_border_color,
            &self.placeholder,
            &self.preedit,
            self.preedit_cursor_byte,
            &self.preedit_spans,
            false,
            false,
        );
        true
    }
}

impl TextFieldRenderObject {
    pub(crate) fn is_focused(&self) -> bool {
        self.focused
    }

    pub(crate) fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.clear_preedit();
        }
    }

    pub(crate) fn set_preedit_state(
        &mut self,
        preedit: &str,
        cursor_byte: u32,
        anchor_byte: u32,
        spans: &[u8],
    ) {
        self.preedit.clear();
        self.preedit.push_str(single_line_text(preedit).as_str());
        self.preedit_cursor_byte = clamp_byte_boundary(&self.preedit, cursor_byte);
        self.preedit_anchor_byte = clamp_byte_boundary(&self.preedit, anchor_byte);
        self.preedit_spans.clear();
        self.preedit_spans.extend_from_slice(spans);
    }

    pub(crate) fn clear_preedit(&mut self) {
        self.preedit.clear();
        self.preedit_cursor_byte = 0;
        self.preedit_anchor_byte = 0;
        self.preedit_spans.clear();
    }

    pub(crate) fn text_input_state(&self) -> TextInputElementState {
        let mut cursor_rect = self
            .layout
            .cursor_rect(self.selection.caret, &self.text_document);
        if !self.preedit.is_empty() {
            let prefix = preedit_prefix(&self.preedit, self.preedit_anchor_byte);
            cursor_rect.origin.x += graphics::measure_text_sized(prefix, self.font_size).0 as f32;
        }
        TextInputElementState {
            cursor_rect,
            surrounding_text: self.text_document.as_str().into_owned(),
            cursor_byte: self.selection.caret.byte as u32,
            anchor_byte: self.selection.anchor.byte as u32,
        }
    }

    fn selection_color(&self) -> Color {
        ColorPalette::default().primary().with_opacity(0.3)
    }
}

pub(crate) fn handle_text_field_keyboard(
    field: &TextField,
    render_object: &mut TextFieldRenderObject,
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
            insert_text(field, render_object, &text);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Space,
            ..
        } => {
            render_object.clear_preedit();
            insert_text(field, render_object, " ");
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Backspace,
            ..
        } => {
            render_object.clear_preedit();
            delete_backward(field, render_object);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Delete,
            ..
        } => {
            render_object.clear_preedit();
            delete_forward(field, render_object);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Left,
            modifiers,
        } => {
            move_caret(render_object, CaretMove::Left, modifiers);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Right,
            modifiers,
        } => {
            move_caret(render_object, CaretMove::Right, modifiers);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Home,
            modifiers,
        } => {
            move_caret(render_object, CaretMove::Start, modifiers);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::End,
            modifiers,
        } => {
            move_caret(render_object, CaretMove::End, modifiers);
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Char('a' | 'A'),
            modifiers,
        } if modifiers.primary() => {
            let len = render_object.text_document.len();
            render_object.selection = TextSelection {
                anchor: TextPosition::new(0),
                caret: TextPosition::new(len),
            };
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Enter,
            ..
        } => {
            field.invoke_submit();
            if field.blur_on_submit {
                render_object.set_focused(false);
            }
            true
        }
        KeyEvent::Pressed {
            keycode: KeyCode::Escape,
            ..
        }
        | KeyEvent::Pressed {
            keycode: KeyCode::Tab,
            ..
        } => {
            render_object.set_focused(false);
            true
        }
        _ => false,
    }
}

pub(crate) fn handle_text_field_focus(
    render_object: &mut TextFieldRenderObject,
    event: FocusEvent,
) -> bool {
    match event {
        FocusEvent::Gained => render_object.set_focused(true),
        FocusEvent::Lost => render_object.set_focused(false),
    }
    true
}

pub(crate) fn handle_text_field_text_input(
    field: &TextField,
    render_object: &mut TextFieldRenderObject,
    event: &crate::event::Event,
) -> bool {
    if !render_object.is_focused() {
        return false;
    }

    match event {
        crate::event::Event::TextInputCommit { text, .. } => {
            render_object.clear_preedit();
            insert_text(field, render_object, &single_line_text(text));
            true
        }
        crate::event::Event::TextInputPreedit {
            cursor_byte,
            anchor_byte,
            text,
            spans,
            ..
        } => {
            render_object.set_preedit_state(text, *cursor_byte, *anchor_byte, spans);
            true
        }
        crate::event::Event::TextInputDeleteSurroundingText {
            before_bytes,
            after_bytes,
            ..
        } => {
            render_object.clear_preedit();
            delete_surrounding_text(field, render_object, *before_bytes, *after_bytes);
            true
        }
        crate::event::Event::TextInputDone { .. } => true,
        _ => false,
    }
}

pub(crate) fn handle_text_field_mouse(
    _field: &TextField,
    render_object: &mut TextFieldRenderObject,
    event: &MouseEvent,
) -> bool {
    match *event {
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count,
        } => handle_primary_click(render_object, x, y, click_count),
        MouseEvent::Moved { x, y } => handle_drag_selection(render_object, x, y),
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            ..
        } => {
            render_object.dragging = false;
            true
        }
        MouseEvent::Entered { .. }
        | MouseEvent::Exited { .. }
        | MouseEvent::Wheel { .. }
        | MouseEvent::ButtonPressed { .. }
        | MouseEvent::ButtonReleased { .. } => false,
    }
}

#[derive(Clone, Copy)]
enum CaretMove {
    Left,
    Right,
    Start,
    End,
}

fn insert_text(field: &TextField, render_object: &mut TextFieldRenderObject, text: &str) {
    let text = single_line_text(text);
    if text.is_empty() {
        return;
    }
    let range = render_object
        .selection
        .normalized_range()
        .unwrap_or(render_object.selection.caret.byte..render_object.selection.caret.byte);
    let (document, delta) = render_object.text_document.replace(range, &text);
    let caret = delta.replaced_range.start + text.len();
    render_object.text_document = document;
    render_object.selection = TextSelection::collapsed(caret);
    render_object.compute_layout();
    field
        .text
        .set(render_object.text_document.as_str().into_owned());
}

fn delete_backward(field: &TextField, render_object: &mut TextFieldRenderObject) {
    if delete_selection(field, render_object) {
        return;
    }
    let text = render_object.text_document.as_str();
    let caret = render_object.selection.caret.byte.min(text.len());
    let Some(start) = previous_grapheme_boundary(&text, caret) else {
        return;
    };
    delete_range(field, render_object, start..caret);
}

fn delete_forward(field: &TextField, render_object: &mut TextFieldRenderObject) {
    if delete_selection(field, render_object) {
        return;
    }
    let text = render_object.text_document.as_str();
    let caret = render_object.selection.caret.byte.min(text.len());
    let Some(end) = next_grapheme_boundary(&text, caret) else {
        return;
    };
    delete_range(field, render_object, caret..end);
}

fn delete_selection(field: &TextField, render_object: &mut TextFieldRenderObject) -> bool {
    let Some(range) = render_object.selection.normalized_range() else {
        return false;
    };
    delete_range(field, render_object, range);
    true
}

fn delete_surrounding_text(
    field: &TextField,
    render_object: &mut TextFieldRenderObject,
    before_bytes: u32,
    after_bytes: u32,
) {
    let text = render_object.text_document.as_str();
    let caret = render_object.selection.caret.byte.min(text.len());
    let start = clamp_byte_boundary_usize(&text, caret.saturating_sub(before_bytes as usize));
    let end = clamp_byte_boundary_usize(&text, caret.saturating_add(after_bytes as usize));
    drop(text);
    delete_range(field, render_object, start..end);
}

fn delete_range(
    field: &TextField,
    render_object: &mut TextFieldRenderObject,
    range: core::ops::Range<usize>,
) {
    if range.is_empty() {
        return;
    }
    let start = range.start;
    let (document, _) = render_object.text_document.delete(range);
    render_object.text_document = document;
    render_object.selection = TextSelection::collapsed(start);
    render_object.compute_layout();
    field
        .text
        .set(render_object.text_document.as_str().into_owned());
}

fn move_caret(
    render_object: &mut TextFieldRenderObject,
    movement: CaretMove,
    modifiers: KeyModifiers,
) {
    let text = render_object.text_document.as_str();
    let current = render_object.selection.caret.byte.min(text.len());
    let byte = match movement {
        CaretMove::Left => previous_grapheme_boundary(&text, current).unwrap_or(0),
        CaretMove::Right => next_grapheme_boundary(&text, current).unwrap_or(text.len()),
        CaretMove::Start => 0,
        CaretMove::End => text.len(),
    };
    let position = TextPosition::new(byte).clamp_to_grapheme(&text);
    if modifiers.shift {
        render_object.selection.caret = position;
    } else {
        render_object.selection = TextSelection {
            anchor: position,
            caret: position,
        };
    }
}

fn handle_primary_click(
    render_object: &mut TextFieldRenderObject,
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
            render_object.selection = TextSelection {
                anchor: TextPosition::new(0),
                caret: TextPosition::new(render_object.text_document.len()),
            };
            render_object.dragging = false;
        }
    }
    true
}

fn handle_drag_selection(render_object: &mut TextFieldRenderObject, x: i32, y: i32) -> bool {
    if !render_object.dragging {
        return false;
    }
    if let Some(position) = hit_test_mouse(render_object, x, y) {
        render_object.selection.caret = position;
    }
    true
}

fn hit_test_mouse(render_object: &TextFieldRenderObject, x: i32, _y: i32) -> Option<TextPosition> {
    let x = (x as f32).max(render_object.layout.text_origin_x);
    let y = render_object.layout.padding + render_object.layout.line_height * 0.5;
    render_object
        .layout
        .hit_test(Point::new(x, y))
        .map(|position| position.clamp_to_grapheme(&render_object.text_document.as_str()))
}

fn word_selection(render_object: &TextFieldRenderObject, byte: usize) -> TextSelection {
    let text = render_object.text_document.as_str();
    if text.is_empty() {
        return TextSelection::collapsed(0);
    }

    let byte = clamp_byte_boundary_usize(&text, byte.min(text.len()));
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

    while let Some((start, grapheme)) = previous_grapheme(word_start, &text)
        && is_word_grapheme(grapheme)
    {
        word_start = start;
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

fn text_field_layout(
    document: &TextDocument,
    font_size: f32,
    padding: f32,
    size: Size,
) -> TextViewLayout {
    TextViewLayout::compute(
        document,
        font_size,
        padding,
        WrapMode::None,
        size,
        TextViewScroll::default(),
        false,
    )
}

fn single_line_text(text: &str) -> String {
    text.chars()
        .filter(|character| *character != '\n' && *character != '\r')
        .collect()
}

fn preedit_prefix(preedit: &str, byte: u32) -> &str {
    let byte = clamp_byte_boundary(preedit, byte) as usize;
    &preedit[..byte]
}

fn clamp_byte_boundary(text: &str, byte: u32) -> u32 {
    clamp_byte_boundary_usize(text, byte as usize) as u32
}

fn clamp_byte_boundary_usize(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

fn previous_grapheme_boundary(text: &str, byte: usize) -> Option<usize> {
    if byte == 0 {
        return None;
    }
    text[..byte]
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .last()
}

fn next_grapheme_boundary(text: &str, byte: usize) -> Option<usize> {
    if byte >= text.len() {
        return None;
    }
    text[byte..]
        .grapheme_indices(true)
        .map(|(offset, grapheme)| byte + offset + grapheme.len())
        .next()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, MouseButton, MouseEvent};
    use crate::renderer::PaintCommand;
    use crate::state::StateId;

    #[test]
    fn paint_preedit_emits_underline() {
        let field = TextField::new(State::new(StateId::new(1), String::new()));
        let mut render_object = TextFieldRenderObject::from_view(&field);
        render_object.focused = true;
        render_object.set_preedit_state("かな", 6, 0, &[]);
        render_object.layout(LayoutConstraints::tight(160.0, 32.0));

        let mut ctx = PaintContext::new();
        render_object.paint(&mut ctx, Point::ZERO);

        let fill_count = ctx
            .commands()
            .iter()
            .filter(|cmd| matches!(cmd, PaintCommand::FillPath { .. }))
            .count();
        assert!(
            fill_count >= 2,
            "expected background fill plus preedit underline"
        );
    }

    #[test]
    fn preedit_hides_placeholder() {
        let field = TextField::new(State::new(StateId::new(2), String::new())).placeholder("Name");
        let mut render_object = TextFieldRenderObject::from_view(&field);
        render_object.focused = true;
        render_object.set_preedit_state("な", 3, 0, &[]);
        render_object.layout(LayoutConstraints::tight(160.0, 32.0));

        let mut ctx = PaintContext::new();
        render_object.paint(&mut ctx, Point::ZERO);

        assert!(!ctx.commands().iter().any(|command| matches!(
            command,
            PaintCommand::DrawText { text, .. } if text == "Name"
        )));
        assert!(ctx.commands().iter().any(|command| matches!(
            command,
            PaintCommand::DrawText { text, .. } if text == "な"
        )));
    }

    #[test]
    fn commit_inserts_at_caret_and_remains_single_line() {
        let text = State::new(StateId::new(3), String::from("ac"));
        let field = TextField::new(text.clone());
        let mut render_object = TextFieldRenderObject::from_view(&field);
        render_object.focused = true;
        render_object.selection = TextSelection::collapsed(1);

        assert!(handle_text_field_text_input(
            &field,
            &mut render_object,
            &Event::TextInputCommit {
                context_id: 1,
                serial: 1,
                text: String::from("b\n"),
            }
        ));

        assert_eq!(text.get(), "abc");
        assert_eq!(render_object.selection, TextSelection::collapsed(2));
    }

    #[test]
    fn mouse_single_click_positions_caret() {
        let field = TextField::new(State::new(StateId::new(4), String::from("abcd")));
        let mut render_object = TextFieldRenderObject::from_view(&field);
        render_object.layout(LayoutConstraints::tight(240.0, 32.0));

        assert!(handle_text_field_mouse(
            &field,
            &mut render_object,
            &MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: text_x("ab"),
                y: 10,
                click_count: 1,
            }
        ));

        assert_eq!(render_object.selection, TextSelection::collapsed(2));
        assert!(render_object.is_focused());
        assert!(render_object.dragging);
    }

    #[test]
    fn mouse_drag_extends_selection_and_release_clears_dragging() {
        let field = TextField::new(State::new(StateId::new(5), String::from("abcdef")));
        let mut render_object = TextFieldRenderObject::from_view(&field);
        render_object.layout(LayoutConstraints::tight(240.0, 32.0));

        assert!(handle_text_field_mouse(
            &field,
            &mut render_object,
            &MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: text_x("a"),
                y: 10,
                click_count: 1,
            }
        ));
        let anchor = render_object.selection.anchor;

        assert!(handle_text_field_mouse(
            &field,
            &mut render_object,
            &MouseEvent::Moved {
                x: text_x("abcd"),
                y: 10,
            }
        ));

        assert_eq!(render_object.selection.anchor, anchor);
        assert_eq!(render_object.selection.caret.byte, 4);

        assert!(handle_text_field_mouse(
            &field,
            &mut render_object,
            &MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: text_x("abcd"),
                y: 10,
                click_count: 1,
            }
        ));
        assert!(!render_object.dragging);
    }

    #[test]
    fn mouse_double_click_selects_word() {
        let field = TextField::new(State::new(StateId::new(6), String::from("hello world")));
        let mut render_object = TextFieldRenderObject::from_view(&field);
        render_object.layout(LayoutConstraints::tight(240.0, 32.0));

        assert!(handle_text_field_mouse(
            &field,
            &mut render_object,
            &MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: text_x("hello wo"),
                y: 10,
                click_count: 2,
            }
        ));

        assert_eq!(render_object.selection.anchor.byte, "hello ".len());
        assert_eq!(render_object.selection.caret.byte, "hello world".len());
    }

    fn text_x(prefix: &str) -> i32 {
        (8.0 + crate::graphics::measure_text_sized(prefix, 14.0).0 as f32) as i32
    }
}
