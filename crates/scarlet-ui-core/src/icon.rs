//! Typed icons, sizing, styling, and renderer-independent mask caching.
//!
//! Icon geometry comes from the separate `scarlet-ui-icons-tabler` asset
//! crate. This module owns the app-facing API and converts vector commands to
//! reusable alpha masks without baking theme colors into the cache.

use crate::geometry::Point;
use crate::os::Mutex;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use scarlet_ui_icons_tabler::{IconCommand, filled_icon_commands, icon_commands};

pub use scarlet_ui_icons_tabler::{ALL_ICONS, Icon};

const TABLER_VIEWBOX_SIZE: f32 = 24.0;
const DEFAULT_STROKE_WIDTH_MILLI: u16 = 1_500;
const MAX_CACHED_MASKS: usize = 512;
const SUPERSAMPLE_GRID: u32 = 4;

/// Semantic stroke weights for outline icons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IconWeight {
    /// Light 1.0-unit strokes.
    Thin,
    /// Standard 1.5-unit strokes.
    #[default]
    Normal,
    /// Emphasized 2.0-unit strokes.
    Bold,
}

impl IconWeight {
    /// Return this weight's stroke width in Tabler view-box units.
    ///
    /// # Returns
    ///
    /// `1.0`, `1.5`, or `2.0` for thin, normal, and bold respectively.
    pub const fn stroke_width(self) -> f32 {
        match self {
            Self::Thin => 1.0,
            Self::Normal => 1.5,
            Self::Bold => 2.0,
        }
    }
}

/// Vector treatment selected for an icon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IconFill {
    /// Draw the icon as stroked paths.
    #[default]
    Outline,
    /// Use the official Tabler filled vector when one exists.
    Filled,
}

/// Standard logical sizes for Scarlet UI icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IconSize {
    /// Compact controls and dense rows, 16 logical pixels.
    Small,
    /// Standard controls and header bars, 20 logical pixels.
    Medium,
    /// Prominent controls, 32 logical pixels.
    Large,
    /// File grids and large affordances, 48 logical pixels.
    ExtraLarge,
    /// Explicit logical pixel size.
    Pixels(u16),
}

impl IconSize {
    /// Return the logical pixel size represented by this value.
    ///
    /// # Returns
    ///
    /// The square side length in logical pixels.
    pub const fn logical_pixels(self) -> u16 {
        match self {
            Self::Small => 16,
            Self::Medium => 20,
            Self::Large => 32,
            Self::ExtraLarge => 48,
            Self::Pixels(pixels) => pixels,
        }
    }
}

impl Default for IconSize {
    fn default() -> Self {
        Self::Medium
    }
}

/// Rendering style for a Tabler icon.
///
/// Stroke width is stored in thousandths of a Tabler view-box unit so style
/// values remain deterministic cache keys without floating-point equality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IconStyle {
    fill: IconFill,
    stroke_width_milli: u16,
}

impl IconStyle {
    /// Create the standard Tabler outline style.
    ///
    /// # Returns
    ///
    /// A style with Scarlet UI's 1.5-unit default stroke width.
    pub const fn outline() -> Self {
        Self {
            fill: IconFill::Outline,
            stroke_width_milli: DEFAULT_STROKE_WIDTH_MILLI,
        }
    }

    /// Create the standard Tabler filled style.
    ///
    /// Icons without an official filled vector fall back to the normal outline.
    ///
    /// # Returns
    ///
    /// A filled style with normal fallback stroke weight.
    pub const fn filled() -> Self {
        Self {
            fill: IconFill::Filled,
            stroke_width_milli: DEFAULT_STROKE_WIDTH_MILLI,
        }
    }

    /// Select outline or filled vector treatment.
    ///
    /// # Arguments
    ///
    /// * `fill` - Requested vector treatment.
    ///
    /// # Returns
    ///
    /// The updated style.
    pub const fn fill(mut self, fill: IconFill) -> Self {
        self.fill = fill;
        self
    }

