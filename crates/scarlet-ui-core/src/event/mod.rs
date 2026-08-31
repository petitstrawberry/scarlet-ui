//! Event types and handling for ScarletUI

use crate::input_environment::InputEnvironment;
use alloc::string::String;
use alloc::vec::Vec;

mod dispatcher;
mod gesture;

pub use dispatcher::{EventDispatcher, HitResult, Phase};
pub use gesture::{
    DragGestureRecognizer, Gesture, GestureManager, GestureRecognizer, LongPressGestureRecognizer,
    TapGestureRecognizer,
};

/// UI Events
#[derive(Clone, Debug)]
pub enum Event {
    /// Quit event - application should exit
    Quit,

    /// The platform's runtime input-device environment changed.
    InputEnvironmentChanged(InputEnvironment),

    /// Window resize event
    Resize { width: u32, height: u32 },

    /// Window fullscreen state changed on the platform.
    FullscreenChanged { fullscreen: bool },

    /// Window pointer-lock state changed on the platform.
    PointerLockChanged { locked: bool },

    /// Screen size changed
    ScreenSizeChanged { width: u32, height: u32 },

    /// Mouse event
    Mouse(MouseEvent),

    /// Keyboard event
    Keyboard(KeyEvent),

    /// Input event (from SWS)
    Input(InputEvent),

    /// Focus event
    Focus(FocusEvent),

    /// Lifecycle event
    Lifecycle(LifecycleEvent),

    /// Custom event with user data
    Custom { event_type: u32, data: Vec<u8> },

    /// Window control event (from Window titlebar buttons)
    Window(WindowEvent),

    /// Menu item activation (from SWS)
    MenuItemActivated {
        window_id: u32,
        menu_item_id: String,
    },

    /// IME preedit text for a text-input context.
    TextInputPreedit {
        context_id: u32,
        serial: u32,
        cursor_byte: u32,
        anchor_byte: u32,
        text: String,
        spans: Vec<u8>,
    },

    /// IME committed text for a text-input context.
    TextInputCommit {
        context_id: u32,
        serial: u32,
        text: String,
    },

    /// Request to delete surrounding text for a text-input context.
    TextInputDeleteSurroundingText {
        context_id: u32,
        serial: u32,
        before_bytes: u32,
        after_bytes: u32,
    },

    /// End of a text-input update batch.
    TextInputDone { context_id: u32, serial: u32 },
}

/// Mouse events.
///
/// Platform adapters that map a direct-touch contact into this compatibility
/// stream must end the contact with `ButtonReleased` or `ButtonCancelled`,
/// followed immediately by `Exited` at the terminal location. This guarantees
/// that the dispatcher clears hover after both activated and cancelled touch
/// contacts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MouseEvent {
    /// Mouse moved
    Moved { x: i32, y: i32 },
    /// Mouse entered an element
    Entered { x: i32, y: i32 },
    /// Mouse exited an element
    Exited { x: i32, y: i32 },

    /// Pointer motion relative to its previous position while pointer lock is active.
    RelativeMotion { dx: i32, dy: i32 },

    /// Mouse button pressed
    ButtonPressed {
        button: MouseButton,
        x: i32,
        y: i32,
        click_count: u8,
    },

    /// Mouse button released
    ButtonReleased {
        button: MouseButton,
        x: i32,
        y: i32,
        click_count: u8,
    },

    /// Mouse button interaction cancelled by the platform without activation.
    ///
    /// Direct-touch adapters emit this before the terminal [`Self::Exited`]
    /// event, so controls clear pressed state without committing an action.
    ButtonCancelled { button: MouseButton, x: i32, y: i32 },

    /// Mouse wheel scrolled.
    ///
    /// Positive deltas move the content offset right/down.
    Wheel {
        delta_x: i32,
        delta_y: i32,
        x: i32,
        y: i32,
        phase: WheelPhase,
        source: ScrollSource,
    },
}

/// Phase of a wheel or trackpad scroll gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelPhase {
    /// A new scroll gesture started.
    Started,
    /// A scroll gesture moved.
    Moved,
    /// A scroll gesture ended normally.
    Ended,
    /// A scroll gesture was cancelled by the platform.
    Cancelled,
}

/// Physical source of a scroll event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollSource {
    /// A discrete mouse wheel or wheel-like device.
    Wheel,
    /// A high-resolution touchpad gesture.
    Trackpad,
    /// A platform source that could not be classified.
    Unknown,
}

impl ScrollSource {
    /// Returns whether this source should keep a single scroll transaction target.
    ///
    /// # Returns
    ///
    /// `true` when events from this source should remain captured by the
    /// initially selected scroll view until the platform ends the gesture.
    pub const fn uses_transaction_capture(self) -> bool {
        matches!(self, Self::Trackpad)
    }
}

/// Mouse button
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// Keyboard modifier flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModifiers {
    /// Shift modifier state.
    pub shift: bool,
    /// Control modifier state.
    pub control: bool,
    /// Alt modifier state.
    pub alt: bool,
    /// Super/Command/Windows modifier state.
    pub super_key: bool,
}

impl KeyModifiers {
    /// Returns the application shortcut modifier state.
    ///
    /// Scarlet applications use Control for ordinary shortcuts. Super is
    /// reserved for desktop-global actions such as the application launcher.
    ///
    /// # Returns
    ///
    /// `true` when Control is pressed.
    pub fn primary(self) -> bool {
        self.control
    }

    /// Returns empty keyboard modifiers.
    ///
    /// # Returns
    ///
    /// A modifier set with Shift, Control, Alt, and Super all cleared.
    pub const fn empty() -> Self {
        Self {
            shift: false,
            control: false,
            alt: false,
            super_key: false,
        }
    }
}

/// Keyboard events
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyEvent {
    /// Key pressed
    Pressed {
        keycode: KeyCode,
        modifiers: KeyModifiers,
    },

    /// Key released
    Released {
        keycode: KeyCode,
        modifiers: KeyModifiers,
    },

    /// Character received (Unicode)
    Char { c: char },
}

/// Key codes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Unknown,
    Escape,
    Enter,
    Tab,
    Backspace,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
    Char(char),
}

/// Input event (from SWS input system)
#[derive(Clone, Copy, Debug)]
pub struct InputEvent {
    pub timestamp: u64,
    pub event_type: u16,
    pub code: u16,
    pub value: i32,
}

/// Window control events
///
/// Fired when user interacts with window titlebar controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEvent {
    /// Close button was clicked
    CloseRequested,
    /// Maximize button was clicked (expand to screen)
    MaximizeRequested,
    /// Restore button was clicked (restore from maximized/minimized)
    RestoreRequested,
    /// Minimize button was clicked
    MinimizeRequested,
    /// Titlebar was pressed to start interactive move
    MoveRequested,
}

/// Focus events
///
/// Fired when an element gains or loses keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusEvent {
    /// Element gained focus
    Gained,
    /// Element lost focus
    Lost,
}

/// Lifecycle events
///
/// Fired during element lifecycle: mount, unmount, appear, disappear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// Element was mounted to the tree
    Mount,
    /// Element will be unmounted from the tree
    Unmount,
    /// Element became visible on screen
    Appear,
    /// Element is no longer visible on screen
    Disappear,
}
