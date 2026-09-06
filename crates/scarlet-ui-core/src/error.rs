//! Error types for ScarletUI

use alloc::string::String;
use core::fmt;

/// Why a frame was not presented and whether the renderer can continue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderFailureKind {
    /// Temporary admission pressure. Accepted work retired; retry may succeed.
    Busy,
    /// Input, support, or resource limits rejected a frame. Accepted work retired;
    /// change the input or available resources before retrying, not the renderer.
    Rejected,
    /// GPU execution/observation failed or retirement is uncertain. Do not reuse
    /// affected images. Recreate the backend/window before attempting more work.
    RecoveryRequired,
}

/// A failed frame, distinct from successful presentation or an idle frame.
///
/// Recoverable failures guarantee that the previous displayed frame is preserved
/// and every accepted GPU access from the discarded frame retired successfully.
/// They do not imply rollback of offscreen caches or permission to replay a raw
/// command stream. The next attempt must encode a new, complete frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderFailure {
    /// Stable recovery classification.
    pub kind: RenderFailureKind,
    /// Diagnostic cause supplied by the renderer; not a machine-parsed contract.
    pub reason: String,
}

impl fmt::Display for RenderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "frame not presented ({:?}): {}",
            self.kind, self.reason
        )
    }
}

/// ScarletUI error types
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// Invalid window size
    InvalidSize { width: u32, height: u32 },

    /// Window creation failed
    WindowCreationFailed,

    /// The selected backend cannot provide the requested frame/titlebar combination.
    WindowDecorationUnsupported,

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

    /// A classified frame failure. Busy/rejected frames permit continued input
    /// and scene updates; recovery-required frames must not reuse the backend.
    RenderFailure(RenderFailure),

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
            Error::WindowDecorationUnsupported => {
                write!(
                    f,
                    "The requested window decoration combination is not supported"
                )
            }
            Error::SurfaceCreationFailed => write!(f, "Failed to create surface"),
            Error::ConnectionFailed => write!(f, "Failed to connect to window server"),
            Error::IoError => write!(f, "IO error"),
            Error::InvalidStateId => write!(f, "Invalid state ID"),
            Error::LayoutConstraintViolation => write!(f, "Layout constraint violation"),
            Error::RenderError => write!(f, "Rendering error"),
            Error::RenderFailure(failure) => failure.fmt(f),
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
