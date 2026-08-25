//! Platform abstraction for window systems.

use crate::buffer::Buffer;
use crate::compositor::DamageRect;
use crate::element::TextInputElementState;
use crate::error::Result;
use crate::event::Event;
use crate::geometry::{Point, Size};
use crate::renderer::{CompositorBackendKind, PaintBackend, RendererBackendKind};
use alloc::boxed::Box;
use alloc::string::String;
use core::any::Any;
use std::time::Duration;

/// Hint used by the window manager when placing a newly created window.
///
/// The compositor may adjust or ignore a placement request. Applications
/// should use this as an initial placement preference, not as ownership of
/// global screen coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowPlacement {
    /// Let the window manager apply its normal placement policy.
    #[default]
    Default,
    /// Place the window at the center of the current workarea.
    Centered,
    /// Request an absolute position in logical screen coordinates.
    At { x: i32, y: i32 },
}

/// Selects who owns the outer frame around a top-level window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowFrame {
    /// ScarletUI draws the frame inside the window surface.
    #[default]
    Custom,
    /// The platform window manager draws its standard frame.
    System,
    /// No outer frame is drawn.
    None,
}

impl WindowFrame {
    /// Return whether ScarletUI owns and draws the frame.
    pub const fn is_custom(self) -> bool {
        matches!(self, Self::Custom)
    }

    /// Return whether the platform window manager owns the frame.
    pub const fn is_system(self) -> bool {
        matches!(self, Self::System)
    }
}

/// Selects who owns the titlebar of a top-level window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowTitleBar {
    /// ScarletUI draws the titlebar and window controls.
    #[default]
    Custom,
    /// The platform window manager draws its standard titlebar and controls.
    System,
    /// No titlebar is drawn.
    None,
}

impl WindowTitleBar {
    /// Return whether ScarletUI owns and draws the titlebar.
    pub const fn is_custom(self) -> bool {
        matches!(self, Self::Custom)
    }

    /// Return whether the platform window manager owns the titlebar.
    pub const fn is_system(self) -> bool {
        matches!(self, Self::System)
    }
}

/// Independent frame and titlebar ownership for a top-level window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowDecoration {
    pub frame: WindowFrame,
    pub title_bar: WindowTitleBar,
}

impl WindowDecoration {
    /// ScarletUI draws both the frame and titlebar.
    pub const CUSTOM: Self = Self::new(WindowFrame::Custom, WindowTitleBar::Custom);
    /// The platform window manager draws both the frame and titlebar.
    pub const SYSTEM: Self = Self::new(WindowFrame::System, WindowTitleBar::System);
    /// Neither a frame nor titlebar is drawn.
    pub const NONE: Self = Self::new(WindowFrame::None, WindowTitleBar::None);

    /// Create a decoration configuration with independent ownership.
    pub const fn new(frame: WindowFrame, title_bar: WindowTitleBar) -> Self {
        Self { frame, title_bar }
    }

    /// Return whether ScarletUI draws any window chrome.
    pub const fn has_custom_chrome(self) -> bool {
        self.frame.is_custom() || self.title_bar.is_custom()
    }

    /// Return whether any visible frame or titlebar is configured.
    pub const fn is_visible(self) -> bool {
        !matches!(self.frame, WindowFrame::None) || !matches!(self.title_bar, WindowTitleBar::None)
    }
}

impl Default for WindowDecoration {
    fn default() -> Self {
        Self::CUSTOM
    }
}

/// Parameters used by a backend to create a window.
pub struct WindowCreateRequest {
    /// Stable application identifier.
    pub app_id: String,
    /// Initial window title.
    pub title: String,
    /// Initial logical window size.
    pub size: Size,
    /// Backend-specific window type.
    pub window_type: u32,
    /// Serialized top-level menu titles.
    pub menu_titles: String,
    /// Whether the window should receive focus when created.
    pub focus_on_create: bool,
    /// Whether focusing the window should activate the app.
    pub active_on_focus: bool,
    /// Whether the window contents are fully opaque.
    pub opaque: bool,
    /// Owner of the visible top-level window frame.
    pub decoration: WindowDecoration,
    /// Initial placement hint passed to the window manager.
    pub placement: WindowPlacement,
}

