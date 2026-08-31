//! SWS (Scarlet Window Server) backend for PlatformWindow
//!
//! This implementation uses the sws-client library to create and manage windows.

#![cfg(target_os = "scarlet")]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
#[cfg(not(feature = "std"))]
extern crate scarlet_std as std;

mod backend;
mod sink;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
pub use backend::{
    DEFAULT_GPU_DEVICE, Error as SgfxPaintError, Result as SgfxPaintResult, SgfxPaintBackend,
    Stage as SgfxPaintStage,
};
use core::sync::atomic::{AtomicBool, Ordering};
use scarlet_ui_core::buffer::Buffer;
use scarlet_ui_core::color::Color;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::element::TextInputElementState;
use scarlet_ui_core::error::{Error, Result};
use scarlet_ui_core::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, ScrollSource, WheelPhase,
};
use scarlet_ui_core::geometry::{EdgeInsets, Point, Rect, Size};
use scarlet_ui_core::input_environment::{InputEnvironment, WindowingMode};
use scarlet_ui_core::platform::{
    PlatformBackend, PlatformWindow, PlatformWindowDefaults, WindowCreateRequest, WindowDecoration,
    WindowPlacement,
};
use scarlet_ui_core::renderer::{
    BackendFrame, CompositorBackendKind, PaintBackend, PaintContext, RendererBackendKind,
};
pub use scarlet_ui_renderer_sgfx::{
    SgfxCanvas, SgfxCanvasDraw, SgfxCanvasFrame, SgfxCanvasHandle, SgfxCanvasRenderObject,
    SgfxCanvasVertex, SgfxMesh, SgfxMeshHandle, SgfxTexture,
};
use sgfx::ImageRef;
pub use sink::{
    SgfxBufferIdentity, SgfxCommitToken, SgfxFrameSink, SgfxSinkError, SgfxSinkResult,
    SgfxSinkStatus,
};
use sws::event::{Event as SwsEvent, abs_code, event_type, key_code, rel_code};
use sws_client as sws;

#[cfg(feature = "std")]
use std::time::{Duration, Instant};

#[cfg(feature = "std")]
macro_rules! logln {
    ($($arg:tt)*) => {
        std::println!($($arg)*)
    };
}

#[cfg(not(feature = "std"))]
macro_rules! logln {
    ($($arg:tt)*) => {
        scarlet_std::println!($($arg)*)
    };
}

const DEFAULT_SCALE_MILLI: u32 = 1000;
const ACTIVATION_TOKEN_ENV: &str = "SWS_ACTIVATION_TOKEN";
static ACTIVATION_TOKEN_CONSUMED: AtomicBool = AtomicBool::new(false);
const WHEEL_LINE_DELTA: i32 = 32;
const WHEEL_HI_RES_UNITS_PER_NOTCH: i32 = 120;
const SWS_LEGACY_WHEEL_PIXELS_PER_NOTCH: i32 = 10;

const KEY_LEFTCTRL: u16 = 0x1d;
const KEY_LEFTSHIFT: u16 = 0x2a;
const KEY_RIGHTSHIFT: u16 = 0x36;
const KEY_LEFTALT: u16 = 0x38;
const KEY_RIGHTCTRL: u16 = 0x61;
const KEY_RIGHTALT: u16 = 0x64;
const KEY_LEFTMETA: u16 = 0x7d;
const KEY_RIGHTMETA: u16 = 0x7e;
const KEY_GRAVE: u16 = 0x29;

fn sgfx_event_epoch_is_stale(backend_lost: Option<u32>, event_epoch: u32) -> bool {
    backend_lost.is_some_and(|lost_epoch| event_epoch < lost_epoch)
}

#[cfg(feature = "std")]
const DOUBLE_CLICK_THRESHOLD: Duration = Duration::from_millis(500);
#[cfg(not(feature = "std"))]
const DOUBLE_CLICK_EVENT_THRESHOLD: u64 = 20;
const DOUBLE_CLICK_DISTANCE: i32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestedRendererBackend {
    Auto,
    Cpu,
    Sgfx,
}

impl RequestedRendererBackend {
    fn from_environment() -> Result<Self> {
        let Some(value) = renderer_backend_environment_value() else {
            return Ok(Self::Auto);
        };

        match value.as_str() {
            "auto" => Ok(Self::Auto),
            "cpu" => Ok(Self::Cpu),
            "sgfx" => Ok(Self::Sgfx),
            _ => Err(scarlet_ui_core::error::Error::InvalidRendererBackend {
                value: value.clone(),
            }),
        }
    }
}

#[cfg(feature = "std")]
fn renderer_backend_environment_value() -> Option<String> {
    std::env::var("SCARLET_UI_BACKEND").ok()
}

#[cfg(not(feature = "std"))]
fn renderer_backend_environment_value() -> Option<String> {
    std::env::var("SCARLET_UI_BACKEND")
}

fn sgfx_backend_override_requested() -> bool {
    !matches!(
        sgfx::BackendPreference::from_environment(),
        Ok(sgfx::BackendPreference::Auto)
    )
}

#[cfg(feature = "std")]
fn activation_token_environment_value() -> Option<String> {
    std::env::var(ACTIVATION_TOKEN_ENV).ok()
}

#[cfg(not(feature = "std"))]
fn activation_token_environment_value() -> Option<String> {
    std::env::var(ACTIVATION_TOKEN_ENV)
}

fn take_launch_activation_token(window_type: u32) -> Option<String> {
    if window_type != sws_protocol::window_types::NORMAL
        || ACTIVATION_TOKEN_CONSUMED.load(Ordering::Acquire)
    {
        return None;
    }

    let token = activation_token_environment_value().filter(|token| !token.is_empty())?;

    if ACTIVATION_TOKEN_CONSUMED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        Some(token)
    } else {
        None
    }
}

struct SwsSgfxFrameSink {
    conn: sws::Connection,
    events: sws::EventReceiver,
    window_id: u32,
    compositor_epoch: u32,
    backend_lost: Option<u32>,
    retained: Vec<SgfxCommitToken>,
    rejected: Vec<(SgfxCommitToken, SgfxSinkError)>,
    next_commit_serial: u64,
    lifecycle_error: Option<SgfxSinkError>,
}

impl SwsSgfxFrameSink {
    fn new(conn: sws::Connection, window_id: u32, compositor_epoch: u32) -> Self {
        let events = conn.subscribe_sgfx_events(window_id);
        Self {
            conn,
            events,
            window_id,
            compositor_epoch,
            backend_lost: None,
            retained: Vec::new(),
            rejected: Vec::new(),
            next_commit_serial: 1,
            lifecycle_error: None,
        }
    }

    fn validate_identity(&self, identity: SgfxBufferIdentity) -> SgfxSinkResult<()> {
        if identity.window_id != self.window_id
            || identity.compositor_epoch != self.compositor_epoch
        {
            return Err(SgfxSinkError::InvalidIdentity);
        }
        if let Some(compositor_epoch) = self.backend_lost {
            return Err(SgfxSinkError::BackendLost { compositor_epoch });
        }
        Ok(())
    }

    fn client_identity(identity: SgfxBufferIdentity) -> sws::SgfxBufferIdentity {
        sws::SgfxBufferIdentity {
            window_id: identity.window_id,
            buffer_id: identity.buffer_id,
            generation: identity.generation,
            compositor_epoch: identity.compositor_epoch,
        }
    }

    fn map_error(error: sws::Error) -> SgfxSinkError {
        match error {
            sws::Error::ServerError(code) => Self::map_error_code(code),
            _ => SgfxSinkError::Protocol,
        }
    }

    fn map_error_code(code: u32) -> SgfxSinkError {
        match code {
            sws_protocol::error_codes::SGFX_UNAVAILABLE => SgfxSinkError::Unavailable,
            sws_protocol::error_codes::SGFX_BUFFER_BUSY => SgfxSinkError::BufferBusy,
            sws_protocol::error_codes::SGFX_IMPORT_FAILED => SgfxSinkError::ImportFailed,
            sws_protocol::error_codes::WINDOW_NOT_OWNED
            | sws_protocol::error_codes::INVALID_SGFX_BUFFER
            | sws_protocol::error_codes::STALE_SGFX_GENERATION => SgfxSinkError::InvalidIdentity,
            _ => SgfxSinkError::Protocol,
        }
    }

    fn event_is_stale(&self, compositor_epoch: u32) -> bool {
        sgfx_event_epoch_is_stale(self.backend_lost, compositor_epoch)
    }

    fn next_commit_token(
        &mut self,
        identity: SgfxBufferIdentity,
    ) -> SgfxSinkResult<SgfxCommitToken> {
        let commit_serial = self.next_commit_serial;
        self.next_commit_serial = commit_serial
            .checked_add(1)
            .ok_or(SgfxSinkError::Protocol)?;
        Ok(SgfxCommitToken {
            identity,
            commit_serial,
        })
    }

    fn take_rejection(&mut self, token: SgfxCommitToken) -> Option<SgfxSinkError> {
        let index = self
            .rejected
            .iter()
            .position(|(candidate, _)| *candidate == token)?;
        Some(self.rejected.remove(index).1)
    }

    fn release_retained(retained: &mut Vec<SgfxCommitToken>, token: SgfxCommitToken) -> bool {
        let Some(index) = retained.iter().position(|candidate| *candidate == token) else {
            return false;
        };
        retained.remove(index);
        true
    }

    fn handle_lifecycle_event(&mut self, event: SwsEvent) {
        match event {
            SwsEvent::SgfxFrameRejected {
                window_id,
                buffer_id,
                generation,
                compositor_epoch,
                commit_serial,
                code,
            } => {
                if self.event_is_stale(compositor_epoch) {
                    return;
                }
                let token = SgfxCommitToken {
                    identity: SgfxBufferIdentity {
                        window_id,
                        buffer_id,
                        generation,
                        compositor_epoch,
                    },
                    commit_serial,
                };
                if Self::release_retained(&mut self.retained, token) {
                    self.rejected.push((token, Self::map_error_code(code)));
                } else {
                    self.lifecycle_error = Some(SgfxSinkError::Protocol);
                }
            }
            SwsEvent::SgfxBufferReleased {
                window_id,
                buffer_id,
                generation,
                compositor_epoch,
                commit_serial,
            } => {
                if self.event_is_stale(compositor_epoch) {
                    return;
                }
                let token = SgfxCommitToken {
                    identity: SgfxBufferIdentity {
                        window_id,
                        buffer_id,
                        generation,
                        compositor_epoch,
                    },
                    commit_serial,
                };
                if !Self::release_retained(&mut self.retained, token) {
                    self.lifecycle_error = Some(SgfxSinkError::Protocol);
                }
            }
            SwsEvent::SgfxBackendLost { compositor_epoch } => {
                if compositor_epoch > self.compositor_epoch {
                    self.backend_lost = Some(compositor_epoch);
                    self.retained.clear();
                    self.rejected.clear();
                    self.lifecycle_error = None;
                }
            }
            _ => {}
        }
    }

