//! Multi-line editable text view.

mod document;
mod editing;
mod layout;
mod paint;
mod selection;
#[cfg(test)]
mod tests;

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

pub use document::{EditDelta, TextDocument, TextPosition};
pub(crate) use editing::{
    handle_text_view_focus, handle_text_view_keyboard, handle_text_view_mouse,
    handle_text_view_text_input,
};
pub use layout::{TextViewLayout, VisualLine};
pub use selection::{TabMode, TextSelection, TextViewScroll, WrapMode};

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{
    Element, ElementRenderObject, LayoutConstraints, RenderElement, TextInputElementState,
};
use crate::geometry::{Point, Rect, Size};
use crate::renderer::PaintContext;
use crate::state::{Listenable, State};
use crate::view::View;

const BORDER_WIDTH: f32 = 1.0;

#[derive(Clone)]
enum TextBinding {
    String(State<String>),
    Document(State<TextDocument>),
}

/// Multi-line editable text view.
#[derive(Clone)]
pub struct TextView {
    text: TextBinding,
    selection: State<TextSelection>,
    scroll: Option<State<TextViewScroll>>,
    wrap_mode: Option<State<WrapMode>>,
    static_wrap_mode: WrapMode,
    placeholder: String,
    font_size: f32,
    padding: f32,
    tab_mode: TabMode,
    line_numbers: bool,
    current_line_highlight: bool,
    background_color: Color,
    text_color: Color,
    placeholder_color: Color,
    selection_color: Color,
    current_line_color: Color,
    border_color: Color,
    focused_border_color: Color,
    on_copy: Option<Arc<dyn for<'a> Fn(Cow<'a, str>) + 'static>>,
    on_paste: Option<Arc<dyn Fn() -> Option<Cow<'static, str>> + 'static>>,
    on_text_change: Option<Arc<dyn Fn(&EditDelta) + 'static>>,
}

impl TextView {
    /// Create a text view bound to a string state.
    ///
    /// # Arguments
    ///
    /// * `text` - String state used by the convenience binding path.
    /// * `selection` - Selection state shared with the view.
    ///
    /// # Returns
    ///
    /// A text view configured with default styling and behavior.
    pub fn new(text: State<String>, selection: State<TextSelection>) -> Self {
        Self::base(TextBinding::String(text), selection)
    }

    /// Create a text view bound to a rope-backed document state.
    ///
    /// # Arguments
    ///
    /// * `text` - Document state used by the O(log n) editing path.
    /// * `selection` - Selection state shared with the view.
    ///
    /// # Returns
    ///
    /// A text view configured with default styling and behavior.
    pub fn with_document(text: State<TextDocument>, selection: State<TextSelection>) -> Self {
        Self::base(TextBinding::Document(text), selection)
    }

    fn base(text: TextBinding, selection: State<TextSelection>) -> Self {
        let palette = ColorPalette::default();
        Self {
            text,
            selection,
            scroll: None,
            wrap_mode: None,
            static_wrap_mode: WrapMode::None,
            placeholder: String::new(),
            font_size: 14.0,
            padding: 8.0,
            tab_mode: TabMode::Tab,
            line_numbers: false,
            current_line_highlight: false,
            background_color: palette.background(),
            text_color: palette.text(),
            placeholder_color: palette.text_secondary(),
            selection_color: palette.primary().with_opacity(0.3),
            current_line_color: palette.background_secondary(),
            border_color: palette.background_tertiary(),
            focused_border_color: palette.primary_light().lighten(0.4),
            on_copy: None,
            on_paste: None,
            on_text_change: None,
        }
    }

