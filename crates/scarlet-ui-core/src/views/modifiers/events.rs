//! Event modifier views
//!
//! Provides event modifiers for any view.

use crate::element::{
    Element, ElementRenderObject, RenderElement, TextInputElementState, UpdateResult,
};
use crate::event::{Event, FocusEvent, KeyEvent, MouseEvent, Phase};
use crate::geometry::Size;
use crate::state::{Listenable, State};
use crate::view::View;
use alloc::boxed::Box;
use alloc::vec;
use core::any::Any;

/// Click event modifier - adds click handler to any view
#[derive(Clone)]
pub struct OnClick<V: View, F: Clone + 'static> {
    inner: V,
    callback: F,
}

/// Focusable modifier - makes any view accept keyboard focus.
#[derive(Clone)]
pub struct Focusable<V: View> {
    inner: V,
    focused: State<bool>,
    text_input: Option<State<Option<TextInputElementState>>>,
}

impl<V: View> Focusable<V> {
    /// Create a new Focusable modifier.
    ///
    /// # Arguments
    ///
    /// * `inner` - Wrapped view.
    /// * `focused` - State that tracks focus.
    ///
    /// # Returns
    ///
    /// A focusable wrapper for the view.
    pub fn new(inner: V, focused: State<bool>) -> Self {
        Self {
            inner,
            focused,
            text_input: None,
        }
    }

    /// Expose the caret and surrounding text of an embedded editor to the IME.
    /// `None` disables text input while the wrapper retains keyboard focus.
    pub fn text_input(mut self, state: State<Option<TextInputElementState>>) -> Self {
        self.text_input = Some(state);
        self
    }

    /// Return the focus state.
    pub fn focused_state(&self) -> &State<bool> {
        &self.focused
    }
}

impl<V: View + Clone> View for Focusable<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| {
                let mut object = FocusableRenderObject::new(view.focused.clone());
                object.text_input = view.text_input.clone();
                object
            },
            update_focusable_render_object::<V>,
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn Listenable> {
        let mut listenables = self.inner.listenables();
        listenables.push(&self.focused);
        if let Some(text_input) = &self.text_input {
            listenables.push(text_input);
        }
        listenables
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn update_focusable_render_object<V: View>(
    render_object: &mut FocusableRenderObject,
    view: &Focusable<V>,
) -> UpdateResult {
    render_object.focused = view.focused.clone();
    render_object.text_input = view.text_input.clone();
    UpdateResult::Updated
}

/// Render object for [`Focusable`].
pub struct FocusableRenderObject {
    focused: State<bool>,
    text_input: Option<State<Option<TextInputElementState>>>,
    size: Size,
}

impl FocusableRenderObject {
    /// Create a new focusable render object.
    pub fn new(focused: State<bool>) -> Self {
        Self {
            focused,
            text_input: None,
            size: Size::ZERO,
        }
    }

    /// Return whether this object is currently focused.
    pub fn is_focused(&self) -> bool {
        self.focused.get()
    }

    pub(crate) fn text_input_state(&self) -> Option<TextInputElementState> {
        if !self.is_focused() {
            return None;
        }
        self.text_input.as_ref()?.get()
    }

    /// Apply a focus event.
    pub fn handle_focus(&self, event: FocusEvent) -> bool {
        match event {
            FocusEvent::Gained => self.focused.set(true),
            FocusEvent::Lost => self.focused.set(false),
        }
        true
    }
}

impl ElementRenderObject for FocusableRenderObject {
    fn layout(&mut self, _constraints: crate::element::LayoutConstraints) -> Size {
        Size::ZERO
    }

    fn layout_with_children(
        &mut self,
        constraints: crate::element::LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        if let Some(child) = children.first_mut() {
            let size = child.layout(constraints);
            self.size = size;
            size
        } else {
            self.size = Size::ZERO;
            Size::ZERO
        }
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: crate::geometry::Point) -> bool {
        point.x >= 0.0 && point.y >= 0.0 && point.x < self.size.width && point.y < self.size.height
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {}
}

impl<V: View, F: Fn() + Clone + 'static> OnClick<V, F> {
    /// Create a new OnClick modifier
    pub fn new(inner: V, callback: F) -> Self {
        Self { inner, callback }
    }

    /// Get the inner view
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Get the callback
    pub fn callback(&self) -> &F {
        &self.callback
    }

    /// Invoke the click callback
    pub fn invoke_on_click(&self) {
        (self.callback)();
    }
}

impl<V: View + Clone, F: Fn() + Clone + 'static> View for OnClick<V, F> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| {
                let mut render_object = OnClickRenderObject::new();
                render_object.set_callback(Box::new(view.callback.clone()));
                render_object
            },
            update_on_click_render_object::<V, F>,
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn update_on_click_render_object<V, F>(
    render_object: &mut OnClickRenderObject,
    view: &OnClick<V, F>,
) -> UpdateResult
where
    V: View,
    F: Fn() + Clone + 'static,
{
    render_object.set_callback(Box::new(view.callback.clone()));
    // Updating a callback changes input behavior, not the child's pixels.
    UpdateResult::NoChange
}

/// Click RenderObject
pub struct OnClickRenderObject {
    is_hovered: bool,
    callback: Option<Box<dyn Fn()>>,
    size: Size,
}