    /// Select a semantic outline stroke weight.
    ///
    /// The weight remains available as the fallback when a requested filled
    /// variant does not exist upstream.
    ///
    /// # Arguments
    ///
    /// * `weight` - Thin, normal, or bold stroke weight.
    ///
    /// # Returns
    ///
    /// The updated style.
    pub fn weight(self, weight: IconWeight) -> Self {
        self.stroke_width(weight.stroke_width())
    }

    /// Set the stroke width in Tabler's 24×24 coordinate system.
    ///
    /// Values are clamped to 0.25–6.0 units.
    ///
    /// # Arguments
    ///
    /// * `width` - Requested stroke width in view-box units.
    ///
    /// # Returns
    ///
    /// The updated style.
    pub fn stroke_width(mut self, width: f32) -> Self {
        let width = if width.is_finite() { width } else { 1.5 };
        self.stroke_width_milli = libm::roundf(width.clamp(0.25, 6.0) * 1_000.0) as u16;
        self
    }

    /// Return the stroke width in Tabler view-box units.
    ///
    /// # Returns
    ///
    /// Stroke width as a floating-point value.
    pub fn get_stroke_width(self) -> f32 {
        self.stroke_width_milli as f32 / 1_000.0
    }

    /// Return the selected vector treatment.
    ///
    /// # Returns
    ///
    /// Outline or filled treatment.
    pub const fn get_fill(self) -> IconFill {
        self.fill
    }
}

impl Default for IconStyle {
    fn default() -> Self {
        Self::outline()
    }
}

/// Stable key for a rasterized icon mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IconMaskKey {
    /// Selected typed icon.
    pub icon: Icon,
    /// Physical square side length in pixels.
    pub pixel_size: u16,
    /// Vector and stroke style used to rasterize the mask.
    pub style: IconStyle,
}

/// Cached alpha-only icon raster.
#[derive(Clone, Debug)]
pub struct RasterizedIcon {
    /// Cache identity for this raster.
    pub key: IconMaskKey,
    /// Mask width in pixels.
    pub width: u32,
    /// Mask height in pixels.
    pub height: u32,
    /// One 8-bit alpha value per pixel, row-major.
    pub mask: Arc<[u8]>,
}

static ICON_MASK_CACHE: Mutex<BTreeMap<IconMaskKey, RasterizedIcon>> = Mutex::new(BTreeMap::new());

/// Rasterize or retrieve an alpha mask for a typed icon.
///
/// Theme color is deliberately excluded from the key and result. Renderers
/// tint the returned alpha mask at composition time, so theme changes do not
/// invalidate vector rasterization.
///
/// # Arguments
///
/// * `icon` - Selected Tabler icon.
/// * `pixel_size` - Physical square side length in pixels.
/// * `style` - Vector treatment and fallback stroke width.
///
/// # Returns
///
/// A shared cached alpha mask.
pub fn rasterize_icon(icon: Icon, pixel_size: u16, style: IconStyle) -> RasterizedIcon {
    let key = IconMaskKey {
        icon,
        pixel_size: pixel_size.clamp(1, 1_024),
        style,
    };
    if let Some(cached) = ICON_MASK_CACHE.lock().get(&key).cloned() {
        return cached;
    }

    let raster = rasterize_uncached(key);
    let mut cache = ICON_MASK_CACHE.lock();
    if cache.len() >= MAX_CACHED_MASKS {
        cache.clear();
    }
    cache.insert(key, raster.clone());
    raster
}

#[derive(Clone, Copy)]
struct Segment {
    from: Point,
    to: Point,
}

fn rasterize_uncached(key: IconMaskKey) -> RasterizedIcon {
    if key.style.get_fill() == IconFill::Filled {
        if let Some(commands) = filled_icon_commands(key.icon) {
            return rasterize_filled(key, commands);
        }
    }

    rasterize_outline(key)
}

