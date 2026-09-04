#![cfg(not(target_os = "scarlet"))]

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::RefCell;
use core::num::NonZeroU32;
use core::sync::atomic::{AtomicU32, Ordering};
use scarlet_ui_core::buffer::Buffer;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::element::TextInputElementState;
use scarlet_ui_core::error::{Error, Result};
use scarlet_ui_core::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, ScrollSource, WheelPhase,
};
use scarlet_ui_core::geometry::{Point, Size};
use scarlet_ui_core::input_environment::{
    InputEnvironment, WindowingMode, current_input_environment,
};
use scarlet_ui_core::platform::{
    PlatformBackend, PlatformWindow, WindowCreateRequest, WindowDecoration,
};
#[cfg(feature = "sgfx")]
use scarlet_ui_core::renderer::PaintBackend;
pub use scarlet_ui_renderer_sgfx::{
    SgfxCanvas, SgfxCanvasDraw, SgfxCanvasFrame, SgfxCanvasHandle, SgfxCanvasRenderObject,
    SgfxCanvasVertex, SgfxMesh, SgfxMeshHandle, SgfxTexture,
};
#[cfg(feature = "sgfx")]
use sgfx_renderer::SgfxWindowPaintBackend;
use std::time::{Duration, Instant};

use ::winit::application::ApplicationHandler;
use ::winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, Position};
use ::winit::event::{
    DeviceEvent, ElementState as WinitElementState, Ime, MouseButton as WinitMouseButton,
    MouseScrollDelta, TouchPhase, WindowEvent,
};
use ::winit::event_loop::{ActiveEventLoop, EventLoop};
use ::winit::keyboard::{Key, ModifiersState, NamedKey};
#[cfg(target_os = "macos")]
use ::winit::platform::macos::WindowAttributesExtMacOS;
use ::winit::platform::pump_events::EventLoopExtPumpEvents;
use ::winit::window::{
    CursorGrabMode, Fullscreen, Window as WinitWindow, WindowAttributes, WindowId,
};

#[cfg(feature = "sgfx")]
mod sgfx_renderer;
mod tablet_mode;

use tablet_mode::parse_tablet_mode_override;

type SoftbufferContext = softbuffer::Context<::winit::event_loop::OwnedDisplayHandle>;
type SoftbufferSurface =
    softbuffer::Surface<::winit::event_loop::OwnedDisplayHandle, Rc<WinitWindow>>;

const DOUBLE_CLICK_THRESHOLD: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_DISTANCE: i32 = 5;
const TRACKPAD_END_GRACE: Duration = Duration::from_millis(120);
const TRACKPAD_MOVED_MIN_INTERVAL: Duration = Duration::from_millis(16);

pub struct WinitBackend {
    shared: Rc<WinitSharedState>,
}

fn startup_input_environment() -> InputEnvironment {
    match parse_tablet_mode_override(std::env::var("SCARLET_TABLET_MODE").ok().as_deref()) {
        Some(true) => InputEnvironment::new(1, Some(true), None, true, false, false, false)
            .with_system_mode(Some(WindowingMode::Focused), Some(true), Some(false)),
        Some(false) => InputEnvironment::new(1, Some(false), None, false, true, true, false)
            .with_system_mode(Some(WindowingMode::Freeform), Some(true), Some(false)),
        None => InputEnvironment::desktop(),
    }
}

impl WinitBackend {
    /// Create a winit backend.
    ///
    /// # Returns
    ///
    /// A backend that creates native desktop windows.
    pub fn new() -> Self {
        let wheel_log_enabled = wheel_log_env_enabled();
        scarlet_ui_core::debug::set_wheel_log_enabled(wheel_log_enabled);
        if wheel_log_enabled {
            println!("[Wheel] logging enabled in scarlet-ui-platform-winit");
        }
        Self {
            shared: Rc::new(WinitSharedState::new()),
        }
    }
}

