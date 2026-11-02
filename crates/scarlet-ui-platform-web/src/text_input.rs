//! Invisible DOM editing surface. ScarletUI remains responsible for all paint.
//! Architectural references (no copied implementation): Flutter text_editing.dart
//! (BSD-3-Clause), egui eframe/web/text_agent.rs (MIT OR Apache-2.0).
use crate::{FrameLoop, text_edit};
use scarlet_ui_core::{
    element::TextInputElementState,
    event::{Event, FocusEvent, KeyEvent, KeyModifiers},
    geometry::{Point, Rect, Size},
};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::{Rc, Weak},
};
use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{HtmlCanvasElement, HtmlTextAreaElement, KeyboardEvent};

/// CSS viewport coordinates -> canvas/content logical coordinates. DPR is
/// intentionally absent: both browser rectangles use CSS pixels already.
fn visible_content_rect(
    canvas: Rect,
    logical: Size,
    viewport: Rect,
    menu_height: f32,
) -> Option<Rect> {
    if canvas.size.width <= 0.0 || canvas.size.height <= 0.0 {
        return None;
    }
    let sx = logical.width / canvas.size.width;
    let sy = logical.height / canvas.size.height;
    let left = canvas.origin.x.max(viewport.origin.x);
    let top = canvas.origin.y.max(viewport.origin.y);
    let right = (canvas.origin.x + canvas.size.width).min(viewport.origin.x + viewport.size.width);
    let bottom =
        (canvas.origin.y + canvas.size.height).min(viewport.origin.y + viewport.size.height);
    if right <= left || bottom <= top {
        return None;
    }
    let x = ((left - canvas.origin.x) * sx).max(0.0);
    let y = ((top - canvas.origin.y) * sy - menu_height).max(0.0);
    let end_x = ((right - canvas.origin.x) * sx).min(logical.width);
    let end_y = ((bottom - canvas.origin.y) * sy - menu_height).min(logical.height - menu_height);
    if end_x <= x || end_y <= y {
        return None;
    }
    Some(Rect::from_xywh(x, y, end_x - x, end_y - y))
}