fn rasterize_outline(key: IconMaskKey) -> RasterizedIcon {
    let size = u32::from(key.pixel_size);
    let segments = flatten_commands(icon_commands(key.icon), key.pixel_size, false);
    let stroke_radius = key.style.get_stroke_width() * 0.5;
    let sample_count = SUPERSAMPLE_GRID * SUPERSAMPLE_GRID;
    let mut mask = Vec::new();
    mask.resize((size * size) as usize, 0);

    for y in 0..size {
        for x in 0..size {
            let mut covered = 0;
            for sample_y in 0..SUPERSAMPLE_GRID {
                for sample_x in 0..SUPERSAMPLE_GRID {
                    let sample = Point::new(
                        (x as f32 + (sample_x as f32 + 0.5) / SUPERSAMPLE_GRID as f32)
                            * TABLER_VIEWBOX_SIZE
                            / size as f32,
                        (y as f32 + (sample_y as f32 + 0.5) / SUPERSAMPLE_GRID as f32)
                            * TABLER_VIEWBOX_SIZE
                            / size as f32,
                    );
                    if segments
                        .iter()
                        .any(|segment| distance_to_segment(sample, *segment) <= stroke_radius)
                    {
                        covered += 1;
                    }
                }
            }
            mask[(y * size + x) as usize] =
                ((covered * 255 + sample_count / 2) / sample_count) as u8;
        }
    }

    RasterizedIcon {
        key,
        width: size,
        height: size,
        mask: Arc::from(mask),
    }
}

fn rasterize_filled(key: IconMaskKey, commands: &[IconCommand]) -> RasterizedIcon {
    let size = u32::from(key.pixel_size);
    let paths = flatten_filled_paths(commands, key.pixel_size);
    let sample_count = SUPERSAMPLE_GRID * SUPERSAMPLE_GRID;
    let mut mask = Vec::new();
    mask.resize((size * size) as usize, 0);

    for y in 0..size {
        for x in 0..size {
            let mut covered = 0;
            for sample_y in 0..SUPERSAMPLE_GRID {
                for sample_x in 0..SUPERSAMPLE_GRID {
                    let sample = Point::new(
                        (x as f32 + (sample_x as f32 + 0.5) / SUPERSAMPLE_GRID as f32)
                            * TABLER_VIEWBOX_SIZE
                            / size as f32,
                        (y as f32 + (sample_y as f32 + 0.5) / SUPERSAMPLE_GRID as f32)
                            * TABLER_VIEWBOX_SIZE
                            / size as f32,
                    );
                    if paths
                        .iter()
                        .any(|segments| winding_number(sample, segments) != 0)
                    {
                        covered += 1;
                    }
                }
            }
            mask[(y * size + x) as usize] =
                ((covered * 255 + sample_count / 2) / sample_count) as u8;
        }
    }

    RasterizedIcon {
        key,
        width: size,
        height: size,
        mask: Arc::from(mask),
    }
}

fn flatten_filled_paths(commands: &[IconCommand], pixel_size: u16) -> Vec<Vec<Segment>> {
    let mut paths = Vec::new();
    let mut start = 0;
    for (index, command) in commands.iter().enumerate() {
        if *command == IconCommand::EndPath {
            if start < index {
                paths.push(flatten_commands(&commands[start..index], pixel_size, true));
            }
            start = index + 1;
        }
    }
    if start < commands.len() {
        paths.push(flatten_commands(&commands[start..], pixel_size, true));
    }
    paths
}