impl OnClickRenderObject {
    pub fn new() -> Self {
        Self {
            is_hovered: false,
            callback: None,
            size: Size::ZERO,
        }
    }

    fn set_callback(&mut self, callback: Box<dyn Fn()>) {
        self.callback = Some(callback);
    }

    pub fn invoke_on_click(&self) {
        if let Some(ref cb) = self.callback {
            cb();
        }
    }
}

impl ElementRenderObject for OnClickRenderObject {
    fn layout(&mut self, _constraints: crate::element::LayoutConstraints) -> Size {
        Size::ZERO
    }

    fn layout_with_children(
        &mut self,
        constraints: crate::element::LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        if let Some(child) = children.first_mut() {
            let size = child.layout(constraints);
            self.size = size;
            if crate::debug::is_enabled() {
                crate::logln!(
                    "[OnClickRenderObject::layout_with_children] size={}x{}",
                    size.width,
                    size.height
                );
            }
            size
        } else {
            self.size = Size::ZERO;
            Size::ZERO
        }
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: crate::geometry::Point) -> bool {
        let result = point.x >= 0.0
            && point.x < self.size.width
            && point.y >= 0.0
            && point.y < self.size.height;
        if crate::debug::is_enabled() {
            crate::logln!(
                "[OnClickRenderObject::hit_test] point=({:?}), size={:?}, result={}",
                point,
                self.size,
                result
            );
        }
        result
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        // Modifier doesn't directly render
    }
}

/// Hover event modifier - adds hover enter handler to any view
#[derive(Clone)]
pub struct OnHover<V: View, F: Clone + 'static> {
    inner: V,
    callback: F,
}

impl<V: View, F: Fn() + Clone + 'static> OnHover<V, F> {
    /// Create a new OnHover modifier
    pub fn new(inner: V, callback: F) -> Self {
        Self { inner, callback }
    }

    /// Get the inner view
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Get the callback
    pub fn callback(&self) -> &F {
        &self.callback
    }
}

impl<V: View + Clone, F: Fn() + Clone + 'static> View for OnHover<V, F> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| {
                let mut render_object = OnHoverRenderObject::new();
                render_object.set_callback(Box::new(view.callback.clone()));
                render_object
            },
            update_on_hover_render_object::<V, F>,
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn update_on_hover_render_object<V, F>(
    render_object: &mut OnHoverRenderObject,
    view: &OnHover<V, F>,
) -> UpdateResult
where
    V: View,
    F: Fn() + Clone + 'static,
{
    render_object.set_callback(Box::new(view.callback.clone()));
    UpdateResult::Updated
}

/// Hover RenderObject
pub struct OnHoverRenderObject {
    is_hovered: bool,
    callback: Option<Box<dyn Fn()>>,
    size: Size,
}

impl OnHoverRenderObject {
    pub fn new() -> Self {
        Self {
            is_hovered: false,
            callback: None,
            size: Size::ZERO,
        }
    }

    fn set_callback(&mut self, callback: Box<dyn Fn()>) {
        self.callback = Some(callback);
    }
}

impl ElementRenderObject for OnHoverRenderObject {
    fn layout(&mut self, _constraints: crate::element::LayoutConstraints) -> Size {
        self.size = Size::ZERO;
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: crate::element::LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        if let Some(child) = children.first_mut() {
            self.size = child.layout(constraints);
        } else {
            self.size = Size::ZERO;
        }
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: crate::geometry::Point) -> bool {
        point.x >= 0.0 && point.y >= 0.0 && point.x < self.size.width && point.y < self.size.height
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }

        match event {
            Event::Mouse(MouseEvent::Entered { .. }) if !self.is_hovered => {
                self.is_hovered = true;
                if let Some(callback) = self.callback.as_ref() {
                    callback();
                    return true;
                }
            }
            Event::Mouse(MouseEvent::Exited { .. }) => {
                self.is_hovered = false;
            }
            _ => {}
        }
        false
    }

    fn render(&mut self) {
        // Modifier doesn't directly render
    }
}

/// Exit event modifier - adds hover exit handler to any view
#[derive(Clone)]
pub struct OnExit<V: View, F: Clone + 'static> {
    inner: V,
    callback: F,
}

impl<V: View, F: Fn() + Clone + 'static> OnExit<V, F> {
    /// Create a new OnExit modifier
    pub fn new(inner: V, callback: F) -> Self {
        Self { inner, callback }
    }

    /// Get the inner view
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Get the callback
    pub fn callback(&self) -> &F {
        &self.callback
    }
}

impl<V: View + Clone, F: Fn() + Clone + 'static> View for OnExit<V, F> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| {
                let mut render_object = OnExitRenderObject::new();
                render_object.set_callback(Box::new(view.callback.clone()));
                render_object
            },
            update_on_exit_render_object::<V, F>,
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Mouse-move event modifier - observes pointer movement without consuming it.
#[derive(Clone)]
pub struct OnMouseMove<V: View, F: Clone + 'static> {
    inner: V,
    callback: F,
}

