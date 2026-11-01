//! Single-canvas browser platform using SGFX's WebGPU execution backend.
#![cfg(target_arch = "wasm32")]

mod damage;
mod menu;
mod paint;
mod touch;
use scarlet_ui_core::{
    Application, ApplicationRunner, Buffer, Error, Result, View,
    event::{
        Event, FocusEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, ScrollSource,
        WheelPhase,
    },
    geometry::{Point, Size},
    platform::{
        PlatformBackend, PlatformWindow, PlatformWindowDefaults, WindowCreateRequest,
        WindowDecoration,
    },
    renderer::{PaintBackend, RendererBackendKind},
};
pub use scarlet_ui_renderer_sgfx::{
    SgfxCanvas, SgfxCanvasDraw, SgfxCanvasFrame, SgfxCanvasHandle, SgfxCanvasRenderObject,
    SgfxCanvasVertex, SgfxMesh, SgfxMeshHandle, SgfxTexture, SgfxTextureHandle,
};
use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};
use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{
    AddEventListenerOptions, HtmlCanvasElement, KeyboardEvent, PointerEvent, WheelEvent,
};

type Tick = Box<dyn FnMut() -> Result<Option<std::time::Duration>>>;

struct FrameLoop {
    tick: Option<Tick>,
    frame: Option<gloo_render::AnimationFrame>,
    error: Option<String>,
    paused: bool,
}

/// Owns the application, GPU resources, event listeners and scheduled frame.
/// Dropping this handle cancels the pending frame and releases those resources,
/// even when the browser has suspended animation frames in a background tab.
pub struct WebApp {
    state: Rc<RefCell<FrameLoop>>,
}
impl WebApp {
    /// Whether the application is still accepting animation frames.
    pub fn is_running(&self) -> bool {
        let state = self.state.borrow();
        !state.paused && state.tick.is_some()
    }
    /// Suspend animation frames while retaining application state and GPU resources.
    /// The embedding page should blur/inert the canvas while it is paused;
    /// event listeners remain installed until this handle is dropped.
    pub fn pause(&self) {
        let mut state = self.state.borrow_mut();
        state.paused = true;
        state.frame.take();
    }

    /// Resume a paused application. Repeated calls do not create extra RAF loops.
    /// A terminally stopped or failed application cannot be restarted this way.
    pub fn resume(&self) {
        let restart = {
            let mut state = self.state.borrow_mut();
            let restart = state.paused && state.tick.is_some();
            state.paused = false;
            restart
        };
        if restart {
            schedule_frame(&self.state);
        }
    }

    /// The terminal runtime error, if rendering or event processing failed.
    pub fn error(&self) -> Option<String> {
        self.state.borrow().error.clone()
    }
}

fn schedule_frame(state: &Rc<RefCell<FrameLoop>>) {
    if state.borrow().paused {
        return;
    }
    let weak = Rc::downgrade(state);
    let frame = gloo_render::request_animation_frame(move |_| {
        let Some(state) = weak.upgrade() else {
            return;
        };
        // Do not keep a RefCell borrow across application callbacks.
        let Some(mut tick) = state.borrow_mut().tick.take() else {
            return;
        };
        match tick() {
            Ok(Some(_)) => {
                state.borrow_mut().tick = Some(tick);
                schedule_frame(&state);
            }
            Ok(None) => {
                state.borrow_mut().frame.take();
            }
            Err(error) => {
                web_sys::console::error_1(&JsValue::from_str(&error.to_string()));
                let mut state = state.borrow_mut();
                state.error = Some(error.to_string());
                state.frame.take();
            }
        }
    });
    state.borrow_mut().frame = Some(frame);
}

