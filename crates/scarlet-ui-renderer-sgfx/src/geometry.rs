//! Paint geometry tessellation and rounded-clip processing.

use alloc::vec::Vec;

use scarlet_ui_core::geometry::{Point, Rect};

use crate::error::{Error, Result};

pub(crate) const MAX_FRAME_VERTICES: usize = 196_608;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Vertex {
    pub(crate) position: [f32; 2],
    pub(crate) tex_coord: [f32; 2],
}

impl Vertex {
    const fn solid(point: Point2) -> Self {
        Self {
            position: [point.x, point.y],
            tex_coord: [0.0, 0.0],
        }
    }

    const fn textured(point: Point2, tex_coord: [f32; 2]) -> Self {
        Self {
            position: [point.x, point.y],
            tex_coord,
        }
    }

    fn interpolate(self, other: Self, amount: f32) -> Self {
        Self {
            position: [
                self.position[0] + (other.position[0] - self.position[0]) * amount,
                self.position[1] + (other.position[1] - self.position[1]) * amount,
            ],
            tex_coord: [
                self.tex_coord[0] + (other.tex_coord[0] - self.tex_coord[0]) * amount,
                self.tex_coord[1] + (other.tex_coord[1] - self.tex_coord[1]) * amount,
            ],
        }
    }