    /// Bind an external scroll state.
    ///
    /// # Arguments
    ///
    /// * `scroll` - Scroll offset state used by the view.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn scroll_state(mut self, scroll: State<TextViewScroll>) -> Self {
        self.scroll = Some(scroll);
        self
    }

    /// Bind an external wrap mode state.
    ///
    /// # Arguments
    ///
    /// * `mode` - Runtime wrap mode state used by the view.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn wrap_mode_state(mut self, mode: State<WrapMode>) -> Self {
        self.wrap_mode = Some(mode);
        self
    }

    /// Set a static wrap mode.
    ///
    /// # Arguments
    ///
    /// * `mode` - Wrap mode to use when no wrap mode state is bound.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn wrap_mode(mut self, mode: WrapMode) -> Self {
        self.static_wrap_mode = mode;
        self.wrap_mode = None;
        self
    }

    /// Set placeholder text shown while the document is empty.
    ///
    /// # Arguments
    ///
    /// * `placeholder` - Placeholder text.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Set the text font size.
    ///
    /// # Arguments
    ///
    /// * `size` - Font size in logical pixels.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Set the content padding.
    ///
    /// # Arguments
    ///
    /// * `padding` - Uniform padding in logical pixels.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Set tab insertion behavior.
    ///
    /// # Arguments
    ///
    /// * `mode` - Tab insertion mode.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn tab_mode(mut self, mode: TabMode) -> Self {
        self.tab_mode = mode;
        self
    }

    /// Set whether line numbers should be shown.
    ///
    /// # Arguments
    ///
    /// * `enabled` - `true` to reserve space for line numbers in later tasks.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn line_numbers(mut self, enabled: bool) -> Self {
        self.line_numbers = enabled;
        self
    }

    /// Set whether the current line should be highlighted.
    ///
    /// # Arguments
    ///
    /// * `enabled` - `true` to enable current-line highlighting in later tasks.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn current_line_highlight(mut self, enabled: bool) -> Self {
        self.current_line_highlight = enabled;
        self
    }

    /// Set the text view background color.
    ///
    /// # Arguments
    ///
    /// * `color` - Background color used to fill the view.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn background(mut self, color: Color) -> Self {
        self.background_color = color;
        self
    }

    /// Set the main text color.
    ///
    /// # Arguments
    ///
    /// * `color` - Color used for document text and the caret.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn text_color(mut self, color: Color) -> Self {
        self.text_color = color;
        self
    }

    /// Set the unfocused border color.
    ///
    /// # Arguments
    ///
    /// * `color` - Border color used while the view is not focused.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn border_color(mut self, color: Color) -> Self {
        self.border_color = color;
        self
    }

    /// Set the focused border color.
    ///
    /// # Arguments
    ///
    /// * `color` - Border color used while the view has keyboard focus.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn focused_border_color(mut self, color: Color) -> Self {
        self.focused_border_color = color;
        self
    }

    /// Set the selection highlight color.
    ///
    /// # Arguments
    ///
    /// * `color` - Color used behind selected text.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn selection_color(mut self, color: Color) -> Self {
        self.selection_color = color;
        self
    }

    /// Set the placeholder text color.
    ///
    /// # Arguments
    ///
    /// * `color` - Color used for placeholder text and line numbers.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn placeholder_color(mut self, color: Color) -> Self {
        self.placeholder_color = color;
        self
    }

    /// Set the current-line highlight color.
    ///
    /// # Arguments
    ///
    /// * `color` - Color used to highlight the caret line.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn current_line_color(mut self, color: Color) -> Self {
        self.current_line_color = color;
        self
    }

    /// Set a callback invoked when text is copied.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function that receives the copied text.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn on_copy(mut self, callback: impl for<'a> Fn(Cow<'a, str>) + 'static) -> Self {
        self.on_copy = Some(Arc::new(callback));
        self
    }

    /// Set a callback invoked when paste text is requested.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function that returns paste text when available.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn on_paste(mut self, callback: impl Fn() -> Option<Cow<'static, str>> + 'static) -> Self {
        self.on_paste = Some(Arc::new(callback));
        self
    }

    /// Set a callback invoked after text edits.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function that receives the edit delta.
    ///
    /// # Returns
    ///
    /// The updated text view.
    pub fn on_text_change(mut self, callback: impl Fn(&EditDelta) + 'static) -> Self {
        self.on_text_change = Some(Arc::new(callback));
        self
    }

    /// Return the bound string state when using the convenience constructor.
    ///
    /// # Returns
    ///
    /// `Some` for `TextView::new`, otherwise `None`.
    pub fn text_state(&self) -> Option<&State<String>> {
        match &self.text {
            TextBinding::String(text) => Some(text),
            TextBinding::Document(_) => None,
        }
    }

    /// Return the bound document state when using the document constructor.
    ///
    /// # Returns
    ///
    /// `Some` for `TextView::with_document`, otherwise `None`.
    pub fn document_state(&self) -> Option<&State<TextDocument>> {
        match &self.text {
            TextBinding::String(_) => None,
            TextBinding::Document(text) => Some(text),
        }
    }

    /// Return the selection state.
    ///
    /// # Returns
    ///
    /// The selection state bound to this text view.
    pub fn selection_state(&self) -> &State<TextSelection> {
        &self.selection
    }
}

