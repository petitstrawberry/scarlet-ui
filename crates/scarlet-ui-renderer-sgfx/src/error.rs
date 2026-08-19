//! Renderer error values.

use core::fmt;

/// SGFX renderer operation that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Defining logical IR resources.
    DefineResources,
    /// Encoding the logical IR command stream.
    EncodeCommands,
    /// Rasterizing or encoding text glyphs.
    RasterizeText,
}

/// Error returned by backend-independent SGFX lowering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A fallible SGFX operation failed.
    Sgfx(Stage),
    /// A frame dimension or paint value was invalid.
    InvalidFrame,
    /// A canvas requested depth testing on a device without depth support.
    DepthUnsupported,
    /// The frame exceeded SGFX's bounded IR command or resource limits.
    FrameTooComplex,
}

impl Error {
    pub(crate) const fn sgfx(stage: Stage) -> Self {
        Self::Sgfx(stage)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sgfx(stage) => write!(formatter, "SGFX operation failed at {stage:?}"),
            Self::InvalidFrame => formatter.write_str("invalid ScarletUI frame"),
            Self::DepthUnsupported => {
                formatter.write_str("SGFX device does not support canvas depth testing")
            }
            Self::FrameTooComplex => formatter.write_str("ScarletUI frame exceeds SGFX IR limits"),
        }
    }
}

/// Error returned while encoding and synchronously executing a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError<E> {
    /// Portable ScarletUI-to-SGFX lowering failed.
    Lowering(Error),
    /// The backend-owned executor supplied by the composition root rejected
    /// or failed the command buffer.
    Execution(E),
}

impl<E: fmt::Display> fmt::Display for FrameError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lowering(error) => write!(formatter, "SGFX frame lowering failed: {error}"),
            Self::Execution(error) => write!(formatter, "SGFX frame execution failed: {error}"),
        }
    }
}

impl<E> From<Error> for FrameError<E> {
    fn from(error: Error) -> Self {
        Self::Lowering(error)
    }
}

/// Result returned by SGFX renderer operations.
pub type Result<T> = core::result::Result<T, Error>;