type Listener = (
    web_sys::EventTarget,
    &'static str,
    Closure<dyn FnMut(web_sys::Event)>,
);
pub(crate) struct TextInput {
    pub element: HtmlTextAreaElement,
    canvas: HtmlCanvasElement,
    state: RefCell<Option<TextInputElementState>>,
    value: RefCell<String>,
    composing: Cell<bool>,
    suspended: Cell<bool>,
    menu_height: Cell<f32>,
    last_visible_rect: Cell<Option<Rect>>,
    composition_state: RefCell<Option<TextInputElementState>>,
    composition_selection: Cell<(u32, u32)>,
    composition_expected_selection: Cell<(u32, u32)>,
    updating: Cell<bool>,
    events: Rc<RefCell<VecDeque<Event>>>,
    runner: Weak<RefCell<FrameLoop>>,
    listeners: RefCell<Vec<Listener>>,
}
impl TextInput {
    pub fn new(
        canvas: HtmlCanvasElement,
        events: Rc<RefCell<VecDeque<Event>>>,
        runner: Weak<RefCell<FrameLoop>>,
    ) -> Result<Rc<Self>, JsValue> {
        let document = canvas
            .owner_document()
            .ok_or_else(|| JsValue::from_str("No document"))?;
        let element: HtmlTextAreaElement = document.create_element("textarea")?.dyn_into()?;
        element.set_attribute("data-scarlet-text-input", "")?;
        element.set_attribute("aria-label", "Text input")?;
        element.set_attribute("autocapitalize", "off")?;
        element.set_attribute("autocomplete", "off")?;
        element.set_attribute("spellcheck", "false")?;
        // The proxy is one pixel wide, not the rendered editor's line box.
        // Native Home/End/Up/Down must follow logical lines, never proxy wraps.
        element.set_attribute("wrap", "off")?;
        element.set_tab_index(-1);
        element.set_attribute("style", "position:absolute;left:0;top:0;width:1px;height:20px;font-size:16px;padding:0;margin:0;border:0;outline:none;resize:none;overflow:hidden;white-space:pre;overflow-wrap:normal;word-break:normal;opacity:0;color:transparent;background:transparent;caret-color:transparent;pointer-events:none;forced-color-adjust:none")?;
        document
            .document_element()
            .ok_or_else(|| JsValue::from_str("No document element"))?
            .append_child(&element)?;
        let this = Rc::new(Self {
            element,
            canvas,
            state: RefCell::new(None),
            value: RefCell::new(String::new()),
            composing: Cell::new(false),
            suspended: Cell::new(false),
            menu_height: Cell::new(0.0),
            last_visible_rect: Cell::new(None),
            composition_state: RefCell::new(None),
            composition_selection: Cell::new((0, 0)),
            composition_expected_selection: Cell::new((0, 0)),
            updating: Cell::new(false),
            events,
            runner,
            listeners: RefCell::new(Vec::new()),
        });
        for kind in [
            "beforeinput",
            "input",
            "compositionstart",
            "compositionend",
            "select",
            "keyup",
            "keydown",
            "focus",
            "blur",
        ] {
            let weak = Rc::downgrade(&this);
            let callback = Closure::wrap(Box::new(move |event: web_sys::Event| {
                if let Some(this) = weak.upgrade() {
                    this.handle(kind, event);
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            this.element
                .add_event_listener_with_callback(kind, callback.as_ref().unchecked_ref())?;
            this.listeners
                .borrow_mut()
                .push((this.element.clone().into(), kind, callback));
        }
        // WebKit selection handles and mobile keyboards need document-level
        // selectionchange; select/keyup alone do not cover those edits.
        let weak = Rc::downgrade(&this);
        let selection = Closure::wrap(Box::new(move |event: web_sys::Event| {
            if let Some(this) = weak.upgrade() {
                this.handle("select", event);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        document.add_event_listener_with_callback(
            "selectionchange",
            selection.as_ref().unchecked_ref(),
        )?;
        this.listeners
            .borrow_mut()
            .push((document.into(), "selectionchange", selection));
        if let Some(viewport) = web_sys::window().and_then(|w| w.visual_viewport()) {
            for kind in ["resize", "scroll"] {
                let weak = Rc::downgrade(&this);
                let callback = Closure::wrap(Box::new(move |_: web_sys::Event| {
                    if let Some(this) = weak.upgrade() {
                        this.position();
                        if kind == "resize" {
                            this.request_reveal(false);
                            this.flush();
                        }
                    }
                }) as Box<dyn FnMut(web_sys::Event)>);
                viewport
                    .add_event_listener_with_callback(kind, callback.as_ref().unchecked_ref())?;
                this.listeners
                    .borrow_mut()
                    .push((viewport.clone().into(), kind, callback));
            }
        }
        Ok(this)
    }
    pub fn focused(&self) -> bool {
        self.element
            .owner_document()
            .and_then(|d| d.active_element())
            .is_some_and(|e| e == self.element.clone().into())
    }
    fn flush(&self) {
        if let Some(runner) = self.runner.upgrade() {
            crate::tick_now(&runner);
        }
    }
    fn offsets(&self) -> (u32, u32) {
        let value = self.element.value();
        let start = text_edit::utf16_to_byte(
            &value,
            self.element.selection_start().ok().flatten().unwrap_or(0),
        );
        let end = text_edit::utf16_to_byte(
            &value,
            self.element.selection_end().ok().flatten().unwrap_or(0),
        );
        if self.element.selection_direction().ok().flatten().as_deref() == Some("backward") {
            (start, end)
        } else {
            (end, start)
        }
    }
    fn handle(&self, kind: &str, event: web_sys::Event) {
        if self.updating.get() {
            return;
        }
        if self.suspended.get() {
            // Late keyboard/composition events must not queue stale edits for
            // replay after resume, or mutate the retained DOM baseline.
            if matches!(kind, "input" | "compositionend") {
                self.apply_state();
            }
            return;
        }
        if matches!(
            kind,
            "beforeinput" | "input" | "compositionstart" | "compositionend"
        ) && !self.focused()
        {
            self.apply_state();
            return;
        }
        match kind {
            "compositionstart" => {
                let snapshot = self.state.borrow().clone();
                if let Some(state) = &snapshot {
                    self.composition_expected_selection
                        .set((state.cursor_byte, state.anchor_byte));
                }
                *self.composition_state.borrow_mut() = snapshot;
                self.composition_selection.set(self.offsets());
                self.composing.set(true);
            }
            "compositionend" => {
                self.commit_dom();
            }
            "input" if self.composing.get() => {
                let value = self.element.value();
                let old = self.value.borrow();
                let (cursor, anchor) = self.composition_selection.get();
                let (start, end, text) =
                    text_edit::composition_replacement(&old, &value, cursor, anchor);
                self.composition_expected_selection
                    .set((end as u32, start as u32));
                self.events.borrow_mut().extend([
                    text_edit::selection(end as u32, start as u32),
                    text_edit::preedit(text.into()),
                ]);
                drop(old);
                self.flush();
                self.request_reveal(true);
                self.flush();
            }
            "input" => self.commit_dom(),
            "select" | "keyup" if !self.composing.get() && self.focused() => {
                let mut released = false;
                if kind == "keyup" {
                    if let Some(key) = event.dyn_ref::<KeyboardEvent>() {
                        if matches!(key.key().as_str(), "Tab" | "Enter" | "Escape") {
                            self.events.borrow_mut().push_back(Event::Keyboard(
                                KeyEvent::Released {
                                    keycode: crate::map_key(&key.key()),
                                    modifiers: KeyModifiers {
                                        shift: key.shift_key(),
                                        control: key.ctrl_key(),
                                        alt: key.alt_key(),
                                        super_key: key.meta_key(),
                                    },
                                },
                            ));
                            released = true;
                        }
                    }
                }
                let (cursor, anchor) = self.offsets();
                let changed = self
                    .state
                    .borrow()
                    .as_ref()
                    .is_some_and(|s| s.cursor_byte != cursor || s.anchor_byte != anchor);
                if changed {
                    self.events
                        .borrow_mut()
                        .push_back(text_edit::selection(cursor, anchor));
                }
                if changed || released {
                    self.flush();
                }
            }
            "beforeinput" if !self.composing.get() => {
                let Some(input) = event.dyn_ref::<web_sys::InputEvent>() else {
                    return;
                };
                if input.cancelable()
                    && matches!(
                        input.input_type().as_str(),
                        "insertLineBreak" | "insertParagraph"
                    )
                {
                    input.prevent_default();
                    // Some mobile keyboards provide only beforeinput, with
                    // no keyup. This is a synthetic command, so balance it here.
                    self.events.borrow_mut().extend([
                        Event::Keyboard(KeyEvent::Pressed {
                            keycode: crate::map_key("Enter"),
                            modifiers: KeyModifiers::empty(),
                        }),
                        Event::Keyboard(KeyEvent::Released {
                            keycode: crate::map_key("Enter"),
                            modifiers: KeyModifiers::empty(),
                        }),
                    ]);
                    self.flush();
                }
            }
            "keydown" => {
                let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
                    return;
                };
                if self.composing.get() || key.is_composing() || key.key_code() == 229 {
                    return;
                }
                // DOM owns ordinary editing, navigation, deletion and clipboard.
                // Core owns submit/newline, focus traversal and Escape.
                if matches!(key.key().as_str(), "Tab" | "Enter" | "Escape") {
                    key.prevent_default();
                    self.events
                        .borrow_mut()
                        .push_back(Event::Keyboard(KeyEvent::Pressed {
                            keycode: crate::map_key(&key.key()),
                            modifiers: KeyModifiers {
                                shift: key.shift_key(),
                                control: key.ctrl_key(),
                                alt: key.alt_key(),
                                super_key: key.meta_key(),
                            },
                        }));
                    self.flush();
                }
            }
            "blur" => {
                self.reset_reveal();
                self.commit_dom();
                let internal = event
                    .dyn_ref::<web_sys::FocusEvent>()
                    .and_then(|e| e.related_target())
                    .is_some_and(|t| t == self.canvas.clone().into());
                if !internal {
                    self.events
                        .borrow_mut()
                        .push_back(Event::Focus(FocusEvent::Lost));
                    self.flush();
                }
            }
            _ => {}
        }
    }
    pub fn commit_dom(&self) {
        if self.suspended.get() {
            return;
        }
        let value = self.element.value();
        let (cursor, anchor) = self.offsets();
        let was_composing = self.composing.replace(false);
        self.composition_state.borrow_mut().take();
        let old = self.value.replace(value.clone());
        if old == value && !was_composing {
            return;
        }
        self.updating.set(true);
        if was_composing {
            self.events
                .borrow_mut()
                .push_back(text_edit::preedit(String::new()));
        }
        self.events
            .borrow_mut()
            .extend(text_edit::edit_events(&old, &value, cursor, anchor));
        self.flush();
        self.updating.set(false);
        self.apply_state();
        self.request_reveal(true);
        self.flush();
    }
    pub fn sync(&self, state: Option<&TextInputElementState>, menu_height: f32) {
        self.menu_height.set(menu_height);
        let mut state = state.cloned();
        let state_cleared = state.is_none() && self.state.borrow().is_some();
        let caret_changed = {
            let old = self.state.borrow();
            match (old.as_ref(), state.as_ref()) {
                (Some(old), Some(new)) => {
                    old.editor_id != new.editor_id
                        || old.surrounding_text != new.surrounding_text
                        || old.cursor_byte != new.cursor_byte
                        || old.anchor_byte != new.anchor_byte
                }
                (None, Some(_)) => true,
                _ => false,
            }
        };
        if state.is_none() {
            self.reset_reveal();
        }
        if let Some(state) = &mut state {
            state.cursor_rect.origin.y += menu_height;
            state.input_rect.origin.y += menu_height;
        }
        let cancel_composition = self.composing.get() && {
            let original = self.composition_state.borrow();
            match (original.as_ref(), state.as_ref()) {
                (Some(old), Some(new)) => {
                    old.editor_id != new.editor_id
                        || old.surrounding_text != new.surrounding_text
                        || (new.cursor_byte, new.anchor_byte)
                            != self.composition_expected_selection.get()
                }
                _ => true,
            }
        };
        if cancel_composition {
            let same_editor = self
                .composition_state
                .borrow()
                .as_ref()
                .zip(state.as_ref())
                .is_some_and(|(old, new)| old.editor_id == new.editor_id);
            if same_editor {
                // Focus loss clears an old editor's preedit. A retained editor
                // receiving external text/selection needs an explicit clear.
                self.events
                    .borrow_mut()
                    .push_back(text_edit::preedit(String::new()));
            }
            self.composing.set(false);
            self.composition_state.borrow_mut().take();
            // Do not commit the old DOM value into a newly focused/replaced
            // editor. Blurring ends the browser's old IME session; callbacks
            // during this operation are deliberately suppressed.
            self.reset_reveal();
            self.updating.set(true);
            let _ = self.element.blur();
            self.updating.set(false);
        }
        *self.state.borrow_mut() = state;
        self.position();
        if !self.composing.get() && !self.updating.get() {
            // Native navigation changes DOM selection before asynchronous
            // selectionchange/keyup arrives. An unchanged RAF/core snapshot is
            // not an instruction to overwrite that pending native selection.
            if caret_changed || state_cleared || cancel_composition {
                self.apply_state();
            }
            if caret_changed {
                self.request_reveal(true);
            }
        }
    }
    fn reset_reveal(&self) {
        if self.last_visible_rect.take().is_some() {
            self.events
                .borrow_mut()
                .push_back(Event::TextInputReveal { visible_rect: None });
        }
    }
    fn request_reveal(&self, force: bool) {
        if self.suspended.get() || !self.focused() || self.state.borrow().is_none() {
            return;
        }
        let Some(browser) = web_sys::window() else {
            return;
        };
        let dom = self.canvas.get_bounding_client_rect();
        let viewport = if let Some(viewport) = browser.visual_viewport() {
            Rect::from_xywh(
                viewport.offset_left() as f32,
                viewport.offset_top() as f32,
                viewport.width() as f32,
                viewport.height() as f32,
            )
        } else {
            Rect::from_xywh(
                0.0,
                0.0,
                browser
                    .inner_width()
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32,
                browser
                    .inner_height()
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32,
            )
        };
        let rect = visible_content_rect(
            Rect::from_xywh(
                dom.left() as f32,
                dom.top() as f32,
                dom.width() as f32,
                dom.height() as f32,
            ),
            Size::new(
                self.canvas.client_width() as f32,
                self.canvas.client_height() as f32,
            ),
            viewport,
            self.menu_height.get(),
        );
        let Some(rect) = rect else {
            self.reset_reveal();
            return;
        };
        if force || self.last_visible_rect.get() != Some(rect) {
            self.last_visible_rect.set(Some(rect));
            self.events.borrow_mut().push_back(Event::TextInputReveal {
                visible_rect: Some(rect),
            });
        }
    }
    fn apply_state(&self) {
        self.updating.set(true);
        if let Some(state) = self.state.borrow().as_ref() {
            if self.element.value() != state.surrounding_text {
                self.element.set_value(&state.surrounding_text);
            }
            *self.value.borrow_mut() = state.surrounding_text.clone();
            let cursor = text_edit::byte_to_utf16(&state.surrounding_text, state.cursor_byte);
            let anchor = text_edit::byte_to_utf16(&state.surrounding_text, state.anchor_byte);
            let start = cursor.min(anchor);
            let end = cursor.max(anchor);
            let direction = if cursor < anchor {
                "backward"
            } else {
                "forward"
            };
            if self.element.selection_start().ok().flatten() != Some(start)
                || self.element.selection_end().ok().flatten() != Some(end)
                || self.element.selection_direction().ok().flatten().as_deref() != Some(direction)
            {
                let _ = self
                    .element
                    .set_selection_range_with_direction(start, end, direction);
            }
        } else {
            self.element.set_value("");
            self.value.borrow_mut().clear();
            if self.focused() {
                let _ = self.canvas.focus();
            }
        }
        self.updating.set(false);
    }
    pub fn position(&self) {
        let state = self.state.borrow();
        let Some(state) = state.as_ref() else {
            let style = self.element.style();
            let _ = style.set_property("left", "0px");
            let _ = style.set_property("top", "0px");
            let _ = style.set_property("height", "1px");
            return;
        };
        let rect = self.canvas.get_bounding_client_rect();
        let browser = web_sys::window().unwrap();
        let scroll_x = browser.scroll_x().unwrap_or(0.0);
        let scroll_y = browser.scroll_y().unwrap_or(0.0);
        let sx = rect.width() / f64::from(self.canvas.client_width().max(1));
        let sy = rect.height() / f64::from(self.canvas.client_height().max(1));
        let style = self.element.style();
        let viewport_width = self
            .element
            .owner_document()
            .and_then(|document| document.document_element())
            .map(|root| f64::from(root.client_width()))
            .unwrap_or(rect.width());
        // A scrolled/offscreen canvas caret must not widen the browser page.
        let left = (rect.left() + scroll_x + f64::from(state.cursor_rect.origin.x) * sx)
            .clamp(scroll_x, scroll_x + (viewport_width - 1.0).max(0.0));
        let _ = style.set_property("left", &format!("{}px", left));
        let _ = style.set_property(
            "top",
            &format!(
                "{}px",
                rect.top() + scroll_y + f64::from(state.cursor_rect.origin.y) * sy
            ),
        );
        let _ = style.set_property(
            "height",
            &format!(
                "{}px",
                (f64::from(state.cursor_rect.size.height) * sy).max(16.0)
            ),
        );
    }
    /// Called only after a completed, unmoved primary tap has run through core.
    pub fn keyboard_focus(&self, allow_focus: impl Fn() -> bool) {
        if !allow_focus() {
            return;
        }
        self.flush();
        let has_state = self.state.borrow().is_some();
        if allow_focus() && has_state {
            self.focus_editor();
        }
    }
    pub fn suspend(&self) {
        if self.suspended.get() {
            return;
        }
        self.commit_dom();
        let was_focused = self.focused();
        self.reset_reveal();
        self.suspended.set(true);
        self.element.set_inert(true);
        let _ = self.element.blur();
        if was_focused {
            self.events
                .borrow_mut()
                .push_back(Event::Focus(FocusEvent::Lost));
        }
        // An unfocused bridge may still have a pending reveal-reset request.
        self.flush();
    }
    pub fn resume(&self) {
        if !self.suspended.replace(false) {
            return;
        }
        self.element.set_inert(false);
        self.apply_state();
    }
    fn focus_editor(&self) {
        if self.suspended.get() {
            return;
        }
        // A resolved tap/focus command is authoritative even if it selects the
        // same core caret. Unlike an unchanged RAF, discard pending DOM ranges.
        if !self.composing.get() && !self.updating.get() {
            self.apply_state();
        }
        self.position();
        let options = web_sys::FocusOptions::new();
        options.set_prevent_scroll(true);
        let _ = self.element.focus_with_options(&options);
        self.request_reveal(true);
        self.flush();
    }
    pub fn focus_at(&self, x: i32, y: i32) {
        let hit = self
            .state
            .borrow()
            .as_ref()
            .is_some_and(|s| s.input_rect.contains(Point::new(x as f32, y as f32)));
        if hit {
            self.focus_editor();
        }
    }
}
impl Drop for TextInput {
    fn drop(&mut self) {
        for (target, kind, listener) in self.listeners.get_mut().drain(..) {
            let _ =
                target.remove_event_listener_with_callback(kind, listener.as_ref().unchecked_ref());
        }
        self.element.remove();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui_core::{
        View,
        geometry::{Rect, Size},
        pipeline::RenderingPipeline,
        state::{State, generate_state_id},
        views::{TextField, TextView},
    };
    use wasm_bindgen_test::*;
    wasm_bindgen_test_configure!(run_in_browser);

    struct Field {
        bridge: Rc<TextInput>,
        _runner: Rc<RefCell<FrameLoop>>,
        text: State<String>,
    }
    impl Field {
        fn new(initial: &str) -> Self {
            Self::new_with_multiline(initial, false)
        }
        fn new_with_multiline(initial: &str, multiline: bool) -> Self {
            let document = web_sys::window().unwrap().document().unwrap();
            let canvas: HtmlCanvasElement = document
                .create_element("canvas")
                .unwrap()
                .dyn_into()
                .unwrap();
            canvas
                .set_attribute(
                    "style",
                    "position:fixed;left:0;top:0;width:300px;height:200px;display:block",
                )
                .unwrap();
            canvas.set_tab_index(0);
            document.body().unwrap().append_child(&canvas).unwrap();
            let events = Rc::new(RefCell::new(VecDeque::new()));
            let runner = Rc::new(RefCell::new(FrameLoop {
                tick: None,
                frame: None,
                error: None,
                paused: false,
                text_input: Weak::new(),
            }));
            let bridge = TextInput::new(canvas, events.clone(), Rc::downgrade(&runner)).unwrap();
            runner.borrow_mut().text_input = Rc::downgrade(&bridge);
            let text = State::new(generate_state_id(), initial.to_owned());
            let mut pipeline = RenderingPipeline::new();
            pipeline.set_root(if multiline {
                TextView::new(
                    text.clone(),
                    State::new(
                        generate_state_id(),
                        scarlet_ui_core::views::TextSelection::collapsed(0),
                    ),
                )
                .create_element()
            } else {
                TextField::new(text.clone())
                    .autofocus(true)
                    .create_element()
            });
            pipeline.layout_initial();
            if multiline {
                use scarlet_ui_core::event::{MouseButton, MouseEvent};
                pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x: 10,
                    y: 10,
                    click_count: 1,
                }));
                pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x: 10,
                    y: 10,
                    click_count: 1,
                }));
                assert!(pipeline.focused_text_input_state().is_some());
            }
            bridge.sync(pipeline.focused_text_input_state().as_ref(), 0.0);
            let weak = Rc::downgrade(&bridge);
            runner.borrow_mut().tick = Some(Box::new(move || {
                loop {
                    let event = events.borrow_mut().pop_front();
                    let Some(event) = event else {
                        break;
                    };
                    pipeline.handle_event(&event);
                }
                if let Some(bridge) = weak.upgrade() {
                    bridge.sync(pipeline.focused_text_input_state().as_ref(), 0.0);
                }
                Ok(Some(std::time::Duration::ZERO))
            }));
            bridge.focus_editor();
            Self {
                bridge,
                _runner: runner,
                text,
            }
        }
        fn send(&self, kind: &str) {
            self.bridge
                .element
                .dispatch_event(&web_sys::Event::new(kind).unwrap())
                .unwrap();
        }
        fn input(&self, value: &str, cursor: u32) {
            self.bridge.element.set_value(value);
            self.bridge
                .element
                .set_selection_range(cursor, cursor)
                .unwrap();
            self.send("input");
        }
    }
    impl Drop for Field {
        fn drop(&mut self) {
            self.bridge.canvas.remove();
        }
    }

    #[wasm_bindgen_test]
    fn minimal_dom_input_route_and_mobile_deletion() {
        let field = Field::new("");
        field.input("日本😀", 4);
        assert_eq!(field.text.get(), "日本😀");
        assert_eq!(
            field.bridge.state.borrow().as_ref().unwrap().cursor_byte,
            10
        );
        field.input("日本", 2);
        assert_eq!(field.text.get(), "日本");
        field.input("", 0);
        assert_eq!(field.text.get(), "");
    }
    #[wasm_bindgen_test]
    fn simulated_composition_preserves_dom_and_commits_once() {
        let field = Field::new("ab");
        field.bridge.element.set_selection_range(1, 1).unwrap();
        field.send("compositionstart");
        field.input("aにb", 2);
        assert_eq!(field.text.get(), "ab");
        assert_eq!(field.bridge.element.value(), "aにb");
        field.input("a日本b", 3);
        field.send("compositionend");
        field.send("input"); // final non-composing input can follow compositionend
        assert_eq!(field.text.get(), "a日本b");
        assert_eq!(field.bridge.element.selection_start().unwrap(), Some(3));
    }
    #[wasm_bindgen_test]
    fn repeated_focus_field_switch_and_geometry_sync() {
        let field = Field::new("one");
        // This test supplies synthetic layout states rather than pipeline states.
        field._runner.borrow_mut().tick = None;
        let _ = field.bridge.element.blur();
        let mut state = TextInputElementState {
            editor_id: None,
            input_rect: Rect::new(Point::new(10.0, 20.0), Size::new(100.0, 30.0)),
            cursor_rect: Rect::new(Point::new(40.0, 22.0), Size::new(1.0, 20.0)),
            surrounding_text: "日本😀".into(),
            cursor_byte: 6,
            anchor_byte: 0,
        };
        field.bridge.sync(Some(&state), 25.0);
        field.bridge.focus_at(250, 150); // ordinary non-editor tap
        assert!(!field.bridge.focused());
        field.bridge.focus_at(20, 50);
        assert!(field.bridge.focused());
        field.bridge.focus_at(20, 50);
        assert!(field.bridge.focused());
        assert_eq!(field.bridge.element.selection_start().unwrap(), Some(0));
        assert_eq!(field.bridge.element.selection_end().unwrap(), Some(2));
        let before = field
            .bridge
            .element
            .style()
            .get_property_value("top")
            .unwrap();
        state.cursor_rect.origin.y += 30.0; // scroll/layout position update
        state.surrounding_text = "next".into();
        state.cursor_byte = 4;
        state.anchor_byte = 4;
        field.bridge.sync(Some(&state), 25.0);
        assert_ne!(
            field
                .bridge
                .element
                .style()
                .get_property_value("top")
                .unwrap(),
            before
        );
        assert_eq!(field.bridge.element.value(), "next");
        assert!(field.bridge.focused());
        field.bridge.sync(None, 25.0);
        assert!(!field.bridge.focused());
        assert_eq!(
            field
                .bridge
                .element
                .style()
                .get_property_value("left")
                .unwrap(),
            "0px"
        );
        assert_eq!(
            field
                .bridge
                .element
                .style()
                .get_property_value("top")
                .unwrap(),
            "0px"
        );
        assert_eq!(field.bridge.element.value(), "");
    }
    #[wasm_bindgen_test]
    fn pause_commits_then_blurs_and_ignores_late_input_without_replay() {
        let field = Field::new("old");
        let app = crate::WebApp {
            state: field._runner.clone(),
        };
        field.bridge.element.set_value("pending");
        field.bridge.element.set_selection_range(7, 7).unwrap();
        app.pause();
        assert_eq!(field.text.get(), "pending");
        assert!(!field.bridge.focused());
        assert!(field.bridge.element.inert());
        field.input("stale", 5);
        field.send("compositionend");
        assert_eq!(field.text.get(), "pending");
        app.resume();
        // The normal RAF tick, not a refocus, must not replay paused edits.
        field.bridge.flush();
        assert_eq!(field.text.get(), "pending");
        assert!(!field.bridge.focused());
        assert!(!field.bridge.element.inert());
        app.pause(); // Cancel the scheduled RAF before this test drops its fixture.
    }

    #[wasm_bindgen_test]
    fn composition_target_change_discards_late_old_dom_commit() {
        let field = Field::new("ab");
        field.bridge.element.set_selection_range(1, 1).unwrap();
        field.send("compositionstart");
        field.input("aにb", 2);
        let mut replacement = field.bridge.state.borrow().clone().unwrap();
        replacement.editor_id = Some(scarlet_ui_core::element::ElementId::generate());
        replacement.surrounding_text = "next".into();
        replacement.cursor_byte = 4;
        replacement.anchor_byte = 4;
        field.bridge.sync(Some(&replacement), 0.0);
        assert!(!field.bridge.composing.get());
        assert!(!field.bridge.focused());
        assert_eq!(field.bridge.element.value(), "next");
        field.bridge.element.set_value("a日本b");
        field.send("compositionend");
        field.send("input");
        assert_eq!(field.text.get(), "ab");
        assert_eq!(field.bridge.element.value(), "next");
    }

    #[wasm_bindgen_test]
    fn external_text_or_selection_change_cancels_retained_editor_composition() {
        for change_text in [false, true] {
            let field = Field::new("ab");
            field.bridge.element.set_selection_range(1, 1).unwrap();
            field.send("compositionstart");
            field.input("aにb", 2);
            let mut replacement = field.bridge.state.borrow().clone().unwrap();
            if change_text {
                replacement.surrounding_text = "xy".into();
            } else {
                replacement.cursor_byte = 0;
                replacement.anchor_byte = 0;
            }
            field.bridge.sync(Some(&replacement), 0.0);
            assert!(!field.bridge.composing.get());
            assert!(!field.bridge.focused());
            assert!(field.bridge.events.borrow().iter().any(
                |event| matches!(event, Event::TextInputPreedit { text, .. } if text.is_empty())
            ));
            field.send("compositionend");
            assert_eq!(field.text.get(), "ab");
        }
    }

    #[wasm_bindgen_test]
    fn offscreen_caret_does_not_extend_horizontal_document_bounds() {
        let field = Field::new("ab");
        let mut state = field.bridge.state.borrow().clone().unwrap();
        state.cursor_rect.origin.x = 1_000_000.0;
        field.bridge.sync(Some(&state), 0.0);
        let left: f64 = field
            .bridge
            .element
            .style()
            .get_property_value("left")
            .unwrap()
            .trim_end_matches("px")
            .parse()
            .unwrap();
        let document = field.bridge.element.owner_document().unwrap();
        let viewport = document.document_element().unwrap().client_width() as f64;
        assert!(left >= 0.0 && left < viewport);
        field.bridge.sync(None, 0.0);
        assert_eq!(
            field
                .bridge
                .element
                .style()
                .get_property_value("left")
                .unwrap(),
            "0px"
        );
        assert_eq!(
            field
                .bridge
                .element
                .style()
                .get_property_value("top")
                .unwrap(),
            "0px"
        );
    }

    #[wasm_bindgen_test]
    fn menu_keyboard_guard_does_not_refocus_editor_or_change_selection() {
        let field = Field::new("selected");
        field.bridge.element.set_selection_range(1, 4).unwrap();
        field.send("select");
        let before = field.bridge.state.borrow().clone().unwrap();
        field.bridge.canvas.focus().unwrap(); // Internal transfer preserves core focus.
        assert!(!field.bridge.focused());
        field.bridge.keyboard_focus(|| false); // Menu owned key before dispatch (including Escape).
        assert!(!field.bridge.focused());
        let calls = Cell::new(0);
        field.bridge.keyboard_focus(|| {
            calls.set(calls.get() + 1);
            calls.get() == 1 // Menu acquired keys while the runner drained.
        });
        assert!(!field.bridge.focused());
        assert_eq!(field.text.get(), "selected");
        let after = field.bridge.state.borrow().clone().unwrap();
        assert_eq!(
            (after.cursor_byte, after.anchor_byte),
            (before.cursor_byte, before.anchor_byte)
        );
    }

    #[wasm_bindgen_test]
    fn viewport_conversion_handles_embeds_scale_scroll_and_menu_offset() {
        // Canvas is CSS-scaled2x. No backing-store/DPR units enter this conversion.
        let canvas = Rect::from_xywh(100.0, 200.0, 600.0, 400.0);
        let logical = Size::new(300.0, 200.0);
        assert_eq!(
            visible_content_rect(
                canvas,
                logical,
                Rect::from_xywh(0.0, 0.0, 800.0, 500.0),
                28.0
            ),
            Some(Rect::from_xywh(0.0, 0.0, 300.0, 122.0))
        );
        // Page scroll moves client rect; visualViewport offsets model pinch/pan.
        assert_eq!(
            visible_content_rect(
                Rect::from_xywh(-100.0, -200.0, 600.0, 400.0),
                logical,
                Rect::from_xywh(20.0, 30.0, 250.0, 100.0),
                28.0
            ),
            Some(Rect::from_xywh(60.0, 87.0, 125.0, 50.0))
        );
        assert_eq!(
            visible_content_rect(
                canvas,
                logical,
                Rect::from_xywh(0.0, 0.0, 50.0, 100.0),
                28.0
            ),
            None
        );
        // Keyboard dismissal restores the complete content viewport.
        assert_eq!(
            visible_content_rect(
                canvas,
                logical,
                Rect::from_xywh(0.0, 0.0, 800.0, 800.0),
                28.0
            ),
            Some(Rect::from_xywh(0.0, 0.0, 300.0, 172.0))
        );
    }

    #[wasm_bindgen_test]
    fn position_only_sync_does_not_request_reveal_but_selection_and_blur_do() {
        let field = Field::new("abc");
        field.bridge.events.borrow_mut().clear();
        let mut state = field.bridge.state.borrow().clone().unwrap();
        state.cursor_rect.origin.y += 50.0; // Manual ancestor scrolling, not editing.
        field.bridge.sync(Some(&state), 0.0);
        assert!(field.bridge.events.borrow().is_empty());
        state.cursor_byte = if state.cursor_byte == 0 { 1 } else { 0 };
        state.anchor_byte = state.cursor_byte;
        field.bridge.sync(Some(&state), 0.0);
        assert!(field.bridge.events.borrow().iter().any(|event| matches!(
            event,
            Event::TextInputReveal {
                visible_rect: Some(_)
            }
        )));
        field.bridge.events.borrow_mut().clear();
        field.bridge.sync(None, 0.0);
        assert!(
            field
                .bridge
                .events
                .borrow()
                .iter()
                .any(|event| matches!(event, Event::TextInputReveal { visible_rect: None }))
        );
        field.bridge.events.borrow_mut().clear();
        field.bridge.sync(None, 0.0);
        assert!(field.bridge.events.borrow().is_empty());
    }

    #[wasm_bindgen_test]
    fn mobile_beforeinput_newline_is_one_balanced_command_and_ime_is_untouched() {
        let field = Field::new("");
        let captured = Rc::new(RefCell::new(Vec::new()));
        let output = captured.clone();
        let queue = field.bridge.events.clone();
        field._runner.borrow_mut().tick = Some(Box::new(move || {
            output.borrow_mut().extend(queue.borrow_mut().drain(..));
            Ok(Some(std::time::Duration::ZERO))
        }));
        for input_type in ["insertLineBreak", "insertParagraph"] {
            captured.borrow_mut().clear();
            let init = web_sys::InputEventInit::new();
            init.set_bubbles(true);
            init.set_cancelable(true);
            init.set_input_type(input_type);
            let event =
                web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
            field.bridge.element.dispatch_event(&event).unwrap();
            assert!(event.default_prevented());
            let events = captured.borrow();
            assert_eq!(events.len(), 2);
            assert!(matches!(
                events[0],
                Event::Keyboard(KeyEvent::Pressed {
                    keycode: scarlet_ui_core::event::KeyCode::Enter,
                    ..
                })
            ));
            assert!(matches!(
                events[1],
                Event::Keyboard(KeyEvent::Released {
                    keycode: scarlet_ui_core::event::KeyCode::Enter,
                    ..
                })
            ));
            assert_eq!(field.text.get(), "");
        }
        captured.borrow_mut().clear();
        field.send("compositionstart");
        let init = web_sys::InputEventInit::new();
        init.set_cancelable(true);
        init.set_input_type("insertLineBreak");
        init.set_is_composing(true);
        let event = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        field.bridge.element.dispatch_event(&event).unwrap();
        assert!(!event.default_prevented());
        assert!(captured.borrow().is_empty());
    }

    #[wasm_bindgen_test]
    fn textarea_uses_logical_lines_and_preserves_multiline_selection() {
        let field = Field::new_with_multiline("first long line\nsecond line", true);
        assert_eq!(field.bridge.element.wrap(), "off");
        assert_eq!(
            field
                .bridge
                .element
                .style()
                .get_property_value("white-space")
                .unwrap(),
            "pre"
        );
        field
            .bridge
            .element
            .set_selection_range_with_direction(5, 18, "forward")
            .unwrap();
        field.send("select");
        assert_eq!(field.text.get(), "first long line\nsecond line");
        let state = field.bridge.state.borrow().clone().unwrap();
        assert_eq!((state.cursor_byte, state.anchor_byte), (18, 5));
        assert_eq!(field.bridge.element.selection_start().unwrap(), Some(5));
        assert_eq!(field.bridge.element.selection_end().unwrap(), Some(18));
    }

    #[wasm_bindgen_test]
    fn unchanged_core_sync_preserves_native_selection_until_selection_event() {
        let field = Field::new_with_multiline("abc def", true);
        field.bridge.element.set_selection_range(7, 7).unwrap();
        field.send("select");
        let core_state = field.bridge.state.borrow().clone().unwrap();
        assert_eq!(core_state.cursor_byte, 7);
        // Native Home changes DOM selection before the asynchronous document
        // selectionchange task. A RAF/layout sync can run in between.
        field.bridge.element.set_selection_range(0, 0).unwrap();
        field.bridge.sync(Some(&core_state), 0.0);
        assert_eq!(field.bridge.element.selection_start().unwrap(), Some(0));
        assert_eq!(field.bridge.element.selection_end().unwrap(), Some(0));
        field.bridge.focus_editor(); // Same-core-caret trusted focus is explicit.
        assert_eq!(field.bridge.element.selection_start().unwrap(), Some(7));
        field.bridge.element.set_selection_range(0, 0).unwrap();
        field.bridge.sync(Some(&core_state), 0.0);
        field.send("select");
        assert_eq!(field.bridge.state.borrow().as_ref().unwrap().cursor_byte, 0);
        // Conversely, an actual external core change must remain authoritative.
        let mut changed_core = field.bridge.state.borrow().clone().unwrap();
        changed_core.cursor_byte = 4;
        changed_core.anchor_byte = 4;
        field.bridge.sync(Some(&changed_core), 0.0);
        assert_eq!(field.bridge.element.selection_start().unwrap(), Some(4));
        // keyup is a fallback for platforms with delayed selectionchange.
        field.bridge.element.set_selection_range(2, 2).unwrap();
        field.bridge.sync(Some(&changed_core), 0.0);
        field.send("keyup");
        assert_eq!(field.bridge.state.borrow().as_ref().unwrap().cursor_byte, 2);
        assert_eq!(field.bridge.element.selection_start().unwrap(), Some(2));
    }

    #[wasm_bindgen_test]
    fn dropping_bridge_removes_editable_element_and_listeners() {
        let field = Field::new("");
        let element = field.bridge.element.clone();
        drop(field);
        assert!(!element.is_connected());
        element
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
    }
}