/// Relative pointer-motion modifier for OS-level pointer lock.
///
/// A pointer press within this modifier registers it as the candidate owner.
/// When the platform subsequently confirms pointer lock, relative motion is
/// routed exclusively to that registered element until lock is released.
#[derive(Clone)]
pub struct OnMouseDelta<V: View, F: Clone + 'static> {
    inner: V,
    callback: F,
}

impl<V: View, F: Fn(i32, i32) + Clone + 'static> OnMouseDelta<V, F> {
    /// Create a relative pointer-motion modifier.
    ///
    /// # Arguments
    ///
    /// * `inner` - Wrapped view
    /// * `callback` - Function called with horizontal and vertical deltas
    ///
    /// # Returns
    ///
    /// A new [`OnMouseDelta`] modifier.
    pub fn new(inner: V, callback: F) -> Self {
        Self { inner, callback }
    }
}

impl<V: View + Clone, F: Fn(i32, i32) + Clone + 'static> View for OnMouseDelta<V, F> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| OnMouseDeltaRenderObject {
                callback: Box::new(view.callback.clone()),
                size: Size::ZERO,
            },
            |render_object, view| {
                render_object.callback = Box::new(view.callback.clone());
                UpdateResult::Updated
            },
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object for [`OnMouseDelta`].
pub struct OnMouseDeltaRenderObject {
    callback: Box<dyn Fn(i32, i32)>,
    size: Size,
}

impl ElementRenderObject for OnMouseDeltaRenderObject {
    fn layout(&mut self, _constraints: crate::element::LayoutConstraints) -> Size {
        self.size = Size::ZERO;
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: crate::element::LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        self.size = children
            .first_mut()
            .map_or(Size::ZERO, |child| child.layout(constraints));
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: crate::geometry::Point) -> bool {
        point.x >= 0.0 && point.y >= 0.0 && point.x < self.size.width && point.y < self.size.height
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if matches!(phase, Phase::Target | Phase::Bubble)
            && let Event::Mouse(MouseEvent::RelativeMotion { dx, dy }) = event
        {
            (self.callback)(*dx, *dy);
        }
        false
    }

    fn render(&mut self) {}
}

/// Mouse-button modifier for press and release events.
#[derive(Clone)]
pub struct OnMouseButton<V: View, F: Clone + 'static> {
    inner: V,
    callback: F,
}

impl<V: View, F: Fn(crate::event::MouseButton, bool) -> bool + Clone + 'static>
    OnMouseButton<V, F>
{
    /// Create a mouse-button modifier.
    ///
    /// # Arguments
    ///
    /// * `inner` - Wrapped view
    /// * `callback` - Function called with the button and `true` for press
    ///
    /// # Returns
    ///
    /// A new [`OnMouseButton`] modifier.
    pub fn new(inner: V, callback: F) -> Self {
        Self { inner, callback }
    }
}

impl<V, F> View for OnMouseButton<V, F>
where
    V: View + Clone,
    F: Fn(crate::event::MouseButton, bool) -> bool + Clone + 'static,
{
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| OnMouseButtonRenderObject {
                callback: Box::new(view.callback.clone()),
                size: Size::ZERO,
            },
            |render_object, view| {
                render_object.callback = Box::new(view.callback.clone());
                UpdateResult::Updated
            },
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Render object for [`OnMouseButton`].
pub struct OnMouseButtonRenderObject {
    callback: Box<dyn Fn(crate::event::MouseButton, bool) -> bool>,
    size: Size,
}

impl ElementRenderObject for OnMouseButtonRenderObject {
    fn layout(&mut self, _constraints: crate::element::LayoutConstraints) -> Size {
        Size::ZERO
    }

    fn layout_with_children(
        &mut self,
        constraints: crate::element::LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        self.size = children
            .first_mut()
            .map_or(Size::ZERO, |child| child.layout(constraints));
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: crate::geometry::Point) -> bool {
        point.x >= 0.0 && point.y >= 0.0 && point.x < self.size.width && point.y < self.size.height
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }
        match event {
            Event::Mouse(MouseEvent::ButtonPressed { button, .. }) => {
                (self.callback)(*button, true)
            }
            Event::Mouse(MouseEvent::ButtonReleased { button, .. }) => {
                (self.callback)(*button, false)
            }
            _ => false,
        }
    }

    fn render(&mut self) {}
}

impl<V: View, F: Fn(i32, i32) + Clone + 'static> OnMouseMove<V, F> {
    /// Create a new mouse-move modifier.
    pub fn new(inner: V, callback: F) -> Self {
        Self { inner, callback }
    }
}

impl<V: View + Clone, F: Fn(i32, i32) + Clone + 'static> View for OnMouseMove<V, F> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| {
                let mut render_object = OnMouseMoveRenderObject::new();
                render_object.set_callback(Box::new(view.callback.clone()));
                render_object
            },
            update_on_mouse_move_render_object::<V, F>,
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn update_on_mouse_move_render_object<V, F>(
    render_object: &mut OnMouseMoveRenderObject,
    view: &OnMouseMove<V, F>,
) -> UpdateResult
where
    V: View,
    F: Fn(i32, i32) + Clone + 'static,
{
    render_object.set_callback(Box::new(view.callback.clone()));
    UpdateResult::Updated
}