impl View for TextView {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            TextViewRenderObject::from_view(self),
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        let mut listenables: Vec<&dyn Listenable> = Vec::new();
        match &self.text {
            TextBinding::String(text) => listenables.push(text),
            TextBinding::Document(text) => listenables.push(text),
        }
        listenables.push(&self.selection);
        if let Some(scroll) = self.scroll.as_ref() {
            listenables.push(scroll);
        }
        if let Some(wrap_mode) = self.wrap_mode.as_ref() {
            listenables.push(wrap_mode);
        }
        listenables
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object shell for [`TextView`].
pub struct TextViewRenderObject {
    text_document: TextDocument,
    selection: TextSelection,
    scroll: TextViewScroll,
    wrap_mode: WrapMode,
    preedit: String,
    preedit_cursor_byte: u32,
    preedit_anchor_byte: u32,
    preedit_spans: Vec<u8>,
    focused: bool,
    dragging: bool,
    desired_x: Option<f32>,
    size: Size,
    layout: TextViewLayout,
    placeholder: String,
    font_size: f32,
    padding: f32,
    tab_mode: TabMode,
    line_numbers: bool,
    current_line_highlight: bool,
    background_color: Color,
    text_color: Color,
    placeholder_color: Color,
    selection_color: Color,
    current_line_color: Color,
    border_color: Color,
    focused_border_color: Color,
}

impl TextViewRenderObject {
    /// Create a render object from a text view.
    ///
    /// # Arguments
    ///
    /// * `view` - Source view used to initialize the render object shell.
    ///
    /// # Returns
    ///
    /// A render object shell with no layout or paint output yet.
    pub fn from_view(view: &TextView) -> Self {
        let text_document = match &view.text {
            TextBinding::String(text) => TextDocument::from_str(&text.get()),
            TextBinding::Document(text) => text.get(),
        };
        let scroll = view
            .scroll
            .as_ref()
            .map(State::get)
            .unwrap_or_else(TextViewScroll::default);
        let wrap_mode = view
            .wrap_mode
            .as_ref()
            .map(State::get)
            .unwrap_or(view.static_wrap_mode);
        let layout = TextViewLayout::compute(
            &text_document,
            view.font_size,
            view.padding,
            wrap_mode,
            Size::ZERO,
            scroll,
            view.line_numbers,
        );

        Self {
            text_document,
            selection: view.selection.get(),
            scroll,
            wrap_mode,
            preedit: String::new(),
            preedit_cursor_byte: 0,
            preedit_anchor_byte: 0,
            preedit_spans: Vec::new(),
            focused: false,
            dragging: false,
            desired_x: None,
            size: Size::ZERO,
            layout,
            placeholder: view.placeholder.clone(),
            font_size: view.font_size,
            padding: view.padding,
            tab_mode: view.tab_mode,
            line_numbers: view.line_numbers,
            current_line_highlight: view.current_line_highlight,
            background_color: view.background_color,
            text_color: view.text_color,
            placeholder_color: view.placeholder_color,
            selection_color: view.selection_color,
            current_line_color: view.current_line_color,
            border_color: view.border_color,
            focused_border_color: view.focused_border_color,
        }
    }

    pub(crate) fn compute_layout(&mut self) {
        let layout = TextViewLayout::compute(
            &self.text_document,
            self.font_size,
            self.padding,
            self.wrap_mode,
            self.size,
            self.scroll,
            self.line_numbers,
        );
        let clamped_scroll = layout.clamp_scroll(self.scroll);
        self.layout = if clamped_scroll == self.scroll {
            layout
        } else {
            self.scroll = clamped_scroll;
            TextViewLayout::compute(
                &self.text_document,
                self.font_size,
                self.padding,
                self.wrap_mode,
                self.size,
                self.scroll,
                self.line_numbers,
            )
        };
    }

    /// Convert a widget-local point to a text position.
    ///
    /// # Arguments
    ///
    /// * `point` - Widget-local point to test.
    ///
    /// # Returns
    ///
    /// The nearest text position, or `None` when outside editable content.
    pub fn hit_test(&self, point: Point) -> Option<TextPosition> {
        self.layout.hit_test(point)
    }

    /// Return the current caret rectangle for IME positioning.
    ///
    /// # Returns
    ///
    /// A widget-local caret rectangle.
    pub fn cursor_rect(&self) -> Rect {
        self.layout
            .cursor_rect(self.selection.caret, &self.text_document)
    }

    /// Return whether this text view currently has keyboard focus.
    ///
    /// # Returns
    ///
    /// `true` when focused.
    pub(crate) fn is_focused(&self) -> bool {
        self.focused
    }

