//! View trait - the core abstraction for ScarletUI
//!
//! Views are blueprints for UI elements. They implement the Factory Method pattern
//! where create_element() manufactures the corresponding Element.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use crate::element::Element;

/// Stable identity attached to a View among its siblings.
///
/// Unkeyed children are reconciled by position. Keyed children can move among
/// siblings without losing their existing Element and RenderObject state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ViewKey {
    /// Numeric identity, commonly used for model identifiers.
    Value(u64),
    /// Human-readable identity.
    Named(String),
}

impl From<u64> for ViewKey {
    fn from(value: u64) -> Self {
        Self::Value(value)
    }
}

impl From<u32> for ViewKey {
    fn from(value: u32) -> Self {
        Self::Value(value as u64)
    }
}

impl From<usize> for ViewKey {
    fn from(value: usize) -> Self {
        Self::Value(value as u64)
    }
}

impl From<String> for ViewKey {
    fn from(value: String) -> Self {
        Self::Named(value)
    }
}

impl From<&str> for ViewKey {
    fn from(value: &str) -> Self {
        Self::Named(String::from(value))
    }
}

/// Object-safe cloning support for type-erased Views.
pub trait ViewClone {
    /// Clone this View into a type-erased View value.
    fn clone_view(&self) -> Box<dyn View>;
}

/// Factory trait for creating UI elements
///
/// Views are immutable descriptions of UI that create Elements when mounted.
/// This follows the Factory Method pattern combined with the Component pattern.
pub trait View: Any + ViewClone {
    /// Create an Element from this View
    ///
    /// This is called when the View is first mounted into the element tree.
    fn create_element(&self) -> Box<dyn Element>;

    /// Get all Listenable dependencies (State references) from this View
    ///
    /// The framework uses this to track when the View needs to rebuild.
    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        Vec::new()
    }

    /// Return this View's identity among siblings, when explicitly keyed.
    fn key(&self) -> Option<&ViewKey> {
        None
    }

    /// Get this View as Any for downcasting
    fn as_any(&self) -> &dyn Any;

    /// Get the TypeId of this View (for Reconciliation)
    ///
    /// This is used during reconciliation to check if two Views are of the same type.
    fn type_id(&self) -> core::any::TypeId {
        core::any::TypeId::of::<Self>()
    }

    /// Get the type name of this View (for debugging)
    fn type_name(&self) -> &str {
        core::any::type_name::<Self>()
    }
}

/// Blanket implementation for Clone + View types
impl<V: View + Clone + 'static> ViewClone for V {
    fn clone_view(&self) -> Box<dyn View> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn View> {
    fn clone(&self) -> Self {
        self.clone_view()
    }
}

/// ViewExt trait - SwiftUI-style view modifiers
///
/// This trait provides convenient methods for applying modifiers to views.
/// It's automatically implemented for all types that implement View.
///
/// # Example
///
/// ```no_run
/// use scarlet_ui_core::prelude::*;
///
/// let view = Text::new("Hello")
///     .padding(10.0)
///     .background(Color::BLUE)
///     .frame(200.0, 50.0);
/// ```
pub trait ViewExt: View {
    /// Assign stable sibling identity to this view.
    ///
    /// Keys are only needed when children of the same parent can be inserted,
    /// removed, or reordered while their runtime state must follow the model.
    fn key<K>(self, key: K) -> crate::views::Keyed<Self>
    where
        Self: Sized + Clone,
        K: Into<ViewKey>,
    {
        crate::views::Keyed::new(self, key.into())
    }

    /// Add padding around this view
    ///
    /// # Arguments
    /// * `insets` - Uniform padding value for all edges
    fn padding(self, insets: f32) -> crate::views::Padding<Self>
    where
        Self: Sized,
    {
        crate::views::Padding::new(self, insets)
    }