    fn pump_lifecycle(&mut self) -> SgfxSinkResult<()> {
        self.conn.dispatch().map_err(Self::map_error)?;
        while let Some(event) = self.events.poll_event() {
            self.handle_lifecycle_event(event);
        }
        if let Some(error) = self.lifecycle_error {
            return Err(error);
        }
        Ok(())
    }

    fn damage_rects(damage: &[DamageRect]) -> SgfxSinkResult<Vec<sws::SgfxDamageRect>> {
        let mut nonempty = damage
            .iter()
            .copied()
            .filter(|(_, _, width, height)| *width > 0 && *height > 0);
        let Some(first) = nonempty.next() else {
            return Ok(Vec::new());
        };

        if damage.len() <= sws_protocol::SGFX_MAX_DAMAGE_RECTS {
            let mut rects = Vec::new();
            rects
                .try_reserve(damage.len())
                .map_err(|_| SgfxSinkError::Protocol)?;
            for (x, y, width, height) in core::iter::once(first).chain(nonempty) {
                rects.push(sws::SgfxDamageRect::new(
                    i32::try_from(x).map_err(|_| SgfxSinkError::Protocol)?,
                    i32::try_from(y).map_err(|_| SgfxSinkError::Protocol)?,
                    width,
                    height,
                ));
            }
            return Ok(rects);
        }

        let (mut left, mut top, first_width, first_height) = first;
        let mut right = left.saturating_add(first_width);
        let mut bottom = top.saturating_add(first_height);
        for (x, y, width, height) in nonempty {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x.saturating_add(width));
            bottom = bottom.max(y.saturating_add(height));
        }
        Ok(vec![sws::SgfxDamageRect::new(
            i32::try_from(left).map_err(|_| SgfxSinkError::Protocol)?,
            i32::try_from(top).map_err(|_| SgfxSinkError::Protocol)?,
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        )])
    }
}

impl SgfxFrameSink for SwsSgfxFrameSink {
    fn window_id(&self) -> u32 {
        self.window_id
    }

    fn status(&mut self) -> SgfxSinkResult<SgfxSinkStatus> {
        self.pump_lifecycle()?;
        if let Some(compositor_epoch) = self.backend_lost {
            Ok(SgfxSinkStatus::BackendLost { compositor_epoch })
        } else {
            Ok(SgfxSinkStatus::Ready {
                compositor_epoch: self.compositor_epoch,
            })
        }
    }

    fn register_shared_image(
        &mut self,
        identity: SgfxBufferIdentity,
        image: ImageRef<'_>,
    ) -> SgfxSinkResult<()> {
        self.pump_lifecycle()?;
        self.validate_identity(identity)?;
        self.conn
            .register_sgfx_buffer(
                Self::client_identity(identity),
                image.width(),
                image.height(),
                image.shared_handle(),
            )
            .map_err(Self::map_error)
    }

    fn wait_until_released(&mut self, token: SgfxCommitToken) -> SgfxSinkResult<()> {
        self.validate_identity(token.identity)?;
        loop {
            self.pump_lifecycle()?;
            if let Some(compositor_epoch) = self.backend_lost {
                return Err(SgfxSinkError::BackendLost { compositor_epoch });
            }
            if let Some(error) = self.take_rejection(token) {
                return Err(error);
            }
            if !self.retained.contains(&token) {
                return Ok(());
            }
            let _ = std::thread::sleep(core::time::Duration::from_millis(1));
        }
    }

    fn commit_shared_image(
        &mut self,
        identity: SgfxBufferIdentity,
        damage: &[DamageRect],
    ) -> SgfxSinkResult<SgfxCommitToken> {
        self.pump_lifecycle()?;
        self.validate_identity(identity)?;
        let damage = Self::damage_rects(damage)?;
        if damage.is_empty() {
            return Err(SgfxSinkError::Protocol);
        }
        if self.retained.iter().any(|token| token.identity == identity) {
            return Err(SgfxSinkError::BufferBusy);
        }
        let token = self.next_commit_token(identity)?;
        self.retained.push(token);
        if let Err(error) = self.conn.commit_sgfx_frame(
            Self::client_identity(identity),
            token.commit_serial,
            &damage,
        ) {
            self.retained.retain(|candidate| *candidate != token);
            return Err(Self::map_error(error));
        }
        Ok(token)
    }

    fn destroy_shared_image(&mut self, identity: SgfxBufferIdentity) -> SgfxSinkResult<()> {
        self.pump_lifecycle()?;
        self.validate_identity(identity)?;
        if self.retained.iter().any(|token| token.identity == identity) {
            return Err(SgfxSinkError::BufferBusy);
        }
        self.conn
            .destroy_sgfx_buffer(Self::client_identity(identity))
            .map_err(Self::map_error)?;
        self.rejected
            .retain(|(token, _)| token.identity != identity);
        Ok(())
    }
}

struct SwsSgfxPaintBackend {
    backend: SgfxPaintBackend<SwsSgfxFrameSink>,
}

impl PaintBackend for SwsSgfxPaintBackend {
    fn resize(&mut self, size: Size, scale_milli: u32) {
        self.backend.resize(size, scale_milli);
    }

    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        background_color: Color,
        _logical_damage: Option<&[Rect]>,
        physical_damage: Option<&[DamageRect]>,
    ) -> Result<BackendFrame<'a>> {
        match self
            .backend
            .render_and_commit(context, background_color, physical_damage)
        {
            Ok(()) => Ok(BackendFrame::External),
            Err(error) => {
                logln!("[ScarletUI SGFX] render failed: {}", error);
                Err(scarlet_ui_core::error::Error::RenderError)
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ClickState {
    last_button: Option<MouseButton>,
    last_x: i32,
    last_y: i32,
    #[cfg(feature = "std")]
    last_time: Option<Instant>,
    #[cfg(not(feature = "std"))]
    last_tick: u64,
    #[cfg(not(feature = "std"))]
    current_tick: u64,
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
            #[cfg(feature = "std")]
            last_time: None,
            #[cfg(not(feature = "std"))]
            last_tick: 0,
            #[cfg(not(feature = "std"))]
            current_tick: 0,
            last_count: 0,
            active_button: None,
            active_count: 1,
        }
    }
}