    /// Set the keyboard focus state.
    ///
    /// # Arguments
    ///
    /// * `focused` - New focus state.
    pub(crate) fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.clear_preedit();
        }
    }

    /// Return the active IME preedit text.
    ///
    /// # Returns
    ///
    /// The current preedit string.
    pub(crate) fn preedit(&self) -> &str {
        &self.preedit
    }

    /// Return the IME preedit cursor byte offset.
    ///
    /// # Returns
    ///
    /// Cursor byte offset within the preedit string.
    pub(crate) fn preedit_cursor_byte(&self) -> u32 {
        self.preedit_cursor_byte
    }

    /// Update the active IME preedit state.
    ///
    /// # Arguments
    ///
    /// * `preedit` - Preedit string to render inline at the caret.
    /// * `cursor_byte` - Cursor byte offset within `preedit`.
    /// * `anchor_byte` - Anchor byte offset within `preedit`.
    /// * `spans` - Serialized preedit span styling data.
    pub(crate) fn set_preedit_state(
        &mut self,
        preedit: &str,
        cursor_byte: u32,
        anchor_byte: u32,
        spans: &[u8],
    ) {
        self.preedit.clear();
        self.preedit.push_str(preedit);
        self.preedit_cursor_byte = clamp_byte_boundary(preedit, cursor_byte);
        self.preedit_anchor_byte = clamp_byte_boundary(preedit, anchor_byte);
        self.preedit_spans.clear();
        self.preedit_spans.extend_from_slice(spans);
    }

    /// Clear the active IME preedit state.
    pub(crate) fn clear_preedit(&mut self) {
        self.preedit.clear();
        self.preedit_cursor_byte = 0;
        self.preedit_anchor_byte = 0;
        self.preedit_spans.clear();
    }

    /// Return the text-input state exposed to platform IME backends.
    ///
    /// # Returns
    ///
    /// Cursor rectangle, surrounding text, and selection byte offsets.
    pub(crate) fn text_input_state(&self) -> TextInputElementState {
        TextInputElementState {
            cursor_rect: self.cursor_rect(),
            surrounding_text: self.text_document.as_str().into_owned(),
            cursor_byte: self.selection.caret.byte as u32,
            anchor_byte: self.selection.anchor.byte as u32,
        }
    }
}

impl ElementRenderObject for TextViewRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        let desired = Size::new(
            if constraints.max_width.is_finite() {
                constraints.max_width
            } else {
                400.0
            },
            if constraints.max_height.is_finite() {
                constraints.max_height
            } else {
                300.0
            },
        );
        self.size = constraints.constrain(desired);
        self.compute_layout();
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn render(&mut self) {}

    fn hit_test(&self, point: Point) -> bool {
        Rect::new(Point::ZERO, self.size).contains(point)
    }

    fn paint<'a>(&'a self, ctx: &mut PaintContext<'a>, origin: Point) -> bool {
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
            self.selection_color,
            self.current_line_color,
            self.border_color,
            self.focused_border_color,
            &self.placeholder,
            self.preedit(),
            self.preedit_cursor_byte(),
            &self.preedit_spans,
            self.line_numbers,
            self.current_line_highlight,
        );
        true
    }

    fn get_buffer(&self) -> Option<&Buffer> {
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
        let Some(view) = new_view.as_any().downcast_ref::<TextView>() else {
            return crate::element::UpdateResult::Replaced;
        };
        let focused = self.focused;
        let dragging = self.dragging;
        let preedit = self.preedit.clone();
        let preedit_cursor_byte = self.preedit_cursor_byte;
        let preedit_anchor_byte = self.preedit_anchor_byte;
        let preedit_spans = self.preedit_spans.clone();
        let desired_x = self.desired_x;
        let size = self.size;
        *self = TextViewRenderObject::from_view(view);
        self.focused = focused;
        self.dragging = dragging;
        self.preedit = preedit;
        self.preedit_cursor_byte = preedit_cursor_byte;
        self.preedit_anchor_byte = preedit_anchor_byte;
        self.preedit_spans = preedit_spans;
        self.desired_x = desired_x;
        self.size = size;
        self.compute_layout();
        crate::element::UpdateResult::Updated
    }

    fn update_needs_layout(&self) -> bool {
        true
    }
}

fn clamp_byte_boundary(text: &str, byte: u32) -> u32 {
    let mut byte = (byte as usize).min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte as u32
}
