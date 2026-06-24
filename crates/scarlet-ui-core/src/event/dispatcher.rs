//! Event Dispatcher - Routes events to target elements
//!
//! EventDispatcher implements hit testing and event routing through
//! the element tree with three-phase event dispatching.

use crate::element::{Element, ElementId, ElementTree};
use crate::event::Event;
use crate::geometry::{Point, Rect};
use alloc::vec::Vec;

/// Monotonic clock for expiring idle wheel gestures.
///
/// `scarlet_std` exposes only `core::time` (no `Instant`), so on the `no_std`
/// target there is no monotonic clock available and `elapsed_since` returns
/// `Duration::ZERO` (idle never expires). Discrete-wheel locking then falls
/// back to pointer-based release via `dispatch_mouse`.
mod wheel_clock {
    #[cfg(feature = "std")]
    pub use std::time::Instant;

    #[cfg(not(feature = "std"))]
    #[derive(Clone, Copy)]
    pub struct Instant;

    #[cfg(not(feature = "std"))]
    impl Instant {
        pub fn now() -> Self {
            Self
        }
    }

    #[cfg(feature = "std")]
    pub fn elapsed_since(instant: Instant) -> core::time::Duration {
        Instant::now().duration_since(instant)
    }

    #[cfg(not(feature = "std"))]
    pub fn elapsed_since(_instant: Instant) -> core::time::Duration {
        core::time::Duration::ZERO
    }
}

use wheel_clock::Instant;

/// Discrete mouse wheels carry no gesture phase, so this idle window acts as
/// the gesture boundary that decides when a locked scroll target may be
/// re-targeted. See `EventDispatcher::wheel_target`.
const WHEEL_GESTURE_IDLE: core::time::Duration = core::time::Duration::from_millis(200);

/// Event dispatch phase
///
/// Events go through three phases:
/// 1. Capture: From root to target's parent
/// 2. Target: At the target element itself
/// 3. Bubble: From target's parent back to root
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Capture phase (root → target)
    Capture,
    /// Target phase (at the target)
    Target,
    /// Bubble phase (target → root)
    Bubble,
}

/// Result of a hit test operation
///
/// Contains the target element and the path from root to target
/// for use in three-phase event dispatching.
pub struct HitResult<'a> {
    /// The target element that was hit
    pub target: &'a dyn Element,
    /// The path from root to target (inclusive)
    /// For capture phase, iterate from index 0 to len-1
    /// For bubble phase, iterate from index len-1 to 0
    pub path: Vec<&'a dyn Element>,
}

impl<'a> HitResult<'a> {
    /// Create a new HitResult
    pub fn new(target: &'a dyn Element, path: Vec<&'a dyn Element>) -> Self {
        Self { target, path }
    }

    /// Get the path for the capture phase (root to target)
    pub fn capture_path(&self) -> impl Iterator<Item = &&'a dyn Element> {
        self.path.iter()
    }

    /// Get the path for the bubble phase (target to root)
    pub fn bubble_path(&self) -> impl Iterator<Item = &&'a dyn Element> {
        self.path.iter().rev()
    }
}

/// Event Dispatcher for routing events to elements
///
/// The dispatcher implements:
/// - Hit testing to find event targets
/// - Capture phase (root → target)
/// - Target phase (handle at target)
/// - Bubble phase (target → root)
pub struct EventDispatcher {
    root_id: Option<ElementId>,
    hovered_id: Option<ElementId>,
    hovered_path: Vec<ElementId>,
    captured_id: Option<ElementId>,
    captured_path: Vec<ElementId>,
    captured_point: Option<Point>,
    /// Element that currently monopolizes an in-progress wheel gesture, if any.
    /// While set, every wheel event routes here regardless of pointer position,
    /// so a parent scroll view does not lose capture to a nested child that
    /// slides under the cursor mid-gesture.
    wheel_target: Option<ElementId>,
    wheel_path: Vec<ElementId>,
    hit_path_scratch: Vec<ElementId>,
    path_origins_scratch: Vec<Point>,
    /// Timestamp of the most recent wheel event, used to expire idle discrete
    /// (non-trackpad) wheel gestures via `WHEEL_GESTURE_IDLE`.
    wheel_last_event_at: Option<Instant>,
    left_button_down: bool,
    focused_id: Option<ElementId>,
    focused_path: Vec<ElementId>,
    /// Events emitted by elements during event handling
    emitted_events: Vec<Event>,
}

impl EventDispatcher {
    /// Create a new EventDispatcher
    pub fn new() -> Self {
        Self {
            root_id: None,
            hovered_id: None,
            hovered_path: Vec::new(),
            captured_id: None,
            captured_path: Vec::new(),
            captured_point: None,
            wheel_target: None,
            wheel_path: Vec::new(),
            hit_path_scratch: Vec::new(),
            path_origins_scratch: Vec::new(),
            wheel_last_event_at: None,
            left_button_down: false,
            focused_id: None,
            focused_path: Vec::new(),
            emitted_events: Vec::new(),
        }
    }

    /// Set the root element ID
    pub fn set_root(&mut self, id: ElementId) {
        self.root_id = Some(id);
    }

    /// Emit an event (called by elements during event handling)
    pub fn emit(&mut self, event: Event) {
        self.emitted_events.push(event);
    }

    /// Take all emitted events and clear the buffer
    pub fn take_emitted_events(&mut self) -> Vec<Event> {
        core::mem::take(&mut self.emitted_events)
    }

    /// Dispatch an event to the appropriate element
    pub fn dispatch(&mut self, element_tree: &mut ElementTree, event: &Event) -> bool {
        if crate::debug::is_enabled() {
            crate::logln!("[EventDispatcher] dispatch: {:?}", event);
        }
        match event {
            Event::Quit => {
                // Quit event is handled at application level
                self.handle_quit(element_tree);
                false
            }
            Event::Resize { width, height } => {
                self.handle_resize(element_tree, *width, *height);
                false
            }
            Event::ScreenSizeChanged { .. } => false,
            Event::Mouse(mouse_event) => self.dispatch_mouse(element_tree, mouse_event),
            Event::Keyboard(key_event) => self.dispatch_keyboard(element_tree, key_event),
            Event::Focus(focus_event) => self.dispatch_focus(element_tree, focus_event),
            Event::Lifecycle(lifecycle_event) => {
                self.dispatch_lifecycle(element_tree, lifecycle_event)
            }
            Event::Window(_window_event) => false,
            Event::Input(_) => {
                // Raw input events are typically handled by higher layers
                false
            }
            Event::MenuItemActivated { .. } => false,
            Event::TextInputPreedit { .. }
            | Event::TextInputCommit { .. }
            | Event::TextInputDeleteSurroundingText { .. }
            | Event::TextInputDone { .. } => self.dispatch_text_input(element_tree, event),
            Event::Custom { .. } => {
                // Custom events can be dispatched similarly
                false
            }
        }
    }