/// Creates platform windows for the application runner.
pub trait PlatformBackend {
    /// Return the current output scale in milli-units.
    fn output_scale_milli(&mut self) -> u32;

    /// Create a new platform window for the supplied request.
    fn create_window(&mut self, request: WindowCreateRequest) -> Result<Box<dyn PlatformWindow>>;
}

/// Platform-independent window interface
///
/// PlatformWindow abstracts platform-specific window functionality,
/// allowing ScarletUI to work with different window systems.
pub trait PlatformWindow: Any {
    /// Create a new platform window
    fn new(app_id: &str, title: &str, size: Size) -> Result<Self>
    where
        Self: Sized;

    /// Poll for events (returns None if no events available)
    fn poll_event(&mut self) -> Option<Event>;

    /// Wait until more events arrive or the timeout expires.
    fn wait_for_event(&mut self, timeout: Duration) {
        std::thread::sleep(timeout);
    }

    /// Return the window output scale in milli-units.
    fn output_scale_milli(&self) -> u32 {
        1000
    }

    /// Return the renderer selected for this window.
    ///
    /// # Returns
    ///
    /// The renderer backend used for frames presented to this window.
    fn renderer_backend(&self) -> RendererBackendKind {
        RendererBackendKind::Cpu
    }

    /// Return the compositor selected by the platform window server.
    ///
    /// # Returns
    ///
    /// The reported compositor backend, or `Unknown` when unavailable.
    fn compositor_backend(&self) -> CompositorBackendKind {
        CompositorBackendKind::Unknown
    }

    /// Take the platform-owned external paint backend, when one is available.
    ///
    /// This method is called once during window setup. Returning an error keeps
    /// strict backend selection failures visible to the application runner.
    ///
    /// # Returns
    ///
    /// An external backend, `None` for CPU rendering, or an initialization error.
    fn take_paint_backend(&mut self) -> Result<Option<Box<dyn PaintBackend>>> {
        Ok(None)
    }

    /// Present a buffer to the screen
    fn present(&mut self, buffer: &Buffer);

    /// Present a buffer to the screen with optional physical damage rectangles.
    ///
    /// # Arguments
    ///
    /// * `buffer` - Pixel buffer to present
    /// * `damage` - Physical pixel regions to update, or `None` for the whole buffer
    fn present_with_damage(&mut self, buffer: &Buffer, damage: Option<&[DamageRect]>) {
        let _ = damage;
        self.present(buffer);
    }

    /// Set the window title
    fn set_title(&mut self, title: &str);

    /// Get the window size
    fn size(&self) -> Size;

    /// Get the window backing-store size in physical pixels.
    fn physical_size(&self) -> (u32, u32) {
        let size = self.size();
        let scale_milli = self.output_scale_milli().max(1) as f32;
        (
            libm::roundf(size.width * scale_milli / 1000.0).max(1.0) as u32,
            libm::roundf(size.height * scale_milli / 1000.0).max(1.0) as u32,
        )
    }

    /// Resize the window
    fn resize(&mut self, width: u32, height: u32) -> Result<()>;

    /// Close the window
    fn close(&mut self) -> Result<()>;

    /// Minimize the window (hide it)
    fn minimize(&mut self) -> Result<()>;

    /// Maximize the window within the platform workarea.
    ///
    /// Maximized state remains distinct from fullscreen state and normally
    /// leaves shell UI such as panels visible.
    fn maximize(&mut self) -> Result<()>;

    /// Set whether the window occupies an entire output in fullscreen mode.
    ///
    /// Fullscreen remains distinct from maximized state. Implementations may
    /// reject an enabled request when another window owns the output fullscreen
    /// slot.
    ///
    /// # Arguments
    ///
    /// * `fullscreen` - `true` to enter fullscreen, or `false` to leave it
    ///
    /// # Returns
    ///
    /// `Ok(())` when the platform accepted the request.
    fn set_fullscreen(&mut self, fullscreen: bool) -> Result<()>;

