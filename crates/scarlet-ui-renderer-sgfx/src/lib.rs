//! Native SGFX renderer for ScarletUI paint commands.
//!
//! The renderer tessellates ScarletUI's backend-neutral paint list directly
//! into SGFX IR. Presentation uses a two-image shared-image pool, so no
//! completed CPU framebuffer is uploaded or copied into SWS.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(all(feature = "std", feature = "legacy-scarlet-std"))]
compile_error!("`std` and `legacy-scarlet-std` are mutually exclusive");

#[cfg(not(any(feature = "std", feature = "legacy-scarlet-std")))]
compile_error!("either `std` or `legacy-scarlet-std` must be enabled");

extern crate alloc;

#[cfg(target_os = "scarlet")]
mod backend;
mod canvas;
mod error;
mod geometry;
mod lowering;
#[cfg(target_os = "scarlet")]
mod sink;
#[cfg(feature = "wgpu")]
mod wgpu_backend;

#[cfg(target_os = "scarlet")]
pub use backend::{DEFAULT_GPU_DEVICE, SgfxPaintBackend};
pub use canvas::{
    SgfxCanvas, SgfxCanvasDraw, SgfxCanvasFrame, SgfxCanvasHandle, SgfxCanvasRenderObject,
    SgfxCanvasVertex, SgfxMesh, SgfxMeshHandle, SgfxTexture,
};
pub use error::{Error, Result, Stage};
#[cfg(target_os = "scarlet")]
pub use scarlet_sgfx_virgl::Image as SgfxImage;
#[cfg(target_os = "scarlet")]
pub use sink::{
    SgfxBufferIdentity, SgfxCommitToken, SgfxFrameSink, SgfxSinkError, SgfxSinkResult,
    SgfxSinkStatus,
};
#[cfg(feature = "wgpu")]
pub use wgpu_backend::WgpuSgfxSession;