fn flatten_commands(
    commands: &[IconCommand],
    pixel_size: u16,
    close_open_subpaths: bool,
) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut current = Point::ZERO;
    let mut subpath_start = Point::ZERO;
    let mut subpath_has_segments = false;
    let curve_steps = (usize::from(pixel_size) / 2).clamp(8, 48);

    for command in commands {
        match *command {
            IconCommand::MoveTo(x, y) => {
                if close_open_subpaths && subpath_has_segments && current != subpath_start {
                    segments.push(Segment {
                        from: current,
                        to: subpath_start,
                    });
                }
                current = Point::new(x, y);
                subpath_start = current;
                subpath_has_segments = false;
            }
            IconCommand::LineTo(x, y) => {
                let next = Point::new(x, y);
                segments.push(Segment {
                    from: current,
                    to: next,
                });
                current = next;
                subpath_has_segments = true;
            }
            IconCommand::QuadTo {
                control_x,
                control_y,
                x,
                y,
            } => {
                let start = current;
                let control = Point::new(control_x, control_y);
                let end = Point::new(x, y);
                append_curve(&mut segments, start, curve_steps, |t| {
                    let inverse = 1.0 - t;
                    Point::new(
                        inverse * inverse * start.x + 2.0 * inverse * t * control.x + t * t * end.x,
                        inverse * inverse * start.y + 2.0 * inverse * t * control.y + t * t * end.y,
                    )
                });
                current = end;
                subpath_has_segments = true;
            }
            IconCommand::CubicTo {
                control_1_x,
                control_1_y,
                control_2_x,
                control_2_y,
                x,
                y,
            } => {
                let start = current;
                let control_1 = Point::new(control_1_x, control_1_y);
                let control_2 = Point::new(control_2_x, control_2_y);
                let end = Point::new(x, y);
                append_curve(&mut segments, start, curve_steps, |t| {
                    let inverse = 1.0 - t;
                    let inverse_2 = inverse * inverse;
                    let t_2 = t * t;
                    Point::new(
                        inverse_2 * inverse * start.x
                            + 3.0 * inverse_2 * t * control_1.x
                            + 3.0 * inverse * t_2 * control_2.x
                            + t_2 * t * end.x,
                        inverse_2 * inverse * start.y
                            + 3.0 * inverse_2 * t * control_1.y
                            + 3.0 * inverse * t_2 * control_2.y
                            + t_2 * t * end.y,
                    )
                });
                current = end;
                subpath_has_segments = true;
            }
            IconCommand::ArcTo {
                radius_x,
                radius_y,
                rotation,
                large_arc,
                sweep,
                x,
                y,
            } => {
                let end = Point::new(x, y);
                append_arc(
                    &mut segments,
                    current,
                    end,
                    radius_x,
                    radius_y,
                    rotation,
                    large_arc,
                    sweep,
                    curve_steps,
                );
                current = end;
                subpath_has_segments = true;
            }
            IconCommand::Close => {
                if current != subpath_start {
                    segments.push(Segment {
                        from: current,
                        to: subpath_start,
                    });
                }
                current = subpath_start;
                subpath_has_segments = false;
            }
            IconCommand::EndPath => {
                if close_open_subpaths && subpath_has_segments && current != subpath_start {
                    segments.push(Segment {
                        from: current,
                        to: subpath_start,
                    });
                }
                subpath_has_segments = false;
            }
        }
    }
    if close_open_subpaths && subpath_has_segments && current != subpath_start {
        segments.push(Segment {
            from: current,
            to: subpath_start,
        });
    }
    segments
}

fn winding_number(point: Point, segments: &[Segment]) -> i32 {
    let mut winding = 0;
    for segment in segments {
        if segment.from.y <= point.y {
            if segment.to.y > point.y && is_left(*segment, point) > 0.0 {
                winding += 1;
            }
        } else if segment.to.y <= point.y && is_left(*segment, point) < 0.0 {
            winding -= 1;
        }
    }
    winding
}

fn is_left(segment: Segment, point: Point) -> f32 {
    (segment.to.x - segment.from.x) * (point.y - segment.from.y)
        - (point.x - segment.from.x) * (segment.to.y - segment.from.y)
}

fn append_curve(
    segments: &mut Vec<Segment>,
    start: Point,
    steps: usize,
    point_at: impl Fn(f32) -> Point,
) {
    let mut previous = start;
    for step in 1..=steps {
        let next = point_at(step as f32 / steps as f32);
        segments.push(Segment {
            from: previous,
            to: next,
        });
        previous = next;
    }
}