    /// Add padding with custom insets
    ///
    /// # Arguments
    /// * `insets` - EdgeInsets specifying different padding for each edge
    fn padding_insets(self, insets: crate::geometry::EdgeInsets) -> crate::views::Padding<Self>
    where
        Self: Sized,
    {
        crate::views::Padding::with_insets(self, insets)
    }

    /// Add a background color behind this view
    ///
    /// # Arguments
    /// * `color` - The background color
    fn background(self, color: crate::color::Color) -> crate::views::Background<Self>
    where
        Self: Sized,
    {
        crate::views::Background::new(self, color)
    }

    /// Set a fixed frame size for this view
    ///
    /// # Arguments
    /// * `width` - The fixed width
    /// * `height` - The fixed height
    fn frame(self, width: f32, height: f32) -> crate::views::Frame<Self>
    where
        Self: Sized,
    {
        crate::views::Frame::new(self, width, height)
    }

    /// Set a fixed width for this view
    ///
    /// # Arguments
    /// * `width` - The fixed width
    fn frame_width(self, width: f32) -> crate::views::Frame<Self>
    where
        Self: Sized,
    {
        crate::views::Frame::width(self, width)
    }

    /// Set a fixed height for this view
    ///
    /// # Arguments
    /// * `height` - The fixed height
    fn frame_height(self, height: f32) -> crate::views::Frame<Self>
    where
        Self: Sized,
    {
        crate::views::Frame::height(self, height)
    }

    /// Set size constraints for this view
    ///
    /// # Arguments
    /// * `min_width` - Minimum width constraint
    /// * `min_height` - Minimum height constraint
    /// * `max_width` - Maximum width constraint
    /// * `max_height` - Maximum height constraint
    fn size_constraints(
        self,
        min_width: f32,
        min_height: f32,
        max_width: f32,
        max_height: f32,
    ) -> crate::views::SetSize<Self>
    where
        Self: Sized,
    {
        crate::views::SetSize::new(self, min_width, min_height, max_width, max_height)
    }

    /// Align this view within its container
    ///
    /// # Arguments
    /// * `alignment` - The alignment to apply
    fn alignment(self, alignment: crate::geometry::Alignment) -> crate::views::AlignmentFrame<Self>
    where
        Self: Sized,
    {
        crate::views::AlignmentFrame::new(self, alignment)
    }

