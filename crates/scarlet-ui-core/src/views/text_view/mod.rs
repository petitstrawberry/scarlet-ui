//! Multi-line editable text view.

mod decoration;
mod document;
mod editing;
mod layout;
pub(crate) mod paint;
mod selection;
#[cfg(test)]
mod tests;

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use decoration::{DecorationIndex, TextDecorations};
pub use decoration::{TextDiagnosticSpan, TextStyleSpan};

pub use document::{EditDelta, TextDocument, TextPosition};
pub(crate) use editing::{
    handle_text_view_focus, handle_text_view_keyboard, handle_text_view_mouse,
    handle_text_view_text_input, handle_text_view_touch,
};
pub use layout::{TextViewLayout, VisualLine};
pub use selection::{TabMode, TextSelection, TextViewScroll, WrapMode};

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{
    Element, ElementRenderObject, LayoutConstraints, RenderElement, TextInputElementState,
};
use crate::geometry::{Point, Rect, Size};
use crate::graphics::FontStack;
use crate::renderer::PaintContext;
use crate::state::{Listenable, State};
use crate::view::View;
use crate::views::style;

const BORDER_WIDTH: f32 = 1.0;

#[derive(Clone)]
enum TextBinding {
    String(State<String>),
    Document(State<TextDocument>),
}

/// A text hit at a widget-local pointer position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextViewHit {
    /// Absolute UTF-8 byte offset at the nearest grapheme boundary.
    pub byte_offset: usize,
    /// Pointer coordinates in logical pixels relative to the TextView viewport.
    pub position: Point,
}