    /// Handle quit event
    fn handle_quit(&mut self, _element_tree: &mut ElementTree) {
        // Signal the application to exit
        // In a full implementation, this would set a flag or call a callback
    }

    /// Handle resize event
    fn handle_resize(&mut self, _element_tree: &mut ElementTree, width: u32, height: u32) {
        // Handle window resize
        // In a full implementation, this would mark elements for relayout
        let _ = (width, height);
    }

    /// Dispatch a mouse event with three-phase event handling
    fn dispatch_mouse(
        &mut self,
        element_tree: &mut ElementTree,
        event: &crate::event::MouseEvent,
    ) -> bool {
        // 1. Hit test to find target and path
        let point = self.extract_point_from_mouse(&event);
        let is_wheel = matches!(event, crate::event::MouseEvent::Wheel { .. });
        let uses_wheel_capture = Self::wheel_uses_transaction_capture(event);
        let mut wheel_target_locked = false;
        let mut wheel_consumed_without_path = false;
        if crate::debug::wheel_log_enabled()
            && let crate::event::MouseEvent::Wheel {
                delta_x,
                delta_y,
                phase,
                source,
                ..
            } = event
        {
            crate::logln!(
                "[Wheel] dispatch source={:?} phase={:?} delta=({}, {}) point=({:.0}, {:.0}) captured={:?} tx={}",
                source,
                phase,
                delta_x,
                delta_y,
                point.x,
                point.y,
                self.wheel_target,
                uses_wheel_capture
            );
        }

        let mut path = if is_wheel {
            // Expire an idle discrete-wheel gesture so a new gesture can
            // re-target after the user pauses. No-op without a monotonic clock.
            if self.wheel_target.is_some() && !uses_wheel_capture && self.wheel_idle_expired() {
                if crate::debug::wheel_log_enabled() {
                    crate::logln!("[Wheel] release idle-expired");
                }
                self.clear_wheel_target();
            }

            // For discrete wheels we need the current hit-path both to decide
            // whether the pointer left the locked scroller and to resolve a
            // fresh target. Trackpad gestures are bound to finger contact, not
            // cursor position, so they skip this.
            let discrete_hit_path = if !uses_wheel_capture {
                let mut hit_path = core::mem::take(&mut self.hit_path_scratch);
                if self.hit_test_with_path_ids_into(element_tree, point, &mut hit_path) {
                    Some(hit_path)
                } else {
                    self.hit_path_scratch = hit_path;
                    None
                }
            } else {
                None
            };

            if let Some(target_id) = self.wheel_target {
                if !uses_wheel_capture
                    && discrete_hit_path
                        .as_ref()
                        .is_some_and(|hit_path| !hit_path.contains(&target_id))
                {
                    // Pointer left the locked scroller: release so the user
                    // can scroll a different area, then resolve from here.
                    if crate::debug::wheel_log_enabled() {
                        crate::logln!("[Wheel] release pointer-left target={:?}", target_id);
                    }
                    self.clear_wheel_target();
                    discrete_hit_path
                } else {
                    if let Some(hit_path) = discrete_hit_path {
                        self.hit_path_scratch = hit_path;
                    }
                    // Gesture in progress: route every wheel event to the
                    // locked target, ignoring pointer movement. This is what
                    // keeps a parent scroll view scrolling when a nested child
                    // slides under the cursor mid-gesture.
                    wheel_target_locked = true;
                    let cached_path = core::mem::take(&mut self.wheel_path);
                    if cached_path.last().copied() == Some(target_id) {
                        if crate::debug::wheel_log_enabled() {
                            crate::logln!("[Wheel] reuse target={:?}", target_id);
                        }
                        Some(cached_path)
                    } else {
                        match element_tree.find_path_ids(target_id) {
                            Some(path) => {
                                if crate::debug::wheel_log_enabled() {
                                    crate::logln!("[Wheel] reuse target={:?}", target_id);
                                }
                                Some(path)
                            }
                            None => {
                                if crate::debug::wheel_log_enabled() {
                                    crate::logln!(
                                        "[Wheel] captured target vanished id={:?}",
                                        target_id
                                    );
                                }
                                wheel_consumed_without_path = true;
                                None
                            }
                        }
                    }
                }
            } else if uses_wheel_capture && self.wheel_phase_finished(event) {
                // Terminal phase with no active gesture: nothing to deliver.
                None
            } else {
                // Gesture start: hit-test, then (for trackpad) prefer the
                // deepest wheel-capturing element. Discrete wheels resolve
                // their handler through the normal capture/target/bubble flow
                // and lock onto whatever handled the event after dispatch.
                let hit_path = if uses_wheel_capture {
                    self.hit_test_with_path_ids(element_tree, point)
                } else {
                    discrete_hit_path
                };
                match hit_path {
                    Some(hit_path) => {
                        if uses_wheel_capture
                            && let Some(capture_path) =
                                Self::wheel_capture_path_for_event(element_tree, &hit_path, event)
                        {
                            self.wheel_target = capture_path.last().copied();
                            wheel_target_locked = true;
                            if crate::debug::wheel_log_enabled() {
                                crate::logln!("[Wheel] acquire target={:?}", self.wheel_target);
                            }
                            Some(capture_path)
                        } else {
                            if crate::debug::wheel_log_enabled() {
                                crate::logln!(
                                    "[Wheel] direct target={:?}",
                                    hit_path.last().copied()
                                );
                            }
                            Some(hit_path)
                        }
                    }
                    None => None,
                }
            }
        } else if matches!(event, crate::event::MouseEvent::Moved { .. })
            && !(self.left_button_down && self.captured_id.is_some())
        {
            self.cached_path_if_inside(element_tree, point)
                .or_else(|| self.hit_test_with_path_ids(element_tree, point))
        } else {
            self.hit_test_with_path_ids(element_tree, point)
        };

        if self.left_button_down {
            if let Some(captured_id) = self.captured_id {
                if let Some(captured_path) = element_tree.find_path_ids(captured_id) {
                    self.captured_path = captured_path.clone();
                    path = Some(captured_path);
                    self.update_captured_point_from_path(element_tree);
                } else if let Some(captured_point) = self.captured_point {
                    if let Some(captured_path) =
                        self.hit_test_with_path_ids(element_tree, captured_point)
                    {
                        self.captured_id = captured_path.last().copied();
                        self.captured_path = captured_path.clone();
                        path = Some(captured_path);
                        self.update_captured_point_from_path(element_tree);
                    } else {
                        self.captured_path.clear();
                    }
                } else {
                    self.captured_path.clear();
                }
            } else if let Some(captured_point) = self.captured_point {
                if let Some(captured_path) =
                    self.hit_test_with_path_ids(element_tree, captured_point)
                {
                    self.captured_id = captured_path.last().copied();
                    self.captured_path = captured_path.clone();
                    path = Some(captured_path);
                    self.update_captured_point_from_path(element_tree);
                }
            }
        }

        if let Some(path) = path {
            let mut path_origins = core::mem::take(&mut self.path_origins_scratch);
            Self::path_origins_into(element_tree, &path, &mut path_origins);
            let target_id = *path.last().unwrap();
            if crate::debug::is_enabled() {
                crate::logln!(
                    "[EventDispatcher] mouse: {:?} point=({:.1},{:.1}) target_id={:?} path_len={}",
                    event,
                    point.x,
                    point.y,
                    target_id,
                    path.len()
                );
                for (index, id) in path.iter().enumerate() {
                    if let Some(element) = element_tree.find_element_mut(*id) {
                        let bounds = element.bounds();
                        crate::logln!(
                            "[EventDispatcher] path[{}] id={:?} type={} bounds=({:.1},{:.1},{:.1},{:.1})",
                            index,
                            id,
                            element.type_name_debug(),
                            bounds.origin.x,
                            bounds.origin.y,
                            bounds.size.width,
                            bounds.size.height
                        );
                    } else {
                        crate::logln!("[EventDispatcher] path[{}] id={:?} (not found)", index, id);
                    }
                }
            }

            if let crate::event::MouseEvent::Moved { x, y } = event {
                if self.captured_id.is_some() || self.wheel_target.is_some() {
                    // Skip hover updates while dragging or mid-scroll, so that
                    // content sliding under a stationary cursor does not fire
                    // Entered/Exited on the children it crosses.
                } else if self.hovered_id != Some(target_id) {
                    if crate::debug::is_enabled() {
                        crate::logln!(
                            "[EventDispatcher] hover change: {:?} -> {:?}",
                            self.hovered_id,
                            Some(target_id)
                        );
                    }
                    if let Some(old_id) = self.hovered_id {
                        let old_origin =
                            Self::path_origin_from_ids(element_tree, &self.hovered_path)
                                .unwrap_or(Point::ZERO);
                        if let Some(old_element) = element_tree.find_element_mut(old_id) {
                            let _ = old_element.handle_event(
                                &Event::Mouse(Self::localize_mouse_event(
                                    &crate::event::MouseEvent::Exited { x: *x, y: *y },
                                    old_origin,
                                )),
                                Phase::Target,
                            );
                        }
                    }

                    if let Some(new_element) = element_tree.find_element_mut(target_id) {
                        let new_origin = *path_origins.last().unwrap_or(&Point::ZERO);
                        let _ = new_element.handle_event(
                            &Event::Mouse(Self::localize_mouse_event(
                                &crate::event::MouseEvent::Entered { x: *x, y: *y },
                                new_origin,
                            )),
                            Phase::Target,
                        );
                    }

                    self.hovered_id = Some(target_id);
                    self.hovered_path = path.clone();
                }
            }

            if let crate::event::MouseEvent::ButtonPressed { .. } = event {
                if let Some(focus_id) = element_tree.nearest_focusable_in_path(&path) {
                    if let Some(focus_path) = element_tree.find_path_ids(focus_id) {
                        self.set_focused_element(element_tree, focus_id, &focus_path);
                    }
                } else {
                    self.clear_focused_element(element_tree);
                }
                if let crate::event::MouseEvent::ButtonPressed {
                    button: crate::event::MouseButton::Left,
                    ..
                } = event
                {
                    self.left_button_down = true;
                    self.captured_id = Some(target_id);
                    self.captured_path = path.clone();
                    self.captured_point = Some(point);
                }
            }

            // 2. Three-phase dispatch
            let mut handled = false;
            let mut handled_id = None;

            // 2.1 Capture Phase: root → target (excluding target)
            for (index, id) in path.iter().take(path.len().saturating_sub(1)).enumerate() {
                if let Some(element) = element_tree.find_element_mut(*id) {
                    let localized =
                        Event::Mouse(Self::localize_mouse_event(event, path_origins[index]));
                    if element.handle_event(&localized, Phase::Capture) {
                        if let Some(window_event) = element.take_window_action() {
                            self.emitted_events.push(Event::Window(window_event));
                        }
                        handled = true;
                        handled_id = Some(*id);
                        break;
                    }
                }
            }

            // 2.2 Target Phase: at the target
            if !handled {
                if let Some(target) = element_tree.find_element_mut(target_id) {
                    let localized = Event::Mouse(Self::localize_mouse_event(
                        event,
                        *path_origins.last().unwrap_or(&Point::ZERO),
                    ));
                    handled = target.handle_event(&localized, Phase::Target);
                    if handled {
                        handled_id = Some(target_id);
                        if let Some(window_event) = target.take_window_action() {
                            self.emitted_events.push(Event::Window(window_event));
                        }
                    }
                }
            }

            // 2.3 Bubble Phase: target's parent → root
            if !handled && !wheel_target_locked {
                for (index, id) in path.iter().rev().skip(1).enumerate() {
                    if let Some(element) = element_tree.find_element_mut(*id) {
                        let origin_index = path.len().saturating_sub(2).saturating_sub(index);
                        let localized = Event::Mouse(Self::localize_mouse_event(
                            event,
                            path_origins[origin_index],
                        ));
                        if element.handle_event(&localized, Phase::Bubble) {
                            if let Some(window_event) = element.take_window_action() {
                                self.emitted_events.push(Event::Window(window_event));
                            }
                            handled = true;
                            handled_id = Some(*id);
                            break;
                        }
                    }
                }
            }

            // Lock the wheel gesture onto the element that actually handled
            // the event. Once locked, subsequent wheel events keep routing
            // here regardless of pointer position (see `wheel_target`), so a
            // parent scroll view does not lose capture to a nested child
            // sliding under the cursor mid-gesture.
            if is_wheel
                && handled
                && let Some(id) = handled_id
                && self.wheel_target.is_none()
            {
                self.wheel_target = Some(id);
                if crate::debug::wheel_log_enabled() {
                    crate::logln!("[Wheel] lock target={:?}", id);
                }
            }
            if is_wheel && self.wheel_target.is_some() {
                self.wheel_last_event_at = Some(Instant::now());
            }

            if uses_wheel_capture && self.wheel_phase_finished(event) {
                if crate::debug::wheel_log_enabled() {
                    crate::logln!("[Wheel] release phase=end");
                }
                self.clear_wheel_target();
            }

            if crate::debug::is_enabled() {
                crate::logln!("[EventDispatcher] mouse handled={}", handled);
            }
            if crate::debug::wheel_log_enabled() && is_wheel {
                crate::logln!(
                    "[Wheel] result target={:?} handler={:?} handled={} locked={} consumed={}",
                    target_id,
                    handled_id,
                    handled,
                    wheel_target_locked,
                    handled || wheel_target_locked
                );
            }
            if let crate::event::MouseEvent::ButtonReleased {
                button: crate::event::MouseButton::Left,
                ..
            } = event
            {
                self.left_button_down = false;
                self.captured_id = None;
                self.captured_path.clear();
                self.captured_point = None;
            }
            let result = handled || wheel_target_locked;
            self.path_origins_scratch = path_origins;
            if is_wheel && self.wheel_target.is_some() {
                self.wheel_path = path;
            } else {
                self.hit_path_scratch = path;
            }
            result
        } else {
            if let crate::event::MouseEvent::Moved { x, y } = event {
                if let Some(old_id) = self.hovered_id {
                    let old_origin = Self::path_origin_from_ids(element_tree, &self.hovered_path)
                        .unwrap_or(Point::ZERO);
                    if let Some(old_element) = element_tree.find_element_mut(old_id) {
                        let _ = old_element.handle_event(
                            &Event::Mouse(Self::localize_mouse_event(
                                &crate::event::MouseEvent::Exited { x: *x, y: *y },
                                old_origin,
                            )),
                            Phase::Target,
                        );
                    }
                }
                self.hovered_id = None;
                self.hovered_path.clear();
            }
            if let crate::event::MouseEvent::ButtonReleased {
                button: crate::event::MouseButton::Left,
                ..
            } = event
            {
                self.left_button_down = false;
                self.captured_id = None;
                self.captured_path.clear();
                self.captured_point = None;
            }
            if uses_wheel_capture && self.wheel_phase_finished(event) {
                if crate::debug::wheel_log_enabled() {
                    crate::logln!("[Wheel] release phase=end");
                }
                self.clear_wheel_target();
            }
            if crate::debug::wheel_log_enabled() && is_wheel {
                crate::logln!(
                    "[Wheel] result no_path consumed={}",
                    wheel_consumed_without_path
                );
            }
            wheel_consumed_without_path
        }
    }

