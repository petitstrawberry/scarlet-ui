use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt::Debug;

use crate::buffer::Buffer;
use crate::color::Color;
use crate::compositor::{Compositor, DamageRect};
use crate::element::{Element, ElementId};
use crate::error::Result;
use crate::geometry::{Offset, Point, Rect, Size};
use crate::icon::{Icon, IconStyle};

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct FrameSize {
    pub width: f32,
    pub height: f32,
    pub scale_milli: u32,
}

pub type Path = Vec<Point>;

#[path = "paint_dependencies.rs"]
mod dependencies;
pub use dependencies::{PaintDependencies, PaintRegion};

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
        for i in 0..=corner_segments {
            let t =
                start_angle + core::f32::consts::FRAC_PI_2 * (i as f32) / (corner_segments as f32);
            pts.push(Point::new(cx + r * libm::cosf(t), cy + r * libm::sinf(t)));
        }
    }
    pts
}

fn interpolate_color(start: Color, end: Color, amount: f32) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    Color {
        r: start.r + (end.r - start.r) * amount,
        g: start.g + (end.g - start.g) * amount,
        b: start.b + (end.b - start.b) * amount,
        a: start.a + (end.a - start.a) * amount,
    }
}

/// Type-erased renderer-specific paint payload.
///
/// Platform-independent views can place an extension in the normal paint
/// order without making ScarletUI core depend on a particular renderer. A
/// backend that recognizes the concrete payload may render it; other backends
/// leave the pixels produced by preceding paint commands unchanged.
pub trait PaintExtension: Any + Debug {
    /// Return this payload as [`Any`] for backend downcasting.
    ///
    /// # Returns
    ///
    /// The type-erased payload.
    fn as_any(&self) -> &dyn Any;
}

impl<T> PaintExtension for T
where
    T: Any + Debug,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaintCommand {
    /// Replay an immutable local-coordinate display list at a new position.
    DrawDisplayList {
        origin: Point,
        list: Arc<DisplayList>,
    },
    FillPath {
        path: Path,
        color: Color,
    },
    FillRoundedRect {
        rect: Rect,
        corner_radius: f32,
        color: Color,
    },
    /// Fill an axis-aligned rounded rectangle with a vertical color gradient.
    FillVerticalGradientRoundedRect {
        rect: Rect,
        corner_radius: f32,
        top_color: Color,
        bottom_color: Color,
    },
    /// Blur pixels already painted behind this rounded rectangle, then apply a
    /// uniform translucent tint. Later commands (including labels) stay sharp.
    /// CPU blur uses three separable box passes in premultiplied color, with a
    /// total physical support capped at 64 pixels. When the blur exceeds the
    /// bounded ROI/frame budget, the tint's RGB becomes an opaque fallback to
    /// retain readability. Explicit zero-radius tint-only painting preserves
    /// its requested alpha. This is destination-dependent and must not be cached
    /// as an isolated transparent picture. Within rounded clips it establishes
    /// a compositing boundary: content before and after the material receives
    /// clip coverage independently, rather than sharing one isolated group.
    BackdropFilterRoundedRect {
        rect: Rect,
        corner_radius: f32,
        blur_radius: f32,
        tint: Color,
        /// Convex edge displacement strength, 0 disables optical treatment.
        refraction: f32,
    },
    /// Draw a blurred rounded-rectangle shadow behind `rect`.
    DrawRoundedRectShadow {
        rect: Rect,
        corner_radius: f32,
        offset: Offset,
        blur_radius: f32,
        spread_radius: f32,
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
    /// Draw a typed icon as a theme-tinted alpha mask.
    DrawIcon {
        /// Logical destination rectangle.
        rect: Rect,
        /// Selected icon.
        icon: Icon,
        /// Outline rasterization style.
        style: IconStyle,
        /// Tint applied after mask rasterization.
        color: Color,
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
    /// Renderer-specific content placed at this point in paint order.
    Extension {
        /// Logical destination rectangle.
        rect: Rect,
        /// Type-erased backend payload.
        payload: Arc<dyn PaintExtension>,
        dependencies: PaintDependencies,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferHandle(pub usize);

/// Immutable paint commands and owned resources, independent of live Elements.
/// A new identity is allocated only when the recorded content changes.
pub struct DisplayList {
    identity: u64,
    cache_identity: u64,
    flat: core::cell::OnceCell<PaintContext<'static>>,
    footprints: core::cell::RefCell<Vec<(u32, Arc<[PaintDependencies]>)>>,
    context: PaintContext<'static>,
    self_contained: bool,
    raster_bounds: Option<Rect>,
    reads_backdrop: bool,
    has_extensions: bool,
}

impl PartialEq for dyn PaintExtension {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self, other)
    }
}
impl PartialEq for DisplayList {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl Debug for DisplayList {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DisplayList")
            .field("identity", &self.identity)
            .field("commands", &self.context.commands.len())
            .finish()
    }
}

impl DisplayList {
    /// Cached dependency on paint preceding this recording.
    pub fn reads_backdrop(&self) -> bool {
        self.reads_backdrop
    }
    /// Whether this recording contains backend-specific payloads.
    pub fn has_extensions(&self) -> bool {
        self.has_extensions
    }
    /// Conservative local bounds of a layer eligible for GPU raster retention.
    /// This is a hint; unsupported commands must retain normal paint semantics.
    pub fn raster_cache_bounds(&self) -> Option<Rect> {
        self.raster_bounds
    }
    /// Stable lineage for successive immutable revisions of one cached picture.
    pub fn cache_identity(&self) -> u64 {
        if self.cache_identity == 0 {
            self.identity
        } else {
            self.cache_identity
        }
    }

    /// Flat command view, materialized once per revision for raster backends.
    /// The original context keeps the identities of unchanged child recordings.
    pub fn flat_context(&self) -> &PaintContext<'static> {
        if self
            .context
            .commands
            .iter()
            .any(|c| matches!(c, PaintCommand::DrawDisplayList { .. }))
        {
            self.flat
                .get_or_init(|| self.context.flattened().into_owned())
        } else {
            &self.context
        }
    }

    /// Stable identity of this immutable recording.
    pub fn identity(&self) -> u64 {
        self.identity
    }

    /// Whether clip/opacity state stays inside the recording.
    /// Stateful recordings must be lowered together with their surrounding commands.
    pub fn is_self_contained(&self) -> bool {
        self.self_contained
    }

    /// Commands and resources retained by this recording.
    pub fn context(&self) -> &PaintContext<'static> {
        &self.context
    }
}

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

    /// Adopt a recording's commands and resources without borrowing its element.
    /// Immutable images share their pixels. Legacy borrowed pixel buffers are
    /// snapshotted once when their paint content changes, never on replay.
    pub fn into_display_list(self) -> Arc<DisplayList> {
        static NEXT_LIST_ID: crate::id::IdAllocator = crate::id::IdAllocator::new();
        let mut depth = 0usize;
        let mut self_contained = true;
        for command in &self.commands {
            match command {
                PaintCommand::PushClip { .. } => depth += 1,
                PaintCommand::PopClip if depth > 0 => depth -= 1,
                PaintCommand::PopClip | PaintCommand::SetOpacity { .. } => self_contained = false,
                PaintCommand::DrawDisplayList { list, .. } if !list.is_self_contained() => {
                    self_contained = false
                }
                _ => {}
            }
        }
        Arc::new(DisplayList {
            reads_backdrop: self.commands.iter().any(|c| match c {
                PaintCommand::BackdropFilterRoundedRect { .. } => true,
                PaintCommand::DrawDisplayList { list, .. } => list.reads_backdrop(),
                _ => false,
            }),
            has_extensions: self.commands.iter().any(|c| match c {
                PaintCommand::Extension { .. } => true,
                PaintCommand::DrawDisplayList { list, .. } => list.has_extensions(),
                _ => false,
            }),
            raster_bounds: None,
            self_contained: self_contained && depth == 0,
            identity: NEXT_LIST_ID.allocate(),
            cache_identity: 0,
            flat: core::cell::OnceCell::new(),
            footprints: core::cell::RefCell::new(Vec::new()),
            context: PaintContext {
                commands: self.commands,
                buffers: self
                    .buffers
                    .into_iter()
                    .map(|buffer| match buffer {
                        PaintBuffer::Shared(buffer) => PaintBuffer::Shared(buffer),
                        PaintBuffer::Temporary(buffer) => PaintBuffer::Shared(Arc::new(buffer)),
                        PaintBuffer::Borrowed(buffer) => {
                            PaintBuffer::Shared(Arc::new(buffer.clone()))
                        }
                    })
                    .collect(),
            },
        })
    }

    /// Record an immutable layer whose unchanged pixels can be retained on the GPU.
    /// `bounds` must include all painting, including shadows and overflowing children.
    /// Backdrop reads must be covered by earlier opaque paint in this recording;
    /// otherwise the result remains a normal display list without a raster hint.
    pub fn into_cached_layer(self, bounds: Rect) -> Arc<DisplayList> {
        let isolated = self.flattened().backdrop_is_local();
        let mut list = self.into_display_list();
        Arc::get_mut(&mut list).unwrap().raster_bounds = isolated.then_some(bounds);
        list
    }

    /// Retain the cache lineage while replacing an immutable picture's children.
    /// Backends compare revisions and redraw only affected regions; earlier
    /// snapshots remain immutable while an in-flight frame still references them.
    pub fn into_cached_layer_revision(
        self,
        bounds: Rect,
        previous: &DisplayList,
    ) -> Arc<DisplayList> {
        let mut list = self.into_cached_layer(bounds);
        Arc::get_mut(&mut list).unwrap().cache_identity = previous.cache_identity();
        list
    }

    /// Whether painting again produced the same commands and immutable resources.
    /// This does not suppress state notifications, builds or layout.
    pub fn same_recording(&self, previous: &DisplayList) -> bool {
        // Extension payloads may contain interior-mutable GPU resources. Pointer
        // equality is not a content revision, even with known spatial footprints.
        !self
            .commands
            .iter()
            .any(|c| matches!(c, PaintCommand::Extension { .. }))
            && self.commands == previous.context.commands
            && self.buffers.len() == previous.context.buffers.len()
            && self
                .buffers
                .iter()
                .zip(&previous.context.buffers)
                .all(|(a, b)| {
                    let (a, b) = (a.as_buffer(), b.as_buffer());
                    (a.identity() == b.identity() && a.revision() == b.revision())
                        || (a.width() == b.width()
                            && a.height() == b.height()
                            && a.scale_milli() == b.scale_milli()
                            && a.as_slice() == b.as_slice())
                })
    }

    fn into_owned(self) -> PaintContext<'static> {
        PaintContext {
            commands: self.commands,
            buffers: self
                .buffers
                .into_iter()
                .map(|buffer| match buffer {
                    PaintBuffer::Shared(b) => PaintBuffer::Shared(b),
                    PaintBuffer::Temporary(b) => PaintBuffer::Shared(Arc::new(b)),
                    PaintBuffer::Borrowed(b) => PaintBuffer::Shared(Arc::new(b.clone())),
                })
                .collect(),
        }
    }

    // Prove that each filter reads only earlier opaque paint in this recording.
    // A glass label alone is not isolated; a card including its opaque backing is.
    fn backdrop_is_local(&self) -> bool {
        fn add_backing(opaque: &mut Vec<Rect>, clips: &[Rect], mut bounds: Rect) {
            for clip in clips {
                let left = bounds.left().max(clip.left());
                let top = bounds.top().max(clip.top());
                bounds = Rect::from_xywh(
                    left,
                    top,
                    (bounds.right().min(clip.right()) - left).max(0.),
                    (bounds.bottom().min(clip.bottom()) - top).max(0.),
                );
            }
            opaque.push(bounds);
        }
        let mut opaque: Vec<Rect> = Vec::new();
        let mut clips: Vec<Rect> = Vec::new();
        for command in &self.commands {
            match command {
                PaintCommand::PushClip {
                    rect,
                    corner_radius,
                } => {
                    let r = corner_radius
                        .max(0.)
                        .min(rect.size.width * 0.5)
                        .min(rect.size.height * 0.5);
                    clips.push(Rect::from_xywh(
                        rect.left() + r,
                        rect.top() + r,
                        (rect.size.width - 2. * r).max(0.),
                        (rect.size.height - 2. * r).max(0.),
                    ));
                }
                PaintCommand::PopClip => {
                    clips.pop();
                }
                PaintCommand::FillRoundedRect {
                    rect,
                    corner_radius,
                    color,
                } if color.a >= 1. => {
                    let r = corner_radius
                        .max(0.)
                        .min(rect.size.width * 0.5)
                        .min(rect.size.height * 0.5);
                    let bounds = Rect::from_xywh(
                        rect.left() + r,
                        rect.top() + r,
                        (rect.size.width - 2. * r).max(0.),
                        (rect.size.height - 2. * r).max(0.),
                    );
                    add_backing(&mut opaque, &clips, bounds);
                }
                PaintCommand::FillPath { path, color } if color.a >= 1. => {
                    // fill_rect, including the standard Background modifier,
                    // records an axis-aligned rectangle as a four-point path.
                    // Prove the actual edges, never assume a path's bounding
                    // box is opaque (triangles and crossed paths have holes).
                    if let [a, b, c, d] = path.as_slice() {
                        if a.y == b.y
                            && b.x == c.x
                            && c.y == d.y
                            && d.x == a.x
                            && b.x > a.x
                            && c.y > b.y
                            && [a.x, a.y, b.x, c.y].iter().all(|value| value.is_finite())
                        {
                            add_backing(
                                &mut opaque,
                                &clips,
                                Rect::from_xywh(a.x, a.y, b.x - a.x, c.y - a.y),
                            );
                        }
                    }
                }
                PaintCommand::BackdropFilterRoundedRect {
                    rect, blur_radius, ..
                } => {
                    let halo = blur_radius.max(0.) * 1.25 + 2.;
                    if !opaque.iter().any(|b| {
                        b.left() <= rect.left() - halo
                            && b.top() <= rect.top() - halo
                            && b.right() >= rect.right() + halo
                            && b.bottom() >= rect.bottom() + halo
                    }) {
                        return false;
                    }
                }
                PaintCommand::Extension { .. }
                | PaintCommand::SetOpacity { .. }
                | PaintCommand::DrawDisplayList { .. } => return false,
                _ => {}
            }
        }
        true
    }

    /// Replay unchanged paint content without cloning strings, paths, or pixels.
    pub fn draw_display_list(&mut self, origin: Point, list: Arc<DisplayList>) {
        self.commands
            .push(PaintCommand::DrawDisplayList { origin, list });
    }

    /// Expand retained recordings for a backend that consumes flat commands.
    pub fn flattened(&self) -> PaintContext<'_> {
        let mut result = PaintContext::new();
        self.append_flattened(&mut result, Point::ZERO);
        result
    }

    /// Prepare a GPU tile without changing the window-space raster snapping of
    /// text, icons or images. The tile offset is an integral logical translation.
    #[doc(hidden)]
    pub fn raster_tile_context(
        &self,
        scene_origin: Point,
        offset: Point,
        scale_milli: u32,
    ) -> PaintContext<'_> {
        self.raster_tile_context_impl(scene_origin, offset, scale_milli, false)
    }

    /// Prepare a tile while preserving independently cached child pictures.
    /// Backends must retain window-space raster snapping when descending into them.
    #[doc(hidden)]
    pub fn raster_tile_composite_context(
        &self,
        scene_origin: Point,
        offset: Point,
        scale_milli: u32,
    ) -> PaintContext<'_> {
        self.raster_tile_context_impl(scene_origin, offset, scale_milli, true)
    }

    fn raster_tile_context_impl(
        &self,
        scene_origin: Point,
        offset: Point,
        scale_milli: u32,
        preserve_cached: bool,
    ) -> PaintContext<'_> {
        let mut result = PaintContext::new();
        self.append_commands_at(&mut result, Point::ZERO, true, preserve_cached);
        let scale = scale_milli.max(1) as f32 / 1000.;
        let base = Point::new(libm::floorf(scene_origin.x), libm::floorf(scene_origin.y));
        let phase = Point::new(scene_origin.x - base.x, scene_origin.y - base.y);
        let mut text_positions = Vec::new();
        let pixel_snap =
            |value: f32, origin: f32| ((value + origin) * scale) as i32 as f32 / scale - origin;
        for (index, command) in result.commands.iter_mut().enumerate() {
            match command {
                PaintCommand::DrawText { position, .. } => {
                    text_positions.push((
                        index,
                        Point::new(
                            (position.x + scene_origin.x) as i32 as f32 - base.x + offset.x,
                            (position.y + scene_origin.y) as i32 as f32 - base.y + offset.y,
                        ),
                    ));
                }
                PaintCommand::DrawBuffer { dst: rect, .. }
                | PaintCommand::DrawBufferRect { dst: rect, .. }
                | PaintCommand::DrawIcon { rect, .. } => {
                    rect.origin.x = pixel_snap(rect.origin.x, scene_origin.x);
                    rect.origin.y = pixel_snap(rect.origin.y, scene_origin.y);
                }
                _ => {}
            }
        }
        // Bake the fractional scene phase into the tile's geometry, then
        // composite at an integer logical origin. Text keeps its exact global
        // logical-pixel snap; the cached image is never fractionally resampled.
        result.translate(Point::new(offset.x + phase.x, offset.y + phase.y));
        for (index, snapped) in text_positions {
            if let PaintCommand::DrawText { position, .. } = &mut result.commands[index] {
                *position = snapped;
            }
        }
        result
    }

    fn append_flattened<'b>(&'b self, result: &mut PaintContext<'b>, offset: Point) {
        self.append_commands_at(result, offset, true, false);
    }

    fn append_commands_at<'b>(
        &'b self,
        result: &mut PaintContext<'b>,
        offset: Point,
        flatten: bool,
        preserve_cached: bool,
    ) {
        for command in &self.commands {
            if let PaintCommand::DrawDisplayList { origin, list } = command {
                let origin = Point::new(offset.x + origin.x, offset.y + origin.y);
                if flatten && !(preserve_cached && list.raster_cache_bounds().is_some()) {
                    list.context
                        .append_commands_at(result, origin, true, preserve_cached);
                } else {
                    result.draw_display_list(origin, Arc::clone(list));
                }
                continue;
            }
            let mut translated = PaintContext::new();
            translated.commands.push(command.clone());
            translated.translate(offset);
            if let (
                PaintCommand::DrawText {
                    position: original, ..
                },
                Some(PaintCommand::DrawText { position, .. }),
            ) = (command, translated.commands.last_mut())
            {
                *position = Point::new(original.x + offset.x, original.y + offset.y);
            }
            let mut command = translated.commands.pop().unwrap();
            if let PaintCommand::DrawBuffer { buffer_idx, .. }
            | PaintCommand::DrawBufferRect { buffer_idx, .. } = &mut command
            {
                if let Some(buffer) = self.buffers.get(*buffer_idx) {
                    *buffer_idx = result.buffers.len();
                    result.buffers.push(match buffer {
                        PaintBuffer::Shared(buffer) => PaintBuffer::Shared(Arc::clone(buffer)),
                        other => PaintBuffer::Borrowed(other.as_buffer()),
                    });
                } else {
                    continue;
                }
            }
            result.commands.push(command);
        }
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
        self.commands.push(PaintCommand::FillRoundedRect {
            rect,
            corner_radius,
            color,
        });
    }

    /// Fill a rounded rectangle with a top-to-bottom linear gradient.
    pub fn fill_vertical_gradient_rounded_rect(
        &mut self,
        rect: Rect,
        corner_radius: f32,
        top_color: Color,
        bottom_color: Color,
    ) {
        self.commands
            .push(PaintCommand::FillVerticalGradientRoundedRect {
                rect,
                corner_radius,
                top_color,
                bottom_color,
            });
    }

    /// Blur the preceding scene and tint it, clipped to a rounded rectangle.
    /// `blur_radius` is the kernel support radius in logical pixels
    /// (the approximate Gaussian standard deviation is one third of it). Zero
    /// disables the blur without disabling the tint. SetOpacity controls the
    /// strength of the entire material, not just its tint. If a nonzero blur
    /// exceeds CPU resource limits, the provided tint RGB is rendered opaque
    /// (still respecting SetOpacity and clip coverage) for readable fallback.
    /// This fallback is a renderer quality decision, not a different material.
    ///
    /// Custom `ElementRenderObject` implementations emitting this command must
    /// return `true` from `reads_backdrop()` while the material is present,
    /// including commands inside nested display lists or popups. This lets the
    /// rendering pipeline bypass isolated picture caches and replay the scene
    /// whenever its backdrop changes.
    pub fn backdrop_filter_rounded_rect(
        &mut self,
        rect: Rect,
        corner_radius: f32,
        blur_radius: f32,
        tint: Color,
    ) {
        self.commands.push(PaintCommand::BackdropFilterRoundedRect {
            rect,
            corner_radius,
            blur_radius,
            tint,
            refraction: 0.0,
        });
    }

    /// Bounded convex refraction of the preceding scene. Like backdrop blur,
    /// requires `reads_backdrop()`. Strength is clamped to 0..1.
    pub fn refractive_glass_rounded_rect(
        &mut self,
        rect: Rect,
        corner_radius: f32,
        blur_radius: f32,
        tint: Color,
        strength: f32,
    ) {
        self.commands.push(PaintCommand::BackdropFilterRoundedRect {
            rect,
            corner_radius,
            blur_radius,
            tint,
            refraction: if strength.is_finite() {
                strength.clamp(0.0, 1.0)
            } else {
                0.0
            },
        });
    }

    /// Draw a soft rounded-rectangle drop shadow.
    pub fn draw_rounded_rect_shadow(
        &mut self,
        rect: Rect,
        corner_radius: f32,
        offset: Offset,
        blur_radius: f32,
        spread_radius: f32,
        color: Color,
    ) {
        self.commands.push(PaintCommand::DrawRoundedRectShadow {
            rect,
            corner_radius,
            offset,
            blur_radius,
            spread_radius,
            color,
        });
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

    /// Draw a typed icon into a logical destination rectangle.
    ///
    /// Rasterization is deferred to the active backend so physical DPI is part
    /// of the alpha-mask cache key. Color is applied after rasterization.
    ///
    /// # Arguments
    ///
    /// * `rect` - Logical square occupied by the icon.
    /// * `icon` - Selected icon.
    /// * `style` - Outline style used for mask generation.
    /// * `color` - Theme or explicit tint color.
    ///
    /// # Returns
    ///
    /// Nothing. The icon command is appended to this paint context.
    pub fn draw_icon(&mut self, rect: Rect, icon: Icon, style: IconStyle, color: Color) {
        self.commands.push(PaintCommand::DrawIcon {
            rect,
            icon,
            style,
            color,
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

    /// Insert renderer-specific content into the paint list.
    ///
    /// # Arguments
    ///
    /// * `rect` - Logical destination rectangle occupied by the extension.
    /// * `payload` - Backend payload shared with the synchronous render pass.
    pub fn draw_extension(&mut self, rect: Rect, payload: Arc<dyn PaintExtension>) {
        self.commands.push(PaintCommand::Extension {
            rect,
            payload,
            dependencies: PaintDependencies::UNKNOWN,
        });
    }

    /// Declare a renderer extension's conservative output and preceding-pixel
    /// reads in logical coordinates. Undeclared extensions use full invalidation.
    pub fn draw_extension_with_dependencies(
        &mut self,
        rect: Rect,
        payload: Arc<dyn PaintExtension>,
        dependencies: PaintDependencies,
    ) {
        self.commands.push(PaintCommand::Extension {
            rect,
            payload,
            dependencies,
        });
    }

    /// Bound CPU raster work by the painted geometry, not a virtual list's
    /// full layout extent. Clip commands do not paint any pixels themselves.
    pub(crate) fn raster_bounds(&self, size: Size, scale_milli: u32) -> Rect {
        if self
            .commands
            .iter()
            .any(|command| matches!(command, PaintCommand::DrawDisplayList { .. }))
        {
            return self.flattened().raster_bounds(size, scale_milli);
        }
        let mut bounds: Option<Rect> = None;
        let scale = scale_milli.max(1) as f32 / 1000.0;
        let mut include = |rect: Rect| {
            if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
                return;
            }
            bounds = Some(match bounds {
                None => rect,
                Some(old) => {
                    let x = old.origin.x.min(rect.origin.x);
                    let y = old.origin.y.min(rect.origin.y);
                    Rect::from_xywh(
                        x,
                        y,
                        (old.origin.x + old.size.width).max(rect.origin.x + rect.size.width) - x,
                        (old.origin.y + old.size.height).max(rect.origin.y + rect.size.height) - y,
                    )
                }
            });
        };
        for command in &self.commands {
            match command {
                PaintCommand::FillPath { path, .. } | PaintCommand::StrokePath { path, .. } => {
                    if let Some(first) = path.first() {
                        let mut left = first.x;
                        let mut top = first.y;
                        let mut right = first.x;
                        let mut bottom = first.y;
                        for point in path {
                            left = left.min(point.x);
                            top = top.min(point.y);
                            right = right.max(point.x);
                            bottom = bottom.max(point.y);
                        }
                        let pad = match command {
                            PaintCommand::StrokePath { stroke_width, .. } => stroke_width.max(1.0),
                            _ => 1.0 / scale,
                        };
                        include(Rect::from_xywh(
                            left - pad,
                            top - pad,
                            right - left + 2.0 * pad,
                            bottom - top + 2.0 * pad,
                        ));
                    }
                }
                PaintCommand::FillRoundedRect { rect, .. }
                | PaintCommand::FillVerticalGradientRoundedRect { rect, .. }
                | PaintCommand::BackdropFilterRoundedRect { rect, .. }
                | PaintCommand::StrokeRect { rect, .. }
                | PaintCommand::StrokeRoundedRect { rect, .. }
                | PaintCommand::DrawIcon { rect, .. } => include(*rect),
                PaintCommand::DrawBuffer { dst, .. } | PaintCommand::DrawBufferRect { dst, .. } => {
                    include(*dst)
                }
                PaintCommand::DrawText {
                    position,
                    text,
                    font_size_px,
                    ..
                } => {
                    // Use the same glyph masks and integer origins as Canvas;
                    // font advance alone misses italic and fallback overhangs.
                    let x = libm::floorf(position.x as i32 as f32 * scale);
                    let y = libm::floorf(position.y as i32 as f32 * scale);
                    for glyph in crate::graphics::rasterize_text(text, *font_size_px, scale_milli) {
                        include(Rect::from_xywh(
                            (x + glyph.x as f32) / scale,
                            (y + glyph.y as f32) / scale,
                            glyph.width as f32 / scale,
                            glyph.height as f32 / scale,
                        ));
                    }
                }
                PaintCommand::DrawRoundedRectShadow {
                    rect,
                    offset,
                    blur_radius,
                    spread_radius,
                    ..
                } => {
                    let expansion = *spread_radius + blur_radius.max(0.0);
                    include(Rect::from_xywh(
                        rect.left() + offset.dx - expansion,
                        rect.top() + offset.dy - expansion,
                        rect.size.width + expansion * 2.0,
                        rect.size.height + expansion * 2.0,
                    ));
                }
                // Unrecognized extensions remain backend-specific.
                _ => {}
            }
        }
        let Some(bounds) = bounds else {
            return Rect::new(Point::ZERO, Size::ZERO);
        };
        let left = bounds.origin.x.max(0.0);
        let top = bounds.origin.y.max(0.0);
        let right = (bounds.origin.x + bounds.size.width).min(size.width);
        let bottom = (bounds.origin.y + bounds.size.height).min(size.height);
        Rect::from_xywh(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }

    pub(crate) fn translate(&mut self, offset: Point) {
        let shift = |point: &mut Point| {
            point.x += offset.x;
            point.y += offset.y;
        };
        for command in &mut self.commands {
            match command {
                PaintCommand::FillPath { path, .. } | PaintCommand::StrokePath { path, .. } => {
                    for point in path {
                        shift(point);
                    }
                }
                PaintCommand::DrawText { position, .. } => {
                    position.x = position.x as i32 as f32 + offset.x;
                    position.y = position.y as i32 as f32 + offset.y;
                }
                PaintCommand::DrawDisplayList { origin, .. } => shift(origin),
                PaintCommand::DrawBuffer { dst, .. } | PaintCommand::DrawBufferRect { dst, .. } => {
                    shift(&mut dst.origin)
                }
                PaintCommand::FillRoundedRect { rect, .. }
                | PaintCommand::FillVerticalGradientRoundedRect { rect, .. }
                | PaintCommand::BackdropFilterRoundedRect { rect, .. }
                | PaintCommand::DrawRoundedRectShadow { rect, .. }
                | PaintCommand::StrokeRect { rect, .. }
                | PaintCommand::StrokeRoundedRect { rect, .. }
                | PaintCommand::DrawIcon { rect, .. }
                | PaintCommand::PushClip { rect, .. } => shift(&mut rect.origin),
                PaintCommand::Extension {
                    rect, dependencies, ..
                } => {
                    shift(&mut rect.origin);
                    *dependencies = dependencies.translated(offset);
                }
                _ => {}
            }
        }
    }

    /// Visit retained commands without cloning their paths, strings or buffers.
    /// Positions supplied here are the accumulated logical replay origin.
    pub fn visit_commands(&self, visit: &mut impl FnMut(&PaintCommand, Point)) {
        self.visit_commands_at(Point::ZERO, visit);
    }
    fn visit_commands_at(&self, offset: Point, visit: &mut impl FnMut(&PaintCommand, Point)) {
        for command in &self.commands {
            if let PaintCommand::DrawDisplayList { origin, list } = command {
                list.context
                    .visit_commands_at(Point::new(offset.x + origin.x, offset.y + origin.y), visit);
            } else {
                visit(command, offset);
            }
        }
    }

    /// Extract one command and its borrowed resource for backend lowering.
    pub fn command_context(&self, index: usize) -> PaintContext<'_> {
        let mut context = PaintContext::new();
        if let Some(command) = self.commands.get(index) {
            let mut command = command.clone();
            if let PaintCommand::DrawBuffer { buffer_idx, .. }
            | PaintCommand::DrawBufferRect { buffer_idx, .. } = &mut command
            {
                let Some(buffer) = self.buffer(BufferHandle(*buffer_idx)) else {
                    return context;
                };
                *buffer_idx = 0;
                context.buffers.push(PaintBuffer::Borrowed(buffer));
            }
            context.commands.push(command);
        }
        context
    }
    /// Append a context, remapping buffer handles and expanding nested lists.
    pub fn append(&mut self, other: &'a PaintContext<'_>) {
        other.append_flattened(self, Point::ZERO);
    }

    /// Append commands at a logical offset, remapping buffers and retained lists.
    pub fn append_at(&mut self, other: &'a PaintContext<'_>, origin: Point) {
        other.append_flattened(self, origin);
    }

    /// Append at an offset while preserving immutable display-list identities
    /// and raster-cache hints. Ordinary commands and buffer handles are
    /// translated/remapped as with [`Self::append_at`].
    pub fn append_retained_at(&mut self, other: &'a PaintContext<'_>, origin: Point) {
        other.append_commands_at(self, origin, false, false);
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

/// Renderer selected for a platform window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererBackendKind {
    /// CPU rasterization into a platform pixel buffer.
    Cpu,
    /// Native SGFX rendering and presentation.
    Sgfx,
}

/// Compositor selected by the platform window server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompositorBackendKind {
    /// CPU window composition.
    Cpu,
    /// Native SGFX window composition.
    Sgfx,
    /// Platform compositor could not be determined.
    Unknown,
}

/// Frame produced by a paint backend.
pub enum BackendFrame<'a> {
    /// CPU pixel buffer that still needs platform presentation.
    Cpu {
        /// Rendered pixel buffer.
        buffer: &'a Buffer,
    },
    /// Frame already presented by an external backend.
    External,
}

/// Backend that renders a paint command list.
///
/// CPU backends return a pixel buffer for the platform to present. External
/// backends own the presentation lifecycle and return [`BackendFrame::External`]
/// only after the frame has been submitted successfully.
pub trait PaintBackend {
    /// Whether the backend can consume immutable display-list replay commands.
    fn supports_retained_display_lists(&self) -> bool {
        false
    }
    /// CPU backends benefit from retained raster pictures. GPU backends consume
    /// the original paint commands directly and must not pre-rasterize UI on
    /// the CPU before encoding their frame.
    fn uses_cpu_picture_cache(&self) -> bool {
        false
    }

    /// Resize the backend render target.
    ///
    /// # Arguments
    ///
    /// * `size` - New logical render-target size.
    /// * `scale_milli` - Physical output scale in milli-units.
    ///
    /// # Returns
    ///
    /// Nothing.
    fn resize(&mut self, size: Size, scale_milli: u32);

    /// Render one paint command list.
    ///
    /// # Arguments
    ///
    /// * `context` - Paint commands and their frame-local buffer resources.
    /// * `background_color` - Color used to clear damaged target regions.
    /// * `logical_damage` - Logical regions to redraw, or `None` for the full target.
    /// * `physical_damage` - Physical regions to present, or `None` for the full target.
    ///
    /// # Returns
    ///
    /// A CPU buffer to present or confirmation that an external frame has
    /// already been presented. A classified [`crate::error::Error::RenderFailure`]
    /// must not be reported as External: recoverable failures retain the previous
    /// display and certify retirement of the discarded frame's accepted work.
    /// RecoveryRequired forbids reuse of uncertain targets until backend recovery.
    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        background_color: Color,
        logical_damage: Option<&[Rect]>,
        physical_damage: Option<&[DamageRect]>,
    ) -> Result<BackendFrame<'a>>;
}