impl ClickState {
    fn press_count(&mut self, button: MouseButton, x: i32, y: i32) -> u8 {
        let same_button = self.last_button == Some(button);
        let close_enough = (self.last_x - x).abs() <= DOUBLE_CLICK_DISTANCE
            && (self.last_y - y).abs() <= DOUBLE_CLICK_DISTANCE;
        #[cfg(feature = "std")]
        let soon_enough = {
            let now = Instant::now();
            let soon = self
                .last_time
                .is_some_and(|last_time| now.duration_since(last_time) <= DOUBLE_CLICK_THRESHOLD);
            self.last_time = Some(now);
            soon
        };
        #[cfg(not(feature = "std"))]
        let soon_enough = {
            self.current_tick = self.current_tick.saturating_add(1);
            let soon =
                self.current_tick.saturating_sub(self.last_tick) <= DOUBLE_CLICK_EVENT_THRESHOLD;
            self.last_tick = self.current_tick;
            soon
        };
        let count = if same_button && close_enough && soon_enough {
            self.last_count.saturating_add(1).max(1)
        } else {
            1
        };
        self.last_button = Some(button);
        self.last_x = x;
        self.last_y = y;
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
}

#[derive(Clone, Copy, Debug)]
struct TextInputContext {
    context_id: u32,
    serial: u32,
    enabled: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct PendingWheelDelta {
    discrete_x: i32,
    discrete_y: i32,
    hi_res_x: i32,
    hi_res_y: i32,
    has_discrete_x: bool,
    has_discrete_y: bool,
    has_hi_res_x: bool,
    has_hi_res_y: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PendingRelativeMotion {
    dx: i32,
    dy: i32,
}

impl PendingRelativeMotion {
    fn push_input(&mut self, type_: u16, code: u16, value: i32, pointer_locked: bool) -> bool {
        if !pointer_locked || type_ != event_type::EV_REL {
            return false;
        }
        match code {
            rel_code::REL_X => self.dx = self.dx.saturating_add(value),
            rel_code::REL_Y => self.dy = self.dy.saturating_add(value),
            _ => return false,
        }
        true
    }

    fn take(&mut self) -> Option<(i32, i32)> {
        if self.dx == 0 && self.dy == 0 {
            return None;
        }
        Some((core::mem::take(&mut self.dx), core::mem::take(&mut self.dy)))
    }
}

fn pointer_lock_supported(capabilities: u64) -> bool {
    capabilities & sws_protocol::capabilities::POINTER_LOCK != 0
}

fn update_pointer_lock_state(current: &mut bool, locked: bool) -> bool {
    if *current == locked {
        false
    } else {
        *current = locked;
        true
    }
}

fn apply_pointer_lock_confirmation(
    current: &mut bool,
    requested: &mut Option<bool>,
    locked: bool,
) -> bool {
    let had_pending_request = requested.take().is_some();
    update_pointer_lock_state(current, locked) || had_pending_request
}

impl PendingWheelDelta {
    fn add_discrete_x(&mut self, value: i32) {
        self.discrete_x = self.discrete_x.saturating_add(value);
        self.has_discrete_x = true;
    }

    fn add_discrete_y(&mut self, value: i32) {
        self.discrete_y = self.discrete_y.saturating_add(value);
        self.has_discrete_y = true;
    }

    fn add_hi_res_x(&mut self, value: i32) {
        self.hi_res_x = self.hi_res_x.saturating_add(value);
        self.has_hi_res_x = true;
    }

    fn add_hi_res_y(&mut self, value: i32) {
        self.hi_res_y = self.hi_res_y.saturating_add(value);
        self.has_hi_res_y = true;
    }

    fn take_normalized(&mut self) -> Option<(i32, i32)> {
        let delta_x = normalized_wheel_axis(
            self.discrete_x,
            self.has_discrete_x,
            self.hi_res_x,
            self.has_hi_res_x,
        );
        let delta_y = normalized_wheel_axis(
            self.discrete_y,
            self.has_discrete_y,
            self.hi_res_y,
            self.has_hi_res_y,
        );
        *self = Self::default();

        if delta_x == 0 && delta_y == 0 {
            None
        } else {
            Some((delta_x, delta_y))
        }
    }

    fn is_empty(&self) -> bool {
        !self.has_discrete_x && !self.has_discrete_y && !self.has_hi_res_x && !self.has_hi_res_y
    }
}

/// SWS platform window implementation
pub struct SWSPlatformWindow {
    conn: sws::Connection,
    event_receiver: sws::EventReceiver,
    surface_id: u32,
    requested_renderer_backend: RequestedRendererBackend,
    renderer_backend: RendererBackendKind,
    compositor_backend: CompositorBackendKind,
    scale_milli: u32,
    current_size: Size,
    window_geometry_insets: EdgeInsets,
    window_geometry_supported: bool,
    fullscreen: bool,
    pointer_locked: bool,
    pointer_lock_requested: Option<bool>,
    pending_events: Vec<Event>,
    pending_head: usize,
    pointer_x: i32,
    pointer_y: i32,
    pending_move: bool,
    left_shift_pressed: bool,
    right_shift_pressed: bool,
    left_control_pressed: bool,
    right_control_pressed: bool,
    left_alt_pressed: bool,
    right_alt_pressed: bool,
    left_super_pressed: bool,
    right_super_pressed: bool,
    click_state: ClickState,
    text_input: Option<TextInputContext>,
    pending_wheel: PendingWheelDelta,
    pending_relative: PendingRelativeMotion,
    needs_full_present: bool,
    transport_failed: bool,
    quit_queued: bool,
}

fn normalized_wheel_axis(discrete: i32, has_discrete: bool, hi_res: i32, has_hi_res: bool) -> i32 {
    if has_hi_res {
        normalize_hi_res_wheel_delta(hi_res)
    } else if has_discrete {
        normalize_discrete_wheel_delta(discrete)
    } else {
        0
    }
}

fn normalize_hi_res_wheel_delta(value: i32) -> i32 {
    scale_i32_round(value, WHEEL_LINE_DELTA, WHEEL_HI_RES_UNITS_PER_NOTCH)
}

fn normalize_discrete_wheel_delta(value: i32) -> i32 {
    let detents = if value.unsigned_abs() >= SWS_LEGACY_WHEEL_PIXELS_PER_NOTCH as u32
        && value % SWS_LEGACY_WHEEL_PIXELS_PER_NOTCH == 0
    {
        value / SWS_LEGACY_WHEEL_PIXELS_PER_NOTCH
    } else {
        value
    };
    detents.saturating_mul(WHEEL_LINE_DELTA)
}

fn scale_i32_round(value: i32, numerator: i32, denominator: i32) -> i32 {
    debug_assert!(denominator > 0);
    let scaled = (value as i64).saturating_mul(numerator as i64);
    let divisor = denominator as i64;
    let rounded = if scaled >= 0 {
        scaled.saturating_add(divisor / 2) / divisor
    } else {
        scaled.saturating_sub(divisor / 2) / divisor
    };
    saturate_i64_to_i32(rounded)
}

fn saturate_i64_to_i32(value: i64) -> i32 {
    if value < i32::MIN as i64 {
        i32::MIN
    } else if value > i32::MAX as i64 {
        i32::MAX
    } else {
        value as i32
    }
}

/// Scarlet Window Server backend.
pub struct SwsBackend {
    connection: Option<sws::Connection>,
}

impl Default for SwsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SwsBackend {
    /// Create an SWS backend.
    ///
    /// # Returns
    ///
    /// A backend that creates SWS platform windows.
    pub fn new() -> Self {
        Self { connection: None }
    }

    fn connection(&mut self) -> Result<sws::Connection> {
        if self.connection.is_none() {
            self.connection = Some(
                sws::Connection::connect("/tmp/sws.sock")
                    .map_err(|_| scarlet_ui_core::error::Error::ConnectionFailed)?,
            );
        }
        self.connection
            .as_ref()
            .cloned()
            .ok_or(scarlet_ui_core::error::Error::ConnectionFailed)
    }
}

impl PlatformBackend for SwsBackend {
    fn window_defaults(&mut self) -> PlatformWindowDefaults {
        PlatformWindowDefaults::new(true)
    }

    fn initial_input_environment(&mut self) -> InputEnvironment {
        self.connection()
            .ok()
            .and_then(|connection| connection.get_input_environment().ok())
            .map(map_sws_input_environment)
            .unwrap_or_default()
    }

    fn output_scale_milli(&mut self) -> u32 {
        let Ok(conn) = self.connection() else {
            return DEFAULT_SCALE_MILLI;
        };
        conn.get_output_scale()
            .map(SWSPlatformWindow::sanitize_scale)
            .unwrap_or(DEFAULT_SCALE_MILLI)
    }

    fn create_window(&mut self, request: WindowCreateRequest) -> Result<Box<dyn PlatformWindow>> {
        validate_window_decoration(request.decoration)?;
        let conn = self.connection()?;
        Ok(Box::new(
            SWSPlatformWindow::create_with_connection_and_policies(
                conn,
                &request.app_id,
                &request.title,
                request.size,
                request.window_type,
                &request.menu_titles,
                request.focus_on_create,
                request.active_on_focus,
                request.opaque,
                request.placement,
                request.window_geometry_insets,
            )?,
        ))
    }
}

fn map_sws_input_environment(environment: sws::InputEnvironment) -> InputEnvironment {
    InputEnvironment::new(
        environment.generation.into(),
        environment.tablet_mode(),
        environment.lid_closed(),
        environment.has_direct_touch(),
        environment.has_fine_pointer(),
        environment.has_keyboard(),
        environment.has_pen(),
    )
    .with_system_mode(
        environment.windowing_mode().map(|mode| match mode {
            sws::WindowingMode::Freeform => WindowingMode::Freeform,
            sws::WindowingMode::Focused => WindowingMode::Focused,
        }),
        environment.tablet_mode_override_active(),
        environment.windowing_mode_override_active(),
    )
}

fn validate_window_decoration(decoration: WindowDecoration) -> Result<()> {
    if decoration.frame.is_system() || decoration.title_bar.is_system() {
        Err(Error::WindowDecorationUnsupported)
    } else {
        Ok(())
    }
}

impl SWSPlatformWindow {
    fn sanitize_scale(scale_milli: u32) -> u32 {
        scale_milli.max(1)
    }

    pub fn query_output_scale() -> u32 {
        let Ok(conn) = sws::Connection::connect("/tmp/sws.sock") else {
            return DEFAULT_SCALE_MILLI;
        };
        conn.get_output_scale()
            .map(Self::sanitize_scale)
            .unwrap_or(DEFAULT_SCALE_MILLI)
    }

    fn logical_to_physical_len_with_scale(value: u32, scale_milli: u32) -> u32 {
        ((value as u64)
            .saturating_mul(scale_milli as u64)
            .saturating_add(999)
            / 1000)
            .max(1) as u32
    }

    fn logical_to_physical_len(&self, value: u32) -> u32 {
        Self::logical_to_physical_len_with_scale(value, self.scale_milli)
    }

    fn logical_to_physical_inset_with_scale(value: f32, scale_milli: u32) -> u32 {
        if !value.is_finite() || value <= 0.0 {
            return 0;
        }
        let scaled = value * scale_milli.max(1) as f32 / 1000.0;
        if scaled >= i32::MAX as f32 {
            i32::MAX as u32
        } else {
            (scaled + 0.5) as u32
        }
    }

    fn physical_window_geometry(
        insets: EdgeInsets,
        surface_width: u32,
        surface_height: u32,
        scale_milli: u32,
    ) -> Result<Option<sws::WindowGeometry>> {
        let left = Self::logical_to_physical_inset_with_scale(insets.left, scale_milli);
        let top = Self::logical_to_physical_inset_with_scale(insets.top, scale_milli);
        let right = Self::logical_to_physical_inset_with_scale(insets.right, scale_milli);
        let bottom = Self::logical_to_physical_inset_with_scale(insets.bottom, scale_milli);
        if left == 0 && top == 0 && right == 0 && bottom == 0 {
            return Ok(None);
        }

        let horizontal = left
            .checked_add(right)
            .filter(|total| *total < surface_width)
            .ok_or(Error::InvalidSize {
                width: surface_width,
                height: surface_height,
            })?;
        let vertical = top
            .checked_add(bottom)
            .filter(|total| *total < surface_height)
            .ok_or(Error::InvalidSize {
                width: surface_width,
                height: surface_height,
            })?;
        Ok(Some(sws::WindowGeometry {
            x: left as i32,
            y: top as i32,
            width: surface_width - horizontal,
            height: surface_height - vertical,
        }))
    }

    fn logical_managed_size(surface_size: Size, insets: EdgeInsets) -> Size {
        Size::new(
            (surface_size.width - insets.left - insets.right).max(1.0),
            (surface_size.height - insets.top - insets.bottom).max(1.0),
        )
    }

    fn logical_surface_size_for_managed(width: u32, height: u32, insets: EdgeInsets) -> (u32, u32) {
        fn ceil_surface_length(value: f32) -> u32 {
            let truncated = value as u32;
            if truncated == 0 {
                1
            } else if truncated < u32::MAX && (truncated as f32) < value {
                truncated + 1
            } else {
                truncated
            }
        }

        (
            ceil_surface_length(width as f32 + insets.left + insets.right),
            ceil_surface_length(height as f32 + insets.top + insets.bottom),
        )
    }

    fn physical_to_logical_len(&self, value: u32) -> u32 {
        ((value as u64)
            .saturating_mul(1000)
            .saturating_add(self.scale_milli as u64 - 1)
            / self.scale_milli as u64)
            .max(1) as u32
    }

    fn logical_to_physical_pos(&self, value: i32) -> i32 {
        Self::logical_to_physical_pos_with_scale(value, self.scale_milli)
    }

    fn logical_to_physical_pos_with_scale(value: i32, scale_milli: u32) -> i32 {
        ((value as i64).saturating_mul(scale_milli as i64) / 1000) as i32
    }

    fn physical_to_logical_pos(&self, value: i32) -> i32 {
        ((value as i64).saturating_mul(1000) / self.scale_milli as i64) as i32
    }

    /// Get the connection
    pub fn connection(&self) -> &sws::Connection {
        &self.conn
    }

    /// Get mutable reference to the connection
    pub fn connection_mut(&mut self) -> &mut sws::Connection {
        &mut self.conn
    }

    /// Create a new platform window with a specific window type
    pub fn create_with_type(
        app_id: &str,
        title: &str,
        size: Size,
        window_type: u32,
    ) -> Result<Self> {
        Self::create_with_type_and_menu_and_policies(
            app_id,
            title,
            size,
            window_type,
            "",
            true,
            window_type == sws_protocol::window_types::NORMAL,
            true,
            WindowPlacement::Default,
        )
    }

    /// Create a new platform window with a specific window type and initial menu titles
    pub fn create_with_type_and_menu(
        app_id: &str,
        title: &str,
        size: Size,
        window_type: u32,
        menu_titles: &str,
    ) -> Result<Self> {
        Self::create_with_type_and_menu_and_policies(
            app_id,
            title,
            size,
            window_type,
            menu_titles,
            true,
            window_type == sws_protocol::window_types::NORMAL,
            true,
            WindowPlacement::Default,
        )
    }

    /// Create a new platform window with a specific window type, menu titles, and focus policies
    pub fn create_with_type_and_menu_and_policies(
        app_id: &str,
        title: &str,
        size: Size,
        window_type: u32,
        menu_titles: &str,
        focus_on_create: bool,
        active_on_focus: bool,
        opaque: bool,
        placement: WindowPlacement,
    ) -> Result<Self> {
        let conn = sws::Connection::connect("/tmp/sws.sock")
            .map_err(|_| scarlet_ui_core::error::Error::ConnectionFailed)?;
        Self::create_with_connection_and_policies(
            conn,
            app_id,
            title,
            size,
            window_type,
            menu_titles,
            focus_on_create,
            active_on_focus,
            opaque,
            placement,
            EdgeInsets::ZERO,
        )
    }

    fn create_with_connection_and_policies(
        conn: sws::Connection,
        app_id: &str,
        title: &str,
        size: Size,
        window_type: u32,
        menu_titles: &str,
        focus_on_create: bool,
        active_on_focus: bool,
        opaque: bool,
        placement: WindowPlacement,
        window_geometry_insets: EdgeInsets,
    ) -> Result<Self> {
        let activation_token = take_launch_activation_token(window_type);
        let requested_renderer_backend = RequestedRendererBackend::from_environment()?;
        let window_geometry_supported = conn
            .get_capabilities()
            .is_ok_and(|capabilities| capabilities.supports_window_geometry());
        let scale_milli = conn
            .get_output_scale()
            .map(Self::sanitize_scale)
            .unwrap_or(DEFAULT_SCALE_MILLI);
        let physical_width =
            Self::logical_to_physical_len_with_scale(size.width.max(1.0) as u32, scale_milli);
        let physical_height =
            Self::logical_to_physical_len_with_scale(size.height.max(1.0) as u32, scale_milli);

        // Create the surface with the placement hint in the same request so
        // the compositor can apply its policy before the first frame.
        let physical_geometry = Self::physical_window_geometry(
            window_geometry_insets,
            physical_width,
            physical_height,
            scale_milli,
        )?;
        let placement = match placement {
            WindowPlacement::Default => sws_protocol::WindowPlacement::Default,
            WindowPlacement::Centered => sws_protocol::WindowPlacement::Centered,
            WindowPlacement::At { x, y } => {
                let mut x = Self::logical_to_physical_pos_with_scale(x, scale_milli);
                let mut y = Self::logical_to_physical_pos_with_scale(y, scale_milli);
                if !window_geometry_supported && let Some(geometry) = physical_geometry {
                    x = x.saturating_sub(geometry.x);
                    y = y.saturating_sub(geometry.y);
                }
                sws_protocol::WindowPlacement::Absolute { x, y }
            }
        };
        let surface_id = if let Some(activation_token) = activation_token {
            conn.create_surface_with_type_and_policies_with_activation_token(
                app_id,
                title,
                menu_titles,
                physical_width,
                physical_height,
                window_type,
                true,
                focus_on_create,
                active_on_focus,
                placement,
                &activation_token,
            )
        } else {
            conn.create_surface_with_type_and_policies_with_placement(
                app_id,
                title,
                menu_titles,
                physical_width,
                physical_height,
                window_type,
                true,
                focus_on_create,
                active_on_focus,
                placement,
            )
        }
        .map_err(|_| scarlet_ui_core::error::Error::SurfaceCreationFailed)?;

        if !opaque {
            conn.set_window_has_alpha_content(surface_id, true)
                .map_err(|_| scarlet_ui_core::error::Error::IoError)?;
        }
        if window_geometry_supported
            && let Some(geometry) = physical_geometry
            && conn.set_window_geometry(surface_id, geometry).is_err()
        {
            let _ = conn.destroy_surface(surface_id);
            return Err(scarlet_ui_core::error::Error::IoError);
        }
        let event_receiver = conn.subscribe_window_events(surface_id);

        Ok(Self {
            conn,
            event_receiver,
            surface_id,
            requested_renderer_backend,
            renderer_backend: RendererBackendKind::Cpu,
            compositor_backend: CompositorBackendKind::Unknown,
            scale_milli,
            current_size: size,
            window_geometry_insets,
            window_geometry_supported,
            fullscreen: false,
            pointer_locked: false,
            pointer_lock_requested: None,
            pending_events: Vec::new(),
            pending_head: 0,
            pointer_x: 0,
            pointer_y: 0,
            pending_move: false,
            left_shift_pressed: false,
            right_shift_pressed: false,
            left_control_pressed: false,
            right_control_pressed: false,
            left_alt_pressed: false,
            right_alt_pressed: false,
            left_super_pressed: false,
            right_super_pressed: false,
            click_state: ClickState::default(),
            text_input: None,
            pending_wheel: PendingWheelDelta::default(),
            pending_relative: PendingRelativeMotion::default(),
            needs_full_present: false,
            transport_failed: false,
            quit_queued: false,
        })
    }

    pub fn new_with_menu(app_id: &str, title: &str, size: Size, menu_titles: &str) -> Result<Self> {
        Self::create_with_type_and_menu_and_policies(
            app_id,
            title,
            size,
            sws_protocol::window_types::NORMAL,
            menu_titles,
            true,
            true,
            true,
            WindowPlacement::Default,
        )
    }

    pub fn new_with_menu_and_policies(
        app_id: &str,
        title: &str,
        size: Size,
        menu_titles: &str,
        focus_on_create: bool,
        active_on_focus: bool,
        opaque: bool,
    ) -> Result<Self> {
        Self::create_with_type_and_menu_and_policies(
            app_id,
            title,
            size,
            sws_protocol::window_types::NORMAL,
            menu_titles,
            focus_on_create,
            active_on_focus,
            opaque,
            WindowPlacement::Default,
        )
    }

    fn sanitize_menu_titles(menu_titles: &str) -> &str {
        if menu_titles
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
        {
            ""
        } else {
            menu_titles
        }
    }

    fn push_event(&mut self, event: Event) {
        // Coalesce consecutive mouse-move events to reduce work.
        if let Event::Mouse(MouseEvent::Moved { .. }) = event {
            if let Some(last) = self.pending_events.last_mut() {
                if let Event::Mouse(MouseEvent::Moved { x, y }) = last {
                    if let Event::Mouse(MouseEvent::Moved { x: new_x, y: new_y }) = event {
                        *x = new_x;
                        *y = new_y;
                        return;
                    }
                }
            }
        }
        self.pending_events.push(event);
    }

    fn queue_quit(&mut self) {
        if !self.quit_queued {
            self.quit_queued = true;
            self.push_event(Event::Quit);
        }
    }

    fn mark_transport_failed(&mut self) {
        if self.transport_failed {
            return;
        }
        self.transport_failed = true;
        self.pending_events.clear();
        self.pending_head = 0;
        logln!(
            "[SWSPlatformWindow] transport failed for surface {}",
            self.surface_id
        );
        self.queue_quit();
    }

    pub fn sync_text_input(&mut self, state: Option<&TextInputElementState>) {
        if self.transport_failed {
            return;
        }

        let Some(state) = state else {
            if let Some(context) = self.text_input.as_mut()
                && context.enabled
            {
                if self.conn.disable_text_input(context.context_id).is_ok() {
                    context.enabled = false;
                }
            }
            return;
        };

        if self.text_input.is_none() {
            match self.conn.create_text_input_context(self.surface_id, 0) {
                Ok((context_id, serial)) => {
                    self.text_input = Some(TextInputContext {
                        context_id,
                        serial,
                        enabled: false,
                    });
                }
                Err(_) => return,
            }
        }

        let Some(context) = self.text_input else {
            return;
        };
        let context_id = context.context_id;
        let cursor_rect = self.logical_rect_to_physical(state.cursor_rect);
        let _ = self.conn.set_text_input_cursor_rect(
            context_id,
            cursor_rect.origin.x as i32,
            cursor_rect.origin.y as i32,
            cursor_rect.size.width.max(1.0) as u32,
            cursor_rect.size.height.max(1.0) as u32,
        );
        let _ = self.conn.set_text_input_surrounding_text(
            context_id,
            state.cursor_byte,
            state.anchor_byte,
            &state.surrounding_text,
        );
        let _ = self.conn.set_text_input_content_type(
            context_id,
            sws_protocol::text_input_content_hints::NONE,
            sws_protocol::text_input_content_purpose::NORMAL,
        );
        if self
            .conn
            .commit_text_input_state(context_id, context.serial)
            .is_ok()
        {
            if let Some(context) = self.text_input.as_mut() {
                context.serial = context.serial.saturating_add(1);
            }
        }

        if !context.enabled && self.conn.enable_text_input(context_id).is_ok() {
            if let Some(context) = self.text_input.as_mut() {
                context.enabled = true;
            }
        }
    }

    /// Set a text-input cursor rectangle using ScarletUI logical coordinates.
    pub fn set_text_input_cursor_rect(
        &mut self,
        context_id: u32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> core::result::Result<(), sws::Error> {
        let cursor_rect = self.logical_rect_to_physical(Rect::from_xywh(
            x as f32,
            y as f32,
            width.max(1) as f32,
            height.max(1) as f32,
        ));
        self.conn.set_text_input_cursor_rect(
            context_id,
            cursor_rect.origin.x as i32,
            cursor_rect.origin.y as i32,
            cursor_rect.size.width.max(1.0) as u32,
            cursor_rect.size.height.max(1.0) as u32,
        )
    }

    fn logical_rect_to_physical(&self, rect: Rect) -> Rect {
        Rect::from_xywh(
            self.logical_to_physical_pos(rect.origin.x as i32) as f32,
            self.logical_to_physical_pos(rect.origin.y as i32) as f32,
            self.logical_to_physical_len(rect.size.width.max(1.0) as u32) as f32,
            self.logical_to_physical_len(rect.size.height.max(1.0) as u32) as f32,
        )
    }

    fn copy_buffer_region(
        buffer: &Buffer,
        dst_data: &mut [u8],
        dst_width: u32,
        dst_height: u32,
        damage: DamageRect,
    ) -> Option<DamageRect> {
        let (x, y, width, height) = damage;
        let x = x.min(dst_width);
        let y = y.min(dst_height);
        let width = width.min(dst_width.saturating_sub(x));
        let height = height.min(dst_height.saturating_sub(y));
        if width == 0 || height == 0 {
            return None;
        }

        let src_data = buffer.data();
        let src_width = buffer.width() as usize;
        let src_height = buffer.height() as usize;
        let dst_width = dst_width as usize;
        let x_start = x as usize;
        let x_end = x.saturating_add(width) as usize;
        let clear_len = width as usize * 4;

        for row in y as usize..y.saturating_add(height) as usize {
            let dst_offset = row
                .saturating_mul(dst_width)
                .saturating_add(x_start)
                .saturating_mul(4);
            let dst_row_end = dst_offset.saturating_add(clear_len).min(dst_data.len());
            if dst_offset >= dst_row_end {
                continue;
            }

            if row >= src_height || x_start >= src_width {
                dst_data[dst_offset..dst_row_end].fill(0);
                continue;
            }

            let copy_x_end = x_end.min(src_width);
            let copy_len = copy_x_end.saturating_sub(x_start).saturating_mul(4);
            if copy_len > 0 {
                let src_offset = row
                    .saturating_mul(src_width)
                    .saturating_add(x_start)
                    .saturating_mul(4);
                let src_end = src_offset.saturating_add(copy_len).min(src_data.len());
                let dst_copy_end = dst_offset.saturating_add(src_end.saturating_sub(src_offset));
                if src_offset < src_end && dst_copy_end <= dst_data.len() {
                    dst_data[dst_offset..dst_copy_end]
                        .copy_from_slice(&src_data[src_offset..src_end]);
                }
            }

            if copy_x_end < x_end {
                let clear_start = dst_offset.saturating_add(copy_len);
                if clear_start < dst_row_end {
                    dst_data[clear_start..dst_row_end].fill(0);
                }
            }
        }

        Some((x, y, width, height))
    }

    fn map_key_code(code: u16) -> KeyCode {
        match code {
            key_code::KEY_ESC => KeyCode::Escape,
            key_code::KEY_ENTER => KeyCode::Enter,
            key_code::KEY_TAB => KeyCode::Tab,
            key_code::KEY_BACKSPACE => KeyCode::Backspace,
            key_code::KEY_SPACE => KeyCode::Space,
            key_code::KEY_LEFT => KeyCode::Left,
            key_code::KEY_RIGHT => KeyCode::Right,
            key_code::KEY_UP => KeyCode::Up,
            key_code::KEY_DOWN => KeyCode::Down,
            key_code::KEY_HOME => KeyCode::Home,
            key_code::KEY_END => KeyCode::End,
            key_code::KEY_PAGEUP => KeyCode::PageUp,
            key_code::KEY_PAGEDOWN => KeyCode::PageDown,
            key_code::KEY_INSERT => KeyCode::Insert,
            key_code::KEY_DELETE => KeyCode::Delete,
            key_code::KEY_F1 => KeyCode::F(1),
            key_code::KEY_F2 => KeyCode::F(2),
            key_code::KEY_F3 => KeyCode::F(3),
            key_code::KEY_F4 => KeyCode::F(4),
            key_code::KEY_F5 => KeyCode::F(5),
            key_code::KEY_F6 => KeyCode::F(6),
            key_code::KEY_F7 => KeyCode::F(7),
            key_code::KEY_F8 => KeyCode::F(8),
            key_code::KEY_F9 => KeyCode::F(9),
            key_code::KEY_F10 => KeyCode::F(10),
            key_code::KEY_F11 => KeyCode::F(11),
            key_code::KEY_F12 => KeyCode::F(12),
            _ => Self::map_key_char(code).map_or(KeyCode::Unknown, KeyCode::Char),
        }
    }

    fn map_key_char(code: u16) -> Option<char> {
        match code {
            key_code::KEY_1 => Some('1'),
            key_code::KEY_2 => Some('2'),
            key_code::KEY_3 => Some('3'),
            key_code::KEY_4 => Some('4'),
            key_code::KEY_5 => Some('5'),
            key_code::KEY_6 => Some('6'),
            key_code::KEY_7 => Some('7'),
            key_code::KEY_8 => Some('8'),
            key_code::KEY_9 => Some('9'),
            key_code::KEY_0 => Some('0'),
            key_code::KEY_Q => Some('q'),
            key_code::KEY_W => Some('w'),
            key_code::KEY_E => Some('e'),
            key_code::KEY_R => Some('r'),
            key_code::KEY_T => Some('t'),
            key_code::KEY_Y => Some('y'),
            key_code::KEY_U => Some('u'),
            key_code::KEY_I => Some('i'),
            key_code::KEY_O => Some('o'),
            key_code::KEY_P => Some('p'),
            key_code::KEY_A => Some('a'),
            key_code::KEY_S => Some('s'),
            key_code::KEY_D => Some('d'),
            key_code::KEY_F => Some('f'),
            key_code::KEY_G => Some('g'),
            key_code::KEY_H => Some('h'),
            key_code::KEY_J => Some('j'),
            key_code::KEY_K => Some('k'),
            key_code::KEY_L => Some('l'),
            key_code::KEY_Z => Some('z'),
            key_code::KEY_X => Some('x'),
            key_code::KEY_C => Some('c'),
            key_code::KEY_V => Some('v'),
            key_code::KEY_B => Some('b'),
            key_code::KEY_N => Some('n'),
            key_code::KEY_M => Some('m'),
            key_code::KEY_COMMA => Some(','),
            key_code::KEY_DOT => Some('.'),
            key_code::KEY_SLASH => Some('/'),
            key_code::KEY_SEMICOLON => Some(';'),
            key_code::KEY_APOSTROPHE => Some('\''),
            KEY_GRAVE => Some('`'),
            key_code::KEY_LEFTBRACE => Some('['),
            key_code::KEY_RIGHTBRACE => Some(']'),
            key_code::KEY_BACKSLASH => Some('\\'),
            key_code::KEY_MINUS => Some('-'),
            key_code::KEY_EQUAL => Some('='),
            key_code::KEY_SPACE => Some(' '),
            _ => None,
        }
    }

    fn is_modifier_key(code: u16) -> bool {
        matches!(
            code,
            KEY_LEFTSHIFT
                | KEY_RIGHTSHIFT
                | KEY_LEFTCTRL
                | KEY_RIGHTCTRL
                | KEY_LEFTALT
                | KEY_RIGHTALT
                | KEY_LEFTMETA
                | KEY_RIGHTMETA
        )
    }

    fn update_modifier_state(&mut self, code: u16, pressed: bool) {
        match code {
            KEY_LEFTSHIFT => self.left_shift_pressed = pressed,
            KEY_RIGHTSHIFT => self.right_shift_pressed = pressed,
            KEY_LEFTCTRL => self.left_control_pressed = pressed,
            KEY_RIGHTCTRL => self.right_control_pressed = pressed,
            KEY_LEFTALT => self.left_alt_pressed = pressed,
            KEY_RIGHTALT => self.right_alt_pressed = pressed,
            KEY_LEFTMETA => self.left_super_pressed = pressed,
            KEY_RIGHTMETA => self.right_super_pressed = pressed,
            _ => {}
        }
    }

    fn reset_transient_modifiers(&mut self) {
        self.left_shift_pressed = false;
        self.right_shift_pressed = false;
        self.left_control_pressed = false;
        self.right_control_pressed = false;
        self.left_alt_pressed = false;
        self.right_alt_pressed = false;
        self.left_super_pressed = false;
        self.right_super_pressed = false;
    }

    fn shift_pressed(&self) -> bool {
        self.left_shift_pressed || self.right_shift_pressed
    }

    fn control_pressed(&self) -> bool {
        self.left_control_pressed || self.right_control_pressed
    }

    fn current_modifiers(&self) -> KeyModifiers {
        KeyModifiers {
            shift: self.shift_pressed(),
            control: self.control_pressed(),
            alt: self.left_alt_pressed || self.right_alt_pressed,
            super_key: self.left_super_pressed || self.right_super_pressed,
        }
    }

    fn push_mouse_button_event(&mut self, button: MouseButton, pressed: bool) {
        let x = self.pointer_x;
        let y = self.pointer_y;
        let click_count = if pressed {
            self.click_state.press_count(button, x, y)
        } else {
            self.click_state.release_count(button)
        };
        let event = if pressed {
            MouseEvent::ButtonPressed {
                button,
                x,
                y,
                click_count,
            }
        } else {
            MouseEvent::ButtonReleased {
                button,
                x,
                y,
                click_count,
            }
        };
        self.push_event(Event::Mouse(event));
    }

    fn map_key_char_with_modifiers(&self, code: u16) -> Option<char> {
        let base = Self::map_key_char(code)?;
        if self.control_pressed() {
            return match base {
                'a'..='z' => Some((base as u8 - b'a' + 1) as char),
                '[' => Some(0x1b as char),
                '\\' => Some(0x1c as char),
                ']' => Some(0x1d as char),
                '-' | '_' => Some(0x1f as char),
                _ => None,
            };
        }

        if base.is_ascii_lowercase() {
            if self.shift_pressed() {
                return Some(base.to_ascii_uppercase());
            }
            return Some(base);
        }

        if self.shift_pressed() {
            return match base {
                '1' => Some('!'),
                '2' => Some('@'),
                '3' => Some('#'),
                '4' => Some('$'),
                '5' => Some('%'),
                '6' => Some('^'),
                '7' => Some('&'),
                '8' => Some('*'),
                '9' => Some('('),
                '0' => Some(')'),
                '-' => Some('_'),
                '=' => Some('+'),
                '[' => Some('{'),
                ']' => Some('}'),
                '\\' => Some('|'),
                ';' => Some(':'),
                '\'' => Some('"'),
                '`' => Some('~'),
                ',' => Some('<'),
                '.' => Some('>'),
                '/' => Some('?'),
                _ => Some(base),
            };
        }

        Some(base)
    }
}

impl PlatformWindow for SWSPlatformWindow {
    fn new(app_id: &str, title: &str, size: Size) -> Result<Self> {
        Self::create_with_type_and_menu_and_policies(
            app_id,
            title,
            size,
            sws_protocol::window_types::NORMAL,
            "",
            true,
            true,
            true,
            WindowPlacement::Default,
        )
    }

    fn poll_event(&mut self) -> Option<Event> {
        let debug = scarlet_ui_core::debug::is_enabled();
        if self.pending_head >= self.pending_events.len() {
            self.pending_events.clear();
            self.pending_head = 0;
        }

        if !self.transport_failed && self.conn.dispatch().is_err() {
            self.mark_transport_failed();
        }

        if !self.transport_failed {
            while let Some(ev) = self.event_receiver.poll_event() {
                self.handle_sws_event(ev);
            }
            self.flush_pending_relative_motion();
        }

        if self.pending_head < self.pending_events.len() {
            let ev = self.pending_events[self.pending_head].clone();
            self.pending_head += 1;
            if self.pending_head >= self.pending_events.len() {
                self.pending_events.clear();
                self.pending_head = 0;
            }
            if debug {
                logln!("[SWSPlatformWindow] poll_event: {:?}", ev);
            }
            Some(ev)
        } else {
            None
        }
    }

    fn output_scale_milli(&self) -> u32 {
        self.scale_milli
    }

    fn renderer_backend(&self) -> RendererBackendKind {
        self.renderer_backend
    }

    fn compositor_backend(&self) -> CompositorBackendKind {
        self.compositor_backend
    }

    fn take_paint_backend(&mut self) -> Result<Option<Box<dyn PaintBackend>>> {
        let capabilities = match self.conn.get_capabilities() {
            Ok(capabilities) => {
                self.compositor_backend = match capabilities.compositor_backend {
                    sws_protocol::compositor_backends::CPU => CompositorBackendKind::Cpu,
                    sws_protocol::compositor_backends::SGFX => CompositorBackendKind::Sgfx,
                    _ => CompositorBackendKind::Unknown,
                };
                Some(capabilities)
            }
            Err(_) => {
                self.compositor_backend = CompositorBackendKind::Unknown;
                None
            }
        };

        if self.requested_renderer_backend == RequestedRendererBackend::Cpu {
            self.renderer_backend = RendererBackendKind::Cpu;
            return Ok(None);
        }

        let shared_sgfx_available = capabilities.is_some_and(|capabilities| {
            capabilities.protocol_version == sws_protocol::SWS_PROTOCOL_VERSION
                && capabilities.capabilities & sws_protocol::capabilities::SGFX_SHARED_IMAGE != 0
                && capabilities.compositor_backend == sws_protocol::compositor_backends::SGFX
                && capabilities.compositor_epoch != 0
        });
        if !shared_sgfx_available {
            self.renderer_backend = RendererBackendKind::Cpu;
            return match self.requested_renderer_backend {
                RequestedRendererBackend::Auto => Ok(None),
                RequestedRendererBackend::Sgfx => Err(scarlet_ui_core::error::Error::RenderError),
                RequestedRendererBackend::Cpu => Ok(None),
            };
        }

        let Some(capabilities) = capabilities else {
            return Err(scarlet_ui_core::error::Error::RenderError);
        };
        let sink = SwsSgfxFrameSink::new(
            self.conn.clone(),
            self.surface_id,
            capabilities.compositor_epoch,
        );
        match SgfxPaintBackend::new(sink, self.current_size, self.scale_milli) {
            Ok(backend) => {
                logln!(
                    "[ScarletUI] platform-sws renderer=sgfx backend={}",
                    backend.backend_kind()
                );
                self.renderer_backend = RendererBackendKind::Sgfx;
                Ok(Some(Box::new(SwsSgfxPaintBackend { backend })))
            }
            Err(error)
                if self.requested_renderer_backend == RequestedRendererBackend::Auto
                    && !sgfx_backend_override_requested() =>
            {
                logln!("[ScarletUI SGFX] initialization failed: {}", error);
                self.renderer_backend = RendererBackendKind::Cpu;
                Ok(None)
            }
            Err(error) => {
                logln!("[ScarletUI SGFX] initialization failed: {}", error);
                Err(scarlet_ui_core::error::Error::RenderError)
            }
        }
    }

    fn present(&mut self, buffer: &Buffer) {
        self.present_with_damage(buffer, None);
    }

    fn present_with_damage(&mut self, buffer: &Buffer, damage: Option<&[DamageRect]>) {
        if self.transport_failed {
            return;
        }

        let damage = if self.needs_full_present {
            None
        } else {
            damage
        };

        if damage.is_some_and(|rects| rects.is_empty()) {
            return;
        }

        // Get the surface and copy pixels
        let copied = self.conn.with_surface_mut(self.surface_id, |surface| {
            // Get the shared memory buffer
            surface.with_buffer(|shm_buf, width, height| {
                let full_damage = [(0, 0, width, height)];
                let regions = damage.unwrap_or(&full_damage);
                for region in regions {
                    let _ = Self::copy_buffer_region(buffer, shm_buf, width, height, *region);
                }
            });
        });
        if copied.is_none() {
            self.queue_quit();
            return;
        }

        let committed = match damage {
            Some(rects) => {
                let mut committed = true;
                for rect in rects {
                    let (x, y, width, height) = *rect;
                    if width > 0
                        && height > 0
                        && self
                            .conn
                            .commit_region(self.surface_id, x, y, width, height)
                            .is_err()
                    {
                        committed = false;
                        break;
                    }
                }
                committed
            }
            None => self.conn.commit(self.surface_id).is_ok(),
        };

        if committed {
            self.needs_full_present = false;
        } else {
            self.mark_transport_failed();
        }
    }

    fn set_title(&mut self, title: &str) {
        // Note: sws-client doesn't have a set_surface_title method
        // The title is set during surface creation
        let _ = title;
    }

    fn size(&self) -> Size {
        self.current_size
    }

    fn managed_size(&self) -> Size {
        Self::logical_managed_size(self.current_size, self.window_geometry_insets)
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(scarlet_ui_core::error::Error::InvalidSize { width, height });
        }

        let new_size = Size {
            width: width as f32,
            height: height as f32,
        };

        let physical_width = self.logical_to_physical_len(width);
        let physical_height = self.logical_to_physical_len(height);
        let physical_geometry = Self::physical_window_geometry(
            self.window_geometry_insets,
            physical_width,
            physical_height,
            self.scale_milli,
        )?;
        let surface_is_current = self
            .conn
            .with_surface(self.surface_id, |surface| {
                surface.width() == physical_width && surface.height() == physical_height
            })
            .unwrap_or(false);
        if self.current_size == new_size && surface_is_current {
            return Ok(());
        }

        self.conn
            .resize_window(self.surface_id, physical_width, physical_height)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)?;
        if self.window_geometry_supported
            && let Some(geometry) = physical_geometry
        {
            self.conn
                .set_window_geometry(self.surface_id, geometry)
                .map_err(|_| scarlet_ui_core::error::Error::IoError)?;
        }

        self.current_size = new_size;
        self.needs_full_present = true;
        Ok(())
    }

    fn resize_managed(&mut self, width: u32, height: u32) -> Result<()> {
        let (surface_width, surface_height) =
            Self::logical_surface_size_for_managed(width, height, self.window_geometry_insets);
        self.resize(surface_width, surface_height)
    }

    fn close(&mut self) -> Result<()> {
        // Destroy the surface
        self.conn
            .destroy_surface(self.surface_id)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)?;

        Ok(())
    }

