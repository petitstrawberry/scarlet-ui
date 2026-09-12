//! Vulkan-produced Scarlet GPU images embedded in ScarletUI paint order.

use alloc::sync::Arc;
use core::fmt;

use scarlet_ui_core::color::Color;
use scarlet_ui_core::renderer::PaintExtension;
use scarlet_ui_renderer_sgfx::{
    SgfxCanvas, SgfxCanvasDraw, SgfxCanvasFrame, SgfxCanvasHandle, SgfxCanvasVertex, SgfxMesh,
    SgfxTexture,
};
use sgfx::Handle;

pub(crate) struct SharedImageSource {
    handle: Handle,
}

impl SharedImageSource {
    pub(crate) fn duplicate_handle(&self) -> core::result::Result<Handle, ()> {
        self.handle.duplicate().map_err(|_| ())
    }
}

/// Failure while adopting a Vulkan-exported Scarlet image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VulkanCanvasImageError {
    /// The image width or height is zero.
    InvalidExtent,
    /// The raw value is not an owned Scarlet GPU image capability.
    InvalidHandle,
}

impl fmt::Display for VulkanCanvasImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExtent => formatter.write_str("Vulkan canvas image extent is empty"),
            Self::InvalidHandle => formatter.write_str("Vulkan canvas image handle is invalid"),
        }
    }
}

impl fmt::Debug for SharedImageSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedImageSource")
            .finish_non_exhaustive()
    }
}

/// One BGRA8 Vulkan image exported through `VK_SGFX_scarlet_image`.
///
/// The object owns the imported Scarlet capability. Rebuild a canvas with a
/// higher frame revision after Vulkan has completed writing new pixels. The
/// Vulkan writer and ScarletUI renderer must not access the image concurrently.
#[derive(Debug)]
pub struct VulkanCanvasImage {
    texture: Arc<SgfxTexture>,
    mesh: Arc<SgfxMesh>,
    width: u32,
    height: u32,
}

impl VulkanCanvasImage {
    /// Adopt an owning Scarlet GPU image handle returned by vulkan-sgfx.
    pub fn new(
        handle: Handle,
        width: u32,
        height: u32,
    ) -> core::result::Result<Arc<Self>, VulkanCanvasImageError> {
        if width == 0 || height == 0 {
            return Err(VulkanCanvasImageError::InvalidExtent);
        }
        let source = Arc::new(SharedImageSource { handle });
        let payload: Arc<dyn PaintExtension> = source;
        let texture = SgfxTexture::external_bgra8(width, height, payload);
        let mesh = SgfxMesh::new(alloc::vec![
            vertex(-1.0, -1.0, 0.0, 1.0),
            vertex(1.0, -1.0, 1.0, 1.0),
            vertex(1.0, 1.0, 1.0, 0.0),
            vertex(-1.0, -1.0, 0.0, 1.0),
            vertex(1.0, 1.0, 1.0, 0.0),
            vertex(-1.0, 1.0, 0.0, 0.0),
        ]);
        Ok(Arc::new(Self {
            texture,
            mesh,
            width,
            height,
        }))
    }

    /// Adopt the owning raw handle returned by `vkGetImageScarletHandleSGFX`.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid, exclusively owned Scarlet handle. This function
    /// consumes that ownership on both success and error; the caller must not
    /// close or adopt the raw value again.
    pub unsafe fn from_raw_handle(
        raw: i32,
        width: u32,
        height: u32,
    ) -> core::result::Result<Arc<Self>, VulkanCanvasImageError> {
        let handle =
            unsafe { Handle::from_raw(raw) }.map_err(|_| VulkanCanvasImageError::InvalidHandle)?;
        Self::new(handle, width, height)
    }

    /// Physical width of the shared image.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Physical height of the shared image.
    pub const fn height(&self) -> u32 {
        self.height
    }
}

/// Factory for a ScarletUI canvas that displays a Vulkan image without copying.
pub struct VulkanCanvas;

impl VulkanCanvas {
    /// Build a canvas view for one completed Vulkan image revision.
    ///
    /// Reuse `handle` for the lifetime of the logical view. Increment `revision`
    /// whenever Vulkan publishes new pixels, after waiting for its submission.
    pub fn new(
        handle: SgfxCanvasHandle,
        width: f32,
        height: f32,
        image: Arc<VulkanCanvasImage>,
        revision: u64,
    ) -> SgfxCanvas {
        let frame = SgfxCanvasFrame::new(revision, Color::TRANSPARENT).draw(
            SgfxCanvasDraw::new(
                Arc::clone(&image.mesh),
                [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ],
            )
            .texture(Arc::clone(&image.texture)),
        );
        SgfxCanvas::new(handle, width, height, Arc::new(frame))
    }
}

fn vertex(x: f32, y: f32, u: f32, v: f32) -> SgfxCanvasVertex {
    SgfxCanvasVertex::new([x, y, 0.0, 1.0], [1.0; 4]).with_tex_coord([u, v])
}