    /// Add a click handler to this view
    ///
    /// # Arguments
    /// * `callback` - Function to call when clicked
    fn on_click<F: Fn() + Clone + 'static>(
        self,
        callback: F,
    ) -> crate::views::modifiers::OnClick<Self, F>
    where
        Self: Sized,
    {
        crate::views::modifiers::OnClick::new(self, callback)
    }

    /// Add a hover enter handler to this view
    ///
    /// # Arguments
    /// * `callback` - Function to call when mouse enters
    fn on_hover<F: Fn() + Clone + 'static>(
        self,
        callback: F,
    ) -> crate::views::modifiers::OnHover<Self, F>
    where
        Self: Sized,
    {
        crate::views::modifiers::OnHover::new(self, callback)
    }

    /// Add a hover exit handler to this view
    ///
    /// # Arguments
    /// * `callback` - Function to call when mouse exits
    fn on_exit<F: Fn() + Clone + 'static>(
        self,
        callback: F,
    ) -> crate::views::modifiers::OnExit<Self, F>
    where
        Self: Sized,
    {
        crate::views::modifiers::OnExit::new(self, callback)
    }

    /// Observe pointer movement within this view without consuming the event.
    fn on_mouse_move<F: Fn(i32, i32) + Clone + 'static>(
        self,
        callback: F,
    ) -> crate::views::modifiers::OnMouseMove<Self, F>
    where
        Self: Sized,
    {
        crate::views::modifiers::OnMouseMove::new(self, callback)
    }

    /// Observe relative pointer motion delivered while OS pointer lock is active.
    ///
    /// The most recently pressed view carrying this modifier becomes the
    /// relative-motion owner when the platform confirms pointer lock.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function called with horizontal and vertical deltas
    ///
    /// # Returns
    ///
    /// A modifier that observes pointer-lock motion.
    fn on_mouse_delta<F: Fn(i32, i32) + Clone + 'static>(
        self,
        callback: F,
    ) -> crate::views::modifiers::OnMouseDelta<Self, F>
    where
        Self: Sized,
    {
        crate::views::modifiers::OnMouseDelta::new(self, callback)
    }

    /// Handle mouse button presses and releases within this view.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function called with the button and `true` for press;
    ///   return `true` to consume the event
    ///
    /// # Returns
    ///
    /// A modifier that handles button presses and releases.
    fn on_mouse_button<F: Fn(crate::event::MouseButton, bool) -> bool + Clone + 'static>(
        self,
        callback: F,
    ) -> crate::views::modifiers::OnMouseButton<Self, F>
    where
        Self: Sized,
    {
        crate::views::modifiers::OnMouseButton::new(self, callback)
    }

    /// Add a keyboard handler to this view
    ///
    /// # Arguments
    /// * `callback` - Function to call for keyboard events. Return `true` to consume the event.
    fn on_key<F: Fn(crate::event::KeyEvent) -> bool + Clone + 'static>(
        self,
        callback: F,
    ) -> crate::views::modifiers::OnKey<Self, F>
    where
        Self: Sized,
    {
        crate::views::modifiers::OnKey::new(self, callback)
    }

    /// Make this view accept keyboard focus.
    ///
    /// # Arguments
    /// * `focused` - State that tracks whether the view is focused.
    fn focusable(
        self,
        focused: crate::state::State<bool>,
    ) -> crate::views::modifiers::Focusable<Self>
    where
        Self: Sized,
    {
        crate::views::modifiers::Focusable::new(self, focused)
    }

    /// Cache this view's rendered subtree behind a repaint boundary.
    ///
    /// Ancestor repaints can reuse the cached subtree and composite it at the
    /// boundary's current position. Changes inside the boundary invalidate the
    /// cache and repaint the subtree.
    fn repaint_boundary(self) -> crate::views::modifiers::RepaintBoundary<Self>
    where
        Self: Sized,
    {
        crate::views::modifiers::RepaintBoundary::new(self)
    }

    /// Clip this view to its bounds
    fn clip(self) -> crate::views::modifiers::Clip<Self>
    where
        Self: Sized,
    {
        crate::views::modifiers::Clip::new(self, 0.0)
    }

    /// Clip this view to rounded corners
    ///
    /// # Arguments
    /// * `radius` - Corner radius
    fn clip_radius(self, radius: f32) -> crate::views::modifiers::Clip<Self>
    where
        Self: Sized,
    {
        crate::views::modifiers::Clip::new(self, radius)
    }

    /// Draw a sharp border over this view.
    ///
    /// The border is painted on top of the view and kept inside its bounds. It
    /// is separate from clipping: use `.clip_radius()` together with
    /// `.border_rounded()` for a rounded view with a matching border.
    ///
    /// # Arguments
    /// * `color` - Border color
    /// * `width` - Border width
    fn border(self, color: crate::color::Color, width: f32) -> crate::views::modifiers::Border<Self>
    where
        Self: Sized,
    {
        crate::views::modifiers::Border::new(self, color, width)
    }

    /// Draw a rounded border over this view.
    ///
    /// # Arguments
    /// * `color` - Border color
    /// * `width` - Border width
    /// * `radius` - Corner radius
    fn border_rounded(
        self,
        color: crate::color::Color,
        width: f32,
        radius: f32,
    ) -> crate::views::modifiers::Border<Self>
    where
        Self: Sized,
    {
        crate::views::modifiers::Border::with_corner_radius(self, color, width, radius)
    }
}

/// Blanket implementation of ViewExt for all View types
///
/// This makes all modifier methods available on any View implementation.
impl<V: View> ViewExt for V {}
