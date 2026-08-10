//! Event modifier views
//!
//! Provides event modifiers for any view.

use crate::element::{Element, ElementRenderObject, RenderElement, UpdateResult};
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
        Self { inner, focused }
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
            |view| FocusableRenderObject::new(view.focused.clone()),
            update_focusable_render_object::<V>,
            |view| vec![view.inner.clone_view()],
        ))
    }

    fn listenables(&self) -> alloc::vec::Vec<&dyn Listenable> {
        let mut listenables = self.inner.listenables();
        listenables.push(&self.focused);
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
    UpdateResult::Updated
}

/// Render object for [`Focusable`].
pub struct FocusableRenderObject {
    focused: State<bool>,
    size: Size,
}

impl FocusableRenderObject {
    /// Create a new focusable render object.
    pub fn new(focused: State<bool>) -> Self {
        Self {
            focused,
            size: Size::ZERO,
        }
    }

    /// Return whether this object is currently focused.
    pub fn is_focused(&self) -> bool {
        self.focused.get()
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
    UpdateResult::Updated
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
    UpdateResult::Updated
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
}
