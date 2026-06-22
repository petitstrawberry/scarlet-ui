//! Event types and handling for ScarletUI

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

    /// Window resize event
    Resize { width: u32, height: u32 },

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

/// Mouse events
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MouseEvent {
    /// Mouse moved
    Moved { x: i32, y: i32 },
    /// Mouse entered an element
    Entered { x: i32, y: i32 },
    /// Mouse exited an element
    Exited { x: i32, y: i32 },

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
    /// Returns the platform primary modifier state.
    ///
    /// Ctrl and Super are both treated as primary so callers can use one shortcut
    /// path across Linux, Windows, macOS, and Scarlet OS.
    ///
    /// # Returns
    ///
    /// `true` when either Control or Super is pressed.
    pub fn primary(self) -> bool {
        self.control || self.super_key
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