    const fn point(self) -> Point2 {
        Point2::new(self.position[0], self.position[1])
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Point2 {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

impl Point2 {
    pub(crate) const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FloatRect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl FloatRect {
    pub(crate) const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub(crate) fn from_logical(rect: Rect, scale: f32) -> Self {
        Self::new(
            rect.origin.x * scale,
            rect.origin.y * scale,
            rect.size.width * scale,
            rect.size.height * scale,
        )
    }

    pub(crate) fn right(self) -> f32 {
        self.x + self.width
    }

    pub(crate) fn bottom(self) -> f32 {
        self.y + self.height
    }

    pub(crate) fn is_empty(self) -> bool {
        !self.x.is_finite()
            || !self.y.is_finite()
            || !self.width.is_finite()
            || !self.height.is_finite()
            || self.width <= 0.0
            || self.height <= 0.0
    }

    pub(crate) fn intersect(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= left || bottom <= top {
            None
        } else {
            Some(Self::new(left, top, right - left, bottom - top))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PixelBounds {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GeometryRange {
    pub(crate) first_vertex: u32,
    pub(crate) vertex_count: u32,
    pub(crate) scissor: PixelBounds,
}

struct ClipShape {
    bounds: FloatRect,
    corner_radius: f32,
    rounded_polygon: Option<Vec<Point2>>,
}

impl ClipShape {
    fn contains_in_rounded_core(&self, vertices: &[Vertex]) -> bool {
        if self.corner_radius <= 0.0 || vertices.is_empty() {
            return false;
        }

        let mut minimum_x = f32::INFINITY;
        let mut minimum_y = f32::INFINITY;
        let mut maximum_x = f32::NEG_INFINITY;
        let mut maximum_y = f32::NEG_INFINITY;
        for vertex in vertices {
            let point = vertex.point();
            minimum_x = minimum_x.min(point.x);
            minimum_y = minimum_y.min(point.y);
            maximum_x = maximum_x.max(point.x);
            maximum_y = maximum_y.max(point.y);
        }

        if minimum_x < self.bounds.x
            || minimum_y < self.bounds.y
            || maximum_x > self.bounds.right()
            || maximum_y > self.bounds.bottom()
        {
            return false;
        }

        let inside_vertical_band = minimum_x >= self.bounds.x + self.corner_radius
            && maximum_x <= self.bounds.right() - self.corner_radius;
        let inside_horizontal_band = minimum_y >= self.bounds.y + self.corner_radius
            && maximum_y <= self.bounds.bottom() - self.corner_radius;
        inside_vertical_band || inside_horizontal_band
    }

    fn excludes(&self, vertices: &[Vertex]) -> bool {
        let mut minimum_x = f32::INFINITY;
        let mut minimum_y = f32::INFINITY;
        let mut maximum_x = f32::NEG_INFINITY;
        let mut maximum_y = f32::NEG_INFINITY;
        for vertex in vertices {
            let point = vertex.point();
            minimum_x = minimum_x.min(point.x);
            minimum_y = minimum_y.min(point.y);
            maximum_x = maximum_x.max(point.x);
            maximum_y = maximum_y.max(point.y);
        }
        maximum_x < self.bounds.x
            || maximum_y < self.bounds.y
            || minimum_x > self.bounds.right()
            || minimum_y > self.bounds.bottom()
    }
}

#[derive(Clone, Copy)]
struct FillEdge {
    start: Point2,
    end: Point2,
}

/// Stateful tessellator for one paint list.
pub(crate) struct Tessellator {
    scale: f32,
    frame_width: u32,
    frame_height: u32,
    render_bounds: FloatRect,
    clip_stack: Vec<ClipShape>,
    vertices: Vec<Vertex>,
}

impl Tessellator {
    pub(crate) fn new(
        scale_milli: u32,
        frame_width: u32,
        frame_height: u32,
        render_bounds: FloatRect,
    ) -> Result<Self> {
        if scale_milli == 0 || frame_width == 0 || frame_height == 0 || render_bounds.is_empty() {
            return Err(Error::InvalidFrame);
        }
        Ok(Self {
            scale: scale_milli as f32 / 1000.0,
            frame_width,
            frame_height,
            render_bounds,
            clip_stack: Vec::new(),
            vertices: Vec::new(),
        })
    }

    pub(crate) fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }

    pub(crate) fn push_clip(&mut self, rect: Rect, corner_radius: f32) -> Result<()> {
        let bounds = FloatRect::from_logical(rect, self.scale);
        if bounds.is_empty() || !corner_radius.is_finite() {
            self.clip_stack.push(ClipShape {
                bounds: FloatRect::new(0.0, 0.0, 0.0, 0.0),
                corner_radius: 0.0,
                rounded_polygon: None,
            });
            return Ok(());
        }
        let radius = (corner_radius.max(0.0) * self.scale)
            .min(bounds.width * 0.5)
            .min(bounds.height * 0.5);
        let rounded_polygon = if radius > 0.0 {
            Some(rounded_rect_points(bounds, radius))
        } else {
            None
        };
        self.clip_stack.push(ClipShape {
            bounds,
            corner_radius: radius,
            rounded_polygon,
        });
        Ok(())
    }

    pub(crate) fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    pub(crate) fn fill_path(&mut self, path: &[Point]) -> Result<Option<GeometryRange>> {
        let mut points = Vec::new();
        points
            .try_reserve(path.len())
            .map_err(|_| Error::FrameTooComplex)?;
        for point in path {
            let point = Point2::new(point.x * self.scale, point.y * self.scale);
            if !point.is_finite() {
                return Err(Error::InvalidFrame);
            }
            if points.last().is_none_or(|last| !points_near(*last, point)) {
                points.push(point);
            }
        }
        if points.len() > 2 && points_near(points[0], points[points.len() - 1]) {
            points.pop();
        }
        if points.len() < 3 {
            return Ok(None);
        }

        let start = self.vertices.len();
        self.tessellate_even_odd(&points)?;
        self.finish_range(start)
    }

    pub(crate) fn stroke_path(
        &mut self,
        path: &[Point],
        stroke_width: f32,
    ) -> Result<Option<GeometryRange>> {
        if !stroke_width.is_finite() {
            return Err(Error::InvalidFrame);
        }
        let mut points = Vec::new();
        points
            .try_reserve(path.len())
            .map_err(|_| Error::FrameTooComplex)?;
        for point in path {
            let point = Point2::new(point.x * self.scale, point.y * self.scale);
            if !point.is_finite() {
                return Err(Error::InvalidFrame);
            }
            points.push(point);
        }
        if points.len() < 2 {
            return Ok(None);
        }

        let start = self.vertices.len();
        let width = stroke_width.max(1.0) * self.scale;
        let closed = points.len() > 2;
        let segment_count = if closed {
            points.len()
        } else {
            points.len() - 1
        };
        for index in 0..segment_count {
            let from = points[index];
            let to = points[(index + 1) % points.len()];
            self.stroke_segment(from, to, width)?;
        }
        self.finish_range(start)
    }

    pub(crate) fn stroke_rect(
        &mut self,
        rect: Rect,
        corner_radius: f32,
        stroke_width: f32,
    ) -> Result<Option<GeometryRange>> {
        if !corner_radius.is_finite() || !stroke_width.is_finite() {
            return Err(Error::InvalidFrame);
        }
        let bounds = FloatRect::from_logical(rect, self.scale);
        if bounds.is_empty() {
            return Ok(None);
        }
        let radius = (corner_radius.max(0.0) * self.scale)
            .min(bounds.width * 0.5)
            .min(bounds.height * 0.5);
        let start = self.vertices.len();
        let width = stroke_width.max(1.0) * self.scale;
        let outer = rounded_rect_points(bounds, radius);
        if width * 2.0 >= bounds.width || width * 2.0 >= bounds.height {
            self.tessellate_even_odd(&outer)?;
            return self.finish_range(start);
        }

        let inner = FloatRect::new(
            bounds.x + width,
            bounds.y + width,
            bounds.width - width * 2.0,
            bounds.height - width * 2.0,
        );
        let inner_radius = (radius - width)
            .max(0.0)
            .min(inner.width * 0.5)
            .min(inner.height * 0.5);
        let segments = if radius > 0.0 {
            ((radius * 0.5) as usize).clamp(4, 16)
        } else {
            1
        };
        let outer = rounded_rect_points_with_segments(bounds, radius, segments);
        let inner = rounded_rect_points_with_segments(inner, inner_radius, segments);
        for index in 0..outer.len() {
            let next = (index + 1) % outer.len();
            self.push_triangle([
                Vertex::solid(outer[index]),
                Vertex::solid(outer[next]),
                Vertex::solid(inner[next]),
            ])?;
            self.push_triangle([
                Vertex::solid(outer[index]),
                Vertex::solid(inner[next]),
                Vertex::solid(inner[index]),
            ])?;
        }
        self.finish_range(start)
    }

    pub(crate) fn textured_rect(
        &mut self,
        destination: FloatRect,
        tex_coords: [[f32; 2]; 4],
    ) -> Result<Option<GeometryRange>> {
        if destination.is_empty()
            || !tex_coords
                .iter()
                .flatten()
                .all(|component| component.is_finite())
        {
            return if destination.is_empty() {
                Ok(None)
            } else {
                Err(Error::InvalidFrame)
            };
        }
        let points = [
            Point2::new(destination.x, destination.y),
            Point2::new(destination.right(), destination.y),
            Point2::new(destination.right(), destination.bottom()),
            Point2::new(destination.x, destination.bottom()),
        ];
        let start = self.vertices.len();
        self.push_triangle([
            Vertex::textured(points[0], tex_coords[0]),
            Vertex::textured(points[1], tex_coords[1]),
            Vertex::textured(points[2], tex_coords[2]),
        ])?;
        self.push_triangle([
            Vertex::textured(points[0], tex_coords[0]),
            Vertex::textured(points[2], tex_coords[2]),
            Vertex::textured(points[3], tex_coords[3]),
        ])?;
        self.finish_range(start)
    }

    pub(crate) fn dummy_draw(&mut self) -> Result<GeometryRange> {
        let Some(scissor) = self.effective_scissor() else {
            return Err(Error::InvalidFrame);
        };
        if self.vertices.len().saturating_add(3) > MAX_FRAME_VERTICES {
            return Err(Error::FrameTooComplex);
        }
        let first_vertex =
            u32::try_from(self.vertices.len()).map_err(|_| Error::FrameTooComplex)?;
        let point = Point2::new(scissor.x as f32, scissor.y as f32);
        self.vertices.extend_from_slice(&[
            Vertex::solid(point),
            Vertex::solid(point),
            Vertex::solid(point),
        ]);
        Ok(GeometryRange {
            first_vertex,
            vertex_count: 3,
            scissor,
        })
    }

    fn tessellate_even_odd(&mut self, points: &[Point2]) -> Result<()> {
        let mut edges = Vec::new();
        let mut bands = Vec::new();
        edges
            .try_reserve(points.len())
            .map_err(|_| Error::FrameTooComplex)?;
        bands
            .try_reserve(points.len())
            .map_err(|_| Error::FrameTooComplex)?;
        for index in 0..points.len() {
            let start = points[index];
            let end = points[(index + 1) % points.len()];
            bands.push(start.y);
            if (start.y - end.y).abs() > 0.0001 {
                edges.push(FillEdge { start, end });
            }
        }
        if edges.len() < 2 {
            return Ok(());
        }

        // Edge crossings are band boundaries too. Between consecutive
        // boundaries the left-to-right edge order is stable, so pairing
        // crossings produces the exact even-odd interior, including
        // self-intersecting paths.
        for left in 0..edges.len() {
            for right in left + 1..edges.len() {
                if let Some(y) = edge_intersection_y(edges[left], edges[right]) {
                    bands
                        .try_reserve(1)
                        .map_err(|_| Error::FrameTooComplex)?;
                    bands.push(y);
                }
            }
        }
        bands.sort_by(|left, right| {
            left.partial_cmp(right).unwrap_or(core::cmp::Ordering::Equal)
        });
        bands.dedup_by(|left, right| (*left - *right).abs() <= 0.0001);

        let mut crossings: Vec<(f32, usize)> = Vec::new();
        for band in bands.windows(2) {
            let top = band[0];
            let bottom = band[1];
            if bottom - top <= 0.0001 {
                continue;
            }
            let middle = top + (bottom - top) * 0.5;
            crossings.clear();
            for (index, edge) in edges.iter().copied().enumerate() {
                let minimum = edge.start.y.min(edge.end.y);
                let maximum = edge.start.y.max(edge.end.y);
                if middle > minimum && middle < maximum {
                    crossings
                        .try_reserve(1)
                        .map_err(|_| Error::FrameTooComplex)?;
                    crossings.push((edge_x_at(edge, middle), index));
                }
            }
            crossings.sort_by(|left, right| {
                left.0
                    .partial_cmp(&right.0)
                    .unwrap_or(core::cmp::Ordering::Equal)
            });
            for pair in crossings.chunks_exact(2) {
                let left = edges[pair[0].1];
                let right = edges[pair[1].1];
                let left_top = edge_x_at(left, top);
                let left_bottom = edge_x_at(left, bottom);
                let right_top = edge_x_at(right, top);
                let right_bottom = edge_x_at(right, bottom);
                if (right_top - left_top).abs() <= 0.0001
                    && (right_bottom - left_bottom).abs() <= 0.0001
                {
                    continue;
                }
                let top_left = Point2::new(left_top, top);
                let top_right = Point2::new(right_top, top);
                let bottom_right = Point2::new(right_bottom, bottom);
                let bottom_left = Point2::new(left_bottom, bottom);
                self.push_triangle([
                    Vertex::solid(top_left),
                    Vertex::solid(top_right),
                    Vertex::solid(bottom_right),
                ])?;
                self.push_triangle([
                    Vertex::solid(top_left),
                    Vertex::solid(bottom_right),
                    Vertex::solid(bottom_left),
                ])?;
            }
        }
        Ok(())
    }

    fn stroke_segment(&mut self, from: Point2, to: Point2, width: f32) -> Result<()> {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let length_squared = dx * dx + dy * dy;
        if !length_squared.is_finite() {
            return Err(Error::InvalidFrame);
        }
        let half = width * 0.5;
        if length_squared < 0.000_001 {
            let a = Point2::new(from.x - half, from.y - half);
            let b = Point2::new(from.x + half, from.y - half);
            let c = Point2::new(from.x + half, from.y + half);
            let d = Point2::new(from.x - half, from.y + half);
            self.push_triangle([Vertex::solid(a), Vertex::solid(b), Vertex::solid(c)])?;
            return self.push_triangle([Vertex::solid(a), Vertex::solid(c), Vertex::solid(d)]);
        }
        let inverse_length = inverse_sqrt(length_squared);
        let direction_x = dx * inverse_length;
        let direction_y = dy * inverse_length;
        let normal_x = -dy * inverse_length * half;
        let normal_y = dx * inverse_length * half;
        let start = Point2::new(
            from.x - direction_x * half,
            from.y - direction_y * half,
        );
        let end = Point2::new(to.x + direction_x * half, to.y + direction_y * half);
        let a = Point2::new(start.x + normal_x, start.y + normal_y);
        let b = Point2::new(end.x + normal_x, end.y + normal_y);
        let c = Point2::new(end.x - normal_x, end.y - normal_y);
        let d = Point2::new(start.x - normal_x, start.y - normal_y);
        self.push_triangle([Vertex::solid(a), Vertex::solid(b), Vertex::solid(c)])?;
        self.push_triangle([Vertex::solid(a), Vertex::solid(c), Vertex::solid(d)])
    }

    fn push_triangle(&mut self, triangle: [Vertex; 3]) -> Result<()> {
        let mut polygon: Option<Vec<Vertex>> = None;
        for clip in &self.clip_stack {
            let Some(rounded) = clip.rounded_polygon.as_ref() else {
                continue;
            };
            let vertices = polygon.as_deref().unwrap_or(&triangle);
            if clip.contains_in_rounded_core(vertices) {
                continue;
            }
            if clip.excludes(vertices) {
                return Ok(());
            }
            let clipped = clip_to_convex_polygon(vertices, rounded)?;
            if clipped.len() < 3 {
                return Ok(());
            }
            polygon = Some(clipped);
        }
        let additional = polygon
            .as_ref()
            .map_or(3, |vertices| vertices.len().saturating_sub(2).saturating_mul(3));
        if self.vertices.len().saturating_add(additional) > MAX_FRAME_VERTICES {
            return Err(Error::FrameTooComplex);
        }
        self.vertices
            .try_reserve(additional)
            .map_err(|_| Error::FrameTooComplex)?;
        if let Some(polygon) = polygon {
            for index in 1..polygon.len() - 1 {
                self.vertices.push(polygon[0]);
                self.vertices.push(polygon[index]);
                self.vertices.push(polygon[index + 1]);
            }
        } else {
            self.vertices.extend_from_slice(&triangle);
        }
        Ok(())
    }

    fn finish_range(&self, start: usize) -> Result<Option<GeometryRange>> {
        let count = self.vertices.len().saturating_sub(start);
        if count == 0 {
            return Ok(None);
        }
        let Some(scissor) = self.effective_scissor() else {
            return Ok(None);
        };
        Ok(Some(GeometryRange {
            first_vertex: u32::try_from(start).map_err(|_| Error::FrameTooComplex)?,
            vertex_count: u32::try_from(count).map_err(|_| Error::FrameTooComplex)?,
            scissor,
        }))
    }

    fn effective_scissor(&self) -> Option<PixelBounds> {
        let mut bounds = self.render_bounds;
        for clip in &self.clip_stack {
            bounds = bounds.intersect(clip.bounds)?;
        }
        pixel_bounds(bounds, self.frame_width, self.frame_height)
    }
}

fn pixel_bounds(rect: FloatRect, frame_width: u32, frame_height: u32) -> Option<PixelBounds> {
    if rect.is_empty() {
        return None;
    }
    let left = floor_i32(rect.x).max(0) as u32;
    let top = floor_i32(rect.y).max(0) as u32;
    let right = ceil_i32(rect.right()).max(0) as u32;
    let bottom = ceil_i32(rect.bottom()).max(0) as u32;
    let left = left.min(frame_width);
    let top = top.min(frame_height);
    let right = right.min(frame_width);
    let bottom = bottom.min(frame_height);
    if right <= left || bottom <= top {
        None
    } else {
        Some(PixelBounds {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
    }
}

fn rounded_rect_points(rect: FloatRect, radius: f32) -> Vec<Point2> {
    if radius <= 0.0 {
        return alloc::vec![
            Point2::new(rect.x, rect.y),
            Point2::new(rect.right(), rect.y),
            Point2::new(rect.right(), rect.bottom()),
            Point2::new(rect.x, rect.bottom()),
        ];
    }
    let segments = ((radius * 0.5) as usize).clamp(4, 16);
    rounded_rect_points_with_segments(rect, radius, segments)
}

fn rounded_rect_points_with_segments(
    rect: FloatRect,
    radius: f32,
    segments: usize,
) -> Vec<Point2> {
    let centers = [
        (rect.x + radius, rect.y + radius, core::f32::consts::PI),
        (
            rect.right() - radius,
            rect.y + radius,
            core::f32::consts::FRAC_PI_2 * 3.0,
        ),
        (rect.right() - radius, rect.bottom() - radius, 0.0),
        (
            rect.x + radius,
            rect.bottom() - radius,
            core::f32::consts::FRAC_PI_2,
        ),
    ];
    let mut points = Vec::new();
    let _ = points.try_reserve(segments.saturating_add(1).saturating_mul(4));
    for (center_x, center_y, start_angle) in centers {
        for index in 0..=segments {
            let angle = start_angle
                + core::f32::consts::FRAC_PI_2 * index as f32 / segments as f32;
            points.push(Point2::new(
                center_x + radius * cosine(angle),
                center_y + radius * sine(angle),
            ));
        }
    }
    points
}

fn clip_to_convex_polygon(vertices: &[Vertex], clip: &[Point2]) -> Result<Vec<Vertex>> {
    if vertices.len() < 3 || clip.len() < 3 {
        return Ok(Vec::new());
    }
    let orientation = if signed_area(clip) >= 0.0 { 1.0 } else { -1.0 };
    let mut input = Vec::from(vertices);
    let mut output = Vec::new();
    output
        .try_reserve(vertices.len().saturating_add(clip.len()))
        .map_err(|_| Error::FrameTooComplex)?;
    for edge_index in 0..clip.len() {
        let edge_start = clip[edge_index];
        let edge_end = clip[(edge_index + 1) % clip.len()];
        if input.is_empty() {
            break;
        }
        output.clear();
        let mut previous = input[input.len() - 1];
        let mut previous_distance = edge_distance(edge_start, edge_end, previous.point(), orientation);
        for current in input.iter().copied() {
            let current_distance = edge_distance(edge_start, edge_end, current.point(), orientation);
            let previous_inside = previous_distance >= -0.0001;
            let current_inside = current_distance >= -0.0001;
            if previous_inside != current_inside {
                let denominator = previous_distance - current_distance;
                if denominator.abs() > f32::EPSILON {
                    let amount = (previous_distance / denominator).clamp(0.0, 1.0);
                    output.push(previous.interpolate(current, amount));
                }
            }
            if current_inside {
                output.push(current);
            }
            previous = current;
            previous_distance = current_distance;
        }
        core::mem::swap(&mut input, &mut output);
    }
    Ok(input)
}

fn edge_distance(start: Point2, end: Point2, point: Point2, orientation: f32) -> f32 {
    ((end.x - start.x) * (point.y - start.y) - (end.y - start.y) * (point.x - start.x))
        * orientation
}

fn signed_area(points: &[Point2]) -> f32 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        area += current.x * next.y - next.x * current.y;
    }
    area * 0.5
}

fn edge_x_at(edge: FillEdge, y: f32) -> f32 {
    let amount = (y - edge.start.y) / (edge.end.y - edge.start.y);
    edge.start.x + (edge.end.x - edge.start.x) * amount
}

fn edge_intersection_y(left: FillEdge, right: FillEdge) -> Option<f32> {
    let left_dx = left.end.x - left.start.x;
    let left_dy = left.end.y - left.start.y;
    let right_dx = right.end.x - right.start.x;
    let right_dy = right.end.y - right.start.y;
    let denominator = left_dx * right_dy - left_dy * right_dx;
    if denominator.abs() <= 0.0001 {
        return None;
    }
    let offset_x = right.start.x - left.start.x;
    let offset_y = right.start.y - left.start.y;
    let left_amount = (offset_x * right_dy - offset_y * right_dx) / denominator;
    let right_amount = (offset_x * left_dy - offset_y * left_dx) / denominator;
    if left_amount <= 0.0001
        || left_amount >= 0.9999
        || right_amount <= 0.0001
        || right_amount >= 0.9999
    {
        return None;
    }
    Some(left.start.y + left_dy * left_amount)
}

fn points_near(left: Point2, right: Point2) -> bool {
    (left.x - right.x).abs() <= 0.0001 && (left.y - right.y).abs() <= 0.0001
}

fn floor_i32(value: f32) -> i32 {
    let integer = value as i32;
    if (integer as f32) > value {
        integer - 1
    } else {
        integer
    }
}

fn ceil_i32(value: f32) -> i32 {
    let integer = value as i32;
    if (integer as f32) < value {
        integer + 1
    } else {
        integer
    }
}

fn inverse_sqrt(value: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }
    let mut estimate = if value >= 1.0 { value } else { 1.0 };
    for _ in 0..8 {
        estimate = 0.5 * (estimate + value / estimate);
    }
    1.0 / estimate
}

fn sine(angle: f32) -> f32 {
    libm::sinf(angle)
}

fn cosine(angle: f32) -> f32 {
    libm::cosf(angle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_clip_keeps_a_centered_textured_rect() {
        let bounds = FloatRect::new(0.0, 0.0, 2_048.0, 1_440.0);
        let mut tessellator =
            Tessellator::new(2_000, 2_048, 1_440, bounds).expect("valid tessellator");
        tessellator
            .push_clip(Rect::from_xywh(0.0, 0.0, 1_024.0, 720.0), 16.0)
            .expect("valid rounded clip");

        let geometry = tessellator
            .textured_rect(
                FloatRect::new(800.0, 600.0, 64.0, 64.0),
                [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            )
            .expect("valid textured rectangle")
            .expect("centered rectangle must survive the clip");

        assert_eq!(geometry.vertex_count, 6);
    }

    #[test]
    fn rounded_rect_polygon_is_convex_and_contains_its_center() {
        let rect = FloatRect::new(0.0, 0.0, 2_048.0, 1_440.0);
        let points = rounded_rect_points_with_segments(rect, 32.0, 16);
        let orientation = if signed_area(&points) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let center = Point2::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);

        for index in 0..points.len() {
            let start = points[index];
            let end = points[(index + 1) % points.len()];
            assert!(edge_distance(start, end, center, orientation) >= -0.0001);
        }
    }

    #[test]
    fn rounded_corner_clipping_preserves_finite_texture_coordinates() {
        let bounds = FloatRect::new(0.0, 0.0, 200.0, 200.0);
        let mut tessellator =
            Tessellator::new(1_000, 200, 200, bounds).expect("valid tessellator");
        tessellator
            .push_clip(Rect::from_xywh(0.0, 0.0, 200.0, 200.0), 32.0)
            .expect("valid rounded clip");

        let geometry = tessellator
            .textured_rect(
                FloatRect::new(0.0, 0.0, 48.0, 48.0),
                [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            )
            .expect("valid textured rectangle")
            .expect("corner overlap must retain visible geometry");
        let start = geometry.first_vertex as usize;
        let end = start + geometry.vertex_count as usize;

        for vertex in &tessellator.vertices()[start..end] {
            assert!(vertex.tex_coord.iter().all(|value| value.is_finite()));
            assert!(
                vertex
                    .tex_coord
                    .iter()
                    .all(|value| (0.0..=1.0).contains(value))
            );
        }
    }
}