#[allow(clippy::too_many_arguments)]
fn append_arc(
    segments: &mut Vec<Segment>,
    start: Point,
    end: Point,
    radius_x: f32,
    radius_y: f32,
    rotation: f32,
    large_arc: bool,
    sweep: bool,
    minimum_steps: usize,
) {
    let mut radius_x = radius_x.abs();
    let mut radius_y = radius_y.abs();
    if radius_x <= f32::EPSILON || radius_y <= f32::EPSILON || start == end {
        segments.push(Segment {
            from: start,
            to: end,
        });
        return;
    }

    let phi = rotation * core::f32::consts::PI / 180.0;
    let cosine = libm::cosf(phi);
    let sine = libm::sinf(phi);
    let half_x = (start.x - end.x) * 0.5;
    let half_y = (start.y - end.y) * 0.5;
    let transformed_x = cosine * half_x + sine * half_y;
    let transformed_y = -sine * half_x + cosine * half_y;
    let lambda = transformed_x * transformed_x / (radius_x * radius_x)
        + transformed_y * transformed_y / (radius_y * radius_y);
    if lambda > 1.0 {
        let scale = libm::sqrtf(lambda);
        radius_x *= scale;
        radius_y *= scale;
    }

    let numerator = (radius_x * radius_x * radius_y * radius_y
        - radius_x * radius_x * transformed_y * transformed_y
        - radius_y * radius_y * transformed_x * transformed_x)
        .max(0.0);
    let denominator = radius_x * radius_x * transformed_y * transformed_y
        + radius_y * radius_y * transformed_x * transformed_x;
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let factor = if denominator <= f32::EPSILON {
        0.0
    } else {
        sign * libm::sqrtf(numerator / denominator)
    };
    let center_transformed_x = factor * radius_x * transformed_y / radius_y;
    let center_transformed_y = factor * -radius_y * transformed_x / radius_x;
    let center = Point::new(
        cosine * center_transformed_x - sine * center_transformed_y + (start.x + end.x) * 0.5,
        sine * center_transformed_x + cosine * center_transformed_y + (start.y + end.y) * 0.5,
    );

    let start_vector = (
        (transformed_x - center_transformed_x) / radius_x,
        (transformed_y - center_transformed_y) / radius_y,
    );
    let end_vector = (
        (-transformed_x - center_transformed_x) / radius_x,
        (-transformed_y - center_transformed_y) / radius_y,
    );
    let start_angle = vector_angle((1.0, 0.0), start_vector);
    let mut sweep_angle = vector_angle(start_vector, end_vector);
    if !sweep && sweep_angle > 0.0 {
        sweep_angle -= core::f32::consts::TAU;
    } else if sweep && sweep_angle < 0.0 {
        sweep_angle += core::f32::consts::TAU;
    }
    let steps =
        (libm::ceilf(sweep_angle.abs() * radius_x.max(radius_y)) as usize).clamp(minimum_steps, 96);
    let mut previous = start;
    for step in 1..=steps {
        let angle = start_angle + sweep_angle * step as f32 / steps as f32;
        let arc_x = radius_x * libm::cosf(angle);
        let arc_y = radius_y * libm::sinf(angle);
        let next = Point::new(
            center.x + cosine * arc_x - sine * arc_y,
            center.y + sine * arc_x + cosine * arc_y,
        );
        segments.push(Segment {
            from: previous,
            to: next,
        });
        previous = next;
    }
}

fn vector_angle(left: (f32, f32), right: (f32, f32)) -> f32 {
    libm::atan2f(
        left.0 * right.1 - left.1 * right.0,
        left.0 * right.0 + left.1 * right.1,
    )
}

fn distance_to_segment(point: Point, segment: Segment) -> f32 {
    let delta_x = segment.to.x - segment.from.x;
    let delta_y = segment.to.y - segment.from.y;
    let length_squared = delta_x * delta_x + delta_y * delta_y;
    let t = if length_squared <= f32::EPSILON {
        0.0
    } else {
        (((point.x - segment.from.x) * delta_x + (point.y - segment.from.y) * delta_y)
            / length_squared)
            .clamp(0.0, 1.0)
    };
    let closest_x = segment.from.x + delta_x * t;
    let closest_y = segment.from.y + delta_y * t;
    let distance_x = point.x - closest_x;
    let distance_y = point.y - closest_y;
    libm::sqrtf(distance_x * distance_x + distance_y * distance_y)
}