/// Initialize WebGPU asynchronously, then run one application cycle per animation frame.
/// Supply an explicitly CSS-sized canvas and install an embedded font before calling.
/// Additional native windows, popups, IME and file dialogs are not supported yet.
pub async fn run<A: Application + View>(
    canvas: HtmlCanvasElement,
    app: A,
) -> std::result::Result<WebApp, JsValue> {
    let size = canvas_size(&canvas);
    let scale = scale_milli();
    update_backing(&canvas, size, scale);
    let gpu = sgfx_backend_wgpu::WindowContext::from_canvas(canvas.clone())
        .await
        .map_err(|e| JsValue::from_str(&format!("WebGPU initialization failed: {e}")))?;
    let menus = Rc::new(RefCell::new(menu::WebMenu::default()));
    menus.borrow_mut().resize(size);
    let paint = paint::WebPaint::new(gpu, canvas.width(), canvas.height(), menus.clone())
        .map_err(js_error)?;
    let window = WebWindow::new_canvas(canvas, size, scale, Box::new(paint), menus)?;
    let tick = ApplicationRunner::new(Box::new(WebBackend(Some(window))))
        .into_tick(app)
        .map_err(js_error)?;
    let state = Rc::new(RefCell::new(FrameLoop {
        tick: Some(Box::new(tick)),
        frame: None,
        error: None,
        paused: false,
    }));
    schedule_frame(&state);
    Ok(WebApp { state })
}