fn wheel_log_env_enabled() -> bool {
    std::env::var("SCARLET_UI_WHEEL_LOG")
        .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

fn winit_wheel_coalesce_env_enabled() -> bool {
    std::env::var("SCARLET_UI_WINIT_WHEEL_COALESCE").is_ok_and(|value| env_flag_enabled(&value))
}

fn env_flag_enabled(value: &str) -> bool {
    matches!(value, "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
}

#[cfg(feature = "sgfx")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WinitRendererPreference {
    Auto,
    Cpu,
    Sgfx,
}

#[cfg(feature = "sgfx")]
fn winit_renderer_preference() -> WinitRendererPreference {
    match std::env::var("SCARLET_UI_WINIT_RENDERER").ok().as_deref() {
        Some("cpu") | Some("CPU") => WinitRendererPreference::Cpu,
        Some("sgfx") | Some("SGFX") | Some("wgpu") | Some("WGPU") => WinitRendererPreference::Sgfx,
        _ => WinitRendererPreference::Auto,
    }
}

#[cfg(feature = "sgfx")]
fn sgfx_backend_override_requested() -> bool {
    !matches!(
        sgfx::BackendPreference::from_environment(),
        Ok(sgfx::BackendPreference::Auto)
    )
}

#[cfg(feature = "sgfx")]
fn create_sgfx_backend(
    window: Rc<WinitWindow>,
    width: u32,
    height: u32,
    transparent: bool,
) -> Result<SgfxWindowPaintBackend> {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

    let raw_window_handle = window
        .window_handle()
        .map_err(|_| Error::SurfaceCreationFailed)?
        .as_raw();
    let raw_display_handle = window
        .display_handle()
        .map_err(|_| Error::SurfaceCreationFailed)?
        .as_raw();
    let instance = sgfx::Instance::new().map_err(|_| Error::RenderError)?;
    let backend = instance.backend();
    // SAFETY: `WinitPlatformWindow` declares its SGFX backend before `window`,
    // and the application drops the paint backend before the platform window.
    let window_context = unsafe {
        instance.create_window_context(raw_display_handle, raw_window_handle, width, height)
    }
    .map_err(|_| Error::SurfaceCreationFailed)?;
    println!("[ScarletUI] platform-winit renderer=sgfx backend={backend}");
    SgfxWindowPaintBackend::new(window_context, window, width, height, transparent)
}

#[cfg(all(feature = "sgfx", target_os = "macos"))]
fn configure_sgfx_surface_alpha(window: &WinitWindow, transparent: bool) {
    use objc2_app_kit::NSView;
    use objc2_quartz_core::CALayer;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    fn set_layer_tree_opaque(layer: &CALayer, opaque: bool) {
        layer.setOpaque(opaque);
        let Some(sublayers) = (unsafe { layer.sublayers() }) else {
            return;
        };
        for index in 0..sublayers.count() {
            // SAFETY: `index` is bounded by the retained NSArray's count.
            let child = unsafe { sublayers.objectAtIndex(index) };
            set_layer_tree_opaque(&child, opaque);
        }
    }

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    let view_ptr = handle.ns_view.as_ptr().cast::<NSView>();
    // SAFETY: Winit owns this NSView for at least as long as `window`, and the
    // caller invokes this only while the window is alive on the UI thread.
    let view = unsafe { &*view_ptr };
    if let Some(root_layer) = unsafe { view.layer() } {
        set_layer_tree_opaque(&root_layer, !transparent);
    }
}

#[cfg(all(feature = "sgfx", not(target_os = "macos")))]
fn configure_sgfx_surface_alpha(_window: &WinitWindow, _transparent: bool) {}

impl PlatformBackend for WinitBackend {
    fn initial_input_environment(&mut self) -> InputEnvironment {
        startup_input_environment()
    }

    fn output_scale_milli(&mut self) -> u32 {
        1000
    }

    fn create_window(&mut self, request: WindowCreateRequest) -> Result<Box<dyn PlatformWindow>> {
        Ok(Box::new(WinitPlatformWindow::create(
            self.shared.clone(),
            request,
        )?))
    }
}

impl Default for WinitBackend {
    fn default() -> Self {
        Self::new()
    }
}

struct WinitEventState {
    scale_factor: f64,
    cursor_physical_x: f64,
    cursor_physical_y: f64,
    cursor_x: i32,
    cursor_y: i32,
    window_focused: bool,
    fullscreen: bool,
    pointer_locked: bool,
    manual_move_active: bool,
    manual_move_origin_outer_x: i32,
    manual_move_origin_outer_y: i32,
    manual_move_origin_global_x: f64,
    manual_move_origin_global_y: f64,
    ime_preedit_active: bool,
    text_input_context_id: u32,
    text_input_serial: u32,
    pending_empty_preedit: Option<(u32, u32)>,
    modifiers: KeyModifiers,
    click_state: ClickState,
    pending_trackpad_end: Option<PendingTrackpadEnd>,
    pending_trackpad_moved: Option<PendingTrackpadMoved>,
    last_trackpad_moved_emit_at: Option<Instant>,
    wheel_coalesce_enabled: bool,
    direct_touch_advertised: bool,
    direct_touch_activation: DirectTouchActivationState,
    queue: VecDeque<Event>,
}

/// Tracks the one touch contact represented by the mouse compatibility stream.
///
/// ScarletUI's core event API currently exposes pointer activation through
/// `MouseEvent`. Keep the first active contact as the primary contact until it
/// ends so extra fingers cannot produce a second press or release.
#[derive(Default)]
struct DirectTouchActivationState {
    primary_touch_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectTouchActivation {
    Pressed,
    Moved,
    Released,
    Cancelled,
}

impl DirectTouchActivationState {
    fn map(&mut self, touch_id: u64, phase: TouchPhase) -> Option<DirectTouchActivation> {
        match phase {
            TouchPhase::Started if self.primary_touch_id.is_none() => {
                self.primary_touch_id = Some(touch_id);
                Some(DirectTouchActivation::Pressed)
            }
            TouchPhase::Moved if self.primary_touch_id == Some(touch_id) => {
                Some(DirectTouchActivation::Moved)
            }
            TouchPhase::Ended if self.primary_touch_id == Some(touch_id) => {
                self.primary_touch_id = None;
                Some(DirectTouchActivation::Released)
            }
            TouchPhase::Cancelled if self.primary_touch_id == Some(touch_id) => {
                self.primary_touch_id = None;
                Some(DirectTouchActivation::Cancelled)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct PendingTrackpadEnd {
    event: Event,
    queued_at: Instant,
}

#[derive(Clone, Debug)]
struct PendingTrackpadMoved {
    event: Event,
}

#[derive(Clone, Copy, Debug)]
struct ClickState {
    last_button: Option<MouseButton>,
    last_x: i32,
    last_y: i32,
    last_time: Option<Instant>,
    last_count: u8,
    active_button: Option<MouseButton>,
    active_count: u8,
}

impl Default for ClickState {
    fn default() -> Self {
        Self {
            last_button: None,
            last_x: 0,
            last_y: 0,
            last_time: None,
            last_count: 0,
            active_button: None,
            active_count: 1,
        }
    }
}

impl ClickState {
    fn press_count(&mut self, button: MouseButton, x: i32, y: i32) -> u8 {
        let now = Instant::now();
        let same_button = self.last_button == Some(button);
        let close_enough = (self.last_x - x).abs() <= DOUBLE_CLICK_DISTANCE
            && (self.last_y - y).abs() <= DOUBLE_CLICK_DISTANCE;
        let soon_enough = self
            .last_time
            .is_some_and(|last_time| now.duration_since(last_time) <= DOUBLE_CLICK_THRESHOLD);
        let count = if same_button && close_enough && soon_enough {
            self.last_count.saturating_add(1).max(1)
        } else {
            1
        };
        self.last_button = Some(button);
        self.last_x = x;
        self.last_y = y;
        self.last_time = Some(now);
        self.last_count = count;
        self.active_button = Some(button);
        self.active_count = count;
        count
    }

    fn release_count(&mut self, button: MouseButton) -> u8 {
        let count = if self.active_button == Some(button) {
            self.active_count
        } else {
            1
        };
        self.active_button = None;
        self.active_count = 1;
        count
    }

    fn cancel(&mut self, button: MouseButton) {
        if self.active_button == Some(button) {
            self.active_button = None;
            self.active_count = 1;
        }
        if self.last_button == Some(button) {
            self.last_button = None;
            self.last_time = None;
            self.last_count = 0;
        }
    }
}

impl WinitEventState {
    fn new(scale_factor: f64) -> Self {
        Self::new_with_wheel_coalesce(scale_factor, winit_wheel_coalesce_env_enabled())
    }

    fn new_with_wheel_coalesce(scale_factor: f64, wheel_coalesce_enabled: bool) -> Self {
        Self {
            scale_factor,
            cursor_physical_x: 0.0,
            cursor_physical_y: 0.0,
            cursor_x: 0,
            cursor_y: 0,
            window_focused: true,
            fullscreen: false,
            pointer_locked: false,
            manual_move_active: false,
            manual_move_origin_outer_x: 0,
            manual_move_origin_outer_y: 0,
            manual_move_origin_global_x: 0.0,
            manual_move_origin_global_y: 0.0,
            ime_preedit_active: false,
            text_input_context_id: 1,
            text_input_serial: 1,
            pending_empty_preedit: None,
            modifiers: KeyModifiers::empty(),
            click_state: ClickState::default(),
            pending_trackpad_end: None,
            pending_trackpad_moved: None,
            last_trackpad_moved_emit_at: None,
            wheel_coalesce_enabled,
            direct_touch_advertised: false,
            direct_touch_activation: DirectTouchActivationState::default(),
            queue: VecDeque::new(),
        }
    }

    fn update_cursor_from_physical(&mut self, position: PhysicalPosition<f64>) -> (i32, i32) {
        self.cursor_physical_x = position.x;
        self.cursor_physical_y = position.y;
        self.cursor_x = physical_to_logical_pos(position.x, self.scale_factor);
        self.cursor_y = physical_to_logical_pos(position.y, self.scale_factor);
        (self.cursor_x, self.cursor_y)
    }

    fn map_direct_touch_events(
        &mut self,
        touch_id: u64,
        phase: TouchPhase,
        location: PhysicalPosition<f64>,
    ) -> Vec<Event> {
        let Some(activation) = self.direct_touch_activation.map(touch_id, phase) else {
            return Vec::new();
        };
        let (x, y) = self.update_cursor_from_physical(location);
        let event = match activation {
            DirectTouchActivation::Pressed => {
                let click_count = self.click_state.press_count(MouseButton::Left, x, y);
                MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x,
                    y,
                    click_count,
                }
            }
            DirectTouchActivation::Moved => MouseEvent::Moved { x, y },
            DirectTouchActivation::Released => {
                let click_count = self.click_state.release_count(MouseButton::Left);
                MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x,
                    y,
                    click_count,
                }
            }
            DirectTouchActivation::Cancelled => {
                self.click_state.cancel(MouseButton::Left);
                MouseEvent::ButtonCancelled {
                    button: MouseButton::Left,
                    x,
                    y,
                }
            }
        };
        let mut events = Vec::with_capacity(2);
        events.push(Event::Mouse(event));
        if matches!(
            activation,
            DirectTouchActivation::Released | DirectTouchActivation::Cancelled
        ) {
            events.push(Event::Mouse(MouseEvent::Exited { x, y }));
        }
        events
    }

    fn push_direct_touch_events(
        &mut self,
        touch_id: u64,
        phase: TouchPhase,
        location: PhysicalPosition<f64>,
    ) {
        for event in self.map_direct_touch_events(touch_id, phase, location) {
            self.push(event);
        }
    }

    fn push(&mut self, mut event: Event) {
        self.flush_expired_trackpad_end();

        if Self::is_trackpad_end(&event) {
            self.flush_pending_trackpad_moved(true);
            self.pending_trackpad_end = Some(PendingTrackpadEnd {
                event,
                queued_at: Instant::now(),
            });
            return;
        }

        if Self::is_trackpad_wheel(&event)
            && let Some(pending) = self.pending_trackpad_end.take()
        {
            if pending.queued_at.elapsed() <= TRACKPAD_END_GRACE {
                if let Event::Mouse(MouseEvent::Wheel {
                    phase: phase @ WheelPhase::Started,
                    ..
                }) = &mut event
                {
                    *phase = WheelPhase::Moved;
                }
                if scarlet_ui_core::debug::wheel_log_enabled() {
                    println!("[Wheel] join deferred trackpad end into continuing gesture");
                }
            } else {
                self.queue.push_back(pending.event);
            }
        }

        if self.coalesce_trackpad_moved(&event) {
            self.flush_pending_trackpad_moved(false);
            return;
        }

        self.flush_pending_trackpad_moved(true);
        self.queue.push_back(event);
    }

    fn pop(&mut self) -> Option<Event> {
        self.flush_expired_trackpad_end();
        self.flush_pending_trackpad_moved(false);
        self.queue.pop_front()
    }

    fn flush_expired_trackpad_end(&mut self) {
        let Some(pending) = self.pending_trackpad_end.as_ref() else {
            return;
        };
        if pending.queued_at.elapsed() >= TRACKPAD_END_GRACE
            && let Some(pending) = self.pending_trackpad_end.take()
        {
            self.queue.push_back(pending.event);
        }
    }

    fn is_trackpad_wheel(event: &Event) -> bool {
        matches!(
            event,
            Event::Mouse(MouseEvent::Wheel {
                source: ScrollSource::Trackpad,
                ..
            })
        )
    }

    fn is_trackpad_end(event: &Event) -> bool {
        matches!(
            event,
            Event::Mouse(MouseEvent::Wheel {
                source: ScrollSource::Trackpad,
                phase: WheelPhase::Ended | WheelPhase::Cancelled,
                ..
            })
        )
    }

    fn coalesce_trackpad_moved(&mut self, event: &Event) -> bool {
        if !self.wheel_coalesce_enabled {
            return false;
        }

        let Event::Mouse(MouseEvent::Wheel {
            delta_x,
            delta_y,
            x,
            y,
            phase: WheelPhase::Moved,
            source: ScrollSource::Trackpad,
        }) = &event
        else {
            return false;
        };

        let pending = if let Some(pending) = self.pending_trackpad_moved.as_mut() {
            pending
        } else {
            self.pending_trackpad_moved = Some(PendingTrackpadMoved {
                event: Event::Mouse(MouseEvent::Wheel {
                    delta_x: *delta_x,
                    delta_y: *delta_y,
                    x: *x,
                    y: *y,
                    phase: WheelPhase::Moved,
                    source: ScrollSource::Trackpad,
                }),
            });
            return true;
        };
        let Event::Mouse(MouseEvent::Wheel {
            delta_x: pending_delta_x,
            delta_y: pending_delta_y,
            x: pending_x,
            y: pending_y,
            phase: WheelPhase::Moved,
            source: ScrollSource::Trackpad,
        }) = &mut pending.event
        else {
            return false;
        };

        *pending_delta_x = pending_delta_x.saturating_add(*delta_x);
        *pending_delta_y = pending_delta_y.saturating_add(*delta_y);
        *pending_x = *x;
        *pending_y = *y;
        if scarlet_ui_core::debug::wheel_log_enabled() {
            println!(
                "[Wheel] coalesced trackpad moved delta=({}, {}) cursor=({}, {})",
                *pending_delta_x, *pending_delta_y, *x, *y
            );
        }
        true
    }

    fn flush_pending_trackpad_moved(&mut self, force: bool) {
        let should_flush = force
            || self
                .last_trackpad_moved_emit_at
                .is_none_or(|last| last.elapsed() >= TRACKPAD_MOVED_MIN_INTERVAL);
        if !should_flush {
            return;
        }
        if let Some(pending) = self.pending_trackpad_moved.take() {
            self.queue.push_back(pending.event);
            self.last_trackpad_moved_emit_at = Some(Instant::now());
        }
    }

    fn next_text_input_serial(&mut self) -> u32 {
        let serial = self.text_input_serial;
        self.text_input_serial = self.text_input_serial.saturating_add(1);
        serial
    }

    fn sync_fullscreen_state(&mut self, window: &WinitWindow) -> bool {
        let fullscreen = window.fullscreen().is_some();
        if fullscreen != self.fullscreen {
            self.fullscreen = fullscreen;
            self.push(Event::FullscreenChanged { fullscreen });
            true
        } else {
            false
        }
    }

    fn defer_empty_preedit(&mut self) {
        let context_id = self.text_input_context_id;
        let serial = self.next_text_input_serial();
        self.pending_empty_preedit = Some((context_id, serial));
    }

    fn discard_pending_empty_preedit(&mut self) {
        self.pending_empty_preedit = None;
    }

    fn flush_pending_empty_preedit(&mut self) {
        let Some((context_id, serial)) = self.pending_empty_preedit.take() else {
            return;
        };
        self.push(Event::TextInputPreedit {
            context_id,
            serial,
            cursor_byte: 0,
            anchor_byte: 0,
            text: alloc::string::String::new(),
            spans: Vec::new(),
        });
    }
}

struct WinitWindowEntry {
    state: Rc<RefCell<WinitEventState>>,
    window: Rc<WinitWindow>,
}

struct WinitSharedState {
    event_loop: RefCell<EventLoop<()>>,
    windows: RefCell<BTreeMap<WindowId, WinitWindowEntry>>,
    pointer_lock_owner: RefCell<Option<WindowId>>,
}

impl WinitSharedState {
    fn new() -> Self {
        let event_loop = EventLoop::new().expect("winit event loop creation must succeed");
        Self {
            event_loop: RefCell::new(event_loop),
            windows: RefCell::new(BTreeMap::new()),
            pointer_lock_owner: RefCell::new(None),
        }
    }

    fn window_entry(
        &self,
        window_id: WindowId,
    ) -> Option<(Rc<RefCell<WinitEventState>>, Rc<WinitWindow>)> {
        self.windows
            .borrow()
            .get(&window_id)
            .map(|entry| (entry.state.clone(), entry.window.clone()))
    }

    fn remove_window(&self, window_id: WindowId) {
        self.clear_pointer_lock_owner(window_id);
        self.windows.borrow_mut().remove(&window_id);
    }

    fn claim_pointer_lock(
        &self,
        window_id: WindowId,
    ) -> Option<(Rc<RefCell<WinitEventState>>, Rc<WinitWindow>)> {
        let previous =
            replace_exclusive_owner(&mut self.pointer_lock_owner.borrow_mut(), window_id);
        previous
            .filter(|previous| *previous != window_id)
            .and_then(|previous| self.window_entry(previous))
    }

    fn clear_pointer_lock_owner(&self, window_id: WindowId) {
        let _ = clear_exclusive_owner(&mut self.pointer_lock_owner.borrow_mut(), window_id);
    }

    fn pointer_lock_owner(&self) -> Option<WindowId> {
        *self.pointer_lock_owner.borrow()
    }
}

fn replace_exclusive_owner<T: Copy + Eq>(owner: &mut Option<T>, new_owner: T) -> Option<T> {
    if *owner == Some(new_owner) {
        None
    } else {
        owner.replace(new_owner)
    }
}

fn clear_exclusive_owner<T: Copy + Eq>(owner: &mut Option<T>, expected_owner: T) -> bool {
    if *owner != Some(expected_owner) {
        false
    } else {
        *owner = None;
        true
    }
}

struct WinitPumpHandler {
    shared: Rc<WinitSharedState>,
}

impl ApplicationHandler for WinitPumpHandler {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some((state, window)) = self.shared.window_entry(window_id) else {
            return;
        };
        let mut state = state.borrow_mut();

        // Winit does not expose a dedicated fullscreen-changed event. Its
        // macOS delegate does update Window::fullscreen(), so synthesize the
        // framework event while handling the next native window event.
        let resize_event = matches!(
            &event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        );
        let fullscreen_changed = state.sync_fullscreen_state(&window);
        if fullscreen_changed && !resize_event {
            let size = window.inner_size();
            let scale_factor = state.scale_factor;
            state.push(Event::Resize {
                width: physical_to_logical_len(size.width, scale_factor),
                height: physical_to_logical_len(size.height, scale_factor),
            });
        }

        match event {
            WindowEvent::CloseRequested => {
                state.push(Event::Window(
                    scarlet_ui_core::event::WindowEvent::CloseRequested,
                ));
            }
            WindowEvent::Destroyed => {
                self.shared.clear_pointer_lock_owner(window_id);
                mark_pointer_lock_released(&mut state);
                state.push(Event::Quit);
            }
            WindowEvent::Resized(size) => {
                let logical_width = physical_to_logical_len(size.width, state.scale_factor);
                let logical_height = physical_to_logical_len(size.height, state.scale_factor);
                state.push(Event::Resize {
                    width: logical_width,
                    height: logical_height,
                });
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                state.scale_factor = scale_factor;
                let size = window.inner_size();
                state.push(Event::Resize {
                    width: physical_to_logical_len(size.width, scale_factor),
                    height: physical_to_logical_len(size.height, scale_factor),
                });
            }
            WindowEvent::Focused(focused) => {
                state.window_focused = focused;
                if !focused {
                    self.shared.clear_pointer_lock_owner(window_id);
                    release_native_pointer_lock(&window, &mut state);
                    state.manual_move_active = false;
                    state.ime_preedit_active = false;
                    state.modifiers = KeyModifiers::empty();
                    state.discard_pending_empty_preedit();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                state.modifiers = map_modifiers(modifiers.state());
            }
            WindowEvent::CursorMoved { position, .. } => {
                let new_x = physical_to_logical_pos(position.x, state.scale_factor);
                let new_y = physical_to_logical_pos(position.y, state.scale_factor);
                if state.manual_move_active {
                    if let Ok(outer) = window.outer_position() {
                        let global_x = outer.x as f64 + position.x;
                        let global_y = outer.y as f64 + position.y;
                        let new_outer_x = state.manual_move_origin_outer_x as f64 + global_x
                            - state.manual_move_origin_global_x;
                        let new_outer_y = state.manual_move_origin_outer_y as f64 + global_y
                            - state.manual_move_origin_global_y;
                        window.set_outer_position(PhysicalPosition::new(
                            f64_to_i32_saturated(new_outer_x.round()),
                            f64_to_i32_saturated(new_outer_y.round()),
                        ));
                    }
                    state.cursor_physical_x = position.x;
                    state.cursor_physical_y = position.y;
                    state.cursor_x = new_x;
                    state.cursor_y = new_y;
                    return;
                }
                state.cursor_physical_x = position.x;
                state.cursor_physical_y = position.y;
                state.cursor_x = new_x;
                state.cursor_y = new_y;
                if state.pointer_locked {
                    return;
                }
                let x = state.cursor_x;
                let y = state.cursor_y;
                state.push(Event::Mouse(MouseEvent::Moved { x, y }));
            }
            WindowEvent::CursorEntered { .. } => {
                let x = state.cursor_x;
                let y = state.cursor_y;
                state.push(Event::Mouse(MouseEvent::Entered { x, y }));
            }
            WindowEvent::CursorLeft { .. } => {
                if state.pointer_locked {
                    self.shared.clear_pointer_lock_owner(window_id);
                    release_native_pointer_lock(&window, &mut state);
                }
                let x = state.cursor_x;
                let y = state.cursor_y;
                state.push(Event::Mouse(MouseEvent::Exited { x, y }));
            }
            WindowEvent::MouseInput {
                state: button_state,
                button,
                ..
            } => {
                let Some(button) = map_mouse_button(button) else {
                    return;
                };
                let x = state.cursor_x;
                let y = state.cursor_y;
                if state.manual_move_active {
                    if button == MouseButton::Left && button_state == WinitElementState::Released {
                        state.manual_move_active = false;
                        let click_count = state.click_state.release_count(MouseButton::Left);
                        state.push(Event::Mouse(MouseEvent::ButtonReleased {
                            button: MouseButton::Left,
                            x,
                            y,
                            click_count,
                        }));
                    }
                    return;
                }
                let event = if button_state == WinitElementState::Pressed {
                    let click_count = state.click_state.press_count(button, x, y);
                    MouseEvent::ButtonPressed {
                        button,
                        x,
                        y,
                        click_count,
                    }
                } else {
                    let click_count = state.click_state.release_count(button);
                    MouseEvent::ButtonReleased {
                        button,
                        x,
                        y,
                        click_count,
                    }
                };
                state.push(Event::Mouse(event));
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                let (delta_x, delta_y, source) = map_winit_wheel_delta(delta, state.scale_factor);
                let x = state.cursor_x;
                let y = state.cursor_y;
                let mapped_phase = map_wheel_phase(phase);
                if scarlet_ui_core::debug::wheel_log_enabled() {
                    println!(
                        "[Wheel] winit source={:?} phase={:?} delta=({}, {}) cursor=({}, {})",
                        source, mapped_phase, delta_x, delta_y, x, y
                    );
                }
                state.push(Event::Mouse(MouseEvent::Wheel {
                    delta_x,
                    delta_y,
                    x,
                    y,
                    phase: mapped_phase,
                    source,
                }));
            }
            WindowEvent::Touch(touch) => {
                if !state.direct_touch_advertised {
                    state.direct_touch_advertised = true;
                    let current = current_input_environment();
                    if !current.direct_touch {
                        state.push(Event::InputEnvironmentChanged(
                            InputEnvironment::new(
                                current.generation.saturating_add(1),
                                current.tablet_mode,
                                current.lid_closed,
                                true,
                                current.fine_pointer,
                                current.keyboard,
                                current.pen,
                            )
                            .with_system_mode(
                                current.windowing_mode,
                                current.tablet_mode_override_active,
                                current.windowing_mode_override_active,
                            ),
                        ));
                    }
                }

                state.push_direct_touch_events(touch.id, touch.phase, touch.location);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let keycode = map_key(&event.logical_key);
                let modifiers = state.modifiers;
                if event.state == WinitElementState::Pressed {
                    state.push(Event::Keyboard(KeyEvent::Pressed { keycode, modifiers }));
                    if !state.ime_preedit_active
                        && let Key::Character(text) = &event.logical_key
                    {
                        for c in text.chars() {
                            if !c.is_control() {
                                state.push(Event::Keyboard(KeyEvent::Char { c }));
                            }
                        }
                    }
                } else {
                    state.push(Event::Keyboard(KeyEvent::Released { keycode, modifiers }));
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                state.discard_pending_empty_preedit();
                state.ime_preedit_active = false;
                let context_id = state.text_input_context_id;
                let serial = state.next_text_input_serial();
                state.push(Event::TextInputCommit {
                    context_id,
                    serial,
                    text,
                });
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                state.discard_pending_empty_preedit();
                if text.is_empty() {
                    state.ime_preedit_active = false;
                    state.defer_empty_preedit();
                    return;
                }
                state.ime_preedit_active = true;
                let (cursor_byte, anchor_byte) = cursor
                    .map_or((text.len() as u32, text.len() as u32), |(start, end)| {
                        (end as u32, start as u32)
                    });
                let context_id = state.text_input_context_id;
                let serial = state.next_text_input_serial();
                state.push(Event::TextInputPreedit {
                    context_id,
                    serial,
                    cursor_byte,
                    anchor_byte,
                    text,
                    spans: Vec::new(),
                });
            }
            WindowEvent::Ime(Ime::Disabled) => {
                state.discard_pending_empty_preedit();
                state.ime_preedit_active = false;
                let context_id = state.text_input_context_id;
                let serial = state.next_text_input_serial();
                state.push(Event::TextInputPreedit {
                    context_id,
                    serial,
                    cursor_byte: 0,
                    anchor_byte: 0,
                    text: alloc::string::String::new(),
                    spans: Vec::new(),
                });
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: ::winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        let DeviceEvent::MouseMotion { delta: (dx, dy) } = event else {
            return;
        };
        let Some(owner) = self.shared.pointer_lock_owner() else {
            return;
        };
        let Some((state, _window)) = self.shared.window_entry(owner) else {
            self.shared.clear_pointer_lock_owner(owner);
            return;
        };
        let mut state = state.borrow_mut();
        if let Some(event) = relative_motion_event(dx, dy, true, &state) {
            state.push(event);
        }
    }
}

fn relative_motion_event(
    dx: f64,
    dy: f64,
    is_pointer_lock_owner: bool,
    state: &WinitEventState,
) -> Option<Event> {
    (is_pointer_lock_owner && state.window_focused && state.pointer_locked).then(|| {
        Event::Mouse(MouseEvent::RelativeMotion {
            dx: f64_to_i32_saturated(dx.round()),
            dy: f64_to_i32_saturated(dy.round()),
        })
    })
}

fn release_native_pointer_lock(window: &WinitWindow, state: &mut WinitEventState) {
    let _ = window.set_cursor_grab(CursorGrabMode::None);
    window.set_cursor_visible(true);
    mark_pointer_lock_released(state);
}

fn mark_pointer_lock_released(state: &mut WinitEventState) {
    if !state.pointer_locked {
        return;
    }
    state.pointer_locked = false;
    state.push(Event::PointerLockChanged { locked: false });
}

pub struct WinitPlatformWindow {
    shared: Rc<WinitSharedState>,
    #[cfg(feature = "sgfx")]
    sgfx_backend: Option<SgfxWindowPaintBackend>,
    #[cfg(feature = "sgfx")]
    using_sgfx: bool,
    window: Rc<WinitWindow>,
    surface: SoftbufferSurface,
    state: Rc<RefCell<WinitEventState>>,
    current_size: Size,
    ime_allowed: bool,
    surface_id: u32,
}

fn validate_window_decoration(decoration: WindowDecoration) -> Result<()> {
    if decoration.title_bar.is_system() && !decoration.frame.is_system() {
        return Err(Error::WindowDecorationUnsupported);
    }
    #[cfg(not(target_os = "macos"))]
    if decoration.frame.is_system() && !decoration.title_bar.is_system() {
        return Err(Error::WindowDecorationUnsupported);
    }
    Ok(())
}

fn system_window_decorations_enabled(decoration: WindowDecoration) -> bool {
    decoration.frame.is_system() || decoration.title_bar.is_system()
}

#[cfg(target_os = "macos")]
fn apply_platform_window_decoration(
    attributes: WindowAttributes,
    decoration: WindowDecoration,
) -> WindowAttributes {
    if decoration.frame.is_system() && !decoration.title_bar.is_system() {
        attributes
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_titlebar_buttons_hidden(true)
            .with_fullsize_content_view(true)
            .with_has_shadow(true)
    } else {
        attributes
    }
}

#[cfg(not(target_os = "macos"))]
fn apply_platform_window_decoration(
    attributes: WindowAttributes,
    _decoration: WindowDecoration,
) -> WindowAttributes {
    attributes
}

impl WinitPlatformWindow {
    fn create(shared: Rc<WinitSharedState>, request: WindowCreateRequest) -> Result<Self> {
        validate_window_decoration(request.decoration)?;
        let placement = request.placement;
        let requested_position = match request.placement {
            scarlet_ui_core::platform::WindowPlacement::Default
            | scarlet_ui_core::platform::WindowPlacement::Centered => None,
            scarlet_ui_core::platform::WindowPlacement::At { x, y } => {
                Some(Position::Logical(LogicalPosition::new(x as f64, y as f64)))
            }
        };
        let attributes = WindowAttributes::default()
            .with_title(request.title)
            .with_decorations(system_window_decorations_enabled(request.decoration))
            .with_transparent(!request.opaque)
            .with_inner_size(LogicalSize::new(request.size.width, request.size.height));
        let mut attributes = apply_platform_window_decoration(attributes, request.decoration);
        if let Some(position) = requested_position {
            attributes = attributes.with_position(position);
        }
        let context = {
            let event_loop = shared.event_loop.borrow();
            SoftbufferContext::new(event_loop.owned_display_handle()).map_err(|_| Error::IoError)?
        };
        #[allow(deprecated)]
        let window = {
            let event_loop = shared.event_loop.borrow();
            Rc::new(
                event_loop
                    .create_window(attributes)
                    .map_err(|_| Error::WindowCreationFailed)?,
            )
        };
        window.set_ime_allowed(false);
        if matches!(
            placement,
            scarlet_ui_core::platform::WindowPlacement::Centered
        ) && let Some(monitor) = window.current_monitor()
        {
            let monitor_position = monitor.position();
            let monitor_size = monitor.size();
            let window_size = window.outer_size();
            window.set_outer_position(PhysicalPosition::new(
                monitor_position.x
                    + (monitor_size.width as i32 - window_size.width as i32).max(0) / 2,
                monitor_position.y
                    + (monitor_size.height as i32 - window_size.height as i32).max(0) / 2,
            ));
        }
        let scale_factor = window.scale_factor();
        let inner_size = window.inner_size();
        let surface =
            SoftbufferSurface::new(&context, window.clone()).map_err(|_| Error::IoError)?;
        #[cfg(feature = "sgfx")]
        let sgfx_backend = match winit_renderer_preference() {
            WinitRendererPreference::Cpu => None,
            preference @ (WinitRendererPreference::Auto | WinitRendererPreference::Sgfx) => {
                match create_sgfx_backend(
                    window.clone(),
                    inner_size.width,
                    inner_size.height,
                    !request.opaque,
                ) {
                    Ok(backend) => Some(backend),
                    Err(error)
                        if preference == WinitRendererPreference::Auto
                            && !sgfx_backend_override_requested() =>
                    {
                        eprintln!(
                            "[ScarletUI] SGFX initialization failed ({error}); falling back to CPU"
                        );
                        None
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        let surface_id = next_surface_id();
        let state = Rc::new(RefCell::new(WinitEventState::new(scale_factor)));
        shared.windows.borrow_mut().insert(
            window.id(),
            WinitWindowEntry {
                state: state.clone(),
                window: window.clone(),
            },
        );
        Ok(Self {
            shared,
            #[cfg(feature = "sgfx")]
            using_sgfx: sgfx_backend.is_some(),
            #[cfg(feature = "sgfx")]
            sgfx_backend,
            window,
            surface,
            state,
            current_size: physical_to_logical_size(
                inner_size.width,
                inner_size.height,
                scale_factor,
            ),
            ime_allowed: false,
            surface_id,
        })
    }

    fn pump_events(&mut self) {
        let mut handler = WinitPumpHandler {
            shared: self.shared.clone(),
        };
        let _ = self
            .shared
            .event_loop
            .borrow_mut()
            .pump_app_events(Some(Duration::ZERO), &mut handler);
        let mut state = self.state.borrow_mut();
        state.flush_pending_empty_preedit();
        state.flush_expired_trackpad_end();
    }

    fn resize_surface(&mut self, width: u32, height: u32) -> Result<()> {
        let width = NonZeroU32::new(width.max(1)).ok_or(Error::InvalidSize { width, height })?;
        let height = NonZeroU32::new(height.max(1)).ok_or(Error::InvalidSize {
            width: width.get(),
            height,
        })?;
        self.surface
            .resize(width, height)
            .map_err(|_| Error::RenderError)
    }

    pub(crate) fn set_observed_logical_size(&mut self, size: Size) {
        self.current_size = size;
    }

    fn release_pointer_lock(&mut self) {
        self.shared.clear_pointer_lock_owner(self.window.id());
        release_native_pointer_lock(&self.window, &mut self.state.borrow_mut());
    }
}

impl PlatformWindow for WinitPlatformWindow {
    fn new(app_id: &str, title: &str, size: Size) -> Result<Self> {
        let backend = WinitBackend::new();
        Self::create(
            backend.shared.clone(),
            WindowCreateRequest {
                app_id: alloc::string::String::from(app_id),
                title: alloc::string::String::from(title),
                size,
                size_limits: scarlet_ui_core::element::WindowSizeLimits::default(),
                window_type: 0,
                menu_titles: alloc::string::String::new(),
                focus_on_create: true,
                active_on_focus: true,
                opaque: true,
                decoration: WindowDecoration::CUSTOM,
                placement: scarlet_ui_core::platform::WindowPlacement::Default,
                window_geometry_insets: scarlet_ui_core::geometry::EdgeInsets::ZERO,
            },
        )
    }

    fn poll_event(&mut self) -> Option<Event> {
        self.pump_events();
        let event = self.state.borrow_mut().pop();
        if let Some(Event::Resize { width, height }) = event {
            let size = Size::new(width as f32, height as f32);
            self.set_observed_logical_size(size);
            Some(Event::Resize { width, height })
        } else {
            event
        }
    }

    fn wait_for_event(&mut self, timeout: Duration) {
        let mut handler = WinitPumpHandler {
            shared: self.shared.clone(),
        };
        let _ = self
            .shared
            .event_loop
            .borrow_mut()
            .pump_app_events(Some(timeout), &mut handler);
        let mut state = self.state.borrow_mut();
        state.flush_pending_empty_preedit();
        state.flush_expired_trackpad_end();
    }

    fn output_scale_milli(&self) -> u32 {
        scale_factor_to_milli(self.window.scale_factor())
    }

    fn renderer_backend(&self) -> scarlet_ui_core::renderer::RendererBackendKind {
        #[cfg(feature = "sgfx")]
        if self.using_sgfx {
            return scarlet_ui_core::renderer::RendererBackendKind::Sgfx;
        }
        scarlet_ui_core::renderer::RendererBackendKind::Cpu
    }

    #[cfg(feature = "sgfx")]
    fn take_paint_backend(&mut self) -> Result<Option<Box<dyn PaintBackend>>> {
        Ok(self
            .sgfx_backend
            .take()
            .map(|backend| Box::new(backend) as Box<dyn PaintBackend>))
    }

    fn present(&mut self, buffer: &Buffer) {
        let _ = self.present_buffer(buffer);
    }

    fn present_with_damage(&mut self, buffer: &Buffer, _damage: Option<&[DamageRect]>) {
        let _ = self.present_buffer(buffer);
    }

    fn set_title(&mut self, title: &str) {
        self.window.set_title(title);
    }

    fn size(&self) -> Size {
        let physical = self.window.inner_size();
        physical_to_logical_size(
            physical.width.max(1),
            physical.height.max(1),
            self.window.scale_factor(),
        )
    }

    fn physical_size(&self) -> (u32, u32) {
        let size = self.window.inner_size();
        (size.width.max(1), size.height.max(1))
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let _ = self
            .window
            .request_inner_size(LogicalSize::new(width as f32, height as f32));
        self.current_size = Size::new(width as f32, height as f32);
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.release_pointer_lock();
        self.window.set_visible(false);
        let mut handler = WinitPumpHandler {
            shared: self.shared.clone(),
        };
        let _ = self
            .shared
            .event_loop
            .borrow_mut()
            .pump_app_events(Some(Duration::ZERO), &mut handler);
        Ok(())
    }

    fn minimize(&mut self) -> Result<()> {
        self.release_pointer_lock();
        self.window.set_minimized(true);
        Ok(())
    }

    fn maximize(&mut self) -> Result<()> {
        self.window.set_maximized(true);
        Ok(())
    }

    fn set_fullscreen(&mut self, fullscreen: bool) -> Result<()> {
        let mode = fullscreen.then(|| Fullscreen::Borderless(self.window.current_monitor()));
        self.window.set_fullscreen(mode);
        Ok(())
    }

    fn set_pointer_lock(&mut self, locked: bool) -> Result<()> {
        if !locked {
            self.release_pointer_lock();
            return Ok(());
        }
        if self.state.borrow().pointer_locked {
            return Ok(());
        }

        self.window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
            .map_err(|_| Error::PointerLockUnsupported)?;
        self.window.set_cursor_visible(false);
        if let Some((previous_state, previous_window)) =
            self.shared.claim_pointer_lock(self.window.id())
        {
            release_native_pointer_lock(&previous_window, &mut previous_state.borrow_mut());
        }
        let mut state = self.state.borrow_mut();
        state.pointer_locked = true;
        state.push(Event::PointerLockChanged { locked: true });
        Ok(())
    }

    fn pointer_locked(&self) -> bool {
        self.state.borrow().pointer_locked
    }

    fn restore(&mut self) -> Result<()> {
        self.window.set_minimized(false);
        self.window.set_maximized(false);
        Ok(())
    }

    fn focus(&mut self) -> Result<()> {
        self.window.focus_window();
        Ok(())
    }

    fn request_move(&mut self) -> Result<()> {
        let outer = self
            .window
            .outer_position()
            .map_err(|_| Error::EventDispatchError)?;
        let mut state = self.state.borrow_mut();
        state.manual_move_active = true;
        state.manual_move_origin_outer_x = outer.x;
        state.manual_move_origin_outer_y = outer.y;
        state.manual_move_origin_global_x = outer.x as f64 + state.cursor_physical_x;
        state.manual_move_origin_global_y = outer.y as f64 + state.cursor_physical_y;
        Ok(())
    }

    fn create_popup(&mut self, _position: Point, _size: Size) -> Result<u32> {
        Ok(0)
    }

    fn destroy_popup(&mut self, _surface_id: u32) -> Result<()> {
        Ok(())
    }

    fn set_workarea(&mut self, _x: i32, _y: i32, _width: u32, _height: u32) -> Result<()> {
        Ok(())
    }

    fn create_window_with_type(
        &mut self,
        app_id: &str,
        title: &str,
        size: Size,
        window_type: u32,
    ) -> Result<Self>
    where
        Self: Sized,
    {
        let _ = app_id;
        Self::create(
            self.shared.clone(),
            WindowCreateRequest {
                app_id: alloc::string::String::from(app_id),
                title: alloc::string::String::from(title),
                size,
                size_limits: scarlet_ui_core::element::WindowSizeLimits::default(),
                window_type,
                menu_titles: alloc::string::String::new(),
                focus_on_create: true,
                active_on_focus: true,
                opaque: true,
                decoration: WindowDecoration::CUSTOM,
                placement: scarlet_ui_core::platform::WindowPlacement::Default,
                window_geometry_insets: scarlet_ui_core::geometry::EdgeInsets::ZERO,
            },
        )
    }

    fn move_window(&mut self, x: i32, y: i32) -> Result<()> {
        self.window
            .set_outer_position(LogicalPosition::new(x as f32, y as f32));
        Ok(())
    }

    fn set_window_type(&mut self, _surface_id: u32, _window_type: u32) -> Result<()> {
        Ok(())
    }

    fn get_screen_size(&mut self) -> Result<(u32, u32)> {
        let Some(monitor) = self.window.current_monitor() else {
            return Ok((
                self.current_size.width as u32,
                self.current_size.height as u32,
            ));
        };
        let size = monitor.size();
        Ok((size.width, size.height))
    }

    fn surface_id(&self) -> u32 {
        self.surface_id
    }

    fn platform_window_id(&self) -> u64 {
        self.surface_id as u64
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn set_resizable(&mut self, resizable: bool) -> Result<()> {
        self.window.set_resizable(resizable);
        Ok(())
    }

    fn set_opaque(&mut self, _opaque: bool) -> Result<()> {
        Ok(())
    }

    fn set_menu_titles(&mut self, _menu_titles: &str) -> Result<()> {
        Ok(())
    }

    fn sync_text_input(&mut self, state: Option<&TextInputElementState>) {
        let Some(state) = state else {
            let mut event_state = self.state.borrow_mut();
            event_state.ime_preedit_active = false;
            event_state.discard_pending_empty_preedit();
            if self.ime_allowed {
                self.window.set_ime_allowed(false);
                self.ime_allowed = false;
            }
            return;
        };
        if !self.ime_allowed {
            self.window.set_ime_allowed(true);
            self.ime_allowed = true;
        }
        self.window.set_ime_cursor_area(
            LogicalPosition::new(state.cursor_rect.origin.x, state.cursor_rect.origin.y),
            LogicalSize::new(state.cursor_rect.size.width, state.cursor_rect.size.height),
        );
    }

    fn raw_window_handle(&self) -> Option<raw_window_handle::RawWindowHandle> {
        use raw_window_handle::HasWindowHandle;
        self.window.window_handle().ok().map(|h| h.as_raw())
    }

    fn raw_display_handle(&self) -> Option<raw_window_handle::RawDisplayHandle> {
        use raw_window_handle::HasDisplayHandle;
        self.window.display_handle().ok().map(|h| h.as_raw())
    }
}

impl Drop for WinitPlatformWindow {
    fn drop(&mut self) {
        self.shared.remove_window(self.window.id());
    }
}

impl WinitPlatformWindow {
    fn present_buffer(&mut self, buffer: &Buffer) -> Result<()> {
        let physical_size = self.window.inner_size();
        let width = physical_size.width.max(1);
        let height = physical_size.height.max(1);
        self.resize_surface(width, height)?;
        let mut surface_buffer = self.surface.buffer_mut().map_err(|_| Error::RenderError)?;
        copy_scaled(buffer, &mut surface_buffer, width, height);
        surface_buffer.present().map_err(|_| Error::RenderError)
    }
}

fn physical_to_logical_len(value: u32, scale_factor: f64) -> u32 {
    ((value as f64 / scale_factor.max(0.001)).round() as u32).max(1)
}

fn scale_factor_to_milli(scale_factor: f64) -> u32 {
    (scale_factor.max(0.001) * 1000.0).round() as u32
}

fn physical_to_logical_pos(value: f64, scale_factor: f64) -> i32 {
    (value / scale_factor.max(0.001)).round() as i32
}

fn map_winit_wheel_delta(delta: MouseScrollDelta, scale_factor: f64) -> (i32, i32, ScrollSource) {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => {
            ((x * 32.0) as i32, (y * 32.0) as i32, ScrollSource::Wheel)
        }
        MouseScrollDelta::PixelDelta(delta) => (
            physical_to_logical_pos(delta.x, scale_factor),
            physical_to_logical_pos(delta.y, scale_factor),
            ScrollSource::Trackpad,
        ),
    }
}

fn f64_to_i32_saturated(value: f64) -> i32 {
    if value <= i32::MIN as f64 {
        i32::MIN
    } else if value >= i32::MAX as f64 {
        i32::MAX
    } else {
        value as i32
    }
}

fn physical_to_logical_size(width: u32, height: u32, scale_factor: f64) -> Size {
    Size::new(
        physical_to_logical_len(width, scale_factor) as f32,
        physical_to_logical_len(height, scale_factor) as f32,
    )
}

fn next_surface_id() -> u32 {
    static NEXT_SURFACE_ID: AtomicU32 = AtomicU32::new(1);
    NEXT_SURFACE_ID.fetch_add(1, Ordering::Relaxed).max(1)
}

fn copy_scaled(buffer: &Buffer, dst: &mut [u32], dst_width: u32, dst_height: u32) {
    let src_width = buffer.width().max(1);
    let src_height = buffer.height().max(1);
    let src = buffer.as_slice();
    if src_width == dst_width && src_height == dst_height {
        let len = dst.len().min(src.len());
        dst[..len].copy_from_slice(&src[..len]);
        return;
    }

    for y in 0..dst_height {
        let src_y = (y as u64 * src_height as u64 / dst_height as u64) as u32;
        for x in 0..dst_width {
            let src_x = (x as u64 * src_width as u64 / dst_width as u64) as u32;
            let src_index = (src_y * src_width + src_x) as usize;
            let dst_index = (y * dst_width + x) as usize;
            if let (Some(dst_px), Some(src_px)) = (dst.get_mut(dst_index), src.get(src_index)) {
                *dst_px = *src_px;
            }
        }
    }
}

fn map_mouse_button(button: WinitMouseButton) -> Option<MouseButton> {
    match button {
        WinitMouseButton::Left => Some(MouseButton::Left),
        WinitMouseButton::Middle => Some(MouseButton::Middle),
        WinitMouseButton::Right => Some(MouseButton::Right),
        _ => None,
    }
}

fn map_wheel_phase(phase: TouchPhase) -> WheelPhase {
    match phase {
        TouchPhase::Started => WheelPhase::Started,
        TouchPhase::Moved => WheelPhase::Moved,
        TouchPhase::Ended => WheelPhase::Ended,
        TouchPhase::Cancelled => WheelPhase::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui_core::platform::{WindowFrame, WindowTitleBar};

    #[test]
    fn system_owned_frame_or_titlebar_enables_platform_window_decorations() {
        assert!(system_window_decorations_enabled(WindowDecoration::SYSTEM));
        assert!(system_window_decorations_enabled(WindowDecoration::new(
            WindowFrame::System,
            WindowTitleBar::Custom,
        )));
        assert!(!system_window_decorations_enabled(WindowDecoration::CUSTOM));
        assert!(!system_window_decorations_enabled(WindowDecoration::NONE));
    }

    #[test]
    fn system_titlebar_requires_a_system_frame() {
        assert_eq!(
            validate_window_decoration(WindowDecoration::new(
                WindowFrame::Custom,
                WindowTitleBar::System,
            )),
            Err(Error::WindowDecorationUnsupported)
        );
    }

    #[test]
    fn line_wheel_mapping_preserves_platform_direction() {
        assert_eq!(
            map_winit_wheel_delta(MouseScrollDelta::LineDelta(1.0, -2.0), 1.0),
            (32, -64, ScrollSource::Wheel)
        );
    }

    #[test]
    fn trackpad_mapping_preserves_platform_direction_while_scaling() {
        assert_eq!(
            map_winit_wheel_delta(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(20.0, -10.0)),
                2.0,
            ),
            (10, -5, ScrollSource::Trackpad)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_supports_system_frame_with_custom_titlebar() {
        assert_eq!(
            validate_window_decoration(WindowDecoration::new(
                WindowFrame::System,
                WindowTitleBar::Custom,
            )),
            Ok(())
        );
    }

    fn wheel(delta_y: i32, phase: WheelPhase, source: ScrollSource) -> Event {
        Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y,
            x: 10,
            y: 20,
            phase,
            source,
        })
    }

    fn trackpad_moved(delta_y: i32, x: i32, y: i32) -> Event {
        Event::Mouse(MouseEvent::Wheel {
            delta_x: 1,
            delta_y,
            x,
            y,
            phase: WheelPhase::Moved,
            source: ScrollSource::Trackpad,
        })
    }

    fn single_direct_touch_event(
        state: &mut WinitEventState,
        touch_id: u64,
        phase: TouchPhase,
        location: PhysicalPosition<f64>,
    ) -> Option<Event> {
        let events = state.map_direct_touch_events(touch_id, phase, location);
        assert!(
            events.len() <= 1,
            "non-terminal touch phases must produce at most one event"
        );
        events.into_iter().next()
    }

    #[test]
    fn device_motion_routes_only_to_exclusive_pointer_lock_owner() {
        let mut owner_state = WinitEventState::new_with_wheel_coalesce(1.0, false);
        let mut other_state = WinitEventState::new_with_wheel_coalesce(1.0, false);
        owner_state.pointer_locked = true;
        other_state.pointer_locked = true;

        assert!(matches!(
            relative_motion_event(3.6, -2.4, true, &owner_state),
            Some(Event::Mouse(MouseEvent::RelativeMotion { dx: 4, dy: -2 }))
        ));
        assert!(relative_motion_event(3.6, -2.4, false, &other_state).is_none());

        owner_state.window_focused = false;
        assert!(relative_motion_event(3.6, -2.4, true, &owner_state).is_none());
    }

    #[test]
    fn claiming_pointer_lock_replaces_previous_owner() {
        let mut owner = Some(1_u32);

        assert_eq!(replace_exclusive_owner(&mut owner, 2), Some(1));
        assert_eq!(owner, Some(2));
        assert_eq!(replace_exclusive_owner(&mut owner, 2), None);
        assert_eq!(owner, Some(2));
    }

    #[test]
    fn lifecycle_release_clears_only_owner_and_notifies_once() {
        let mut owner = Some(2_u32);
        assert!(!clear_exclusive_owner(&mut owner, 1));
        assert_eq!(owner, Some(2));
        assert!(clear_exclusive_owner(&mut owner, 2));
        assert_eq!(owner, None);

        let mut state = WinitEventState::new_with_wheel_coalesce(1.0, false);
        state.pointer_locked = true;
        mark_pointer_lock_released(&mut state);
        mark_pointer_lock_released(&mut state);
        assert!(matches!(
            state.pop(),
            Some(Event::PointerLockChanged { locked: false })
        ));
        assert!(state.pop().is_none());
    }

    #[test]
    fn direct_touch_state_keeps_the_first_contact_primary_until_release() {
        let mut touch = DirectTouchActivationState::default();

        assert_eq!(
            touch.map(10, TouchPhase::Started),
            Some(DirectTouchActivation::Pressed)
        );
        assert_eq!(touch.map(11, TouchPhase::Started), None);
        assert_eq!(touch.map(11, TouchPhase::Moved), None);
        assert_eq!(
            touch.map(10, TouchPhase::Moved),
            Some(DirectTouchActivation::Moved)
        );
        assert_eq!(touch.map(11, TouchPhase::Ended), None);
        assert_eq!(
            touch.map(10, TouchPhase::Cancelled),
            Some(DirectTouchActivation::Cancelled)
        );
        assert_eq!(touch.map(10, TouchPhase::Ended), None);
        assert_eq!(
            touch.map(11, TouchPhase::Started),
            Some(DirectTouchActivation::Pressed)
        );
    }

    #[test]
    fn direct_touch_maps_primary_contact_to_scaled_mouse_events() {
        let mut state = WinitEventState::new_with_wheel_coalesce(2.0, false);

        assert!(matches!(
            single_direct_touch_event(
                &mut state,
                7,
                TouchPhase::Started,
                PhysicalPosition::new(20.0, 12.0),
            ),
            Some(Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 10,
                y: 6,
                click_count: 1,
            }))
        ));
        assert_eq!(state.cursor_physical_x, 20.0);
        assert_eq!(state.cursor_physical_y, 12.0);
        assert_eq!((state.cursor_x, state.cursor_y), (10, 6));

        assert!(matches!(
            single_direct_touch_event(
                &mut state,
                7,
                TouchPhase::Moved,
                PhysicalPosition::new(30.0, 16.0),
            ),
            Some(Event::Mouse(MouseEvent::Moved { x: 15, y: 8 }))
        ));
        assert!(
            state
                .map_direct_touch_events(8, TouchPhase::Started, PhysicalPosition::new(80.0, 80.0))
                .is_empty()
        );
        assert_eq!(state.cursor_physical_x, 30.0);
        assert_eq!(state.cursor_physical_y, 16.0);

        assert!(matches!(
            state
                .map_direct_touch_events(7, TouchPhase::Ended, PhysicalPosition::new(40.0, 24.0))
                .as_slice(),
            [
                Event::Mouse(MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x: 20,
                    y: 12,
                    click_count: 1,
                }),
                Event::Mouse(MouseEvent::Exited { x: 20, y: 12 }),
            ]
        ));
        assert!(
            state
                .map_direct_touch_events(7, TouchPhase::Ended, PhysicalPosition::new(40.0, 24.0))
                .is_empty()
        );
    }

    #[test]
    fn cancelled_direct_touch_never_maps_to_a_release_or_click() {
        let mut state = WinitEventState::new_with_wheel_coalesce(1.0, false);

        assert!(matches!(
            single_direct_touch_event(
                &mut state,
                7,
                TouchPhase::Started,
                PhysicalPosition::new(10.0, 12.0),
            ),
            Some(Event::Mouse(MouseEvent::ButtonPressed {
                click_count: 1,
                ..
            }))
        ));
        assert!(matches!(
            state
                .map_direct_touch_events(
                    7,
                    TouchPhase::Cancelled,
                    PhysicalPosition::new(20.0, 24.0)
                )
                .as_slice(),
            [
                Event::Mouse(MouseEvent::ButtonCancelled {
                    button: MouseButton::Left,
                    x: 20,
                    y: 24,
                }),
                Event::Mouse(MouseEvent::Exited { x: 20, y: 24 }),
            ]
        ));
        assert!(
            state
                .map_direct_touch_events(7, TouchPhase::Ended, PhysicalPosition::new(20.0, 24.0))
                .is_empty()
        );
        assert!(matches!(
            single_direct_touch_event(
                &mut state,
                8,
                TouchPhase::Started,
                PhysicalPosition::new(10.0, 12.0),
            ),
            Some(Event::Mouse(MouseEvent::ButtonPressed {
                click_count: 1,
                ..
            }))
        ));
    }

    #[test]
    fn direct_touch_terminal_events_keep_exit_after_the_terminal_event_in_queue() {
        let mut state = WinitEventState::new_with_wheel_coalesce(1.0, false);
        state.push_direct_touch_events(7, TouchPhase::Started, PhysicalPosition::new(10.0, 12.0));
        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::ButtonPressed { .. }))
        ));

        state.push_direct_touch_events(7, TouchPhase::Ended, PhysicalPosition::new(20.0, 24.0));
        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::ButtonReleased {
                x: 20,
                y: 24,
                ..
            }))
        ));
        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::Exited { x: 20, y: 24 }))
        ));
        assert!(state.pop().is_none());

        state.push_direct_touch_events(8, TouchPhase::Started, PhysicalPosition::new(30.0, 36.0));
        let _ = state.pop();
        state.push_direct_touch_events(8, TouchPhase::Cancelled, PhysicalPosition::new(40.0, 48.0));
        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::ButtonCancelled {
                x: 40,
                y: 48,
                ..
            }))
        ));
        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::Exited { x: 40, y: 48 }))
        ));
        assert!(state.pop().is_none());
    }

    #[test]
    fn consecutive_trackpad_moved_events_are_coalesced() {
        let mut state = WinitEventState::new_with_wheel_coalesce(1.0, true);

        state.push(wheel(0, WheelPhase::Started, ScrollSource::Trackpad));
        state.push(trackpad_moved(4, 10, 20));
        state.push(trackpad_moved(7, 30, 40));
        state.push(trackpad_moved(-2, 50, 60));

        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::Wheel {
                phase: WheelPhase::Started,
                source: ScrollSource::Trackpad,
                ..
            }))
        ));
        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::Wheel {
                delta_x: 1,
                delta_y: 4,
                x: 10,
                y: 20,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }))
        ));
        assert!(state.pop().is_none());
        state.last_trackpad_moved_emit_at = Some(
            Instant::now()
                .checked_sub(TRACKPAD_MOVED_MIN_INTERVAL + Duration::from_millis(1))
                .unwrap(),
        );
        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::Wheel {
                delta_x: 2,
                delta_y: 5,
                x: 50,
                y: 60,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }))
        ));
        assert!(state.pop().is_none());
    }

    #[test]
    fn trackpad_moved_events_are_rate_limited() {
        let mut state = WinitEventState::new_with_wheel_coalesce(1.0, true);

        state.push(trackpad_moved(4, 10, 20));
        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::Wheel {
                delta_y: 4,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
                ..
            }))
        ));

        state.push(trackpad_moved(7, 30, 40));
        assert!(state.pop().is_none());

        state.last_trackpad_moved_emit_at = Some(
            Instant::now()
                .checked_sub(TRACKPAD_MOVED_MIN_INTERVAL + Duration::from_millis(1))
                .unwrap(),
        );
        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::Wheel {
                delta_y: 7,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
                ..
            }))
        ));
    }

    #[test]
    fn discrete_wheel_moved_events_are_not_coalesced() {
        let mut state = WinitEventState::new_with_wheel_coalesce(1.0, true);

        state.push(wheel(4, WheelPhase::Moved, ScrollSource::Wheel));
        state.push(wheel(7, WheelPhase::Moved, ScrollSource::Wheel));

        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::Wheel {
                delta_y: 4,
                phase: WheelPhase::Moved,
                source: ScrollSource::Wheel,
                ..
            }))
        ));
        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::Wheel {
                delta_y: 7,
                phase: WheelPhase::Moved,
                source: ScrollSource::Wheel,
                ..
            }))
        ));
        assert!(state.pop().is_none());
    }

    #[test]
    fn winit_trackpad_moved_coalescing_can_be_disabled() {
        let mut state = WinitEventState::new_with_wheel_coalesce(1.0, false);

        state.push(trackpad_moved(4, 10, 20));
        state.push(trackpad_moved(7, 30, 40));

        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::Wheel {
                delta_x: 1,
                delta_y: 4,
                x: 10,
                y: 20,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }))
        ));
        assert!(matches!(
            state.pop(),
            Some(Event::Mouse(MouseEvent::Wheel {
                delta_x: 1,
                delta_y: 7,
                x: 30,
                y: 40,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }))
        ));
        assert!(state.pop().is_none());
    }

    #[test]
    fn winit_wheel_coalesce_env_flag_defaults_off_and_one_enables() {
        assert!(env_flag_enabled("1"));
        assert!(env_flag_enabled("true"));
        assert!(env_flag_enabled("on"));
        assert!(!env_flag_enabled(""));
        assert!(!env_flag_enabled("0"));
        assert!(!env_flag_enabled("false"));
        assert!(!env_flag_enabled("off"));
    }
}

