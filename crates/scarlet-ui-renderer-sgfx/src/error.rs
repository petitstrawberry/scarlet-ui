//! Renderer error values.

use core::fmt;

/// SGFX renderer operation that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Opening the graphics device.
    OpenDevice,
    /// Creating a graphics context.
    CreateContext,
    /// Creating the graphics queue.
    CreateQueue,
    /// Creating a shared render-target image.
    CreateSharedImage,
    /// Releasing a retired shared image.
    ReleaseSharedImage,
    /// Defining logical IR resources.
    DefineResources,
    /// Creating the IR resource cache.
    CreateIrResources,
    /// Mapping a logical target to a shared image.
    MapSharedImage,
    /// Encoding the logical IR command stream.
    EncodeCommands,
    /// Submitting logical IR to SGFX.
    SubmitCommands,
    /// Acquiring a WGPU surface frame.
    AcquireSurfaceFrame,
    /// Presenting a WGPU surface frame.
    PresentSurfaceFrame,
    /// Registering a shared image with the frame sink.
    RegisterImage,
    /// Waiting for a retained image to be released.
    WaitForRelease,
    /// Atomically attaching and committing a damaged image.
    CommitImage,
    /// Destroying a retired image registration.
    DestroyImage,
    /// Rasterizing or encoding text glyphs.
    RasterizeText,
}

/// Error returned by SGFX lowering and presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A fallible SGFX operation failed.
    Sgfx(Stage),
    /// A frame-sink operation failed.
    Sink {
        /// Operation that was in progress.
        stage: Stage,
        /// Stable sink-level failure category.
        source: crate::sink::SgfxSinkError,
    },
    /// A frame dimension or paint value was invalid.
    InvalidFrame,
    /// A canvas requested depth testing on a device without depth support.
    DepthUnsupported,
    /// The frame exceeded SGFX's bounded IR command or resource limits.
    FrameTooComplex,
    /// The image-pool generation counter was exhausted.
    GenerationExhausted,
}

impl Error {
    pub(crate) const fn sgfx(stage: Stage) -> Self {
        Self::Sgfx(stage)
    }

    pub(crate) const fn sink(stage: Stage, source: crate::sink::SgfxSinkError) -> Self {
        Self::Sink { stage, source }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sgfx(stage) => write!(formatter, "SGFX operation failed at {stage:?}"),
            Self::Sink { stage, source } => {
                write!(formatter, "SGFX sink failed at {stage:?}: {source}")
            }
            Self::InvalidFrame => formatter.write_str("invalid ScarletUI frame"),
            Self::DepthUnsupported => {
                formatter.write_str("SGFX device does not support canvas depth testing")
            }
            Self::FrameTooComplex => formatter.write_str("ScarletUI frame exceeds SGFX IR limits"),
            Self::GenerationExhausted => formatter.write_str("SGFX buffer generation exhausted"),
        }
    }
}

/// Result returned by SGFX renderer operations.
pub type Result<T> = core::result::Result<T, Error>;
