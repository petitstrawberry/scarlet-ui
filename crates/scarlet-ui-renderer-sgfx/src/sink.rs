//! Shared-image presentation contract implemented by an SWS platform backend.

use core::fmt;

use scarlet_ui_core::compositor::DamageRect;
use sgfx::Image;

/// Complete identity of one SWS shared SGFX buffer registration.
///
/// Every lifecycle operation uses all four fields. A buffer number alone is
/// never sufficient to identify a reusable shared image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SgfxBufferIdentity {
    /// SWS window that owns the registration.
    pub window_id: u32,
    /// Stable pool slot number.
    pub buffer_id: u32,
    /// Image allocation generation for this slot.
    pub generation: u32,
    /// SWS compositor epoch.
    pub compositor_epoch: u32,
}

/// One exact submitted use of a registered shared SGFX buffer.
///
/// The serial prevents a delayed release or rejection for an earlier use of a
/// pool slot from being mistaken for the slot's current use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SgfxCommitToken {
    /// Registered shared-buffer identity.
    pub identity: SgfxBufferIdentity,
    /// Non-zero serial scoped to the target window.
    pub commit_serial: u64,
}

/// Coarse frame-sink availability state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SgfxSinkStatus {
    /// Shared SGFX image presentation is ready at this compositor epoch.
    Ready { compositor_epoch: u32 },
    /// Identities older than the named compositor epoch were invalidated.
    BackendLost { compositor_epoch: u32 },
}

/// Stable failure categories returned by [`SgfxFrameSink`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SgfxSinkError {
    /// SWS did not negotiate shared-image support.
    Unavailable,
    /// Identities older than the named compositor epoch were invalidated.
    BackendLost { compositor_epoch: u32 },
    /// SWS still retains the selected buffer.
    BufferBusy,
    /// A registration identity was stale or otherwise invalid.
    InvalidIdentity,
    /// The shared image could not be imported by SWS.
    ImportFailed,
    /// The platform connection or protocol exchange failed.
    Protocol,
}

impl fmt::Display for SgfxSinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("shared SGFX images are unavailable"),
            Self::BackendLost { compositor_epoch } => {
                write!(formatter, "SWS compositor epoch {compositor_epoch} was lost")
            }
            Self::BufferBusy => formatter.write_str("shared SGFX buffer is busy"),
            Self::InvalidIdentity => formatter.write_str("invalid shared SGFX buffer identity"),
            Self::ImportFailed => formatter.write_str("SWS failed to import shared SGFX image"),
            Self::Protocol => formatter.write_str("SWS shared-image protocol failed"),
        }
    }
}

/// Result returned by a shared-image frame sink.
pub type SgfxSinkResult<T> = core::result::Result<T, SgfxSinkError>;

/// Presentation half of the SGFX renderer.
///
/// `scarlet-ui-platform-sws` implements this trait with the same accepted SWS
/// connection that created `window_id`; opening a second connection would fail
/// SWS's window-ownership check. Implementations must route asynchronous buffer
/// release and backend-loss messages without consuming normal UI input events.
pub trait SgfxFrameSink {
    /// Return the window owned by this sink.
    ///
    /// # Returns
    ///
    /// The stable SWS window identifier.
    fn window_id(&self) -> u32;

    /// Query shared-image availability and the current compositor epoch.
    ///
    /// # Returns
    ///
    /// Current sink status, or a connection error.
    fn status(&mut self) -> SgfxSinkResult<SgfxSinkStatus>;

    /// Transfer and register one shared image capability.
    ///
    /// `image.width()` and `image.height()` are the registration dimensions and
    /// `image.shared_handle()` is the borrowed capability to transfer.
    ///
    /// # Arguments
    ///
    /// * `identity` - Complete registration identity.
    /// * `image` - Shared SGFX image retained by the renderer.
    ///
    /// # Returns
    ///
    /// Success after SWS confirms import, or a sink error.
    fn register_shared_image(
        &mut self,
        identity: SgfxBufferIdentity,
        image: &Image,
    ) -> SgfxSinkResult<()>;

    /// Wait until SWS no longer retains a previously committed image.
    ///
    /// New or registered-but-never-committed identities must return
    /// immediately. Implementations abort the wait on backend loss.
    ///
    /// # Arguments
    ///
    /// * `token` - Exact submitted buffer use whose release is required.
    ///
    /// # Returns
    ///
    /// Success when the image may be rendered again, or a sink error.
    fn wait_until_released(&mut self, token: SgfxCommitToken) -> SgfxSinkResult<()>;

    /// Atomically attach an image and commit all damage in one request.
    ///
    /// The method returns after the one-way commit frame has been serialized;
    /// SWS reports rejection or release asynchronously. `damage` must not be
    /// empty; callers skip an idle frame instead of issuing an empty commit.
    ///
    /// # Arguments
    ///
    /// * `identity` - Registered image selected for this frame.
    /// * `damage` - Non-empty physical damage rectangle slice.
    ///
    /// # Returns
    ///
    /// The exact submitted buffer-use token, or a sink error.
    fn commit_shared_image(
        &mut self,
        identity: SgfxBufferIdentity,
        damage: &[DamageRect],
    ) -> SgfxSinkResult<SgfxCommitToken>;

    /// Remove a released image registration.
    ///
    /// # Arguments
    ///
    /// * `identity` - Exact released identity to destroy.
    ///
    /// # Returns
    ///
    /// Success after the request is written, or a sink error.
    fn destroy_shared_image(&mut self, identity: SgfxBufferIdentity) -> SgfxSinkResult<()>;
}