    fn minimize(&mut self) -> Result<()> {
        self.conn
            .minimize_window(self.surface_id)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)
    }

    fn maximize(&mut self) -> Result<()> {
        self.conn
            .maximize_window(self.surface_id)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)
    }

    fn set_fullscreen(&mut self, fullscreen: bool) -> Result<()> {
        let result = if fullscreen {
            self.conn.set_fullscreen(self.surface_id)
        } else {
            self.conn.unset_fullscreen(self.surface_id)
        };
        result.map_err(|_| scarlet_ui_core::error::Error::IoError)
    }

    fn set_pointer_lock(&mut self, locked: bool) -> Result<()> {
        if self.pointer_lock_requested == Some(locked)
            || (self.pointer_lock_requested.is_none() && locked == self.pointer_locked)
        {
            return Ok(());
        }
        let capabilities = self
            .conn
            .get_capabilities()
            .map_err(|_| scarlet_ui_core::error::Error::IoError)?;
        if !pointer_lock_supported(capabilities.capabilities) {
            return Err(scarlet_ui_core::error::Error::PointerLockUnsupported);
        }
        self.conn
            .set_pointer_lock(self.surface_id, locked)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)?;
        self.pointer_lock_requested = Some(locked);
        Ok(())
    }

    fn pointer_locked(&self) -> bool {
        self.pointer_locked
    }

    fn restore(&mut self) -> Result<()> {
        self.conn
            .restore_window(self.surface_id)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)
    }

    fn focus(&mut self) -> Result<()> {
        self.conn
            .focus_window(self.surface_id)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)
    }

    fn request_move(&mut self) -> Result<()> {
        self.conn
            .request_move_window(self.surface_id)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)
    }

    fn create_popup(&mut self, position: Point, size: Size) -> Result<u32> {
        let x = self.logical_to_physical_pos(position.x as i32);
        let y = self.logical_to_physical_pos(position.y as i32);

        // Create and place the popup in one request.  This avoids exposing the
        // compositor's fallback position for one frame before move_window is
        // processed.
        let popup_surface_id = self
            .conn
            .create_surface_with_type_and_policies_with_placement(
                "org.scarlet-os.popup",
                "Popup",
                "",
                self.logical_to_physical_len(size.width.max(1.0) as u32),
                self.logical_to_physical_len(size.height.max(1.0) as u32),
                sws_protocol::window_types::ALWAYS_ON_TOP,
                true,
                true,
                false,
                sws_protocol::WindowPlacement::Absolute { x, y },
            )
            .map_err(|_| scarlet_ui_core::error::Error::SurfaceCreationFailed)?;

        self.conn
            .set_window_parent(popup_surface_id, Some(self.surface_id))
            .map_err(|_| scarlet_ui_core::error::Error::IoError)?;
        self.conn
            .set_window_transient_flags(
                popup_surface_id,
                sws::TransientFlags::FOLLOW_PARENT_MOVE | sws::TransientFlags::RAISE_WITH_PARENT,
            )
            .map_err(|_| scarlet_ui_core::error::Error::IoError)?;

        Ok(popup_surface_id)
    }

    fn destroy_popup(&mut self, surface_id: u32) -> Result<()> {
        self.conn
            .destroy_surface(surface_id)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)
    }

    fn set_workarea(&mut self, x: i32, y: i32, width: u32, height: u32) -> Result<()> {
        self.conn
            .set_workarea(
                self.logical_to_physical_pos(x),
                self.logical_to_physical_pos(y),
                self.logical_to_physical_len(width),
                self.logical_to_physical_len(height),
            )
            .map_err(|_| scarlet_ui_core::error::Error::IoError)
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
        Self::create_with_connection_and_policies(
            self.conn.clone(),
            app_id,
            title,
            size,
            window_type,
            "",
            true,
            window_type == sws_protocol::window_types::NORMAL,
            true,
            WindowPlacement::Default,
            EdgeInsets::ZERO,
        )
    }

    fn move_window(&mut self, x: i32, y: i32) -> Result<()> {
        self.conn
            .move_window(
                self.surface_id,
                self.logical_to_physical_pos(x),
                self.logical_to_physical_pos(y),
            )
            .map_err(|_| scarlet_ui_core::error::Error::IoError)
    }

    fn set_window_type(&mut self, surface_id: u32, window_type: u32) -> Result<()> {
        self.conn
            .set_window_type(surface_id, window_type)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)
    }

    fn get_screen_size(&mut self) -> Result<(u32, u32)> {
        let (width, height) = self
            .conn
            .get_screen_size()
            .map_err(|_| scarlet_ui_core::error::Error::IoError)?;
        Ok((
            self.physical_to_logical_len(width),
            self.physical_to_logical_len(height),
        ))
    }

    fn surface_id(&self) -> u32 {
        self.surface_id
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }

    fn set_resizable(&mut self, resizable: bool) -> Result<()> {
        self.conn
            .set_window_resizable(self.surface_id, resizable)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)?;

        if resizable {
            let _ = self
                .conn
                .set_window_size_limits(self.surface_id, sws::WindowSizeLimits::NONE);
        } else {
            let limits = sws::WindowSizeLimits {
                min_width: self.logical_to_physical_len(self.current_size.width.max(1.0) as u32),
                min_height: self.logical_to_physical_len(self.current_size.height.max(1.0) as u32),
                max_width: self.logical_to_physical_len(self.current_size.width.max(1.0) as u32),
                max_height: self.logical_to_physical_len(self.current_size.height.max(1.0) as u32),
            };
            let _ = self.conn.set_window_size_limits(self.surface_id, limits);
        }

        Ok(())
    }

    fn set_opaque(&mut self, opaque: bool) -> Result<()> {
        self.conn
            .set_window_has_alpha_content(self.surface_id, !opaque)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)
    }

    fn set_menu_titles(&mut self, menu_titles: &str) -> Result<()> {
        self.conn
            .set_window_menu_titles(self.surface_id, menu_titles)
            .map_err(|_| scarlet_ui_core::error::Error::IoError)
    }

    fn sync_text_input(&mut self, state: Option<&TextInputElementState>) {
        SWSPlatformWindow::sync_text_input(self, state);
    }
}

