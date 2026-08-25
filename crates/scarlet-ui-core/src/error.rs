//! Error types for ScarletUI

use alloc::string::String;
use core::fmt;

/// ScarletUI error types
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// Invalid window size
    InvalidSize { width: u32, height: u32 },

    /// Window creation failed
    WindowCreationFailed,

    /// The selected backend cannot provide a system-managed window frame.
    SystemWindowDecorationUnsupported,

    /// Surface creation failed
    SurfaceCreationFailed,

    /// Connection to window server failed
    ConnectionFailed,

    /// IO error
    IoError,

    /// Invalid state ID
    InvalidStateId,

    /// Layout constraint violation
    LayoutConstraintViolation,

    /// Rendering error
    RenderError,

    /// Unknown renderer backend requested through configuration.
    InvalidRendererBackend { value: String },

    /// Event dispatch error
    EventDispatchError,

    /// Pointer lock is not supported by the selected platform backend.
    PointerLockUnsupported,

    /// Duplicate scene window key
    DuplicateSceneWindowKey,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidSize { width, height } => {
                write!(f, "Invalid window size: {}x{}", width, height)
            }
            Error::WindowCreationFailed => write!(f, "Failed to create window"),
            Error::SystemWindowDecorationUnsupported => {
                write!(
                    f,
                    "System window decorations are not supported by this backend"
                )
            }
            Error::SurfaceCreationFailed => write!(f, "Failed to create surface"),
            Error::ConnectionFailed => write!(f, "Failed to connect to window server"),
            Error::IoError => write!(f, "IO error"),
            Error::InvalidStateId => write!(f, "Invalid state ID"),
            Error::LayoutConstraintViolation => write!(f, "Layout constraint violation"),
            Error::RenderError => write!(f, "Rendering error"),
            Error::InvalidRendererBackend { value } => {
                write!(f, "Invalid renderer backend: {}", value)
            }
            Error::EventDispatchError => write!(f, "Event dispatch error"),
            Error::PointerLockUnsupported => write!(f, "Pointer lock is not supported"),
            Error::DuplicateSceneWindowKey => write!(f, "Duplicate scene window key"),
        }
    }
}

/// Result type for ScarletUI operations
pub type Result<T> = core::result::Result<T, Error>;
