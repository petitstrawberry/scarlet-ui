// SPDX-License-Identifier: MIT
//! A small CPU framebuffer uploaded once and sampled into a larger GPU rect.
use alloc::sync::Arc;
use core::fmt;
use scarlet_ui_core::{buffer::Buffer, geometry::Rect, renderer::PaintContext};

pub(crate) struct ScaledBufferPaint {
    pub buffer: Arc<Buffer>,
}
impl fmt::Debug for ScaledBufferPaint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScaledBufferPaint")
            .field("width", &self.buffer.width())
            .field("height", &self.buffer.height())
            .finish()
    }
}
/// Retain a small BGRA buffer and draw it with nearest-neighbor GPU sampling.
/// Reuse the Buffer identity between frames to reuse the existing texture cache.
pub fn paint_scaled_buffer(ctx: &mut PaintContext<'_>, rect: Rect, buffer: Arc<Buffer>) {
    ctx.draw_extension(rect, Arc::new(ScaledBufferPaint { buffer }));
}
