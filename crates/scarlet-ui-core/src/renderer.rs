use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::buffer::Buffer;
use crate::color::Color;
use crate::compositor::{Compositor, DamageRect};
use crate::element::{Element, ElementId};
use crate::geometry::{Point, Rect, Size};

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct FrameSize {
    pub width: f32,
    pub height: f32,
    pub scale_milli: u32,
}

pub type Path = Vec<Point>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipRegion {
    pub rect: Rect,
    pub corner_radius: f32,
}

pub fn path_rect(rect: Rect) -> Path {
    alloc::vec![
        Point::new(rect.origin.x, rect.origin.y),
        Point::new(rect.origin.x + rect.size.width, rect.origin.y),
        Point::new(
            rect.origin.x + rect.size.width,
            rect.origin.y + rect.size.height
        ),
        Point::new(rect.origin.x, rect.origin.y + rect.size.height),
    ]
}

pub fn path_triangle(a: Point, b: Point, c: Point) -> Path {
    alloc::vec![a, b, c]
}

pub fn path_circle(center: Point, radius: f32) -> Path {
    const SEGMENTS: usize = 48;
    (0..SEGMENTS)
        .map(|i| {
            let angle = 2.0 * core::f32::consts::PI * (i as f32) / (SEGMENTS as f32);
            Point::new(
                center.x + radius * libm::cosf(angle),
                center.y + radius * libm::sinf(angle),
            )
        })
        .collect()
}

pub fn path_rounded_rect(rect: Rect, corner_radius: f32) -> Path {
    let r = corner_radius
        .min(rect.size.width / 2.0)
        .min(rect.size.height / 2.0);
    let corner_segments = (libm::ceilf(r * 0.75) as usize).clamp(8, 32);
    let x0 = rect.origin.x;
    let y0 = rect.origin.y;
    let x1 = rect.origin.x + rect.size.width;
    let y1 = rect.origin.y + rect.size.height;
    let mut pts = Vec::new();
    let corners = [
        (x1 - r, y1 - r, 0.0),
        (x0 + r, y1 - r, core::f32::consts::FRAC_PI_2),
        (x0 + r, y0 + r, core::f32::consts::PI),
        (x1 - r, y0 + r, 3.0 * core::f32::consts::FRAC_PI_2),
    ];
    for (cx, cy, start_angle) in corners {
        for i in 0..corner_segments {
            let t =
                start_angle + core::f32::consts::FRAC_PI_2 * (i as f32) / (corner_segments as f32);
            pts.push(Point::new(cx + r * libm::cosf(t), cy + r * libm::sinf(t)));
        }
    }
    pts
}

#[derive(Debug, Clone)]
pub enum PaintCommand {
    FillPath {
        path: Path,
        color: Color,
    },
    StrokePath {
        path: Path,
        stroke_width: f32,
        color: Color,
    },
    StrokeRect {
        rect: Rect,
        stroke_width: f32,
        color: Color,
    },
    StrokeRoundedRect {
        rect: Rect,
        corner_radius: f32,
        stroke_width: f32,
        color: Color,
    },
    DrawText {
        position: Point,
        text: String,
        color: Color,
        font_size_px: f32,
    },
    DrawBuffer {
        dst: Rect,
        buffer_idx: usize,
    },
    DrawBufferRect {
        dst: Rect,
        src: Rect,
        buffer_idx: usize,
        opacity: f32,
    },
    PushClip {
        rect: Rect,
        corner_radius: f32,
    },
    PopClip,
    SetOpacity {
        opacity: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferHandle(pub usize);

/// A buffer referenced by the current paint list.
///
/// Borrowed buffers are owned by render objects and are valid only for the
/// synchronous paint pass that created this context. Temporary buffers are an
/// explicit compatibility path for legacy callers that still have to produce a
/// one-frame buffer value.
pub enum PaintBuffer<'a> {
    Borrowed(&'a Buffer),
    Shared(Arc<Buffer>),
    Temporary(Buffer),
}

impl<'a> PaintBuffer<'a> {
    pub fn as_buffer(&self) -> &Buffer {
        match self {
            Self::Borrowed(buffer) => buffer,
            Self::Shared(buffer) => buffer.as_ref(),
            Self::Temporary(buffer) => buffer,
        }
    }

    pub fn is_temporary(&self) -> bool {
        matches!(self, Self::Temporary(_))
    }
}

pub struct PaintContext<'a> {
    commands: Vec<PaintCommand>,
    buffers: Vec<PaintBuffer<'a>>,
}

impl Default for PaintContext<'_> {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            buffers: Vec::new(),
        }
    }
}