/// CPU implementation of the backend-neutral paint interface.
pub struct CpuPaintBackend {
    renderer: CpuPaintRenderer,
}

impl CpuPaintBackend {
    /// Create a CPU paint backend.
    ///
    /// # Arguments
    ///
    /// * `size` - Initial logical render-target size.
    /// * `scale_milli` - Physical output scale in milli-units.
    /// * `background_color` - Initial target clear color.
    ///
    /// # Returns
    ///
    /// A CPU paint backend ready to render frames.
    pub fn new(size: Size, scale_milli: u32, background_color: Color) -> Self {
        Self {
            renderer: CpuPaintRenderer::new(size, scale_milli, background_color),
        }
    }
}

impl PaintBackend for CpuPaintBackend {
    fn uses_cpu_picture_cache(&self) -> bool {
        true
    }

    fn resize(&mut self, size: Size, scale_milli: u32) {
        self.renderer.resize(size, scale_milli);
    }

    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        background_color: Color,
        logical_damage: Option<&[Rect]>,
        _physical_damage: Option<&[DamageRect]>,
    ) -> Result<BackendFrame<'a>> {
        self.renderer.set_background_color(background_color);
        self.renderer.execute_with_damage(context, logical_damage);
        Ok(BackendFrame::Cpu {
            buffer: self.renderer.buffer(),
        })
    }
}

/// Result of preparing a frame for platform presentation.
pub enum PresentedFrame<'a> {
    /// CPU-rendered frame that still needs platform presentation.
    Cpu {
        /// Pixel buffer containing the rendered frame.
        buffer: &'a Buffer,
        /// Physical regions to present, or `None` for the full buffer.
        damage: Option<&'a [DamageRect]>,
    },
    /// Frame already rendered and presented by an external backend.
    External,
    /// No pixels changed and no presentation was submitted.
    Idle,
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
    layer_pool: Vec<Buffer>,
    clip_stack: Vec<ClipRegion>,
    clip_entries: Vec<ClipEntry>,
    scratch: RasterScratch,
    shadow_cache: Vec<CpuShadowRaster>,
    backdrop: BackdropScratch,
}

// The CPU prototype bounds both memory and frame work independently of the
// caller's geometry. Two retained premultiplied surfaces use at most 32 MiB.
const MAX_BACKDROP_RADIUS: usize = 64;
const MAX_BACKDROP_PIXELS: usize = 1_000_000;
const MAX_BACKDROP_FRAME_PIXELS: usize = 8_000_000;

#[derive(Default)]
struct BackdropScratch {
    pixels: Vec<[f32; 4]>,
    work: Vec<[f32; 4]>,
    frame_pixels: usize,
    fallback_count: usize,
}

fn premultiply_pixel(pixel: u32) -> [f32; 4] {
    let p = pixel.to_le_bytes();
    let alpha = p[3] as f32 / 255.0;
    [
        p[0] as f32 * alpha,
        p[1] as f32 * alpha,
        p[2] as f32 * alpha,
        p[3] as f32,
    ]
}

fn unpremultiply_pixel(p: [f32; 4]) -> u32 {
    if p[3] < 0.000_01 {
        return 0;
    }
    let inv_alpha = 255.0 / p[3];
    u32::from_le_bytes([
        (p[0] * inv_alpha + 0.5).clamp(0.0, 255.0) as u8,
        (p[1] * inv_alpha + 0.5).clamp(0.0, 255.0) as u8,
        (p[2] * inv_alpha + 0.5).clamp(0.0, 255.0) as u8,
        (p[3] + 0.5).clamp(0.0, 255.0) as u8,
    ])
}

/// Sliding-window separable box pass: O(pixels), independent of radius after
/// a bounded initial sum. Clamped sampling extends the actual window edge.
fn backdrop_box_pass(
    src: &[[f32; 4]],
    dst: &mut [[f32; 4]],
    width: usize,
    height: usize,
    radius: usize,
    horizontal: bool,
) {
    let (lines, length, step) = if horizontal {
        (height, width, 1)
    } else {
        (width, height, width)
    };
    let divisor = (radius * 2 + 1) as f32;
    for line in 0..lines {
        let base = if horizontal { line * width } else { line };
        let mut sum = [0.0; 4];
        for tap in -(radius as isize)..=radius as isize {
            let position = tap.clamp(0, length as isize - 1) as usize;
            for c in 0..4 {
                sum[c] += src[base + position * step][c];
            }
        }
        for position in 0..length {
            for c in 0..4 {
                dst[base + position * step][c] = sum[c] / divisor;
            }
            let remove = position.saturating_sub(radius);
            let add = (position + radius + 1).min(length - 1);
            for c in 0..4 {
                sum[c] += src[base + add * step][c] - src[base + remove * step][c];
            }
        }
    }
}