fn js_error(error: Error) -> JsValue {
    JsValue::from_str(&error.to_string())
}
fn scale_milli() -> u32 {
    (web_sys::window().unwrap().device_pixel_ratio() * 1000.0)
        .round()
        .max(1.0) as u32
}
fn canvas_size(canvas: &HtmlCanvasElement) -> Size {
    Size::new(
        canvas.client_width().max(1) as f32,
        canvas.client_height().max(1) as f32,
    )
}
fn update_backing(canvas: &HtmlCanvasElement, size: Size, scale: u32) {
    let w = (size.width * scale as f32 / 1000.0).ceil().max(1.0) as u32;
    let h = (size.height * scale as f32 / 1000.0).ceil().max(1.0) as u32;
    if canvas.width() != w {
        canvas.set_width(w);
    }
    if canvas.height() != h {
        canvas.set_height(h);
    }
}
struct WebBackend(Option<WebWindow>);
impl PlatformBackend for WebBackend {
    fn window_defaults(&mut self) -> PlatformWindowDefaults {
        PlatformWindowDefaults::new(false).with_decoration(WindowDecoration::NONE)
    }
    fn output_scale_milli(&mut self) -> u32 {
        scale_milli()
    }
    fn create_window(&mut self, _: WindowCreateRequest) -> Result<Box<dyn PlatformWindow>> {
        self.0
            .take()
            .map(|w| Box::new(w) as Box<dyn PlatformWindow>)
            .ok_or(Error::WindowCreationFailed)
    }
}
struct WebWindow {
    canvas: HtmlCanvasElement,
    size: Size,
    scale: u32,
    events: Rc<RefCell<VecDeque<Event>>>,
    paint: Option<Box<dyn PaintBackend>>,
    listeners: Vec<(&'static str, Closure<dyn FnMut(PointerEvent)>)>,
    input_listeners: Vec<(&'static str, Closure<dyn FnMut(web_sys::Event)>)>,
    lifecycle_listeners: Vec<(
        web_sys::EventTarget,
        &'static str,
        Closure<dyn FnMut(web_sys::Event)>,
    )>,
    original_tab_index: Option<String>,
    menus: Rc<RefCell<menu::WebMenu>>,
    hover_handoff: menu::HoverHandoff,
    menu_touch: touch::MenuTouchRouting,
}
impl WebWindow {
    fn new_canvas(
        canvas: HtmlCanvasElement,
        size: Size,
        scale: u32,
        paint: Box<dyn PaintBackend>,
        menus: Rc<RefCell<menu::WebMenu>>,
    ) -> std::result::Result<Self, JsValue> {
        let original_tab_index = canvas.get_attribute("tabindex");
        canvas.set_tab_index(0);
        let mut window = Self {
            canvas,
            size,
            scale,
            events: Rc::default(),
            paint: Some(paint),
            listeners: Vec::new(),
            input_listeners: Vec::new(),
            lifecycle_listeners: Vec::new(),
            original_tab_index,
            menus,
            hover_handoff: menu::HoverHandoff::default(),
            menu_touch: touch::MenuTouchRouting::default(),
        };
        let pressed = Rc::new(Cell::new(None));
        let touches = Rc::new(RefCell::new(touch::TouchContacts::default()));
        for kind in [
            "pointermove",
            "pointerdown",
            "pointerup",
            "pointercancel",
            "pointerleave",
            "lostpointercapture",
        ] {
            let canvas = window.canvas.clone();
            let events = window.events.clone();
            let pressed = pressed.clone();
            let touches = touches.clone();
            let callback = Closure::wrap(Box::new(move |e: PointerEvent| {
                if e.pointer_type() != "touch" && !e.is_primary() {
                    return;
                }
                let rect = canvas.get_bounding_client_rect();
                let x = ((e.client_x() as f64 - rect.left()) * canvas.client_width() as f64
                    / rect.width().max(1.0))
                .floor() as i32;
                let y = ((e.client_y() as f64 - rect.top()) * canvas.client_height() as f64
                    / rect.height().max(1.0))
                .floor() as i32;
                if e.pointer_type() == "touch" {
                    if kind == "pointerdown" {
                        let _ = canvas.focus();
                        let _ = canvas.set_pointer_capture(e.pointer_id());
                    }
                    let frame =
                        touches
                            .borrow_mut()
                            .pointer(kind, e.pointer_id(), x, y, e.time_stamp());
                    if let Some(frame) = frame {
                        events.borrow_mut().push_back(Event::TouchFrame(frame));
                    }
                    if matches!(kind, "pointerup" | "pointercancel") {
                        let _ = canvas.release_pointer_capture(e.pointer_id());
                    }
                    return;
                }
                let button = match e.button() {
                    1 => MouseButton::Middle,
                    2 => MouseButton::Right,
                    _ => MouseButton::Left,
                };
                let event = match kind {
                    "pointerdown" => {
                        let _ = canvas.focus();
                        pressed.set(Some(button));
                        let _ = canvas.set_pointer_capture(e.pointer_id());
                        MouseEvent::ButtonPressed {
                            button,
                            x,
                            y,
                            click_count: 1,
                        }
                    }
                    "pointerup" => {
                        pressed.set(None);
                        let _ = canvas.release_pointer_capture(e.pointer_id());
                        MouseEvent::ButtonReleased {
                            button,
                            x,
                            y,
                            click_count: 1,
                        }
                    }
                    "pointercancel" | "lostpointercapture" => {
                        let Some(button) = pressed.take() else {
                            return;
                        };
                        MouseEvent::ButtonCancelled { button, x, y }
                    }
                    "pointerleave" => MouseEvent::Exited { x, y },
                    _ => MouseEvent::Moved { x, y },
                };
                events.borrow_mut().push_back(Event::Mouse(event));
            }) as Box<dyn FnMut(PointerEvent)>);
            window
                .canvas
                .add_event_listener_with_callback(kind, callback.as_ref().unchecked_ref())?;
            window.listeners.push((kind, callback));
        }
        let events = window.events.clone();
        let blur_touches = touches.clone();
        window.listen("blur", move |event| {
            if let Some(frame) = blur_touches.borrow_mut().cancel_all(event.time_stamp()) {
                events.borrow_mut().push_back(Event::TouchFrame(frame));
            }
        })?;
        if let Some(browser) = web_sys::window() {
            for kind in ["blur", "pagehide"] {
                let events = window.events.clone();
                let contacts = touches.clone();
                window.listen_lifecycle(browser.clone().into(), kind, move |event| {
                    // Queue cancellation even while RAF is paused. On resume it
                    // precedes new contacts, so the arena cannot retain a drag.
                    if let Some(frame) = contacts.borrow_mut().cancel_all(event.time_stamp()) {
                        events.borrow_mut().push_back(Event::TouchFrame(frame));
                    }
                })?;
            }
            if let Some(document) = browser.document() {
                let events = window.events.clone();
                window.listen_lifecycle(
                    document.clone().into(),
                    "visibilitychange",
                    move |event| {
                        if document.hidden() {
                            if let Some(frame) = touches.borrow_mut().cancel_all(event.time_stamp())
                            {
                                events.borrow_mut().push_back(Event::TouchFrame(frame));
                            }
                        }
                    },
                )?;
            }
        }
        window.install_keyboard_and_wheel()?;
        let menus = window.menus.clone();
        window.listen("contextmenu", move |event| {
            if menus.borrow().has_menu() {
                event.prevent_default();
            }
        })?;
        Ok(window)
    }

    fn listen_lifecycle(
        &mut self,
        target: web_sys::EventTarget,
        kind: &'static str,
        callback: impl FnMut(web_sys::Event) + 'static,
    ) -> std::result::Result<(), JsValue> {
        let callback = Closure::wrap(Box::new(callback) as Box<dyn FnMut(web_sys::Event)>);
        target.add_event_listener_with_callback(kind, callback.as_ref().unchecked_ref())?;
        self.lifecycle_listeners.push((target, kind, callback));
        Ok(())
    }

    fn listen(
        &mut self,
        kind: &'static str,
        callback: impl FnMut(web_sys::Event) + 'static,
    ) -> std::result::Result<(), JsValue> {
        let callback = Closure::wrap(Box::new(callback) as Box<dyn FnMut(web_sys::Event)>);
        let options = AddEventListenerOptions::new();
        // Wheel listeners must be non-passive for canvas-local scroll handling.
        options.set_passive(false);
        self.canvas
            .add_event_listener_with_callback_and_add_event_listener_options(
                kind,
                callback.as_ref().unchecked_ref(),
                &options,
            )?;
        self.input_listeners.push((kind, callback));
        Ok(())
    }

    fn install_keyboard_and_wheel(&mut self) -> std::result::Result<(), JsValue> {
        let held_keys: Rc<RefCell<Vec<(String, KeyCode)>>> = Rc::default();
        for kind in ["keydown", "keyup"] {
            let events = self.events.clone();
            let held_keys = held_keys.clone();
            self.listen(kind, move |event| {
                let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
                    return;
                };
                // Canvas has no editable DOM/IME context. Never synthesize text
                // from an in-progress composition or browser shortcut.
                if key.is_composing() || key.key() == "Dead" || key.key() == "Process" {
                    return;
                }
                let keycode = map_key(&key.key());
                let modifiers = KeyModifiers {
                    shift: key.shift_key(),
                    control: key.ctrl_key(),
                    alt: key.alt_key(),
                    super_key: key.meta_key(),
                };
                let alt_graph = key.get_modifier_state("AltGraph");
                let text_allowed =
                    alt_graph || (!modifiers.control && !modifiers.super_key && !modifiers.alt);
                let down = kind == "keydown";
                let mut events = events.borrow_mut();
                let mut held = held_keys.borrow_mut();
                if down {
                    if !held.iter().any(|(code, _)| *code == key.code()) {
                        held.push((key.code(), keycode));
                    }
                    events.push_back(Event::Keyboard(KeyEvent::Pressed { keycode, modifiers }));
                    if text_allowed && matches!(keycode, KeyCode::Char(_) | KeyCode::Space) {
                        for c in key.key().chars().filter(|c| !c.is_control()) {
                            events.push_back(Event::Keyboard(KeyEvent::Char { c }));
                        }
                    }
                } else {
                    // Use the pressed logical value if Shift changed while held.
                    let released = held
                        .iter()
                        .position(|(code, _)| *code == key.code())
                        .map(|index| held.remove(index).1)
                        .unwrap_or(keycode);
                    events.push_back(Event::Keyboard(KeyEvent::Released {
                        keycode: released,
                        modifiers,
                    }));
                }
                // Keep browser shortcuts (reload, address bar, find, devtools,
                // zoom, etc.) intact. Ordinary canvas navigation belongs to UI.
                let navigation = matches!(
                    keycode,
                    KeyCode::Tab
                        | KeyCode::Enter
                        | KeyCode::Space
                        | KeyCode::Backspace
                        | KeyCode::Delete
                        | KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Home
                        | KeyCode::End
                        | KeyCode::PageUp
                        | KeyCode::PageDown
                );
                let select_all = modifiers.control
                    && !modifiers.alt
                    && !modifiers.super_key
                    && matches!(keycode, KeyCode::Char('a' | 'A'));
                if (text_allowed && (navigation || matches!(keycode, KeyCode::Char(_))))
                    || select_all
                {
                    key.prevent_default();
                }
            })?;
        }
        for kind in ["focus", "blur"] {
            let events = self.events.clone();
            let held_keys = held_keys.clone();
            self.listen(kind, move |_| {
                let mut events = events.borrow_mut();
                if kind == "blur" {
                    for (_, keycode) in held_keys.borrow_mut().drain(..) {
                        events.push_back(Event::Keyboard(KeyEvent::Released {
                            keycode,
                            modifiers: KeyModifiers::empty(),
                        }));
                    }
                }
                events.push_back(Event::Focus(if kind == "focus" {
                    FocusEvent::Gained
                } else {
                    FocusEvent::Lost
                }));
            })?;
        }
        let events = self.events.clone();
        let canvas = self.canvas.clone();
        let mut remainder = [0.0_f64; 2];
        self.listen("wheel", move |event| {
            let Some(wheel) = event.dyn_ref::<WheelEvent>() else {
                return;
            };
            // Ctrl-wheel includes trackpad pinch-to-zoom; let the browser own it.
            if wheel.ctrl_key() || wheel.meta_key() {
                return;
            }
            let mode = wheel.delta_mode();
            let unit = match mode {
                WheelEvent::DOM_DELTA_LINE => [32.0, 32.0],
                WheelEvent::DOM_DELTA_PAGE => [
                    canvas.client_width().max(1) as f64,
                    canvas.client_height().max(1) as f64,
                ],
                // DOM pixel deltas already use CSS pixels: do NOT divide by DPR.
                _ => [1.0, 1.0],
            };
            // DOM deltas describe scroll-offset motion (positive = down/right).
            // ScarletUI, like winit, describes content motion: scrolling down
            // moves the content up, so negate both axes exactly once.
            let mut delta = [-wheel.delta_x() * unit[0], -wheel.delta_y() * unit[1]];
            if wheel.shift_key() && delta[0] == 0.0 {
                delta.swap(0, 1);
            }
            let mut logical = [0; 2];
            for axis in 0..2 {
                let total = delta[axis] + remainder[axis];
                if total.is_finite() {
                    logical[axis] = total.round() as i32;
                    remainder[axis] = (total - f64::from(logical[axis])).clamp(-0.5, 0.5);
                } else {
                    remainder[axis] = 0.0;
                }
            }
            wheel.prevent_default();
            if logical == [0, 0] {
                return;
            }
            let rect = canvas.get_bounding_client_rect();
            let x = ((wheel.client_x() as f64 - rect.left()) * canvas.client_width() as f64
                / rect.width().max(1.0))
            .floor() as i32;
            let y = ((wheel.client_y() as f64 - rect.top()) * canvas.client_height() as f64
                / rect.height().max(1.0))
            .floor() as i32;
            events
                .borrow_mut()
                .push_back(Event::Mouse(MouseEvent::Wheel {
                    delta_x: logical[0],
                    delta_y: logical[1],
                    x,
                    y,
                    phase: WheelPhase::Moved,
                    // Browser wheel events do not reliably identify a trackpad or
                    // expose gesture boundaries; do not invent captured gestures.
                    source: if mode == WheelEvent::DOM_DELTA_PIXEL {
                        ScrollSource::Unknown
                    } else {
                        ScrollSource::Wheel
                    },
                }));
        })?;
        Ok(())
    }
}

fn map_key(key: &str) -> KeyCode {
    match key {
        "Escape" => KeyCode::Escape,
        "Enter" => KeyCode::Enter,
        "Tab" => KeyCode::Tab,
        "Backspace" => KeyCode::Backspace,
        " " => KeyCode::Space,
        "ArrowLeft" => KeyCode::Left,
        "ArrowRight" => KeyCode::Right,
        "ArrowUp" => KeyCode::Up,
        "ArrowDown" => KeyCode::Down,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Insert" => KeyCode::Insert,
        "Delete" => KeyCode::Delete,
        _ => {
            if let Some(number) = key
                .strip_prefix('F')
                .and_then(|n| n.parse::<u8>().ok())
                .filter(|n| (1..=24).contains(n))
            {
                KeyCode::F(number)
            } else {
                let mut chars = key.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if !c.is_control() => KeyCode::Char(c),
                    _ => KeyCode::Unknown,
                }
            }
        }
    }
}

