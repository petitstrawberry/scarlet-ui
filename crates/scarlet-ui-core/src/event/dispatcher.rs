//! Event Dispatcher - Routes events to target elements
//!
//! EventDispatcher implements hit testing and event routing through
//! the element tree with three-phase event dispatching.

use crate::clock::Instant;
use crate::element::{Element, ElementId, ElementTree};
use crate::event::{
    Event, GesturePhase, TouchArena, TouchChange, TouchDragAxis, TouchFrame, TouchGesture,
    TouchPhase,
};
use crate::geometry::{Point, Rect};
use alloc::vec::Vec;
use core::time::Duration;
use scarlet_scroll_motion::{ScrollMomentum, ScrollVelocityTracker};

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
    modifiers: crate::event::KeyModifiers,
    focused_id: Option<ElementId>,
    focused_path: Vec<ElementId>,
    /// Relative-motion handler selected by the most recent pointer press.
    pointer_lock_candidate_id: Option<ElementId>,
    /// Handler that owns relative motion while platform pointer lock is active.
    pointer_lock_target_id: Option<ElementId>,
    /// Events emitted by elements during event handling
    emitted_events: Vec<Event>,
    touch_routes: Vec<TouchRoute>,
    touch_arena: TouchArena,
    scroll_momentum: Option<MomentumRoute>,
    pointer_drag_animation: Option<ElementId>,
    text_input_reveal_path: Vec<ElementId>,
    // Outer Option means queued; inner None is a queued restoration.
    pending_text_input_reveal: Option<Option<Rect>>,
}

struct TouchRoute {
    id: u64,
    path: Vec<ElementId>,
    scrollable: bool,
    drag_axis: Option<TouchDragAxis>,
    press_bounds: Option<Rect>,
    scroll_target: Option<ElementId>,
    scroll_last_consumer: Option<ElementId>,
    scroll_consumers: Vec<ElementId>,
    velocity: ScrollVelocityTracker,
    press_repeat: Option<PressRepeatRoute>,
    press_activated: bool,
}

#[derive(Clone, Copy)]
struct PressRepeatRoute {
    target: ElementId,
    elapsed: Duration,
    next_fire: Duration,
    interval: Duration,
}

