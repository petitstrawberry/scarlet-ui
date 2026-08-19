//! Backend-independent SGFX encoder for ScarletUI paint commands.
//!
//! The renderer tessellates ScarletUI's backend-neutral paint list directly
//! into SGFX IR. SGFX backend sessions retain ownership of physical images,
//! resource caches, queues, and presentation.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(all(feature = "std", feature = "legacy-scarlet-std"))]
compile_error!("`std` and `legacy-scarlet-std` are mutually exclusive");

#[cfg(not(any(feature = "std", feature = "legacy-scarlet-std")))]
compile_error!("either `std` or `legacy-scarlet-std` must be enabled");

extern crate alloc;

mod canvas;
mod error;
mod geometry;
mod lowering;

pub use canvas::{
    SgfxCanvas, SgfxCanvasDraw, SgfxCanvasFrame, SgfxCanvasHandle, SgfxCanvasRenderObject,
    SgfxCanvasVertex, SgfxMesh, SgfxMeshHandle, SgfxTexture,
};
pub use error::{Error, FrameError, Result, Stage};
pub use lowering::SgfxPaintEncoder;
