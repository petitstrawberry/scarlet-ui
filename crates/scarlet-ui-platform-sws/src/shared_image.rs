//! Scarlet shared GPU images exposed as ordinary SGFX textures.

use alloc::sync::Arc;
use core::fmt;

use scarlet_ui_core::renderer::PaintExtension;
use scarlet_ui_renderer_sgfx::SgfxTexture;
use sgfx::Handle;

pub(crate) struct SharedImageSource {
    handle: Handle,
}

impl SharedImageSource {
    pub(crate) fn duplicate_handle(&self) -> core::result::Result<Handle, ()> {
        self.handle.duplicate().map_err(|_| ())
    }
}

impl fmt::Debug for SharedImageSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedImageSource")
            .finish_non_exhaustive()
    }
}

/// Failure while adopting a Scarlet shared GPU image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharedImageError {
    /// The image width or height is zero.
    InvalidExtent,
    /// The raw value is not a valid owned Scarlet handle.
    InvalidHandle,
}

impl fmt::Display for SharedImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExtent => formatter.write_str("shared image extent is empty"),
            Self::InvalidHandle => formatter.write_str("shared image handle is invalid"),
        }
    }
}

/// Adopt a BGRA8 Scarlet shared GPU image as an ordinary SGFX texture.
///
/// The returned texture owns `handle` and can be used by a normal `SgfxCanvas`
/// alongside CPU-uploaded and SGFX-produced textures. Reuse the texture while
/// publishing new image contents. The producer and ScarletUI renderer must not
/// access the image concurrently.
pub fn shared_bgra8_texture(
    handle: Handle,
    width: u32,
    height: u32,
) -> core::result::Result<Arc<SgfxTexture>, SharedImageError> {
    if width == 0 || height == 0 {
        return Err(SharedImageError::InvalidExtent);
    }

    let source = Arc::new(SharedImageSource { handle });
    let payload: Arc<dyn PaintExtension> = source;
    Ok(SgfxTexture::external_bgra8(width, height, payload))
}

/// Adopt an owning raw Scarlet handle as an ordinary BGRA8 SGFX texture.
///
/// This entry point is suitable for handles exported by Vulkan-SGFX or another
/// Scarlet GPU producer.
///
/// # Safety
///
/// `raw` must be a valid, exclusively owned Scarlet handle. This function
/// consumes that ownership on both success and error; the caller must not close
/// or adopt the raw value again.
pub unsafe fn shared_bgra8_texture_from_raw(
    raw: i32,
    width: u32,
    height: u32,
) -> core::result::Result<Arc<SgfxTexture>, SharedImageError> {
    let handle = unsafe { Handle::from_raw(raw) }.map_err(|_| SharedImageError::InvalidHandle)?;
    shared_bgra8_texture(handle, width, height)
}