#[cfg(test)]
mod tests {
    use super::{ALL_ICONS, Icon, IconFill, IconStyle, IconWeight, rasterize_icon};
    use crate::color::Color;
    use crate::geometry::{Rect, Size};
    use crate::renderer::{CpuPaintRenderer, PaintContext};
    use alloc::sync::Arc;

    #[test]
    fn rasterized_tabler_icon_has_visible_alpha() {
        let raster = rasterize_icon(Icon::Folder, 32, IconStyle::outline());
        assert!(raster.mask.iter().any(|alpha| *alpha > 0));
    }

    #[test]
    fn repeated_rasterization_reuses_cached_mask() {
        let first = rasterize_icon(Icon::Settings, 20, IconStyle::outline());
        let second = rasterize_icon(Icon::Settings, 20, IconStyle::outline());
        assert!(Arc::ptr_eq(&first.mask, &second.mask));
    }

    #[test]
    fn stroke_width_is_configurable_and_part_of_cache_identity() {
        let default_style = IconStyle::outline();
        let thin_style = default_style.weight(IconWeight::Thin);
        assert_eq!(default_style.get_stroke_width(), 1.5);
        assert_eq!(thin_style.get_stroke_width(), 1.0);
        assert_eq!(
            default_style.weight(IconWeight::Bold).get_stroke_width(),
            2.0
        );

        let standard = rasterize_icon(Icon::Search, 20, default_style);
        let thin = rasterize_icon(Icon::Search, 20, thin_style);
        assert_ne!(standard.key, thin.key);
        assert!(!Arc::ptr_eq(&standard.mask, &thin.mask));
    }

    #[test]
    fn official_filled_icons_and_outline_fallback_are_supported() {
        assert!(Icon::Folder.has_filled());
        let filled_style = IconStyle::filled();
        assert_eq!(filled_style.get_fill(), IconFill::Filled);
        let filled = rasterize_icon(Icon::Folder, 32, filled_style);
        let outline = rasterize_icon(Icon::Folder, 32, IconStyle::outline());
        assert!(filled.mask.iter().any(|alpha| *alpha > 0));
        assert_ne!(filled.mask.as_ref(), outline.mask.as_ref());

        assert!(!Icon::ArrowLeft.has_filled());
        let fallback = rasterize_icon(Icon::ArrowLeft, 20, filled_style);
        let expected = rasterize_icon(Icon::ArrowLeft, 20, IconStyle::outline());
        assert_eq!(fallback.mask.as_ref(), expected.mask.as_ref());
    }

    #[test]
    fn every_selected_icon_rasterizes_to_visible_alpha() {
        for icon in ALL_ICONS {
            let raster = rasterize_icon(*icon, 16, IconStyle::outline());
            assert!(
                raster.mask.iter().any(|alpha| *alpha > 0),
                "{} produced an empty mask",
                icon.name()
            );
        }
    }

    #[test]
    fn every_available_filled_icon_rasterizes_to_visible_alpha() {
        for icon in ALL_ICONS.iter().filter(|icon| icon.has_filled()) {
            let raster = rasterize_icon(*icon, 16, IconStyle::filled());
            assert!(
                raster.mask.iter().any(|alpha| *alpha > 0),
                "{} filled variant produced an empty mask",
                icon.name()
            );
        }
    }

    #[test]
    fn cpu_renderer_tints_mask_at_physical_dpi() {
        let mut context = PaintContext::new();
        context.draw_icon(
            Rect::from_xywh(4.0, 4.0, 16.0, 16.0),
            Icon::Check,
            IconStyle::outline(),
            Color::rgb(1.0, 0.0, 0.0),
        );
        let mut renderer = CpuPaintRenderer::new(Size::new(24.0, 24.0), 2_000, Color::CLEAR);
        renderer.execute(&context);

        assert_eq!(renderer.buffer().width(), 48);
        assert!((0..renderer.buffer().height()).any(|y| {
            (0..renderer.buffer().width()).any(|x| {
                let pixel = renderer.buffer().get_pixel(x, y).unwrap_or_default();
                let [blue, green, red, alpha] = pixel.to_le_bytes();
                red > 0 && alpha > 0 && blue == 0 && green == 0
            })
        }));
    }
}
