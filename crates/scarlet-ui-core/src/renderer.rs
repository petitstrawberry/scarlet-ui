use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::buffer::Buffer;
use crate::color::Color;
use crate::compositor::{Compositor, DamageRect};
use crate::element::{Element, ElementId};
use crate::geometry::{Rect, Size};

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct FrameSize {
    pub width: f32,
    pub height: f32,
    pub scale_milli: u32,
}

#[derive(Debug, Clone)]
pub enum PaintCommand {
    FillRect {
        rect: Rect,
        color: Color,
    },
    FillRoundedRect {
        rect: Rect,
        corner_radius: f32,
        color: Color,
    },
    DrawBuffer {
        dst: Rect,
        buffer_idx: usize,
    },
    PushClip {
        rect: Rect,
    },
    PopClip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferHandle(pub usize);

pub struct PaintContext {
    commands: Vec<PaintCommand>,
    buffers: Vec<Buffer>,
}

impl Default for PaintContext {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            buffers: Vec::new(),
        }
    }
}

impl PaintContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.commands.push(PaintCommand::FillRect { rect, color });
    }

    pub fn fill_rounded_rect(&mut self, rect: Rect, corner_radius: f32, color: Color) {
        self.commands.push(PaintCommand::FillRoundedRect {
            rect,
            corner_radius,
            color,
        });
    }

    pub fn draw_buffer(&mut self, dst: Rect, buffer: Buffer) -> BufferHandle {
        let idx = self.buffers.len();
        self.buffers.push(buffer);
        self.commands.push(PaintCommand::DrawBuffer {
            dst,
            buffer_idx: idx,
        });
        BufferHandle(idx)
    }

    pub fn push_clip(&mut self, rect: Rect) {
        self.commands.push(PaintCommand::PushClip { rect });
    }

    pub fn pop_clip(&mut self) {
        self.commands.push(PaintCommand::PopClip);
    }

    pub fn commands(&self) -> &[PaintCommand] {
        &self.commands
    }

    pub fn buffers(&self) -> &[Buffer] {
        &self.buffers
    }

    pub fn clear(&mut self) {
        self.commands.clear();
        self.buffers.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

pub struct Frame<'a> {
    pub buffer: &'a Buffer,
    pub damage: Option<&'a [DamageRect]>,
}

pub trait Renderer {
    fn resize(&mut self, size: FrameSize);

    fn set_background_color(&mut self, color: Color);

    fn composite(&mut self, root: &dyn Element, dirty_ids: &[ElementId]);

    fn render_paint(&mut self, ctx: &PaintContext);

    fn buffer(&self) -> &Buffer;

    fn buffer_mut(&mut self) -> &mut Buffer;

    fn damage(&self) -> Option<&[DamageRect]>;
}

pub struct CpuRenderer {
    compositor: Compositor,
}

impl CpuRenderer {
    pub fn new(size: Size, scale_milli: u32, background_color: Color) -> Self {
        Self {
            compositor: Compositor::new(size, scale_milli, background_color),
        }
    }

    pub fn compositor(&self) -> &Compositor {
        &self.compositor
    }

    pub fn compositor_mut(&mut self) -> &mut Compositor {
        &mut self.compositor
    }
}

impl Renderer for CpuRenderer {
    fn resize(&mut self, size: FrameSize) {
        let logical = Size::new(size.width, size.height);
        self.compositor.set_scale_milli(size.scale_milli, logical);
        self.compositor.resize(logical);
    }

    fn set_background_color(&mut self, color: Color) {
        self.compositor.set_background_color(color);
    }

    fn composite(&mut self, root: &dyn Element, dirty_ids: &[ElementId]) {
        self.compositor
            .composite_elements_with_dirty(root, dirty_ids);
    }

    fn render_paint(&mut self, _ctx: &PaintContext) {}

    fn buffer(&self) -> &Buffer {
        self.compositor.window_buffer()
    }

    fn buffer_mut(&mut self) -> &mut Buffer {
        self.compositor.window_buffer_mut()
    }

    fn damage(&self) -> Option<&[DamageRect]> {
        self.compositor.last_damage_rects()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_context_fill_rect() {
        let mut ctx = PaintContext::new();
        ctx.fill_rect(Rect::zero(), Color::rgb(255, 0, 0));
        assert_eq!(ctx.commands().len(), 1);
        assert!(matches!(ctx.commands()[0], PaintCommand::FillRect { .. }));
    }

    #[test]
    fn paint_context_clip_stack() {
        let mut ctx = PaintContext::new();
        ctx.push_clip(Rect::zero());
        ctx.fill_rect(Rect::zero(), Color::rgb(0, 255, 0));
        ctx.pop_clip();
        assert_eq!(ctx.commands().len(), 3);
    }

    use crate::geometry::Size;

    #[test]
    fn paint_context_draw_buffer_assigns_handle() {
        let mut ctx = PaintContext::new();
        let buf = Buffer::new(Size::new(1.0, 1.0));
        let handle = ctx.draw_buffer(Rect::zero(), buf);
        assert_eq!(handle, BufferHandle(0));
        assert_eq!(ctx.buffers().len(), 1);
        assert!(matches!(ctx.commands()[0], PaintCommand::DrawBuffer { .. }));
    }

    #[test]
    fn paint_context_clear() {
        let mut ctx = PaintContext::new();
        ctx.fill_rect(Rect::zero(), Color::rgb(0, 0, 0));
        ctx.clear();
        assert!(ctx.is_empty());
    }
}
