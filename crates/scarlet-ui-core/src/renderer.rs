use alloc::string::String;
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
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect()
}

pub fn path_rounded_rect(rect: Rect, corner_radius: f32) -> Path {
    let r = corner_radius
        .min(rect.size.width / 2.0)
        .min(rect.size.height / 2.0);
    let corner_segments = ((r * 0.75).ceil() as usize).clamp(8, 32);
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
            pts.push(Point::new(cx + r * t.cos(), cy + r * t.sin()));
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
    },
    PopClip,
    SetOpacity {
        opacity: f32,
    },
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
        self.stroke_path(path_rect(rect), stroke_width, color);
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

    pub fn draw_buffer(&mut self, dst: Rect, buffer: Buffer) -> BufferHandle {
        let idx = self.buffers.len();
        self.buffers.push(buffer);
        self.commands.push(PaintCommand::DrawBuffer {
            dst,
            buffer_idx: idx,
        });
        BufferHandle(idx)
    }

    pub fn draw_buffer_rect(
        &mut self,
        dst: Rect,
        src: Rect,
        buffer: Buffer,
        opacity: f32,
    ) -> BufferHandle {
        let idx = self.buffers.len();
        self.buffers.push(buffer);
        self.commands.push(PaintCommand::DrawBufferRect {
            dst,
            src,
            buffer_idx: idx,
            opacity,
        });
        BufferHandle(idx)
    }

    pub fn push_clip(&mut self, rect: Rect) {
        self.commands.push(PaintCommand::PushClip { rect });
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

pub struct CpuPaintRenderer {
    buffer: Buffer,
    background_color: Color,
    scale_milli: u32,
    clip_stack: Vec<Rect>,
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
            clip_stack: Vec::new(),
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

    pub fn execute(&mut self, ctx: &PaintContext) {
        self.buffer.clear(self.background_color);
        self.clip_stack.clear();

        for cmd in ctx.commands() {
            let clip = self.clip_stack.last().copied();
            match cmd {
                PaintCommand::FillPath { path, color } => {
                    let scaled: Vec<Point> =
                        path.iter().copied().map(|p| self.scale_point(p)).collect();
                    let clip_i = clip.map(|c| {
                        let r = self.scale_rect(c);
                        (r.origin.x, r.origin.y, r.size.width, r.size.height)
                    });
                    fill_polygon(&mut self.buffer, &scaled, *color, clip_i);
                }
                PaintCommand::StrokePath {
                    path,
                    stroke_width,
                    color,
                } => {
                    let sw = stroke_width.max(1.0) as i32;
                    let mut canvas = crate::graphics::Canvas::for_buffer(&mut self.buffer);
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
                    let mut canvas = crate::graphics::Canvas::for_buffer(&mut self.buffer);
                    canvas.draw_text_sized(
                        position.x as i32,
                        position.y as i32,
                        text,
                        *color,
                        *font_size_px,
                    );
                }
                PaintCommand::DrawBuffer { dst, buffer_idx } => {
                    if let Some(src) = ctx.buffers().get(*buffer_idx) {
                        let dst = self.scale_rect(*dst);
                        let dst_x = dst.origin.x as i32;
                        let dst_y = dst.origin.y as i32;
                        if let Some(c) = clip {
                            let c = self.scale_rect(c);
                            self.buffer.composite_clipped(
                                src,
                                dst_x,
                                dst_y,
                                1.0,
                                c.origin.x as i32,
                                c.origin.y as i32,
                                c.size.width as i32,
                                c.size.height as i32,
                            );
                        } else {
                            self.buffer.composite(src, dst_x, dst_y, 1.0);
                        }
                    }
                }
                PaintCommand::DrawBufferRect {
                    dst,
                    src: _,
                    buffer_idx,
                    opacity,
                } => {
                    if let Some(buf) = ctx.buffers().get(*buffer_idx) {
                        let dst = self.scale_rect(*dst);
                        let dst_x = dst.origin.x as i32;
                        let dst_y = dst.origin.y as i32;
                        self.buffer.composite(buf, dst_x, dst_y, *opacity);
                    }
                }
                PaintCommand::PushClip { rect } => {
                    self.clip_stack.push(*rect);
                }
                PaintCommand::PopClip => {
                    self.clip_stack.pop();
                }
                PaintCommand::SetOpacity { opacity: _ } => {}
            }
        }
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub fn set_background_color(&mut self, color: Color) {
        self.background_color = color;
    }
}

fn fill_polygon(
    buffer: &mut Buffer,
    path: &[Point],
    color: Color,
    clip: Option<(f32, f32, f32, f32)>,
) {
    if path.len() < 3 {
        return;
    }
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for p in path {
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }
    if let Some((_, cy, _, ch)) = clip {
        min_y = min_y.max(cy);
        max_y = max_y.min(cy + ch);
    }
    let min_y = min_y.floor() as i32;
    let max_y = max_y.ceil() as i32;
    let bw = buffer.width() as i32;
    let bh = buffer.height() as i32;
    let bgra = color.to_bgra();
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
            let x_start = ((pair[0] - 0.5).ceil() as i32).max(0);
            let x_end = ((pair[1] - 0.5).ceil() as i32).min(bw);
            if x_start >= x_end || x_start >= bw {
                continue;
            }
            let start = (y as usize * bw as usize) + x_start as usize;
            let count = (x_end - x_start) as usize;
            let row = &mut data[start..start + count];
            row.fill(bgra);
        }
    }
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
    fn draw_buffer_composites() {
        let mut src = Buffer::new(Size::new(5.0, 5.0));
        src.fill_rect(0, 0, 5, 5, Color::rgb(0, 255, 0));
        let mut ctx = PaintContext::new();
        ctx.draw_buffer(Rect::new(Point::new(10.0, 10.0), Size::new(5.0, 5.0)), src);
        let mut r = CpuPaintRenderer::new(Size::new(100.0, 100.0), 1000, Color::rgb(0, 0, 0));
        r.execute(&ctx);
        assert!(r.buffer().get_pixel(12, 12).unwrap() > 0);
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