impl<'a> PaintContext<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fill_path(&mut self, path: impl Into<Path>, color: Color) {
        self.commands.push(PaintCommand::FillPath {
            path: path.into(),
            color,
        });
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.fill_path(path_rect(rect), color);
    }

    pub fn fill_circle(&mut self, center: Point, radius: f32, color: Color) {
        self.fill_path(path_circle(center, radius), color);
    }

    pub fn fill_triangle(&mut self, a: Point, b: Point, c: Point, color: Color) {
        self.fill_path(path_triangle(a, b, c), color);
    }

    pub fn fill_rounded_rect(&mut self, rect: Rect, corner_radius: f32, color: Color) {
        self.fill_path(path_rounded_rect(rect, corner_radius), color);
    }

    pub fn stroke_path(&mut self, path: impl Into<Path>, stroke_width: f32, color: Color) {
        self.commands.push(PaintCommand::StrokePath {
            path: path.into(),
            stroke_width,
            color,
        });
    }

    pub fn stroke_rect(&mut self, rect: Rect, stroke_width: f32, color: Color) {
        self.commands.push(PaintCommand::StrokeRect {
            rect,
            stroke_width,
            color,
        });
    }

    pub fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        corner_radius: f32,
        stroke_width: f32,
        color: Color,
    ) {
        self.commands.push(PaintCommand::StrokeRoundedRect {
            rect,
            corner_radius,
            stroke_width,
            color,
        });
    }

    pub fn draw_line(&mut self, from: Point, to: Point, stroke_width: f32, color: Color) {
        self.stroke_path(alloc::vec![from, to], stroke_width, color);
    }

    pub fn draw_text(
        &mut self,
        position: Point,
        text: impl Into<String>,
        color: Color,
        font_size_px: f32,
    ) {
        self.commands.push(PaintCommand::DrawText {
            position,
            text: text.into(),
            color,
            font_size_px,
        });
    }

    pub fn draw_buffer_ref(&mut self, dst: Rect, buffer: &'a Buffer) -> BufferHandle {
        let idx = self.buffers.len();
        self.buffers.push(PaintBuffer::Borrowed(buffer));
        self.commands.push(PaintCommand::DrawBuffer {
            dst,
            buffer_idx: idx,
        });
        BufferHandle(idx)
    }

    pub fn draw_temporary_buffer(&mut self, dst: Rect, buffer: Buffer) -> BufferHandle {
        let idx = self.buffers.len();
        self.buffers.push(PaintBuffer::Temporary(buffer));
        self.commands.push(PaintCommand::DrawBuffer {
            dst,
            buffer_idx: idx,
        });
        BufferHandle(idx)
    }

    #[deprecated(
        since = "0.1.0",
        note = "use draw_buffer_ref for retained buffers or draw_temporary_buffer for one-frame compatibility buffers"
    )]
    pub fn draw_buffer(&mut self, dst: Rect, buffer: Buffer) -> BufferHandle {
        self.draw_temporary_buffer(dst, buffer)
    }

    pub fn draw_buffer_rect_ref(
        &mut self,
        dst: Rect,
        src: Rect,
        buffer: &'a Buffer,
        opacity: f32,
    ) -> BufferHandle {
        let idx = self.buffers.len();
        self.buffers.push(PaintBuffer::Borrowed(buffer));
        self.commands.push(PaintCommand::DrawBufferRect {
            dst,
            src,
            buffer_idx: idx,
            opacity,
        });
        BufferHandle(idx)
    }

    pub fn draw_buffer_rect_shared(
        &mut self,
        dst: Rect,
        src: Rect,
        buffer: Arc<Buffer>,
        opacity: f32,
    ) -> BufferHandle {
        let idx = self.buffers.len();
        self.buffers.push(PaintBuffer::Shared(buffer));
        self.commands.push(PaintCommand::DrawBufferRect {
            dst,
            src,
            buffer_idx: idx,
            opacity,
        });
        BufferHandle(idx)
    }

    pub fn draw_temporary_buffer_rect(
        &mut self,
        dst: Rect,
        src: Rect,
        buffer: Buffer,
        opacity: f32,
    ) -> BufferHandle {
        let idx = self.buffers.len();
        self.buffers.push(PaintBuffer::Temporary(buffer));
        self.commands.push(PaintCommand::DrawBufferRect {
            dst,
            src,
            buffer_idx: idx,
            opacity,
        });
        BufferHandle(idx)
    }

    #[deprecated(
        since = "0.1.0",
        note = "use draw_buffer_rect_ref for retained buffers or draw_temporary_buffer_rect for one-frame compatibility buffers"
    )]
    pub fn draw_buffer_rect(
        &mut self,
        dst: Rect,
        src: Rect,
        buffer: Buffer,
        opacity: f32,
    ) -> BufferHandle {
        self.draw_temporary_buffer_rect(dst, src, buffer, opacity)
    }

    pub fn push_clip(&mut self, rect: Rect) {
        self.push_rounded_clip(rect, 0.0);
    }

    pub fn push_rounded_clip(&mut self, rect: Rect, corner_radius: f32) {
        self.commands.push(PaintCommand::PushClip {
            rect,
            corner_radius,
        });
    }

    pub fn pop_clip(&mut self) {
        self.commands.push(PaintCommand::PopClip);
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.commands.push(PaintCommand::SetOpacity { opacity });
    }

    pub fn commands(&self) -> &[PaintCommand] {
        &self.commands
    }
    pub fn buffers(&self) -> &[PaintBuffer<'a>] {
        &self.buffers
    }
    pub fn buffer(&self, handle: BufferHandle) -> Option<&Buffer> {
        self.buffers.get(handle.0).map(PaintBuffer::as_buffer)
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
    fn render_paint(&mut self, ctx: &PaintContext<'_>);
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
    fn render_paint(&mut self, _ctx: &PaintContext<'_>) {}
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

pub struct CpuPaintRenderer {
    buffer: Buffer,
    background_color: Color,
    scale_milli: u32,
    layer_stack: Vec<PaintLayer>,
}

struct PaintLayer {
    buffer: Buffer,
    clip: ClipRegion,
}

impl CpuPaintRenderer {
    pub fn new(size: Size, scale_milli: u32, background_color: Color) -> Self {
        Self {
            buffer: Buffer::from_logical_dimensions_with_scale(
                size.width as u32,
                size.height as u32,
                scale_milli,
            ),
            background_color,
            scale_milli,
            layer_stack: Vec::new(),
        }
    }

    pub fn resize(&mut self, size: Size, scale_milli: u32) {
        self.buffer = Buffer::from_logical_dimensions_with_scale(
            size.width as u32,
            size.height as u32,
            scale_milli,
        );
        self.scale_milli = scale_milli;
    }

    fn scale_f32(&self, value: f32) -> f32 {
        value * (self.scale_milli as f32) / 1000.0
    }

    fn scale_point(&self, p: Point) -> Point {
        Point::new(self.scale_f32(p.x), self.scale_f32(p.y))
    }

    fn scale_rect(&self, rect: Rect) -> Rect {
        Rect::new(
            self.scale_point(rect.origin),
            Size::new(
                self.scale_f32(rect.size.width),
                self.scale_f32(rect.size.height),
            ),
        )
    }

    fn current_buffer_mut(&mut self) -> &mut Buffer {
        if let Some(layer) = self.layer_stack.last_mut() {
            &mut layer.buffer
        } else {
            &mut self.buffer
        }
    }

    fn scale_clip_region(&self, clip: ClipRegion) -> ClipRegion {
        ClipRegion {
            rect: self.scale_rect(clip.rect),
            corner_radius: self.scale_f32(clip.corner_radius),
        }
    }

    fn push_clip_layer(&mut self, clip: ClipRegion) {
        let layer = PaintLayer {
            buffer: Buffer::from_logical_dimensions_with_scale(
                self.buffer.logical_width(),
                self.buffer.logical_height(),
                self.scale_milli,
            ),
            clip,
        };
        self.layer_stack.push(layer);
    }

    fn pop_clip_layer(&mut self) {
        let Some(layer) = self.layer_stack.pop() else {
            return;
        };
        let clip = self.scale_clip_region(layer.clip);
        if clip.corner_radius > 0.0 {
            self.current_buffer_mut().composite_clipped_rounded(
                &layer.buffer,
                0,
                0,
                1.0,
                clip.rect.origin.x as i32,
                clip.rect.origin.y as i32,
                clip.rect.size.width as i32,
                clip.rect.size.height as i32,
                clip.corner_radius,
            );
        } else {
            self.current_buffer_mut().composite_clipped(
                &layer.buffer,
                0,
                0,
                1.0,
                clip.rect.origin.x as i32,
                clip.rect.origin.y as i32,
                clip.rect.size.width as i32,
                clip.rect.size.height as i32,
            );
        }
    }

    pub fn execute(&mut self, ctx: &PaintContext<'_>) {
        self.execute_with_damage(ctx, None);
    }

    pub fn execute_with_damage(&mut self, ctx: &PaintContext<'_>, damage_rects: Option<&[Rect]>) {
        match damage_rects {
            Some(rects) => {
                for rect in rects {
                    let (x, y, width, height) = self.rect_to_u32(*rect);
                    self.buffer
                        .clear_rect(x, y, width, height, self.background_color);
                }
            }
            None => self.buffer.clear(self.background_color),
        }
        self.layer_stack.clear();

        for cmd in ctx.commands() {
            match cmd {
                PaintCommand::FillPath { path, color } => {
                    let scaled: Vec<Point> =
                        path.iter().copied().map(|p| self.scale_point(p)).collect();
                    if let Some(rects) = damage_rects {
                        for rect in rects {
                            let clip = ClipRegion {
                                rect: self.scale_rect(*rect),
                                corner_radius: 0.0,
                            };
                            fill_polygon(self.current_buffer_mut(), &scaled, *color, Some(clip));
                        }
                    } else {
                        fill_polygon(self.current_buffer_mut(), &scaled, *color, None);
                    }
                }
                PaintCommand::StrokeRect {
                    rect,
                    stroke_width,
                    color,
                } => {
                    let rect = self.scale_rect(*rect);
                    let stroke_width = self.scale_f32(stroke_width.max(1.0));
                    if let Some(rects) = damage_rects {
                        for damage_rect in rects {
                            let clip = ClipRegion {
                                rect: self.scale_rect(*damage_rect),
                                corner_radius: 0.0,
                            };
                            stroke_rounded_rect(
                                self.current_buffer_mut(),
                                rect,
                                0.0,
                                stroke_width,
                                *color,
                                Some(clip),
                            );
                        }
                    } else {
                        stroke_rounded_rect(
                            self.current_buffer_mut(),
                            rect,
                            0.0,
                            stroke_width,
                            *color,
                            None,
                        );
                    }
                }
                PaintCommand::StrokeRoundedRect {
                    rect,
                    corner_radius,
                    stroke_width,
                    color,
                } => {
                    let rect = self.scale_rect(*rect);
                    let radius = self.scale_f32(*corner_radius);
                    let stroke_width = self.scale_f32(stroke_width.max(1.0));
                    if let Some(rects) = damage_rects {
                        for damage_rect in rects {
                            let clip = ClipRegion {
                                rect: self.scale_rect(*damage_rect),
                                corner_radius: 0.0,
                            };
                            stroke_rounded_rect(
                                self.current_buffer_mut(),
                                rect,
                                radius,
                                stroke_width,
                                *color,
                                Some(clip),
                            );
                        }
                    } else {
                        stroke_rounded_rect(
                            self.current_buffer_mut(),
                            rect,
                            radius,
                            stroke_width,
                            *color,
                            None,
                        );
                    }
                }
                PaintCommand::StrokePath {
                    path,
                    stroke_width,
                    color,
                } => {
                    let sw = stroke_width.max(1.0) as i32;
                    let mut canvas = crate::graphics::Canvas::for_buffer(self.current_buffer_mut());
                    for window in path.windows(2) {
                        canvas.draw_line(
                            window[0].x as i32,
                            window[0].y as i32,
                            window[1].x as i32,
                            window[1].y as i32,
                            *color,
                        );
                    }
                    if path.len() > 2 {
                        let first = path[0];
                        let last = path[path.len() - 1];
                        canvas.draw_line(
                            last.x as i32,
                            last.y as i32,
                            first.x as i32,
                            first.y as i32,
                            *color,
                        );
                    }
                    let _ = sw;
                }
                PaintCommand::DrawText {
                    position,
                    text,
                    color,
                    font_size_px,
                } => {
                    let mut canvas = crate::graphics::Canvas::for_buffer(self.current_buffer_mut());
                    if let Some(rects) = damage_rects {
                        for rect in rects {
                            canvas.draw_text_sized_clipped(
                                position.x as i32,
                                position.y as i32,
                                text,
                                *color,
                                *font_size_px,
                                *rect,
                                0.0,
                            );
                        }
                    } else {
                        canvas.draw_text_sized(
                            position.x as i32,
                            position.y as i32,
                            text,
                            *color,
                            *font_size_px,
                        );
                    }
                }
                PaintCommand::DrawBuffer { dst, buffer_idx } => {
                    if let Some(src) = ctx.buffer(BufferHandle(*buffer_idx)) {
                        let dst = self.scale_rect(*dst);
                        let dst_x = dst.origin.x as i32;
                        let dst_y = dst.origin.y as i32;
                        if let Some(rects) = damage_rects {
                            for rect in rects {
                                let clip = self.scale_rect(*rect);
                                self.current_buffer_mut().composite_clipped(
                                    src,
                                    dst_x,
                                    dst_y,
                                    1.0,
                                    clip.origin.x as i32,
                                    clip.origin.y as i32,
                                    clip.size.width as i32,
                                    clip.size.height as i32,
                                );
                            }
                        } else {
                            self.current_buffer_mut().composite(src, dst_x, dst_y, 1.0);
                        }
                    }
                }
                PaintCommand::DrawBufferRect {
                    dst,
                    src,
                    buffer_idx,
                    opacity,
                } => {
                    if let Some(buf) = ctx.buffer(BufferHandle(*buffer_idx)) {
                        let dst = self.scale_rect(*dst);
                        let src = self.scale_rect(*src);
                        let dst_x = dst.origin.x as i32;
                        let dst_y = dst.origin.y as i32;
                        let src_x = src.origin.x as i32;
                        let src_y = src.origin.y as i32;
                        let src_w = src.size.width as i32;
                        let src_h = src.size.height as i32;
                        if let Some(rects) = damage_rects {
                            for rect in rects {
                                let clip = self.scale_rect(*rect);
                                self.current_buffer_mut().composite_rect_clipped(
                                    buf,
                                    src_x,
                                    src_y,
                                    src_w,
                                    src_h,
                                    dst_x,
                                    dst_y,
                                    *opacity,
                                    clip.origin.x as i32,
                                    clip.origin.y as i32,
                                    clip.size.width as i32,
                                    clip.size.height as i32,
                                );
                            }
                        } else {
                            self.current_buffer_mut().composite_rect(
                                buf, src_x, src_y, src_w, src_h, dst_x, dst_y, *opacity,
                            );
                        }
                    }
                }
                PaintCommand::PushClip {
                    rect,
                    corner_radius,
                } => {
                    self.push_clip_layer(ClipRegion {
                        rect: *rect,
                        corner_radius: *corner_radius,
                    });
                }
                PaintCommand::PopClip => {
                    self.pop_clip_layer();
                }
                PaintCommand::SetOpacity { opacity: _ } => {}
            }
        }

        while !self.layer_stack.is_empty() {
            self.pop_clip_layer();
        }
    }

    fn rect_to_u32(&self, rect: Rect) -> (u32, u32, u32, u32) {
        let x0 = libm::floorf(rect.origin.x * self.scale_milli as f32 / 1000.0).max(0.0);
        let y0 = libm::floorf(rect.origin.y * self.scale_milli as f32 / 1000.0).max(0.0);
        let x1 = libm::ceilf((rect.origin.x + rect.size.width) * self.scale_milli as f32 / 1000.0)
            .min(self.buffer.width() as f32);
        let y1 = libm::ceilf((rect.origin.y + rect.size.height) * self.scale_milli as f32 / 1000.0)
            .min(self.buffer.height() as f32);
        let w = (x1 - x0).max(0.0);
        let h = (y1 - y0).max(0.0);
        (x0 as u32, y0 as u32, w as u32, h as u32)
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub fn into_buffer(self) -> Buffer {
        self.buffer
    }

    pub fn set_background_color(&mut self, color: Color) {
        self.background_color = color;
    }
}

fn fill_polygon(buffer: &mut Buffer, path: &[Point], color: Color, clip: Option<ClipRegion>) {
    if path.len() < 3 || color.a <= 0.0 {
        return;
    }
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for p in path {
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }
    if let Some(clip) = clip {
        min_y = min_y.max(clip.rect.origin.y);
        max_y = max_y.min(clip.rect.origin.y + clip.rect.size.height);
    }
    let min_y = libm::floorf(min_y) as i32;
    let max_y = libm::ceilf(max_y) as i32;
    let bw = buffer.width() as i32;
    let bh = buffer.height() as i32;
    let bgra = color.to_bgra();
    let is_opaque = color.a >= 1.0;
    let data = buffer.as_mut_slice();
    let n = path.len();

    for y in min_y..=max_y {
        if y < 0 || y >= bh {
            continue;
        }
        let yc = y as f32 + 0.5;
        let mut crossings: Vec<f32> = Vec::new();
        for i in 0..n {
            let p0 = path[i];
            let p1 = path[(i + 1) % n];
            let cond0 = p0.y <= yc;
            let cond1 = p1.y <= yc;
            if cond0 != cond1 {
                let dy = p1.y - p0.y;
                if dy.abs() > 0.001 {
                    let t = (yc - p0.y) / dy;
                    let x = p0.x + t * (p1.x - p0.x);
                    crossings.push(x);
                }
            }
        }
        crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
        let mut chunks = crossings.chunks_exact(2);
        for pair in chunks.by_ref() {
            let mut x_start = (libm::ceilf(pair[0] - 0.5) as i32).max(0);
            let mut x_end = (libm::ceilf(pair[1] - 0.5) as i32).min(bw);
            if let Some(clip) = clip {
                x_start = x_start.max(libm::floorf(clip.rect.origin.x) as i32);
                x_end = x_end.min(libm::ceilf(clip.rect.origin.x + clip.rect.size.width) as i32);
            }
            if x_start >= x_end || x_start >= bw {
                continue;
            }
            let start = (y as usize * bw as usize) + x_start as usize;
            let count = (x_end - x_start) as usize;
            let row = &mut data[start..start + count];
            if let Some(clip) = clip.filter(|clip| clip.corner_radius > 0.0) {
                for (offset, dst) in row.iter_mut().enumerate() {
                    let x = x_start + offset as i32;
                    if !contains_rounded_rect(
                        clip.rect,
                        clip.corner_radius,
                        x as f32 + 0.5,
                        y as f32 + 0.5,
                    ) {
                        continue;
                    }
                    if is_opaque {
                        *dst = bgra;
                    } else {
                        *dst = Buffer::blend_pixels(*dst, bgra, 1.0);
                    }
                }
            } else if is_opaque {
                row.fill(bgra);
            } else {
                for dst in row {
                    *dst = Buffer::blend_pixels(*dst, bgra, 1.0);
                }
            }
        }
    }
}

fn stroke_rounded_rect(
    buffer: &mut Buffer,
    rect: Rect,
    corner_radius: f32,
    stroke_width: f32,
    color: Color,
    clip: Option<ClipRegion>,
) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 || stroke_width <= 0.0 || color.a <= 0.0 {
        return;
    }

    let inner = inset_rect(rect, stroke_width);
    let inner_radius = (corner_radius - stroke_width).max(0.0);
    let (left, top, right, bottom) = raster_bounds(buffer, rect, clip.map(|c| c.rect));
    if right <= left || bottom <= top {
        return;
    }

    let pixel = color.to_bgra();
    let is_opaque = color.a >= 1.0;
    let width = buffer.width() as usize;
    let data = buffer.as_mut_slice();

    for y in top..bottom {
        let py = y as f32 + 0.5;
        for x in left..right {
            let px = x as f32 + 0.5;
            if let Some(clip) = clip {
                if !contains_rounded_rect(clip.rect, clip.corner_radius, px, py) {
                    continue;
                }
            }
            if contains_rounded_rect(rect, corner_radius, px, py)
                && !contains_rounded_rect(inner, inner_radius, px, py)
            {
                let idx = y as usize * width + x as usize;
                if is_opaque {
                    data[idx] = pixel;
                } else {
                    data[idx] = Buffer::blend_pixels(data[idx], pixel, 1.0);
                }
            }
        }
    }
}

fn raster_bounds(buffer: &Buffer, rect: Rect, clip: Option<Rect>) -> (i32, i32, i32, i32) {
    let mut left = libm::floorf(rect.origin.x) as i32;
    let mut top = libm::floorf(rect.origin.y) as i32;
    let mut right = libm::ceilf(rect.origin.x + rect.size.width) as i32;
    let mut bottom = libm::ceilf(rect.origin.y + rect.size.height) as i32;

    if let Some(clip) = clip {
        left = left.max(libm::floorf(clip.origin.x) as i32);
        top = top.max(libm::floorf(clip.origin.y) as i32);
        right = right.min(libm::ceilf(clip.origin.x + clip.size.width) as i32);
        bottom = bottom.min(libm::ceilf(clip.origin.y + clip.size.height) as i32);
    }

    (
        left.max(0),
        top.max(0),
        right.min(buffer.width() as i32),
        bottom.min(buffer.height() as i32),
    )
}

fn contains_rounded_rect(rect: Rect, corner_radius: f32, px: f32, py: f32) -> bool {
    let left = rect.origin.x;
    let top = rect.origin.y;
    let right = rect.origin.x + rect.size.width;
    let bottom = rect.origin.y + rect.size.height;
    if px < left || px >= right || py < top || py >= bottom {
        return false;
    }

    let radius = corner_radius
        .max(0.0)
        .min(rect.size.width * 0.5)
        .min(rect.size.height * 0.5);
    if radius <= 0.0 {
        return true;
    }

    let cx = px.clamp(left + radius, right - radius);
    let cy = py.clamp(top + radius, bottom - radius);
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy <= radius * radius
}

fn inset_rect(rect: Rect, inset: f32) -> Rect {
    Rect::new(
        Point::new(rect.origin.x + inset, rect.origin.y + inset),
        Size::new(
            (rect.size.width - inset * 2.0).max(0.0),
            (rect.size.height - inset * 2.0).max(0.0),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_helpers() {
        let r = path_rect(Rect::new(Point::new(0.0, 0.0), Size::new(10.0, 20.0)));
        assert_eq!(r.len(), 4);

        let c = path_circle(Point::new(50.0, 50.0), 10.0);
        assert_eq!(c.len(), 48);

        let t = path_triangle(Point::ZERO, Point::new(10.0, 0.0), Point::new(0.0, 10.0));
        assert_eq!(t.len(), 3);

        let rr = path_rounded_rect(Rect::new(Point::new(0.0, 0.0), Size::new(40.0, 40.0)), 8.0);
        assert_eq!(rr.len(), 32);
    }

    #[test]
    fn fill_rect_writes_pixels() {
        let mut ctx = PaintContext::new();
        ctx.fill_rect(
            Rect::new(Point::new(0.0, 0.0), Size::new(10.0, 10.0)),
            Color::rgb(255, 0, 0),
        );
        let mut r = CpuPaintRenderer::new(Size::new(100.0, 100.0), 1000, Color::rgb(0, 0, 0));
        r.execute(&ctx);
        assert!(r.buffer().get_pixel(5, 5).unwrap() > 0);
    }

    #[test]
    fn transparent_fill_preserves_existing_pixels() {
        let bg = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.fill_rect(
            Rect::new(Point::new(0.0, 0.0), Size::new(10.0, 10.0)),
            Color::TRANSPARENT,
        );
        let mut r = CpuPaintRenderer::new(Size::new(20.0, 20.0), 1000, bg);
        r.execute(&ctx);
        assert_eq!(r.buffer().get_pixel(5, 5).unwrap(), bg.to_bgra());
    }

    #[test]
    fn transparent_stroke_preserves_existing_pixels() {
        let bg = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.stroke_rounded_rect(
            Rect::from_xywh(0.0, 0.0, 10.0, 10.0),
            2.0,
            2.0,
            Color::TRANSPARENT,
        );
        let mut r = CpuPaintRenderer::new(Size::new(20.0, 20.0), 1000, bg);
        r.execute(&ctx);
        assert_eq!(r.buffer().get_pixel(1, 1).unwrap(), bg.to_bgra());
    }

    #[test]
    fn translucent_fill_blends_with_existing_pixels() {
        let bg = Color::rgb(255, 255, 255);
        let mut ctx = PaintContext::new();
        ctx.fill_rect(
            Rect::new(Point::new(0.0, 0.0), Size::new(10.0, 10.0)),
            Color::rgba(0, 0, 0, 128),
        );
        let mut r = CpuPaintRenderer::new(Size::new(20.0, 20.0), 1000, bg);
        r.execute(&ctx);

        let bytes = r.buffer().get_pixel(5, 5).unwrap().to_le_bytes();
        assert!((126..=127).contains(&bytes[0]));
        assert!((126..=127).contains(&bytes[1]));
        assert!((126..=127).contains(&bytes[2]));
        assert_eq!(bytes[3], 255);
    }

    #[test]
    fn fill_circle_writes_center() {
        let mut ctx = PaintContext::new();
        ctx.fill_circle(Point::new(50.0, 50.0), 20.0, Color::rgb(0, 255, 0));
        let mut r = CpuPaintRenderer::new(Size::new(100.0, 100.0), 1000, Color::rgb(0, 0, 0));
        r.execute(&ctx);
        assert!(r.buffer().get_pixel(50, 50).unwrap() > 0);
    }

    #[test]
    fn fill_triangle_writes_centroid() {
        let mut ctx = PaintContext::new();
        ctx.fill_triangle(
            Point::new(10.0, 10.0),
            Point::new(50.0, 10.0),
            Point::new(30.0, 50.0),
            Color::rgb(0, 0, 255),
        );
        let mut r = CpuPaintRenderer::new(Size::new(100.0, 100.0), 1000, Color::rgb(0, 0, 0));
        r.execute(&ctx);
        assert!(r.buffer().get_pixel(30, 23).unwrap() > 0);
    }

    #[test]
    fn fill_rounded_rect_writes_center() {
        let mut ctx = PaintContext::new();
        ctx.fill_rounded_rect(
            Rect::new(Point::new(10.0, 10.0), Size::new(80.0, 80.0)),
            10.0,
            Color::rgb(255, 255, 0),
        );
        let mut r = CpuPaintRenderer::new(Size::new(100.0, 100.0), 1000, Color::rgb(0, 0, 0));
        r.execute(&ctx);
        assert!(r.buffer().get_pixel(50, 50).unwrap() > 0);
    }

    #[test]
    fn clip_skips_outside() {
        let mut ctx = PaintContext::new();
        ctx.push_clip(Rect::new(Point::new(50.0, 50.0), Size::new(10.0, 10.0)));
        ctx.fill_rect(
            Rect::new(Point::new(0.0, 0.0), Size::new(10.0, 10.0)),
            Color::rgb(255, 0, 0),
        );
        ctx.pop_clip();
        let bg = Color::rgb(0, 0, 0);
        let mut r = CpuPaintRenderer::new(Size::new(100.0, 100.0), 1000, bg);
        r.execute(&ctx);
        let bg_pixel = r.buffer().get_pixel(0, 0).unwrap();
        let outside_pixel = r.buffer().get_pixel(5, 5).unwrap();
        assert_eq!(outside_pixel, bg_pixel);
    }

    #[test]
    fn rounded_clip_skips_corners() {
        let bg = Color::rgb(0, 0, 0);
        let fill = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.push_rounded_clip(Rect::from_xywh(0.0, 0.0, 20.0, 20.0), 10.0);
        ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 20.0, 20.0), fill);
        ctx.pop_clip();

        let mut r = CpuPaintRenderer::new(Size::new(30.0, 30.0), 1000, bg);
        r.execute(&ctx);

        assert_eq!(r.buffer().get_pixel(0, 0).unwrap(), bg.to_bgra());
        assert_eq!(r.buffer().get_pixel(10, 10).unwrap(), fill.to_bgra());
    }

    #[test]
    fn clip_layer_composites_only_inside_clip() {
        let bg = Color::rgb(0, 0, 0);
        let fill = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.push_clip(Rect::from_xywh(10.0, 10.0, 20.0, 20.0));
        ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 50.0, 50.0), fill);
        ctx.pop_clip();

        let mut r = CpuPaintRenderer::new(Size::new(50.0, 50.0), 1000, bg);
        r.execute(&ctx);

        let bg_pixel = bg.to_bgra();
        assert_eq!(r.buffer().get_pixel(5, 5).unwrap(), bg_pixel);
        assert_eq!(r.buffer().get_pixel(15, 15).unwrap(), fill.to_bgra());
        assert_eq!(r.buffer().get_pixel(35, 15).unwrap(), bg_pixel);
    }

    #[test]
    fn nested_clip_layers_intersect_during_composite() {
        let bg = Color::rgb(0, 0, 0);
        let fill = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.push_clip(Rect::from_xywh(10.0, 10.0, 30.0, 30.0));
        ctx.push_clip(Rect::from_xywh(20.0, 0.0, 30.0, 30.0));
        ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 60.0, 60.0), fill);
        ctx.pop_clip();
        ctx.pop_clip();

        let mut r = CpuPaintRenderer::new(Size::new(60.0, 60.0), 1000, bg);
        r.execute(&ctx);

        let bg_pixel = bg.to_bgra();
        assert_eq!(r.buffer().get_pixel(25, 15).unwrap(), fill.to_bgra());
        assert_eq!(r.buffer().get_pixel(15, 15).unwrap(), bg_pixel);
        assert_eq!(r.buffer().get_pixel(45, 15).unwrap(), bg_pixel);
        assert_eq!(r.buffer().get_pixel(25, 5).unwrap(), bg_pixel);
    }

    #[test]
    fn draw_buffer_composites() {
        let mut src = Buffer::new(Size::new(5.0, 5.0));
        src.fill_rect(0, 0, 5, 5, Color::rgb(0, 255, 0));
        let mut ctx = PaintContext::new();
        ctx.draw_temporary_buffer(Rect::new(Point::new(10.0, 10.0), Size::new(5.0, 5.0)), src);
        let mut r = CpuPaintRenderer::new(Size::new(100.0, 100.0), 1000, Color::rgb(0, 0, 0));
        r.execute(&ctx);
        assert!(r.buffer().get_pixel(12, 12).unwrap() > 0);
    }

    #[test]
    fn draw_buffer_ref_composites_without_temporary_storage() {
        let mut src = Buffer::new(Size::new(5.0, 5.0));
        src.fill_rect(0, 0, 5, 5, Color::rgb(0, 255, 0));
        let mut ctx = PaintContext::new();
        let handle =
            ctx.draw_buffer_ref(Rect::new(Point::new(10.0, 10.0), Size::new(5.0, 5.0)), &src);

        assert!(!ctx.buffers()[handle.0].is_temporary());

        let mut r = CpuPaintRenderer::new(Size::new(100.0, 100.0), 1000, Color::rgb(0, 0, 0));
        r.execute(&ctx);
        assert!(r.buffer().get_pixel(12, 12).unwrap() > 0);
    }

    #[test]
    fn draw_buffer_rect_uses_source_rect() {
        let mut src = Buffer::new(Size::new(4.0, 4.0));
        let selected = Color::rgb(255, 0, 0);
        src.set_pixel(2, 1, selected.to_bgra());

        let mut ctx = PaintContext::new();
        ctx.draw_buffer_rect_ref(
            Rect::from_xywh(0.0, 0.0, 1.0, 1.0),
            Rect::from_xywh(2.0, 1.0, 1.0, 1.0),
            &src,
            1.0,
        );

        let mut r = CpuPaintRenderer::new(Size::new(4.0, 4.0), 1000, Color::rgb(0, 0, 0));
        r.execute(&ctx);

        assert_eq!(r.buffer().get_pixel(0, 0).unwrap(), selected.to_bgra());
        assert_eq!(
            r.buffer().get_pixel(1, 0).unwrap(),
            Color::rgb(0, 0, 0).to_bgra()
        );
    }

    #[test]
    fn stroke_rect_stays_inside_rect_bounds() {
        let bg = Color::rgb(0, 0, 0);
        let stroke = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.stroke_rect(Rect::from_xywh(0.0, 0.0, 10.0, 10.0), 1.0, stroke);
        let mut r = CpuPaintRenderer::new(Size::new(20.0, 20.0), 1000, bg);
        r.execute(&ctx);

        let bg_pixel = bg.to_bgra();
        assert_eq!(r.buffer().get_pixel(10, 5).unwrap(), bg_pixel);
        assert_eq!(r.buffer().get_pixel(5, 10).unwrap(), bg_pixel);
    }

    #[test]
    fn stroke_rounded_rect_stays_inside_rect_bounds() {
        let bg = Color::rgb(0, 0, 0);
        let stroke = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.stroke_rounded_rect(Rect::from_xywh(0.0, 0.0, 10.0, 10.0), 2.0, 1.0, stroke);
        let mut r = CpuPaintRenderer::new(Size::new(20.0, 20.0), 1000, bg);
        r.execute(&ctx);

        let bg_pixel = bg.to_bgra();
        assert_eq!(r.buffer().get_pixel(10, 5).unwrap(), bg_pixel);
        assert_eq!(r.buffer().get_pixel(5, 10).unwrap(), bg_pixel);
    }

    #[test]
    fn draw_text_command_recorded() {
        let mut ctx = PaintContext::new();
        ctx.draw_text(
            Point::new(10.0, 10.0),
            "Hello",
            Color::rgb(255, 255, 255),
            14.0,
        );
        assert_eq!(ctx.commands().len(), 1);
        assert!(matches!(ctx.commands()[0], PaintCommand::DrawText { .. }));
    }

    #[test]
    fn draw_text_respects_clip() {
        if crate::graphics::default_font_stack().is_none() {
            return;
        }

        let bg = Color::rgb(255, 255, 255);
        let fg = Color::rgb(0, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.push_clip(Rect::from_xywh(0.0, 0.0, 120.0, 12.0));
        ctx.draw_text(Point::new(4.0, 0.0), "Hg", fg, 20.0);
        ctx.pop_clip();

        let mut r = CpuPaintRenderer::new(Size::new(120.0, 40.0), 1000, bg);
        r.execute(&ctx);

        let bg_pixel = bg.to_bgra();
        let mut touched_inside = false;
        for y in 0..12u32 {
            for x in 0..120u32 {
                if r.buffer().get_pixel(x, y).unwrap() != bg_pixel {
                    touched_inside = true;
                    break;
                }
            }
            if touched_inside {
                break;
            }
        }
        assert!(touched_inside);

        for y in 12..40u32 {
            for x in 0..120u32 {
                assert_eq!(r.buffer().get_pixel(x, y).unwrap(), bg_pixel);
            }
        }
    }

    #[test]
    fn clear_removes_all() {
        let mut ctx = PaintContext::new();
        ctx.fill_rect(Rect::zero(), Color::rgb(0, 0, 0));
        ctx.clear();
        assert!(ctx.is_empty());
    }

    #[cfg(feature = "std")]
    #[test]
    #[ignore]
    fn visual_dump() {
        let mut ctx = PaintContext::new();
        ctx.fill_rect(
            Rect::new(Point::new(0.0, 0.0), Size::new(200.0, 200.0)),
            Color::rgb(40, 40, 40),
        );
        ctx.fill_rect(
            Rect::new(Point::new(10.0, 10.0), Size::new(80.0, 80.0)),
            Color::rgb(255, 100, 100),
        );
        ctx.fill_circle(Point::new(150.0, 50.0), 30.0, Color::rgb(100, 255, 100));
        ctx.fill_triangle(
            Point::new(50.0, 110.0),
            Point::new(90.0, 180.0),
            Point::new(10.0, 180.0),
            Color::rgb(100, 100, 255),
        );
        ctx.fill_rounded_rect(
            Rect::new(Point::new(120.0, 120.0), Size::new(70.0, 60.0)),
            15.0,
            Color::rgb(255, 255, 100),
        );
        ctx.stroke_rect(
            Rect::new(Point::new(5.0, 5.0), Size::new(190.0, 190.0)),
            2.0,
            Color::rgb(200, 200, 200),
        );

        let mut r = CpuPaintRenderer::new(Size::new(200.0, 200.0), 1000, Color::rgb(20, 20, 20));
        r.execute(&ctx);

        let w = r.buffer().width();
        let h = r.buffer().height();
        let path = std::env::temp_dir().join("scarlet_paint_test.ppm");
        let mut f = std::fs::File::create(&path).unwrap();
        use std::io::Write;
        writeln!(f, "P6\n{} {}\n255", w, h).unwrap();
        for px in r.buffer().as_slice() {
            let bytes = px.to_le_bytes();
            f.write_all(&[bytes[2], bytes[1], bytes[0]]).unwrap();
        }
        eprintln!("visual dump written to {}", path.display());
    }
}