fn union_rect(previous: Option<Rect>, rect: Rect) -> Rect {
    let Some(old) = previous else {
        return rect;
    };
    let left = old.left().min(rect.left());
    let top = old.top().min(rect.top());
    Rect::from_xywh(
        left,
        top,
        old.right().max(rect.right()) - left,
        old.bottom().max(rect.bottom()) - top,
    )
}

// Conservative logical bounds for pending clip layers. Only used on scenes
// containing a backdrop, and avoids clearing/scanning a whole window for each
// small control. Text uses the real cached glyph masks, including overhangs.
fn command_paint_bounds(
    command: &PaintCommand,
    context: &PaintContext<'_>,
    scale_milli: u32,
) -> Option<Rect> {
    let scale = scale_milli.max(1) as f32 / 1000.0;
    let rect = match command {
        PaintCommand::FillRoundedRect { rect, .. }
        | PaintCommand::FillVerticalGradientRoundedRect { rect, .. }
        | PaintCommand::StrokeRect { rect, .. }
        | PaintCommand::StrokeRoundedRect { rect, .. } => *rect,
        PaintCommand::DrawIcon {
            rect, icon, style, ..
        } => {
            let size = libm::ceilf(
                (rect.size.width * scale)
                    .min(rect.size.height * scale)
                    .max(1.0),
            )
            .min(u16::MAX as f32) as u16;
            let raster = crate::icon::rasterize_icon(*icon, size, *style);
            Rect::from_xywh(
                (rect.left() * scale) as i32 as f32 / scale,
                (rect.top() * scale) as i32 as f32 / scale,
                raster.width as f32 / scale,
                raster.height as f32 / scale,
            )
        }
        PaintCommand::DrawBuffer { dst, buffer_idx } => {
            // This legacy command does not scale to dst.size: it composites
            // the source's physical pixels at the snapped destination origin.
            let source = context.buffer(BufferHandle(*buffer_idx))?;
            Rect::from_xywh(
                (dst.left() * scale) as i32 as f32 / scale,
                (dst.top() * scale) as i32 as f32 / scale,
                source.width() as f32 / scale,
                source.height() as f32 / scale,
            )
        }
        PaintCommand::DrawBufferRect { dst, .. } => *dst,
        PaintCommand::DrawRoundedRectShadow {
            rect,
            offset,
            blur_radius,
            spread_radius,
            ..
        } => {
            let expansion = spread_radius + blur_radius.max(0.0);
            Rect::from_xywh(
                rect.left() + offset.dx - expansion,
                rect.top() + offset.dy - expansion,
                rect.size.width + expansion * 2.0,
                rect.size.height + expansion * 2.0,
            )
        }
        PaintCommand::FillPath { path, .. } | PaintCommand::StrokePath { path, .. } => {
            let first = path.first()?;
            let (mut left, mut top, mut right, mut bottom) = (first.x, first.y, first.x, first.y);
            for p in path {
                left = left.min(p.x);
                top = top.min(p.y);
                right = right.max(p.x);
                bottom = bottom.max(p.y);
            }
            let pad = match command {
                PaintCommand::StrokePath { stroke_width, .. } => stroke_width.max(1.0),
                _ => 1.0 / scale,
            };
            Rect::from_xywh(
                left - pad,
                top - pad,
                right - left + pad * 2.0,
                bottom - top + pad * 2.0,
            )
        }
        PaintCommand::DrawText {
            position,
            text,
            font_size_px,
            ..
        } => {
            let x = libm::floorf(position.x as i32 as f32 * scale);
            let y = libm::floorf(position.y as i32 as f32 * scale);
            let mut bounds = None;
            for glyph in
                crate::graphics::rasterize_text_cached(text, *font_size_px, scale_milli).iter()
            {
                bounds = Some(union_rect(
                    bounds,
                    Rect::from_xywh(
                        (x + glyph.x as f32) / scale,
                        (y + glyph.y as f32) / scale,
                        glyph.width as f32 / scale,
                        glyph.height as f32 / scale,
                    ),
                ));
            }
            bounds?
        }
        _ => return None,
    };
    if [rect.left(), rect.top(), rect.right(), rect.bottom()]
        .iter()
        .all(|v| v.is_finite())
        && rect.size.width > 0.0
        && rect.size.height > 0.0
    {
        Some(rect)
    } else {
        None
    }
}

// Bounded, geometry/color-keyed shadow images. Position is excluded except for
// subpixel phase, so repeated identical controls share one raster at each scale.
struct CpuShadowRaster {
    key: [u32; 8],
    buffer: Buffer,
}
const MAX_SHADOW_CACHE_PIXELS: usize = 8_000_000;
const MAX_SHADOW_CACHE_ENTRIES: usize = 16;
const CPU_SHADOW_WEIGHTS: [f32; 8] = [0.02, 0.03, 0.05, 0.08, 0.12, 0.17, 0.23, 0.30];

pub(crate) struct RasterScratch {
    scaled_points: Vec<Point>,
    crossings: Vec<f32>,
    coverage: Vec<f32>,
    clip_rects: Vec<Rect>,
    path_points: Vec<Point>,
}

impl RasterScratch {
    fn new() -> Self {
        Self {
            scaled_points: Vec::new(),
            crossings: Vec::new(),
            coverage: Vec::new(),
            clip_rects: Vec::new(),
            path_points: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.scaled_points.clear();
        self.crossings.clear();
        self.coverage.clear();
        self.clip_rects.clear();
        self.path_points.clear();
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RasterScratchCapacity {
    pub(crate) scaled_points: usize,
    pub(crate) crossings: usize,
    pub(crate) coverage: usize,
    pub(crate) clip_rects: usize,
    pub(crate) path_points: usize,
}

struct PaintLayer {
    buffer: Buffer,
    clip: ClipRegion,
    // Logical bounds of pending pixels, used by backdrop compositing barriers.
    pending_bounds: Option<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipEntry {
    Rect,
    Layer,
}

enum EffectiveClipRects {
    Unclipped,
    Empty,
    Rects,
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
            layer_pool: Vec::new(),
            clip_stack: Vec::new(),
            clip_entries: Vec::new(),
            scratch: RasterScratch::new(),
            shadow_cache: Vec::new(),
            backdrop: BackdropScratch::default(),
        }
    }

    #[deprecated(
        since = "0.1.0",
        note = "use execute_into_external_buffer on a retained CpuPaintRenderer to reuse raster scratch"
    )]
    pub fn execute_into_buffer(
        buffer: &mut Buffer,
        background_color: Color,
        ctx: &PaintContext<'_>,
        damage_rects: Option<&[Rect]>,
    ) {
        let scale_milli = buffer.scale_milli();
        let mut renderer = Self {
            buffer: core::mem::replace(buffer, Buffer::empty()),
            background_color,
            scale_milli,
            layer_stack: Vec::new(),
            layer_pool: Vec::new(),
            clip_stack: Vec::new(),
            clip_entries: Vec::new(),
            scratch: RasterScratch::new(),
            shadow_cache: Vec::new(),
            backdrop: BackdropScratch::default(),
        };
        renderer.execute_with_damage(ctx, damage_rects);
        *buffer = renderer.into_buffer();
    }

    pub fn execute_into_external_buffer(
        &mut self,
        buffer: &mut Buffer,
        background_color: Color,
        ctx: &PaintContext<'_>,
        damage_rects: Option<&[Rect]>,
    ) {
        let window_buffer = core::mem::replace(
            &mut self.buffer,
            core::mem::replace(buffer, Buffer::empty()),
        );
        let window_background = self.background_color;
        let window_scale = self.scale_milli;
        self.background_color = background_color;
        self.scale_milli = self.buffer.scale_milli();
        self.execute_with_damage(ctx, damage_rects);
        *buffer = core::mem::replace(&mut self.buffer, window_buffer);
        self.background_color = window_background;
        self.scale_milli = window_scale;
    }

    pub fn resize(&mut self, size: Size, scale_milli: u32) {
        self.layer_stack.clear();
        self.layer_pool.clear();
        self.clip_stack.clear();
        self.clip_entries.clear();
        self.scratch.clear();
        self.shadow_cache.clear();
        self.backdrop = BackdropScratch::default();
        self.buffer = Buffer::from_logical_dimensions_with_scale(
            size.width as u32,
            size.height as u32,
            scale_milli,
        );
        self.scale_milli = scale_milli;
    }

    pub fn reserve_scratch_for_path(&mut self, points: usize) {
        self.scratch.scaled_points.reserve(points);
        self.scratch.path_points.reserve(points.max(4));
        self.scratch.crossings.reserve(points);
    }