impl Drop for WebWindow {
    fn drop(&mut self) {
        for (target, kind, listener) in &self.lifecycle_listeners {
            let _ =
                target.remove_event_listener_with_callback(kind, listener.as_ref().unchecked_ref());
        }
        for (kind, listener) in &self.input_listeners {
            let _ = self
                .canvas
                .remove_event_listener_with_callback(kind, listener.as_ref().unchecked_ref());
        }
        if let Some(value) = &self.original_tab_index {
            let _ = self.canvas.set_attribute("tabindex", value);
        } else {
            let _ = self.canvas.remove_attribute("tabindex");
        }
        for (kind, listener) in &self.listeners {
            let _ = self
                .canvas
                .remove_event_listener_with_callback(kind, listener.as_ref().unchecked_ref());
        }
    }
}
impl PlatformWindow for WebWindow {
    fn new(_: &str, _: &str, _: Size) -> Result<Self> {
        Err(Error::WindowCreationFailed)
    }
    fn poll_event(&mut self) -> Option<Event> {
        let canvas_size = canvas_size(&self.canvas);
        let scale = scale_milli();
        let changed = canvas_size != self.size || scale != self.scale;
        if changed {
            self.size = canvas_size;
            self.scale = scale;
            update_backing(&self.canvas, canvas_size, scale);
            self.menus.borrow_mut().resize(canvas_size);
        }
        let dirty = self.menus.borrow_mut().take_dirty();
        if changed || dirty {
            let size = self.size();
            return Some(Event::Resize {
                width: size.width as u32,
                height: size.height as u32,
            });
        }
        loop {
            let mut event = self.events.borrow_mut().pop_front()?;
            let (consumed, action) = if let Event::TouchFrame(frame) = &mut event {
                let mut menus = self.menus.borrow_mut();
                let action = self.menu_touch.route(frame, |event| menus.handle(event));
                (frame.changes.is_empty(), action)
            } else {
                self.menus.borrow_mut().handle(&event)
            };
            if let Event::Mouse(mouse) = &event {
                let offset = self.menus.borrow().height() as i32;
                if let Some(exit) = self.hover_handoff.route(mouse, consumed, offset) {
                    // Keep the menu's dirty flag intact: the next poll schedules
                    // its full repaint, after the scene has cleared old hover.
                    return Some(Event::Mouse(exit));
                }
            }
            if let Some(menu_item_id) = action {
                return Some(Event::MenuItemActivated {
                    window_id: self.surface_id(),
                    menu_item_id,
                });
            }
            if consumed {
                if self.menus.borrow_mut().take_dirty() {
                    let size = self.size();
                    return Some(Event::Resize {
                        width: size.width as u32,
                        height: size.height as u32,
                    });
                }
                continue;
            }
            let offset = self.menus.borrow().height() as i32;
            if let Event::Mouse(ref mut mouse) = event {
                match mouse {
                    MouseEvent::Moved { y, .. }
                    | MouseEvent::Entered { y, .. }
                    | MouseEvent::Exited { y, .. }
                    | MouseEvent::ButtonPressed { y, .. }
                    | MouseEvent::ButtonReleased { y, .. }
                    | MouseEvent::ButtonCancelled { y, .. }
                    | MouseEvent::Wheel { y, .. } => *y -= offset,
                    _ => {}
                }
            }
            if let Event::TouchFrame(ref mut frame) = event {
                for change in &mut frame.changes {
                    change.y -= offset;
                }
            }
            return Some(event);
        }
    }
    fn wait_for_event(&mut self, _: std::time::Duration) {}
    fn output_scale_milli(&self) -> u32 {
        self.scale
    }
    fn renderer_backend(&self) -> RendererBackendKind {
        RendererBackendKind::Sgfx
    }
    fn take_paint_backend(&mut self) -> Result<Option<Box<dyn PaintBackend>>> {
        Ok(self.paint.take())
    }
    fn present(&mut self, _: &Buffer) {
        unreachable!("Web platform requires SGFX paint backend")
    }
    fn set_title(&mut self, title: &str) {
        if let Some(doc) = self.canvas.owner_document() {
            doc.set_title(title);
        }
    }
    fn size(&self) -> Size {
        self.menus.borrow().content_size(self.size)
    }
    fn resize(&mut self, _: u32, _: u32) -> Result<()> {
        Ok(())
    }
    fn close(&mut self) -> Result<()> {
        self.events.borrow_mut().push_back(Event::Quit);
        Ok(())
    }
    fn minimize(&mut self) -> Result<()> {
        Err(Error::EventDispatchError)
    }
    fn maximize(&mut self) -> Result<()> {
        Err(Error::EventDispatchError)
    }
    fn set_fullscreen(&mut self, _: bool) -> Result<()> {
        Err(Error::EventDispatchError)
    }
    fn restore(&mut self) -> Result<()> {
        Err(Error::EventDispatchError)
    }
    fn request_move(&mut self) -> Result<()> {
        Err(Error::EventDispatchError)
    }
    fn create_popup(&mut self, _: Point, _: Size) -> Result<u32> {
        Err(Error::WindowCreationFailed)
    }
    fn destroy_popup(&mut self, _: u32) -> Result<()> {
        Err(Error::EventDispatchError)
    }
    fn set_workarea(&mut self, _: i32, _: i32, _: u32, _: u32) -> Result<()> {
        Err(Error::EventDispatchError)
    }
    fn create_window_with_type(&mut self, _: &str, _: &str, _: Size, _: u32) -> Result<Self> {
        Err(Error::WindowCreationFailed)
    }
    fn move_window(&mut self, _: i32, _: i32) -> Result<()> {
        Err(Error::EventDispatchError)
    }
    fn set_window_type(&mut self, _: u32, _: u32) -> Result<()> {
        Err(Error::EventDispatchError)
    }
    fn get_screen_size(&mut self) -> Result<(u32, u32)> {
        Ok((self.size.width as u32, self.size.height as u32))
    }
    fn surface_id(&self) -> u32 {
        1
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn set_resizable(&mut self, _: bool) -> Result<()> {
        Ok(())
    }
    fn set_opaque(&mut self, _: bool) -> Result<()> {
        Ok(())
    }
    fn set_menu_model(&mut self, model: &scarlet_ui_core::menu_model::MenuBarModel) -> Result<()> {
        self.menus.borrow_mut().set_model(model);
        Ok(())
    }
    fn wants_menu_model_updates(&self) -> bool {
        true
    }
    fn set_menu_titles(&mut self, titles: &str) -> Result<()> {
        if titles.is_empty() {
            Ok(())
        } else {
            Err(Error::EventDispatchError)
        }
    }
}