impl SWSPlatformWindow {
    fn flush_pending_relative_motion(&mut self) {
        if let Some((dx, dy)) = self.pending_relative.take() {
            self.push_event(Event::Mouse(MouseEvent::RelativeMotion { dx, dy }));
        }
    }

    fn flush_pending_wheel(&mut self) {
        if self.pending_wheel.is_empty() {
            return;
        }

        let Some((delta_x, delta_y)) = self.pending_wheel.take_normalized() else {
            return;
        };

        if scarlet_ui_core::debug::wheel_log_enabled() {
            logln!(
                "[Wheel] sws normalized delta=({}, {}) cursor=({}, {})",
                -delta_x,
                -delta_y,
                self.pointer_x,
                self.pointer_y
            );
        }

        self.push_event(Event::Mouse(MouseEvent::Wheel {
            delta_x: -delta_x,
            delta_y: -delta_y,
            x: self.pointer_x,
            y: self.pointer_y,
            phase: WheelPhase::Moved,
            source: ScrollSource::Wheel,
        }));
    }

    fn handle_sws_event(&mut self, ev: SwsEvent) {
        let debug = scarlet_ui_core::debug::is_enabled();
        if debug {
            logln!("[SWSPlatformWindow] sws_event: {:?}", ev);
        }
        match ev {
            SwsEvent::Input(input) => {
                if debug && input.type_ == event_type::EV_KEY {
                    logln!(
                        "[SWSPlatformWindow] raw key: input_surface={} window_surface={} code={} value={}",
                        input.surface_id,
                        self.surface_id,
                        input.code,
                        input.value
                    );
                }
                if input.surface_id != self.surface_id {
                    if debug && input.type_ == event_type::EV_KEY {
                        logln!(
                            "[SWSPlatformWindow] ignored key for another surface: input_surface={} window_surface={}",
                            input.surface_id,
                            self.surface_id
                        );
                    }
                    return;
                }

                match (input.type_, input.code) {
                    (event_type::EV_ABS, abs_code::ABS_X) if !self.pointer_locked => {
                        self.pointer_x = self.physical_to_logical_pos(input.value);
                        self.pending_move = true;
                        if debug {
                            logln!("[SWSPlatformWindow] ABS_X: {}", input.value);
                        }
                    }
                    (event_type::EV_ABS, abs_code::ABS_Y) if !self.pointer_locked => {
                        self.pointer_y = self.physical_to_logical_pos(input.value);
                        self.pending_move = true;
                        if debug {
                            logln!("[SWSPlatformWindow] ABS_Y: {}", input.value);
                        }
                    }
                    (event_type::EV_SYN, _) => {
                        if self.pending_move {
                            self.push_event(Event::Mouse(MouseEvent::Moved {
                                x: self.pointer_x,
                                y: self.pointer_y,
                            }));
                            if debug {
                                logln!(
                                    "[SWSPlatformWindow] MouseMoved: x={}, y={}",
                                    self.pointer_x,
                                    self.pointer_y
                                );
                            }
                            self.pending_move = false;
                        }
                        self.flush_pending_wheel();
                        self.flush_pending_relative_motion();
                    }
                    (event_type::EV_REL, rel_code::REL_X) if self.pointer_locked => {
                        let _ = self.pending_relative.push_input(
                            input.type_,
                            input.code,
                            input.value,
                            self.pointer_locked,
                        );
                    }
                    (event_type::EV_REL, rel_code::REL_Y) if self.pointer_locked => {
                        let _ = self.pending_relative.push_input(
                            input.type_,
                            input.code,
                            input.value,
                            self.pointer_locked,
                        );
                    }
                    (event_type::EV_REL, rel_code::REL_WHEEL) => {
                        self.pending_wheel.add_discrete_y(input.value);
                    }
                    (event_type::EV_REL, rel_code::REL_HWHEEL) => {
                        self.pending_wheel.add_discrete_x(input.value);
                    }
                    (event_type::EV_REL, rel_code::REL_WHEEL_HI_RES) => {
                        self.pending_wheel.add_hi_res_y(input.value);
                    }
                    (event_type::EV_REL, rel_code::REL_HWHEEL_HI_RES) => {
                        self.pending_wheel.add_hi_res_x(input.value);
                    }
                    (event_type::EV_KEY, key_code::BTN_LEFT) => {
                        let button = MouseButton::Left;
                        if input.value != 0 {
                            self.push_mouse_button_event(button, true);
                            if debug {
                                logln!(
                                    "[SWSPlatformWindow] MouseDown: left x={}, y={}",
                                    self.pointer_x,
                                    self.pointer_y
                                );
                            }
                        } else {
                            self.push_mouse_button_event(button, false);
                            if debug {
                                logln!(
                                    "[SWSPlatformWindow] MouseUp: left x={}, y={}",
                                    self.pointer_x,
                                    self.pointer_y
                                );
                            }
                        }
                    }
                    (event_type::EV_KEY, key_code::BTN_RIGHT) => {
                        let button = MouseButton::Right;
                        if input.value != 0 {
                            self.push_mouse_button_event(button, true);
                            if debug {
                                logln!(
                                    "[SWSPlatformWindow] MouseDown: right x={}, y={}",
                                    self.pointer_x,
                                    self.pointer_y
                                );
                            }
                        } else {
                            self.push_mouse_button_event(button, false);
                            if debug {
                                logln!(
                                    "[SWSPlatformWindow] MouseUp: right x={}, y={}",
                                    self.pointer_x,
                                    self.pointer_y
                                );
                            }
                        }
                    }
                    (event_type::EV_KEY, key_code::BTN_MIDDLE) => {
                        let button = MouseButton::Middle;
                        if input.value != 0 {
                            self.push_mouse_button_event(button, true);
                            if debug {
                                logln!(
                                    "[SWSPlatformWindow] MouseDown: middle x={}, y={}",
                                    self.pointer_x,
                                    self.pointer_y
                                );
                            }
                        } else {
                            self.push_mouse_button_event(button, false);
                            if debug {
                                logln!(
                                    "[SWSPlatformWindow] MouseUp: middle x={}, y={}",
                                    self.pointer_x,
                                    self.pointer_y
                                );
                            }
                        }
                    }
                    (event_type::EV_KEY, code) => {
                        let pressed = input.value != 0;
                        if Self::is_modifier_key(code) {
                            self.update_modifier_state(code, pressed);
                            return;
                        }
                        let mapped_char = self.map_key_char_with_modifiers(code);
                        let mapped = if mapped_char.is_some_and(|c| c.is_control()) {
                            mapped_char.map_or(KeyCode::Unknown, KeyCode::Char)
                        } else {
                            Self::map_key_code(code)
                        };
                        if debug {
                            logln!(
                                "[SWSPlatformWindow] key dispatch: code={} value={} mapped={:?} char={:?}",
                                code,
                                input.value,
                                mapped,
                                mapped_char
                            );
                        }
                        let modifiers = self.current_modifiers();
                        if pressed {
                            self.push_event(Event::Keyboard(KeyEvent::Pressed {
                                keycode: mapped,
                                modifiers,
                            }));
                            if let Some(c) = mapped_char
                                && !c.is_control()
                            {
                                self.push_event(Event::Keyboard(KeyEvent::Char { c }));
                            }
                        } else {
                            self.push_event(Event::Keyboard(KeyEvent::Released {
                                keycode: mapped,
                                modifiers,
                            }));
                        }
                    }
                    _ => {}
                }
            }
            SwsEvent::SurfaceConfigure {
                surface_id,
                width,
                height,
            } => {
                if surface_id == self.surface_id {
                    let logical_width = self.physical_to_logical_len(width);
                    let logical_height = self.physical_to_logical_len(height);
                    self.push_event(Event::Resize {
                        width: logical_width,
                        height: logical_height,
                    });
                    if debug {
                        logln!(
                            "[SWSPlatformWindow] SurfaceConfigure: physical={}x{} logical={}x{}",
                            width,
                            height,
                            logical_width,
                            logical_height
                        );
                    }
                }
            }
            SwsEvent::SurfaceStateChanged {
                surface_id,
                state_flags,
            } if surface_id == self.surface_id => {
                let fullscreen = state_flags & sws::window_state::FULLSCREEN != 0;
                if fullscreen != self.fullscreen {
                    self.fullscreen = fullscreen;
                    self.push_event(Event::FullscreenChanged { fullscreen });
                }
            }
            SwsEvent::PointerLockChanged { window_id, locked } if window_id == self.surface_id => {
                if !apply_pointer_lock_confirmation(
                    &mut self.pointer_locked,
                    &mut self.pointer_lock_requested,
                    locked,
                ) {
                    return;
                }
                if !locked {
                    self.flush_pending_relative_motion();
                    self.pending_relative = PendingRelativeMotion::default();
                }
                self.push_event(Event::PointerLockChanged { locked });
            }
            SwsEvent::ScreenSizeChanged { width, height } => {
                self.push_event(Event::ScreenSizeChanged {
                    width: self.physical_to_logical_len(width),
                    height: self.physical_to_logical_len(height),
                });
            }
            SwsEvent::OutputScaleChanged { scale_milli } => {
                self.scale_milli = Self::sanitize_scale(scale_milli);
                self.push_event(Event::Resize {
                    width: self.current_size.width.max(1.0) as u32,
                    height: self.current_size.height.max(1.0) as u32,
                });
            }
            SwsEvent::InputEnvironmentChanged(environment) => {
                self.push_event(Event::InputEnvironmentChanged(map_sws_input_environment(
                    environment,
                )));
            }
            SwsEvent::SurfaceDestroyed { surface_id } => {
                if surface_id == self.surface_id {
                    self.queue_quit();
                    if debug {
                        logln!("[SWSPlatformWindow] SurfaceDestroyed");
                    }
                }
            }
            SwsEvent::MenuItemActivated {
                window_id,
                menu_item_id,
            } => {
                if window_id == self.surface_id {
                    self.push_event(Event::MenuItemActivated {
                        window_id,
                        menu_item_id,
                    });
                }
            }
            SwsEvent::TextInputPreedit {
                context_id,
                serial,
                cursor_byte,
                anchor_byte,
                text,
                spans,
            } => {
                self.push_event(Event::TextInputPreedit {
                    context_id,
                    serial,
                    cursor_byte,
                    anchor_byte,
                    text,
                    spans,
                });
            }
            SwsEvent::TextInputCommit {
                context_id,
                serial,
                text,
            } => {
                self.push_event(Event::TextInputCommit {
                    context_id,
                    serial,
                    text,
                });
            }
            SwsEvent::TextInputDeleteSurroundingText {
                context_id,
                serial,
                before_bytes,
                after_bytes,
            } => {
                self.push_event(Event::TextInputDeleteSurroundingText {
                    context_id,
                    serial,
                    before_bytes,
                    after_bytes,
                });
            }
            SwsEvent::TextInputDone { context_id, serial } => {
                self.push_event(Event::TextInputDone { context_id, serial });
            }
            SwsEvent::FocusChanged {
                window_id,
                app_id,
                app_name,
                title,
                menu_titles,
            } => {
                let menu_titles = Self::sanitize_menu_titles(&menu_titles);
                if window_id != self.surface_id {
                    self.reset_transient_modifiers();
                    self.pointer_lock_requested = None;
                    if self.pointer_locked {
                        self.flush_pending_relative_motion();
                        self.pointer_locked = false;
                        self.pending_relative = PendingRelativeMotion::default();
                        self.push_event(Event::PointerLockChanged { locked: false });
                    }
                }
                // Push FocusChanged event for all windows to receive
                // This allows TaskBar to update its menu based on focus changes
                if debug {
                    logln!(
                        "[SWSPlatformWindow] FocusChanged: window_id={}, app_name={}, menu_titles={}",
                        window_id,
                        app_name,
                        menu_titles
                    );
                }
                self.push_event(Event::Custom {
                    event_type: 0xF0C0F, // FocusChanged event type
                    data: {
                        // Encode the focus change data
                        let mut data = Vec::new();
                        data.extend_from_slice(&window_id.to_le_bytes());
                        data.extend_from_slice(&(app_id.len() as u32).to_le_bytes());
                        data.extend_from_slice(app_id.as_bytes());
                        data.extend_from_slice(&(app_name.len() as u32).to_le_bytes());
                        data.extend_from_slice(app_name.as_bytes());
                        data.extend_from_slice(&(title.len() as u32).to_le_bytes());
                        data.extend_from_slice(title.as_bytes());
                        data.extend_from_slice(&(menu_titles.len() as u32).to_le_bytes());
                        data.extend_from_slice(menu_titles.as_bytes());
                        data
                    },
                });
            }
            SwsEvent::ActiveAppChanged {
                window_id,
                app_id,
                app_name,
                title,
                menu_titles,
            } => {
                let menu_titles = Self::sanitize_menu_titles(&menu_titles);
                // Push ActiveAppChanged event for TaskBar to update menu bar
                // This is ONLY sent for normal windows (not TaskBar/Desktop/etc)
                // and only when the active APPLICATION changes (same app, different window = no broadcast)
                if debug {
                    logln!(
                        "[SWSPlatformWindow] ActiveAppChanged: window_id={}, app_name={}, menu_titles={}",
                        window_id,
                        app_name,
                        menu_titles
                    );
                }
                self.push_event(Event::Custom {
                    event_type: 0xF0C0A, // ActiveAppChanged event type
                    data: {
                        // Encode the active app change data (same format as FocusChanged)
                        let mut data = Vec::new();
                        data.extend_from_slice(&window_id.to_le_bytes());
                        data.extend_from_slice(&(app_id.len() as u32).to_le_bytes());
                        data.extend_from_slice(app_id.as_bytes());
                        data.extend_from_slice(&(app_name.len() as u32).to_le_bytes());
                        data.extend_from_slice(app_name.as_bytes());
                        data.extend_from_slice(&(title.len() as u32).to_le_bytes());
                        data.extend_from_slice(title.as_bytes());
                        data.extend_from_slice(&(menu_titles.len() as u32).to_le_bytes());
                        data.extend_from_slice(menu_titles.as_bytes());
                        data
                    },
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sws_enables_the_standard_normal_window_shadow_by_default() {
        let mut backend = SwsBackend::new();
        assert_eq!(backend.window_defaults(), PlatformWindowDefaults::new(true));
    }

    #[test]
    fn window_geometry_scales_shadow_outsets_inside_the_surface() {
        assert_eq!(
            SWSPlatformWindow::physical_window_geometry(
                EdgeInsets::new(10.0, 6.0, 10.0, 14.0),
                648,
                520,
                2000,
            ),
            Ok(Some(sws::WindowGeometry {
                x: 20,
                y: 12,
                width: 608,
                height: 480,
            }))
        );
        assert_eq!(
            SWSPlatformWindow::physical_window_geometry(EdgeInsets::ZERO, 304, 240, 1000),
            Ok(None)
        );
    }

    #[test]
    fn managed_resize_preserves_shadow_outsets_in_the_complete_surface() {
        let insets = EdgeInsets::new(10.0, 6.0, 10.0, 14.0);
        let body = Size::new(304.0, 188.0);
        let surface = Size::new(324.0, 208.0);

        assert_eq!(
            SWSPlatformWindow::logical_surface_size_for_managed(304, 188, insets),
            (324, 208)
        );
        assert_eq!(
            SWSPlatformWindow::logical_managed_size(surface, insets),
            body
        );
    }

    #[test]
    fn window_geometry_rejects_outsets_that_consume_the_surface() {
        assert_eq!(
            SWSPlatformWindow::physical_window_geometry(
                EdgeInsets::symmetric(6.0, 10.0),
                20,
                40,
                1000,
            ),
            Err(Error::InvalidSize {
                width: 20,
                height: 40,
            })
        );
    }

    #[test]
    fn system_owned_window_chrome_is_rejected_explicitly() {
        assert_eq!(
            validate_window_decoration(WindowDecoration::SYSTEM),
            Err(Error::WindowDecorationUnsupported)
        );
        assert_eq!(validate_window_decoration(WindowDecoration::CUSTOM), Ok(()));
        assert_eq!(validate_window_decoration(WindowDecoration::NONE), Ok(()));
    }

    #[test]
    fn relative_motion_coalesces_axes_and_drains_without_syn() {
        let mut pending = PendingRelativeMotion::default();
        assert!(pending.push_input(event_type::EV_REL, rel_code::REL_X, 4, true));
        assert!(pending.push_input(event_type::EV_REL, rel_code::REL_Y, -3, true));
        assert!(pending.push_input(event_type::EV_REL, rel_code::REL_X, 2, true));
        assert_eq!(pending.take(), Some((6, -3)));
        assert_eq!(pending.take(), None);
    }

    #[test]
    fn relative_motion_is_ignored_without_pointer_lock() {
        let mut pending = PendingRelativeMotion::default();
        assert!(!pending.push_input(event_type::EV_REL, rel_code::REL_X, 4, false));
        assert_eq!(pending.take(), None);
    }

    #[test]
    fn pointer_lock_capability_and_revoke_state_are_explicit() {
        assert!(!pointer_lock_supported(0));
        assert!(pointer_lock_supported(
            sws_protocol::capabilities::POINTER_LOCK
        ));

        let mut locked = true;
        assert!(update_pointer_lock_state(&mut locked, false));
        assert!(!locked);
        assert!(!update_pointer_lock_state(&mut locked, false));
    }

    #[test]
    fn rejected_pending_lock_still_emits_confirmation() {
        let mut locked = false;
        let mut requested = Some(true);

        assert!(apply_pointer_lock_confirmation(
            &mut locked,
            &mut requested,
            false,
        ));
        assert!(!locked);
        assert_eq!(requested, None);
    }

    #[test]
    fn wheel_delta_prefers_hi_res_over_discrete() {
        let mut pending = PendingWheelDelta::default();
        pending.add_discrete_y(10);
        pending.add_hi_res_y(120);

        let (delta_x, delta_y) = pending.take_normalized().unwrap();

        assert_eq!(delta_x, 0);
        assert_eq!(delta_y, WHEEL_LINE_DELTA);
    }

    #[test]
    fn legacy_sws_wheel_pixels_normalize_to_line_delta() {
        assert_eq!(normalize_discrete_wheel_delta(10), WHEEL_LINE_DELTA);
        assert_eq!(normalize_discrete_wheel_delta(-10), -WHEEL_LINE_DELTA);
        assert_eq!(normalize_discrete_wheel_delta(1), WHEEL_LINE_DELTA);
    }

    #[test]
    fn delayed_release_cannot_release_a_new_generation_or_commit() {
        let identity = SgfxBufferIdentity {
            window_id: 7,
            buffer_id: 1,
            generation: 3,
            compositor_epoch: 5,
        };
        let current = SgfxCommitToken {
            identity,
            commit_serial: 12,
        };
        let mut retained = vec![current];

        let delayed_commit = SgfxCommitToken {
            identity,
            commit_serial: 11,
        };
        assert!(!SwsSgfxFrameSink::release_retained(
            &mut retained,
            delayed_commit,
        ));

        let delayed_generation = SgfxCommitToken {
            identity: SgfxBufferIdentity {
                generation: 2,
                ..identity
            },
            commit_serial: 12,
        };
        assert!(!SwsSgfxFrameSink::release_retained(
            &mut retained,
            delayed_generation,
        ));
        assert_eq!(retained, vec![current]);
        assert!(SwsSgfxFrameSink::release_retained(&mut retained, current,));
        assert!(retained.is_empty());
    }

    #[test]
    fn stale_epoch_is_ignored_only_after_backend_loss() {
        assert!(!sgfx_event_epoch_is_stale(None, 4));
        assert!(sgfx_event_epoch_is_stale(Some(5), 4));
        assert!(!sgfx_event_epoch_is_stale(Some(5), 5));
        assert!(!sgfx_event_epoch_is_stale(Some(5), 6));
    }
}