    #[cfg(test)]
    pub(crate) fn scratch_capacity_for_test(&self) -> RasterScratchCapacity {
        RasterScratchCapacity {
            scaled_points: self.scratch.scaled_points.capacity(),
            crossings: self.scratch.crossings.capacity(),
            coverage: self.scratch.coverage.capacity(),
            clip_rects: self.scratch.clip_rects.capacity(),
            path_points: self.scratch.path_points.capacity(),
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_boundary_scratch_capacity_for_test(&self) -> RasterScratchCapacity {
        self.scratch_capacity_for_test()
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

    fn acquire_layer_buffer(&mut self) -> Buffer {
        let logical_width = self.buffer.logical_width();
        let logical_height = self.buffer.logical_height();
        let scale_milli = self.scale_milli;
        if let Some(index) = self.layer_pool.iter().rposition(|buffer| {
            buffer.logical_width() == logical_width
                && buffer.logical_height() == logical_height
                && buffer.scale_milli() == scale_milli
        }) {
            let mut buffer = self.layer_pool.swap_remove(index);
            buffer.clear(Color::TRANSPARENT);
            return buffer;
        }

        Buffer::from_logical_dimensions_with_scale(logical_width, logical_height, scale_milli)
    }

    fn push_clip_layer(&mut self, clip: ClipRegion) {
        let buffer = self.acquire_layer_buffer();
        let layer = PaintLayer {
            buffer,
            clip,
            pending_bounds: None,
        };
        self.layer_stack.push(layer);
    }

    fn pop_clip_layer(&mut self, damage_rects: Option<&[Rect]>) {
        let Some(layer) = self.layer_stack.pop() else {
            return;
        };
        if let (Some(parent), Some(bounds)) = (self.layer_stack.last_mut(), layer.pending_bounds) {
            parent.pending_bounds = Some(union_rect(parent.pending_bounds, bounds));
        }
        let clip = self.scale_clip_region(layer.clip);
        let effective_clips = self.rebuild_effective_clip_rects(damage_rects);

        match effective_clips {
            EffectiveClipRects::Unclipped => {
                composite_layer_clipped(self.current_buffer_mut(), &layer.buffer, clip, None);
            }
            EffectiveClipRects::Empty => {}
            EffectiveClipRects::Rects => {
                for index in 0..self.scratch.clip_rects.len() {
                    let rect = self.scratch.clip_rects[index];
                    let scaled_rect = self.scale_rect(rect);
                    composite_layer_clipped(
                        self.current_buffer_mut(),
                        &layer.buffer,
                        clip,
                        Some(scaled_rect),
                    );
                }
            }
        }

        self.layer_pool.push(layer.buffer);
    }

    fn push_clip(&mut self, clip: ClipRegion) {
        self.clip_stack.push(clip);
        if clip.corner_radius <= 0.0 {
            self.clip_entries.push(ClipEntry::Rect);
        } else {
            self.push_clip_layer(clip);
            self.clip_entries.push(ClipEntry::Layer);
        }
    }

    fn pop_clip(&mut self, damage_rects: Option<&[Rect]>) {
        let Some(entry) = self.clip_entries.pop() else {
            return;
        };
        self.clip_stack.pop();
        if entry == ClipEntry::Layer {
            self.pop_clip_layer(damage_rects);
        }
    }

    fn active_clip_rect(&self) -> Option<Rect> {
        let mut active: Option<Rect> = None;
        for clip in &self.clip_stack {
            active = match active {
                Some(current) => intersect_rect(current, clip.rect),
                None => Some(clip.rect),
            };
            active?;
        }
        active
    }

    fn rebuild_effective_clip_rects(
        &mut self,
        damage_rects: Option<&[Rect]>,
    ) -> EffectiveClipRects {
        self.scratch.clip_rects.clear();
        let active_clip = self.active_clip_rect();
        if !self.clip_stack.is_empty() && active_clip.is_none() {
            return EffectiveClipRects::Empty;
        }

        match (active_clip, damage_rects) {
            (Some(active), Some(damage)) => {
                for rect in damage {
                    if let Some(rect) = intersect_rect(active, *rect) {
                        self.scratch.clip_rects.push(rect);
                    }
                }
                if self.scratch.clip_rects.is_empty() {
                    EffectiveClipRects::Empty
                } else {
                    EffectiveClipRects::Rects
                }
            }
            (Some(active), None) => {
                self.scratch.clip_rects.push(active);
                EffectiveClipRects::Rects
            }
            (None, Some(damage)) if damage.is_empty() => EffectiveClipRects::Empty,
            (None, Some(damage)) => {
                self.scratch.clip_rects.extend_from_slice(damage);
                EffectiveClipRects::Rects
            }
            (None, None) => EffectiveClipRects::Unclipped,
        }
    }

    fn rebuild_scaled_points(&mut self, path: &[Point]) {
        self.scratch.scaled_points.clear();
        let scale_milli = self.scale_milli as f32;
        self.scratch.scaled_points.extend(
            path.iter()
                .map(|p| Point::new(p.x * scale_milli / 1000.0, p.y * scale_milli / 1000.0)),
        );
    }

    fn paint_rounded_rect(
        &mut self,
        rect: Rect,
        radius: f32,
        stroke_width: Option<f32>,
        color: Color,
        clip: Option<ClipRegion>,
    ) {
        self.paint_gradient_rect(rect, radius, stroke_width, color, None, clip);
    }

    fn paint_gradient_rect(
        &mut self,
        rect: Rect,
        radius: f32,
        stroke_width: Option<f32>,
        color: Color,
        bottom_color: Option<Color>,
        clip: Option<ClipRegion>,
    ) {
        let scratch = &mut self.scratch;
        let buffer = if let Some(layer) = self.layer_stack.last_mut() {
            &mut layer.buffer
        } else {
            &mut self.buffer
        };
        raster_rounded_rect(
            buffer,
            rect,
            radius,
            stroke_width,
            color,
            bottom_color,
            clip,
            &mut scratch.coverage,
        );
    }

    /// A backdrop establishes a compositing barrier: resolve preceding layers
    /// before sampling, without copying a resolved backdrop into a transparent
    /// layer (which would apply its alpha twice when popped). Subsequent paints
    /// continue in the still-active, now empty layers above the material.
    fn flush_backdrop_layers(&mut self) {
        for index in (0..self.layer_stack.len()).rev() {
            let (parents, children) = self.layer_stack.split_at_mut(index);
            let layer = &mut children[0];
            let Some(bounds) = layer.pending_bounds.take() else {
                continue;
            };
            let scale = self.scale_milli as f32 / 1000.0;
            let scaled = |rect: Rect| {
                Rect::from_xywh(
                    rect.left() * scale,
                    rect.top() * scale,
                    rect.size.width * scale,
                    rect.size.height * scale,
                )
            };
            let clip = ClipRegion {
                rect: scaled(layer.clip.rect),
                corner_radius: layer.clip.corner_radius * scale,
            };
            let target = if let Some(parent) = parents.last_mut() {
                parent.pending_bounds = Some(union_rect(parent.pending_bounds, bounds));
                &mut parent.buffer
            } else {
                &mut self.buffer
            };
            composite_layer_clipped(target, &layer.buffer, clip, Some(scaled(bounds)));
            let (left, top, right, bottom) = raster_bounds(&layer.buffer, scaled(bounds), None);
            if right > left && bottom > top {
                layer.buffer.clear_rect(
                    left as u32,
                    top as u32,
                    (right - left) as u32,
                    (bottom - top) as u32,
                    Color::TRANSPARENT,
                );
            }
        }
    }

    fn paint_backdrop(
        &mut self,
        rect: Rect,
        radius: f32,
        blur_radius: f32,
        tint: Color,
        opacity: f32,
        refraction: f32,
    ) {
        if ![
            rect.left(),
            rect.top(),
            rect.right(),
            rect.bottom(),
            radius,
            blur_radius,
            refraction,
            tint.r,
            tint.g,
            tint.b,
            tint.a,
            opacity,
        ]
        .iter()
        .all(|v| v.is_finite())
            || rect.size.width <= 0.0
            || rect.size.height <= 0.0
            || opacity <= 0.0
        {
            return;
        }
        let rect = self.scale_rect(rect);
        let radius = self
            .scale_f32(radius)
            .max(0.0)
            .min(rect.size.width * 0.5)
            .min(rect.size.height * 0.5);
        if ![rect.left(), rect.top(), rect.right(), rect.bottom(), radius]
            .iter()
            .all(|v| v.is_finite())
        {
            return;
        }
        let active = self.active_clip_rect();
        if !self.clip_stack.is_empty() && active.is_none() {
            return;
        }
        let active = active.map(|r| self.scale_rect(r));
        let (left, top, right, bottom) = raster_bounds(&self.buffer, rect, active);
        if right <= left || bottom <= top {
            return;
        }
        let support = libm::ceilf(
            self.scale_f32(blur_radius.max(0.0))
                .min(MAX_BACKDROP_RADIUS as f32),
        ) as usize;
        let tint_pixel = tint.to_bgra();
        if support == 0 && tint_pixel >> 24 == 0 {
            return;
        }
        self.flush_backdrop_layers();

        let sample_left = (left - support as i32).max(0);
        let sample_top = (top - support as i32).max(0);
        let sample_right = (right + support as i32).min(self.buffer.width() as i32);
        let sample_bottom = (bottom + support as i32).min(self.buffer.height() as i32);
        let width = (sample_right - sample_left) as usize;
        let height = (sample_bottom - sample_top) as usize;
        let area = width.checked_mul(height).unwrap_or(usize::MAX);
        let needs_blur = support > 0 && tint_pixel >> 24 < 255;
        let blur = needs_blur
            && area <= MAX_BACKDROP_PIXELS
            && area <= MAX_BACKDROP_FRAME_PIXELS.saturating_sub(self.backdrop.frame_pixels);
        // Quality fallback keeps the material's semantic RGB, but removes
        // busy sharp background detail when there is no budget to soften it.
        // An explicit radius of zero is still an ordinary translucent tint.
        let tint_pixel = if needs_blur && !blur {
            self.backdrop.fallback_count += 1;
            tint_pixel | 0xff00_0000
        } else {
            tint_pixel
        };
        let stride = self.buffer.width() as usize;
        if blur {
            self.backdrop.frame_pixels += area;
            // reserve_exact prevents geometric Vec growth exceeding the cap.
            if self.backdrop.pixels.capacity() < area {
                self.backdrop
                    .pixels
                    .reserve_exact(area - self.backdrop.pixels.len());
            }
            if self.backdrop.work.capacity() < area {
                self.backdrop
                    .work
                    .reserve_exact(area - self.backdrop.work.len());
            }
            self.backdrop.pixels.resize(area, [0.0; 4]);
            self.backdrop.work.resize(area, [0.0; 4]);
            for y in 0..height {
                let start = (sample_top as usize + y) * stride + sample_left as usize;
                for x in 0..width {
                    self.backdrop.pixels[y * width + x] =
                        premultiply_pixel(self.buffer.as_slice()[start + x]);
                }
            }
            // Three box kernels approximate a Gaussian, with exact total
            // support equal to the capped physical blur_radius.
            for pass in 0..3 {
                let r = (support + pass) / 3;
                if r == 0 {
                    continue;
                }
                backdrop_box_pass(
                    &self.backdrop.pixels,
                    &mut self.backdrop.work,
                    width,
                    height,
                    r,
                    true,
                );
                backdrop_box_pass(
                    &self.backdrop.work,
                    &mut self.backdrop.pixels,
                    width,
                    height,
                    r,
                    false,
                );
            }
        }
        let tint = premultiply_pixel(tint_pixel);
        let tint_alpha = tint[3] / 255.0;
        self.scratch
            .coverage
            .resize((right - left) as usize + 1, 0.0);
        let scale = self.scale_milli as f32 / 1000.0;
        let logical = Rect::from_xywh(
            rect.left() / scale,
            rect.top() / scale,
            rect.size.width / scale,
            rect.size.height / scale,
        );
        let bezel = crate::glass::glass_bezel(logical, radius / scale);
        for y in top..bottom {
            self.scratch.coverage.fill(0.0);
            for sample in 0..COVERAGE_SAMPLES {
                let yc = y as f32 + (sample as f32 + 0.5) * COVERAGE_WEIGHT;
                let Some((mut a, mut b)) = rounded_span(rect, radius, yc) else {
                    continue;
                };
                // Intersect spans before coverage integration, rather than
                // multiplying AA masks and darkening coincident edges.
                for clip in &self.clip_stack {
                    let scaled = ClipRegion {
                        rect: Rect::from_xywh(
                            clip.rect.left() * scale,
                            clip.rect.top() * scale,
                            clip.rect.size.width * scale,
                            clip.rect.size.height * scale,
                        ),
                        corner_radius: clip.corner_radius * scale,
                    };
                    if let Some((c, d)) = clip_span_at(scaled, yc) {
                        a = a.max(c);
                        b = b.min(d);
                    } else {
                        b = a;
                        break;
                    }
                }
                add_coverage_span(&mut self.scratch.coverage, left, a, b);
            }
            let mut coverage = 0.0;
            let pixels = self.buffer.as_mut_slice();
            for x in left..right {
                coverage += self.scratch.coverage[(x - left) as usize];
                let amount = coverage.clamp(0.0, 1.0) * opacity;
                if amount <= 0.000_01 {
                    continue;
                }
                let index = y as usize * stride + x as usize;
                let old = premultiply_pixel(pixels[index]);
                let filtered = if blur && refraction <= 0.0 {
                    self.backdrop.pixels
                        [(y - sample_top) as usize * width + (x - sample_left) as usize]
                } else if blur {
                    let p = crate::glass::glass_sample(
                        logical,
                        radius / scale,
                        Point::new((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale),
                        refraction,
                    );
                    let sx =
                        (p.x * scale - 0.5 - sample_left as f32).clamp(0.0, (width - 1) as f32);
                    let sy =
                        (p.y * scale - 0.5 - sample_top as f32).clamp(0.0, (height - 1) as f32);
                    let x0 = sx as usize;
                    let y0 = sy as usize;
                    let x1 = (x0 + 1).min(width - 1);
                    let y1 = (y0 + 1).min(height - 1);
                    let tx = sx - x0 as f32;
                    let ty = sy - y0 as f32;
                    let mut sample = [0.0; 4];
                    for c in 0..4 {
                        let a = self.backdrop.pixels[y0 * width + x0][c] * (1.0 - tx)
                            + self.backdrop.pixels[y0 * width + x1][c] * tx;
                        let b = self.backdrop.pixels[y1 * width + x0][c] * (1.0 - tx)
                            + self.backdrop.pixels[y1 * width + x1][c] * tx;
                        sample[c] = a * (1.0 - ty) + b * ty;
                    }
                    sample
                } else {
                    old
                };
                let (light, near, line) = if blur && refraction > 0.0 {
                    let (d, nx, ny) = crate::glass::glass_edge(
                        rect,
                        radius,
                        Point::new(x as f32 + 0.5, y as f32 + 0.5),
                    );
                    let (light, near, line) = crate::glass::glass_surface_layers_at_scale(
                        (d / scale).max(0.),
                        nx,
                        ny,
                        bezel,
                        scale,
                    );
                    let strength = refraction.clamp(0., 1.);
                    (light * strength, near * strength, line * strength)
                } else {
                    (0.0, 0.0, 0.0)
                };
                let mut output = [0.0; 4];
                for c in 0..4 {
                    let mut material = tint[c] + filtered[c] * (1.0 - tint_alpha);
                    let shadow_target = if c == 3 { 255. } else { 0. };
                    material += (shadow_target - material) * near;
                    material += (255. - material) * light;
                    material += (shadow_target - material) * line;
                    output[c] = old[c] + (material - old[c]) * amount;
                }
                pixels[index] = unpremultiply_pixel(output);
            }
        }
    }

    fn paint_soft_shadow(
        &mut self,
        rect: Rect,
        radius: f32,
        blur: f32,
        spread: f32,
        color: Color,
        damage_rects: Option<&[Rect]>,
    ) {
        if color.a <= 0.0 || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
            return;
        }
        let expansion = spread + blur;
        let bounds = [
            rect.left() - expansion,
            rect.top() - expansion,
            rect.right() + expansion,
            rect.bottom() + expansion,
        ];
        if ![rect.size.width, rect.size.height, radius, blur, spread]
            .iter()
            .chain(bounds.iter())
            .all(|v| v.is_finite())
            || bounds[2] <= 0.0
            || bounds[3] <= 0.0
            || bounds[0] >= self.buffer.width() as f32
            || bounds[1] >= self.buffer.height() as f32
        {
            return;
        }
        let x = libm::floorf(rect.left() - expansion) as i32;
        let y = libm::floorf(rect.top() - expansion) as i32;
        let width = (libm::ceilf(rect.right() + expansion) - x as f32).max(0.0) as u32;
        let height = (libm::ceilf(rect.bottom() + expansion) - y as f32).max(0.0) as u32;
        if width == 0 || height == 0 {
            return;
        }
        let pixels = u64::from(width) * u64::from(height);
        // Geometric partition clips are half-open at pixel centers, so adjacent
        // retained shadow strips never blend the same pixel twice. Damage alone
        // still rounds outward to match clearing; rounded masks apply later.
        let scale = self.scale_milli as f32 / 1000.0;
        let mut real_scissor = (i32::MIN, i32::MIN, i32::MAX, i32::MAX);
        for clip in self
            .clip_stack
            .iter()
            .filter(|clip| clip.corner_radius <= 0.0)
        {
            real_scissor.0 = real_scissor
                .0
                .max(libm::ceilf(clip.rect.left() * scale - 0.5) as i32);
            real_scissor.1 = real_scissor
                .1
                .max(libm::ceilf(clip.rect.top() * scale - 0.5) as i32);
            real_scissor.2 = real_scissor
                .2
                .min(libm::ceilf(clip.rect.right() * scale - 0.5) as i32);
            real_scissor.3 = real_scissor
                .3
                .min(libm::ceilf(clip.rect.bottom() * scale - 0.5) as i32);
        }
        let clips = self.rebuild_effective_clip_rects(damage_rects);
        if matches!(clips, EffectiveClipRects::Empty) {
            return;
        }
        if pixels > MAX_SHADOW_CACHE_PIXELS as u64 {
            // Keep pathological huge shadows bounded in memory; ordinary UI
            // shadows use the cached path below.
            for (index, weight) in CPU_SHADOW_WEIGHTS.iter().enumerate() {
                let expansion = spread + blur * (7 - index) as f32 / 7.0;
                let shape = Rect::from_xywh(
                    rect.left() - expansion,
                    rect.top() - expansion,
                    rect.size.width + 2.0 * expansion,
                    rect.size.height + 2.0 * expansion,
                );
                let tint = color.with_opacity(color.a * *weight);
                match clips {
                    EffectiveClipRects::Unclipped => self.paint_rounded_rect(
                        shape,
                        (radius + expansion).max(0.0),
                        None,
                        tint,
                        None,
                    ),
                    EffectiveClipRects::Rects => {
                        for i in 0..self.scratch.clip_rects.len() {
                            let bounds = self.scale_rect(self.scratch.clip_rects[i]);
                            let left = (libm::floorf(bounds.left()) as i32).max(real_scissor.0);
                            let top = (libm::floorf(bounds.top()) as i32).max(real_scissor.1);
                            let right = (libm::ceilf(bounds.right()) as i32).min(real_scissor.2);
                            let bottom = (libm::ceilf(bounds.bottom()) as i32).min(real_scissor.3);
                            let clip = ClipRegion {
                                rect: Rect::from_xywh(
                                    left as f32,
                                    top as f32,
                                    (right - left).max(0) as f32,
                                    (bottom - top).max(0) as f32,
                                ),
                                corner_radius: 0.0,
                            };
                            self.paint_rounded_rect(
                                shape,
                                (radius + expansion).max(0.0),
                                None,
                                tint,
                                Some(clip),
                            );
                        }
                    }
                    EffectiveClipRects::Empty => {}
                }
            }
            return;
        }
        let pixels = pixels as usize;
        let local = Rect::from_xywh(
            rect.left() - x as f32,
            rect.top() - y as f32,
            rect.size.width,
            rect.size.height,
        );
        let key = [
            local.left().to_bits(),
            local.top().to_bits(),
            local.size.width.to_bits(),
            local.size.height.to_bits(),
            radius.to_bits(),
            blur.to_bits(),
            spread.to_bits(),
            color.to_bgra(),
        ];
        let index = if let Some(index) = self.shadow_cache.iter().position(|entry| entry.key == key)
        {
            index
        } else {
            while !self.shadow_cache.is_empty()
                && (self.shadow_cache.len() >= MAX_SHADOW_CACHE_ENTRIES
                    || self
                        .shadow_cache
                        .iter()
                        .map(|entry| entry.buffer.as_slice().len())
                        .sum::<usize>()
                        + pixels
                        > MAX_SHADOW_CACHE_PIXELS)
            {
                self.shadow_cache.remove(0);
            }
            let mut buffer = Buffer::from_dimensions(width, height);
            for (index, weight) in CPU_SHADOW_WEIGHTS.iter().enumerate() {
                let expansion = spread + blur * (7 - index) as f32 / 7.0;
                let shape = Rect::from_xywh(
                    local.left() - expansion,
                    local.top() - expansion,
                    local.size.width + 2.0 * expansion,
                    local.size.height + 2.0 * expansion,
                );
                raster_rounded_rect(
                    &mut buffer,
                    shape,
                    (radius + expansion).max(0.0),
                    None,
                    color.with_opacity(color.a * *weight),
                    None,
                    None,
                    &mut self.scratch.coverage,
                );
            }
            self.shadow_cache.push(CpuShadowRaster { key, buffer });
            self.shadow_cache.len() - 1
        };
        let source = &self.shadow_cache[index].buffer;
        let target = if let Some(layer) = self.layer_stack.last_mut() {
            &mut layer.buffer
        } else {
            &mut self.buffer
        };
        match clips {
            EffectiveClipRects::Unclipped => target.composite(source, x, y, 1.0),
            EffectiveClipRects::Rects => {
                let scale = self.scale_milli as f32 / 1000.0;
                for clip in &self.scratch.clip_rects {
                    let left = (libm::floorf(clip.left() * scale) as i32).max(real_scissor.0);
                    let top = (libm::floorf(clip.top() * scale) as i32).max(real_scissor.1);
                    let right = (libm::ceilf(clip.right() * scale) as i32).min(real_scissor.2);
                    let bottom = (libm::ceilf(clip.bottom() * scale) as i32).min(real_scissor.3);
                    target.composite_clipped(
                        source,
                        x,
                        y,
                        1.0,
                        left,
                        top,
                        right - left,
                        bottom - top,
                    );
                }
            }
            EffectiveClipRects::Empty => {}
        }
    }

    fn fill_scaled_path(&mut self, color: Color, clip: Option<ClipRegion>) {
        let scratch = &mut self.scratch;
        let path = &scratch.scaled_points;
        let crossings = &mut scratch.crossings;
        if let Some(layer) = self.layer_stack.last_mut() {
            fill_polygon(
                &mut layer.buffer,
                path,
                color,
                clip,
                crossings,
                &mut scratch.coverage,
            );
        } else {
            fill_polygon(
                &mut self.buffer,
                path,
                color,
                clip,
                crossings,
                &mut scratch.coverage,
            );
        }
    }

    fn draw_scaled_path_stroke(&mut self, stroke_width: f32, color: Color, clip: Option<Rect>) {
        let point_count = self.scratch.scaled_points.len();
        for index in 0..point_count.saturating_sub(1) {
            let from = self.scratch.scaled_points[index];
            let to = self.scratch.scaled_points[index + 1];
            self.draw_thick_line(stroke_width, color, from, to, clip);
        }
        if point_count > 2 {
            let from = self.scratch.scaled_points[point_count - 1];
            let to = self.scratch.scaled_points[0];
            self.draw_thick_line(stroke_width, color, from, to, clip);
        }
    }

    fn draw_thick_line(
        &mut self,
        width: f32,
        color: Color,
        from: Point,
        to: Point,
        clip: Option<Rect>,
    ) {
        let scratch = &mut self.scratch;
        if let Some(layer) = self.layer_stack.last_mut() {
            draw_thick_line_clipped(
                &mut layer.buffer,
                from,
                to,
                width,
                color,
                clip,
                &mut scratch.path_points,
                &mut scratch.crossings,
                &mut scratch.coverage,
            );
        } else {
            draw_thick_line_clipped(
                &mut self.buffer,
                from,
                to,
                width,
                color,
                clip,
                &mut scratch.path_points,
                &mut scratch.crossings,
                &mut scratch.coverage,
            );
        }
    }

    fn recycle_active_layers(&mut self) {
        while let Some(layer) = self.layer_stack.pop() {
            self.layer_pool.push(layer.buffer);
        }
        self.clip_stack.clear();
        self.clip_entries.clear();
    }

    #[cfg(test)]
    fn retained_layer_count(&self) -> usize {
        self.layer_pool.len()
    }

    pub fn execute(&mut self, ctx: &PaintContext<'_>) {
        self.execute_with_damage(ctx, None);
    }

    pub fn execute_with_damage(&mut self, ctx: &PaintContext<'_>, damage_rects: Option<&[Rect]>) {
        self.execute_damage_impl(ctx, damage_rects, false);
    }

    fn execute_damage_impl(
        &mut self,
        ctx: &PaintContext<'_>,
        damage_rects: Option<&[Rect]>,
        reconstructing: bool,
    ) {
        if ctx
            .commands()
            .iter()
            .any(|c| matches!(c, PaintCommand::DrawDisplayList { .. }))
        {
            return self.execute_damage_impl(&ctx.flattened(), damage_rects, reconstructing);
        }
        self.backdrop.fallback_count = 0;
        let uses_backdrop = ctx
            .commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::BackdropFilterRoundedRect { .. }));
        if uses_backdrop && !reconstructing {
            if let Some(rects) = damage_rects {
                if rects.is_empty() {
                    return;
                }
                // Reconstruct only the dependency closure into a clean surface.
                // Preserve the direct caller's requested output area; the scene
                // pipeline separately propagates output damage to later readers.
                let clean = self.acquire_layer_buffer();
                let mut previous = core::mem::replace(&mut self.buffer, clean);
                let factor = self.scale_milli.max(1) as f32 / 1000.;
                let regions = ctx.reconstruction_regions(
                    rects,
                    Rect::from_xywh(
                        0.,
                        0.,
                        previous.width() as f32 / factor,
                        previous.height() as f32 / factor,
                    ),
                    self.scale_milli,
                );
                self.execute_damage_impl(ctx, Some(&regions), true);
                let width = previous.width() as usize;
                for rect in rects {
                    let (x, y, w, h) = self.rect_to_u32(*rect);
                    if w == 0 || h == 0 {
                        continue;
                    }
                    for row in y..y + h {
                        let start = row as usize * width + x as usize;
                        previous.as_mut_slice()[start..start + w as usize]
                            .copy_from_slice(&self.buffer.as_slice()[start..start + w as usize]);
                    }
                }
                let clean = core::mem::replace(&mut self.buffer, previous);
                self.layer_pool.push(clean);
                return;
            }
        }
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
        self.recycle_active_layers();

        self.backdrop.frame_pixels = 0;

        // Opacity is part of the backend-neutral paint stream. Keeping it as
        // state here makes translucent focus/selection layers match SGFX.
        let mut opacity = 1.0f32;

        for cmd in ctx.commands() {
            if uses_backdrop && !self.layer_stack.is_empty() {
                if let Some(bounds) = command_paint_bounds(cmd, ctx, self.scale_milli) {
                    let bounds = self
                        .active_clip_rect()
                        .and_then(|clip| intersect_rect(bounds, clip));
                    if let (Some(layer), Some(bounds)) = (self.layer_stack.last_mut(), bounds) {
                        layer.pending_bounds = Some(union_rect(layer.pending_bounds, bounds));
                    }
                }
            }
            match cmd {
                PaintCommand::DrawDisplayList { .. } => unreachable!("recordings expanded above"),
                PaintCommand::FillPath { path, color } => {
                    let color = color.with_opacity(color.a * opacity);
                    self.rebuild_scaled_points(path);
                    match self.rebuild_effective_clip_rects(damage_rects) {
                        EffectiveClipRects::Unclipped => {
                            self.fill_scaled_path(color, None);
                        }
                        EffectiveClipRects::Empty => {}
                        EffectiveClipRects::Rects => {
                            for index in 0..self.scratch.clip_rects.len() {
                                let rect = self.scratch.clip_rects[index];
                                let clip = ClipRegion {
                                    rect: self.scale_rect(rect),
                                    corner_radius: 0.0,
                                };
                                self.fill_scaled_path(color, Some(clip));
                            }
                        }
                    }
                }
                PaintCommand::FillRoundedRect {
                    rect,
                    corner_radius,
                    color,
                } => {
                    let color = color.with_opacity(color.a * opacity);
                    let rect = self.scale_rect(*rect);
                    let radius = self.scale_f32(*corner_radius);
                    match self.rebuild_effective_clip_rects(damage_rects) {
                        EffectiveClipRects::Unclipped => {
                            self.paint_rounded_rect(rect, radius, None, color, None);
                        }
                        EffectiveClipRects::Empty => {}
                        EffectiveClipRects::Rects => {
                            for index in 0..self.scratch.clip_rects.len() {
                                let clip_rect = self.scratch.clip_rects[index];
                                let clip = ClipRegion {
                                    rect: self.scale_rect(clip_rect),
                                    corner_radius: 0.0,
                                };
                                self.paint_rounded_rect(rect, radius, None, color, Some(clip));
                            }
                        }
                    }
                }
                PaintCommand::FillVerticalGradientRoundedRect {
                    rect,
                    corner_radius,
                    top_color,
                    bottom_color,
                } => {
                    let color = top_color.with_opacity(top_color.a * opacity);
                    let bottom = bottom_color.with_opacity(bottom_color.a * opacity);
                    let rect = self.scale_rect(*rect);
                    let radius = self.scale_f32(*corner_radius);
                    match self.rebuild_effective_clip_rects(damage_rects) {
                        EffectiveClipRects::Unclipped => {
                            self.paint_gradient_rect(rect, radius, None, color, Some(bottom), None);
                        }
                        EffectiveClipRects::Empty => {}
                        EffectiveClipRects::Rects => {
                            for index in 0..self.scratch.clip_rects.len() {
                                let clip_rect = self.scratch.clip_rects[index];
                                let clip = ClipRegion {
                                    rect: self.scale_rect(clip_rect),
                                    corner_radius: 0.0,
                                };
                                self.paint_gradient_rect(
                                    rect,
                                    radius,
                                    None,
                                    color,
                                    Some(bottom),
                                    Some(clip),
                                );
                            }
                        }
                    }
                }
                PaintCommand::BackdropFilterRoundedRect {
                    rect,
                    corner_radius,
                    blur_radius,
                    tint,
                    refraction,
                } => {
                    if damage_rects.is_some_and(|damage| {
                        !damage.iter().any(|area| {
                            intersect_rect(
                                *area,
                                Rect::from_xywh(
                                    rect.left() - 1.,
                                    rect.top() - 1.,
                                    rect.size.width + 2.,
                                    rect.size.height + 2.,
                                ),
                            )
                            .is_some()
                        })
                    }) {
                        continue;
                    }
                    self.paint_backdrop(
                        *rect,
                        *corner_radius,
                        *blur_radius,
                        *tint,
                        opacity,
                        *refraction,
                    );
                }
                PaintCommand::DrawRoundedRectShadow {
                    rect,
                    corner_radius,
                    offset,
                    blur_radius,
                    spread_radius,
                    color,
                } => {
                    if ![
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                        *corner_radius,
                        offset.dx,
                        offset.dy,
                        *blur_radius,
                        *spread_radius,
                    ]
                    .iter()
                    .all(|v| v.is_finite())
                    {
                        continue;
                    }
                    let physical = self.scale_rect(Rect::from_xywh(
                        rect.left() + offset.dx,
                        rect.top() + offset.dy,
                        rect.size.width,
                        rect.size.height,
                    ));
                    self.paint_soft_shadow(
                        physical,
                        self.scale_f32(*corner_radius),
                        self.scale_f32(blur_radius.max(0.0)),
                        self.scale_f32(*spread_radius),
                        color.with_opacity(color.a * opacity),
                        damage_rects,
                    );
                }
                PaintCommand::StrokeRect {
                    rect,
                    stroke_width,
                    color,
                } => {
                    let color = color.with_opacity(color.a * opacity);
                    let rect = self.scale_rect(*rect);
                    let stroke_width = self.scale_f32(stroke_width.max(0.0));
                    match self.rebuild_effective_clip_rects(damage_rects) {
                        EffectiveClipRects::Unclipped => {
                            self.paint_rounded_rect(rect, 0.0, Some(stroke_width), color, None)
                        }
                        EffectiveClipRects::Empty => {}
                        EffectiveClipRects::Rects => {
                            for index in 0..self.scratch.clip_rects.len() {
                                let clip_rect = self.scratch.clip_rects[index];
                                let clip = ClipRegion {
                                    rect: self.scale_rect(clip_rect),
                                    corner_radius: 0.0,
                                };
                                self.paint_rounded_rect(
                                    rect,
                                    0.0,
                                    Some(stroke_width),
                                    color,
                                    Some(clip),
                                );
                            }
                        }
                    }
                }
                PaintCommand::StrokeRoundedRect {
                    rect,
                    corner_radius,
                    stroke_width,
                    color,
                } => {
                    let color = color.with_opacity(color.a * opacity);
                    let rect = self.scale_rect(*rect);
                    let radius = self.scale_f32(*corner_radius);
                    let stroke_width = self.scale_f32(stroke_width.max(0.0));
                    match self.rebuild_effective_clip_rects(damage_rects) {
                        EffectiveClipRects::Unclipped => {
                            self.paint_rounded_rect(rect, radius, Some(stroke_width), color, None)
                        }
                        EffectiveClipRects::Empty => {}
                        EffectiveClipRects::Rects => {
                            for index in 0..self.scratch.clip_rects.len() {
                                let clip_rect = self.scratch.clip_rects[index];
                                let clip = ClipRegion {
                                    rect: self.scale_rect(clip_rect),
                                    corner_radius: 0.0,
                                };
                                self.paint_rounded_rect(
                                    rect,
                                    radius,
                                    Some(stroke_width),
                                    color,
                                    Some(clip),
                                );
                            }
                        }
                    }
                }
                PaintCommand::StrokePath {
                    path,
                    stroke_width,
                    color,
                } => {
                    let color = color.with_opacity(color.a * opacity);
                    let scaled_width = self.scale_f32(stroke_width.max(0.0));
                    self.rebuild_scaled_points(path);
                    match self.rebuild_effective_clip_rects(damage_rects) {
                        EffectiveClipRects::Unclipped => {
                            self.draw_scaled_path_stroke(scaled_width, color, None);
                        }
                        EffectiveClipRects::Empty => {}
                        EffectiveClipRects::Rects => {
                            for index in 0..self.scratch.clip_rects.len() {
                                let clip_rect = self.scratch.clip_rects[index];
                                let clip = self.scale_rect(clip_rect);
                                self.draw_scaled_path_stroke(scaled_width, color, Some(clip));
                            }
                        }
                    }
                }
                PaintCommand::DrawText {
                    position,
                    text,
                    color,
                    font_size_px,
                } => {
                    let color = color.with_opacity(color.a * opacity);
                    match self.rebuild_effective_clip_rects(damage_rects) {
                        EffectiveClipRects::Unclipped => {
                            let mut canvas =
                                crate::graphics::Canvas::for_buffer(self.current_buffer_mut());
                            canvas.draw_text_sized(
                                position.x as i32,
                                position.y as i32,
                                text,
                                color,
                                *font_size_px,
                            );
                        }
                        EffectiveClipRects::Empty => {}
                        EffectiveClipRects::Rects => {
                            for index in 0..self.scratch.clip_rects.len() {
                                let rect = self.scratch.clip_rects[index];
                                let mut canvas =
                                    crate::graphics::Canvas::for_buffer(self.current_buffer_mut());
                                canvas.draw_text_sized_clipped(
                                    position.x as i32,
                                    position.y as i32,
                                    text,
                                    color,
                                    *font_size_px,
                                    rect,
                                    0.0,
                                );
                            }
                        }
                    }
                }
                PaintCommand::DrawIcon {
                    rect,
                    icon,
                    style,
                    color,
                } => {
                    let color = color.with_opacity(color.a * opacity);
                    let destination = self.scale_rect(*rect);
                    let pixel_size =
                        libm::ceilf(destination.size.width.min(destination.size.height).max(1.0))
                            .min(u16::MAX as f32) as u16;
                    let raster = crate::icon::rasterize_icon(*icon, pixel_size, *style);
                    let destination_x = destination.origin.x as i32;
                    let destination_y = destination.origin.y as i32;
                    match self.rebuild_effective_clip_rects(damage_rects) {
                        EffectiveClipRects::Unclipped => {
                            self.current_buffer_mut().composite_alpha_mask(
                                raster.mask.as_ref(),
                                raster.width,
                                raster.height,
                                destination_x,
                                destination_y,
                                color,
                                None,
                            );
                        }
                        EffectiveClipRects::Empty => {}
                        EffectiveClipRects::Rects => {
                            for index in 0..self.scratch.clip_rects.len() {
                                let clip = self.scale_rect(self.scratch.clip_rects[index]);
                                self.current_buffer_mut().composite_alpha_mask(
                                    raster.mask.as_ref(),
                                    raster.width,
                                    raster.height,
                                    destination_x,
                                    destination_y,
                                    color,
                                    Some(clip),
                                );
                            }
                        }
                    }
                }
                PaintCommand::DrawBuffer { dst, buffer_idx } => {
                    if let Some(src) = ctx.buffer(BufferHandle(*buffer_idx)) {
                        let dst = self.scale_rect(*dst);
                        let dst_x = dst.origin.x as i32;
                        let dst_y = dst.origin.y as i32;
                        match self.rebuild_effective_clip_rects(damage_rects) {
                            EffectiveClipRects::Unclipped => {
                                self.current_buffer_mut()
                                    .composite(src, dst_x, dst_y, opacity);
                            }
                            EffectiveClipRects::Empty => {}
                            EffectiveClipRects::Rects => {
                                for index in 0..self.scratch.clip_rects.len() {
                                    let rect = self.scratch.clip_rects[index];
                                    let clip = self.scale_rect(rect);
                                    self.current_buffer_mut().composite_clipped(
                                        src,
                                        dst_x,
                                        dst_y,
                                        opacity,
                                        clip.origin.x as i32,
                                        clip.origin.y as i32,
                                        clip.size.width as i32,
                                        clip.size.height as i32,
                                    );
                                }
                            }
                        }
                    }
                }
                PaintCommand::DrawBufferRect {
                    dst,
                    src,
                    buffer_idx,
                    opacity: command_opacity,
                } => {
                    if let Some(buf) = ctx.buffer(BufferHandle(*buffer_idx)) {
                        let dst = self.scale_rect(*dst);
                        let source_scale = buf.scale_milli() as f32 / 1000.0;
                        let src = Rect::from_xywh(
                            src.origin.x * source_scale,
                            src.origin.y * source_scale,
                            src.size.width * source_scale,
                            src.size.height * source_scale,
                        );
                        match self.rebuild_effective_clip_rects(damage_rects) {
                            EffectiveClipRects::Unclipped => composite_scaled_buffer(
                                self.current_buffer_mut(),
                                buf,
                                dst,
                                src,
                                *command_opacity * opacity,
                                None,
                            ),
                            EffectiveClipRects::Empty => {}
                            EffectiveClipRects::Rects => {
                                for index in 0..self.scratch.clip_rects.len() {
                                    let clip = self.scale_rect(self.scratch.clip_rects[index]);
                                    composite_scaled_buffer(
                                        self.current_buffer_mut(),
                                        buf,
                                        dst,
                                        src,
                                        *command_opacity * opacity,
                                        Some(clip),
                                    );
                                }
                            }
                        }
                    }
                }
                PaintCommand::PushClip {
                    rect,
                    corner_radius,
                } => {
                    self.push_clip(ClipRegion {
                        rect: *rect,
                        corner_radius: *corner_radius,
                    });
                }
                PaintCommand::PopClip => {
                    self.pop_clip(damage_rects);
                }
                PaintCommand::SetOpacity {
                    opacity: next_opacity,
                } => {
                    opacity = if next_opacity.is_finite() {
                        next_opacity.clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                }
                PaintCommand::Extension {
                    rect: _,
                    payload: _,
                    ..
                } => {}
            }
        }

        while !self.clip_entries.is_empty() {
            self.pop_clip(damage_rects);
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

    /// Number of backdrop commands replaced with opaque semantic tint because
    /// of the CPU ROI/frame budget during the most recent replay. Explicit
    /// zero-radius tints and already-opaque materials are not fallbacks. An
    /// empty-damage execution resets this diagnostic to zero.
    pub fn backdrop_fallback_count(&self) -> usize {
        self.backdrop.fallback_count
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

// Integrate horizontal coverage exactly and use eight subpixel scanlines in Y.
// Accumulate a difference row so opaque interiors still cost one write per pixel,
// not eight. Scratch is retained across commands/frames; there are no row allocations.
const COVERAGE_SAMPLES: usize = 8;
const COVERAGE_WEIGHT: f32 = 1.0 / COVERAGE_SAMPLES as f32;

fn add_coverage_span(row: &mut [f32], origin: i32, left: f32, right: f32) {
    let width = row.len() - 1;
    let left = (left - origin as f32).clamp(0.0, width as f32);
    let right = (right - origin as f32).clamp(0.0, width as f32);
    if right <= left {
        return;
    }
    let first = libm::floorf(left) as usize;
    let last = libm::floorf(right) as usize;
    if first == last {
        let amount = (right - left) * COVERAGE_WEIGHT;
        row[first] += amount;
        row[first + 1] -= amount;
    } else {
        let left_fraction = left - first as f32;
        let right_fraction = right - last as f32;
        row[first] += (1.0 - left_fraction) * COVERAGE_WEIGHT;
        row[first + 1] += left_fraction * COVERAGE_WEIGHT;
        row[last] -= (1.0 - right_fraction) * COVERAGE_WEIGHT;
        if last < width {
            row[last + 1] -= right_fraction * COVERAGE_WEIGHT;
        }
    }
}

fn blend_coverage_row(buffer: &mut Buffer, y: i32, left: i32, row: &[f32], color: Color) {
    let pixel = color.to_bgra();
    let opaque = color.a >= 1.0;
    let start = y as usize * buffer.width() as usize + left as usize;
    let data = &mut buffer.as_mut_slice()[start..start + row.len() - 1];
    let mut coverage = 0.0f32;
    for (dst, delta) in data.iter_mut().zip(row.iter()) {
        coverage += delta;
        // Ignore roundoff outside the shape, and retain exact opaque interiors.
        if coverage >= 0.999_99 {
            *dst = if opaque {
                pixel
            } else {
                Buffer::blend_pixels(*dst, pixel, 1.0)
            };
        } else if coverage > 0.000_01 {
            *dst = Buffer::blend_pixels(*dst, pixel, coverage.clamp(0.0, 1.0));
        }
    }
}

fn rounded_span(rect: Rect, radius: f32, y: f32) -> Option<(f32, f32)> {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 || y < rect.top() || y >= rect.bottom() {
        return None;
    }
    let radius = radius
        .max(0.0)
        .min(rect.size.width * 0.5)
        .min(rect.size.height * 0.5);
    let dy = (rect.top() + radius - y)
        .max(y - (rect.bottom() - radius))
        .max(0.0);
    let inset = if dy > 0.0 {
        if radius > 1.0e10 {
            let r = radius as f64;
            let d = dy as f64;
            (r - libm::sqrt((r * r - d * d).max(0.0))) as f32
        } else {
            radius - libm::sqrtf((radius * radius - dy * dy).max(0.0))
        }
    } else {
        0.0
    };
    Some((rect.left() + inset, rect.right() - inset))
}

// Rectangular clip/damage bounds remain a hard pixel scissor, matching the
// existing clear and retained-repaint contract. Rounded clips get their mask
// once at layer composition, not also at the layer's rectangular bounds.
fn clip_span_at(clip: ClipRegion, y: f32) -> Option<(f32, f32)> {
    if clip.corner_radius > 0.0 {
        return rounded_span(clip.rect, clip.corner_radius, y);
    }
    if y < libm::floorf(clip.rect.top()) || y >= libm::ceilf(clip.rect.bottom()) {
        return None;
    }
    Some((
        libm::floorf(clip.rect.left()),
        libm::ceilf(clip.rect.right()),
    ))
}

fn add_clipped_span(row: &mut [f32], left: i32, span: (f32, f32), clip: Option<(f32, f32)>) {
    let (mut a, mut b) = span;
    if let Some((c, d)) = clip {
        a = a.max(c);
        b = b.min(d);
    }
    add_coverage_span(row, left, a, b);
}

fn fill_polygon(
    buffer: &mut Buffer,
    path: &[Point],
    color: Color,
    clip: Option<ClipRegion>,
    crossings: &mut Vec<f32>,
    coverage: &mut Vec<f32>,
) {
    if path.len() < 3 || color.a <= 0.0 || path.iter().any(|p| !p.x.is_finite() || !p.y.is_finite())
    {
        return;
    }
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for p in path {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    let bounds = Rect::from_xywh(min_x, min_y, max_x - min_x, max_y - min_y);
    let (left, top, right, bottom) = raster_bounds(buffer, bounds, clip.map(|c| c.rect));
    if right <= left || bottom <= top {
        return;
    }
    coverage.resize((right - left) as usize + 1, 0.0);
    for y in top..bottom {
        coverage.fill(0.0);
        for sample in 0..COVERAGE_SAMPLES {
            let yc = y as f32 + (sample as f32 + 0.5) * COVERAGE_WEIGHT;
            let clip_span = if let Some(c) = clip {
                let Some(span) = clip_span_at(c, yc) else {
                    continue;
                };
                Some(span)
            } else {
                None
            };
            crossings.clear();
            for i in 0..path.len() {
                let p0 = path[i];
                let p1 = path[(i + 1) % path.len()];
                if (p0.y <= yc) != (p1.y <= yc) {
                    let t = (yc - p0.y) / (p1.y - p0.y);
                    crossings.push(p0.x + t * (p1.x - p0.x));
                }
            }
            crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            for pair in crossings.chunks_exact(2) {
                add_clipped_span(coverage, left, (pair[0], pair[1]), clip_span);
            }
        }
        blend_coverage_row(buffer, y, left, coverage, color);
    }
}

fn raster_rounded_rect(
    buffer: &mut Buffer,
    rect: Rect,
    radius: f32,
    stroke_width: Option<f32>,
    color: Color,
    bottom_color: Option<Color>,
    clip: Option<ClipRegion>,
    coverage: &mut Vec<f32>,
) {
    if [rect.left(), rect.top(), rect.size.width, rect.size.height]
        .iter()
        .any(|v| !v.is_finite())
        || (color.a <= 0.0 && bottom_color.is_none_or(|bottom| bottom.a <= 0.0))
        || !radius.is_finite()
        || stroke_width.is_some_and(|w| !w.is_finite() || w <= 0.0)
    {
        return;
    }
    let (left, top, right, bottom) = raster_bounds(buffer, rect, clip.map(|c| c.rect));
    if right <= left || bottom <= top {
        return;
    }
    let radius = radius
        .max(0.0)
        .min(rect.size.width * 0.5)
        .min(rect.size.height * 0.5);
    let inner = stroke_width.map(|width| (inset_rect(rect, width), (radius - width).max(0.0)));
    coverage.resize((right - left) as usize + 1, 0.0);
    for y in top..bottom {
        coverage.fill(0.0);
        for sample in 0..COVERAGE_SAMPLES {
            let yc = y as f32 + (sample as f32 + 0.5) * COVERAGE_WEIGHT;
            let Some(outer) = rounded_span(rect, radius, yc) else {
                continue;
            };
            let clip_span = if let Some(c) = clip {
                let Some(span) = clip_span_at(c, yc) else {
                    continue;
                };
                Some(span)
            } else {
                None
            };
            if let Some((inner_rect, inner_radius)) = inner {
                if let Some(hole) = rounded_span(inner_rect, inner_radius, yc) {
                    add_clipped_span(coverage, left, (outer.0, hole.0), clip_span);
                    add_clipped_span(coverage, left, (hole.1, outer.1), clip_span);
                    continue;
                }
            }
            add_clipped_span(coverage, left, outer, clip_span);
        }
        let row_color = bottom_color.map_or(color, |bottom| {
            interpolate_color(
                color,
                bottom,
                ((y as f32 + 0.5 - rect.top()) / rect.size.height).clamp(0.0, 1.0),
            )
        });
        blend_coverage_row(buffer, y, left, coverage, row_color);
    }
}

fn composite_layer_clipped(
    dst: &mut Buffer,
    src: &Buffer,
    clip: ClipRegion,
    extra_clip: Option<Rect>,
) {
    let Some(rect) = extra_clip.map_or(Some(clip.rect), |extra| intersect_rect(clip.rect, extra))
    else {
        return;
    };

    let left = libm::floorf(rect.origin.x).max(0.0) as i32;
    let top = libm::floorf(rect.origin.y).max(0.0) as i32;
    let right = libm::ceilf(rect.origin.x + rect.size.width)
        .min(dst.width() as f32)
        .min(src.width() as f32) as i32;
    let bottom = libm::ceilf(rect.origin.y + rect.size.height)
        .min(dst.height() as f32)
        .min(src.height() as f32) as i32;
    if right <= left || bottom <= top {
        return;
    }

    let dst_width = dst.width() as usize;
    let src_width = src.width() as usize;
    let src_data = src.as_slice();
    let dst_data = dst.as_mut_slice();

    for y in top..bottom {
        let src_row = y as usize * src_width;
        let dst_row = y as usize * dst_width;
        for x in left..right {
            let coverage = rounded_pixel_coverage(clip.rect, clip.corner_radius, x, y);
            if coverage <= 0.0 {
                continue;
            }
            let src_pixel = src_data[src_row + x as usize];
            let dst_idx = dst_row + x as usize;
            dst_data[dst_idx] = Buffer::blend_pixels(dst_data[dst_idx], src_pixel, coverage);
        }
    }
}

/// Rasterize all stroke widths as covered quads, including subpixel hairlines.
/// Square caps match the existing path convention.
fn draw_thick_line_clipped(
    buffer: &mut Buffer,
    from: Point,
    to: Point,
    width: f32,
    color: Color,
    clip: Option<Rect>,
    path_points: &mut Vec<Point>,
    crossings: &mut Vec<f32>,
    coverage: &mut Vec<f32>,
) {
    if color.a <= 0.0 || width <= 0.0 {
        return;
    }

    let half_w = width / 2.0;
    let clip_region = clip.map(|rect| ClipRegion {
        rect,
        corner_radius: 0.0,
    });

    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = libm::sqrtf(dx * dx + dy * dy);

    if len < 0.001 {
        path_points.clear();
        path_points.push(Point::new(from.x - half_w, from.y - half_w));
        path_points.push(Point::new(from.x + half_w, from.y - half_w));
        path_points.push(Point::new(from.x + half_w, from.y + half_w));
        path_points.push(Point::new(from.x - half_w, from.y + half_w));
        fill_polygon(buffer, path_points, color, clip_region, crossings, coverage);
        return;
    }

    let inv_len = 1.0 / len;
    let dirx = dx * inv_len;
    let diry = dy * inv_len;
    let nx = -diry;
    let ny = dirx;

    let start_x = from.x - dirx * half_w;
    let start_y = from.y - diry * half_w;
    let end_x = to.x + dirx * half_w;
    let end_y = to.y + diry * half_w;

    path_points.clear();
    path_points.push(Point::new(start_x + nx * half_w, start_y + ny * half_w));
    path_points.push(Point::new(end_x + nx * half_w, end_y + ny * half_w));
    path_points.push(Point::new(end_x - nx * half_w, end_y - ny * half_w));
    path_points.push(Point::new(start_x - nx * half_w, start_y - ny * half_w));
    fill_polygon(buffer, path_points, color, clip_region, crossings, coverage);
}

fn intersect_rect(a: Rect, b: Rect) -> Option<Rect> {
    let left = a.origin.x.max(b.origin.x);
    let top = a.origin.y.max(b.origin.y);
    let right = (a.origin.x + a.size.width).min(b.origin.x + b.size.width);
    let bottom = (a.origin.y + a.size.height).min(b.origin.y + b.size.height);
    if right <= left || bottom <= top {
        return None;
    }
    Some(Rect::from_xywh(left, top, right - left, bottom - top))
}
fn rounded_pixel_coverage(rect: Rect, radius: f32, x: i32, y: i32) -> f32 {
    let radius = radius
        .max(0.0)
        .min(rect.size.width * 0.5)
        .min(rect.size.height * 0.5);
    let x0 = x as f32;
    let y0 = y as f32;
    if x0 >= rect.left()
        && x0 + 1.0 <= rect.right()
        && y0 >= rect.top()
        && y0 + 1.0 <= rect.bottom()
        && ((x0 >= rect.left() + radius && x0 + 1.0 <= rect.right() - radius)
            || (y0 >= rect.top() + radius && y0 + 1.0 <= rect.bottom() - radius))
    {
        return 1.0;
    }
    let mut coverage = 0.0;
    for sample in 0..COVERAGE_SAMPLES {
        let yc = y as f32 + (sample as f32 + 0.5) * COVERAGE_WEIGHT;
        if let Some((left, right)) = rounded_span(rect, radius, yc) {
            coverage +=
                (right.min(x as f32 + 1.0) - left.max(x as f32)).clamp(0.0, 1.0) * COVERAGE_WEIGHT;
        }
    }
    coverage
}

fn composite_scaled_buffer(
    target: &mut Buffer,
    source: &Buffer,
    dst: Rect,
    src: Rect,
    opacity: f32,
    clip: Option<Rect>,
) {
    let (dx, dy, dw, dh) = (
        dst.origin.x as i32,
        dst.origin.y as i32,
        dst.size.width as i32,
        dst.size.height as i32,
    );
    let (sx, sy, sw, sh) = (
        src.origin.x as i32,
        src.origin.y as i32,
        src.size.width as i32,
        src.size.height as i32,
    );
    if dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 {
        return;
    }
    let clip = clip.unwrap_or(Rect::from_xywh(
        0.0,
        0.0,
        target.width() as f32,
        target.height() as f32,
    ));
    if dw == sw && dh == sh {
        target.composite_rect_clipped(
            source,
            sx,
            sy,
            sw,
            sh,
            dx,
            dy,
            opacity,
            clip.origin.x as i32,
            clip.origin.y as i32,
            clip.size.width as i32,
            clip.size.height as i32,
        );
        return;
    }
    let left = dx.max(0).max(clip.origin.x as i32);
    let top = dy.max(0).max(clip.origin.y as i32);
    let right = (dx + dw)
        .min(target.width() as i32)
        .min((clip.origin.x + clip.size.width) as i32);
    let bottom = (dy + dh)
        .min(target.height() as i32)
        .min((clip.origin.y + clip.size.height) as i32);
    let stride = target.width() as usize;
    let pixels = target.as_mut_slice();
    for y in top..bottom {
        let source_y = sy + (((y - dy) as f64 + 0.5) * sh as f64 / dh as f64) as i32;
        for x in left..right {
            let source_x = sx + (((x - dx) as f64 + 0.5) * sw as f64 / dw as f64) as i32;
            if source_x < 0 || source_y < 0 {
                continue;
            }
            if let Some(pixel) = source.get_pixel(source_x as u32, source_y as u32) {
                let index = y as usize * stride + x as usize;
                pixels[index] = Buffer::blend_pixels(pixels[index], pixel, opacity);
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
    use crate::testing::alloc_counter::measure_allocations;

    #[test]
    fn retained_composition_preserves_cache_identity_and_fractional_offset() {
        let bounds = Rect::from_xywh(0., 0., 100., 300.);
        let mut paint = PaintContext::new();
        paint.fill_rect(bounds, Color::WHITE);
        let list = paint.into_cached_layer(bounds);
        let mut scene = PaintContext::new();
        scene.draw_display_list(Point::new(2.25, -20.5), Arc::clone(&list));
        let mut composed = PaintContext::new();
        composed.append_retained_at(&scene, Point::new(13., 28.));
        assert!(
            matches!(&composed.commands()[0], PaintCommand::DrawDisplayList { origin, list: replayed }
            if *origin == Point::new(15.25, 7.5) && Arc::ptr_eq(replayed, &list)
                && replayed.raster_cache_bounds() == Some(bounds))
        );
        assert!(
            matches!(&composed.flattened().commands()[0], PaintCommand::FillPath { path, .. }
            if path[0] == Point::new(15.25, 7.5))
        );
    }

    #[test]
    fn cached_glass_recognizes_opaque_rect_paths_but_not_their_bounding_boxes() {
        let bounds = Rect::from_xywh(0., 0., 120., 80.);
        let glass = |ctx: &mut PaintContext<'_>| {
            ctx.refractive_glass_rounded_rect(
                Rect::from_xywh(20., 20., 80., 40.),
                12.,
                4.,
                Color::WHITE.with_opacity(0.5),
                1.,
            )
        };
        let mut opaque = PaintContext::new();
        opaque.fill_rect(bounds, Color::WHITE);
        glass(&mut opaque);
        assert_eq!(
            opaque.into_cached_layer(bounds).raster_cache_bounds(),
            Some(bounds)
        );
        for (path, color) in [
            (path_rect(bounds), Color::WHITE.with_opacity(0.9)),
            (
                path_triangle(
                    Point::new(0., 0.),
                    Point::new(120., 0.),
                    Point::new(0., 80.),
                ),
                Color::WHITE,
            ),
            (
                alloc::vec![
                    Point::new(0., 0.),
                    Point::new(120., 80.),
                    Point::new(120., 0.),
                    Point::new(0., 80.)
                ],
                Color::WHITE,
            ),
        ] {
            let mut paint = PaintContext::new();
            paint.fill_path(path, color);
            glass(&mut paint);
            assert!(
                paint
                    .into_cached_layer(bounds)
                    .raster_cache_bounds()
                    .is_none(),
                "translucent, triangular and self-intersecting backings are not opaque rectangles"
            );
        }
        let mut clipped = PaintContext::new();
        clipped.push_clip(Rect::from_xywh(30., 30., 50., 20.));
        clipped.fill_rect(bounds, Color::WHITE);
        clipped.pop_clip();
        glass(&mut clipped);
        assert!(
            clipped
                .into_cached_layer(bounds)
                .raster_cache_bounds()
                .is_none(),
            "the backing must include the filter halo after clipping"
        );
        let mut late = PaintContext::new();
        glass(&mut late);
        late.fill_rect(bounds, Color::WHITE);
        assert!(
            late.into_cached_layer(bounds)
                .raster_cache_bounds()
                .is_none(),
            "later paint cannot back an earlier filter"
        );
    }

    fn aa_pixels(ctx: &PaintContext<'_>, scale: u32, background: Color) -> Buffer {
        let mut renderer = CpuPaintRenderer::new(Size::new(40.0, 40.0), scale, background);
        renderer.execute(ctx);
        renderer.into_buffer()
    }

    #[test]
    fn cpu_backdrop_command_bounds_translation_and_recording() {
        let mut local = PaintContext::new();
        local.backdrop_filter_rounded_rect(
            Rect::from_xywh(2.0, 3.0, 20.0, 12.0),
            4.0,
            9.0,
            Color::WHITE,
        );
        let mut ctx = PaintContext::new();
        ctx.draw_display_list(Point::new(5.0, 7.0), local.into_display_list());
        let flat = ctx.flattened();
        assert!(
            matches!(flat.commands()[0], PaintCommand::BackdropFilterRoundedRect { rect, corner_radius: 4.0, blur_radius: 9.0, .. }
            if rect == Rect::from_xywh(7.0, 10.0, 20.0, 12.0))
        );
        assert_eq!(
            ctx.raster_bounds(Size::new(100.0, 100.0), 2000),
            Rect::from_xywh(7.0, 10.0, 20.0, 12.0)
        );
    }

    #[test]
    fn cpu_backdrop_samples_halo_and_changes_with_background_at_both_scales() {
        for scale in [1000, 2000] {
            let scene = |color, filter| {
                let mut ctx = PaintContext::new();
                ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 12.0, 40.0), color);
                if filter {
                    ctx.backdrop_filter_rounded_rect(
                        Rect::from_xywh(12.0, 8.0, 20.0, 24.0),
                        6.0,
                        9.0,
                        Color::TRANSPARENT,
                    );
                }
                aa_pixels(&ctx, scale, Color::BLACK)
            };
            let plain = scene(Color::rgb(255, 0, 0), false);
            let red = scene(Color::rgb(255, 0, 0), true);
            let blue = scene(Color::rgb(0, 0, 255), true);
            let s = scale as usize / 1000;
            let sample = 20 * s * red.width() as usize + 13 * s;
            assert!(
                red.as_slice()[sample].to_le_bytes()[2] > 0,
                "sample outside the shape must affect its inside"
            );
            assert_ne!(red.as_slice()[sample], blue.as_slice()[sample]);
            for y in 0..red.height() as i32 {
                for x in 0..red.width() as i32 {
                    if rounded_pixel_coverage(
                        Rect::from_xywh(
                            12.0 * s as f32,
                            8.0 * s as f32,
                            20.0 * s as f32,
                            24.0 * s as f32,
                        ),
                        6.0 * s as f32,
                        x,
                        y,
                    ) == 0.0
                    {
                        let i = y as usize * red.width() as usize + x as usize;
                        assert_eq!(
                            red.as_slice()[i],
                            plain.as_slice()[i],
                            "outside shape at {x},{y}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cpu_backdrop_uses_immutable_snapshot_and_leaves_later_foreground_sharp() {
        for scale in [1000, 2000] {
            let mut ctx = PaintContext::new();
            ctx.fill_rect(Rect::from_xywh(19.0, 0.0, 2.0, 40.0), Color::WHITE);
            ctx.backdrop_filter_rounded_rect(
                Rect::from_xywh(0.0, 0.0, 40.0, 40.0),
                0.0,
                12.0,
                Color::TRANSPARENT,
            );
            let soft = aa_pixels(&ctx, scale, Color::BLACK);
            let s = scale as usize / 1000;
            let width = soft.width() as usize;
            for x in 0..width {
                assert_eq!(
                    soft.as_slice()[20 * s * width + x],
                    soft.as_slice()[20 * s * width + width - 1 - x]
                );
            }
            ctx.fill_rect(Rect::from_xywh(19.0, 0.0, 2.0, 40.0), Color::rgb(255, 0, 0));
            let sharp = aa_pixels(&ctx, scale, Color::BLACK);
            for x in 0..width {
                let i = 20 * s * width + x;
                if (19 * s..21 * s).contains(&x) {
                    assert_eq!(sharp.as_slice()[i], Color::rgb(255, 0, 0).to_bgra());
                } else {
                    assert_eq!(sharp.as_slice()[i], soft.as_slice()[i]);
                }
            }
        }
    }

    #[test]
    fn cpu_backdrop_blurs_premultiplied_alpha_and_replaces_not_source_over() {
        let mut source = Buffer::from_logical_dimensions_with_scale(40, 40, 1000);
        for y in 0..40 {
            for x in 0..40 {
                source.as_mut_slice()[y * 40 + x] = if x < 20 {
                    Color::rgba(255, 0, 0, 128).to_bgra()
                } else {
                    0x0000_00ff
                };
            }
        }
        let mut ctx = PaintContext::new();
        ctx.draw_buffer_ref(Rect::from_xywh(0.0, 0.0, 40.0, 40.0), &source);
        ctx.backdrop_filter_rounded_rect(
            Rect::from_xywh(0.0, 0.0, 40.0, 40.0),
            0.0,
            9.0,
            Color::TRANSPARENT,
        );
        let result = aa_pixels(&ctx, 1000, Color::TRANSPARENT);
        let middle = result.as_slice()[20 * 40 + 19].to_le_bytes();
        assert_eq!(
            middle[0], 0,
            "hidden blue in zero-alpha taps must not bleed"
        );
        assert_eq!(middle[2], 255);
        assert!(
            middle[3] > 0 && middle[3] < 128,
            "replacement must be allowed to reduce alpha"
        );
        assert_eq!(
            result.as_slice()[20 * 40 + 5],
            Color::rgba(255, 0, 0, 128).to_bgra()
        );
    }

    #[test]
    fn cpu_backdrop_nested_clips_resolve_transparent_layers_once() {
        for scale in [1000, 2000] {
            let outer = Rect::from_xywh(3.0, 3.0, 34.0, 34.0);
            let inner = Rect::from_xywh(7.0, 7.0, 26.0, 26.0);
            let prefix = |ctx: &mut PaintContext<'_>| {
                ctx.fill_rect(
                    Rect::from_xywh(0.0, 0.0, 20.0, 40.0),
                    Color::rgba(255, 0, 0, 100),
                );
                ctx.push_rounded_clip(outer, 8.0);
                ctx.fill_rect(
                    Rect::from_xywh(2.0, 2.0, 36.0, 36.0),
                    Color::rgba(0, 0, 255, 110),
                );
                ctx.push_clip(Rect::from_xywh(5.0, 5.0, 30.0, 27.0));
                ctx.push_rounded_clip(inner, 6.0);
                ctx.fill_rect(
                    Rect::from_xywh(20.0, 0.0, 20.0, 40.0),
                    Color::rgba(0, 255, 0, 90),
                );
            };
            let material_and_foreground = |ctx: &mut PaintContext<'_>| {
                ctx.set_opacity(0.6);
                ctx.backdrop_filter_rounded_rect(
                    Rect::from_xywh(4.0, 4.0, 32.0, 32.0),
                    7.0,
                    6.0,
                    Color::rgba(255, 255, 255, 70),
                );
                ctx.fill_rect(Rect::from_xywh(6.0, 6.0, 15.0, 6.0), Color::BLACK);
                ctx.pop_clip();
                ctx.pop_clip();
                ctx.pop_clip();
            };
            let mut pending = PaintContext::new();
            prefix(&mut pending);
            material_and_foreground(&mut pending);
            let actual = aa_pixels(&pending, scale, Color::TRANSPARENT);
            let mut resolved = PaintContext::new();
            prefix(&mut resolved);
            resolved.pop_clip();
            resolved.pop_clip();
            resolved.pop_clip();
            resolved.push_rounded_clip(outer, 8.0);
            resolved.push_clip(Rect::from_xywh(5.0, 5.0, 30.0, 27.0));
            resolved.push_rounded_clip(inner, 6.0);
            material_and_foreground(&mut resolved);
            let expected = aa_pixels(&resolved, scale, Color::TRANSPARENT);
            assert_eq!(actual.as_slice(), expected.as_slice(), "scale {scale}");
        }
    }

    #[test]
    fn cpu_backdrop_flushes_actual_legacy_buffer_extent_not_destination_size() {
        for scale in [1000, 2000] {
            let mut source = Buffer::from_logical_dimensions_with_scale(20, 20, scale);
            source.clear(Color::rgba(220, 40, 80, 190));
            let render = |resolve_first| {
                let mut ctx = PaintContext::new();
                let clip = Rect::from_xywh(2.0, 2.0, 36.0, 36.0);
                ctx.push_rounded_clip(clip, 6.0);
                ctx.draw_buffer_ref(Rect::from_xywh(6.25, 6.75, 1.0, 1.0), &source);
                if resolve_first {
                    ctx.pop_clip();
                    ctx.push_rounded_clip(clip, 6.0);
                }
                ctx.backdrop_filter_rounded_rect(
                    Rect::from_xywh(4.0, 4.0, 32.0, 32.0),
                    4.0,
                    9.0,
                    Color::rgba(255, 255, 255, 60),
                );
                ctx.pop_clip();
                aa_pixels(&ctx, scale, Color::TRANSPARENT)
            };
            assert_eq!(render(false).as_slice(), render(true).as_slice());
        }
    }

    #[test]
    fn cpu_backdrop_partial_damage_reconstructs_clean_prefix_without_feedback() {
        for scale in [1000, 2000] {
            let scene = |color| {
                let mut ctx = PaintContext::new();
                ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 20.0, 40.0), color);
                ctx.push_rounded_clip(Rect::from_xywh(2.0, 2.0, 36.0, 36.0), 7.0);
                ctx.backdrop_filter_rounded_rect(
                    Rect::from_xywh(5.0, 5.0, 30.0, 30.0),
                    5.0,
                    12.0,
                    Color::rgba(255, 255, 255, 60),
                );
                ctx.fill_rect(Rect::from_xywh(19.0, 5.0, 2.0, 30.0), Color::BLACK);
                ctx.pop_clip();
                ctx
            };
            let old = scene(Color::rgb(255, 0, 0));
            let changed = scene(Color::rgb(0, 0, 255));
            let expected = aa_pixels(&changed, scale, Color::WHITE);
            let mut renderer = CpuPaintRenderer::new(Size::new(40.0, 40.0), scale, Color::WHITE);
            renderer.execute(&old);
            let before = renderer.buffer().as_slice().to_vec();
            let damage = Rect::from_xywh(14.25, 4.25, 12.0, 30.0);
            for _ in 0..6 {
                renderer.execute_with_damage(&changed, Some(&[damage, damage]));
            }
            let (x, y, w, h) = renderer.rect_to_u32(damage);
            for py in 0..expected.height() {
                for px in 0..expected.width() {
                    let index = py as usize * expected.width() as usize + px as usize;
                    let value = if (x..x + w).contains(&px) && (y..y + h).contains(&py) {
                        expected.as_slice()[index]
                    } else {
                        before[index]
                    };
                    assert_eq!(
                        renderer.buffer().as_slice()[index],
                        value,
                        "scale {scale}, pixel {px},{py}"
                    );
                }
            }
            let saved = renderer.buffer().as_slice().to_vec();
            renderer.execute_with_damage(&changed, Some(&[]));
            assert_eq!(saved, renderer.buffer().as_slice());
        }
    }

    #[test]
    fn cpu_backdrop_overlapping_materials_sample_prior_material() {
        let mut first = PaintContext::new();
        first.fill_rect(Rect::from_xywh(0.0, 0.0, 20.0, 40.0), Color::rgb(255, 0, 0));
        first.backdrop_filter_rounded_rect(
            Rect::from_xywh(2.0, 2.0, 25.0, 30.0),
            5.0,
            6.0,
            Color::rgba(255, 255, 255, 80),
        );
        let prefix = aa_pixels(&first, 1000, Color::BLACK);
        first.backdrop_filter_rounded_rect(
            Rect::from_xywh(12.0, 10.0, 26.0, 28.0),
            5.0,
            9.0,
            Color::rgba(0, 0, 255, 50),
        );
        let actual = aa_pixels(&first, 1000, Color::BLACK);
        let mut replay = PaintContext::new();
        replay.draw_buffer_ref(Rect::from_xywh(0.0, 0.0, 40.0, 40.0), &prefix);
        replay.backdrop_filter_rounded_rect(
            Rect::from_xywh(12.0, 10.0, 26.0, 28.0),
            5.0,
            9.0,
            Color::rgba(0, 0, 255, 50),
        );
        assert_eq!(
            actual.as_slice(),
            aa_pixels(&replay, 1000, Color::BLACK).as_slice()
        );
    }

    #[test]
    fn cpu_backdrop_reuses_scratch_and_bounds_work() {
        let mut renderer = CpuPaintRenderer::new(Size::new(40.0, 40.0), 2000, Color::WHITE);
        let mut ctx = PaintContext::new();
        ctx.push_rounded_clip(Rect::from_xywh(0.0, 0.0, 40.0, 40.0), 8.0);
        ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 20.0, 40.0), Color::BLACK);
        ctx.backdrop_filter_rounded_rect(
            Rect::from_xywh(4.0, 4.0, 32.0, 32.0),
            6.0,
            f32::MAX,
            Color::rgba(255, 255, 255, 30),
        );
        ctx.pop_clip();
        renderer.execute(&ctx);
        assert_eq!(
            measure_allocations(|| renderer.execute(&ctx)).allocations,
            0
        );
        assert!(renderer.backdrop.pixels.capacity() <= MAX_BACKDROP_PIXELS);
        renderer.buffer.clear(Color::BLACK);
        renderer.backdrop.frame_pixels = MAX_BACKDROP_FRAME_PIXELS;
        renderer.paint_backdrop(
            Rect::from_xywh(0.0, 0.0, 40.0, 40.0),
            0.0,
            12.0,
            Color::rgba(255, 255, 255, 128),
            1.0,
            0.0,
        );
        assert_eq!(renderer.backdrop.frame_pixels, MAX_BACKDROP_FRAME_PIXELS);
        assert_eq!(renderer.backdrop_fallback_count(), 1);
        assert!(
            renderer
                .buffer()
                .as_slice()
                .iter()
                .all(|p| *p == Color::WHITE.to_bgra())
        );
        renderer.resize(Size::new(40.0, 40.0), 1000);
        assert_eq!(renderer.backdrop.pixels.capacity(), 0);
        assert_eq!(renderer.backdrop_fallback_count(), 0);
    }

    #[test]
    fn cpu_backdrop_large_roi_uses_opaque_semantic_tint_fallback() {
        let mut renderer = CpuPaintRenderer::new(Size::new(1001.0, 1000.0), 1000, Color::BLACK);
        let mut ctx = PaintContext::new();
        ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 500.0, 1000.0), Color::WHITE);
        ctx.backdrop_filter_rounded_rect(
            Rect::from_xywh(0.0, 0.0, 1001.0, 1000.0),
            0.0,
            12.0,
            Color::rgba(255, 0, 0, 128),
        );
        renderer.execute(&ctx);
        assert_eq!(renderer.backdrop.pixels.len(), 0);
        assert_eq!(renderer.backdrop.frame_pixels, 0);
        assert_eq!(renderer.backdrop_fallback_count(), 1);
        assert_eq!(
            renderer.buffer().as_slice()[500 * 1001 + 500],
            Color::rgb(255, 0, 0).to_bgra()
        );
        assert_eq!(
            renderer.buffer().as_slice()[500 * 1001 + 499],
            Color::rgb(255, 0, 0).to_bgra()
        );
        renderer.execute_with_damage(&ctx, Some(&[]));
        assert_eq!(renderer.backdrop_fallback_count(), 0);
        renderer.execute(&ctx);
        assert_eq!(
            renderer.backdrop_fallback_count(),
            1,
            "counter is per replay, not cumulative"
        );
        renderer.execute(&PaintContext::new());
        assert_eq!(renderer.backdrop_fallback_count(), 0);
    }

    #[test]
    fn cpu_backdrop_budget_fallback_preserves_explicit_tint_and_opacity_contracts() {
        let mut renderer = CpuPaintRenderer::new(Size::new(40.0, 40.0), 1000, Color::BLACK);
        let rect = Rect::from_xywh(4.0, 4.0, 32.0, 32.0);
        let center = 20 * 40 + 20;
        for radius in [0.0, -4.0] {
            renderer.buffer.clear(Color::BLACK);
            renderer.backdrop.frame_pixels = MAX_BACKDROP_FRAME_PIXELS;
            renderer.paint_backdrop(rect, 6.0, radius, Color::rgba(255, 255, 255, 128), 1.0, 0.0);
            assert_eq!(
                renderer.buffer().as_slice()[center],
                Color::rgb(128, 128, 128).to_bgra()
            );
            assert_eq!(renderer.backdrop_fallback_count(), 0);
        }
        renderer.buffer.clear(Color::BLACK);
        renderer.paint_backdrop(rect, 6.0, 12.0, Color::rgb(220, 40, 80), 1.0, 0.0);
        assert_eq!(
            renderer.buffer().as_slice()[center],
            Color::rgb(220, 40, 80).to_bgra()
        );
        assert_eq!(
            renderer.backdrop_fallback_count(),
            0,
            "opaque materials need no blur"
        );
        renderer.buffer.clear(Color::BLACK);
        renderer.paint_backdrop(rect, 6.0, 12.0, Color::rgba(220, 40, 80, 60), 0.5, 0.0);
        assert_eq!(
            renderer.buffer().as_slice()[center],
            Color::rgb(110, 20, 40).to_bgra()
        );
        assert_eq!(
            renderer.buffer().as_slice()[4 * 40 + 4],
            Color::BLACK.to_bgra(),
            "fallback still respects rounded coverage"
        );
        assert_eq!(renderer.backdrop_fallback_count(), 1);
    }

    #[test]
    fn cpu_backdrop_invalid_inputs_are_noops_and_huge_bounds_are_clipped() {
        let mut renderer = CpuPaintRenderer::new(Size::new(40.0, 40.0), 1000, Color::BLACK);
        let mut ctx = PaintContext::new();
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            ctx.backdrop_filter_rounded_rect(
                Rect::from_xywh(v, 0.0, 40.0, 40.0),
                4.0,
                9.0,
                Color::WHITE,
            );
            ctx.backdrop_filter_rounded_rect(
                Rect::from_xywh(0.0, 0.0, 40.0, 40.0),
                v,
                9.0,
                Color::WHITE,
            );
            ctx.backdrop_filter_rounded_rect(
                Rect::from_xywh(0.0, 0.0, 40.0, 40.0),
                4.0,
                v,
                Color::WHITE,
            );
            ctx.backdrop_filter_rounded_rect(
                Rect::from_xywh(0.0, 0.0, 40.0, 40.0),
                4.0,
                9.0,
                Color {
                    r: v,
                    ..Color::WHITE
                },
            );
        }
        renderer.execute(&ctx);
        assert!(
            renderer
                .buffer()
                .as_slice()
                .iter()
                .all(|p| *p == Color::BLACK.to_bgra())
        );
        assert_eq!(renderer.backdrop.pixels.len(), 0);
        let mut huge = PaintContext::new();
        huge.backdrop_filter_rounded_rect(
            Rect::from_xywh(-1.0, -1.0, 1.0e20, 1.0e20),
            0.0,
            f32::MAX,
            Color::rgba(255, 255, 255, 128),
        );
        renderer.execute(&huge);
        assert_eq!(renderer.backdrop.pixels.len(), 1600);
        assert!(
            renderer
                .buffer()
                .as_slice()
                .iter()
                .all(|p| *p == Color::rgb(128, 128, 128).to_bgra())
        );
    }

    #[test]
    fn cpu_backdrop_opacity_modulates_the_whole_material() {
        let scene = |opacity| {
            let mut ctx = PaintContext::new();
            ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 20.0, 40.0), Color::WHITE);
            ctx.set_opacity(opacity);
            ctx.backdrop_filter_rounded_rect(
                Rect::from_xywh(0.0, 0.0, 40.0, 40.0),
                0.0,
                9.0,
                Color::TRANSPARENT,
            );
            aa_pixels(&ctx, 1000, Color::BLACK)
        };
        let zero = scene(0.0);
        let half = scene(0.5);
        let full = scene(1.0);
        for i in 0..1600 {
            let a = zero.as_slice()[i].to_le_bytes()[0] as i32;
            let b = full.as_slice()[i].to_le_bytes()[0] as i32;
            let c = half.as_slice()[i].to_le_bytes()[0] as i32;
            assert!((c * 2 - a - b).abs() <= 1);
        }
    }

    #[test]
    fn cpu_huge_shadow_fallback_keeps_partition_clips_disjoint() {
        let shadow = |ctx: &mut PaintContext<'_>| {
            ctx.draw_rounded_rect_shadow(
                Rect::from_xywh(4.0, 4.0, 300_000.0, 30.0),
                5.0,
                Offset::new(0.0, 2.0),
                4.0,
                0.0,
                Color::rgba(0, 0, 0, 100),
            )
        };
        let mut whole = PaintContext::new();
        shadow(&mut whole);
        let expected = aa_pixels(&whole, 1000, Color::WHITE);
        let mut split = PaintContext::new();
        for rect in [
            Rect::from_xywh(0.0, 0.0, 10.25, 40.0),
            Rect::from_xywh(10.25, 0.0, 29.75, 40.0),
        ] {
            split.push_clip(rect);
            shadow(&mut split);
            split.pop_clip();
        }
        let actual = aa_pixels(&split, 1000, Color::WHITE);
        assert_eq!(expected.as_slice(), actual.as_slice());
    }