    /// Request or release OS-level pointer lock for this window.
    ///
    /// # Arguments
    ///
    /// * `locked` - `true` to hide and constrain the pointer, or `false` to release it
    ///
    /// # Returns
    ///
    /// `Ok(())` when the backend accepted the request. Backends without
    /// pointer-lock support return [`crate::error::Error::PointerLockUnsupported`].
    fn set_pointer_lock(&mut self, locked: bool) -> Result<()> {
        let _ = locked;
        Err(crate::error::Error::PointerLockUnsupported)
    }

    /// Return the pointer-lock state confirmed by the platform backend.
    ///
    /// # Returns
    ///
    /// `true` only while the backend considers pointer lock active.
    fn pointer_locked(&self) -> bool {
        false
    }

    /// Restore the window from minimized or maximized state.
    ///
    /// This does not leave fullscreen state; call [`Self::set_fullscreen`]
    /// with `false` for that transition.
    fn restore(&mut self) -> Result<()>;

    /// Focus and raise the window through the platform window manager.
    ///
    /// Backends that cannot explicitly request focus may leave this as a
    /// no-op. Window managers remain free to reject the request.
    fn focus(&mut self) -> Result<()> {
        Ok(())
    }

    /// Request that the window manager begins an interactive move
    fn request_move(&mut self) -> Result<()>;

    /// Create a popup window (e.g., for dropdown menus)
    ///
    /// Returns the surface ID of the created popup window.
    fn create_popup(&mut self, position: Point, size: Size) -> Result<u32>;

    /// Destroy a popup window by surface ID
    fn destroy_popup(&mut self, surface_id: u32) -> Result<()>;

    /// Set the workarea (usable screen space excluding panels like taskbars)
    ///
    /// This informs the window manager about the area available for normal windows.
    fn set_workarea(&mut self, x: i32, y: i32, width: u32, height: u32) -> Result<()>;

    /// Create a window with a specific window type
    ///
    /// This is used to create special windows like TASKBAR, ALWAYS_ON_TOP, etc.
    fn create_window_with_type(
        &mut self,
        app_id: &str,
        title: &str,
        size: Size,
        window_type: u32,
    ) -> Result<Self>
    where
        Self: Sized;

    /// Move a window to a specific position
    fn move_window(&mut self, x: i32, y: i32) -> Result<()>;

    /// Set the window type (NORMAL, TASKBAR, ALWAYS_ON_TOP, etc.)
    fn set_window_type(&mut self, surface_id: u32, window_type: u32) -> Result<()>;

    /// Get the screen size
    fn get_screen_size(&mut self) -> Result<(u32, u32)>;

    /// Get the underlying surface ID (for SWS-specific operations)
    fn surface_id(&self) -> u32;

    /// Get the backend-native window ID as a backend-neutral integer.
    fn platform_window_id(&self) -> u64 {
        self.surface_id() as u64
    }

    /// Return mutable Any for backend-specific escape hatches.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Set whether the window is resizable
    fn set_resizable(&mut self, resizable: bool) -> Result<()>;

    /// Set whether the window contents are fully opaque.
    fn set_opaque(&mut self, opaque: bool) -> Result<()>;

    /// Update menu titles for the window (format: "menu1|menu2|menu3")
    fn set_menu_titles(&mut self, menu_titles: &str) -> Result<()>;

    /// Synchronize focused text-input state with the backend.
    fn sync_text_input(&mut self, _state: Option<&TextInputElementState>) {}

    #[cfg(feature = "std")]
    fn raw_window_handle(&self) -> Option<raw_window_handle::RawWindowHandle> {
        None
    }

    #[cfg(feature = "std")]
    fn raw_display_handle(&self) -> Option<raw_window_handle::RawDisplayHandle> {
        None
    }
}