fn map_modifiers(modifiers: ModifiersState) -> KeyModifiers {
    KeyModifiers {
        shift: modifiers.shift_key(),
        control: modifiers.control_key(),
        alt: modifiers.alt_key(),
        super_key: modifiers.super_key(),
    }
}

fn map_key(key: &Key) -> KeyCode {
    match key {
        Key::Named(NamedKey::Escape) => KeyCode::Escape,
        Key::Named(NamedKey::Enter) => KeyCode::Enter,
        Key::Named(NamedKey::Tab) => KeyCode::Tab,
        Key::Named(NamedKey::Backspace) => KeyCode::Backspace,
        Key::Named(NamedKey::Space) => KeyCode::Space,
        Key::Named(NamedKey::ArrowLeft) => KeyCode::Left,
        Key::Named(NamedKey::ArrowRight) => KeyCode::Right,
        Key::Named(NamedKey::ArrowUp) => KeyCode::Up,
        Key::Named(NamedKey::ArrowDown) => KeyCode::Down,
        Key::Named(NamedKey::Home) => KeyCode::Home,
        Key::Named(NamedKey::End) => KeyCode::End,
        Key::Named(NamedKey::PageUp) => KeyCode::PageUp,
        Key::Named(NamedKey::PageDown) => KeyCode::PageDown,
        Key::Named(NamedKey::Insert) => KeyCode::Insert,
        Key::Named(NamedKey::Delete) => KeyCode::Delete,
        Key::Named(NamedKey::F1) => KeyCode::F(1),
        Key::Named(NamedKey::F2) => KeyCode::F(2),
        Key::Named(NamedKey::F3) => KeyCode::F(3),
        Key::Named(NamedKey::F4) => KeyCode::F(4),
        Key::Named(NamedKey::F5) => KeyCode::F(5),
        Key::Named(NamedKey::F6) => KeyCode::F(6),
        Key::Named(NamedKey::F7) => KeyCode::F(7),
        Key::Named(NamedKey::F8) => KeyCode::F(8),
        Key::Named(NamedKey::F9) => KeyCode::F(9),
        Key::Named(NamedKey::F10) => KeyCode::F(10),
        Key::Named(NamedKey::F11) => KeyCode::F(11),
        Key::Named(NamedKey::F12) => KeyCode::F(12),
        Key::Character(text) => text.chars().next().map_or(KeyCode::Unknown, KeyCode::Char),
        _ => KeyCode::Unknown,
    }
}