    #[test]
    fn cpu_shadow_cache_shares_positions_is_bounded_and_resets_at_resize() {
        let mut renderer = CpuPaintRenderer::new(Size::new(120.0, 80.0), 1000, Color::WHITE);
        let shadow = |x: f32, width: f32| {
            let mut ctx = PaintContext::new();
            ctx.draw_rounded_rect_shadow(
                Rect::from_xywh(x, 10.0, width, 20.0),
                5.0,
                Offset::new(0.0, 2.0),
                4.0,
                0.0,
                Color::rgba(0, 0, 0, 80),
            );
            ctx
        };
        renderer.execute(&shadow(10.0, 20.0));
        renderer.execute(&shadow(50.0, 20.0));
        assert_eq!(renderer.shadow_cache.len(), 1);
        renderer.execute(&shadow(50.5, 20.0));
        assert_eq!(renderer.shadow_cache.len(), 2);
        for width in 21..45 {
            renderer.execute(&shadow(10.0, width as f32));
        }
        assert_eq!(renderer.shadow_cache.len(), MAX_SHADOW_CACHE_ENTRIES);
        assert!(
            renderer
                .shadow_cache
                .iter()
                .map(|entry| entry.buffer.as_slice().len())
                .sum::<usize>()
                <= MAX_SHADOW_CACHE_PIXELS
        );
        renderer.resize(Size::new(120.0, 80.0), 2000);
        assert!(renderer.shadow_cache.is_empty());
    }