    /// Dispatch a keyboard event
    fn dispatch_keyboard(
        &mut self,
        element_tree: &mut ElementTree,
        event: &crate::event::KeyEvent,
    ) -> bool {
        let path = self
            .focused_id
            .and_then(|focused_id| {
                element_tree
                    .element_wants_keyboard_focus(focused_id)
                    .then(|| element_tree.find_path_ids(focused_id))
                    .flatten()
            })
            .or_else(|| element_tree.find_keyboard_focus_path_ids())
            .or_else(|| {
                if self
                    .focused_path
                    .last()
                    .is_some_and(|id| element_tree.element_wants_keyboard_focus(*id))
                {
                    Some(self.focused_path.clone())
                } else {
                    None
                }
            })
            .or_else(|| element_tree.root().map(|root| alloc::vec![root.id()]));

        let Some(path) = path else {
            return false;
        };
        let Some(target_id) = path.last().copied() else {
            return false;
        };
        self.focused_id = Some(target_id);
        self.focused_path = path.clone();

        if crate::debug::is_enabled() {
            crate::logln!(
                "[EventDispatcher] keyboard: {:?} target_id={:?} path_len={}",
                event,
                target_id,
                path.len()
            );
        }

        let keyboard_event = Event::Keyboard(*event);
        let mut handled = false;

        for id in path.iter().take(path.len().saturating_sub(1)) {
            if let Some(element) = element_tree.find_element_mut(*id) {
                if element.handle_event(&keyboard_event, Phase::Capture) {
                    handled = true;
                    break;
                }
            }
        }

        if !handled {
            if let Some(target) = element_tree.find_element_mut(target_id) {
                handled = target.handle_event(&keyboard_event, Phase::Target);
            }
        }

        if !handled {
            for id in path.iter().rev().skip(1) {
                if let Some(element) = element_tree.find_element_mut(*id) {
                    if element.handle_event(&keyboard_event, Phase::Bubble) {
                        handled = true;
                        break;
                    }
                }
            }
        }

        if crate::debug::is_enabled() {
            crate::logln!("[EventDispatcher] keyboard handled={}", handled);
        }
        handled
    }