/// Multi-line editable text view.
#[derive(Clone)]
pub struct TextView {
    text: TextBinding,
    selection: State<TextSelection>,
    scroll: Option<State<TextViewScroll>>,
    wrap_mode: Option<State<WrapMode>>,
    static_wrap_mode: WrapMode,
    reveal_selection_on_change: bool,
    autofocus: bool,
    placeholder: String,
    font_size: f32,
    font_stack: Option<Arc<FontStack>>,
    padding: f32,
    tab_mode: TabMode,
    line_numbers: bool,
    current_line_highlight: bool,
    background_color: Color,
    text_color: Color,
    decorations: TextDecorations,
    placeholder_color: Color,
    selection_color: Color,
    current_line_color: Color,
    border_color: Color,
    focused_border_color: Color,
    on_copy: Option<Arc<dyn for<'a> Fn(Cow<'a, str>) + 'static>>,
    on_paste: Option<Arc<dyn Fn() -> Option<Cow<'static, str>> + 'static>>,
    on_text_change: Option<Arc<dyn Fn(&EditDelta) + 'static>>,
    on_hover: Option<Arc<dyn Fn(Option<TextViewHit>) + 'static>>,
    on_modified_click: Option<Arc<dyn Fn(TextViewHit) + 'static>>,
    caret_rect: Option<State<Rect>>,
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

    /// Create a text view bound to a shared immutable document state.
    ///
    /// # Arguments
    ///
    /// * `text` - Document state with shared snapshots and indexed line access.
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
            reveal_selection_on_change: false,
            autofocus: false,
            placeholder: String::new(),
            font_size: 14.0,
            font_stack: None,
            padding: 8.0,
            tab_mode: TabMode::Tab,
            line_numbers: false,
            current_line_highlight: false,
            background_color: style::surface_color(&palette, style::SurfaceRole::Canvas),
            text_color: palette.text(),
            decorations: TextDecorations::default(),
            placeholder_color: palette.text_secondary(),
            selection_color: style::text_selection_highlight(
                palette.primary(),
                style::surface_color(&palette, style::SurfaceRole::Canvas),
            ),
            current_line_color: palette.background_secondary(),
            border_color: palette.divider(),
            focused_border_color: style::focus_highlight(&palette),
            on_copy: None,
            on_paste: None,
            on_text_change: None,
            on_hover: None,
            on_modified_click: None,
            caret_rect: None,
        }
    }

    /// Reveal an externally changed selection, for example Find or Undo.
    /// Scroll-only updates leave the caret where it is, so reading another
    /// part of the document never jumps back to the selection.
    pub fn reveal_selection_on_change(mut self, reveal: bool) -> Self {
        self.reveal_selection_on_change = reveal;
        self
    }

    /// Set initial keyboard focus. An unchanged value preserves subsequent
    /// user focus changes; changing the value explicitly requests that focus.
    pub fn autofocus(mut self, autofocus: bool) -> Self {
        self.autofocus = autofocus;
        self
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

    /// Use an explicit primary/fallback stack for this editor's text.
    ///
    /// Applies to document text, line numbers, placeholder and IME composition.
    /// Font sizes remain logical pixels per em. Other views keep their fonts.
    /// Without this override, the text view uses the global default stack.
    pub fn font_stack(mut self, font_stack: FontStack) -> Self {
        self.font_stack = Some(Arc::new(font_stack));
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

    /// Replace document foreground decorations. See [`TextStyleSpan`] for range
    /// handling and overlap precedence. Uncovered text uses `text_color`.
    ///
    /// Decorations affect painting only. Callers should replace them after
    /// edits; ranges are not automatically rebased onto a new document revision.
    pub fn text_spans(mut self, spans: Vec<TextStyleSpan>) -> Self {
        self.decorations.foreground =
            DecorationIndex::new(spans.into_iter().map(|span| (span.range, span.color)));
        self
    }

    /// Replace document diagnostic underlines. See [`TextDiagnosticSpan`].
    /// As with foreground spans, callers supply ranges for the current document.
    pub fn diagnostic_spans(mut self, spans: Vec<TextDiagnosticSpan>) -> Self {
        self.decorations.diagnostics =
            DecorationIndex::new(spans.into_iter().map(|span| (span.range, span.color)));
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

    /// Observe pointer movement without changing selection. Receives `None`
    /// on exit or outside editable content. Use captured `State` for updates;
    /// applications own hover delays and cancellation.
    pub fn on_hover(mut self, callback: impl Fn(Option<TextViewHit>) + 'static) -> Self {
        self.on_hover = Some(Arc::new(callback));
        self
    }

    /// Handle a primary-modifier left press (Command on macOS, Control elsewhere).
    /// Installing this callback consumes that press without moving the selection.
    pub fn on_modified_click(mut self, callback: impl Fn(TextViewHit) + 'static) -> Self {
        self.on_modified_click = Some(Arc::new(callback));
        self
    }

    /// Publish the caret rectangle in widget-local logical pixels after geometry
    /// changes. Writes only when different; this output is not a view listenable.
    /// Anchor overlays to this state without reserving editor layout space.
    pub fn caret_rect_state(mut self, rect: State<Rect>) -> Self {
        self.caret_rect = Some(rect);
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
    autofocus: bool,
    dragging: bool,
    pub(crate) pointer_modifiers: crate::event::KeyModifiers,
    caret_rect: Option<State<Rect>>,
    desired_x: Option<f32>,
    size: Size,
    layout: TextViewLayout,
    placeholder: String,
    font_size: f32,
    font_stack: Option<Arc<FontStack>>,
    padding: f32,
    tab_mode: TabMode,
    line_numbers: bool,
    current_line_highlight: bool,
    background_color: Color,
    text_color: Color,
    decorations: TextDecorations,
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
        // The first real constraints determine wrapping. Avoid laying out the
        // entire document into one-pixel rows while constructing the shell.
        let layout = TextViewLayout::compute_with_font_stack(
            &TextDocument::new(),
            view.font_size,
            view.padding,
            wrap_mode,
            Size::ZERO,
            TextViewScroll::default(),
            view.line_numbers,
            view.font_stack.as_ref(),
        );

        let requested_selection = view.selection.get();
        let selection = TextSelection {
            anchor: text_document.clamp_position(requested_selection.anchor),
            caret: text_document.clamp_position(requested_selection.caret),
        };

        Self {
            text_document,
            selection,
            scroll,
            wrap_mode,
            preedit: String::new(),
            preedit_cursor_byte: 0,
            preedit_anchor_byte: 0,
            preedit_spans: Vec::new(),
            focused: view.autofocus,
            autofocus: view.autofocus,
            dragging: false,
            pointer_modifiers: crate::event::KeyModifiers::empty(),
            caret_rect: view.caret_rect.clone(),
            desired_x: None,
            size: Size::ZERO,
            layout,
            placeholder: view.placeholder.clone(),
            font_size: view.font_size,
            font_stack: view.font_stack.clone(),
            padding: view.padding,
            tab_mode: view.tab_mode,
            line_numbers: view.line_numbers,
            current_line_highlight: view.current_line_highlight,
            background_color: view.background_color,
            text_color: view.text_color,
            decorations: view.decorations.clone(),
            placeholder_color: view.placeholder_color,
            selection_color: view.selection_color,
            current_line_color: view.current_line_color,
            border_color: view.border_color,
            focused_border_color: view.focused_border_color,
        }
    }

    pub(crate) fn compute_layout(&mut self) {
        if !self.layout.matches_with_font_stack(
            &self.text_document,
            self.font_size,
            self.padding,
            self.wrap_mode,
            self.size,
            self.line_numbers,
            self.font_stack.as_ref(),
        ) {
            self.layout = TextViewLayout::compute_with_font_stack(
                &self.text_document,
                self.font_size,
                self.padding,
                self.wrap_mode,
                self.size,
                self.scroll,
                self.line_numbers,
                self.font_stack.as_ref(),
            );
        }
        self.layout.set_viewport(self.size);
        let base_width = match self.wrap_mode {
            WrapMode::None => self.layout.max_line_width,
            WrapMode::Soft => self.layout.viewport_width,
        };
        self.layout.content_width = base_width;
        if !self.preedit.is_empty() {
            // Composition is painted inline without changing document byte
            // ranges. Allow a temporary horizontal reveal even on wrapped rows.
            let caret = self.cursor_rect();
            let preedit_width = self.layout.measure_text(&self.preedit).0 as f32;
            let composition_end = caret.origin.x + self.layout.scroll.x - self.layout.text_origin_x
                + preedit_width
                + caret.size.width;
            self.layout.content_width = base_width.max(composition_end);
        }
        self.scroll = self.layout.set_scroll(self.scroll);
    }

    fn publish_caret_rect(&self) {
        if let Some(state) = &self.caret_rect {
            let rect = self.text_input_cursor_rect();
            if state.get() != rect {
                state.set(rect);
            }
        }
    }

    /// Resolve a pointer hit without changing selection or scrolling.
    pub fn hit_at_point(&self, position: Point) -> Option<TextViewHit> {
        if !Rect::new(Point::ZERO, self.size).contains(position) {
            return None;
        }
        self.hit_test(position).map(|hit| TextViewHit {
            byte_offset: self.text_document.clamp_position(hit).byte,
            position,
        })
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

    fn text_input_cursor_rect(&self) -> Rect {
        let mut cursor_rect = self.cursor_rect();
        if !self.preedit.is_empty() {
            let prefix = preedit_prefix(&self.preedit, self.preedit_anchor_byte);
            cursor_rect.origin.x += self.layout.measure_text(prefix).0 as f32;
        }
        cursor_rect
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
        self.compute_layout();
        self.publish_caret_rect();
    }

    /// Clear the active IME preedit state.
    pub(crate) fn clear_preedit(&mut self) {
        let had_preedit = !self.preedit.is_empty();
        self.preedit.clear();
        self.preedit_cursor_byte = 0;
        self.preedit_anchor_byte = 0;
        self.preedit_spans.clear();
        if had_preedit {
            self.compute_layout();
            self.publish_caret_rect();
        }
    }

    /// Return the text-input state exposed to platform IME backends.
    ///
    /// # Returns
    ///
    /// Cursor rectangle, surrounding text, and selection byte offsets.
    pub(crate) fn text_input_state(&self) -> TextInputElementState {
        TextInputElementState {
            cursor_rect: self.text_input_cursor_rect(),
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

    fn flush_layout_notifications(&mut self) {
        self.publish_caret_rect();
    }

    fn size(&self) -> Size {
        self.size
    }

    fn render(&mut self) {}

    fn hit_test(&self, point: Point) -> bool {
        Rect::new(Point::ZERO, self.size).contains(point)
    }

    fn paint<'a>(&'a self, ctx: &mut PaintContext<'a>, origin: Point) -> bool {
        paint::paint_text_view_with_decorations(
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
            &self.decorations,
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
        if self.autofocus != view.autofocus {
            self.set_focused(view.autofocus);
            self.autofocus = view.autofocus;
        }
        match &view.text {
            TextBinding::String(text) => {
                let text = text.get();
                if self.text_document.as_str().as_ref() != text {
                    self.text_document = TextDocument::from_str(&text);
                }
            }
            TextBinding::Document(text) => self.text_document = text.get(),
        }
        let requested_selection = view.selection.get();
        let selection = TextSelection {
            anchor: self
                .text_document
                .clamp_position(requested_selection.anchor),
            caret: self.text_document.clamp_position(requested_selection.caret),
        };
        let selection_changed = self.selection != selection;
        self.selection = selection;
        if let Some(scroll) = &view.scroll {
            self.scroll = scroll.get();
        }
        self.wrap_mode = view
            .wrap_mode
            .as_ref()
            .map(State::get)
            .unwrap_or(view.static_wrap_mode);
        self.caret_rect = view.caret_rect.clone();
        self.placeholder.clone_from(&view.placeholder);
        self.font_size = view.font_size;
        let font_changed = self.font_stack.as_deref().map(FontStack::cache_id)
            != view.font_stack.as_deref().map(FontStack::cache_id);
        self.font_stack.clone_from(&view.font_stack);
        self.padding = view.padding;
        self.tab_mode = view.tab_mode;
        self.line_numbers = view.line_numbers;
        self.current_line_highlight = view.current_line_highlight;
        self.background_color = view.background_color;
        self.text_color = view.text_color;
        self.decorations = view.decorations.clone();
        self.placeholder_color = view.placeholder_color;
        self.selection_color = view.selection_color;
        self.current_line_color = view.current_line_color;
        self.border_color = view.border_color;
        self.focused_border_color = view.focused_border_color;
        self.compute_layout();
        if font_changed {
            self.desired_x = None;
        }
        if selection_changed {
            self.desired_x = None;
            if view.reveal_selection_on_change {
                editing::ensure_caret_visible(view, self);
            }
        }
        self.publish_caret_rect();
        crate::element::UpdateResult::Updated
    }

    fn update_needs_layout(&self) -> bool {
        // TextView fills its constraints; selection, scroll, text and styling
        // only change internal geometry, already refreshed in update(). Avoid
        // re-running ancestor measurement passes on every selection event.
        false
    }
}

fn clamp_byte_boundary(text: &str, byte: u32) -> u32 {
    let mut byte = (byte as usize).min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte as u32
}

fn preedit_prefix(preedit: &str, byte: u32) -> &str {
    let byte = clamp_byte_boundary(preedit, byte) as usize;
    &preedit[..byte]
}