    #[test]
    fn cpu_gradient_and_shadow_repaint_preserve_damage_and_scratch() {
        for scale in [1000, 2000] {
            let mut ctx = PaintContext::new();
            ctx.draw_rounded_rect_shadow(
                Rect::from_xywh(8.0, 8.0, 20.0, 20.0),
                6.0,
                Offset::new(0.0, 2.0),
                5.0,
                0.0,
                Color::rgba(0, 0, 0, 70),
            );
            ctx.fill_vertical_gradient_rounded_rect(
                Rect::from_xywh(8.0, 8.0, 20.0, 20.0),
                6.0,
                Color::rgba(255, 255, 255, 220),
                Color::rgba(210, 215, 225, 150),
            );
            let mut renderer = CpuPaintRenderer::new(Size::new(40.0, 40.0), scale, Color::WHITE);
            renderer.execute(&ctx);
            let before = renderer.buffer().as_slice().to_vec();
            renderer.execute_with_damage(&ctx, Some(&[Rect::from_xywh(10.25, 5.25, 16.0, 29.0)]));
            assert_eq!(before, renderer.buffer().as_slice());
            let allocations = measure_allocations(|| renderer.execute(&ctx));
            assert_eq!(allocations.allocations, 0);
        }
    }

    #[test]
    fn cpu_shadow_bounds_include_offset_spread_and_blur() {
        let mut ctx = PaintContext::new();
        ctx.draw_rounded_rect_shadow(
            Rect::from_xywh(20.0, 20.0, 30.0, 20.0),
            6.0,
            Offset::new(2.0, 3.0),
            5.0,
            1.0,
            Color::BLACK,
        );
        assert_eq!(
            ctx.raster_bounds(Size::new(100.0, 100.0), 1000),
            Rect::from_xywh(16.0, 17.0, 42.0, 32.0)
        );
    }