/// Render object for [`OnMouseMove`].
pub struct OnMouseMoveRenderObject {
    callback: Option<Box<dyn Fn(i32, i32)>>,
    size: Size,
}

impl OnMouseMoveRenderObject {
    pub fn new() -> Self {
        Self {
            callback: None,
            size: Size::ZERO,
        }
    }

    fn set_callback(&mut self, callback: Box<dyn Fn(i32, i32)>) {
        self.callback = Some(callback);
    }
}

impl ElementRenderObject for OnMouseMoveRenderObject {
    fn layout(&mut self, _constraints: crate::element::LayoutConstraints) -> Size {
        self.size = Size::ZERO;
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: crate::element::LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        if let Some(child) = children.first_mut() {
            self.size = child.layout(constraints);
        } else {
            self.size = Size::ZERO;
        }
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: crate::geometry::Point) -> bool {
        point.x >= 0.0 && point.y >= 0.0 && point.x < self.size.width && point.y < self.size.height
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }

        match event {
            Event::Mouse(MouseEvent::Entered { x, y })
            | Event::Mouse(MouseEvent::Moved { x, y }) => {
                if let Some(callback) = self.callback.as_ref() {
                    callback(*x, *y);
                }
            }
            _ => {}
        }
        false
    }

    fn render(&mut self) {}
}

fn update_on_exit_render_object<V, F>(
    render_object: &mut OnExitRenderObject,
    view: &OnExit<V, F>,
) -> UpdateResult
where
    V: View,
    F: Fn() + Clone + 'static,
{
    render_object.set_callback(Box::new(view.callback.clone()));
    UpdateResult::Updated
}

/// Exit RenderObject
pub struct OnExitRenderObject {
    is_hovered: bool,
    callback: Option<Box<dyn Fn()>>,
    size: Size,
}

impl OnExitRenderObject {
    pub fn new() -> Self {
        Self {
            is_hovered: false,
            callback: None,
            size: Size::ZERO,
        }
    }

    pub fn set_callback(&mut self, callback: Box<dyn Fn()>) {
        self.callback = Some(callback);
    }
}

impl ElementRenderObject for OnExitRenderObject {
    fn layout(&mut self, _constraints: crate::element::LayoutConstraints) -> Size {
        self.size = Size::ZERO;
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: crate::element::LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        if let Some(child) = children.first_mut() {
            self.size = child.layout(constraints);
        } else {
            self.size = Size::ZERO;
        }
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: crate::geometry::Point) -> bool {
        point.x >= 0.0 && point.y >= 0.0 && point.x < self.size.width && point.y < self.size.height
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if !matches!(phase, Phase::Target | Phase::Bubble) {
            return false;
        }

        match event {
            Event::Mouse(MouseEvent::Entered { .. }) => {
                self.is_hovered = true;
            }
            Event::Mouse(MouseEvent::Exited { .. }) if self.is_hovered => {
                self.is_hovered = false;
                if let Some(callback) = self.callback.as_ref() {
                    callback();
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn render(&mut self) {
        // Modifier doesn't directly render
    }
}

/// Keyboard event modifier - adds a key handler to any view.
#[derive(Clone)]
pub struct OnKey<V: View, F: Clone + 'static> {
    inner: V,
    callback: F,
}

/// Raw event modifier for views that provide their own input model.
#[derive(Clone)]
pub struct OnEvent<V: View, F: Clone + 'static> {
    inner: V,
    callback: F,
    phase: Phase,
}

impl<V: View, F: Fn(&Event) -> bool + Clone + 'static> OnEvent<V, F> {
    /// Create a raw event modifier for target and bubble dispatch.
    pub fn new(inner: V, callback: F) -> Self {
        Self {
            inner,
            callback,
            phase: Phase::Bubble,
        }
    }

    /// Create a raw event modifier for capture dispatch only.
    ///
    /// Returning `true` stops dispatch before the target handles the event.
    pub fn new_capture(inner: V, callback: F) -> Self {
        Self {
            inner,
            callback,
            phase: Phase::Capture,
        }
    }
}

impl<V: View + Clone, F: Fn(&Event) -> bool + Clone + 'static> View for OnEvent<V, F> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| OnEventRenderObject {
                callback: Box::new(view.callback.clone()),
                phase: view.phase,
                size: Size::ZERO,
            },
            update_on_event_render_object::<V, F>,
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn update_on_event_render_object<V, F>(
    render_object: &mut OnEventRenderObject,
    view: &OnEvent<V, F>,
) -> UpdateResult
where
    V: View,
    F: Fn(&Event) -> bool + Clone + 'static,
{
    render_object.callback = Box::new(view.callback.clone());
    render_object.phase = view.phase;
    UpdateResult::Updated
}

/// Render object for [`OnEvent`].
pub struct OnEventRenderObject {
    callback: Box<dyn Fn(&Event) -> bool>,
    phase: Phase,
    size: Size,
}

impl ElementRenderObject for OnEventRenderObject {
    fn layout(&mut self, _constraints: crate::element::LayoutConstraints) -> Size {
        Size::ZERO
    }