    fn dispatch_text_input(&mut self, element_tree: &mut ElementTree, event: &Event) -> bool {
        let path = self
            .focused_id
            .and_then(|focused_id| {
                element_tree
                    .element_wants_keyboard_focus(focused_id)
                    .then(|| element_tree.find_path_ids(focused_id))
                    .flatten()
            })
            .or_else(|| element_tree.find_keyboard_focus_path_ids())
            .or_else(|| {
                if self
                    .focused_path
                    .last()
                    .is_some_and(|id| element_tree.element_wants_keyboard_focus(*id))
                {
                    Some(self.focused_path.clone())
                } else {
                    None
                }
            });

        let Some(path) = path else {
            return false;
        };
        let Some(target_id) = path.last().copied() else {
            return false;
        };
        self.focused_id = Some(target_id);
        self.focused_path = path;

        element_tree
            .find_element_mut(target_id)
            .is_some_and(|target| target.handle_event(event, Phase::Target))
    }

    /// Dispatch a focus event
    fn dispatch_focus(
        &mut self,
        element_tree: &mut ElementTree,
        event: &crate::event::FocusEvent,
    ) -> bool {
        // Focus events are sent to the element gaining or losing focus
        // For now, send to the root with Target phase
        if let Some(root) = element_tree.root_mut() {
            root.handle_event(&Event::Focus(event.clone()), Phase::Target)
        } else {
            false
        }
    }

    /// Dispatch a lifecycle event
    fn dispatch_lifecycle(
        &mut self,
        element_tree: &mut ElementTree,
        event: &crate::event::LifecycleEvent,
    ) -> bool {
        // Lifecycle events are sent to elements during mount/unmount
        // For now, send to the root with Target phase
        if let Some(root) = element_tree.root_mut() {
            root.handle_event(&Event::Lifecycle(event.clone()), Phase::Target)
        } else {
            false
        }
    }