    #[test]
    fn aa_blending_keeps_straight_alpha_in_transparent_layers() {
        let white = Buffer::blend_pixels(0, Color::WHITE.to_bgra(), 0.5).to_le_bytes();
        assert_eq!(white, [255, 255, 255, 127]);
        let red = Color::rgba(255, 0, 0, 128).to_bgra();
        let blue = Color::rgba(0, 0, 255, 128).to_bgra();
        let blend = Buffer::blend_pixels(blue, red, 1.0).to_le_bytes();
        assert!((blend[0] as i32 - 84).abs() <= 1);
        assert_eq!(blend[1], 0);
        assert!((blend[2] as i32 - 170).abs() <= 1);
        assert!((blend[3] as i32 - 191).abs() <= 1);
    }

    #[test]
    fn aa_rounded_circle_polygon_and_diagonal_have_partial_edge_coverage() {
        for scale in [1000, 2000] {
            for shape in 0..4 {
                let mut ctx = PaintContext::new();
                match shape {
                    0 => ctx.fill_rounded_rect(
                        Rect::from_xywh(4.0, 4.0, 30.0, 24.0),
                        7.0,
                        Color::WHITE,
                    ),
                    1 => ctx.fill_circle(Point::new(20.0, 20.0), 12.0, Color::WHITE),
                    2 => ctx.fill_triangle(
                        Point::new(4.0, 4.0),
                        Point::new(31.0, 11.0),
                        Point::new(13.0, 32.0),
                        Color::WHITE,
                    ),
                    _ => ctx.draw_line(
                        Point::new(4.0, 7.0),
                        Point::new(30.0, 22.0),
                        1.0,
                        Color::WHITE,
                    ),
                }
                let buffer = aa_pixels(&ctx, scale, Color::BLACK);
                assert!(
                    buffer.as_slice().iter().any(|p| {
                        let v = p & 255;
                        v > 0 && v < 255
                    }),
                    "shape {shape} at {scale}"
                );
            }
        }
    }

    #[test]
    fn aa_integer_rectangles_stay_crisp_and_fractional_rectangles_keep_area() {
        let mut ctx = PaintContext::new();
        ctx.fill_rect(Rect::from_xywh(4.0, 4.0, 10.0, 10.0), Color::WHITE);
        let b = aa_pixels(&ctx, 1000, Color::BLACK);
        assert!(
            b.as_slice()
                .iter()
                .all(|p| *p == Color::BLACK.to_bgra() || *p == Color::WHITE.to_bgra())
        );
        let mut ctx = PaintContext::new();
        ctx.fill_rect(Rect::from_xywh(4.25, 4.5, 10.0, 10.0), Color::WHITE);
        let b = aa_pixels(&ctx, 1000, Color::BLACK);
        assert_eq!(b.get_pixel(4, 6).unwrap() & 255, 191);
        assert_eq!(b.get_pixel(6, 4).unwrap() & 255, 127);
        assert_eq!(b.get_pixel(14, 6).unwrap() & 255, 63);
    }