    fn layout_with_children(
        &mut self,
        constraints: crate::element::LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        self.size = children
            .first_mut()
            .map_or(Size::ZERO, |child| child.layout(constraints));
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: crate::geometry::Point) -> bool {
        point.x >= 0.0 && point.x < self.size.width && point.y >= 0.0 && point.y < self.size.height
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn handle_event(&mut self, event: &Event, phase: Phase) -> bool {
        if phase == self.phase || (self.phase == Phase::Bubble && phase == Phase::Target) {
            (self.callback)(event)
        } else {
            false
        }
    }

    fn render(&mut self) {}
}

impl<V: View, F: Fn(KeyEvent) -> bool + Clone + 'static> OnKey<V, F> {
    /// Create a new OnKey modifier.
    ///
    /// # Arguments
    ///
    /// * `inner` - Wrapped view.
    /// * `callback` - Function called for keyboard events.
    ///
    /// # Returns
    ///
    /// A new [`OnKey`] modifier.
    pub fn new(inner: V, callback: F) -> Self {
        Self { inner, callback }
    }

    /// Get the inner view.
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Get the callback.
    pub fn callback(&self) -> &F {
        &self.callback
    }
}

impl<V: View + Clone, F: Fn(KeyEvent) -> bool + Clone + 'static> View for OnKey<V, F> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| {
                let mut render_object = OnKeyRenderObject::new();
                render_object.set_callback(Box::new(view.callback.clone()));
                render_object
            },
            update_on_key_render_object::<V, F>,
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn crate::state::Listenable> {
        self.inner.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn update_on_key_render_object<V, F>(
    render_object: &mut OnKeyRenderObject,
    view: &OnKey<V, F>,
) -> UpdateResult
where
    V: View,
    F: Fn(KeyEvent) -> bool + Clone + 'static,
{
    render_object.set_callback(Box::new(view.callback.clone()));
    UpdateResult::NoChange
}

/// Render object for [`OnKey`].
pub struct OnKeyRenderObject {
    callback: Option<Box<dyn Fn(KeyEvent) -> bool>>,
    size: Size,
}

impl OnKeyRenderObject {
    /// Create an empty key modifier render object.
    pub fn new() -> Self {
        Self {
            callback: None,
            size: Size::ZERO,
        }
    }

    /// Set the key callback.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function called for keyboard events.
    pub fn set_callback(&mut self, callback: Box<dyn Fn(KeyEvent) -> bool>) {
        self.callback = Some(callback);
    }

    /// Invoke the key callback.
    ///
    /// # Arguments
    ///
    /// * `event` - Keyboard event.
    ///
    /// # Returns
    ///
    /// `true` when the event was consumed.
    pub fn invoke_on_key(&self, event: KeyEvent) -> bool {
        if crate::debug::is_enabled() {
            crate::logln!(
                "[OnKeyRenderObject] invoke: event={:?} has_callback={}",
                event,
                self.callback.is_some()
            );
        }
        let handled = self
            .callback
            .as_ref()
            .map(|callback| callback(event))
            .unwrap_or(false);
        if crate::debug::is_enabled() {
            crate::logln!("[OnKeyRenderObject] handled={}", handled);
        }
        handled
    }
}

impl ElementRenderObject for OnKeyRenderObject {
    fn layout(&mut self, _constraints: crate::element::LayoutConstraints) -> Size {
        Size::ZERO
    }

    fn layout_with_children(
        &mut self,
        constraints: crate::element::LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        if let Some(child) = children.first_mut() {
            let size = child.layout(constraints);
            self.size = size;
            size
        } else {
            self.size = Size::ZERO;
            Size::ZERO
        }
    }

    fn size(&self) -> Size {
        self.size
    }

    fn hit_test(&self, point: crate::geometry::Point) -> bool {
        point.x >= 0.0 && point.x < self.size.width && point.y >= 0.0 && point.y < self.size.height
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn render(&mut self) {
        // Modifier doesn't directly render.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::ViewExt;
    use crate::views::Text;
    use core::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn capture_stops_focused_text_field_before_target_and_bubble() {
        use crate::event::{KeyCode, KeyModifiers};
        use crate::pipeline::RenderingPipeline;
        use crate::views::TextField;

        let text = State::new(crate::state::generate_state_id(), "address".into());
        let phases = Rc::new(core::cell::RefCell::new(Vec::new()));
        let bubble_phases = phases.clone();
        let inner_phases = phases.clone();
        let outer_phases = phases.clone();
        let view = TextField::new(text.clone())
            .autofocus(true)
            .on_event(move |_| {
                bubble_phases.borrow_mut().push("bubble");
                true
            })
            .on_event_capture(move |event| {
                inner_phases.borrow_mut().push("inner capture");
                matches!(
                    event,
                    Event::Keyboard(KeyEvent::Char { c: 'r' })
                        | Event::Keyboard(KeyEvent::Pressed {
                            keycode: KeyCode::Left,
                            ..
                        })
                )
            })
            .padding(4.0)
            .on_event_capture(move |_| {
                outer_phases.borrow_mut().push("outer capture");
                false
            });
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(Box::new(crate::element::ComponentElement::new(view)));
        pipeline.layout_initial();

        assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: 'r' })));
        assert_eq!(text.get(), "address");
        assert_eq!(*phases.borrow(), ["outer capture", "inner capture"]);

        phases.borrow_mut().clear();
        assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Left,
            modifiers: KeyModifiers::default(),
        })));
        assert_eq!(*phases.borrow(), ["outer capture", "inner capture"]);

        phases.borrow_mut().clear();
        assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: 'a' })));
        assert_eq!(
            text.get(),
            "addressa",
            "captured Left must not move the caret"
        );
        assert_eq!(*phases.borrow(), ["outer capture", "inner capture"]);

        phases.borrow_mut().clear();
        assert!(pipeline.handle_event(&Event::Keyboard(KeyEvent::Released {
            keycode: KeyCode::Char('a'),
            modifiers: KeyModifiers::default(),
        })));
        assert_eq!(text.get(), "addressa");
        assert_eq!(
            *phases.borrow(),
            ["outer capture", "inner capture", "bubble"]
        );
    }

    #[test]
    fn capture_modifier_preserves_target_only_text_input_commits() {
        use crate::pipeline::RenderingPipeline;
        use crate::views::TextField;

        let text = State::new(crate::state::generate_state_id(), String::new());
        let phases = Rc::new(core::cell::RefCell::new(Vec::new()));
        let inner_phases = phases.clone();
        let outer_phases = phases.clone();
        let view = TextField::new(text.clone())
            .autofocus(true)
            .on_event_capture(move |event| {
                inner_phases.borrow_mut().push("inner capture");
                matches!(event, Event::TextInputCommit { text, .. } if text == "blocked")
            })
            .padding(4.0)
            .on_event_capture(move |_| {
                outer_phases.borrow_mut().push("outer capture");
                false
            });
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(Box::new(crate::element::ComponentElement::new(view)));
        pipeline.layout_initial();

        for (serial, commit, expected) in
            [(1, "blocked", "blocked"), (2, "日本語", "blocked日本語")]
        {
            phases.borrow_mut().clear();
            assert!(pipeline.handle_event(&Event::TextInputCommit {
                context_id: 1,
                serial,
                text: commit.into(),
            }));
            assert_eq!(text.get(), expected);
            assert!(
                phases.borrow().is_empty(),
                "IME commits retain target-only dispatch"
            );
        }
    }

    #[test]
    fn capture_observes_local_pointer_once_without_changing_click_or_tap() {
        use crate::event::{MouseButton, TouchChange, TouchFrame, TouchPhase};
        use crate::pipeline::RenderingPipeline;
        use crate::views::Button;

        let clicks = Rc::new(Cell::new(0));
        let clicked = clicks.clone();
        let positions = Rc::new(core::cell::RefCell::new(Vec::new()));
        let captured_positions = positions.clone();
        let view = Button::new("target")
            .on_click(move || clicked.set(clicked.get() + 1))
            .on_event_capture(move |event| {
                if let Event::Mouse(
                    MouseEvent::ButtonPressed { x, y, .. }
                    | MouseEvent::ButtonReleased { x, y, .. },
                ) = event
                {
                    captured_positions.borrow_mut().push((*x, *y));
                }
                false
            })
            .padding(4.0);
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(Box::new(crate::element::ComponentElement::new(view)));
        pipeline.layout_initial();
        for event in [
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 10,
                y: 10,
                click_count: 1,
            },
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 10,
                y: 10,
                click_count: 1,
            },
        ] {
            pipeline.handle_event(&Event::Mouse(event));
        }
        assert_eq!(*positions.borrow(), [(6, 6), (6, 6)]);
        assert_eq!(clicks.get(), 1);

        for (serial, phase) in [(1, TouchPhase::Down), (2, TouchPhase::Up)] {
            pipeline.handle_event(&Event::TouchFrame(TouchFrame {
                seat_id: 0,
                serial,
                time_ns: serial * 16_000_000,
                changes: vec![TouchChange {
                    seat_id: 0,
                    serial,
                    time_ns: serial * 16_000_000,
                    id: 1,
                    phase,
                    x: 10,
                    y: 10,
                    pressure: None,
                    touch_major: None,
                }],
            }));
        }
        assert_eq!(clicks.get(), 2, "touch release must activate exactly once");
    }

    #[test]
    fn capture_consumes_consecutive_wheels_without_becoming_the_target() {
        use crate::event::{ScrollSource, WheelPhase};
        use crate::pipeline::RenderingPipeline;

        let count = Rc::new(Cell::new(0));
        let captured = count.clone();
        let view = Text::new("wheel target")
            .frame(100.0, 50.0)
            .on_event_capture(move |event| {
                if matches!(event, Event::Mouse(MouseEvent::Wheel { .. })) {
                    captured.set(captured.get() + 1);
                    true
                } else {
                    false
                }
            });
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(view.create_element());
        pipeline.layout_initial();
        for expected in 1..=2 {
            assert!(pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
                delta_x: 0,
                delta_y: -1,
                x: 10,
                y: 10,
                source: ScrollSource::Wheel,
                phase: WheelPhase::Moved,
            })));
            assert_eq!(count.get(), expected);
        }
    }

    #[test]
    fn consumed_mouse_press_keeps_capture_on_motion_and_release_outside_bounds() {
        use crate::event::MouseButton;
        use crate::pipeline::RenderingPipeline;

        let count = Rc::new(Cell::new(0));
        let captured = count.clone();
        let view = Text::new("pointer target")
            .frame(100.0, 50.0)
            .on_event_capture(move |event| {
                if matches!(
                    event,
                    Event::Mouse(
                        MouseEvent::ButtonPressed { .. }
                            | MouseEvent::Moved { .. }
                            | MouseEvent::ButtonReleased { .. }
                    )
                ) {
                    captured.set(captured.get() + 1);
                    true
                } else {
                    false
                }
            });
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(view.create_element());
        pipeline.layout_initial();
        for (event, expected) in [
            (
                MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x: 10,
                    y: 10,
                    click_count: 1,
                },
                1,
            ),
            (MouseEvent::Moved { x: 200, y: 200 }, 2),
            (
                MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x: 200,
                    y: 200,
                    click_count: 1,
                },
                3,
            ),
        ] {
            assert!(pipeline.handle_event(&Event::Mouse(event)));
            assert_eq!(count.get(), expected);
        }
        pipeline.handle_event(&Event::Mouse(MouseEvent::Moved { x: 200, y: 200 }));
        assert_eq!(
            count.get(),
            3,
            "release must clear the pointer capture route"
        );
    }

    #[test]
    fn capture_consumes_touch_scroll_without_becoming_the_scroll_target() {
        use crate::event::{GesturePhase, TouchChange, TouchFrame, TouchGesture, TouchPhase};
        use crate::pipeline::RenderingPipeline;
        use crate::views::{ScrollView, ScrollViewRenderObject};

        let phases = Rc::new(core::cell::RefCell::new(Vec::new()));
        let captured = phases.clone();
        let view = ScrollView::new(Text::new("scroll target"))
            .content_size(100.0, 500.0)
            .frame(100.0, 100.0)
            .on_event_capture(move |event| {
                if let Event::TouchGesture(TouchGesture::Scroll { phase, .. }) = event {
                    captured.borrow_mut().push(*phase);
                    true
                } else {
                    false
                }
            });
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(view.create_element());
        pipeline.layout_initial();
        for (serial, phase, y) in [
            (1, TouchPhase::Down, 90),
            (2, TouchPhase::Move, 70),
            (3, TouchPhase::Move, 50),
            (4, TouchPhase::Up, 50),
        ] {
            pipeline.handle_event(&Event::TouchFrame(TouchFrame {
                seat_id: 0,
                serial,
                time_ns: serial * 16_000_000,
                changes: vec![TouchChange {
                    seat_id: 0,
                    serial,
                    time_ns: serial * 16_000_000,
                    id: 1,
                    phase,
                    x: 10,
                    y,
                    pressure: None,
                    touch_major: None,
                }],
            }));
        }
        assert_eq!(
            *phases.borrow(),
            [
                GesturePhase::Started,
                GesturePhase::Moved,
                GesturePhase::Ended
            ]
        );
        let root = pipeline.element_tree().root().unwrap();
        let scroll = root.children()[0].children()[0]
            .render_object()
            .unwrap()
            .as_any()
            .downcast_ref::<ScrollViewRenderObject<Text>>()
            .unwrap();
        assert_eq!(scroll.offset(), (0.0, 0.0));
        assert!(!pipeline.has_active_animation());
    }

    #[test]
    fn raw_event_modifier_preserves_default_phases_and_updates_capture_in_place() {
        fn event_view(counter: Rc<Cell<u32>>, capture: bool) -> impl View + Clone {
            let callback = move |_: &Event| {
                counter.set(counter.get() + 1);
                true
            };
            if capture {
                OnEvent::new_capture(Text::new("target"), callback)
            } else {
                OnEvent::new(Text::new("target"), callback)
            }
        }

        let first_count = Rc::new(Cell::new(0));
        let first = event_view(first_count.clone(), false);
        let mut element = first.create_element();
        let element_id = element.id();
        let render_object_address =
            element.render_object().unwrap() as *const dyn ElementRenderObject as *const ();
        let event = Event::Keyboard(KeyEvent::Char { c: 'a' });
        assert!(!element.handle_event(&event, Phase::Capture));
        assert!(element.handle_event(&event, Phase::Target));
        assert!(element.handle_event(&event, Phase::Bubble));
        assert_eq!(first_count.get(), 2);

        let second_count = Rc::new(Cell::new(0));
        let second = event_view(second_count.clone(), true);
        assert!(matches!(element.update(&second), UpdateResult::Updated));
        assert_eq!(element.id(), element_id);
        assert_eq!(
            element.render_object().unwrap() as *const dyn ElementRenderObject as *const (),
            render_object_address
        );
        assert!(!element.handle_event(&event, Phase::Target));
        assert!(!element.handle_event(&event, Phase::Bubble));
        assert!(element.handle_event(&event, Phase::Capture));
        assert_eq!(first_count.get(), 2);
        assert_eq!(second_count.get(), 1);
    }

    #[test]
    fn focused_raw_event_view_receives_keyboard_and_ime_once() {
        use crate::element::ElementTree;
        use crate::event::{EventDispatcher, KeyCode, KeyModifiers};
        let events = Rc::new(core::cell::RefCell::new(Vec::new()));
        let recorded = Rc::clone(&events);
        let focused = State::new(crate::state::generate_state_id(), true);
        let view = Text::new("embedded editor")
            .on_event(move |event| {
                recorded.borrow_mut().push(format!("{event:?}"));
                true
            })
            .focusable(focused);
        let mut tree = ElementTree::new();
        tree.set_root(view.create_element());
        let mut dispatcher = EventDispatcher::new();
        let expected = vec![
            Event::Keyboard(KeyEvent::Pressed {
                keycode: KeyCode::Char('a'),
                modifiers: KeyModifiers::default(),
            }),
            Event::Keyboard(KeyEvent::Char { c: 'a' }),
            Event::TextInputPreedit {
                context_id: 1,
                serial: 1,
                cursor_byte: 3,
                anchor_byte: 3,
                text: "あ".into(),
                spans: vec![],
            },
            Event::TextInputCommit {
                context_id: 1,
                serial: 2,
                text: "日本語".into(),
            },
        ];
        for event in &expected {
            assert!(dispatcher.dispatch(&mut tree, event));
        }
        assert_eq!(
            *events.borrow(),
            expected
                .iter()
                .map(|event| format!("{event:?}"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn embedded_text_input_state_tracks_focus_and_editor_updates() {
        let focused = State::new(crate::state::generate_state_id(), true);
        let input = State::new(
            crate::state::generate_state_id(),
            Some(TextInputElementState {
                cursor_rect: crate::geometry::Rect::new(
                    crate::geometry::Point::ZERO,
                    Size::new(1.0, 20.0),
                ),
                surrounding_text: "日本語".into(),
                cursor_byte: 9,
                anchor_byte: 0,
            }),
        );
        let mut element = Text::new("editor")
            .focusable(focused.clone())
            .text_input(input.clone())
            .create_element();
        assert_eq!(
            element.text_input_state().unwrap().surrounding_text,
            "日本語"
        );
        assert!(element.handle_event(&Event::Focus(FocusEvent::Lost), Phase::Target));
        assert!(element.text_input_state().is_none());
        assert!(element.handle_event(&Event::Focus(FocusEvent::Gained), Phase::Target));
        assert_eq!(element.text_input_state().unwrap().cursor_byte, 9);
        input.set(None);
        assert!(element.text_input_state().is_none());
    }

    fn hover_view(counter: Rc<Cell<u32>>) -> impl View + Clone {
        Text::new("hover target").on_hover(move || counter.set(counter.get() + 1))
    }

    #[test]
    fn compatible_update_preserves_hover_state_and_replaces_callback() {
        let first_count = Rc::new(Cell::new(0));
        let first = hover_view(first_count.clone());
        let mut element = first.create_element();
        let element_id = element.id();
        let render_object_address = element
            .render_object()
            .expect("hover modifier should own a render object")
            as *const dyn ElementRenderObject as *const ();
        let entered = Event::Mouse(MouseEvent::Entered { x: 1, y: 1 });
        let exited = Event::Mouse(MouseEvent::Exited { x: 1, y: 1 });

        assert!(element.handle_event(&entered, Phase::Target));
        assert_eq!(first_count.get(), 1);

        let second_count = Rc::new(Cell::new(0));
        let second = hover_view(second_count.clone());
        assert!(matches!(element.update(&second), UpdateResult::Updated));
        assert_eq!(element.id(), element_id);
        assert_eq!(
            element
                .render_object()
                .expect("hover render object should be retained")
                as *const dyn ElementRenderObject as *const (),
            render_object_address
        );

        assert!(!element.handle_event(&entered, Phase::Target));
        assert_eq!(first_count.get(), 1);
        assert_eq!(second_count.get(), 0);
        assert!(!element.handle_event(&exited, Phase::Target));
        assert!(element.handle_event(&entered, Phase::Target));
        assert_eq!(second_count.get(), 1);
    }

    #[test]
    fn mouse_delta_modifier_observes_relative_motion() {
        let dx_seen = Rc::new(Cell::new(0));
        let dy_seen = Rc::new(Cell::new(0));
        let dx_callback = dx_seen.clone();
        let dy_callback = dy_seen.clone();
        let mut element = Text::new("relative target")
            .on_mouse_delta(move |dx, dy| {
                dx_callback.set(dx);
                dy_callback.set(dy);
            })
            .create_element();

        assert!(!element.handle_event(
            &Event::Mouse(MouseEvent::RelativeMotion { dx: 7, dy: -4 }),
            Phase::Target,
        ));
        assert_eq!(dx_seen.get(), 7);
        assert_eq!(dy_seen.get(), -4);
    }

    #[test]
    fn mouse_button_modifier_reports_button_state_and_consumption() {
        let seen = Rc::new(Cell::new(None));
        let callback_seen = seen.clone();
        let mut element = Text::new("button target")
            .on_mouse_button(move |button, pressed| {
                callback_seen.set(Some((button, pressed)));
                true
            })
            .create_element();

        assert!(element.handle_event(
            &Event::Mouse(MouseEvent::ButtonPressed {
                button: crate::event::MouseButton::Right,
                x: 2,
                y: 3,
                click_count: 1,
            }),
            Phase::Target,
        ));
        assert_eq!(seen.get(), Some((crate::event::MouseButton::Right, true)));
    }
}