    /// Hit test to find the element at a point
    pub fn hit_test<'a>(
        &'a self,
        element_tree: &'a ElementTree,
        point: Point,
    ) -> Option<&'a dyn Element> {
        let root = element_tree.root()?;
        self.hit_test_select_overlay_recursive(root, point)
            .or_else(|| self.hit_test_recursive(root, point))
            .map(|(target, _)| target)
    }

    /// Hit test to find the element at a point with the path from root
    ///
    /// This returns a HitResult containing both the target and the full path,
    /// which is necessary for three-phase event dispatching.
    pub fn hit_test_with_path<'a>(
        &'a self,
        element_tree: &'a ElementTree,
        point: Point,
    ) -> Option<HitResult<'a>> {
        let root = element_tree.root()?;
        self.hit_test_select_overlay_recursive(root, point)
            .or_else(|| self.hit_test_recursive(root, point))
            .map(|(target, path)| HitResult::new(target, path))
    }

    /// Recursive hit test implementation that returns target and path
    fn hit_test_recursive<'a>(
        &'a self,
        element: &'a dyn Element,
        point: Point,
    ) -> Option<(&'a dyn Element, Vec<&'a dyn Element>)> {
        if !Self::point_inside_element_clip(element, point) {
            return None;
        }

        let local_point = Point {
            x: point.x - element.position().x,
            y: point.y - element.position().y,
        };
        // Check children first (reverse order for z-index)
        for child in element.children().iter().rev() {
            if let Some((found, mut path)) = self.hit_test_recursive(child.as_ref(), local_point) {
                // Add this element to the path
                path.push(element);
                return Some((found, path));
            }
        }

        // Check this element
        if element.hit_test(point) {
            let mut path = Vec::new();
            path.push(element);
            return Some((element, path));
        }

        None
    }

    /// Hit test to find the element at a point with the path of IDs from root
    pub fn hit_test_with_path_ids(
        &self,
        element_tree: &ElementTree,
        point: Point,
    ) -> Option<Vec<ElementId>> {
        let root = element_tree.root()?;
        let mut path = self
            .hit_test_select_overlay_recursive_ids(root, point)
            .or_else(|| self.hit_test_recursive_ids(root, point))?;
        path.reverse();
        Some(path)
    }

    fn hit_test_with_path_ids_into(
        &self,
        element_tree: &ElementTree,
        point: Point,
        path: &mut Vec<ElementId>,
    ) -> bool {
        path.clear();
        let Some(root) = element_tree.root() else {
            return false;
        };
        self.hit_test_recursive_ids_into(root, point, path)
    }

    fn hit_test_recursive_ids_into(
        &self,
        element: &dyn Element,
        point: Point,
        path: &mut Vec<ElementId>,
    ) -> bool {
        if !Self::point_inside_element_clip(element, point) {
            return false;
        }

        path.push(element.id());
        let local_point = Point {
            x: point.x - element.position().x,
            y: point.y - element.position().y,
        };
        for child in element.children().iter().rev() {
            if self.hit_test_recursive_ids_into(child.as_ref(), local_point, path) {
                return true;
            }
        }

        if element.hit_test(point) {
            return true;
        }
        path.pop();
        false
    }

    fn hit_test_recursive_ids(
        &self,
        element: &dyn Element,
        point: Point,
    ) -> Option<Vec<ElementId>> {
        if !Self::point_inside_element_clip(element, point) {
            return None;
        }

        let local_point = Point {
            x: point.x - element.position().x,
            y: point.y - element.position().y,
        };
        for child in element.children().iter().rev() {
            if let Some(mut path) = self.hit_test_recursive_ids(child.as_ref(), local_point) {
                path.push(element.id());
                return Some(path);
            }
        }

        if element.hit_test(point) {
            return Some(alloc::vec![element.id()]);
        }

        None
    }

    fn hit_test_select_overlay_recursive<'a>(
        &'a self,
        element: &'a dyn Element,
        point: Point,
    ) -> Option<(&'a dyn Element, Vec<&'a dyn Element>)> {
        let local_point = Point {
            x: point.x - element.position().x,
            y: point.y - element.position().y,
        };

        for child in element.children().iter().rev() {
            if let Some((found, mut path)) =
                self.hit_test_select_overlay_recursive(child.as_ref(), local_point)
            {
                path.push(element);
                return Some((found, path));
            }
        }

        if Self::is_expanded_select(element) && element.hit_test(point) {
            return Some((element, alloc::vec![element]));
        }

        None
    }

    fn hit_test_select_overlay_recursive_ids(
        &self,
        element: &dyn Element,
        point: Point,
    ) -> Option<Vec<ElementId>> {
        let local_point = Point {
            x: point.x - element.position().x,
            y: point.y - element.position().y,
        };

        for child in element.children().iter().rev() {
            if let Some(mut path) =
                self.hit_test_select_overlay_recursive_ids(child.as_ref(), local_point)
            {
                path.push(element.id());
                return Some(path);
            }
        }

        if Self::is_expanded_select(element) && element.hit_test(point) {
            return Some(alloc::vec![element.id()]);
        }

        None
    }

    fn point_inside_element_clip(element: &dyn Element, point: Point) -> bool {
        let Some(render_object) = element.render_object() else {
            return true;
        };
        let Some((clip_rect, _radius)) = render_object.clip_bounds(element.position()) else {
            return true;
        };
        clip_rect.contains(point)
    }

    fn wheel_capture_path_for_event(
        element_tree: &mut ElementTree,
        hit_path: &[ElementId],
        event: &crate::event::MouseEvent,
    ) -> Option<Vec<ElementId>> {
        if !matches!(event, crate::event::MouseEvent::Wheel { .. }) {
            return None;
        }

        let origins = Self::path_origins(element_tree, hit_path);
        for index in (0..hit_path.len()).rev() {
            let localized = Self::localize_mouse_event(event, origins[index]);
            let Some(element) = element_tree.find_element_mut(hit_path[index]) else {
                continue;
            };
            if element
                .render_object()
                .is_some_and(|render_object| render_object.captures_wheel_event(&localized))
            {
                return Some(hit_path[..=index].to_vec());
            }
        }

        None
    }

    fn is_expanded_select(element: &dyn Element) -> bool {
        element
            .render_object()
            .and_then(|render_object| {
                render_object
                    .as_any()
                    .downcast_ref::<crate::views::SelectRenderObject>()
            })
            .is_some_and(|select| select.is_expanded())
    }

    /// Extract point from a mouse event
    fn extract_point_from_mouse(&self, event: &crate::event::MouseEvent) -> Point {
        match event {
            crate::event::MouseEvent::Moved { x, y } => Point {
                x: *x as f32,
                y: *y as f32,
            },
            crate::event::MouseEvent::Entered { x, y } => Point {
                x: *x as f32,
                y: *y as f32,
            },
            crate::event::MouseEvent::Exited { x, y } => Point {
                x: *x as f32,
                y: *y as f32,
            },
            crate::event::MouseEvent::ButtonPressed { x, y, .. } => Point {
                x: *x as f32,
                y: *y as f32,
            },
            crate::event::MouseEvent::ButtonReleased { x, y, .. } => Point {
                x: *x as f32,
                y: *y as f32,
            },
            crate::event::MouseEvent::Wheel { x, y, .. } => Point {
                x: *x as f32,
                y: *y as f32,
            },
        }
    }

    fn cached_path_if_inside(
        &mut self,
        element_tree: &mut ElementTree,
        point: Point,
    ) -> Option<Vec<ElementId>> {
        let hovered_id = self.hovered_id?;
        let last = *self.hovered_path.last()?;
        if last != hovered_id {
            self.hovered_path.clear();
            return None;
        }

        let mut parent_origin = Point::ZERO;
        if self.hovered_path.len() > 1 {
            for id in self.hovered_path.iter().take(self.hovered_path.len() - 1) {
                let Some(element) = element_tree.find_element_mut(*id) else {
                    self.hovered_path.clear();
                    return None;
                };
                let pos = element.position();
                parent_origin.x += pos.x;
                parent_origin.y += pos.y;
            }
        }

        let Some(target) = element_tree.find_element_mut(hovered_id) else {
            self.hovered_path.clear();
            return None;
        };
        let target_pos = target.position();
        let absolute_origin = Point {
            x: parent_origin.x + target_pos.x,
            y: parent_origin.y + target_pos.y,
        };
        let size = target.bounds().size;
        let rect = Rect::from_xywh(
            absolute_origin.x,
            absolute_origin.y,
            size.width,
            size.height,
        );
        if rect.contains(point) {
            return Some(self.hovered_path.clone());
        }

        None
    }

    fn update_captured_point_from_path(&mut self, element_tree: &mut ElementTree) {
        let Some(target_id) = self.captured_path.last().copied() else {
            return;
        };

        let mut parent_origin = Point::ZERO;
        if self.captured_path.len() > 1 {
            for id in self.captured_path.iter().take(self.captured_path.len() - 1) {
                let Some(element) = element_tree.find_element_mut(*id) else {
                    return;
                };
                let pos = element.position();
                parent_origin.x += pos.x;
                parent_origin.y += pos.y;
            }
        }

        let Some(target) = element_tree.find_element_mut(target_id) else {
            return;
        };
        let target_pos = target.position();
        let bounds = target.bounds().size;
        let absolute_origin = Point {
            x: parent_origin.x + target_pos.x,
            y: parent_origin.y + target_pos.y,
        };
        self.captured_point = Some(Point {
            x: absolute_origin.x + bounds.width / 2.0,
            y: absolute_origin.y + bounds.height / 2.0,
        });
    }

    fn clear_wheel_target(&mut self) {
        self.wheel_target = None;
        self.wheel_path.clear();
        self.wheel_last_event_at = None;
    }

    fn wheel_idle_expired(&self) -> bool {
        self.wheel_last_event_at
            .is_some_and(|last| wheel_clock::elapsed_since(last) > WHEEL_GESTURE_IDLE)
    }

    fn wheel_phase_finished(&self, event: &crate::event::MouseEvent) -> bool {
        matches!(
            event,
            crate::event::MouseEvent::Wheel {
                phase: crate::event::WheelPhase::Ended | crate::event::WheelPhase::Cancelled,
                ..
            }
        )
    }

    fn wheel_uses_transaction_capture(event: &crate::event::MouseEvent) -> bool {
        matches!(
            event,
            crate::event::MouseEvent::Wheel { source, .. } if source.uses_transaction_capture()
        )
    }

    fn set_focused_element(
        &mut self,
        element_tree: &mut ElementTree,
        target_id: ElementId,
        path: &[ElementId],
    ) {
        if self.focused_id == Some(target_id) {
            self.focused_path = path.to_vec();
            return;
        }

        if let Some(old_path) = self.current_focus_path(element_tree)
            && let Some(old_target_id) = old_path.last().copied()
            && old_target_id != target_id
            && let Some(old_target) = element_tree.find_element_mut(old_target_id)
        {
            let _ = old_target
                .handle_event(&Event::Focus(crate::event::FocusEvent::Lost), Phase::Target);
        }

        self.focused_id = Some(target_id);
        self.focused_path = path.to_vec();
        if let Some(target) = element_tree.find_element_mut(target_id) {
            let _ = target.handle_event(
                &Event::Focus(crate::event::FocusEvent::Gained),
                Phase::Target,
            );
        }
    }

    fn clear_focused_element(&mut self, element_tree: &mut ElementTree) {
        if let Some(old_path) = self.current_focus_path(element_tree)
            && let Some(old_target_id) = old_path.last().copied()
            && let Some(old_target) = element_tree.find_element_mut(old_target_id)
        {
            let _ = old_target
                .handle_event(&Event::Focus(crate::event::FocusEvent::Lost), Phase::Target);
        }
        self.focused_id = None;
        self.focused_path.clear();
    }

    fn current_focus_path(&mut self, element_tree: &mut ElementTree) -> Option<Vec<ElementId>> {
        self.focused_id
            .and_then(|focused_id| {
                element_tree
                    .find_path_ids(focused_id)
                    .filter(|_| element_tree.element_wants_keyboard_focus(focused_id))
            })
            .or_else(|| element_tree.find_keyboard_focus_path_ids())
    }

    fn path_origins(element_tree: &mut ElementTree, path: &[ElementId]) -> Vec<Point> {
        let mut origins = Vec::with_capacity(path.len());
        Self::path_origins_into(element_tree, path, &mut origins);
        origins
    }

    fn path_origins_into(
        element_tree: &mut ElementTree,
        path: &[ElementId],
        origins: &mut Vec<Point>,
    ) {
        origins.clear();
        let mut acc = Point::ZERO;
        for id in path {
            if let Some(element) = element_tree.find_element_mut(*id) {
                let pos = element.position();
                acc.x += pos.x;
                acc.y += pos.y;
            }
            origins.push(acc);
        }
    }

    fn path_origin_from_ids(element_tree: &mut ElementTree, path: &[ElementId]) -> Option<Point> {
        if path.is_empty() {
            return None;
        }
        let mut acc = Point::ZERO;
        for id in path {
            let element = element_tree.find_element_mut(*id)?;
            let pos = element.position();
            acc.x += pos.x;
            acc.y += pos.y;
        }
        Some(acc)
    }

    fn localize_mouse_event(
        event: &crate::event::MouseEvent,
        origin: Point,
    ) -> crate::event::MouseEvent {
        match *event {
            crate::event::MouseEvent::Moved { x, y } => crate::event::MouseEvent::Moved {
                x: x - origin.x as i32,
                y: y - origin.y as i32,
            },
            crate::event::MouseEvent::Entered { x, y } => crate::event::MouseEvent::Entered {
                x: x - origin.x as i32,
                y: y - origin.y as i32,
            },
            crate::event::MouseEvent::Exited { x, y } => crate::event::MouseEvent::Exited {
                x: x - origin.x as i32,
                y: y - origin.y as i32,
            },
            crate::event::MouseEvent::ButtonPressed {
                button,
                x,
                y,
                click_count,
            } => crate::event::MouseEvent::ButtonPressed {
                button,
                x: x - origin.x as i32,
                y: y - origin.y as i32,
                click_count,
            },
            crate::event::MouseEvent::ButtonReleased {
                button,
                x,
                y,
                click_count,
            } => crate::event::MouseEvent::ButtonReleased {
                button,
                x: x - origin.x as i32,
                y: y - origin.y as i32,
                click_count,
            },
            crate::event::MouseEvent::Wheel {
                delta_x,
                delta_y,
                x,
                y,
                phase,
                source,
            } => crate::event::MouseEvent::Wheel {
                delta_x,
                delta_y,
                x: x - origin.x as i32,
                y: y - origin.y as i32,
                phase,
                source,
            },
        }
    }
}