    #[test]
    fn aa_fractional_borders_keep_width_and_empty_center() {
        for (width, expected) in [(0.0, 0), (0.5, 127), (0.75, 191), (1.0, 255)] {
            let mut ctx = PaintContext::new();
            ctx.stroke_rounded_rect(
                Rect::from_xywh(4.0, 4.0, 24.0, 24.0),
                6.0,
                width,
                Color::WHITE,
            );
            let b = aa_pixels(&ctx, 1000, Color::BLACK);
            assert_eq!(b.get_pixel(4, 16).unwrap() & 255, expected);
            assert_eq!(b.get_pixel(16, 16).unwrap(), Color::BLACK.to_bgra());
            assert_eq!(b.get_pixel(3, 16).unwrap(), Color::BLACK.to_bgra());
        }
    }

    #[test]
    fn aa_rounded_fill_and_clip_are_symmetric() {
        for scale in [1000, 2000] {
            for clip in [false, true] {
                let mut ctx = PaintContext::new();
                let rect = Rect::from_xywh(4.0, 4.0, 24.0, 24.0);
                if clip {
                    ctx.push_rounded_clip(rect, 8.0);
                    ctx.fill_rect(rect, Color::WHITE);
                    ctx.pop_clip();
                } else {
                    ctx.fill_rounded_rect(rect, 8.0, Color::WHITE);
                }
                let b = aa_pixels(&ctx, scale, Color::BLACK);
                let k = scale / 1000;
                for y in 4 * k..28 * k {
                    for x in 4 * k..28 * k {
                        let a = (b.get_pixel(x, y).unwrap() & 255) as i32;
                        let h = (b.get_pixel(32 * k - 1 - x, y).unwrap() & 255) as i32;
                        let v = (b.get_pixel(x, 32 * k - 1 - y).unwrap() & 255) as i32;
                        assert!(
                            (a - h).abs() <= 1 && (a - v).abs() <= 1,
                            "{x},{y}: {a},{h},{v}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn aa_rounded_layer_does_not_dark_fringe_white_shape() {
        for scale in [1000, 2000] {
            let mut plain = PaintContext::new();
            plain.fill_circle(Point::new(20.0, 20.0), 8.0, Color::WHITE);
            let expected = aa_pixels(&plain, scale, Color::BLACK);
            let mut clipped = PaintContext::new();
            clipped.push_rounded_clip(Rect::from_xywh(0.0, 0.0, 40.0, 40.0), 3.0);
            clipped.fill_circle(Point::new(20.0, 20.0), 8.0, Color::WHITE);
            clipped.pop_clip();
            let actual = aa_pixels(&clipped, scale, Color::BLACK);
            for (index, (a, b)) in expected
                .as_slice()
                .iter()
                .zip(actual.as_slice())
                .enumerate()
            {
                assert_eq!(a, b, "pixel {index} at {scale}");
            }
        }
    }

    #[test]
    fn aa_fractional_rounded_clip_applies_edge_mask_once() {
        let mut ctx = PaintContext::new();
        ctx.push_rounded_clip(Rect::from_xywh(4.25, 4.0, 24.0, 24.0), 4.0);
        ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 40.0, 40.0), Color::WHITE);
        ctx.pop_clip();
        let b = aa_pixels(&ctx, 1000, Color::BLACK);
        assert_eq!(b.get_pixel(4, 16).unwrap() & 255, 191);
    }

    #[test]
    fn aa_fractional_damage_repaint_matches_full_frame() {
        for scale in [1000, 1250, 2000] {
            let mut ctx = PaintContext::new();
            ctx.fill_rounded_rect(
                Rect::from_xywh(2.25, 2.5, 32.0, 30.0),
                8.0,
                Color::rgba(230, 90, 50, 180),
            );
            let mut renderer = CpuPaintRenderer::new(Size::new(40.0, 40.0), scale, Color::BLACK);
            renderer.execute(&ctx);
            let before = renderer.buffer().as_slice().to_vec();
            renderer.execute_with_damage(&ctx, Some(&[Rect::from_xywh(8.25, 3.25, 13.0, 22.5)]));
            assert_eq!(before, renderer.buffer().as_slice());
        }
    }

    #[test]
    fn aa_rounded_contours_reuse_coverage_scratch_without_allocating() {
        let mut ctx = PaintContext::new();
        ctx.fill_rounded_rect(Rect::from_xywh(4.25, 4.5, 28.0, 24.0), 8.0, Color::WHITE);
        ctx.stroke_rounded_rect(
            Rect::from_xywh(4.25, 4.5, 28.0, 24.0),
            8.0,
            0.75,
            Color::BLACK,
        );
        let mut renderer = CpuPaintRenderer::new(Size::new(40.0, 40.0), 2000, Color::BLACK);
        renderer.execute(&ctx);
        let capacity = renderer.scratch_capacity_for_test();
        let snapshot = measure_allocations(|| renderer.execute(&ctx));
        assert_eq!(snapshot.allocations, 0);
        assert_eq!(capacity, renderer.scratch_capacity_for_test());
    }

    #[test]
    fn append_at_translates_nested_geometry_text_and_remaps_buffers() {
        let mut local = PaintContext::new();
        local.draw_text(Point::new(1.25, 2.75), "Menu offset", Color::BLACK, 13.0);
        local.stroke_rect(Rect::from_xywh(1.0, 2.0, 8.0, 9.0), 1.0, Color::BLACK);
        local.draw_temporary_buffer(
            Rect::from_xywh(2.0, 3.0, 4.0, 5.0),
            Buffer::new(Size::new(4.0, 5.0)),
        );
        let mut nested = PaintContext::new();
        nested.draw_display_list(Point::new(3.0, 4.0), local.into_display_list());
        let mut output = PaintContext::new();
        output.draw_temporary_buffer(
            Rect::from_xywh(0.0, 0.0, 1.0, 1.0),
            Buffer::new(Size::new(1.0, 1.0)),
        );
        output.append_at(&nested, Point::new(10.0, 20.0));
        assert!(
            matches!(&output.commands()[1], PaintCommand::DrawText { position, .. } if *position == Point::new(14.25, 26.75))
        );
        assert!(
            matches!(&output.commands()[2], PaintCommand::StrokeRect { rect, .. } if rect.origin == Point::new(14.0, 26.0))
        );
        assert!(
            matches!(&output.commands()[3], PaintCommand::DrawBuffer { dst, buffer_idx } if dst.origin == Point::new(15.0, 27.0) && *buffer_idx == 1)
        );
        assert_eq!(output.buffers().len(), 2);
        assert_eq!(output.buffer(BufferHandle(1)).unwrap().width(), 4);
    }

    #[test]
    fn grouped_display_lists_preserve_shared_image_identity() {
        let buffer = Arc::new(Buffer::new(Size::new(32., 32.)));
        let mut local = PaintContext::new();
        let rect = Rect::from_xywh(0., 0., 32., 32.);
        local.draw_buffer_rect_shared(rect, rect, Arc::clone(&buffer), 1.);
        let mut group = PaintContext::new();
        group.draw_display_list(Point::new(10., 20.), local.into_display_list());
        let recording = group.flattened().into_display_list();
        assert!(
            matches!(&recording.context().buffers()[0],PaintBuffer::Shared(shared) if Arc::ptr_eq(shared,&buffer)),
            "grouping must share image pixels and resource identity, rather than clone a borrowed snapshot again"
        );
    }
    #[test]
    fn path_helpers() {
        let r = path_rect(Rect::new(Point::new(0.0, 0.0), Size::new(10.0, 20.0)));
        assert_eq!(r.len(), 4);

        let c = path_circle(Point::new(50.0, 50.0), 10.0);
        assert_eq!(c.len(), 48);

        let t = path_triangle(Point::ZERO, Point::new(10.0, 0.0), Point::new(0.0, 10.0));
        assert_eq!(t.len(), 3);

        let rr = path_rounded_rect(Rect::new(Point::new(0.0, 0.0), Size::new(40.0, 40.0)), 8.0);
        assert_eq!(rr.len(), 36);
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
    fn paint_opacity_modulates_solid_commands() {
        let mut ctx = PaintContext::new();
        ctx.set_opacity(0.5);
        ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 4.0, 4.0), Color::rgb(255, 0, 0));
        let mut renderer = CpuPaintRenderer::new(Size::new(8.0, 8.0), 1000, Color::BLACK);
        renderer.execute(&ctx);

        let pixel = Color::from_bgra(renderer.buffer().get_pixel(1, 1).unwrap());
        assert!((pixel.r - 0.5).abs() < 0.01);
        assert!(pixel.g < 0.01);
        assert!(pixel.b < 0.01);
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
    fn vertical_gradient_records_semantic_endpoints() {
        let rect = Rect::from_xywh(4.0, 6.0, 80.0, 28.0);
        let mut ctx = PaintContext::new();
        ctx.fill_vertical_gradient_rounded_rect(rect, 6.0, Color::WHITE, Color::BLACK);

        let [
            PaintCommand::FillVerticalGradientRoundedRect {
                rect: recorded,
                corner_radius,
                top_color,
                bottom_color,
            },
        ] = ctx.commands()
        else {
            panic!("expected one vertical gradient command");
        };
        assert_eq!(*recorded, rect);
        assert_eq!(*corner_radius, 6.0);
        assert_eq!(*top_color, Color::WHITE);
        assert_eq!(*bottom_color, Color::BLACK);
    }

    #[test]
    fn cpu_gradient_interpolates_native_rows() {
        let mut ctx = PaintContext::new();
        ctx.fill_vertical_gradient_rounded_rect(
            Rect::from_xywh(2.0, 2.0, 20.0, 20.0),
            4.0,
            Color::WHITE,
            Color::BLACK,
        );
        let mut renderer = CpuPaintRenderer::new(Size::new(24.0, 24.0), 1000, Color::TRANSPARENT);
        renderer.execute(&ctx);

        let pixel = Color::from_bgra(renderer.buffer().get_pixel(12, 12).unwrap());
        assert!((pixel.r - 0.475).abs() < 0.01);
        assert!((pixel.g - 0.475).abs() < 0.01);
        assert!((pixel.b - 0.475).abs() < 0.01);
        let top = Color::from_bgra(renderer.buffer().get_pixel(12, 2).unwrap());
        let bottom = Color::from_bgra(renderer.buffer().get_pixel(12, 21).unwrap());
        assert!(top.r > 0.95 && bottom.r < 0.05);
    }

    #[test]
    fn cpu_shadow_has_offset_and_soft_falloff() {
        let background = Color::rgb(24, 32, 40);
        let mut ctx = PaintContext::new();
        ctx.draw_rounded_rect_shadow(
            Rect::from_xywh(8.0, 8.0, 20.0, 20.0),
            6.0,
            Offset::new(0.0, 3.0),
            8.0,
            0.0,
            Color::rgba(0, 0, 0, 64),
        );
        let mut renderer = CpuPaintRenderer::new(Size::new(40.0, 40.0), 1000, background);
        renderer.execute(&ctx);

        let near = renderer.buffer().get_pixel(20, 32).unwrap() & 255;
        let far = renderer.buffer().get_pixel(20, 37).unwrap() & 255;
        assert!(near < far && far < (background.to_bgra() & 255));
        assert_eq!(
            renderer.buffer().get_pixel(39, 39).unwrap(),
            background.to_bgra()
        );
    }

    #[test]
    fn cpu_paint_renderer_reuses_scaled_point_scratch() {
        let mut ctx = PaintContext::new();
        ctx.fill_circle(Point::new(50.0, 50.0), 20.0, Color::rgb(0, 255, 0));
        let mut renderer =
            CpuPaintRenderer::new(Size::new(100.0, 100.0), 1000, Color::rgb(0, 0, 0));

        renderer.execute(&ctx);
        let first = renderer.scratch_capacity_for_test();
        assert!(first.scaled_points >= 48);

        renderer.execute(&ctx);
        let second = renderer.scratch_capacity_for_test();
        assert_eq!(first.scaled_points, second.scaled_points);
    }

    #[test]
    fn cpu_paint_renderer_reuses_scanline_crossings_scratch() {
        let mut ctx = PaintContext::new();
        ctx.fill_path(
            path_rounded_rect(Rect::from_xywh(10.0, 10.0, 80.0, 80.0), 10.0),
            Color::rgb(255, 255, 0),
        );
        let mut renderer =
            CpuPaintRenderer::new(Size::new(100.0, 100.0), 1000, Color::rgb(0, 0, 0));

        renderer.execute(&ctx);
        let first = renderer.scratch_capacity_for_test();
        assert!(first.crossings > 0);

        let snapshot = measure_allocations(|| {
            renderer.execute(&ctx);
        });
        let second = renderer.scratch_capacity_for_test();

        assert_eq!(first.crossings, second.crossings);
        assert_eq!(first.coverage, second.coverage);
        assert_eq!(snapshot.allocations, 0);
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
    fn rectangular_clip_does_not_allocate_layer_buffer() {
        let bg = Color::rgb(0, 0, 0);
        let fill = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.push_clip(Rect::from_xywh(10.0, 10.0, 20.0, 20.0));
        ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 50.0, 50.0), fill);
        ctx.pop_clip();

        let mut r = CpuPaintRenderer::new(Size::new(50.0, 50.0), 1000, bg);
        r.execute(&ctx);

        assert_eq!(r.retained_layer_count(), 0);
    }

    #[test]
    fn rounded_clip_layer_buffer_is_reused() {
        let bg = Color::rgb(0, 0, 0);
        let fill = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.push_rounded_clip(Rect::from_xywh(10.0, 10.0, 20.0, 20.0), 8.0);
        ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 50.0, 50.0), fill);
        ctx.pop_clip();

        let mut r = CpuPaintRenderer::new(Size::new(50.0, 50.0), 1000, bg);
        r.execute(&ctx);
        assert_eq!(r.retained_layer_count(), 1);

        r.execute(&ctx);
        assert_eq!(r.retained_layer_count(), 1);
    }

    #[test]
    fn rounded_clip_layer_buffer_matches_target_size() {
        let fill = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.push_rounded_clip(Rect::from_xywh(0.0, 0.0, 50.0, 100.0), 8.0);
        ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 50.0, 100.0), fill);
        ctx.pop_clip();

        let mut renderer = CpuPaintRenderer::new(Size::new(50.0, 50.0), 1000, Color::CLEAR);
        let mut first = Buffer::from_logical_dimensions(50, 50);
        renderer.execute_into_external_buffer(&mut first, Color::CLEAR, &ctx, None);

        let mut expanded = Buffer::from_logical_dimensions(50, 100);
        renderer.execute_into_external_buffer(&mut expanded, Color::CLEAR, &ctx, None);

        assert_eq!(expanded.get_pixel(25, 75), Some(fill.to_bgra()));
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
    fn three_nested_rectangular_clips_intersect_without_layer_buffer() {
        let bg = Color::rgb(0, 0, 0);
        let fill = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.push_clip(Rect::from_xywh(5.0, 5.0, 40.0, 40.0));
        ctx.push_clip(Rect::from_xywh(15.0, 0.0, 30.0, 50.0));
        ctx.push_clip(Rect::from_xywh(0.0, 20.0, 50.0, 15.0));
        ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 60.0, 60.0), fill);
        ctx.pop_clip();
        ctx.pop_clip();
        ctx.pop_clip();

        let mut r = CpuPaintRenderer::new(Size::new(60.0, 60.0), 1000, bg);
        r.execute(&ctx);

        let bg_pixel = bg.to_bgra();
        assert_eq!(r.buffer().get_pixel(20, 25).unwrap(), fill.to_bgra());
        assert_eq!(r.buffer().get_pixel(10, 25).unwrap(), bg_pixel);
        assert_eq!(r.buffer().get_pixel(20, 15).unwrap(), bg_pixel);
        assert_eq!(r.buffer().get_pixel(48, 25).unwrap(), bg_pixel);
        assert_eq!(r.buffer().get_pixel(20, 38).unwrap(), bg_pixel);
        assert_eq!(r.retained_layer_count(), 0);
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
    fn draw_buffer_rect_respects_destination_size_and_source_dpi() {
        let mut image = Buffer::from_dimensions(2, 1);
        image.set_pixel(0, 0, Color::RED.to_bgra());
        image.set_pixel(1, 0, Color::BLUE.to_bgra());
        let mut ctx = PaintContext::new();
        ctx.draw_buffer_rect_ref(
            Rect::from_xywh(0.0, 0.0, 4.0, 2.0),
            Rect::from_xywh(0.0, 0.0, 2.0, 1.0),
            &image,
            1.0,
        );
        let mut renderer = CpuPaintRenderer::new(Size::new(4.0, 2.0), 2000, Color::WHITE);
        renderer.execute(&ctx);
        for y in 0..4 {
            for x in 0..8 {
                assert_eq!(
                    renderer.buffer().get_pixel(x, y),
                    Some(if x < 4 { Color::RED } else { Color::BLUE }.to_bgra())
                );
            }
        }
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
    fn stroke_path_honors_stroke_width() {
        let bg = Color::rgb(0, 0, 0);
        let stroke = Color::rgb(255, 0, 0);
        let mut ctx = PaintContext::new();
        ctx.draw_line(Point::new(2.0, 5.0), Point::new(8.0, 5.0), 4.0, stroke);
        let mut r = CpuPaintRenderer::new(Size::new(20.0, 20.0), 1000, bg);
        r.execute(&ctx);

        let bg_pixel = bg.to_bgra();
        let stroke_pixel = stroke.to_bgra();
        assert_eq!(r.buffer().get_pixel(5, 2).unwrap(), bg_pixel);
        assert_eq!(r.buffer().get_pixel(5, 3).unwrap(), stroke_pixel);
        assert_eq!(r.buffer().get_pixel(5, 4).unwrap(), stroke_pixel);
        assert_eq!(r.buffer().get_pixel(5, 5).unwrap(), stroke_pixel);
        assert_eq!(r.buffer().get_pixel(5, 6).unwrap(), stroke_pixel);
        assert_eq!(r.buffer().get_pixel(5, 7).unwrap(), bg_pixel);
    }

    #[test]
    fn stroke_path_scales_width_with_scale_factor() {
        let bg = Color::rgb(0, 0, 0);
        let stroke = Color::rgb(255, 0, 0);

        let mut ctx = PaintContext::new();
        ctx.draw_line(Point::new(2.0, 5.0), Point::new(8.0, 5.0), 1.0, stroke);
        let mut r = CpuPaintRenderer::new(Size::new(40.0, 40.0), 2000, bg);
        r.execute(&ctx);

        let bg_pixel = bg.to_bgra();
        let stroke_pixel = stroke.to_bgra();
        assert_eq!(r.buffer().get_pixel(10, 8).unwrap(), bg_pixel);
        assert_eq!(r.buffer().get_pixel(10, 9).unwrap(), stroke_pixel);
        assert_eq!(r.buffer().get_pixel(10, 10).unwrap(), stroke_pixel);
        assert_eq!(r.buffer().get_pixel(10, 11).unwrap(), bg_pixel);
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
    #[ignore = "Manual visual inspection: writes scarlet_paint_test.ppm in the temp directory"]
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