struct MomentumRoute {
    id: u64,
    target: ElementId,
    motion: ScrollMomentum,
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
            modifiers: crate::event::KeyModifiers::empty(),
            focused_id: None,
            focused_path: Vec::new(),
            pointer_lock_candidate_id: None,
            pointer_lock_target_id: None,
            emitted_events: Vec::new(),
            touch_routes: Vec::new(),
            touch_arena: TouchArena::default(),
            scroll_momentum: None,
            pointer_drag_animation: None,
            text_input_reveal_path: Vec::new(),
            pending_text_input_reveal: None,
        }
    }

    /// Set the root element ID
    pub fn set_root(&mut self, id: ElementId) {
        self.root_id = Some(id);
        self.pointer_drag_animation = None;
        self.scroll_momentum = None;
        self.text_input_reveal_path.clear();
        self.pending_text_input_reveal = None;
        self.touch_routes.clear();
        self.touch_arena = TouchArena::default();
    }

    /// Whether this window has scrolling motion or held-button repeats to advance.
    pub fn has_active_animation(&self) -> bool {
        self.scroll_momentum.is_some()
            || self.pointer_drag_animation.is_some()
            || self
                .touch_routes
                .iter()
                .any(|route| route.press_repeat.is_some())
    }

    /// Advance UI-owned motion using the elapsed time since the previous tick.
    pub fn advance_animations(
        &mut self,
        element_tree: &mut ElementTree,
        elapsed: Duration,
    ) -> bool {
        let mut advanced = self.advance_scroll_momentum(element_tree, elapsed);
        if let Some(id) = self.pointer_drag_animation.take()
            && self.left_button_down
            && self.captured_id == Some(id)
            && let Some(element) = element_tree.find_element_mut(id)
        {
            advanced |= element.advance_pointer_drag(elapsed);
            if element.pointer_drag_needs_animation() {
                self.pointer_drag_animation = Some(id);
            }
        }
        let mut repeat_targets = Vec::new();
        for route in &mut self.touch_routes {
            let Some(repeat) = route.press_repeat.as_mut() else {
                continue;
            };
            repeat.elapsed = repeat.elapsed.saturating_add(elapsed);
            if repeat.elapsed >= repeat.next_fire {
                repeat_targets.push(repeat.target);
                // Do not dump a burst of stale repeats after a delayed frame.
                repeat.next_fire = repeat.elapsed.saturating_add(repeat.interval);
            }
        }
        for target in repeat_targets {
            let Some(element) = element_tree.find_element_mut(target) else {
                continue;
            };
            advanced |= element.invoke_press_repeat();
        }
        advanced
    }

    fn advance_scroll_momentum(
        &mut self,
        element_tree: &mut ElementTree,
        elapsed: Duration,
    ) -> bool {
        let Some(mut route) = self.scroll_momentum.take() else {
            return false;
        };
        let Some(path) = element_tree.find_path_ids(route.target) else {
            return false;
        };
        let (delta_x, delta_y, active) = route.motion.advance(elapsed);
        if delta_x != 0 || delta_y != 0 {
            let (_, consumer) = self.dispatch_touch_to_path(
                element_tree,
                &path,
                &Event::TouchGesture(TouchGesture::ScrollMomentum {
                    id: route.id,
                    phase: GesturePhase::Moved,
                    delta_x,
                    delta_y,
                }),
            );
            let Some(consumer) = consumer else {
                Self::send_momentum_terminal(element_tree, &route, GesturePhase::Ended);
                return false;
            };
            if consumer != route.target {
                Self::send_momentum_terminal(element_tree, &route, GesturePhase::Ended);
                route.target = consumer;
            }
        }
        if active {
            self.scroll_momentum = Some(route);
        } else {
            Self::send_momentum_terminal(element_tree, &route, GesturePhase::Ended);
        }
        true
    }

    /// Stop a fling when a new interaction or presentation suspension takes over.
    pub fn cancel_animations(&mut self, element_tree: &mut ElementTree) {
        self.pointer_drag_animation = None;
        if let Some(route) = self.scroll_momentum.take() {
            Self::send_momentum_terminal(element_tree, &route, GesturePhase::Cancelled);
        }
        for route in &mut self.touch_routes {
            route.press_repeat = None;
        }
    }

    fn send_momentum_terminal(
        element_tree: &mut ElementTree,
        route: &MomentumRoute,
        phase: GesturePhase,
    ) {
        if let Some(element) = element_tree.find_element_mut(route.target) {
            element.handle_event(
                &Event::TouchGesture(TouchGesture::ScrollMomentum {
                    id: route.id,
                    phase,
                    delta_x: 0,
                    delta_y: 0,
                }),
                Phase::Target,
            );
        }
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
        if matches!(
            event,
            Event::Mouse(
                crate::event::MouseEvent::Wheel { .. }
                    | crate::event::MouseEvent::ButtonPressed { .. }
            ) | Event::Keyboard(crate::event::KeyEvent::Pressed { .. })
                | Event::Focus(crate::event::FocusEvent::Lost)
        ) || matches!(event, Event::TouchFrame(frame) if frame.changes.iter().any(|change| change.phase == TouchPhase::Down))
        {
            self.cancel_animations(element_tree);
        }
        let handled = match event {
            Event::ModifiersChanged(modifiers) => {
                self.modifiers = *modifiers;
                self.dispatch_text_input(element_tree, event)
            }
            Event::Quit => {
                // Quit event is handled at application level
                self.handle_quit(element_tree);
                false
            }
            Event::Resize { width, height } => {
                self.handle_resize(element_tree, *width, *height);
                false
            }
            Event::FullscreenConfigure { .. } | Event::FullscreenChanged { .. } => false,
            Event::TabletPresentationChanged { .. } | Event::InputPanelOcclusionChanged { .. } => {
                false
            }
            Event::FrameReady { .. } | Event::WindowSuspendedChanged { .. } => false,
            Event::InputEnvironmentChanged(_) => false,
            Event::PointerLockChanged { locked } => {
                self.update_pointer_lock_target(element_tree, *locked);
                false
            }
            Event::ScreenSizeChanged { .. } => false,
            Event::Mouse(mouse_event) => self.dispatch_mouse(element_tree, mouse_event),
            Event::TouchFrame(frame) => self.dispatch_touch_frame(element_tree, frame),
            Event::Touch(_) | Event::TouchGesture(_) => false,
            Event::Keyboard(key_event) => {
                if let crate::event::KeyEvent::Pressed { modifiers, .. }
                | crate::event::KeyEvent::Released { modifiers, .. } = key_event
                {
                    self.modifiers = *modifiers;
                }
                self.dispatch_keyboard(element_tree, key_event)
            }
            Event::Gamepad(_) => {
                // Route to element focus, or the window root when none exists.
                let path = self
                    .focused_id
                    .and_then(|id| element_tree.find_path_ids(id))
                    .or_else(|| element_tree.find_keyboard_focus_path_ids());
                if let Some(path) = path {
                    for id in path.iter().take(path.len().saturating_sub(1)) {
                        if element_tree
                            .find_element_mut(*id)
                            .is_some_and(|e| e.handle_event(event, Phase::Capture))
                        {
                            return true;
                        }
                    }
                    if let Some(id) = path.last() {
                        if element_tree
                            .find_element_mut(*id)
                            .is_some_and(|e| e.handle_event(event, Phase::Target))
                        {
                            return true;
                        }
                    }
                    for id in path.iter().rev().skip(1) {
                        if element_tree
                            .find_element_mut(*id)
                            .is_some_and(|e| e.handle_event(event, Phase::Bubble))
                        {
                            return true;
                        }
                    }
                    false
                } else {
                    element_tree
                        .root_mut()
                        .is_some_and(|e| e.handle_event(event, Phase::Target))
                }
            }
            Event::Focus(focus_event) => {
                if *focus_event == crate::event::FocusEvent::Lost {
                    self.modifiers = crate::event::KeyModifiers::empty();
                    if self.left_button_down {
                        let point = self.captured_point.unwrap_or(Point::ZERO);
                        self.dispatch_mouse(
                            element_tree,
                            &crate::event::MouseEvent::ButtonCancelled {
                                button: crate::event::MouseButton::Left,
                                x: point.x as i32,
                                y: point.y as i32,
                            },
                        );
                    }
                }
                self.dispatch_focus(element_tree, focus_event)
            }
            Event::Lifecycle(lifecycle_event) => {
                self.dispatch_lifecycle(element_tree, lifecycle_event)
            }
            Event::Window(_window_event) => false,
            Event::SafeAreaChanged { .. } => false,
            Event::Input(_) => {
                // Raw input events are typically handled by higher layers
                false
            }
            Event::MenuItemActivated { .. } => false,
            Event::TextInputReveal { visible_rect } => {
                self.reveal_text_input(element_tree, *visible_rect)
            }
            Event::TextInputSelection { .. }
            | Event::TextInputPreedit { .. }
            | Event::TextInputCommit { .. }
            | Event::TextInputDeleteSurroundingText { .. }
            | Event::TextInputDone { .. } => self.dispatch_text_input(element_tree, event),
            Event::Custom { .. } => {
                // Custom events can be dispatched similarly
                false
            }
        };
        self.pointer_drag_animation = self.captured_id.filter(|id| {
            self.left_button_down
                && element_tree
                    .find_element(*id)
                    .is_some_and(|element| element.pointer_drag_needs_animation())
        });
        if !self.left_button_down && self.touch_routes.is_empty() {
            if let Some(visible) = self.pending_text_input_reveal.take() {
                return self.reveal_text_input(element_tree, visible) || handled;
            }
            if !self.text_input_reveal_path.is_empty()
                && element_tree.focused_text_input_state().is_none()
            {
                return self.reset_text_input_reveal(element_tree) || handled;
            }
        }
        handled
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

    fn dispatch_touch_frame(&mut self, element_tree: &mut ElementTree, frame: &TouchFrame) -> bool {
        let mut handled = false;
        for &change in &frame.changes {
            if change.phase == TouchPhase::Down {
                let point = Point::new(change.x as f32, change.y as f32);
                let Some(path) = self.hit_test_with_path_ids(element_tree, point) else {
                    continue;
                };
                let scrollable = path.iter().any(|id| {
                    element_tree.find_element_mut(*id).is_some_and(|element| {
                        element
                            .render_object()
                            .is_some_and(|object| object.accepts_touch_scroll())
                    })
                });
                let origins = Self::path_origins(element_tree, &path);
                let drag_axis = path.iter().zip(&origins).rev().find_map(|(id, origin)| {
                    let local = Point::new(point.x - origin.x, point.y - origin.y);
                    element_tree
                        .find_element_mut(*id)
                        .and_then(|element| element.render_object())
                        .and_then(|object| object.touch_drag_axis(local))
                });
                if let Some(focus_id) = element_tree.nearest_focusable_in_path(&path)
                    && let Some(focus_path) = element_tree.find_path_ids(focus_id)
                {
                    self.set_focused_element(element_tree, focus_id, &focus_path);
                }
                self.touch_routes.retain(|route| route.id != change.id);
                let press_repeat = path.iter().rev().find_map(|id| {
                    element_tree.find_element_mut(*id).and_then(|element| {
                        element
                            .press_repeat_timing()
                            .map(|(delay, interval)| PressRepeatRoute {
                                target: *id,
                                elapsed: Duration::ZERO,
                                next_fire: delay,
                                interval,
                            })
                    })
                });
                let press_bounds = path.last().zip(origins.last()).and_then(|(id, origin)| {
                    let element = element_tree.find_element_mut(*id)?;
                    element
                        .render_object()?
                        .keeps_touch_press_within_bounds()
                        .then(|| Rect::new(*origin, element.bounds().size))
                });
                self.touch_routes.push(TouchRoute {
                    id: change.id,
                    path,
                    scrollable,
                    drag_axis,
                    press_bounds,
                    scroll_target: None,
                    scroll_last_consumer: None,
                    scroll_consumers: Vec::new(),
                    velocity: ScrollVelocityTracker::new(change.time_ns, change.x, change.y),
                    press_repeat,
                    press_activated: false,
                });
            }
            let Some(route_index) = self
                .touch_routes
                .iter()
                .position(|route| route.id == change.id)
            else {
                continue;
            };
            let path = self.touch_routes[route_index].path.clone();
            if path
                .last()
                .is_none_or(|id| element_tree.find_path_ids(*id).is_none())
            {
                let mut cancel = change;
                cancel.phase = TouchPhase::Cancel;
                self.touch_arena.process(cancel, false);
                self.touch_routes.remove(route_index);
                continue;
            }
            let scrollable = self.touch_routes[route_index].scrollable;
            let drag_axis = self.touch_routes[route_index].drag_axis;
            let press_bounds = self.touch_routes[route_index].press_bounds;
            if change.phase == TouchPhase::Move {
                self.touch_routes[route_index]
                    .velocity
                    .observe(change.time_ns, change.x, change.y);
            }
            let release_velocity = if change.phase == TouchPhase::Up {
                self.touch_routes[route_index].velocity.release_velocity(
                    change.time_ns,
                    change.x,
                    change.y,
                )
            } else {
                None
            };
            handled |= self
                .dispatch_touch_to_path(element_tree, &path, &Event::Touch(change))
                .0;
            if change.phase == TouchPhase::Down
                && let Some(target) = self.touch_routes[route_index]
                    .press_repeat
                    .map(|repeat| repeat.target)
                && let Some(element) = element_tree.find_element_mut(target)
            {
                let activated = element.invoke_press_repeat();
                self.touch_routes[route_index].press_activated = activated;
                handled |= activated;
            }
            let mut completed_scroll = false;
            for gesture in self.touch_arena.process_with_press_bounds(
                change,
                scrollable,
                drag_axis,
                press_bounds,
            ) {
                let primary_id = gesture.primary_id();
                if matches!(gesture, TouchGesture::CancelPress { .. })
                    && let Some(route) = self
                        .touch_routes
                        .iter_mut()
                        .find(|route| route.id == primary_id)
                {
                    route.press_repeat = None;
                }
                if matches!(gesture, TouchGesture::Tap { .. })
                    && self
                        .touch_routes
                        .iter()
                        .find(|route| route.id == primary_id)
                        .is_some_and(|route| route.press_activated)
                {
                    continue;
                }
                completed_scroll |= matches!(
                    gesture,
                    TouchGesture::Scroll {
                        phase: GesturePhase::Ended,
                        ..
                    }
                );
                let route_path = if let TouchGesture::Pinch {
                    first_id,
                    second_id,
                    ..
                } = gesture
                {
                    let first = self.touch_routes.iter().find(|route| route.id == first_id);
                    let second = self.touch_routes.iter().find(|route| route.id == second_id);
                    match (first, second) {
                        (Some(first), Some(second)) => {
                            let shared = first
                                .path
                                .iter()
                                .zip(&second.path)
                                .take_while(|(a, b)| a == b)
                                .count();
                            first.path[..shared].to_vec()
                        }
                        _ => Vec::new(),
                    }
                } else {
                    let Some(route) = self
                        .touch_routes
                        .iter()
                        .find(|route| route.id == gesture.primary_id())
                    else {
                        continue;
                    };
                    if matches!(gesture, TouchGesture::Scroll { .. }) {
                        if let Some(target_id) = route.scroll_target {
                            if let Some(index) = route.path.iter().position(|id| *id == target_id) {
                                route.path[..=index].to_vec()
                            } else {
                                route.path.clone()
                            }
                        } else {
                            route.path.clone()
                        }
                    } else {
                        route.path.clone()
                    }
                };
                if route_path.is_empty() {
                    continue;
                }
                // A finger resting on or panning a non-editable surface must
                // not dismiss the editor. Match mouse focus clearing only once
                // the arena resolves an actual tap, before its action runs.
                if matches!(gesture, TouchGesture::Tap { .. })
                    && element_tree
                        .nearest_focusable_in_path(&route_path)
                        .is_none()
                {
                    self.clear_focused_element(element_tree);
                }
                if matches!(
                    gesture,
                    TouchGesture::Scroll {
                        phase: GesturePhase::Ended | GesturePhase::Cancelled,
                        ..
                    }
                ) && let Some(consumers) = self
                    .touch_routes
                    .iter()
                    .find(|route| route.id == gesture.primary_id())
                    .map(|route| route.scroll_consumers.clone())
                    && !consumers.is_empty()
                {
                    for id in consumers {
                        if let Some(element) = element_tree.find_element_mut(id) {
                            handled |=
                                element.handle_event(&Event::TouchGesture(gesture), Phase::Target);
                            if let Some(action) = element.take_window_action() {
                                self.emitted_events.push(Event::Window(action));
                            }
                        }
                    }
                    continue;
                }
                let (consumed, consumer) = self.dispatch_touch_to_path(
                    element_tree,
                    &route_path,
                    &Event::TouchGesture(gesture),
                );
                handled |= consumed;
                if let TouchGesture::Scroll { phase, .. } = gesture
                    && matches!(phase, GesturePhase::Started | GesturePhase::Moved)
                    && let Some(consumer) = consumer
                    && let Some(route) = self
                        .touch_routes
                        .iter_mut()
                        .find(|route| route.id == gesture.primary_id())
                {
                    if phase == GesturePhase::Started {
                        route.scroll_target = Some(consumer);
                    }
                    route.scroll_last_consumer = Some(consumer);
                    if !route.scroll_consumers.contains(&consumer) {
                        route.scroll_consumers.push(consumer);
                    }
                }
            }
            if matches!(change.phase, TouchPhase::Up | TouchPhase::Cancel) {
                if completed_scroll
                    && let Some((vx, vy)) = release_velocity
                    && let Some(target) = self.touch_routes[route_index].scroll_last_consumer
                {
                    handled |= self.begin_momentum(element_tree, change.id, target, vx, vy);
                }
                self.touch_routes.retain(|route| route.id != change.id);
            }
        }
        handled
    }

    fn begin_momentum(
        &mut self,
        element_tree: &mut ElementTree,
        id: u64,
        target: ElementId,
        vx: f32,
        vy: f32,
    ) -> bool {
        self.cancel_animations(element_tree);
        let Some(element) = element_tree.find_element_mut(target) else {
            return false;
        };
        let started = element.handle_event(
            &Event::TouchGesture(TouchGesture::ScrollMomentum {
                id,
                phase: GesturePhase::Started,
                delta_x: 0,
                delta_y: 0,
            }),
            Phase::Target,
        );
        if started {
            self.scroll_momentum = Some(MomentumRoute {
                id,
                target,
                motion: ScrollMomentum::new(vx, vy),
            });
        }
        started
    }

    fn dispatch_touch_to_path(
        &mut self,
        element_tree: &mut ElementTree,
        path: &[ElementId],
        event: &Event,
    ) -> (bool, Option<ElementId>) {
        let origins = Self::path_origins(element_tree, path);
        for (index, id) in path.iter().take(path.len().saturating_sub(1)).enumerate() {
            if let Some(element) = element_tree.find_element_mut(*id)
                && element.handle_event(
                    &Self::localize_touch_event(event, origins[index]),
                    Phase::Capture,
                )
            {
                if let Some(action) = element.take_window_action() {
                    self.emitted_events.push(Event::Window(action));
                }
                // Keep a capture-only interceptor on the ancestor route.
                return (true, None);
            }
        }
        if let Some(id) = path.last()
            && let Some(element) = element_tree.find_element_mut(*id)
            && element.handle_event(
                &Self::localize_touch_event(event, *origins.last().unwrap_or(&Point::ZERO)),
                Phase::Target,
            )
        {
            if let Some(action) = element.take_window_action() {
                self.emitted_events.push(Event::Window(action));
            }
            return (true, Some(*id));
        }
        for (index, id) in path.iter().rev().skip(1).enumerate() {
            let origin_index = path.len().saturating_sub(2).saturating_sub(index);
            if let Some(element) = element_tree.find_element_mut(*id)
                && element.handle_event(
                    &Self::localize_touch_event(event, origins[origin_index]),
                    Phase::Bubble,
                )
            {
                if let Some(action) = element.take_window_action() {
                    self.emitted_events.push(Event::Window(action));
                }
                return (true, Some(*id));
            }
        }
        (false, None)
    }

    fn localize_touch_event(event: &Event, origin: Point) -> Event {
        match event {
            Event::Touch(change) => Event::Touch(TouchChange {
                x: change.x - origin.x as i32,
                y: change.y - origin.y as i32,
                ..*change
            }),
            Event::TouchGesture(gesture) => Event::TouchGesture(match *gesture {
                TouchGesture::Tap { id, x, y } => TouchGesture::Tap {
                    id,
                    x: x - origin.x as i32,
                    y: y - origin.y as i32,
                },
                TouchGesture::LongPress { id, x, y } => TouchGesture::LongPress {
                    id,
                    x: x - origin.x as i32,
                    y: y - origin.y as i32,
                },
                TouchGesture::Drag {
                    id,
                    phase,
                    x,
                    y,
                    delta_x,
                    delta_y,
                } => TouchGesture::Drag {
                    id,
                    phase,
                    x: x - origin.x as i32,
                    y: y - origin.y as i32,
                    delta_x,
                    delta_y,
                },
                TouchGesture::Pinch {
                    first_id,
                    second_id,
                    phase,
                    center_x,
                    center_y,
                    scale,
                } => TouchGesture::Pinch {
                    first_id,
                    second_id,
                    phase,
                    center_x: center_x - origin.x as i32,
                    center_y: center_y - origin.y as i32,
                    scale,
                },
                other => other,
            }),
            _ => event.clone(),
        }
    }

    /// Dispatch a mouse event with three-phase event handling
    fn dispatch_mouse(
        &mut self,
        element_tree: &mut ElementTree,
        event: &crate::event::MouseEvent,
    ) -> bool {
        if matches!(event, crate::event::MouseEvent::RelativeMotion { .. }) {
            return self.dispatch_relative_mouse(element_tree, event);
        }
        // 1. Hit test to find target and path
        let point = self.extract_point_from_mouse(&event);
        let is_wheel = matches!(event, crate::event::MouseEvent::Wheel { .. });
        let uses_wheel_capture = Self::wheel_uses_transaction_capture(event);
        let mut wheel_target_locked = false;
        let mut wheel_target_reused = false;
        let wheel_consumed_without_path = false;
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
                    wheel_target_reused = true;
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
                                self.clear_wheel_target();
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
        } else {
            self.hit_test_with_path_ids(element_tree, point)
        };

        // Hover follows the pointer's real hit path, independently from the
        // path used for button capture below. A captured press still receives
        // move/release events, but leaving its bounds must emit Exited so
        // controls can cancel their pressed appearance and click action.
        match event {
            crate::event::MouseEvent::Moved { x, y }
            | crate::event::MouseEvent::Entered { x, y } => {
                self.update_hover_path(element_tree, path.as_deref(), *x, *y);
            }
            crate::event::MouseEvent::Exited { x, y } => {
                self.update_hover_path(element_tree, None, *x, *y);
            }
            _ => {}
        }

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

            if let crate::event::MouseEvent::ButtonPressed { .. } = event {
                self.pointer_lock_candidate_id = path.iter().rev().copied().find(|id| {
                    element_tree.find_element_mut(*id).is_some_and(|element| {
                        element.render_object().is_some_and(|render_object| {
                            render_object
                                .as_any()
                                .is::<crate::views::modifiers::OnMouseDeltaRenderObject>()
                        })
                    })
                });
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
            let mut handled_in_capture = false;

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
                        handled_in_capture = true;
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
                    target.handle_event(&Event::ModifiersChanged(self.modifiers), Phase::Target);
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

            // A divider or scrollbar can claim a press during bubbling even
            // though a child was hit first. Deliver the whole drag to that
            // handler; otherwise the child can consume release and leave the
            // ancestor dragging after the physical button is up.
            if matches!(
                event,
                crate::event::MouseEvent::ButtonPressed {
                    button: crate::event::MouseButton::Left,
                    ..
                }
            ) && !handled_in_capture
                && let Some(id) = handled_id
                && let Some(handler_path) = element_tree.find_path_ids(id)
            {
                self.captured_id = Some(id);
                self.captured_path = handler_path;
                self.captured_point = Some(point);
            }

            // Lock the wheel gesture onto the element that actually handled
            // the event. Once locked, subsequent wheel events keep routing
            // here regardless of pointer position (see `wheel_target`), so a
            // parent scroll view does not lose capture to a nested child
            // sliding under the cursor mid-gesture.
            if is_wheel
                && handled
                // Capture-only handlers must stay ancestors on future events.
                && !handled_in_capture
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

            if uses_wheel_capture && wheel_target_reused && !handled {
                if crate::debug::wheel_log_enabled() {
                    crate::logln!("[Wheel] release stale-unhandled target={:?}", target_id);
                }
                self.clear_wheel_target();
                self.path_origins_scratch = path_origins;
                self.hit_path_scratch = path;
                return self.dispatch_mouse(element_tree, event);
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
            if matches!(
                event,
                crate::event::MouseEvent::ButtonReleased {
                    button: crate::event::MouseButton::Left,
                    ..
                } | crate::event::MouseEvent::ButtonCancelled {
                    button: crate::event::MouseButton::Left,
                    ..
                }
            ) {
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
            if matches!(
                event,
                crate::event::MouseEvent::ButtonReleased {
                    button: crate::event::MouseButton::Left,
                    ..
                } | crate::event::MouseEvent::ButtonCancelled {
                    button: crate::event::MouseButton::Left,
                    ..
                }
            ) {
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

    /// Route pointer-lock motion without hit testing absolute coordinates.
    fn dispatch_relative_mouse(
        &mut self,
        element_tree: &mut ElementTree,
        event: &crate::event::MouseEvent,
    ) -> bool {
        let Some(target_id) = self.pointer_lock_target_id else {
            return false;
        };
        let Some(path) = element_tree.find_path_ids(target_id) else {
            self.pointer_lock_target_id = None;
            return false;
        };
        let event = Event::Mouse(*event);

        for id in path.iter().take(path.len().saturating_sub(1)) {
            if element_tree
                .find_element_mut(*id)
                .is_some_and(|element| element.handle_event(&event, Phase::Capture))
            {
                return true;
            }
        }
        if element_tree
            .find_element_mut(target_id)
            .is_some_and(|target| target.handle_event(&event, Phase::Target))
        {
            return true;
        }
        for id in path.iter().rev().skip(1) {
            if element_tree
                .find_element_mut(*id)
                .is_some_and(|element| element.handle_event(&event, Phase::Bubble))
            {
                return true;
            }
        }
        false
    }

    fn update_pointer_lock_target(&mut self, element_tree: &mut ElementTree, locked: bool) {
        if !locked {
            self.pointer_lock_target_id = None;
            self.pointer_lock_candidate_id = None;
            return;
        }

        self.pointer_lock_target_id = self
            .pointer_lock_candidate_id
            .filter(|id| element_tree.find_path_ids(*id).is_some());
    }

    /// Dispatch a keyboard event
    fn dispatch_keyboard(
        &mut self,
        element_tree: &mut ElementTree,
        event: &crate::event::KeyEvent,
    ) -> bool {
        self.reconcile_keyboard_focus(element_tree);
        if let crate::event::KeyEvent::Pressed {
            keycode: crate::event::KeyCode::Tab,
            modifiers,
        } = event
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.super_key
        {
            let accepts_tab_input = self
                .current_focus_path(element_tree)
                .and_then(|path| path.last().copied())
                .and_then(|id| element_tree.find_element(id))
                .and_then(|element| element.render_object())
                .is_some_and(|render| {
                    render.as_any().is::<crate::views::TextViewRenderObject>()
                        || render
                            .as_any()
                            .downcast_ref::<crate::views::modifiers::FocusableRenderObject>()
                            .is_some_and(|control| control.accepts_tab_input())
                });
            // Multiline editors and embedded controls can own Tab input.
            // Other controls participate in the window's focus order.
            if !accepts_tab_input && self.advance_keyboard_focus(element_tree, modifiers.shift) {
                return true;
            }
        }
        let path = self
            .current_focus_path(element_tree)
            .or_else(|| element_tree.root().map(|root| alloc::vec![root.id()]));

        let Some(path) = path else {
            return false;
        };
        let Some(target_id) = path.last().copied() else {
            return false;
        };
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

    fn advance_keyboard_focus(&mut self, element_tree: &mut ElementTree, reverse: bool) -> bool {
        let candidates = element_tree.keyboard_focus_candidates();
        if candidates.is_empty() {
            return false;
        }
        let current = self
            .current_focus_path(element_tree)
            .and_then(|path| path.last().copied())
            .and_then(|id| candidates.iter().position(|candidate| *candidate == id));
        let next = match (current, reverse) {
            (Some(index), false) => (index + 1) % candidates.len(),
            (Some(0) | None, true) => candidates.len() - 1,
            (Some(index), true) => index - 1,
            (None, false) => 0,
        };
        let target_id = candidates[next];
        let Some(path) = element_tree.find_path_ids(target_id) else {
            return false;
        };
        self.set_focused_element(element_tree, target_id, &path);
        true
    }

    fn reset_text_input_reveal(&mut self, tree: &mut ElementTree) -> bool {
        self.pending_text_input_reveal = None;
        let mut changed = false;
        for id in core::mem::take(&mut self.text_input_reveal_path) {
            if let Some(element) = tree.find_element_mut(id) {
                changed |= element.reveal_text_input(None);
            }
        }
        changed
    }

    fn reveal_text_input(&mut self, tree: &mut ElementTree, visible: Option<Rect>) -> bool {
        let visible = visible.filter(|r| {
            r.size.width > 0.0
                && r.size.height > 0.0
                && r.left().is_finite()
                && r.top().is_finite()
                && r.right().is_finite()
                && r.bottom().is_finite()
        });
        // Neither reveal nor restoration may move a pressed target before its
        // release action. A real window FocusLost clears directly instead.
        if self.left_button_down || !self.touch_routes.is_empty() {
            self.pending_text_input_reveal = Some(visible);
            return false;
        }
        self.pending_text_input_reveal = None;
        let Some(visible) = visible else {
            return self.reset_text_input_reveal(tree);
        };
        let Some(path) = tree.find_keyboard_focus_path_ids() else {
            return self.reset_text_input_reveal(tree);
        };
        // Preserve offsets/allowances on shared ancestors. Resetting them before
        // every request would unnecessarily undo a user's still-visible position.
        let previous = core::mem::take(&mut self.text_input_reveal_path);
        let mut changed = false;
        for id in &previous {
            if !path.contains(id)
                && let Some(element) = tree.find_element_mut(*id)
            {
                changed |= element.reveal_text_input(None);
            }
        }
        self.scroll_momentum = None;
        for (index, id) in path.iter().enumerate().rev() {
            let Some(state) = tree.focused_text_input_state() else {
                break;
            };
            let origins = Self::path_origins(tree, &path);
            let mut clipped = visible;
            for (ancestor, origin) in path[..=index].iter().zip(&origins) {
                if let Some((clip, _)) = tree
                    .find_element_mut(*ancestor)
                    .and_then(|e| e.render_object())
                    .and_then(|r| r.clip_bounds(*origin))
                {
                    let left = clipped.left().max(clip.left());
                    let top = clipped.top().max(clip.top());
                    clipped = Rect::from_xywh(
                        left,
                        top,
                        (clipped.right().min(clip.right()) - left).max(0.0),
                        (clipped.bottom().min(clip.bottom()) - top).max(0.0),
                    );
                }
            }
            let origin = origins[index];
            let Some(element) = tree.find_element_mut(*id) else {
                continue;
            };
            // An inner viewport can itself be below the keyboard. Reveal into
            // its own viewport first; an outer scroll ancestor then moves it.
            if clipped.size.width <= 0.0 || clipped.size.height <= 0.0 {
                let Some((clip, _)) = element.render_object().and_then(|r| r.clip_bounds(origin))
                else {
                    continue;
                };
                clipped = clip;
            }
            if clipped.size.width <= 0.0 || clipped.size.height <= 0.0 {
                continue;
            }
            let local = |r: Rect| {
                Rect::from_xywh(
                    r.left() - origin.x,
                    r.top() - origin.y,
                    r.size.width,
                    r.size.height,
                )
            };
            let moved = element.reveal_text_input(Some((local(state.cursor_rect), local(clipped))));
            changed |= moved;
            if moved || previous.contains(id) {
                self.text_input_reveal_path.push(*id);
            }
        }
        changed
    }

    fn dispatch_text_input(&mut self, element_tree: &mut ElementTree, event: &Event) -> bool {
        self.reconcile_keyboard_focus(element_tree);
        let path = self.current_focus_path(element_tree);

        let Some(path) = path else {
            return false;
        };
        let Some(target_id) = path.last().copied() else {
            return false;
        };
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
        if *event == crate::event::FocusEvent::Lost {
            self.reset_text_input_reveal(element_tree);
        }
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
        if let Some(mut overlay) = self.hit_test_select_overlay_recursive_ids(root, point) {
            overlay.reverse();
            path.extend(overlay);
            return true;
        }
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
            crate::event::MouseEvent::RelativeMotion { .. } => Point::ZERO,
            crate::event::MouseEvent::ButtonPressed { x, y, .. } => Point {
                x: *x as f32,
                y: *y as f32,
            },
            crate::event::MouseEvent::ButtonReleased { x, y, .. } => Point {
                x: *x as f32,
                y: *y as f32,
            },
            crate::event::MouseEvent::ButtonCancelled { x, y, .. } => Point {
                x: *x as f32,
                y: *y as f32,
            },
            crate::event::MouseEvent::Wheel { x, y, .. } => Point {
                x: *x as f32,
                y: *y as f32,
            },
        }
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
            .is_some_and(|last| last.elapsed() > WHEEL_GESTURE_IDLE)
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

    pub(crate) fn set_focused_element(
        &mut self,
        element_tree: &mut ElementTree,
        target_id: ElementId,
        path: &[ElementId],
    ) {
        for old_target_id in element_tree.keyboard_focus_claims() {
            if old_target_id != target_id
                && let Some(old_target) = element_tree.find_element_mut(old_target_id)
            {
                let _ = old_target
                    .handle_event(&Event::Focus(crate::event::FocusEvent::Lost), Phase::Target);
            }
        }

        self.focused_id = Some(target_id);
        self.focused_path = path.to_vec();
        if !element_tree.element_wants_keyboard_focus(target_id)
            && let Some(target) = element_tree.find_element_mut(target_id)
        {
            let _ = target.handle_event(
                &Event::Focus(crate::event::FocusEvent::Gained),
                Phase::Target,
            );
        }
    }

    fn clear_focused_element(&mut self, element_tree: &mut ElementTree) {
        for old_target_id in element_tree.keyboard_focus_claims() {
            if let Some(old_target) = element_tree.find_element_mut(old_target_id) {
                let _ = old_target
                    .handle_event(&Event::Focus(crate::event::FocusEvent::Lost), Phase::Target);
            }
        }
        self.focused_id = None;
        self.focused_path.clear();
    }

    /// Keep programmatic focus, visual caret state and the dispatch route in
    /// agreement. A new request takes precedence over the retained target. If
    /// several are mounted together, the last declared request wins; this also
    /// lets an autofocus child take precedence over its focusable wrapper.
    pub(crate) fn reconcile_keyboard_focus(&mut self, element_tree: &mut ElementTree) {
        let claims = element_tree.keyboard_focus_claims();
        let requested = claims
            .iter()
            .rev()
            .copied()
            .find(|id| Some(*id) != self.focused_id)
            .or_else(|| self.focused_id.filter(|id| claims.contains(id)));
        // When an editor disappears or explicitly blurs, retain its nearest
        // surviving focusable ancestor. Routing straight to Window would skip
        // application capture/bubble handlers below that root.
        let target = requested.or_else(|| {
            self.focused_path.iter().rev().skip(1).copied().find(|id| {
                element_tree
                    .find_element(*id)
                    .is_some_and(|element| element.accepts_keyboard_focus())
            })
        });
        if let Some(target) = target
            && let Some(path) = element_tree.find_path_ids(target)
        {
            self.set_focused_element(element_tree, target, &path);
        } else {
            self.focused_id = None;
            self.focused_path.clear();
        }
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

    fn update_hover_path(
        &mut self,
        element_tree: &mut ElementTree,
        new_path: Option<&[ElementId]>,
        x: i32,
        y: i32,
    ) {
        let new_path = new_path.unwrap_or(&[]);
        if self.hovered_path.as_slice() == new_path {
            self.hovered_id = new_path.last().copied();
            return;
        }

        let old_path = core::mem::take(&mut self.hovered_path);
        let shared_len = old_path
            .iter()
            .zip(new_path.iter())
            .take_while(|(old, new)| old == new)
            .count();

        if crate::debug::is_enabled() {
            crate::logln!(
                "[EventDispatcher] hover change: {:?} -> {:?} shared={}",
                old_path.last(),
                new_path.last(),
                shared_len
            );
        }

        if shared_len < old_path.len() {
            Self::dispatch_hover_path_suffix(
                element_tree,
                &old_path,
                shared_len,
                &crate::event::MouseEvent::Exited { x, y },
            );
        }
        if shared_len < new_path.len() {
            Self::dispatch_hover_path_suffix(
                element_tree,
                new_path,
                shared_len,
                &crate::event::MouseEvent::Entered { x, y },
            );
        }

        self.hovered_path.extend_from_slice(new_path);
        self.hovered_id = new_path.last().copied();
    }

    fn dispatch_hover_path_suffix(
        element_tree: &mut ElementTree,
        path: &[ElementId],
        start: usize,
        event: &crate::event::MouseEvent,
    ) {
        let origins = Self::path_origins(element_tree, path);
        for index in (start..path.len()).rev() {
            let Some(element) = element_tree.find_element_mut(path[index]) else {
                continue;
            };
            let phase = if index == path.len() - 1 {
                Phase::Target
            } else {
                Phase::Bubble
            };
            let localized = Event::Mouse(Self::localize_mouse_event(event, origins[index]));
            let _ = element.handle_event(&localized, phase);
        }
    }

    fn localize_mouse_event(
        event: &crate::event::MouseEvent,
        origin: Point,
    ) -> crate::event::MouseEvent {
        match *event {
            crate::event::MouseEvent::RelativeMotion { dx, dy } => {
                crate::event::MouseEvent::RelativeMotion { dx, dy }
            }
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
            crate::event::MouseEvent::ButtonCancelled { button, x, y } => {
                crate::event::MouseEvent::ButtonCancelled {
                    button,
                    x: x - origin.x as i32,
                    y: y - origin.y as i32,
                }
            }
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
#[path = "focus_tests.rs"]
mod focus_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementRenderObject, LayoutConstraints, UpdateResult};
    use crate::event::{MouseEvent, ScrollSource, WheelPhase};
    use crate::geometry::{Rect, Size};
    use crate::view::{View, ViewExt};
    use crate::views::{Button, HStack, Rectangle};
    use alloc::boxed::Box;
    use alloc::rc::Rc;
    use core::any::Any;
    use core::cell::Cell;
    use core::cell::RefCell;

    #[test]
    fn navigation_click_after_text_view_edit_activates_on_first_release() {
        use crate::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton};
        use crate::pipeline::RenderingPipeline;
        use crate::state::{Listenable, State};
        use crate::views::{
            NavigationLink, ScrollView, Text, TextDocument, TextSelection, TextView,
        };
        #[derive(Clone)]
        struct Root {
            text: State<TextDocument>,
            selection: State<TextSelection>,
            builds: Rc<Cell<usize>>,
        }
        impl Root {
            fn body(&self) -> impl View + Clone + use<> {
                let text = self.text.clone();
                let selection = self.selection.clone();
                let nav = crate::navigation! {
                    NavigationLink::new("Inputs", move || {
                        ScrollView::new(TextView::with_document(text.clone(), selection.clone()).frame(300.0, 180.0))
                            .vertical().content_size(300.0, 560.0)
                    }),
                    NavigationLink::new("Display", || Text::new("Display page")),
                };
                if self.builds.get() == 0 {
                    nav.selected_index_state().set(0);
                }
                self.builds.set(self.builds.get() + 1);
                nav
            }
        }
        impl View for Root {
            fn create_element(&self) -> Box<dyn Element> {
                Box::new(crate::element::ComponentElement::new_with_builder(
                    self.clone(),
                    |root| Box::new(root.body()),
                ))
            }
            fn listenables(&self) -> Vec<&dyn Listenable> {
                alloc::vec![&self.text, &self.selection]
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
        fn selected_in_tree(element: &dyn Element) -> Option<usize> {
            if let Some(sidebar) = element.render_object().and_then(|render| {
                render
                    .as_any()
                    .downcast_ref::<crate::views::navigation::NavigationSidebarRenderObject>()
            }) {
                return Some(sidebar.selected_index().get());
            }
            element
                .children()
                .iter()
                .find_map(|child| selected_in_tree(child.as_ref()))
        }
        fn contains_editor(element: &dyn Element) -> bool {
            element
                .render_object()
                .is_some_and(|render| render.as_any().is::<crate::views::TextViewRenderObject>())
                || element
                    .children()
                    .iter()
                    .any(|child| contains_editor(child.as_ref()))
        }
        let _environment = crate::input_environment::install_test_input_environment(
            crate::InputEnvironment::desktop(),
        );
        for render_between_pointer_events in [false, true] {
            let text = State::new(
                crate::state::generate_state_id(),
                TextDocument::from_str("Type here..."),
            );
            let selection = State::new(crate::state::generate_state_id(), TextSelection::default());
            let builds = Rc::new(Cell::new(0));
            let root = Root {
                text: text.clone(),
                selection,
                builds: builds.clone(),
            };
            let mut pipeline = RenderingPipeline::new();
            pipeline.set_root(root.create_element());
            pipeline.layout_initial();
            pipeline.render_with_damage();
            for event in [
                MouseEvent::Moved { x: 240, y: 25 },
                MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x: 240,
                    y: 25,
                    click_count: 1,
                },
                MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x: 240,
                    y: 25,
                    click_count: 1,
                },
            ] {
                pipeline.handle_event(&Event::Mouse(event));
                if render_between_pointer_events {
                    pipeline.render_with_damage();
                }
            }
            pipeline.render_with_damage();
            for event in [
                KeyEvent::Pressed {
                    keycode: KeyCode::End,
                    modifiers: KeyModifiers::default(),
                },
                KeyEvent::Char { c: 'x' },
                KeyEvent::Pressed {
                    keycode: KeyCode::Backspace,
                    modifiers: KeyModifiers::default(),
                },
            ] {
                assert!(pipeline.handle_event(&Event::Keyboard(event)));
                pipeline.render_with_damage();
            }

            for event in [
                MouseEvent::Moved { x: 32, y: 72 },
                MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x: 32,
                    y: 72,
                    click_count: 1,
                },
                MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x: 32,
                    y: 72,
                    click_count: 1,
                },
            ] {
                pipeline.handle_event(&Event::Mouse(event));
                if render_between_pointer_events {
                    pipeline.render_with_damage();
                }
            }
            pipeline.render_with_damage();
            assert!(
                builds.get() > 1,
                "editing must rebuild fresh navigation views"
            );
            assert!(
                !contains_editor(pipeline.element_tree().root().unwrap()),
                "actual page must switch to Display"
            );
            assert_eq!(text.get().line_text(0).unwrap().as_ref(), "Type here...");
            assert_eq!(
                selected_in_tree(pipeline.element_tree().root().unwrap()),
                Some(1),
                "first click should navigate after editing"
            );
        }
    }

    struct WheelTestElement {
        id: ElementId,
        position: Point,
        size: Size,
        handles_wheel: bool,
        wheel_count: Rc<Cell<u32>>,
        render_object: WheelCaptureRenderObject,
        children: Vec<Box<dyn Element>>,
        gamepad_phases: Option<Rc<RefCell<Vec<(ElementId, Phase)>>>>,
        consume_gamepad: Option<Phase>,
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
                gamepad_phases: None,
                consume_gamepad: None,
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
            if matches!(event, Event::Gamepad(_)) {
                if let Some(phases) = &self.gamepad_phases {
                    phases.borrow_mut().push((self.id, phase));
                }
                return self.consume_gamepad == Some(phase);
            }
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

    #[test]
    fn gamepad_routes_to_element_focus_and_capture_can_consume_it() {
        let phases = Rc::new(RefCell::new(Vec::new()));
        let mut child = WheelTestElement::new(
            2,
            Point::ZERO,
            Size::new(10.0, 10.0),
            false,
            false,
            Rc::new(Cell::new(0)),
            Vec::new(),
        );
        child.gamepad_phases = Some(phases.clone());
        let mut root = WheelTestElement::new(
            1,
            Point::ZERO,
            Size::new(10.0, 10.0),
            false,
            false,
            Rc::new(Cell::new(0)),
            alloc::vec![Box::new(child)],
        );
        root.gamepad_phases = Some(phases.clone());
        let mut tree = ElementTree::new();
        tree.set_root(Box::new(root));
        let mut dispatcher = EventDispatcher::new();
        dispatcher.focused_id = Some(ElementId::new(2));
        let event = Event::Gamepad(crate::event::GamepadEvent::default());
        assert!(!dispatcher.dispatch(&mut tree, &event));
        assert_eq!(
            *phases.borrow(),
            alloc::vec![
                (ElementId::new(1), Phase::Capture),
                (ElementId::new(2), Phase::Target),
                (ElementId::new(1), Phase::Bubble)
            ]
        );
        phases.borrow_mut().clear();
        tree.root_mut()
            .unwrap()
            .as_any_mut()
            .downcast_mut::<WheelTestElement>()
            .unwrap()
            .consume_gamepad = Some(Phase::Capture);
        assert!(dispatcher.dispatch(&mut tree, &event));
        assert_eq!(
            *phases.borrow(),
            alloc::vec![(ElementId::new(1), Phase::Capture)]
        );
        phases.borrow_mut().clear();
        dispatcher.focused_id = Some(ElementId::new(99));
        assert!(!dispatcher.dispatch(&mut tree, &event));
        assert_eq!(
            *phases.borrow(),
            alloc::vec![(ElementId::new(1), Phase::Target)]
        );
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

    #[test]
    fn pointer_lock_routes_relative_motion_to_clicked_nonfocusable_canvas() {
        let first_seen = Rc::new(Cell::new(0));
        let second_seen = Rc::new(Cell::new(0));
        let first_callback = first_seen.clone();
        let second_callback = second_seen.clone();
        let root = HStack::new((
            Rectangle::new()
                .frame(50.0, 50.0)
                .on_mouse_delta(move |dx, _| first_callback.set(dx)),
            Rectangle::new()
                .frame(50.0, 50.0)
                .on_mouse_delta(move |dx, _| second_callback.set(dx)),
        ));
        let mut tree = ElementTree::new();
        tree.set_root(root.create_element());
        tree.layout(LayoutConstraints::tight(100.0, 50.0));
        let mut dispatcher = EventDispatcher::new();

        assert!(!dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::RelativeMotion { dx: 99, dy: 0 }),
        ));
        assert_eq!(first_seen.get(), 0);
        assert_eq!(second_seen.get(), 0);

        let press_first = Event::Mouse(MouseEvent::ButtonPressed {
            button: crate::event::MouseButton::Left,
            x: 25,
            y: 25,
            click_count: 1,
        });
        let _ = dispatcher.dispatch(&mut tree, &press_first);
        let _ = dispatcher.dispatch(&mut tree, &Event::PointerLockChanged { locked: true });
        assert!(!dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::RelativeMotion { dx: 13, dy: -2 }),
        ));
        assert_eq!(first_seen.get(), 13);
        assert_eq!(second_seen.get(), 0);

        let _ = dispatcher.dispatch(&mut tree, &Event::PointerLockChanged { locked: false });
        let _ = dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::ButtonReleased {
                button: crate::event::MouseButton::Left,
                x: 25,
                y: 25,
                click_count: 1,
            }),
        );
        let _ = dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::ButtonPressed {
                button: crate::event::MouseButton::Left,
                x: 75,
                y: 25,
                click_count: 1,
            }),
        );
        let _ = dispatcher.dispatch(&mut tree, &Event::PointerLockChanged { locked: true });
        assert!(!dispatcher.dispatch(
            &mut tree,
            &Event::Mouse(MouseEvent::RelativeMotion { dx: -7, dy: 4 }),
        ));
        assert_eq!(first_seen.get(), 13);
        assert_eq!(second_seen.get(), -7);
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
    fn wheel_capture_clears_target_when_captured_element_disappears_mid_gesture() {
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
        assert_eq!(new_inner_count.get(), 1);
        assert_eq!(dispatcher.wheel_target, Some(ElementId::new(30)));

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Ended)));
        assert_eq!(new_outer_count.get(), 0);
        assert_eq!(new_inner_count.get(), 2);

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Started)));
        assert_eq!(new_outer_count.get(), 0);
        assert_eq!(new_inner_count.get(), 3);
    }

    #[test]
    fn wheel_capture_reacquires_when_reused_target_stops_handling() {
        let outer_count = Rc::new(Cell::new(0));
        let inner_count = Rc::new(Cell::new(0));
        let mut tree = nested_wheel_tree(outer_count.clone(), inner_count.clone());
        let mut dispatcher = EventDispatcher::new();

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(60, WheelPhase::Started)));
        assert_eq!(outer_count.get(), 0);
        assert_eq!(inner_count.get(), 1);
        assert_eq!(dispatcher.wheel_target, Some(ElementId::new(3)));

        let inner = tree
            .find_element_mut(ElementId::new(3))
            .and_then(|element| element.as_any_mut().downcast_mut::<WheelTestElement>())
            .expect("inner test element should exist");
        inner.handles_wheel = false;

        assert!(dispatcher.dispatch(&mut tree, &wheel_event(10, WheelPhase::Moved)));
        assert_eq!(outer_count.get(), 1);
        assert_eq!(inner_count.get(), 1);
        assert_eq!(dispatcher.wheel_target, Some(ElementId::new(2)));
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

    fn repeating_button_fixture() -> (ElementTree, EventDispatcher, Rc<Cell<u32>>) {
        button_fixture(true)
    }

    fn button_fixture(repeat: bool) -> (ElementTree, EventDispatcher, Rc<Cell<u32>>) {
        let activations = Rc::new(Cell::new(0));
        let callback_activations = activations.clone();
        let button = Button::new("delete")
            .on_click(move || callback_activations.set(callback_activations.get() + 1));
        let button = if repeat {
            button.repeat_while_pressed(Duration::from_millis(420), Duration::from_millis(55))
        } else {
            button
        };
        let root = button.frame(100.0, 50.0);
        let mut tree = ElementTree::new();
        tree.set_root(root.create_element());
        tree.layout(LayoutConstraints::tight(100.0, 50.0));
        let root_id = tree.root().unwrap().id();
        let mut dispatcher = EventDispatcher::new();
        dispatcher.set_root(root_id);

        (tree, dispatcher, activations)
    }

    fn button_touch(phase: TouchPhase, time_ns: u64) -> Event {
        Event::TouchFrame(TouchFrame {
            seat_id: 1,
            serial: time_ns,
            time_ns,
            changes: alloc::vec![TouchChange {
                seat_id: 1,
                serial: time_ns,
                time_ns,
                id: 7,
                phase,
                x: 25,
                y: 25,
                pressure: None,
                touch_major: None,
            }],
        })
    }

    #[test]
    fn standard_button_releases_activate_after_in_bounds_motion_and_long_holds() {
        for (x, time_ns) in [(45, 50_000_000), (25, 650_000_000), (45, 650_000_000)] {
            let (mut tree, mut dispatcher, activations) = button_fixture(false);
            dispatcher.dispatch(&mut tree, &button_touch(TouchPhase::Down, 1));
            let mut moved = button_touch(TouchPhase::Move, 20_000_000);
            if let Event::TouchFrame(frame) = &mut moved {
                frame.changes[0].x = x;
            }
            dispatcher.dispatch(&mut tree, &moved);
            assert_eq!(activations.get(), 0);
            let mut release = button_touch(TouchPhase::Up, time_ns);
            if let Event::TouchFrame(frame) = &mut release {
                frame.changes[0].x = x;
            }
            dispatcher.dispatch(&mut tree, &release);
            assert_eq!(activations.get(), 1);
            dispatcher.dispatch(&mut tree, &release);
            assert_eq!(activations.get(), 1);
        }
    }

    #[test]
    fn standard_button_exit_then_return_never_activates() {
        let (mut tree, mut dispatcher, activations) = button_fixture(false);
        dispatcher.dispatch(&mut tree, &button_touch(TouchPhase::Down, 1));
        let mut outside = button_touch(TouchPhase::Move, 20_000_000);
        if let Event::TouchFrame(frame) = &mut outside {
            frame.changes[0].x = 101;
        }
        dispatcher.dispatch(&mut tree, &outside);
        dispatcher.dispatch(&mut tree, &button_touch(TouchPhase::Move, 30_000_000));
        dispatcher.dispatch(&mut tree, &button_touch(TouchPhase::Up, 650_000_000));
        assert_eq!(activations.get(), 0);
    }

    #[test]
    fn button_press_policy_rebuilds_preserve_enabled_and_repeat_semantics() {
        let (mut tree, mut dispatcher, activations) = button_fixture(false);
        let target = tree.keyboard_focus_candidates()[0];
        for (enabled, repeat) in [
            (true, false),
            (true, true),
            (true, false),
            (false, false),
            (true, false),
        ] {
            let count = activations.clone();
            let button = Button::new("delete")
                .enabled(enabled)
                .on_click(move || count.set(count.get() + 1));
            let button = if repeat {
                button.repeat_while_pressed(Duration::from_millis(420), Duration::from_millis(55))
            } else {
                button
            };
            tree.find_element_mut(target).unwrap().update(&button);
            let before = activations.get();
            dispatcher.dispatch(&mut tree, &button_touch(TouchPhase::Down, 1));
            assert_eq!(activations.get(), before + u32::from(enabled && repeat));
            let mut moved = button_touch(TouchPhase::Move, 20_000_000);
            if let Event::TouchFrame(frame) = &mut moved {
                frame.changes[0].x = 45;
            }
            dispatcher.dispatch(&mut tree, &moved);
            dispatcher.dispatch(&mut tree, &button_touch(TouchPhase::Up, 650_000_000));
            assert_eq!(activations.get(), before + u32::from(enabled));
        }
    }

    #[test]
    fn touch_button_activates_on_press_repeats_and_suppresses_release_tap() {
        let (mut tree, mut dispatcher, activations) = repeating_button_fixture();

        assert!(dispatcher.dispatch(&mut tree, &button_touch(TouchPhase::Down, 1)));
        assert_eq!(activations.get(), 1);
        assert!(dispatcher.has_active_animation());
        assert!(!dispatcher.advance_animations(&mut tree, Duration::from_millis(419)));
        assert_eq!(activations.get(), 1);
        assert!(dispatcher.advance_animations(&mut tree, Duration::from_millis(1)));
        assert_eq!(activations.get(), 2);
        assert!(dispatcher.advance_animations(&mut tree, Duration::from_millis(55)));
        assert_eq!(activations.get(), 3);

        assert!(dispatcher.dispatch(&mut tree, &button_touch(TouchPhase::Up, 500_000_000)));
        assert_eq!(activations.get(), 3);
        assert!(!dispatcher.has_active_animation());
    }

    #[test]
    fn touch_button_repeat_cancellation_never_reactivates_on_release() {
        for cancellation in [
            button_touch(TouchPhase::Cancel, 100_000_000),
            Event::Focus(crate::event::FocusEvent::Lost),
        ] {
            let (mut tree, mut dispatcher, activations) = repeating_button_fixture();
            dispatcher.dispatch(&mut tree, &button_touch(TouchPhase::Down, 1));
            assert_eq!(activations.get(), 1);

            dispatcher.dispatch(&mut tree, &cancellation);
            assert!(!dispatcher.has_active_animation());
            assert!(!dispatcher.advance_animations(&mut tree, Duration::from_secs(1)));
            dispatcher.dispatch(&mut tree, &button_touch(TouchPhase::Up, 200_000_000));
            assert_eq!(activations.get(), 1);
        }
    }

    #[test]
    fn touch_button_repeat_does_not_burst_after_a_delayed_frame() {
        let (mut tree, mut dispatcher, activations) = repeating_button_fixture();
        dispatcher.dispatch(&mut tree, &button_touch(TouchPhase::Down, 1));
        assert!(dispatcher.advance_animations(&mut tree, Duration::from_secs(2)));
        assert_eq!(activations.get(), 2);
        assert!(!dispatcher.advance_animations(&mut tree, Duration::from_millis(54)));
        assert!(dispatcher.advance_animations(&mut tree, Duration::from_millis(1)));
        assert_eq!(activations.get(), 3);
    }
    #[test]
    fn direct_touch_scroll_cancels_button_tap_and_cancel_allows_fresh_tap() {
        use crate::views::{ScrollView, ScrollViewRenderObject};
        let clicks = Rc::new(Cell::new(0));
        let activated = clicks.clone();
        let button =
            Button::new("touch target").on_click(move || activated.set(activated.get() + 1));
        let root = ScrollView::new(button).content_size(100.0, 500.0);
        let mut tree = ElementTree::new();
        tree.set_root(root.create_element());
        tree.layout(LayoutConstraints::tight(100.0, 100.0));
        let mut dispatcher = EventDispatcher::new();
        dispatcher.set_root(tree.root().unwrap().id());
        let mut serial = 0;
        let mut send = |tree: &mut ElementTree, phase, y| {
            serial += 1;
            let mut event = button_touch(phase, serial * 16_000_000);
            if let Event::TouchFrame(frame) = &mut event {
                frame.changes[0].y = y;
            }
            dispatcher.dispatch(tree, &event);
        };
        send(&mut tree, TouchPhase::Down, 10);
        send(&mut tree, TouchPhase::Move, -30);
        send(&mut tree, TouchPhase::Up, -30);
        assert_eq!(clicks.get(), 0, "panning from a button must not click it");
        let offset = tree
            .root()
            .unwrap()
            .render_object()
            .unwrap()
            .as_any()
            .downcast_ref::<ScrollViewRenderObject<Button>>()
            .unwrap()
            .offset()
            .1;
        assert!(
            offset > 0.0,
            "the containing scroll view must actually move"
        );

        // A separate stationary button proves cancellation is not a release,
        // and a later contact still activates normally.
        let activated = clicks.clone();
        let root = Button::new("touch target").on_click(move || activated.set(activated.get() + 1));
        tree.set_root(root.create_element());
        tree.layout(LayoutConstraints::tight(100.0, 100.0));
        send(&mut tree, TouchPhase::Down, 10);
        send(&mut tree, TouchPhase::Cancel, 10);
        send(&mut tree, TouchPhase::Up, 10);
        assert_eq!(clicks.get(), 0);
        send(&mut tree, TouchPhase::Down, 10);
        send(&mut tree, TouchPhase::Up, 10);
        assert_eq!(clicks.get(), 1);
    }
    #[test]
    fn touch_blurs_editor_only_after_resolved_non_editable_tap() {
        use crate::state::{State, generate_state_id};
        use crate::views::{ScrollView, TextField, VStack};
        let text = State::new(generate_state_id(), "keep editing".into());
        let root = VStack::new((
            TextField::new(text.clone())
                .autofocus(true)
                .frame(100.0, 30.0),
            ScrollView::new(Rectangle::new())
                .vertical()
                .content_size(100.0, 500.0)
                .frame(100.0, 100.0),
        ));
        let mut tree = ElementTree::new();
        tree.set_root(root.create_element());
        tree.layout(LayoutConstraints::tight(100.0, 130.0));
        let mut dispatcher = EventDispatcher::new();
        dispatcher.set_root(tree.root().unwrap().id());
        assert!(tree.focused_text_input_state().is_some());
        let mut serial = 0;
        let mut send = |tree: &mut ElementTree, dispatcher: &mut EventDispatcher, phase, y| {
            serial += 1;
            let mut event = button_touch(phase, serial * 16_000_000);
            if let Event::TouchFrame(frame) = &mut event {
                frame.changes[0].y = y;
            }
            dispatcher.dispatch(tree, &event);
        };
        send(&mut tree, &mut dispatcher, TouchPhase::Down, 80);
        assert!(
            tree.focused_text_input_state().is_some(),
            "pending contact retains focus"
        );
        send(&mut tree, &mut dispatcher, TouchPhase::Cancel, 80);
        assert!(
            tree.focused_text_input_state().is_some(),
            "cancel is not a tap"
        );
        send(&mut tree, &mut dispatcher, TouchPhase::Down, 80);
        send(&mut tree, &mut dispatcher, TouchPhase::Move, 40);
        send(&mut tree, &mut dispatcher, TouchPhase::Up, 40);
        assert!(
            tree.focused_text_input_state().is_some(),
            "scrolling retains focus"
        );
        send(&mut tree, &mut dispatcher, TouchPhase::Down, 80);
        send(&mut tree, &mut dispatcher, TouchPhase::Up, 80);
        assert!(
            tree.focused_text_input_state().is_none(),
            "completed blank tap dismisses editor"
        );
        assert!(!dispatcher.dispatch(
            &mut tree,
            &Event::TextInputSelection {
                cursor_byte: 0,
                anchor_byte: 0
            }
        ));
        send(&mut tree, &mut dispatcher, TouchPhase::Down, 10);
        send(&mut tree, &mut dispatcher, TouchPhase::Up, 10);
        assert!(
            tree.focused_text_input_state().is_some(),
            "editor can be focused again"
        );
        assert_eq!(text.get(), "keep editing");
    }
    #[test]
    fn dismissing_keyboard_does_not_move_button_before_touch_or_mouse_activation() {
        use crate::state::{State, generate_state_id};
        use crate::views::{ScrollView, TextField, VStack};
        fn button_center(e: &dyn Element, origin: Point) -> Option<Point> {
            let origin = Point::new(origin.x + e.position().x, origin.y + e.position().y);
            if let Some(render) = e.render_object()
                && render.as_any().is::<crate::views::ButtonRenderObject>()
            {
                return Some(Point::new(
                    origin.x + render.size().width / 2.0,
                    origin.y + render.size().height / 2.0,
                ));
            }
            e.children()
                .iter()
                .find_map(|child| button_center(child.as_ref(), origin))
        }
        for touch in [true, false] {
            let clicks = Rc::new(Cell::new(0));
            let clicked = clicks.clone();
            let root = ScrollView::new(VStack::new((
                Rectangle::new().frame(100.0, 130.0),
                Button::new("Done")
                    .on_click(move || clicked.set(clicked.get() + 1))
                    .frame(100.0, 30.0),
                TextField::new(State::new(generate_state_id(), "caret".into()))
                    .autofocus(true)
                    .frame(100.0, 40.0),
            )))
            .vertical()
            .content_size(100.0, 200.0)
            .frame(100.0, 200.0)
            .padding(10.0);
            let mut tree = ElementTree::new();
            tree.set_root(root.create_element());
            tree.layout(LayoutConstraints::tight(120.0, 220.0));
            let mut dispatcher = EventDispatcher::new();
            dispatcher.set_root(tree.root().unwrap().id());
            dispatcher.dispatch(
                &mut tree,
                &Event::TextInputReveal {
                    visible_rect: Some(Rect::from_xywh(10.0, 10.0, 100.0, 100.0)),
                },
            );
            let point = button_center(tree.root().unwrap(), Point::ZERO).unwrap();
            assert!(point.y > 10.0 && point.y < 110.0);
            let x = point.x as i32;
            let y = point.y as i32;
            if touch {
                for phase in [TouchPhase::Down, TouchPhase::Up] {
                    let mut event = button_touch(phase, 1);
                    if let Event::TouchFrame(frame) = &mut event {
                        frame.changes[0].x = x;
                        frame.changes[0].y = y;
                    }
                    dispatcher.dispatch(&mut tree, &event);
                }
            } else {
                dispatcher.dispatch(
                    &mut tree,
                    &Event::Mouse(MouseEvent::ButtonPressed {
                        button: crate::event::MouseButton::Left,
                        x,
                        y,
                        click_count: 1,
                    }),
                );
                // The DOM bridge notices focus loss immediately after Down.
                dispatcher.dispatch(&mut tree, &Event::TextInputReveal { visible_rect: None });
                dispatcher.dispatch(
                    &mut tree,
                    &Event::Mouse(MouseEvent::ButtonReleased {
                        button: crate::event::MouseButton::Left,
                        x,
                        y,
                        click_count: 1,
                    }),
                );
            }
            assert_eq!(
                clicks.get(),
                1,
                "touch={touch}: restore only after the action"
            );
            assert!(dispatcher.text_input_reveal_path.is_empty());
        }
    }

    #[test]
    fn caret_reveal_handles_fitting_content_restoration_and_active_touch() {
        use crate::state::{State, generate_state_id};
        use crate::views::{ScrollView, TextField, VStack};
        let root = ScrollView::new(VStack::new((
            Rectangle::new().frame(100.0, 160.0),
            TextField::new(State::new(generate_state_id(), "caret".into()))
                .autofocus(true)
                .frame(100.0, 40.0),
        )))
        .vertical()
        .content_size(100.0, 200.0)
        .frame(100.0, 200.0)
        .padding(10.0);
        let mut tree = ElementTree::new();
        tree.set_root(root.create_element());
        tree.layout(LayoutConstraints::tight(120.0, 220.0));
        let mut dispatcher = EventDispatcher::new();
        dispatcher.set_root(tree.root().unwrap().id());
        let original = tree.focused_text_input_state().unwrap().cursor_rect;
        assert!(original.bottom() > 110.0);
        let reveal = Event::TextInputReveal {
            visible_rect: Some(Rect::from_xywh(10.0, 10.0, 100.0, 100.0)),
        };
        let mut down = button_touch(TouchPhase::Down, 1);
        if let Event::TouchFrame(frame) = &mut down {
            frame.changes[0].y = 185;
        }
        dispatcher.dispatch(&mut tree, &down);
        assert!(
            !dispatcher.dispatch(&mut tree, &reveal),
            "do not fight a live contact"
        );
        assert_eq!(
            tree.focused_text_input_state()
                .unwrap()
                .cursor_rect
                .origin
                .y,
            original.origin.y
        );
        if let Event::TouchFrame(frame) = &mut down {
            frame.changes[0].phase = TouchPhase::Cancel;
        }
        dispatcher.dispatch(&mut tree, &down);
        assert!(
            !dispatcher.dispatch(&mut tree, &reveal),
            "pending reveal already ran after cancel"
        );
        let revealed = tree.focused_text_input_state().unwrap().cursor_rect;
        assert!(
            !dispatcher.dispatch(&mut tree, &reveal),
            "unchanged reveal is idempotent"
        );
        assert_eq!(
            tree.focused_text_input_state().unwrap().cursor_rect,
            revealed
        );
        assert!(
            revealed.top() >= 10.0 && revealed.bottom() <= 110.01,
            "{revealed:?}"
        );
        assert!(dispatcher.dispatch(
            &mut tree,
            &Event::TextInputReveal {
                visible_rect: Some(Rect::from_xywh(10.0, 10.0, 100.0, 200.0)),
            }
        ));
        assert_eq!(
            tree.focused_text_input_state()
                .unwrap()
                .cursor_rect
                .origin
                .y,
            original.origin.y
        );
        dispatcher.dispatch(&mut tree, &reveal);
        assert!(dispatcher.dispatch(&mut tree, &Event::TextInputReveal { visible_rect: None }));
        assert_eq!(
            tree.focused_text_input_state()
                .unwrap()
                .cursor_rect
                .origin
                .y,
            original.origin.y
        );
        assert!(dispatcher.text_input_reveal_path.is_empty());
        if let Event::TouchFrame(frame) = &mut down {
            frame.changes[0].phase = TouchPhase::Down;
        }
        dispatcher.dispatch(&mut tree, &down);
        dispatcher.dispatch(&mut tree, &reveal);
        assert!(dispatcher.pending_text_input_reveal.is_some());
        dispatcher.dispatch(&mut tree, &Event::TextInputReveal { visible_rect: None });
        if let Event::TouchFrame(frame) = &mut down {
            frame.changes[0].phase = TouchPhase::Cancel;
        }
        dispatcher.dispatch(&mut tree, &down);
        assert_eq!(
            tree.focused_text_input_state()
                .unwrap()
                .cursor_rect
                .origin
                .y,
            original.origin.y
        );
        assert!(dispatcher.pending_text_input_reveal.is_none());
    }

    #[test]
    fn caret_reveal_walks_nested_scrolls_and_restores_allowances_on_blur() {
        use crate::state::{State, generate_state_id};
        use crate::views::{ScrollView, TextField, VStack};
        let inner = ScrollView::new(VStack::new((
            Rectangle::new().frame(100.0, 160.0),
            TextField::new(State::new(generate_state_id(), "nested".into()))
                .autofocus(true)
                .frame(100.0, 30.0),
        )))
        .vertical()
        .content_size(100.0, 190.0)
        .frame(100.0, 100.0);
        let outer = ScrollView::new(VStack::new((Rectangle::new().frame(100.0, 120.0), inner)))
            .vertical()
            .content_size(100.0, 220.0)
            .frame(100.0, 200.0)
            .padding(10.0);
        let mut tree = ElementTree::new();
        tree.set_root(outer.create_element());
        tree.layout(LayoutConstraints::tight(120.0, 220.0));
        let mut dispatcher = EventDispatcher::new();
        dispatcher.set_root(tree.root().unwrap().id());
        let editor_id = tree.focused_text_input_state().unwrap().editor_id;
        assert!(dispatcher.dispatch(
            &mut tree,
            &Event::TextInputReveal {
                visible_rect: Some(Rect::from_xywh(10.0, 10.0, 100.0, 100.0)),
            }
        ));
        let state = tree.focused_text_input_state().unwrap();
        assert_eq!(state.editor_id, editor_id);
        assert!(
            state.cursor_rect.top() >= 10.0 && state.cursor_rect.bottom() <= 110.01,
            "{:?}",
            state.cursor_rect
        );
        assert_eq!(
            dispatcher.text_input_reveal_path.len(),
            2,
            "both scroll ancestors participate"
        );
        dispatcher.dispatch(&mut tree, &Event::Focus(crate::event::FocusEvent::Lost));
        assert!(dispatcher.text_input_reveal_path.is_empty());
        // The outer view returns to its authored maximum20, not keyboard-only120.
        fn offsets(e: &dyn Element, out: &mut Vec<f32>) {
            if e.type_name_debug().contains("ScrollViewRenderObject") {
                out.push(-e.children()[0].position().y);
            }
            for child in e.children() {
                offsets(child.as_ref(), out);
            }
        }
        let mut remaining = Vec::new();
        offsets(tree.root().unwrap(), &mut remaining);
        assert_eq!(remaining.len(), 2);
        assert!(remaining[0] <= 20.01);
        assert!(remaining[1] <= 90.01);
    }
}