impl Default for EventDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementRenderObject, LayoutConstraints, UpdateResult};
    use crate::event::{MouseEvent, ScrollSource, WheelPhase};
    use crate::geometry::Size;
    use crate::view::View;
    use alloc::boxed::Box;
    use alloc::rc::Rc;
    use core::any::Any;
    use core::cell::Cell;

    struct WheelTestElement {
        id: ElementId,
        position: Point,
        size: Size,
        handles_wheel: bool,
        wheel_count: Rc<Cell<u32>>,
        render_object: WheelCaptureRenderObject,
        children: Vec<Box<dyn Element>>,
    }

    struct WheelCaptureRenderObject {
        size: Size,
        captures_wheel: bool,
    }

    impl ElementRenderObject for WheelCaptureRenderObject {
        fn layout(&mut self, _constraints: LayoutConstraints) -> Size {
            self.size
        }

        fn size(&self) -> Size {
            self.size
        }

        fn render(&mut self) {}

        fn captures_wheel_event(&self, event: &MouseEvent) -> bool {
            self.captures_wheel && matches!(event, MouseEvent::Wheel { .. })
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    impl WheelTestElement {
        fn new(
            id: u32,
            position: Point,
            size: Size,
            handles_wheel: bool,
            captures_wheel: bool,
            wheel_count: Rc<Cell<u32>>,
            children: Vec<Box<dyn Element>>,
        ) -> Self {
            Self {
                id: ElementId::new(id),
                position,
                size,
                handles_wheel,
                wheel_count,
                render_object: WheelCaptureRenderObject {
                    size,
                    captures_wheel,
                },
                children,
            }
        }
    }

    impl Element for WheelTestElement {
        fn id(&self) -> ElementId {
            self.id
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }

        fn children(&self) -> &[Box<dyn Element>] {
            &self.children
        }

        fn children_mut(&mut self) -> &mut [Box<dyn Element>] {
            &mut self.children
        }

        fn update(&mut self, _new_view: &dyn View) -> UpdateResult {
            UpdateResult::NoChange
        }

        fn rebuild(&mut self) -> UpdateResult {
            UpdateResult::NoChange
        }

        fn layout(&mut self, _constraints: LayoutConstraints) -> Size {
            self.size
        }

        fn position(&self) -> Point {
            self.position
        }

        fn set_position(&mut self, position: Point) {
            self.position = position;
        }

        fn bounds(&self) -> Rect {
            Rect::new(self.position, self.size)
        }

        fn hit_test(&self, point: Point) -> bool {
            self.bounds().contains(point)
        }

        fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
            if !self.handles_wheel || !matches!(phase, Phase::Target | Phase::Bubble) {
                return false;
            }
            if matches!(event, Event::Mouse(MouseEvent::Wheel { .. })) {
                self.wheel_count.set(self.wheel_count.get() + 1);
                return true;
            }
            false
        }

        fn render_object(&self) -> Option<&dyn ElementRenderObject> {
            Some(&self.render_object)
        }
    }

    fn wheel_event(y: i32, phase: WheelPhase) -> Event {
        wheel_event_with_source(y, phase, ScrollSource::Trackpad)
    }

    fn wheel_event_with_source(y: i32, phase: WheelPhase, source: ScrollSource) -> Event {
        Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: 40,
            x: 10,
            y,
            phase,
            source,
        })
    }

    fn mouse_moved(y: i32) -> Event {
        Event::Mouse(MouseEvent::Moved { x: 10, y })
    }

    fn nested_wheel_tree(outer_count: Rc<Cell<u32>>, inner_count: Rc<Cell<u32>>) -> ElementTree {
        nested_wheel_tree_with_inner_behavior(outer_count, inner_count, true)
    }

    fn nested_wheel_tree_with_inner_behavior(
        outer_count: Rc<Cell<u32>>,
        inner_count: Rc<Cell<u32>>,
        inner_handles_wheel: bool,
    ) -> ElementTree {
        nested_wheel_tree_with_ids(1, 2, 3, outer_count, inner_count, inner_handles_wheel)
    }

    fn nested_wheel_tree_with_ids(
        root_id: u32,
        outer_id: u32,
        inner_id: u32,
        outer_count: Rc<Cell<u32>>,
        inner_count: Rc<Cell<u32>>,
        inner_handles_wheel: bool,
    ) -> ElementTree {
        let inner = Box::new(WheelTestElement::new(
            inner_id,
            Point::new(0.0, 50.0),
            Size::new(100.0, 50.0),
            inner_handles_wheel,
            true,
            inner_count,
            Vec::new(),
        ));
        let outer = Box::new(WheelTestElement::new(
            outer_id,
            Point::ZERO,
            Size::new(100.0, 100.0),
            true,
            true,
            outer_count,
            alloc::vec![inner],
        ));
        let root = Box::new(WheelTestElement::new(
            root_id,
            Point::ZERO,
            Size::new(100.0, 100.0),
            false,
            false,
            Rc::new(Cell::new(0)),
            alloc::vec![outer],
        ));

        let mut tree = ElementTree::new();
        tree.set_root(root);
        tree
    }

    #[test]
    fn wheel_capture_keeps_initial_handler_even_when_pointer_moves_over_child() {
        let outer_count = Rc::new(Cell::new(0));
        let inner_count = Rc::new(Cell::new(0));
        let mut tree = nested_wheel_tree(outer_count.clone(), inner_count.clone());
        let mut dispatcher = EventDispatcher::new();

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(10, WheelPhase::Started)));
        assert_eq!(outer_count.get(), 1);
        assert_eq!(inner_count.get(), 0);

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Moved)));
        assert_eq!(outer_count.get(), 2);
        assert_eq!(inner_count.get(), 0);

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Started)));
        assert_eq!(outer_count.get(), 3);
        assert_eq!(inner_count.get(), 0);
    }

    #[test]
    fn discrete_wheel_locks_to_initial_handler_across_nested_scroll_views() {
        let outer_count = Rc::new(Cell::new(0));
        let inner_count = Rc::new(Cell::new(0));
        let mut tree = nested_wheel_tree(outer_count.clone(), inner_count.clone());
        let mut dispatcher = EventDispatcher::new();

        // Start scrolling over the outer region.
        assert!(dispatcher.dispatch(
            &mut tree,
            &wheel_event_with_source(10, WheelPhase::Moved, ScrollSource::Wheel)
        ));
        assert_eq!(outer_count.get(), 1);
        assert_eq!(inner_count.get(), 0);

        // Moving the pointer over the nested child must NOT retarget: the
        // outer scroll view keeps monopolizing the gesture until the user
        // stops scrolling (see `WHEEL_GESTURE_IDLE`).
        assert!(dispatcher.dispatch(
            &mut tree,
            &wheel_event_with_source(60, WheelPhase::Moved, ScrollSource::Wheel)
        ));
        assert_eq!(outer_count.get(), 2);
        assert_eq!(inner_count.get(), 0);
        assert_eq!(dispatcher.wheel_target, Some(ElementId::new(2)));
    }

    #[test]
    fn discrete_wheel_sticks_to_actual_handler_not_hit_leaf() {
        let outer_count = Rc::new(Cell::new(0));
        let inner_count = Rc::new(Cell::new(0));
        let mut tree =
            nested_wheel_tree_with_inner_behavior(outer_count.clone(), inner_count.clone(), false);
        let mut dispatcher = EventDispatcher::new();

        assert!(dispatcher.dispatch(
            &mut tree,
            &wheel_event_with_source(60, WheelPhase::Moved, ScrollSource::Wheel)
        ));

        assert_eq!(outer_count.get(), 1);
        assert_eq!(inner_count.get(), 0);
        assert_eq!(dispatcher.wheel_target, Some(ElementId::new(2)));
    }

    #[test]
    fn discrete_wheel_does_not_bubble_from_sticky_target_at_edge() {
        let outer_count = Rc::new(Cell::new(0));
        let inner_count = Rc::new(Cell::new(0));
        let mut tree = nested_wheel_tree(outer_count.clone(), inner_count.clone());
        let mut dispatcher = EventDispatcher::new();

        assert!(dispatcher.dispatch(
            &mut tree,
            &wheel_event_with_source(60, WheelPhase::Moved, ScrollSource::Wheel)
        ));
        assert_eq!(outer_count.get(), 0);
        assert_eq!(inner_count.get(), 1);
        assert_eq!(dispatcher.wheel_target, Some(ElementId::new(3)));

        let inner = tree
            .find_element_mut(ElementId::new(3))
            .and_then(|element| element.as_any_mut().downcast_mut::<WheelTestElement>())
            .expect("inner test element should exist");
        inner.handles_wheel = false;

        assert!(dispatcher.dispatch(
            &mut tree,
            &wheel_event_with_source(60, WheelPhase::Moved, ScrollSource::Wheel)
        ));
        assert_eq!(outer_count.get(), 0);
        assert_eq!(inner_count.get(), 1);
    }

    #[test]
    fn wheel_capture_can_start_from_moved_phase() {
        let outer_count = Rc::new(Cell::new(0));
        let inner_count = Rc::new(Cell::new(0));
        let mut tree = nested_wheel_tree(outer_count.clone(), inner_count.clone());
        let mut dispatcher = EventDispatcher::new();

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(10, WheelPhase::Moved)));
        assert_eq!(outer_count.get(), 1);
        assert_eq!(inner_count.get(), 0);

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Moved)));
        assert_eq!(outer_count.get(), 2);
        assert_eq!(inner_count.get(), 0);

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Ended)));
        assert_eq!(outer_count.get(), 3);
        assert_eq!(inner_count.get(), 0);

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Moved)));
        assert_eq!(outer_count.get(), 3);
        assert_eq!(inner_count.get(), 1);
    }

    #[test]
    fn wheel_capture_ignores_mouse_move_until_wheel_phase_ends() {
        let outer_count = Rc::new(Cell::new(0));
        let inner_count = Rc::new(Cell::new(0));
        let mut tree = nested_wheel_tree(outer_count.clone(), inner_count.clone());
        let mut dispatcher = EventDispatcher::new();

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(10, WheelPhase::Started)));
        assert_eq!(outer_count.get(), 1);
        assert_eq!(inner_count.get(), 0);

        assert!(!dispatcher.dispatch(&mut tree, &mouse_moved(60)));
        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Moved)));
        assert_eq!(outer_count.get(), 2);
        assert_eq!(inner_count.get(), 0);

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Ended)));
        assert_eq!(outer_count.get(), 3);
        assert_eq!(inner_count.get(), 0);

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Started)));
        assert_eq!(outer_count.get(), 3);
        assert_eq!(inner_count.get(), 1);
    }

    #[test]
    fn wheel_capture_does_not_retarget_when_captured_element_disappears_mid_gesture() {
        let old_outer_count = Rc::new(Cell::new(0));
        let old_inner_count = Rc::new(Cell::new(0));
        let mut tree = nested_wheel_tree_with_ids(
            1,
            2,
            3,
            old_outer_count.clone(),
            old_inner_count.clone(),
            true,
        );
        let mut dispatcher = EventDispatcher::new();

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(10, WheelPhase::Started)));
        assert_eq!(old_outer_count.get(), 1);
        assert_eq!(old_inner_count.get(), 0);

        let new_outer_count = Rc::new(Cell::new(0));
        let new_inner_count = Rc::new(Cell::new(0));
        tree = nested_wheel_tree_with_ids(
            10,
            20,
            30,
            new_outer_count.clone(),
            new_inner_count.clone(),
            true,
        );

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Moved)));
        assert_eq!(new_outer_count.get(), 0);
        assert_eq!(new_inner_count.get(), 0);

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Ended)));
        assert_eq!(new_outer_count.get(), 0);
        assert_eq!(new_inner_count.get(), 0);

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Started)));
        assert_eq!(new_outer_count.get(), 0);
        assert_eq!(new_inner_count.get(), 1);
    }

    #[test]
    fn wheel_capture_does_not_bubble_to_parent_when_selected_target_is_at_edge() {
        let outer_count = Rc::new(Cell::new(0));
        let inner_count = Rc::new(Cell::new(0));
        let mut tree =
            nested_wheel_tree_with_inner_behavior(outer_count.clone(), inner_count.clone(), false);
        let mut dispatcher = EventDispatcher::new();

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Started)));
        assert_eq!(outer_count.get(), 0);
        assert_eq!(inner_count.get(), 0);
    }
}
